//! User preferences and preference-aware formatting.
//!
//! Preferences are display-only: the model always stores canonical integer
//! units, and file formats are always written in metric. This module turns
//! those canonical values into strings in whatever units the diver prefers.

use serde::{Deserialize, Serialize};

use crate::units::{
    Depth, Pressure, Temperature, Volume, Weight, BRACKISH_SALINITY, EN13319_SALINITY,
    FRESHWATER_SALINITY, SEAWATER_SALINITY,
};

/// Which set of units to display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnitSystem {
    /// Metres, °C, bar, kg, litres.
    #[default]
    Metric,
    /// Feet, °F, psi, lbs, cubic feet.
    Imperial,
}

impl UnitSystem {
    pub const ALL: [UnitSystem; 2] = [UnitSystem::Metric, UnitSystem::Imperial];

    pub fn label(self) -> &'static str {
        match self {
            UnitSystem::Metric => "Metric",
            UnitSystem::Imperial => "Imperial",
        }
    }
}

/// The language the interface is displayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    French,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::French];

    /// The language's own name, so the picker is readable in any locale.
    pub fn label(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
        }
    }

    /// BCP-47 tag, used for the web `lang` attribute.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
        }
    }
}

/// The colour theme for the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }
}

/// How to render dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    /// `2024-05-12`.
    #[default]
    Iso,
    /// `05/12/2024`.
    Us,
    /// `12/05/2024`.
    European,
}

impl DateFormat {
    pub const ALL: [DateFormat; 3] = [DateFormat::Iso, DateFormat::Us, DateFormat::European];

    pub fn label(self) -> &'static str {
        match self {
            DateFormat::Iso => "YYYY-MM-DD",
            DateFormat::Us => "MM/DD/YYYY",
            DateFormat::European => "DD/MM/YYYY",
        }
    }
}

/// How to render times of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormat {
    /// 24-hour clock.
    #[default]
    H24,
    /// 12-hour clock with AM/PM.
    H12,
}

impl TimeFormat {
    pub const ALL: [TimeFormat; 2] = [TimeFormat::H24, TimeFormat::H12];

    pub fn label(self) -> &'static str {
        match self {
            TimeFormat::H24 => "24-hour",
            TimeFormat::H12 => "12-hour",
        }
    }
}

/// A water-salinity preset, used for pressure/depth conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Salinity {
    Freshwater,
    Brackish,
    #[default]
    En13319,
    Seawater,
}

impl Salinity {
    pub const ALL: [Salinity; 4] = [
        Salinity::Freshwater,
        Salinity::Brackish,
        Salinity::En13319,
        Salinity::Seawater,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Salinity::Freshwater => "Fresh water",
            Salinity::Brackish => "Brackish",
            Salinity::En13319 => "EN13319",
            Salinity::Seawater => "Sea water",
        }
    }

    /// The salinity in grams of salt per 10 litres of water.
    pub fn value(self) -> i32 {
        match self {
            Salinity::Freshwater => FRESHWATER_SALINITY,
            Salinity::Brackish => BRACKISH_SALINITY,
            Salinity::En13319 => EN13319_SALINITY,
            Salinity::Seawater => SEAWATER_SALINITY,
        }
    }
}

/// Everything the user can configure. Serialized to local storage separately
/// from the dive log so preferences survive replacing the log.
///
/// Kept `Copy` so it is trivial to capture in UI closures; if a non-`Copy`
/// field is ever added, switch to reading the signal inside closures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Preferences {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub units: UnitSystem,
    #[serde(default)]
    pub date_format: DateFormat,
    #[serde(default)]
    pub time_format: TimeFormat,
    #[serde(default)]
    pub default_salinity: Salinity,
    /// Index into `equipment::CYLINDER_PRESETS` used for newly added
    /// cylinders, or `None` for a plain default cylinder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cylinder: Option<usize>,
    /// Set once the first-run welcome has been dismissed, so it never shows
    /// again on this device.
    #[serde(default)]
    pub seen_welcome: bool,
}

impl Preferences {
    pub fn metric() -> Self {
        Self::default()
    }

    pub fn imperial() -> Self {
        Self {
            units: UnitSystem::Imperial,
            ..Default::default()
        }
    }

    pub fn is_metric(&self) -> bool {
        self.units == UnitSystem::Metric
    }

