//! The neutral dive-computer ingest format.
//!
//! Both the native libdivecomputer FFI and the wasm shim feed the same
//! [`RawDive`] structure into [`RawDive::to_dive`], which is where the model
//! is actually built. Keeping this here (rather than in the FFI crate) means
//! the mapping is platform-free and the web build reuses it verbatim.
//!
//! The field names mirror libdivecomputer's parser fields; the sample event
//! ids are libdivecomputer's `parser_sample_event_t` values, so both producers
//! can pass them straight through.

use serde::{Deserialize, Serialize};

use crate::gas::GasMix;
use crate::model::{Cylinder, CylinderUse, Dive, DiveComputer, Event, Sample, SensorPressure};
use crate::units::{Bearing, Depth, Duration, O2Pressure, Pressure, Temperature, Volume};

/// A dive as extracted from a dive computer, before the domain model is built.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawDive {
    pub vendor: String,
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime: Option<RawDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divetime: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_depth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmospheric: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_surface: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_min: Option<f64>,
    /// Water density in kg/m^3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salinity_density: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divemode: Option<RawDivemode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gf: Option<RawGradientFactors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<RawDeviceInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gasmixes: Vec<RawGasMix>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tanks: Vec<RawTank>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<RawSample>,
}

/// The uncompressed wall-clock time reported by the computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Seconds east of UTC, or `None` when the computer did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<i32>,
}

