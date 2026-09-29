//! Device discovery and download over serial, USB, USB HID and Bluetooth.
//!
//! libdivecomputer owns the transport and protocol details; this module drives
//! it: enumerate the devices a model can talk to, open a connection, download
//! every new dive and hand each raw dump to the same parser used for fixture
//! files.
//!
//! The download is synchronous and blocking, matching libdivecomputer's own
//! API. Cancellation is cooperative through an [`AtomicBool`] the caller can
//! flip from another thread.

use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_uint, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null_mut;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use benthic_core::Dive;

use crate::error::{check, Error};
use crate::ffi::*;
use crate::parse::{find_descriptor, parse_with, DeviceDescriptor, Transport};

/// How a discovered device is reached. Carries only what is needed to open it
/// again after discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceId {
    /// A serial device node, e.g. `/dev/ttyUSB0`.
    Serial(String),
    /// A USB device identified by vendor and product id.
    Usb { vid: u16, pid: u16 },
    /// A USB HID device identified by vendor and product id.
    UsbHid { vid: u16, pid: u16 },
    /// A Bluetooth RFCOMM device.
    Bluetooth {
        address: u64,
        port: u32,
        name: String,
    },
    /// A Bluetooth LE device, reached through BlueZ rather than libdivecomputer.
    Ble { address: String },
}

impl DeviceId {
    pub fn transport(&self) -> Transport {
        match self {
            DeviceId::Serial(_) => Transport::Serial,
            DeviceId::Usb { .. } => Transport::Usb,
            DeviceId::UsbHid { .. } => Transport::UsbHid,
            DeviceId::Bluetooth { .. } => Transport::Bluetooth,
            DeviceId::Ble { .. } => Transport::Ble,
        }
    }

    /// A human-readable address, suitable for display or logging.
    pub fn address(&self) -> String {
        match self {
            DeviceId::Serial(name) => name.clone(),
            DeviceId::Usb { vid, pid } | DeviceId::UsbHid { vid, pid } => {
                format!("{vid:04x}:{pid:04x}")
            }
            DeviceId::Bluetooth { name, .. } => name.clone(),
            DeviceId::Ble { address } => address.clone(),
        }
    }
}

/// A device found on a bus that can be downloaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    /// Friendly name reported by the transport, when it has one.
    pub name: String,
}

impl DiscoveredDevice {
    /// A label for a device picker.
    pub fn label(&self) -> String {
        if self.name.is_empty() || self.name == self.id.address() {
            self.id.address()
        } else {
            format!("{} ({})", self.name, self.id.address())
        }
    }
}

/// Identity a device reports at the start of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    pub model: u32,
    pub firmware: u32,
    pub serial: u32,
    pub hw_id: u32,
}

/// Progress and status reported while a download runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The device is busy and the transfer has stalled momentarily.
    Waiting,
    Progress {
        current: u32,
        maximum: u32,
    },
    /// Device identity, reported once near the start.
    Info(DeviceInfo),
    Clock {
        device_time: u32,
        system_time: i64,
    },
}

/// One dive recovered from a device, with the fingerprint libdivecomputer uses
/// to recognise it again.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadedDive {
    pub dive: Dive,
    pub fingerprint: Vec<u8>,
}

/// The result of a download.
#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub dives: Vec<DownloadedDive>,
    pub info: Option<DeviceInfo>,
    /// Fingerprint of the most recent dive, to register before the next
    /// download so already-seen dives are skipped.
    pub latest_fingerprint: Vec<u8>,
}

/// List the devices of a given transport that match a model.
pub fn scan(
    descriptor: &DeviceDescriptor,
    transport: Transport,
) -> Result<Vec<DiscoveredDevice>, Error> {
    unsafe {
        let mut context: *mut dc_context_t = null_mut();
        check(dc_context_new(&mut context))?;
        let result = match find_descriptor(context, &descriptor.vendor, &descriptor.product) {
            Ok(raw) => {
                let result = scan_raw(context, raw, transport);
                dc_descriptor_free(raw);
                result
            }
            Err(e) => Err(e),
        };
        dc_context_free(context);
        result
    }
}

