//! Safe wrappers over libdivecomputer: device discovery and dump parsing.
//!
//! A raw memory dump (as produced by `dc_device_dump`, or by the fixtures in
//! `vendor/libdivecomputer/test/fixtures`) can be turned into a
//! [`benthic_core::Dive`] without any hardware attached, which is what the
//! tests exercise.

use std::ffi::CStr;
use std::os::raw::{c_char, c_double, c_int, c_uint, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null_mut;

use benthic_core::divecomputer::{
    RawDateTime, RawDecoKind, RawDeviceInfo, RawDive, RawDivemode, RawGasMix, RawGradientFactors,
    RawSample, RawTank, RawUsage,
};
use benthic_core::Dive;

use crate::error::{check, Error};
use crate::ffi::*;

/// A dive computer model known to libdivecomputer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub vendor: String,
    pub product: String,
    /// libdivecomputer family id (opaque to callers).
    pub family: u32,
    /// Model number within the family.
    pub model: u32,
    /// Bitmask of [`Transport`] values the device can use.
    pub transports: u32,
}

impl DeviceDescriptor {
    /// "Vendor Product", the way a dive computer is normally named.
    pub fn name(&self) -> String {
        format!("{} {}", self.vendor, self.product)
    }

    /// The transports this model supports, in display order.
    pub fn transport_list(&self) -> Vec<Transport> {
        Transport::ALL
            .into_iter()
            .filter(|t| self.transports & t.bit() != 0)
            .collect()
    }
}

/// How a dive computer can be connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Serial,
    Usb,
    UsbHid,
    Infrared,
    Bluetooth,
    Ble,
    UsbStorage,
}

impl Transport {
    pub const ALL: [Transport; 7] = [
        Transport::Serial,
        Transport::Usb,
        Transport::UsbHid,
        Transport::Infrared,
        Transport::Bluetooth,
        Transport::Ble,
        Transport::UsbStorage,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            Transport::Serial => DC_TRANSPORT_SERIAL,
            Transport::Usb => DC_TRANSPORT_USB,
            Transport::UsbHid => DC_TRANSPORT_USBHID,
            Transport::Infrared => DC_TRANSPORT_IRDA,
            Transport::Bluetooth => DC_TRANSPORT_BLUETOOTH,
            Transport::Ble => DC_TRANSPORT_BLE,
            Transport::UsbStorage => DC_TRANSPORT_USBSTORAGE,
        }
    }

    /// A lowercase label for the UI.
    pub const fn label(self) -> &'static str {
        match self {
            Transport::Serial => "serial",
            Transport::Usb => "usb",
            Transport::UsbHid => "usbhid",
            Transport::Infrared => "irda",
            Transport::Bluetooth => "bluetooth",
            Transport::Ble => "ble",
            Transport::UsbStorage => "usb storage",
        }
    }
}

/// Every dive computer model libdivecomputer supports.
pub fn descriptors() -> Result<Vec<DeviceDescriptor>, Error> {
    unsafe {
        let mut context: *mut dc_context_t = null_mut();
        check(dc_context_new(&mut context))?;
        let result = descriptors_with(context);
        dc_context_free(context);
        result
    }
}

unsafe fn descriptors_with(context: *mut dc_context_t) -> Result<Vec<DeviceDescriptor>, Error> {
    let mut iterator: *mut dc_iterator_t = null_mut();
    check(dc_descriptor_iterator_new(&mut iterator, context))?;

    let mut out = Vec::new();
    loop {
        let mut descriptor: *mut dc_descriptor_t = null_mut();
        if dc_iterator_next(iterator, &mut descriptor as *mut _ as *mut c_void) != DC_STATUS_SUCCESS
        {
            break;
        }
        out.push(read_descriptor(descriptor));
        dc_descriptor_free(descriptor);
    }
    dc_iterator_free(iterator);
    Ok(out)
}

