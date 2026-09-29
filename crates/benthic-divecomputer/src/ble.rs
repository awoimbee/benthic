// Desktop BLE transport, over BlueZ via `bluer`.
//
// libdivecomputer has no native BLE support: it expects the application to
// provide a `dc_iostream` for a specific service. We connect, pick a known
// GATT serial service, and stream over its write/notify characteristics.
//
// libdivecomputer is synchronous while `bluer` is async, so the connection
// lives on its own thread with a current-thread tokio runtime. The C
// callbacks send a request over a channel and block on the reply.

use std::collections::VecDeque;
use std::os::raw::{c_int, c_uint, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bluer::{AdapterEvent, Session, Uuid};
use futures::StreamExt;

use crate::error::{check, Error};
use crate::ffi::*;

/// GATT serial services, in preference order (mirrors Subsurface).
pub const SERIAL_SERVICE_UUIDS: [&str; 14] = [
    "0000fefb-0000-1000-8000-00805f9b34fb",
    "2456e1b9-26e2-8f83-e744-f34f01e9d701",
    "544e326b-5b72-c6b0-1c46-41c1bc448118",
    "98ae7120-e62e-11e3-badd-0002a5d5c51b",
    "cb3c4555-d670-4670-bc20-b61dbc851e9a",
    "ca7b0001-f785-4c38-b599-c7c5fbadb034",
    "fdcdeaaa-295d-470e-bf15-04217b7aa0a0",
    "fe25c237-0ece-443c-b0aa-e02033e7029d",
    "1aa44039-1667-4b29-87cc-dfecaaf31d97",
    "0000fcef-0000-1000-8000-00805f9b34fb",
    "6e400001-b5a3-f393-e0a9-e50e24dc10b8",
    "6e400001-b5a3-f393-e0a9-e50e24dcca9e",
    "00000001-8c3b-4f2c-a59e-8c08224f3253",
    "84968ffe-d26d-478a-b953-5010bcf58bca",
];

fn is_serial_service(uuid: &Uuid) -> bool {
    let text = uuid.to_string().to_lowercase();
    SERIAL_SERVICE_UUIDS.contains(&text.as_str())
}

/// A Bluetooth LE dive computer found by [`scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleDevice {
    pub address: String,
    pub name: String,
}

/// Scan for BLE devices that advertise a known serial service.
pub fn scan(timeout: Duration) -> Result<Vec<BleDevice>, Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::Message(error.to_string()))?;
    runtime.block_on(async move {
        let session = Session::new().await.map_err(message)?;
        let adapter = session.default_adapter().await.map_err(message)?;
        adapter.set_powered(true).await.map_err(message)?;
        let mut events = adapter.discover_devices().await.map_err(message)?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut found = Vec::new();
        loop {
            tokio::select! {
                event = events.next() => match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        if let Ok(device) = adapter.device(address) {
                            let uuids = device.uuids().await.ok().flatten().unwrap_or_default();
                            if uuids.iter().any(is_serial_service) {
                                found.push(BleDevice {
                                    address: address.to_string(),
                                    name: device.name().await.ok().flatten().unwrap_or_default(),
                                });
                            }
                        }
                    }
                    Some(_) => {}
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        Ok(found)
    })
}

fn message(error: bluer::Error) -> Error {
    Error::Message(error.to_string())
}

// -- request/response bridge ------------------------------------------------

enum BleResponse {
    Bytes(Vec<u8>),
    Count(usize),
    Bool(bool),
    Ok,
    /// Kept for diagnostics; the callbacks only surface a status code.
    #[allow(dead_code)]
    Error(String),
}

type Reply = Sender<BleResponse>;

enum BleRequest {
    Read {
        size: usize,
        timeout: Duration,
        reply: Reply,
    },
    Write {
        data: Vec<u8>,
        reply: Reply,
    },
    Poll {
        timeout: Duration,
        reply: Reply,
    },
    Available {
        reply: Reply,
    },
    Sleep {
        ms: u64,
        reply: Reply,
    },
    Ioctl {
        op: u32,
        data: Vec<u8>,
        reply: Reply,
    },
    Close {
        reply: Reply,
    },
}

