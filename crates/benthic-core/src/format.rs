//! User preferences and preference-aware formatting.
//!
//! Preferences are display-only: the model always stores canonical integer
//! units, and file formats are always written in metric. This module turns
//! those canonical values into strings in whatever units the diver prefers.

use serde::{Deserialize, Serialize};

use crate::units::{Depth, Pressure, Temperature, Volume, Weight};

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

/// Everything the user can configure. Serialized to local storage separately
/// from the dive log so preferences survive replacing the log.
///
/// Kept `Copy` so it is trivial to capture in UI closures; if a non-`Copy`
/// field is ever added, switch to reading the signal inside closures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Preferences {
    #[serde(default)]
    pub units: UnitSystem,
}

impl Preferences {
    pub fn metric() -> Self {
        Self::default()
    }

    pub fn imperial() -> Self {
        Self {
            units: UnitSystem::Imperial,
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
}