pub(crate) unsafe fn read_descriptor(descriptor: *const dc_descriptor_t) -> DeviceDescriptor {
    let string = |p: *const c_char| {
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    DeviceDescriptor {
        vendor: string(dc_descriptor_get_vendor(descriptor)),
        product: string(dc_descriptor_get_product(descriptor)),
        family: dc_descriptor_get_type(descriptor),
        model: dc_descriptor_get_model(descriptor),
        transports: dc_descriptor_get_transports(descriptor),
    }
}

/// Parse a raw dive-computer memory dump for a known model into a [`Dive`].
///
/// `vendor`/`product` must match one of the descriptors returned by
/// [`descriptors`]; the fixture manifest in the vendored library uses the same
/// names.
pub fn parse_dump(vendor: &str, product: &str, data: &[u8]) -> Result<Dive, Error> {
    unsafe {
        let mut context: *mut dc_context_t = null_mut();
        check(dc_context_new(&mut context))?;
        let result = match find_descriptor(context, vendor, product) {
            Ok(descriptor) => {
                let result = parse_with(context, descriptor, data);
                dc_descriptor_free(descriptor);
                result
            }
            Err(e) => Err(e),
        };
        dc_context_free(context);
        result
    }
}

/// Find the libdivecomputer descriptor for a model. The caller owns the
/// returned pointer and must release it with `dc_descriptor_free`.
pub(crate) unsafe fn find_descriptor(
    context: *mut dc_context_t,
    vendor: &str,
    product: &str,
) -> Result<*mut dc_descriptor_t, Error> {
    let mut iterator: *mut dc_iterator_t = null_mut();
    check(dc_descriptor_iterator_new(&mut iterator, context))?;

    let mut found: *mut dc_descriptor_t = null_mut();
    loop {
        let mut descriptor: *mut dc_descriptor_t = null_mut();
        if dc_iterator_next(iterator, &mut descriptor as *mut _ as *mut c_void) != DC_STATUS_SUCCESS
        {
            break;
        }
        let info = read_descriptor(descriptor);
        if info.vendor == vendor && info.product == product {
            found = descriptor;
            break;
        }
        dc_descriptor_free(descriptor);
    }
    dc_iterator_free(iterator);

    if found.is_null() {
        Err(Error::UnknownDevice {
            vendor: vendor.to_string(),
            product: product.to_string(),
        })
    } else {
        Ok(found)
    }
}

pub(crate) unsafe fn parse_with(
    context: *mut dc_context_t,
    descriptor: *mut dc_descriptor_t,
    data: &[u8],
) -> Result<Dive, Error> {
    let mut parser: *mut dc_parser_t = null_mut();
    check(dc_parser_new2(
        &mut parser,
        context,
        descriptor,
        data.as_ptr(),
        data.len(),
    ))?;

    let result = parse_parser(parser, descriptor);
    dc_parser_destroy(parser);
    result
}

pub(crate) unsafe fn parse_parser(
    parser: *mut dc_parser_t,
    descriptor: *const dc_descriptor_t,
) -> Result<Dive, Error> {
    let info = read_descriptor(descriptor);
    let mut raw = RawDive {
        vendor: info.vendor,
        product: info.product,
        ..Default::default()
    };

    let mut datetime = dc_datetime_t {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        timezone: DC_TIMEZONE_NONE,
    };
    if dc_parser_get_datetime(parser, &mut datetime) == DC_STATUS_SUCCESS {
        raw.datetime = Some(RawDateTime {
            year: datetime.year,
            month: datetime.month as u8,
            day: datetime.day as u8,
            hour: datetime.hour as u8,
            minute: datetime.minute as u8,
            second: datetime.second as u8,
            timezone: (datetime.timezone != DC_TIMEZONE_NONE).then_some(datetime.timezone),
        });
    }

    raw.divetime = field_uint(parser, DC_FIELD_DIVETIME);
    raw.max_depth = field_double(parser, DC_FIELD_MAXDEPTH);
    raw.mean_depth = field_double(parser, DC_FIELD_AVGDEPTH);
    raw.atmospheric = field_double(parser, DC_FIELD_ATMOSPHERIC);
    raw.temperature_surface = field_double(parser, DC_FIELD_TEMPERATURE_SURFACE);
    raw.temperature_min = field_double(parser, DC_FIELD_TEMPERATURE_MINIMUM);
    raw.divemode = field_uint(parser, DC_FIELD_DIVEMODE).map(divemode_of);

    let mut salinity = dc_salinity_t {
        water: 0,
        density: 0.0,
    };
    if dc_parser_get_field(
        parser,
        DC_FIELD_SALINITY,
        0,
        &mut salinity as *mut _ as *mut c_void,
    ) == DC_STATUS_SUCCESS
        && salinity.density > 0.0
    {
        raw.salinity_density = Some(salinity.density);
    }

    let mut decomodel = dc_decomodel_t {
        type_: 0,
        conservatism: 0,
        params: dc_decomodel_params_t {
            gf: dc_gf_t { high: 0, low: 0 },
        },
    };
    if dc_parser_get_field(
        parser,
        DC_FIELD_DECOMODEL,
        0,
        &mut decomodel as *mut _ as *mut c_void,
    ) == DC_STATUS_SUCCESS
    {
        let gf = decomodel.params.gf;
        if gf.low > 0 || gf.high > 0 {
            raw.gf = Some(RawGradientFactors {
                low: gf.low,
                high: gf.high,
            });
        }
    }

    let mut devinfo = dc_event_devinfo_t {
        model: 0,
        firmware: 0,
        serial: 0,
        hw_id: 0,
    };
    if dc_parser_get_device_info(parser, &mut devinfo) == DC_STATUS_SUCCESS {
        raw.info = Some(RawDeviceInfo {
            model: devinfo.model,
            firmware: devinfo.firmware,
            serial: devinfo.serial,
            hw_id: devinfo.hw_id,
        });
    }

    raw.gasmixes = read_gasmixes(parser);
    raw.tanks = read_tanks(parser);

    let mut collector = SampleAccumulator::default();
    check(dc_parser_samples_foreach(
        parser,
        sample_callback,
        &mut collector as *mut _ as *mut c_void,
    ))?;
    raw.samples = collector.samples;

    Ok(raw.to_dive())
}

unsafe fn read_gasmixes(parser: *mut dc_parser_t) -> Vec<RawGasMix> {
    let count = field_uint(parser, DC_FIELD_GASMIX_COUNT).unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            let mut mix = dc_gasmix_t {
                helium: 0.0,
                oxygen: 0.0,
                nitrogen: 0.0,
                usage: DC_USAGE_NONE,
            };
            let status = dc_parser_get_field(
                parser,
                DC_FIELD_GASMIX,
                i,
                &mut mix as *mut _ as *mut c_void,
            );
            (status == DC_STATUS_SUCCESS).then(|| RawGasMix {
                oxygen: mix.oxygen,
                helium: mix.helium,
                usage: raw_usage(mix.usage),
            })
        })
        .collect()
}