impl RawDateTime {
    /// Seconds since the Unix epoch, treating the fields as local time when a
    /// timezone is present (matching `dc_datetime_mktime`).
    pub fn to_unix(self) -> i64 {
        use time::{Date, Month, PrimitiveDateTime, Time};
        let Ok(month) = Month::try_from(self.month) else {
            return 0;
        };
        let Ok(date) = Date::from_calendar_date(self.year, month, self.day) else {
            return 0;
        };
        let Ok(time) = Time::from_hms(self.hour, self.minute, self.second) else {
            return 0;
        };
        let unix = PrimitiveDateTime::new(date, time)
            .assume_utc()
            .unix_timestamp();
        match self.timezone {
            Some(offset) => unix - offset as i64,
            None => unix,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawDivemode {
    Freedive,
    Gauge,
    Oc,
    Ccr,
    Scr,
}

impl From<RawDivemode> for crate::model::Divemode {
    fn from(value: RawDivemode) -> Self {
        use crate::model::Divemode;
        match value {
            RawDivemode::Freedive => Divemode::Freedive,
            RawDivemode::Ccr => Divemode::Ccr,
            RawDivemode::Scr => Divemode::Pscr,
            RawDivemode::Gauge | RawDivemode::Oc => Divemode::OpenCircuit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawGradientFactors {
    pub low: u32,
    pub high: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RawDeviceInfo {
    #[serde(default)]
    pub model: u32,
    #[serde(default)]
    pub firmware: u32,
    #[serde(default)]
    pub serial: u32,
    #[serde(default)]
    pub hw_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawGasMix {
    /// Oxygen fraction (0..1).
    pub oxygen: f64,
    /// Helium fraction (0..1).
    pub helium: f64,
    pub usage: RawUsage,
}

impl RawGasMix {
    fn gas(self) -> GasMix {
        // `GasMix` is permille; f64 -> u16 rounding is bounded by the domain.
        GasMix::new(
            (self.oxygen * 1000.0).round() as u16,
            (self.helium * 1000.0).round() as u16,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawUsage {
    None,
    Oxygen,
    Diluent,
    OpenCircuit,
}

impl From<RawUsage> for CylinderUse {
    fn from(value: RawUsage) -> Self {
        match value {
            RawUsage::Oxygen => CylinderUse::Oxygen,
            RawUsage::Diluent => CylinderUse::Diluent,
            RawUsage::OpenCircuit | RawUsage::None => CylinderUse::OcGas,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawTank {
    /// Index into [`RawDive::gasmixes`], or `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gasmix: Option<u32>,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub workpressure: f64,
    #[serde(default)]
    pub beginpressure: f64,
    #[serde(default)]
    pub endpressure: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawDecoKind {
    Ndl,
    SafetyStop,
    DecoStop,
    DeepStop,
}

/// One record of libdivecomputer's flat sample stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RawSample {
    /// A new sample begins; `ms` is milliseconds since the dive started.
    Time {
        ms: u32,
    },
    Depth {
        m: f64,
    },
    Pressure {
        tank: u32,
        bar: f64,
    },
    Temperature {
        c: f64,
    },
    Event {
        kind: u32,
        #[serde(default)]
        ms: u32,
        #[serde(default)]
        flags: u32,
        #[serde(default)]
        value: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Rbt {
        s: u32,
    },
    Heartbeat {
        bpm: u32,
    },
    Bearing {
        deg: u32,
    },
    Setpoint {
        bar: f64,
    },
    Ppo2 {
        sensor: u32,
        bar: f64,
    },
    /// CNS toxicity as a fraction (0..1).
    Cns {
        fraction: f64,
    },
    Deco {
        kind: RawDecoKind,
        s: u32,
        m: f64,
        #[serde(default)]
        tts: u32,
    },
    Gasmix {
        index: u32,
    },
    /// Time to surface, in seconds.
    Tts {
        s: u32,
    },
}

impl RawDive {
    /// "Vendor Product".
    pub fn name(&self) -> String {
        format!("{} {}", self.vendor, self.product)
    }

    /// Build the domain model from the extracted fields.
    pub fn to_dive(&self) -> Dive {
        let mut computer = DiveComputer {
            model: self.name(),
            ..Default::default()
        };

        if let Some(seconds) = self.divetime {
            computer.duration = Some(Duration::new(seconds as i32));
        }
        if let Some(meters) = self.max_depth {
            computer.max_depth = Some(Depth::from_meters(meters));
        }
        if let Some(meters) = self.mean_depth {
            computer.mean_depth = Some(Depth::from_meters(meters));
        }
        if let Some(bar) = self.atmospheric {
            computer.surface_pressure = Some(Pressure::from_bar(bar));
        }
        if let Some(celsius) = self.temperature_surface {
            computer.air_temp = Some(Temperature::from_celsius(celsius));
        }
        if let Some(celsius) = self.temperature_min {
            computer.water_temp = Some(Temperature::from_celsius(celsius));
        }
        if let Some(mode) = self.divemode {
            computer.divemode = mode.into();
        }
        if let Some(density) = self.salinity_density {
            if density > 0.0 {
                // kg/m^3 -> grams of salt per 10 litres (1020 -> 10200).
                computer.salinity = Some((density * 10.0).round() as i32);
            }
        }
        if let Some(gf) = self.gf {
            computer
                .extra_data
                .push(("GF".to_string(), format!("{}/{}", gf.low, gf.high)));
        }
        if let Some(info) = self.info {
            computer.device_id = info.serial;
            computer.firmware = Some(info.firmware.to_string());
            if info.serial != 0 {
                computer.serial = Some(info.serial.to_string());
            }
        }
        if let Some(datetime) = self.datetime {
            computer.timezone_offset = datetime.timezone;
        }

        let mut collector = Collector::new(&self.gasmixes);
        for sample in &self.samples {
            collector.push(sample);
        }
        collector.finish();
        computer.samples = collector.samples;
        computer.events = collector.events;

        Dive {
            when: self.datetime.map(RawDateTime::to_unix).unwrap_or(0),
            cylinders: self.build_cylinders(),
            computers: vec![computer],
            ..Default::default()
        }
    }

    fn build_cylinders(&self) -> Vec<Cylinder> {
        if self.tanks.is_empty() {
            return self
                .gasmixes
                .iter()
                .map(|mix| Cylinder {
                    gas: mix.gas(),
                    use_: mix.usage.into(),
                    ..Default::default()
                })
                .collect();
        }
        self.tanks
            .iter()
            .map(|tank| {
                let mix = tank
                    .gasmix
                    .and_then(|index| self.gasmixes.get(index as usize));
                Cylinder {
                    size: (tank.volume > 0.0).then(|| Volume::from_liters(tank.volume)),
                    working_pressure: (tank.workpressure > 0.0)
                        .then(|| Pressure::from_bar(tank.workpressure)),
                    gas: mix.map(|mix| mix.gas()).unwrap_or_default(),
                    start_pressure: (tank.beginpressure > 0.0)
                        .then(|| Pressure::from_bar(tank.beginpressure)),
                    end_pressure: (tank.endpressure > 0.0)
                        .then(|| Pressure::from_bar(tank.endpressure)),
                    use_: mix.map(|mix| mix.usage.into()).unwrap_or_default(),
                    ..Default::default()
                }
            })
            .collect()
    }
}

/// Replays the flat sample stream into per-time samples and events.
struct Collector<'a> {
    gasmixes: &'a [RawGasMix],
    samples: Vec<Sample>,
    events: Vec<Event>,
    current: Option<Sample>,
}

impl<'a> Collector<'a> {
    fn new(gasmixes: &'a [RawGasMix]) -> Self {
        Self {
            gasmixes,
            samples: Vec::new(),
            events: Vec::new(),
            current: None,
        }
    }

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

    fn push(&mut self, raw: &RawSample) {
        match raw {
            RawSample::Time { ms } => {
                self.finish();
                self.current = Some(Sample {
                    time: Duration::new((ms / 1000) as i32),
                    ..Default::default()
                });
            }
            RawSample::Depth { m } => {
                self.ensure().depth = Depth::from_meters(*m);
            }
            RawSample::Temperature { c } => {
                self.ensure().temperature = Some(Temperature::from_celsius(*c));
            }
            RawSample::Pressure { tank, bar } => {
                let sensor = *tank as i16;
                let pressure = Pressure::from_bar(*bar);
                let sample = self.ensure();
                match sample.pressures.iter_mut().find(|p| p.sensor == sensor) {
                    Some(existing) => existing.pressure = pressure,
                    None => sample.pressures.push(SensorPressure { sensor, pressure }),
                }
            }
            RawSample::Setpoint { bar } => {
                self.ensure().setpoint = Some(O2Pressure::from_bar(*bar));
            }
            RawSample::Ppo2 { sensor, bar } => {
                let sensor = *sensor as usize;
                let sample = self.ensure();
                if sample.o2_sensors.len() <= sensor {
                    sample.o2_sensors.resize(sensor + 1, O2Pressure::ZERO);
                }
                sample.o2_sensors[sensor] = O2Pressure::from_bar(*bar);
            }
            RawSample::Cns { fraction } => {
                self.ensure().cns = Some((fraction * 100.0).round() as u16);
            }
            RawSample::Rbt { s } => {
                self.ensure().rbt = Some(Duration::new(*s as i32));
            }
            RawSample::Heartbeat { bpm } => {
                self.ensure().heartbeat = Some(*bpm as u8);
            }
            RawSample::Bearing { deg } => {
                self.ensure().bearing = Some(Bearing::new(*deg as i16));
            }
            RawSample::Tts { s } => {
                self.ensure().tts = Some(Duration::new(*s as i32));
            }
            RawSample::Deco { kind, s, m, .. } => match kind {
                RawDecoKind::Ndl => self.ensure().ndl = Some(Duration::new(*s as i32)),
                _ => {
                    let sample = self.ensure();
                    sample.stop_depth = Some(Depth::from_meters(*m));
                    sample.stop_time = Some(Duration::new(*s as i32));
                    if *kind == RawDecoKind::DecoStop {
                        sample.in_deco = true;
                    }
                }
            },
            RawSample::Gasmix { index } => {
                let time = self.time();
                let gas = self.gasmixes.get(*index as usize).map(|mix| mix.gas());
                self.events.push(Event {
                    time,
                    name: "gaschange".to_string(),
                    gas: gas.map(|gas| (*index as i32, gas)),
                    ..Default::default()
                });
            }
            RawSample::Event {
                kind,
                ms,
                flags,
                value,
                name,
            } => {
                let time = if *ms != 0 {
                    Duration::new((ms / 1000) as i32)
                } else {
                    self.time()
                };
                self.events.push(Event {
                    time,
                    name: event_name(*kind, name.as_deref()),
                    flags: *flags as i32,
                    value: *value,
                    ..Default::default()
                });
            }
        }
    }
}

/// libdivecomputer `parser_sample_event_t` values, resolved to display names.
fn event_name(kind: u32, name: Option<&str>) -> String {
    let static_name = match kind {
        1 => Some("deco"),
        2 => Some("rbt"),
        3 => Some("ascent"),
        4 => Some("ceiling"),
        5 => Some("workload"),
        6 => Some("transmitter"),
        7 => Some("violation"),
        8 => Some("bookmark"),
        9 => Some("surface"),
        10 => Some("safety stop"),
        11 | 25 => Some("gaschange"),
        12 => Some("safety stop (voluntary)"),
        13 => Some("safety stop (mandatory)"),
        14 => Some("deepstop"),
        15 => Some("ceiling-safety"),
        16 => Some("floor"),
        17 => Some("divetime"),
        18 => Some("maxdepth"),
        19 => Some("OLF"),
        20 => Some("PO2"),
        21 => Some("airtime"),
        22 => Some("RGBM"),
        23 => Some("heading"),
        24 => Some("tissuelevel"),
        _ => None,
    };
    if let Some(name) = static_name {
        return name.to_string();
    }
    name.unwrap_or("event").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Divemode;

    fn sample_dive() -> RawDive {
        RawDive {
            vendor: "Shearwater".into(),
            product: "Petrel 2".into(),
            datetime: Some(RawDateTime {
                year: 2024,
                month: 6,
                day: 1,
                hour: 12,
                minute: 0,
                second: 0,
                timezone: Some(3600),
            }),
            divetime: Some(120),
            max_depth: Some(20.0),
            temperature_min: Some(14.0),
            divemode: Some(RawDivemode::Ccr),
            gf: Some(RawGradientFactors { low: 30, high: 70 }),
            info: Some(RawDeviceInfo {
                serial: 1234,
                firmware: 7,
                ..Default::default()
            }),
            gasmixes: vec![
                RawGasMix {
                    oxygen: 0.21,
                    helium: 0.0,
                    usage: RawUsage::Diluent,
                },
                RawGasMix {
                    oxygen: 1.0,
                    helium: 0.0,
                    usage: RawUsage::Oxygen,
                },
            ],
            tanks: vec![RawTank {
                gasmix: Some(0),
                volume: 12.0,
                workpressure: 200.0,
                beginpressure: 200.0,
                endpressure: 50.0,
            }],
            samples: vec![
                RawSample::Time { ms: 0 },
                RawSample::Depth { m: 0.0 },
                RawSample::Setpoint { bar: 0.7 },
                RawSample::Gasmix { index: 0 },
                RawSample::Time { ms: 1000 },
                RawSample::Depth { m: 10.0 },
                RawSample::Temperature { c: 15.0 },
                RawSample::Pressure {
                    tank: 0,
                    bar: 180.0,
                },
                RawSample::Deco {
                    kind: RawDecoKind::Ndl,
                    s: 600,
                    m: 0.0,
                    tts: 0,
                },
                RawSample::Cns { fraction: 0.03 },
                RawSample::Event {
                    kind: 3,
                    ms: 1500,
                    flags: 4,
                    value: 0,
                    name: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn builds_the_model() {
        let dive = sample_dive().to_dive();
        assert_eq!(dive.when, 1_717_239_600); // 2024-06-01 12:00 UTC+1
        let computer = dive.primary_computer().unwrap();
        assert_eq!(computer.model, "Shearwater Petrel 2");
        assert_eq!(computer.divemode, Divemode::Ccr);
        assert_eq!(computer.duration.unwrap().seconds, 120);
        assert_eq!(computer.max_depth.unwrap().mm, 20_000);
        assert_eq!(computer.water_temp.unwrap().mkelvin, 287_150);
        assert_eq!(computer.serial.as_deref(), Some("1234"));
        assert_eq!(computer.samples.len(), 2);
        assert_eq!(computer.samples[1].depth.mm, 10_000);
        assert_eq!(computer.samples[1].pressures[0].pressure.mbar, 180_000);
        assert_eq!(computer.samples[1].ndl.unwrap().seconds, 600);
        assert_eq!(computer.samples[1].cns, Some(3));
        assert_eq!(computer.samples[1].setpoint, None);
        assert_eq!(computer.events.len(), 2);
        assert_eq!(computer.events[0].name, "gaschange");
        assert_eq!(computer.events[1].name, "ascent");
        assert_eq!(computer.events[1].time.seconds, 1);
        assert_eq!(dive.cylinders.len(), 1);
        assert_eq!(dive.cylinders[0].gas.o2_permille, 210);
        assert_eq!(dive.cylinders[0].use_, CylinderUse::Diluent);
        assert_eq!(dive.cylinders[0].end_pressure.unwrap().mbar, 50_000);
    }

    #[test]
    fn raw_dive_round_trips_through_json() {
        let raw = sample_dive();
        let text = serde_json::to_string(&raw).unwrap();
        let back: RawDive = serde_json::from_str(&text).unwrap();
        assert_eq!(raw, back);
        assert!(text.contains(r#""t":"time""#));
        assert!(text.contains(r#""t":"deco""#));
    }

    #[test]
    fn datetime_without_timezone_is_utc() {
        let dt = RawDateTime {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            timezone: None,
        };
        assert_eq!(dt.to_unix(), 0);
        let shifted = RawDateTime {
            timezone: Some(-3600),
            ..dt
        };
        assert_eq!(shifted.to_unix(), 3600);
    }
}
