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

use benthic_core::{
    Bearing, Cylinder, CylinderUse, Depth, Dive, DiveComputer, Divemode, Duration, Event, GasMix,
    O2Pressure, Pressure, Sample, SensorPressure, Temperature, Volume,
};

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
    let mut datetime = dc_datetime_t {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        timezone: DC_TIMEZONE_NONE,
    };
    let have_datetime = dc_parser_get_datetime(parser, &mut datetime) == DC_STATUS_SUCCESS;

    let mut devinfo = dc_event_devinfo_t {
        model: 0,
        firmware: 0,
        serial: 0,
        hw_id: 0,
    };
    let have_devinfo = dc_parser_get_device_info(parser, &mut devinfo) == DC_STATUS_SUCCESS;

    let gasmixes = read_gasmixes(parser);
    let tanks = read_tanks(parser);

    let mut collector = Collector {
        gasmixes: &gasmixes,
        samples: Vec::new(),
        events: Vec::new(),
        current: None,
    };
    check(dc_parser_samples_foreach(
        parser,
        sample_callback,
        &mut collector as *mut _ as *mut c_void,
    ))?;
    collector.finish();

    let info = read_descriptor(descriptor);
    Ok(build_dive(
        &info,
        &datetime,
        have_datetime,
        have_devinfo.then_some(&devinfo),
        parser,
        &gasmixes,
        &tanks,
        collector,
    ))
}

unsafe fn read_gasmixes(parser: *mut dc_parser_t) -> Vec<(GasMix, c_uint)> {
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
            (status == DC_STATUS_SUCCESS).then(|| {
                (
                    GasMix::new(
                        (mix.oxygen * 1000.0).round() as u16,
                        (mix.helium * 1000.0).round() as u16,
                    ),
                    mix.usage,
                )
            })
        })
        .collect()
}

unsafe fn read_tanks(parser: *mut dc_parser_t) -> Vec<dc_tank_t> {
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
            (status == DC_STATUS_SUCCESS).then_some(tank)
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

#[allow(clippy::too_many_arguments)]
unsafe fn build_dive(
    info: &DeviceDescriptor,
    datetime: &dc_datetime_t,
    have_datetime: bool,
    devinfo: Option<&dc_event_devinfo_t>,
    parser: *mut dc_parser_t,
    gasmixes: &[(GasMix, c_uint)],
    tanks: &[dc_tank_t],
    collector: Collector,
) -> Dive {
    let mut computer = DiveComputer {
        model: info.name(),
        ..Default::default()
    };

    if let Some(seconds) = field_uint(parser, DC_FIELD_DIVETIME) {
        computer.duration = Some(Duration::new(seconds as i32));
    }
    if let Some(meters) = field_double(parser, DC_FIELD_MAXDEPTH) {
        computer.max_depth = Some(Depth::from_meters(meters));
    }
    if let Some(meters) = field_double(parser, DC_FIELD_AVGDEPTH) {
        computer.mean_depth = Some(Depth::from_meters(meters));
    }
    if let Some(bar) = field_double(parser, DC_FIELD_ATMOSPHERIC) {
        computer.surface_pressure = Some(Pressure::from_bar(bar));
    }
    if let Some(celsius) = field_double(parser, DC_FIELD_TEMPERATURE_SURFACE) {
        computer.air_temp = Some(Temperature::from_celsius(celsius));
    }
    if let Some(celsius) = field_double(parser, DC_FIELD_TEMPERATURE_MINIMUM) {
        computer.water_temp = Some(Temperature::from_celsius(celsius));
    }
    if let Some(usage) = field_uint(parser, DC_FIELD_DIVEMODE) {
        computer.divemode = divemode_of(usage);
    }
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
        // libdivecomputer reports density in kg/m^3; the model stores
        // grams of salt per 10 litres (so 1020 kg/m^3 -> 10200).
        computer.salinity = Some((salinity.density * 10.0).round() as i32);
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
            computer
                .extra_data
                .push(("GF".to_string(), format!("{}/{}", gf.low, gf.high)));
        }
    }
    if have_datetime && datetime.timezone != DC_TIMEZONE_NONE {
        computer.timezone_offset = Some(datetime.timezone);
    }
    if let Some(devinfo) = devinfo {
        computer.device_id = devinfo.serial;
        computer.firmware = Some(devinfo.firmware.to_string());
        if devinfo.serial != 0 {
            computer.serial = Some(devinfo.serial.to_string());
        }
    }
    computer.samples = collector.samples;
    computer.events = collector.events;

    let cylinders = build_cylinders(gasmixes, tanks);
    let when = if have_datetime {
        dc_datetime_mktime(datetime)
    } else {
        0
    };

    Dive {
        when,
        cylinders,
        computers: vec![computer],
        ..Default::default()
    }
}