unsafe fn read_tanks(parser: *mut dc_parser_t) -> Vec<RawTank> {
    let count = field_uint(parser, DC_FIELD_TANK_COUNT).unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            let mut tank = dc_tank_t {
                gasmix: DC_GASMIX_UNKNOWN,
                type_: 0,
                volume: 0.0,
                workpressure: 0.0,
                beginpressure: 0.0,
                endpressure: 0.0,
                usage: 0,
            };
            let status =
                dc_parser_get_field(parser, DC_FIELD_TANK, i, &mut tank as *mut _ as *mut c_void);
            (status == DC_STATUS_SUCCESS).then(|| RawTank {
                gasmix: (tank.gasmix != DC_GASMIX_UNKNOWN).then_some(tank.gasmix),
                volume: tank.volume,
                workpressure: tank.workpressure,
                beginpressure: tank.beginpressure,
                endpressure: tank.endpressure,
            })
        })
        .collect()
}

unsafe fn field_uint(parser: *mut dc_parser_t, field: c_int) -> Option<c_uint> {
    let mut value: c_uint = 0;
    (dc_parser_get_field(parser, field, 0, &mut value as *mut _ as *mut c_void)
        == DC_STATUS_SUCCESS)
        .then_some(value)
}

unsafe fn field_double(parser: *mut dc_parser_t, field: c_int) -> Option<c_double> {
    let mut value: c_double = 0.0;
    (dc_parser_get_field(parser, field, 0, &mut value as *mut _ as *mut c_void)
        == DC_STATUS_SUCCESS)
        .then_some(value)
}