/// The stable userdata every iostream callback receives.
struct BleHandle {
    requests: Sender<BleRequest>,
    timeout_ms: AtomicI32,
}

impl BleHandle {
    fn call(&self, make: impl FnOnce(Reply) -> BleRequest) -> Result<BleResponse, ()> {
        let (tx, rx) = mpsc::channel();
        self.requests.send(make(tx)).map_err(|_| ())?;
        rx.recv().map_err(|_| ())
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed).max(0) as u64)
    }
}

/// An open BLE connection. Keeps the GATT thread alive while it exists.
pub struct BleConnection {
    handle: Box<BleHandle>,
    thread: Option<JoinHandle<()>>,
}

impl BleConnection {
    /// Connect, discover the serial service and start notifications.
    pub fn connect(address: &str) -> Result<Self, Error> {
        let (request_tx, request_rx) = mpsc::channel::<BleRequest>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let address = address.to_string();
        let thread = std::thread::Builder::new()
            .name("benthic-ble".to_string())
            .spawn(move || gatt_thread(address, request_rx, ready_tx))
            .map_err(|error| Error::Message(error.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                handle: Box::new(BleHandle {
                    requests: request_tx,
                    timeout_ms: AtomicI32::new(0),
                }),
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(Error::Message(error)),
            Err(_) => Err(Error::Message("BLE thread failed to start".to_string())),
        }
    }

    /// Create the libdivecomputer iostream backed by this connection.
    ///
    /// # Safety
    /// `context` must be a valid libdivecomputer context. The returned stream
    /// borrows `self`, which must outlive it.
    pub unsafe fn open_iostream(
        &self,
        context: *mut dc_context_t,
    ) -> Result<*mut dc_iostream_t, Error> {
        let mut iostream: *mut dc_iostream_t = null_mut();
        let userdata = self.handle.as_ref() as *const BleHandle as *mut c_void;
        check(dc_custom_open(
            &mut iostream,
            context,
            DC_TRANSPORT_BLE,
            &BLE_CALLBACKS,
            userdata,
        ))?;
        Ok(iostream)
    }
}

impl Drop for BleConnection {
    fn drop(&mut self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.handle.requests.send(BleRequest::Close { reply: tx });
        let _ = rx.recv();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct BleWorker {
    device: bluer::Device,
    service: bluer::gatt::remote::Service,
    write_char: bluer::gatt::remote::Characteristic,
    notify_char: Option<bluer::gatt::remote::Characteristic>,
    read_char: Option<bluer::gatt::remote::Characteristic>,
    name: String,
    access_code: Vec<u8>,
    queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    notifier: Arc<tokio::sync::Notify>,
}

impl BleWorker {
    async fn connect(address: &str) -> Result<Self, String> {
        let session = Session::new().await.map_err(text)?;
        let adapter = session.default_adapter().await.map_err(text)?;
        adapter.set_powered(true).await.map_err(text)?;
        let address: bluer::Address = address.parse().map_err(|_| "invalid address".to_string())?;
        let device = adapter.device(address).map_err(text)?;
        if !device.is_connected().await.map_err(text)? {
            device.connect().await.map_err(text)?;
        }

        for service in device.services().await.map_err(text)? {
            let uuid = service.uuid().await.map_err(text)?;
            if !is_serial_service(&uuid) {
                continue;
            }
            let mut write_char = None;
            let mut notify_char = None;
            let mut read_char = None;
            for characteristic in service.characteristics().await.map_err(text)? {
                let flags = characteristic.flags().await.map_err(text)?;
                if flags.write || flags.write_without_response {
                    write_char = Some(characteristic.clone());
                }
                if flags.notify || flags.indicate {
                    notify_char = Some(characteristic.clone());
                }
                if flags.read {
                    read_char = Some(characteristic.clone());
                }
            }
            if let Some(write_char) = write_char {
                return Ok(Self {
                    name: device.name().await.ok().flatten().unwrap_or_default(),
                    device,
                    service,
                    write_char,
                    notify_char,
                    read_char,
                    access_code: Vec::new(),
                    queue: Arc::new(Mutex::new(VecDeque::new())),
                    notifier: Arc::new(tokio::sync::Notify::new()),
                });
            }
        }
        Err("no known serial service on this device".to_string())
    }

    fn spawn_collector(&self, runtime: &tokio::runtime::Runtime) {
        let Some(characteristic) = self.notify_char.clone() else {
            return;
        };
        let queue = self.queue.clone();
        let notifier = self.notifier.clone();
        runtime.spawn(async move {
            let Ok(stream) = characteristic.notify().await else {
                return;
            };
            futures::pin_mut!(stream);
            while let Some(packet) = stream.next().await {
                queue.lock().expect("ble queue").push_back(packet);
                notifier.notify_waiters();
            }
        });
    }

    async fn handle(&mut self, request: BleRequest) {
        match request {
            BleRequest::Read {
                size,
                timeout,
                reply,
            } => {
                let deadline = tokio::time::Instant::now() + timeout;
                loop {
                    if let Some(packet) = self.queue.lock().expect("ble queue").pop_front() {
                        let _ = reply.send(BleResponse::Bytes(truncate(packet, size)));
                        return;
                    }
                    if self.notify_char.is_none() {
                        let response = match &self.read_char {
                            Some(characteristic) => match characteristic.read().await {
                                Ok(value) => BleResponse::Bytes(truncate(value, size)),
                                Err(error) => BleResponse::Error(error.to_string()),
                            },
                            None => BleResponse::Bytes(Vec::new()),
                        };
                        let _ = reply.send(response);
                        return;
                    }
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        let _ = reply.send(BleResponse::Bytes(Vec::new()));
                        return;
                    }
                    tokio::select! {
                        _ = self.notifier.notified() => {}
                        _ = tokio::time::sleep(remaining) => {
                            let _ = reply.send(BleResponse::Bytes(Vec::new()));
                            return;
                        }
                    }
                }
            }
            BleRequest::Write { data, reply } => {
                let response = match self.write_char.write(&data).await {
                    Ok(()) => BleResponse::Count(data.len()),
                    Err(error) => BleResponse::Error(error.to_string()),
                };
                let _ = reply.send(response);
            }
            BleRequest::Poll { timeout, reply } => {
                if !self.queue.lock().expect("ble queue").is_empty() {
                    let _ = reply.send(BleResponse::Bool(true));
                    return;
                }
                if self.notify_char.is_none() {
                    let _ = reply.send(BleResponse::Bool(false));
                    return;
                }
                tokio::select! {
                    _ = self.notifier.notified() => { let _ = reply.send(BleResponse::Bool(true)); }
                    _ = tokio::time::sleep(timeout) => { let _ = reply.send(BleResponse::Bool(false)); }
                }
            }
            BleRequest::Available { reply } => {
                let count = self.queue.lock().expect("ble queue").len();
                let _ = reply.send(BleResponse::Count(count));
            }
            BleRequest::Sleep { ms, reply } => {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let _ = reply.send(BleResponse::Ok);
            }
            BleRequest::Ioctl { op, data, reply } => {
                let response = self.ioctl(op, data).await;
                let _ = reply.send(response);
            }
            BleRequest::Close { reply } => {
                let _ = self.device.disconnect().await;
                let _ = reply.send(BleResponse::Ok);
            }
        }
    }

    async fn ioctl(&mut self, op: u32, data: Vec<u8>) -> BleResponse {
        match op {
            0 => {
                let mut name = self.name.clone().into_bytes();
                name.push(0);
                BleResponse::Bytes(name)
            }
            1 => BleResponse::Bytes(vec![0]),
            2 => BleResponse::Bytes(self.access_code.clone()),
            3 => {
                self.access_code = data;
                BleResponse::Ok
            }
            4 => {
                // The first 16 bytes are the characteristic UUID.
                if data.len() < 16 {
                    return BleResponse::Error("short characteristic uuid".to_string());
                }
                let uuid = uuid_from_bytes(&data[..16]);
                let characteristics = match self.service.characteristics().await {
                    Ok(characteristics) => characteristics,
                    Err(error) => return BleResponse::Error(error.to_string()),
                };
                for characteristic in characteristics {
                    if characteristic.uuid().await.ok() == Some(uuid) {
                        return match characteristic.read().await {
                            Ok(value) => BleResponse::Bytes(value),
                            Err(error) => BleResponse::Error(error.to_string()),
                        };
                    }
                }
                BleResponse::Error("characteristic not found".to_string())
            }
            other => BleResponse::Error(format!("unsupported BLE ioctl {other}")),
        }
    }
}

fn text(error: bluer::Error) -> String {
    error.to_string()
}

fn truncate(mut value: Vec<u8>, size: usize) -> Vec<u8> {
    value.truncate(size);
    value
}

fn uuid_from_bytes(bytes: &[u8]) -> Uuid {
    use std::fmt::Write;
    let mut text = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        let _ = write!(text, "{byte:02x}");
    }
    Uuid::parse_str(&text).unwrap_or(Uuid::nil())
}

fn gatt_thread(address: String, requests: Receiver<BleRequest>, ready: Sender<Result<(), String>>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let worker = runtime.block_on(BleWorker::connect(&address));
    let mut worker = match worker {
        Ok(worker) => worker,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    worker.spawn_collector(&runtime);
    let _ = ready.send(Ok(()));

    while let Ok(request) = requests.recv() {
        let closing = matches!(request, BleRequest::Close { .. });
        runtime.block_on(worker.handle(request));
        if closing {
            break;
        }
    }
}

// -- C callbacks ------------------------------------------------------------

unsafe fn handle<'a>(userdata: *mut c_void) -> &'a BleHandle {
    unsafe { &*(userdata as *const BleHandle) }
}

extern "C" fn io_set_timeout(userdata: *mut c_void, timeout: c_int) -> c_int {
    let handle = unsafe { handle(userdata) };
    handle.timeout_ms.store(timeout, Ordering::Relaxed);
    DC_STATUS_SUCCESS
}

extern "C" fn io_configure(
    _userdata: *mut c_void,
    _baudrate: c_uint,
    _databits: c_uint,
    _parity: c_uint,
    _stopbits: c_uint,
    _flowcontrol: c_uint,
) -> c_int {
    DC_STATUS_SUCCESS
}

extern "C" fn io_read(
    userdata: *mut c_void,
    data: *mut c_void,
    size: usize,
    actual: *mut usize,
) -> c_int {
    let handle = unsafe { handle(userdata) };
    let timeout = handle.timeout();
    match handle.call(|reply| BleRequest::Read {
        size,
        timeout,
        reply,
    }) {
        Ok(BleResponse::Bytes(bytes)) => {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, bytes.len());
                *actual = bytes.len();
            }
            DC_STATUS_SUCCESS
        }
        _ => DC_STATUS_IO,
    }
}