/// Download the dives from a device.
///
/// `fingerprint` is the value from a previous [`Download::latest_fingerprint`];
/// pass an empty slice to fetch everything. `cancel` may be flipped from
/// another thread to abort. `on_event` is called for progress updates.
pub fn download(
    descriptor: &DeviceDescriptor,
    id: &DeviceId,
    fingerprint: &[u8],
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(DeviceEvent),
) -> Result<Download, Error> {
    unsafe {
        let mut context: *mut dc_context_t = null_mut();
        check(dc_context_new(&mut context))?;
        let result = download_inner(context, descriptor, id, fingerprint, cancel, &mut on_event);
        dc_context_free(context);
        result
    }
}

unsafe fn download_inner(
    context: *mut dc_context_t,
    descriptor: &DeviceDescriptor,
    id: &DeviceId,
    fingerprint: &[u8],
    cancel: Arc<AtomicBool>,
    on_event: &mut dyn FnMut(DeviceEvent),
) -> Result<Download, Error> {
    let raw = find_descriptor(context, &descriptor.vendor, &descriptor.product)?;
    // Closes whatever was opened, on every exit path.
    let mut owned = Owned {
        descriptor: raw,
        iostream: null_mut(),
        device: null_mut(),
        _ble: None,
    };

    let (iostream, ble) = open_stream(context, raw, id)?;
    owned.iostream = iostream;
    owned._ble = ble;
    check(dc_device_open(
        &mut owned.device,
        context,
        raw,
        owned.iostream,
    ))?;

    let mut state = DownloadState {
        context,
        descriptor: raw,
        dives: Vec::new(),
        info: None,
        latest_fingerprint: Vec::new(),
        error: None,
        cancel: cancel.clone(),
        on_event,
    };

    let events = DC_EVENT_WAITING | DC_EVENT_PROGRESS | DC_EVENT_DEVINFO | DC_EVENT_CLOCK;
    check(dc_device_set_events(
        owned.device,
        events,
        event_callback,
        &mut state as *mut _ as *mut c_void,
    ))?;
    check(dc_device_set_cancel(
        owned.device,
        cancel_callback,
        &mut state as *mut _ as *mut c_void,
    ))?;
    if !fingerprint.is_empty() {
        check(dc_device_set_fingerprint(
            owned.device,
            fingerprint.as_ptr(),
            fingerprint.len() as c_uint,
        ))?;
    }

    let status = dc_device_foreach(
        owned.device,
        dive_callback,
        &mut state as *mut _ as *mut c_void,
    );

    if let Some(error) = state.error {
        return Err(error);
    }
    if status == DC_STATUS_CANCELLED || cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    check(status)?;

    Ok(Download {
        dives: state.dives,
        info: state.info,
        latest_fingerprint: state.latest_fingerprint,
    })
}

/// RAII guard for the C objects opened during a download.
struct Owned {
    descriptor: *mut dc_descriptor_t,
    iostream: *mut dc_iostream_t,
    device: *mut dc_device_t,
    /// Keeps the BLE GATT thread alive for the whole download.
    _ble: Option<crate::ble::BleConnection>,
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            if !self.device.is_null() {
                dc_device_close(self.device);
            }
            if !self.iostream.is_null() {
                dc_iostream_close(self.iostream);
            }
            if !self.descriptor.is_null() {
                dc_descriptor_free(self.descriptor);
            }
        }
    }
}

unsafe fn open_stream(
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    id: &DeviceId,
) -> Result<(*mut dc_iostream_t, Option<crate::ble::BleConnection>), Error> {
    let mut iostream: *mut dc_iostream_t = null_mut();
    let mut ble = None;
    match id {
        DeviceId::Serial(name) => {
            let name = CString::new(name.as_str()).map_err(|_| Error::NoDevice {
                transport: "serial",
            })?;
            check(dc_serial_open(&mut iostream, context, name.as_ptr()))?;
        }
        DeviceId::Usb { vid, pid } => {
            let device = find_usb(context, descriptor, *vid, *pid)?;
            let result = check(dc_usb_open(&mut iostream, context, device));
            dc_usb_device_free(device);
            result?;
        }
        DeviceId::UsbHid { vid, pid } => {
            let device = find_usbhid(context, descriptor, *vid, *pid)?;
            let result = check(dc_usbhid_open(&mut iostream, context, device));
            dc_usbhid_device_free(device);
            result?;
        }
        DeviceId::Bluetooth { address, port, .. } => {
            check(dc_bluetooth_open(&mut iostream, context, *address, *port))?;
        }
        DeviceId::Ble { address } => {
            let connection = crate::ble::BleConnection::connect(address)?;
            iostream = connection.open_iostream(context)?;
            ble = Some(connection);
        }
    }
    Ok((iostream, ble))
}