fn divemode_of(mode: c_uint) -> RawDivemode {
    match mode {
        DC_DIVEMODE_FREEDIVE => RawDivemode::Freedive,
        DC_DIVEMODE_GAUGE => RawDivemode::Gauge,
        DC_DIVEMODE_CCR => RawDivemode::Ccr,
        DC_DIVEMODE_SCR => RawDivemode::Scr,
        _ => RawDivemode::Oc,
    }
}

fn raw_usage(usage: c_uint) -> RawUsage {
    match usage {
        DC_USAGE_OXYGEN => RawUsage::Oxygen,
        DC_USAGE_DILUENT => RawUsage::Diluent,
        DC_USAGE_OPEN_CIRCUIT => RawUsage::OpenCircuit,
        _ => RawUsage::None,
    }
}

fn deco_kind(kind: c_uint) -> RawDecoKind {
    match kind {
        DC_DECO_NDL => RawDecoKind::Ndl,
        DC_DECO_SAFETYSTOP => RawDecoKind::SafetyStop,
        DC_DECO_DEEPSTOP => RawDecoKind::DeepStop,
        _ => RawDecoKind::DecoStop,
    }
}

/// Collects libdivecomputer's flat sample stream into the neutral records.
#[derive(Default)]
struct SampleAccumulator {
    samples: Vec<RawSample>,
}

extern "C" fn sample_callback(kind: c_int, value: *const dc_sample_value_t, userdata: *mut c_void) {
    // A panic must never unwind across the C boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if value.is_null() || userdata.is_null() {
            return;
        }
        let accumulator = unsafe { &mut *(userdata as *mut SampleAccumulator) };
        let value = unsafe { &*value };
        if let Some(sample) = unsafe { raw_sample(kind, value) } {
            accumulator.samples.push(sample);
        }
    }));
}

/// Safety: `value` is read only for the variant matching `kind`, which is the
/// contract of the C callback.
unsafe fn raw_sample(kind: c_int, value: &dc_sample_value_t) -> Option<RawSample> {
    Some(match kind {
        DC_SAMPLE_TIME => RawSample::Time { ms: value.time },
        DC_SAMPLE_DEPTH => RawSample::Depth { m: value.depth },
        DC_SAMPLE_PRESSURE => {
            let reading = value.pressure;
            RawSample::Pressure {
                tank: reading.tank,
                bar: reading.value,
            }
        }
        DC_SAMPLE_TEMPERATURE => RawSample::Temperature {
            c: value.temperature,
        },
        DC_SAMPLE_SETPOINT => RawSample::Setpoint {
            bar: value.setpoint,
        },
        DC_SAMPLE_PPO2 => {
            let reading = value.ppo2;
            RawSample::Ppo2 {
                sensor: reading.sensor,
                bar: reading.value,
            }
        }
        DC_SAMPLE_CNS => RawSample::Cns {
            fraction: value.cns,
        },
        DC_SAMPLE_RBT => RawSample::Rbt { s: value.rbt },
        DC_SAMPLE_HEARTBEAT => RawSample::Heartbeat {
            bpm: value.heartbeat,
        },
        DC_SAMPLE_BEARING => RawSample::Bearing { deg: value.bearing },
        DC_SAMPLE_TTS => RawSample::Tts { s: value.time },
        DC_SAMPLE_DECO => {
            let deco = value.deco;
            RawSample::Deco {
                kind: deco_kind(deco.type_),
                s: deco.time,
                m: deco.depth,
                tts: deco.tts,
            }
        }
        DC_SAMPLE_GASMIX => {
            if value.gasmix == DC_GASMIX_UNKNOWN {
                return None;
            }
            RawSample::Gasmix {
                index: value.gasmix,
            }
        }
        DC_SAMPLE_EVENT => {
            let event = value.event;
            let name = if event.name.is_null() {
                None
            } else {
                Some(CStr::from_ptr(event.name).to_string_lossy().into_owned())
            };
            RawSample::Event {
                kind: event.type_,
                ms: event.time,
                flags: event.flags,
                value: event.value as i32,
                name,
            }
        }
        _ => return None,
    })
}
