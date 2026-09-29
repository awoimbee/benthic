//! Raw FFI declarations for the subset of libdivecomputer that benthic uses.
//!
//! Hand-written rather than generated so the build needs only a C compiler,
//! not `clang`/bindgen. Only the opaque types, the sample union and the few
//! entry points we call are declared; everything else stays hidden.
#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_double, c_int, c_uint, c_void};

// -- Opaque handles ---------------------------------------------------------

pub enum dc_context_t {}
pub enum dc_descriptor_t {}
pub enum dc_iterator_t {}
pub enum dc_parser_t {}
pub enum dc_device_t {}

// -- Status codes (dc_status_t) --------------------------------------------

pub const DC_STATUS_SUCCESS: c_int = 0;
pub const DC_STATUS_DONE: c_int = 1;
pub const DC_STATUS_UNSUPPORTED: c_int = -1;
pub const DC_STATUS_INVALIDARGS: c_int = -2;
pub const DC_STATUS_NOMEMORY: c_int = -3;
pub const DC_STATUS_NODEVICE: c_int = -4;
pub const DC_STATUS_NOACCESS: c_int = -5;
pub const DC_STATUS_IO: c_int = -6;
pub const DC_STATUS_TIMEOUT: c_int = -7;
pub const DC_STATUS_PROTOCOL: c_int = -8;
pub const DC_STATUS_DATAFORMAT: c_int = -9;
pub const DC_STATUS_CANCELLED: c_int = -10;

// -- Transports (dc_transport_t, a bitmask) --------------------------------

pub const DC_TRANSPORT_NONE: c_uint = 0;
pub const DC_TRANSPORT_SERIAL: c_uint = 1 << 0;
pub const DC_TRANSPORT_USB: c_uint = 1 << 1;
pub const DC_TRANSPORT_USBHID: c_uint = 1 << 2;
pub const DC_TRANSPORT_IRDA: c_uint = 1 << 3;
pub const DC_TRANSPORT_BLUETOOTH: c_uint = 1 << 4;
pub const DC_TRANSPORT_BLE: c_uint = 1 << 5;
pub const DC_TRANSPORT_USBSTORAGE: c_uint = 1 << 6;

// -- Sample types (dc_sample_type_t) ---------------------------------------

pub const DC_SAMPLE_TIME: c_int = 0;
pub const DC_SAMPLE_DEPTH: c_int = 1;
pub const DC_SAMPLE_PRESSURE: c_int = 2;
pub const DC_SAMPLE_TEMPERATURE: c_int = 3;
pub const DC_SAMPLE_EVENT: c_int = 4;
pub const DC_SAMPLE_RBT: c_int = 5;
pub const DC_SAMPLE_HEARTBEAT: c_int = 6;
pub const DC_SAMPLE_BEARING: c_int = 7;
pub const DC_SAMPLE_VENDOR: c_int = 8;
pub const DC_SAMPLE_SETPOINT: c_int = 9;
pub const DC_SAMPLE_PPO2: c_int = 10;
pub const DC_SAMPLE_CNS: c_int = 11;
pub const DC_SAMPLE_DECO: c_int = 12;
pub const DC_SAMPLE_GASMIX: c_int = 13;
pub const DC_SAMPLE_TTS: c_int = 14;
pub const DC_SAMPLE_LOCATION: c_int = 15;

// -- Parser fields (dc_field_type_t) ---------------------------------------

pub const DC_FIELD_DIVETIME: c_int = 0;
pub const DC_FIELD_MAXDEPTH: c_int = 1;
pub const DC_FIELD_AVGDEPTH: c_int = 2;
pub const DC_FIELD_GASMIX_COUNT: c_int = 3;
pub const DC_FIELD_GASMIX: c_int = 4;
pub const DC_FIELD_SALINITY: c_int = 5;
pub const DC_FIELD_ATMOSPHERIC: c_int = 6;
pub const DC_FIELD_TEMPERATURE_SURFACE: c_int = 7;
pub const DC_FIELD_TEMPERATURE_MINIMUM: c_int = 8;
pub const DC_FIELD_TEMPERATURE_MAXIMUM: c_int = 9;
pub const DC_FIELD_TANK_COUNT: c_int = 10;
pub const DC_FIELD_TANK: c_int = 11;
pub const DC_FIELD_DIVEMODE: c_int = 12;
pub const DC_FIELD_DECOMODEL: c_int = 13;