unsafe fn scan_raw(
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    transport: Transport,
) -> Result<Vec<DiscoveredDevice>, Error> {
    let mut iterator: *mut dc_iterator_t = null_mut();
    let mut out = Vec::new();
    match transport {
        Transport::Serial => {
            check(dc_serial_iterator_new(&mut iterator, context, descriptor))?;
            loop {
                let mut device: *mut dc_serial_device_t = null_mut();
                if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void)
                    != DC_STATUS_SUCCESS
                {
                    break;
                }
                let name = c_string(dc_serial_device_get_name(device));
                out.push(DiscoveredDevice {
                    id: DeviceId::Serial(name.clone()),
                    name,
                });
                dc_serial_device_free(device);
            }
        }
        Transport::Usb => {
            check(dc_usb_iterator_new(&mut iterator, context, descriptor))?;
            loop {
                let mut device: *mut dc_usb_device_t = null_mut();
                if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void)
                    != DC_STATUS_SUCCESS
                {
                    break;
                }
                let id = DeviceId::Usb {
                    vid: dc_usb_device_get_vid(device) as u16,
                    pid: dc_usb_device_get_pid(device) as u16,
                };
                out.push(DiscoveredDevice {
                    name: id.address(),
                    id,
                });
                dc_usb_device_free(device);
            }
        }
        Transport::UsbHid => {
            check(dc_usbhid_iterator_new(&mut iterator, context, descriptor))?;
            loop {
                let mut device: *mut dc_usbhid_device_t = null_mut();
                if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void)
                    != DC_STATUS_SUCCESS
                {
                    break;
                }
                let id = DeviceId::UsbHid {
                    vid: dc_usbhid_device_get_vid(device) as u16,
                    pid: dc_usbhid_device_get_pid(device) as u16,
                };
                out.push(DiscoveredDevice {
                    name: id.address(),
                    id,
                });
                dc_usbhid_device_free(device);
            }
        }
        Transport::Bluetooth => {
            check(dc_bluetooth_iterator_new(
                &mut iterator,
                context,
                descriptor,
            ))?;
            loop {
                let mut device: *mut dc_bluetooth_device_t = null_mut();
                if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void)
                    != DC_STATUS_SUCCESS
                {
                    break;
                }
                let address = dc_bluetooth_device_get_address(device);
                let name = c_string(dc_bluetooth_device_get_name(device));
                out.push(DiscoveredDevice {
                    id: DeviceId::Bluetooth {
                        address,
                        port: 0,
                        name: name.clone(),
                    },
                    name,
                });
                dc_bluetooth_device_free(device);
            }
        }
        _ => return Err(unsupported()),
    }
    dc_iterator_free(iterator);
    Ok(out)
}

unsafe fn find_usb(
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    vid: u16,
    pid: u16,
) -> Result<*mut dc_usb_device_t, Error> {
    let mut iterator: *mut dc_iterator_t = null_mut();
    check(dc_usb_iterator_new(&mut iterator, context, descriptor))?;
    let mut found: *mut dc_usb_device_t = null_mut();
    loop {
        let mut device: *mut dc_usb_device_t = null_mut();
        if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void) != DC_STATUS_SUCCESS {
            break;
        }
        if dc_usb_device_get_vid(device) as u16 == vid
            && dc_usb_device_get_pid(device) as u16 == pid
        {
            found = device;
            break;
        }
        dc_usb_device_free(device);
    }
    dc_iterator_free(iterator);
    if found.is_null() {
        Err(Error::NoDevice { transport: "usb" })
    } else {
        Ok(found)
    }
}