extern "C" fn io_write(
    userdata: *mut c_void,
    data: *const c_void,
    size: usize,
    actual: *mut usize,
) -> c_int {
    let handle = unsafe { handle(userdata) };
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size) }.to_vec();
    match handle.call(|reply| BleRequest::Write { data: bytes, reply }) {
        Ok(BleResponse::Count(count)) => {
            unsafe { *actual = count };
            DC_STATUS_SUCCESS
        }
        _ => DC_STATUS_IO,
    }
}

extern "C" fn io_poll(userdata: *mut c_void, timeout: c_int) -> c_int {
    let handle = unsafe { handle(userdata) };
    let timeout = Duration::from_millis(timeout.max(0) as u64);
    match handle.call(|reply| BleRequest::Poll { timeout, reply }) {
        Ok(BleResponse::Bool(true)) => DC_STATUS_SUCCESS,
        _ => DC_STATUS_TIMEOUT,
    }
}

extern "C" fn io_get_available(userdata: *mut c_void, value: *mut usize) -> c_int {
    let handle = unsafe { handle(userdata) };
    match handle.call(|reply| BleRequest::Available { reply }) {
        Ok(BleResponse::Count(count)) => {
            unsafe { *value = count };
            DC_STATUS_SUCCESS
        }
        _ => DC_STATUS_IO,
    }
}