// -- Dive modes (dc_divemode_t) --------------------------------------------

pub const DC_DIVEMODE_FREEDIVE: c_uint = 0;
pub const DC_DIVEMODE_GAUGE: c_uint = 1;
pub const DC_DIVEMODE_OC: c_uint = 2;
pub const DC_DIVEMODE_CCR: c_uint = 3;
pub const DC_DIVEMODE_SCR: c_uint = 4;

// -- Decompression sample kinds (dc_deco_type_t) ---------------------------

pub const DC_DECO_NDL: c_uint = 0;
pub const DC_DECO_SAFETYSTOP: c_uint = 1;
pub const DC_DECO_DECOSTOP: c_uint = 2;
pub const DC_DECO_DEEPSTOP: c_uint = 3;

// -- Gas usage (dc_usage_t) ------------------------------------------------

pub const DC_USAGE_NONE: c_uint = 0;
pub const DC_USAGE_OXYGEN: c_uint = 1;
pub const DC_USAGE_DILUENT: c_uint = 2;
pub const DC_USAGE_OPEN_CIRCUIT: c_uint = 3;

// -- Water type (dc_water_t) -----------------------------------------------

pub const DC_WATER_SALT: c_uint = 1;

// -- Unsupported sentinels -------------------------------------------------

pub const DC_GASMIX_UNKNOWN: c_uint = 0xFFFF_FFFF;
pub const DC_TIMEZONE_NONE: c_int = c_int::MIN;
pub const DC_SAMPLE_EVENT_NONE: c_uint = 0;
pub const DC_SAMPLE_EVENT_DECOSTOP: c_uint = 1;
pub const DC_SAMPLE_EVENT_RBT: c_uint = 2;
pub const DC_SAMPLE_EVENT_ASCENT: c_uint = 3;
pub const DC_SAMPLE_EVENT_CEILING: c_uint = 4;
pub const DC_SAMPLE_EVENT_WORKLOAD: c_uint = 5;
pub const DC_SAMPLE_EVENT_TRANSMITTER: c_uint = 6;
pub const DC_SAMPLE_EVENT_VIOLATION: c_uint = 7;
pub const DC_SAMPLE_EVENT_BOOKMARK: c_uint = 8;
pub const DC_SAMPLE_EVENT_SURFACE: c_uint = 9;
pub const DC_SAMPLE_EVENT_SAFETYSTOP: c_uint = 10;
pub const DC_SAMPLE_EVENT_GASCHANGE: c_uint = 11;
pub const DC_SAMPLE_EVENT_SAFETYSTOP_VOLUNTARY: c_uint = 12;
pub const DC_SAMPLE_EVENT_SAFETYSTOP_MANDATORY: c_uint = 13;
pub const DC_SAMPLE_EVENT_DEEPSTOP: c_uint = 14;
pub const DC_SAMPLE_EVENT_CEILING_SAFETYSTOP: c_uint = 15;
pub const DC_SAMPLE_EVENT_FLOOR: c_uint = 16;
pub const DC_SAMPLE_EVENT_DIVETIME: c_uint = 17;
pub const DC_SAMPLE_EVENT_MAXDEPTH: c_uint = 18;
pub const DC_SAMPLE_EVENT_OLF: c_uint = 19;
pub const DC_SAMPLE_EVENT_PO2: c_uint = 20;
pub const DC_SAMPLE_EVENT_AIRTIME: c_uint = 21;
pub const DC_SAMPLE_EVENT_RGBM: c_uint = 22;
pub const DC_SAMPLE_EVENT_HEADING: c_uint = 23;
pub const DC_SAMPLE_EVENT_TISSUELEVEL: c_uint = 24;
pub const DC_SAMPLE_EVENT_GASCHANGE2: c_uint = 25;
pub const DC_SAMPLE_EVENT_STRING: c_uint = 26;