unsafe fn find_usbhid(
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    vid: u16,
    pid: u16,
) -> Result<*mut dc_usbhid_device_t, Error> {
    let mut iterator: *mut dc_iterator_t = null_mut();
    check(dc_usbhid_iterator_new(&mut iterator, context, descriptor))?;
    let mut found: *mut dc_usbhid_device_t = null_mut();
    loop {
        let mut device: *mut dc_usbhid_device_t = null_mut();
        if dc_iterator_next(iterator, &mut device as *mut _ as *mut c_void) != DC_STATUS_SUCCESS {
            break;
        }
        if dc_usbhid_device_get_vid(device) as u16 == vid
            && dc_usbhid_device_get_pid(device) as u16 == pid
        {
            found = device;
            break;
        }
        dc_usbhid_device_free(device);
    }
    dc_iterator_free(iterator);
    if found.is_null() {
        Err(Error::NoDevice {
            transport: "usbhid",
        })
    } else {
        Ok(found)
    }
}

fn unsupported() -> Error {
    Error::Status {
        status: DC_STATUS_UNSUPPORTED,
        message: "unsupported transport",
    }
}

fn c_string(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Mutable state shared with the C callbacks. The lifetimes are erased when the
/// pointer crosses the FFI boundary; the object outlives every callback.
struct DownloadState<'a> {
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    dives: Vec<DownloadedDive>,
    info: Option<DeviceInfo>,
    latest_fingerprint: Vec<u8>,
    error: Option<Error>,
    cancel: Arc<AtomicBool>,
    on_event: &'a mut dyn FnMut(DeviceEvent),
}

extern "C" fn dive_callback(
    data: *const u8,
    size: c_uint,
    fingerprint: *const u8,
    fsize: c_uint,
    userdata: *mut c_void,
) -> c_int {
    let state = unsafe { &mut *(userdata as *mut DownloadState<'static>) };
    let keep_going = catch_unwind(AssertUnwindSafe(|| {
        if state.cancel.load(Ordering::Relaxed) {
            return false;
        }
        if data.is_null() {
            return true;
        }
        let bytes = unsafe { slice::from_raw_parts(data, size as usize) };
        let fingerprint = if fingerprint.is_null() || fsize == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(fingerprint, fsize as usize) }.to_vec()
        };
        if state.latest_fingerprint.is_empty() && !fingerprint.is_empty() {
            state.latest_fingerprint.clone_from(&fingerprint);
        }
        match unsafe { parse_with(state.context, state.descriptor, bytes) } {
            Ok(dive) => {
                state.dives.push(DownloadedDive { dive, fingerprint });
                true
            }
            Err(error) => {
                state.error = Some(error);
                false
            }
        }
    }));
    match keep_going {
        Ok(true) => 1,
        _ => 0,
    }
}

extern "C" fn event_callback(
    _device: *mut dc_device_t,
    event: c_uint,
    data: *const c_void,
    userdata: *mut c_void,
) {
    let state = unsafe { &mut *(userdata as *mut DownloadState<'static>) };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let event = match event {
            DC_EVENT_WAITING => DeviceEvent::Waiting,
            DC_EVENT_PROGRESS => {
                let progress = unsafe { &*(data as *const dc_event_progress_t) };
                DeviceEvent::Progress {
                    current: progress.current,
                    maximum: progress.maximum,
                }
            }
            DC_EVENT_DEVINFO => {
                let devinfo = unsafe { &*(data as *const dc_event_devinfo_t) };
                let info = DeviceInfo {
                    model: devinfo.model,
                    firmware: devinfo.firmware,
                    serial: devinfo.serial,
                    hw_id: devinfo.hw_id,
                };
                state.info = Some(info);
                DeviceEvent::Info(info)
            }
            DC_EVENT_CLOCK => {
                let clock = unsafe { &*(data as *const dc_event_clock_t) };
                DeviceEvent::Clock {
                    device_time: clock.devtime,
                    system_time: clock.systime,
                }
            }
            _ => return,
        };
        (state.on_event)(event);
    }));
}

extern "C" fn cancel_callback(userdata: *mut c_void) -> c_int {
    let state = unsafe { &*(userdata as *const DownloadState<'static>) };
    if state.cancel.load(Ordering::Relaxed) {
        1
    } else {
        0
    }
}