    /// Unit label for depth, e.g. `m` or `ft`.
    pub fn depth_unit(&self) -> &'static str {
        match self.units {
            UnitSystem::Metric => "m",
            UnitSystem::Imperial => "ft",
        }
    }

    /// Depth as a bare number in the active unit.
    pub fn depth_value(&self, depth: Depth) -> f64 {
        match self.units {
            UnitSystem::Metric => depth.meters(),
            UnitSystem::Imperial => depth.feet(),
        }
    }

    /// Build a depth from a bare number in the active unit.
    pub fn depth_from_value(&self, value: f64) -> Depth {
        match self.units {
            UnitSystem::Metric => Depth::from_meters(value),
            UnitSystem::Imperial => Depth::from_feet(value),
        }
    }

    /// Format a depth with its unit.
    pub fn depth(&self, depth: Depth) -> String {
        match self.units {
            UnitSystem::Metric => format!("{:.1} m", depth.meters()),
            UnitSystem::Imperial => format!("{:.0} ft", depth.feet()),
        }
    }

    /// Format a temperature with its unit.
    pub fn temperature(&self, temperature: Temperature) -> String {
        match self.units {
            UnitSystem::Metric => format!("{:.1} °C", temperature.celsius()),
            UnitSystem::Imperial => format!("{:.1} °F", temperature.fahrenheit()),
        }
    }

    pub fn pressure_value(&self, pressure: Pressure) -> f64 {
        match self.units {
            UnitSystem::Metric => pressure.bar(),
            UnitSystem::Imperial => pressure.psi(),
        }
    }

    pub fn pressure_from_value(&self, value: f64) -> Pressure {
        match self.units {
            UnitSystem::Metric => Pressure::from_bar(value),
            UnitSystem::Imperial => Pressure::from_psi(value),
        }
    }

    /// Format a pressure with its unit.
    pub fn pressure(&self, pressure: Pressure) -> String {
        match self.units {
            UnitSystem::Metric => format!("{:.0} bar", pressure.bar()),
            UnitSystem::Imperial => format!("{:.0} psi", pressure.psi()),
        }
    }

    /// Format a volume with its unit.
    pub fn volume(&self, volume: Volume) -> String {
        match self.units {
            UnitSystem::Metric => format!("{:.1} L", volume.liters()),
            UnitSystem::Imperial => format!("{:.1} cuft", volume.cubic_feet()),
        }
    }

    pub fn weight_value(&self, weight: Weight) -> f64 {
        match self.units {
            UnitSystem::Metric => weight.kg(),
            UnitSystem::Imperial => weight.lbs(),
        }
    }

    pub fn weight_from_value(&self, value: f64) -> Weight {
        match self.units {
            UnitSystem::Metric => Weight::from_kg(value),
            UnitSystem::Imperial => Weight::from_lbs(value),
        }
    }

    /// Format a weight with its unit.
    pub fn weight(&self, weight: Weight) -> String {
        match self.units {
            UnitSystem::Metric => format!("{:.2} kg", weight.kg()),
            UnitSystem::Imperial => format!("{:.1} lbs", weight.lbs()),
        }
    }

    /// The active weight unit label.
    pub fn weight_unit(&self) -> &'static str {
        match self.units {
            UnitSystem::Metric => "kg",
            UnitSystem::Imperial => "lbs",
        }
    }

    /// The active pressure unit label.
    pub fn pressure_unit(&self) -> &'static str {
        match self.units {
            UnitSystem::Metric => "bar",
            UnitSystem::Imperial => "psi",
        }
    }

    /// Format just the date of a timestamp.
    pub fn date(&self, timestamp: crate::units::Timestamp) -> String {
        let dt = odt(timestamp);
        let date = dt.date();
        let (year, month, day) = (date.year(), u8::from(date.month()), date.day());
        match self.date_format {
            DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::Us => format!("{month:02}/{day:02}/{year:04}"),
            DateFormat::European => format!("{day:02}/{month:02}/{year:04}"),
        }
    }

    /// Format just the time of day of a timestamp.
    pub fn time(&self, timestamp: crate::units::Timestamp) -> String {
        let time = odt(timestamp).time();
        let (hour, minute) = (time.hour(), time.minute());
        match self.time_format {
            TimeFormat::H24 => format!("{hour:02}:{minute:02}"),
            TimeFormat::H12 => {
                let suffix = if hour < 12 { "AM" } else { "PM" };
                let hour12 = match hour % 12 {
                    0 => 12,
                    h => h,
                };
                format!("{hour12}:{minute:02} {suffix}")
            }
        }
    }

    /// Format a timestamp as date and time using the active preferences.
    pub fn timestamp(&self, timestamp: crate::units::Timestamp) -> String {
        format!("{} {}", self.date(timestamp), self.time(timestamp))
    }
}

fn odt(timestamp: crate::units::Timestamp) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(timestamp).unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

/// Format a timestamp for a native date/time input, in UTC.
///
/// The string is the `YYYY-MM-DDTHH:MM` form an `<input type="datetime-local">`
/// expects. Times are kept in UTC to match [`Preferences::timestamp`], which
/// renders the same wall clock.
pub fn datetime_local(timestamp: crate::units::Timestamp) -> String {
    let dt = odt(timestamp);
    let date = dt.date();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        dt.hour(),
        dt.minute()
    )
}