extern "C" fn io_sleep(userdata: *mut c_void, ms: c_uint) -> c_int {
    let handle = unsafe { handle(userdata) };
    let _ = handle.call(|reply| BleRequest::Sleep {
        ms: ms as u64,
        reply,
    });
    DC_STATUS_SUCCESS
}

extern "C" fn io_flush(_userdata: *mut c_void) -> c_int {
    DC_STATUS_SUCCESS
}

extern "C" fn io_purge(_userdata: *mut c_void, _direction: c_uint) -> c_int {
    DC_STATUS_SUCCESS
}

extern "C" fn io_close(_userdata: *mut c_void) -> c_int {
    // The connection is closed when `BleConnection` is dropped.
    DC_STATUS_SUCCESS
}

extern "C" fn io_ioctl(
    userdata: *mut c_void,
    request: c_uint,
    data: *mut c_void,
    size: usize,
) -> c_int {
    let handle = unsafe { handle(userdata) };
    let op = match request {
        DC_IOCTL_BLE_GET_NAME => 0,
        DC_IOCTL_BLE_GET_PINCODE => 1,
        DC_IOCTL_BLE_GET_ACCESSCODE => 2,
        DC_IOCTL_BLE_SET_ACCESSCODE => 3,
        DC_IOCTL_BLE_CHARACTERISTIC_READ => 4,
        _ => return DC_STATUS_UNSUPPORTED,
    };
    let input = unsafe { std::slice::from_raw_parts(data as *const u8, size) }.to_vec();
    match handle.call(|reply| BleRequest::Ioctl {
        op,
        data: input,
        reply,
    }) {
        Ok(BleResponse::Bytes(bytes)) => {
            unsafe {
                let count = bytes.len().min(size);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, count);
            }
            DC_STATUS_SUCCESS
        }
        Ok(BleResponse::Ok) => DC_STATUS_SUCCESS,
        _ => DC_STATUS_IO,
    }
}

