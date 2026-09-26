//! Strongly-typed physical units.
//!
//! Like Subsurface, we store all measurements as integers in well-defined
//! base units (millimetres, millibar, millikelvin, ...) so that repeated
//! conversions never accumulate floating-point error. Human-friendly
//! constructors and accessors live alongside each type.

use serde::{Deserialize, Serialize};

/// Seconds since the Unix epoch (UTC).
pub type Timestamp = i64;

/// Temperature of 0 °C in millikelvin.
pub const ZERO_CELSIUS_MKELVIN: u32 = 273_150;

/// Salinity presets, in grams of salt per 10 litres of water.
pub const FRESHWATER_SALINITY: i32 = 10_000;
pub const BRACKISH_SALINITY: i32 = 10_100;
pub const EN13319_SALINITY: i32 = 10_200;
pub const SEAWATER_SALINITY: i32 = 10_300;

macro_rules! integer_unit {
    ($(#[$meta:meta])* $name:ident, $field:ident : $ty:ty) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
        #[serde(transparent)]
        pub struct $name {
            pub $field: $ty,
        }

        impl $name {
            pub const ZERO: Self = Self { $field: 0 };
            pub const fn new($field: $ty) -> Self {
                Self { $field }
            }
        }
    };
}

integer_unit! {
    /// A duration or an offset, in seconds.
    Duration, seconds: i32
}
integer_unit! {
    /// A depth, in millimetres.
    Depth, mm: i32
}
integer_unit! {
    /// A pressure, in millibar.
    Pressure, mbar: i32
}
integer_unit! {
    /// An oxygen partial pressure, in millibar (unsigned, small range).
    O2Pressure, mbar: u16
}
integer_unit! {
    /// A temperature, in millikelvin (always positive).
    Temperature, mkelvin: u32
}
integer_unit! {
    /// A volume, in millilitres.
    Volume, ml: i32
}
integer_unit! {
    /// A gas fraction, in permille (e.g. 320 == 32.0%).
    Fraction, permille: i32
}
integer_unit! {
    /// A weight, in grams.
    Weight, grams: i32
}
integer_unit! {
    /// A compass bearing, in whole degrees.
    Bearing, degrees: i16
}
integer_unit! {
    /// An angle expressed in microdegrees.
    Degrees, udeg: i32
}

impl Duration {
    pub const fn from_minutes(minutes: i32) -> Self {
        Self {
            seconds: minutes * 60,
        }
    }

    pub fn hours(self) -> f64 {
        self.seconds as f64 / 3600.0
    }
}

impl Depth {
    pub fn from_meters(m: f64) -> Self {
        Self {
            mm: (m * 1000.0).round() as i32,
        }
    }

    pub fn from_feet(ft: f64) -> Self {
        Self {
            mm: (ft * 304.8).round() as i32,
        }
    }

    pub fn meters(self) -> f64 {
        self.mm as f64 / 1000.0
    }

    pub fn feet(self) -> f64 {
        self.mm as f64 / 304.8
    }
}

impl Pressure {
    pub fn from_bar(bar: f64) -> Self {
        Self {
            mbar: (bar * 1000.0).round() as i32,
        }
    }

    pub fn bar(self) -> f64 {
        self.mbar as f64 / 1000.0
    }

    pub fn from_psi(psi: f64) -> Self {
        Self::from_bar(psi / 14.503_773_8)
    }

    pub fn psi(self) -> f64 {
        self.bar() * 14.503_773_8
    }
}

impl O2Pressure {
    pub fn from_bar(bar: f64) -> Self {
        Self {
            mbar: (bar * 1000.0).round() as u16,
        }
    }

    pub fn bar(self) -> f64 {
        self.mbar as f64 / 1000.0
    }
}

impl Temperature {
    pub fn from_celsius(c: f64) -> Self {
        Self {
            mkelvin: (c * 1000.0 + ZERO_CELSIUS_MKELVIN as f64).round() as u32,
        }
    }

    pub fn from_fahrenheit(f: f64) -> Self {
        Self::from_celsius((f - 32.0) * 5.0 / 9.0)
    }

    pub fn celsius(self) -> f64 {
        (self.mkelvin as f64 - ZERO_CELSIUS_MKELVIN as f64) / 1000.0
    }

    pub fn fahrenheit(self) -> f64 {
        self.celsius() * 9.0 / 5.0 + 32.0
    }
}

impl Volume {
    pub fn from_liters(l: f64) -> Self {
        Self {
            ml: (l * 1000.0).round() as i32,
        }
    }

    pub fn liters(self) -> f64 {
        self.ml as f64 / 1000.0
    }

    pub fn from_cubic_feet(cuft: f64) -> Self {
        Self {
            ml: (cuft * 28_316.846_6).round() as i32,
        }
    }

    pub fn cubic_feet(self) -> f64 {
        self.liters() / 28.316_846_6
    }
}

impl Fraction {
    pub fn from_percent(p: f64) -> Self {
        Self {
            permille: (p * 10.0).round() as i32,
        }
    }

    pub fn percent(self) -> f64 {
        self.permille as f64 / 10.0
    }
}

impl Weight {
    pub fn from_kg(kg: f64) -> Self {
        Self {
            grams: (kg * 1000.0).round() as i32,
        }
    }

    pub fn kg(self) -> f64 {
        self.grams as f64 / 1000.0
    }

    pub fn from_lbs(lbs: f64) -> Self {
        Self {
            grams: (lbs * 453.592_37).round() as i32,
        }
    }

    pub fn lbs(self) -> f64 {
        self.grams as f64 / 453.592_37
    }
}

/// A geographic location in WGS-84 decimal degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

impl Location {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// True when the location carries meaningful coordinates.
    pub fn is_valid(&self) -> bool {
        !(self.lat == 0.0 && self.lon == 0.0)
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon)
    }
}

/// Format a [`Duration`] the way dive logs usually display it: `MM:SS` or `H:MM:SS`.
pub fn format_duration(d: Duration) -> String {
    let total = d.seconds;
    let sign = if total < 0 { "-" } else { "" };
    let total = total.unsigned_abs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{sign}{h}:{m:02}:{s:02}")
    } else {
        format!("{sign}{m}:{s:02}")
    }
}

/// Format a depth in metres with one decimal.
pub fn format_depth_m(d: Depth) -> String {
    format!("{:.1} m", d.meters())
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn format_timestamp_utc(ts: Timestamp) -> String {
    let dt =
        time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let time = dt.time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}