// -- Structs ---------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_datetime_t {
    pub year: c_int,
    pub month: c_int,
    pub day: c_int,
    pub hour: c_int,
    pub minute: c_int,
    pub second: c_int,
    pub timezone: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_gasmix_t {
    pub helium: c_double,
    pub oxygen: c_double,
    pub nitrogen: c_double,
    pub usage: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_salinity_t {
    pub water: c_uint,
    pub density: c_double,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_tank_t {
    pub gasmix: c_uint,
    pub type_: c_uint,
    pub volume: c_double,
    pub workpressure: c_double,
    pub beginpressure: c_double,
    pub endpressure: c_double,
    pub usage: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_gf_t {
    pub high: c_uint,
    pub low: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union dc_decomodel_params_t {
    pub gf: dc_gf_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_decomodel_t {
    pub type_: c_uint,
    pub conservatism: c_int,
    pub params: dc_decomodel_params_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_event_devinfo_t {
    pub model: c_uint,
    pub firmware: c_uint,
    pub serial: c_uint,
    pub hw_id: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_location_t {
    pub latitude: c_double,
    pub longitude: c_double,
    pub altitude: c_double,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_sample_pressure_t {
    pub tank: c_uint,
    pub value: c_double,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_sample_event_t {
    pub type_: c_uint,
    pub time: c_uint,
    pub flags: c_uint,
    pub value: c_uint,
    pub name: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_sample_vendor_t {
    pub type_: c_uint,
    pub size: c_uint,
    pub data: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_sample_ppo2_t {
    pub sensor: c_uint,
    pub value: c_double,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_sample_deco_t {
    pub type_: c_uint,
    pub time: c_uint,
    pub depth: c_double,
    pub tts: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union dc_sample_value_t {
    pub time: c_uint,
    pub depth: c_double,
    pub pressure: dc_sample_pressure_t,
    pub temperature: c_double,
    pub event: dc_sample_event_t,
    pub rbt: c_uint,
    pub heartbeat: c_uint,
    pub bearing: c_uint,
    pub vendor: dc_sample_vendor_t,
    pub setpoint: c_double,
    pub ppo2: dc_sample_ppo2_t,
    pub cns: c_double,
    pub deco: dc_sample_deco_t,
    pub gasmix: c_uint,
    pub location: dc_location_t,
}

pub type dc_sample_callback_t =
    extern "C" fn(type_: c_int, value: *const dc_sample_value_t, userdata: *mut c_void);

// -- Functions -------------------------------------------------------------

extern "C" {
    pub fn dc_context_new(context: *mut *mut dc_context_t) -> c_int;
    pub fn dc_context_free(context: *mut dc_context_t) -> c_int;
    pub fn dc_context_set_loglevel(context: *mut dc_context_t, level: c_int) -> c_int;

    pub fn dc_descriptor_iterator_new(
        iterator: *mut *mut dc_iterator_t,
        context: *mut dc_context_t,
    ) -> c_int;
    pub fn dc_iterator_next(iterator: *mut dc_iterator_t, item: *mut c_void) -> c_int;
    pub fn dc_iterator_free(iterator: *mut dc_iterator_t) -> c_int;
    pub fn dc_descriptor_free(descriptor: *mut dc_descriptor_t);
    pub fn dc_descriptor_get_vendor(descriptor: *const dc_descriptor_t) -> *const c_char;
    pub fn dc_descriptor_get_product(descriptor: *const dc_descriptor_t) -> *const c_char;
    pub fn dc_descriptor_get_type(descriptor: *const dc_descriptor_t) -> c_uint;
    pub fn dc_descriptor_get_model(descriptor: *const dc_descriptor_t) -> c_uint;
    pub fn dc_descriptor_get_transports(descriptor: *const dc_descriptor_t) -> c_uint;

    pub fn dc_parser_new2(
        parser: *mut *mut dc_parser_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
        data: *const u8,
        size: usize,
    ) -> c_int;
    pub fn dc_parser_get_datetime(parser: *mut dc_parser_t, datetime: *mut dc_datetime_t) -> c_int;
    pub fn dc_parser_get_device_info(
        parser: *mut dc_parser_t,
        devinfo: *mut dc_event_devinfo_t,
    ) -> c_int;
    pub fn dc_parser_get_field(
        parser: *mut dc_parser_t,
        type_: c_int,
        flags: c_uint,
        value: *mut c_void,
    ) -> c_int;
    pub fn dc_parser_samples_foreach(
        parser: *mut dc_parser_t,
        callback: dc_sample_callback_t,
        userdata: *mut c_void,
    ) -> c_int;
    pub fn dc_parser_destroy(parser: *mut dc_parser_t) -> c_int;

    pub fn dc_datetime_mktime(datetime: *const dc_datetime_t) -> i64;
}

// -- Devices, I/O streams and transports -----------------------------------

pub enum dc_iostream_t {}
pub enum dc_serial_device_t {}
pub enum dc_usb_device_t {}
pub enum dc_usbhid_device_t {}
pub enum dc_bluetooth_device_t {}

pub type dc_bluetooth_address_t = u64;

pub const DC_EVENT_WAITING: c_uint = 1 << 0;
pub const DC_EVENT_PROGRESS: c_uint = 1 << 1;
pub const DC_EVENT_DEVINFO: c_uint = 1 << 2;
pub const DC_EVENT_CLOCK: c_uint = 1 << 3;
pub const DC_EVENT_VENDOR: c_uint = 1 << 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_event_progress_t {
    pub current: c_uint,
    pub maximum: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct dc_event_clock_t {
    pub devtime: c_uint,
    pub systime: i64,
}

pub type dc_cancel_callback_t = extern "C" fn(userdata: *mut c_void) -> c_int;
pub type dc_event_callback_t = extern "C" fn(
    device: *mut dc_device_t,
    event: c_uint,
    data: *const c_void,
    userdata: *mut c_void,
);
pub type dc_dive_callback_t = extern "C" fn(
    data: *const u8,
    size: c_uint,
    fingerprint: *const u8,
    fsize: c_uint,
    userdata: *mut c_void,
) -> c_int;

extern "C" {
    // Device
    pub fn dc_device_open(
        device: *mut *mut dc_device_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
        iostream: *mut dc_iostream_t,
    ) -> c_int;
    pub fn dc_device_get_type(device: *mut dc_device_t) -> c_uint;
    pub fn dc_device_set_cancel(
        device: *mut dc_device_t,
        callback: dc_cancel_callback_t,
        userdata: *mut c_void,
    ) -> c_int;
    pub fn dc_device_set_events(
        device: *mut dc_device_t,
        events: c_uint,
        callback: dc_event_callback_t,
        userdata: *mut c_void,
    ) -> c_int;
    pub fn dc_device_set_fingerprint(
        device: *mut dc_device_t,
        data: *const u8,
        size: c_uint,
    ) -> c_int;
    pub fn dc_device_foreach(
        device: *mut dc_device_t,
        callback: dc_dive_callback_t,
        userdata: *mut c_void,
    ) -> c_int;
    pub fn dc_device_close(device: *mut dc_device_t) -> c_int;

    // I/O stream
    pub fn dc_iostream_close(iostream: *mut dc_iostream_t) -> c_int;

    // Serial
    pub fn dc_serial_iterator_new(
        iterator: *mut *mut dc_iterator_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
    ) -> c_int;
    pub fn dc_serial_device_get_name(device: *mut dc_serial_device_t) -> *const c_char;
    pub fn dc_serial_device_free(device: *mut dc_serial_device_t);
    pub fn dc_serial_open(
        iostream: *mut *mut dc_iostream_t,
        context: *mut dc_context_t,
        name: *const c_char,
    ) -> c_int;

    // USB
    pub fn dc_usb_iterator_new(
        iterator: *mut *mut dc_iterator_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
    ) -> c_int;
    pub fn dc_usb_device_get_vid(device: *mut dc_usb_device_t) -> c_uint;
    pub fn dc_usb_device_get_pid(device: *mut dc_usb_device_t) -> c_uint;
    pub fn dc_usb_device_free(device: *mut dc_usb_device_t);
    pub fn dc_usb_open(
        iostream: *mut *mut dc_iostream_t,
        context: *mut dc_context_t,
        device: *mut dc_usb_device_t,
    ) -> c_int;

    // USB HID
    pub fn dc_usbhid_iterator_new(
        iterator: *mut *mut dc_iterator_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
    ) -> c_int;
    pub fn dc_usbhid_device_get_vid(device: *mut dc_usbhid_device_t) -> c_uint;
    pub fn dc_usbhid_device_get_pid(device: *mut dc_usbhid_device_t) -> c_uint;
    pub fn dc_usbhid_device_free(device: *mut dc_usbhid_device_t);
    pub fn dc_usbhid_open(
        iostream: *mut *mut dc_iostream_t,
        context: *mut dc_context_t,
        device: *mut dc_usbhid_device_t,
    ) -> c_int;

    // Bluetooth
    pub fn dc_bluetooth_iterator_new(
        iterator: *mut *mut dc_iterator_t,
        context: *mut dc_context_t,
        descriptor: *mut dc_descriptor_t,
    ) -> c_int;
    pub fn dc_bluetooth_device_get_address(
        device: *mut dc_bluetooth_device_t,
    ) -> dc_bluetooth_address_t;
    pub fn dc_bluetooth_device_get_name(device: *mut dc_bluetooth_device_t) -> *const c_char;
    pub fn dc_bluetooth_device_free(device: *mut dc_bluetooth_device_t);
    pub fn dc_bluetooth_open(
        iostream: *mut *mut dc_iostream_t,
        context: *mut dc_context_t,
        address: dc_bluetooth_address_t,
        port: c_uint,
    ) -> c_int;
}

// -- Custom I/O streams (custom.h) -----------------------------------------
//
// The BLE ioctl numbers are `DC_IOCTL_IOR/IOW('b', n, 0)`, computed with the
// same formula as ioctl.h so they match the compiled library.

pub const DC_IOCTL_BLE_GET_NAME: c_uint = 0x4000_6200;
pub const DC_IOCTL_BLE_GET_PINCODE: c_uint = 0x4000_6201;
pub const DC_IOCTL_BLE_GET_ACCESSCODE: c_uint = 0x4000_6202;
pub const DC_IOCTL_BLE_SET_ACCESSCODE: c_uint = 0x8000_6202;
pub const DC_IOCTL_BLE_CHARACTERISTIC_READ: c_uint = 0x4000_6203;

pub type dc_custom_fn0 = extern "C" fn(*mut c_void) -> c_int;
pub type dc_custom_fn_u32 = extern "C" fn(*mut c_void, c_uint) -> c_int;
pub type dc_custom_fn_timeout = extern "C" fn(*mut c_void, c_int) -> c_int;
pub type dc_custom_fn_lines = extern "C" fn(*mut c_void, *mut c_uint) -> c_int;
pub type dc_custom_fn_available = extern "C" fn(*mut c_void, *mut usize) -> c_int;
pub type dc_custom_fn_configure =
    extern "C" fn(*mut c_void, c_uint, c_uint, c_uint, c_uint, c_uint) -> c_int;
pub type dc_custom_fn_poll = extern "C" fn(*mut c_void, c_int) -> c_int;
pub type dc_custom_fn_read = extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize) -> c_int;
pub type dc_custom_fn_write = extern "C" fn(*mut c_void, *const c_void, usize, *mut usize) -> c_int;
pub type dc_custom_fn_ioctl = extern "C" fn(*mut c_void, c_uint, *mut c_void, usize) -> c_int;
pub type dc_custom_fn_direction = extern "C" fn(*mut c_void, c_uint) -> c_int;

#[repr(C)]
pub struct dc_custom_cbs_t {
    pub set_timeout: Option<dc_custom_fn_timeout>,
    pub set_break: Option<dc_custom_fn_u32>,
    pub set_dtr: Option<dc_custom_fn_u32>,
    pub set_rts: Option<dc_custom_fn_u32>,
    pub get_lines: Option<dc_custom_fn_lines>,
    pub get_available: Option<dc_custom_fn_available>,
    pub configure: Option<dc_custom_fn_configure>,
    pub poll: Option<dc_custom_fn_poll>,
    pub read: Option<dc_custom_fn_read>,
    pub write: Option<dc_custom_fn_write>,
    pub ioctl: Option<dc_custom_fn_ioctl>,
    pub flush: Option<dc_custom_fn0>,
    pub purge: Option<dc_custom_fn_direction>,
    pub sleep: Option<dc_custom_fn_u32>,
    pub close: Option<dc_custom_fn0>,
}

extern "C" {
    pub fn dc_custom_open(
        iostream: *mut *mut dc_iostream_t,
        context: *mut dc_context_t,
        transport: c_uint,
        callbacks: *const dc_custom_cbs_t,
        userdata: *mut c_void,
    ) -> c_int;
}