fn build_cylinders(gasmixes: &[(GasMix, c_uint)], tanks: &[dc_tank_t]) -> Vec<Cylinder> {
    let mix_of = |index: c_uint| -> Option<&(GasMix, c_uint)> {
        if index == DC_GASMIX_UNKNOWN {
            None
        } else {
            gasmixes.get(index as usize)
        }
    };

    if tanks.is_empty() {
        return gasmixes
            .iter()
            .map(|(gas, usage)| Cylinder {
                gas: *gas,
                use_: cylinder_use(*usage),
                ..Default::default()
            })
            .collect();
    }

    tanks
        .iter()
        .map(|tank| {
            let mix = mix_of(tank.gasmix);
            Cylinder {
                size: (tank.volume > 0.0).then(|| Volume::from_liters(tank.volume)),
                working_pressure: (tank.workpressure > 0.0)
                    .then(|| Pressure::from_bar(tank.workpressure)),
                gas: mix.map(|(gas, _)| *gas).unwrap_or_default(),
                start_pressure: (tank.beginpressure > 0.0)
                    .then(|| Pressure::from_bar(tank.beginpressure)),
                end_pressure: (tank.endpressure > 0.0)
                    .then(|| Pressure::from_bar(tank.endpressure)),
                use_: mix
                    .map(|(_, usage)| cylinder_use(*usage))
                    .unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect()
}

fn divemode_of(mode: c_uint) -> Divemode {
    match mode {
        DC_DIVEMODE_FREEDIVE => Divemode::Freedive,
        DC_DIVEMODE_CCR => Divemode::Ccr,
        DC_DIVEMODE_SCR => Divemode::Pscr,
        _ => Divemode::OpenCircuit,
    }
}

fn cylinder_use(usage: c_uint) -> CylinderUse {
    match usage {
        DC_USAGE_OXYGEN => CylinderUse::Oxygen,
        DC_USAGE_DILUENT => CylinderUse::Diluent,
        DC_USAGE_OPEN_CIRCUIT => CylinderUse::OcGas,
        _ => CylinderUse::OcGas,
    }
}

/// Accumulates the flat libdivecomputer sample stream into per-time samples.
struct Collector<'a> {
    gasmixes: &'a [(GasMix, c_uint)],
    samples: Vec<Sample>,
    events: Vec<Event>,
    current: Option<Sample>,
}

impl Collector<'_> {
    fn ensure(&mut self) -> &mut Sample {
        self.current.get_or_insert_with(Sample::default)
    }

    fn finish(&mut self) {
        if let Some(sample) = self.current.take() {
            self.samples.push(sample);
        }
    }

    fn time(&self) -> Duration {
        self.current.as_ref().map(|s| s.time).unwrap_or_default()
    }

    fn on_sample(&mut self, kind: c_int, value: &dc_sample_value_t) {
        // Safety: the union is only read for the variant matching `kind`,
        // which is exactly the contract of the C callback.
        unsafe {
            match kind {
                DC_SAMPLE_TIME => {
                    self.finish();
                    self.current = Some(Sample {
                        time: Duration::new((value.time / 1000) as i32),
                        ..Default::default()
                    });
                }
                DC_SAMPLE_DEPTH => {
                    self.ensure().depth = Depth::from_meters(value.depth);
                }
                DC_SAMPLE_TEMPERATURE => {
                    self.ensure().temperature = Some(Temperature::from_celsius(value.temperature));
                }
                DC_SAMPLE_PRESSURE => {
                    let reading = value.pressure;
                    let sensor = reading.tank as i16;
                    let pressure = Pressure::from_bar(reading.value);
                    let sample = self.ensure();
                    match sample.pressures.iter_mut().find(|p| p.sensor == sensor) {
                        Some(existing) => existing.pressure = pressure,
                        None => sample.pressures.push(SensorPressure { sensor, pressure }),
                    }
                }
                DC_SAMPLE_SETPOINT => {
                    self.ensure().setpoint = Some(O2Pressure::from_bar(value.setpoint));
                }
                DC_SAMPLE_PPO2 => {
                    let reading = value.ppo2;
                    let sensor = reading.sensor as usize;
                    let sample = self.ensure();
                    if sample.o2_sensors.len() <= sensor {
                        sample.o2_sensors.resize(sensor + 1, O2Pressure::ZERO);
                    }
                    sample.o2_sensors[sensor] = O2Pressure::from_bar(reading.value);
                }
                DC_SAMPLE_CNS => {
                    self.ensure().cns = Some((value.cns * 100.0).round() as u16);
                }
                DC_SAMPLE_RBT => {
                    self.ensure().rbt = Some(Duration::new(value.rbt as i32));
                }
                DC_SAMPLE_HEARTBEAT => {
                    self.ensure().heartbeat = Some(value.heartbeat as u8);
                }
                DC_SAMPLE_BEARING => {
                    self.ensure().bearing = Some(Bearing::new(value.bearing as i16));
                }
                DC_SAMPLE_TTS => {
                    self.ensure().tts = Some(Duration::new(value.time as i32));
                }
                DC_SAMPLE_DECO => {
                    let deco = value.deco;
                    if deco.type_ == DC_DECO_NDL {
                        self.ensure().ndl = Some(Duration::new(deco.time as i32));
                    } else {
                        let sample = self.ensure();
                        sample.stop_depth = Some(Depth::from_meters(deco.depth));
                        sample.stop_time = Some(Duration::new(deco.time as i32));
                        if deco.type_ == DC_DECO_DECOSTOP {
                            sample.in_deco = true;
                        }
                    }
                }
                DC_SAMPLE_GASMIX => {
                    let index = value.gasmix;
                    if index != DC_GASMIX_UNKNOWN {
                        let time = self.time();
                        let gas = self.gasmixes.get(index as usize).map(|(gas, _)| *gas);
                        self.events.push(Event {
                            time,
                            name: "gaschange".to_string(),
                            gas: gas.map(|gas| (index as i32, gas)),
                            ..Default::default()
                        });
                    }
                }
                DC_SAMPLE_EVENT => {
                    let raw = value.event;
                    let time = if raw.time != 0 {
                        Duration::new((raw.time / 1000) as i32)
                    } else {
                        self.time()
                    };
                    let name = event_name(raw.type_, raw.name);
                    self.events.push(Event {
                        time,
                        name,
                        flags: raw.flags as i32,
                        value: raw.value as i32,
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }
    }
}

extern "C" fn sample_callback(kind: c_int, value: *const dc_sample_value_t, userdata: *mut c_void) {
    // A panic must never unwind across the C boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if value.is_null() || userdata.is_null() {
            return;
        }
        let collector = unsafe { &mut *(userdata as *mut Collector) };
        let value = unsafe { &*value };
        collector.on_sample(kind, value);
    }));
}

fn event_name(kind: c_uint, name: *const c_char) -> String {
    let static_name = match kind {
        DC_SAMPLE_EVENT_DECOSTOP => Some("deco"),
        DC_SAMPLE_EVENT_RBT => Some("rbt"),
        DC_SAMPLE_EVENT_ASCENT => Some("ascent"),
        DC_SAMPLE_EVENT_CEILING => Some("ceiling"),
        DC_SAMPLE_EVENT_WORKLOAD => Some("workload"),
        DC_SAMPLE_EVENT_TRANSMITTER => Some("transmitter"),
        DC_SAMPLE_EVENT_VIOLATION => Some("violation"),
        DC_SAMPLE_EVENT_BOOKMARK => Some("bookmark"),
        DC_SAMPLE_EVENT_SURFACE => Some("surface"),
        DC_SAMPLE_EVENT_SAFETYSTOP => Some("safety stop"),
        DC_SAMPLE_EVENT_GASCHANGE | DC_SAMPLE_EVENT_GASCHANGE2 => Some("gaschange"),
        DC_SAMPLE_EVENT_SAFETYSTOP_VOLUNTARY => Some("safety stop (voluntary)"),
        DC_SAMPLE_EVENT_SAFETYSTOP_MANDATORY => Some("safety stop (mandatory)"),
        DC_SAMPLE_EVENT_DEEPSTOP => Some("deepstop"),
        DC_SAMPLE_EVENT_CEILING_SAFETYSTOP => Some("ceiling-safety"),
        DC_SAMPLE_EVENT_FLOOR => Some("floor"),
        DC_SAMPLE_EVENT_DIVETIME => Some("divetime"),
        DC_SAMPLE_EVENT_MAXDEPTH => Some("maxdepth"),
        DC_SAMPLE_EVENT_OLF => Some("OLF"),
        DC_SAMPLE_EVENT_PO2 => Some("PO2"),
        DC_SAMPLE_EVENT_AIRTIME => Some("airtime"),
        DC_SAMPLE_EVENT_RGBM => Some("RGBM"),
        DC_SAMPLE_EVENT_HEADING => Some("heading"),
        DC_SAMPLE_EVENT_TISSUELEVEL => Some("tissuelevel"),
        _ => None,
    };
    if let Some(name) = static_name {
        return name.to_string();
    }
    if !name.is_null() {
        return unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
    }
    "event".to_string()
}