/// Parse the value of a native date/time input back into a UTC timestamp.
/// Returns `None` when the value is incomplete or out of range.
pub fn parse_datetime_local(value: &str) -> Option<crate::units::Timestamp> {
    let value = value.trim();
    let (date, clock) = value.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month = time::Month::try_from(date_parts.next()?.parse::<u8>().ok()?).ok()?;
    let day: u8 = date_parts.next()?.parse().ok()?;
    let mut clock_parts = clock.split(':');
    let hour: u8 = clock_parts.next()?.parse().ok()?;
    let minute: u8 = clock_parts.next()?.parse().ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let clock = time::Time::from_hms(hour, minute, 0).ok()?;
    Some(
        time::PrimitiveDateTime::new(date, clock)
            .assume_utc()
            .unix_timestamp(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_roundtrip() {
        let prefs = Preferences::metric();
        assert_eq!(prefs.depth(Depth::from_meters(12.34)), "12.3 m");
        assert_eq!(
            prefs.temperature(Temperature::from_celsius(20.0)),
            "20.0 °C"
        );
        assert_eq!(prefs.pressure(Pressure::from_bar(200.0)), "200 bar");
        assert!((prefs.depth_from_value(30.0).meters() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn imperial_conversions() {
        let prefs = Preferences::imperial();
        // 10 m is about 32.8 ft.
        assert_eq!(prefs.depth(Depth::from_meters(10.0)), "33 ft");
        // 20 °C is 68 °F.
        assert_eq!(
            prefs.temperature(Temperature::from_celsius(20.0)),
            "68.0 °F"
        );
        // 200 bar is about 2900 psi.
        assert_eq!(prefs.pressure(Pressure::from_bar(200.0)), "2901 psi");
        // 12 L is about 0.42 cuft.
        assert!(prefs.volume(Volume::from_liters(12.0)).starts_with("0.4"));
        // 6 kg is about 13.2 lbs.
        assert_eq!(prefs.weight(Weight::from_kg(6.0)), "13.2 lbs");
    }

    #[test]
    fn imperial_roundtrip_values() {
        let prefs = Preferences::imperial();
        let depth = prefs.depth_from_value(100.0);
        assert!((prefs.depth_value(depth) - 100.0).abs() < 0.1);
        let pressure = prefs.pressure_from_value(3000.0);
        assert!((prefs.pressure_value(pressure) - 3000.0).abs() < 1.0);
        let weight = prefs.weight_from_value(10.0);
        assert!((prefs.weight_value(weight) - 10.0).abs() < 0.01);
    }

    #[test]
    fn date_and_time_formats() {
        // 2024-05-12 20:15:00 UTC.
        let timestamp = 1_715_544_900;
        let mut prefs = Preferences::metric();
        assert_eq!(prefs.date(timestamp), "2024-05-12");
        assert_eq!(prefs.time(timestamp), "20:15");
        assert_eq!(prefs.timestamp(timestamp), "2024-05-12 20:15");

        prefs.date_format = DateFormat::Us;
        assert_eq!(prefs.date(timestamp), "05/12/2024");
        prefs.date_format = DateFormat::European;
        assert_eq!(prefs.date(timestamp), "12/05/2024");

        prefs.date_format = DateFormat::Iso;
        prefs.time_format = TimeFormat::H12;
        assert_eq!(prefs.time(timestamp), "8:15 PM");
        assert_eq!(prefs.timestamp(timestamp), "2024-05-12 8:15 PM");

        // Midnight renders as 12:00 AM in 12-hour time.
        assert_eq!(prefs.time(1_715_544_900 - 20 * 3600 - 15 * 60), "12:00 AM");
    }

    #[test]
    fn datetime_local_round_trips() {
        let timestamp = 1_715_544_900;
        let value = datetime_local(timestamp);
        assert_eq!(value, "2024-05-12T20:15");
        assert_eq!(parse_datetime_local(&value), Some(timestamp));
        assert_eq!(parse_datetime_local("not a date"), None);
        assert_eq!(parse_datetime_local("2024-13-40T99:99"), None);
    }

    #[test]
    fn language_defaults_to_english() {
        assert_eq!(Preferences::default().language, Language::English);
        assert_eq!(Language::French.code(), "fr");
        assert_eq!(Language::ALL.len(), 2);
    }

    #[test]
    fn salinity_presets() {
        assert_eq!(Salinity::Freshwater.value(), FRESHWATER_SALINITY);
        assert_eq!(Salinity::Seawater.value(), SEAWATER_SALINITY);
        assert_eq!(Preferences::default().default_salinity, Salinity::En13319);
        assert_eq!(Preferences::default().default_cylinder, None);
    }
}