extern "C" fn io_unsupported_u32(_userdata: *mut c_void, _value: c_uint) -> c_int {
    DC_STATUS_UNSUPPORTED
}

static BLE_CALLBACKS: dc_custom_cbs_t = dc_custom_cbs_t {
    set_timeout: Some(io_set_timeout),
    set_break: Some(io_unsupported_u32),
    set_dtr: Some(io_unsupported_u32),
    set_rts: Some(io_unsupported_u32),
    get_lines: None,
    get_available: Some(io_get_available),
    configure: Some(io_configure),
    poll: Some(io_poll),
    read: Some(io_read),
    write: Some(io_write),
    ioctl: Some(io_ioctl),
    flush: Some(io_flush),
    purge: Some(io_purge),
    sleep: Some(io_sleep),
    close: Some(io_close),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_known_serial_services() {
        let shearwater = Uuid::parse_str("fe25c237-0ece-443c-b0aa-e02033e7029d").unwrap();
        assert!(is_serial_service(&shearwater));
        let other = Uuid::parse_str("180d0000-0000-1000-8000-00805f9b34fb").unwrap();
        assert!(!is_serial_service(&other));
    }

    #[test]
    fn formats_characteristic_uuids() {
        let bytes: [u8; 16] = [
            0xfe, 0x25, 0xc2, 0x37, 0x0e, 0xce, 0x44, 0x3c, 0xb0, 0xaa, 0xe0, 0x20, 0x33, 0xe7,
            0x02, 0x9d,
        ];
        assert_eq!(
            uuid_from_bytes(&bytes).to_string(),
            "fe25c237-0ece-443c-b0aa-e02033e7029d"
        );
    }
}
