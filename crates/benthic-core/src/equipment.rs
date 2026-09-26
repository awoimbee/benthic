//! Equipment presets and helpers.
//!
//! Divers think in named cylinders ("AL80", "LP85", "steel 15"), not in
//! litres and bars. This module maps those names to physical properties and
//! applies them to a [`Cylinder`] while preserving anything already measured
//! (gas and start/end pressure).

use crate::model::Cylinder;
use crate::units::{Pressure, Volume};

/// A named, commonly-used cylinder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CylinderPreset {
    pub name: &'static str,
    /// Nominal water volume in litres.
    pub volume_l: f64,
    /// Working (service) pressure in bar.
    pub working_pressure_bar: f64,
}

/// Common open-circuit cylinders. Values are nominal and rounded.
pub const CYLINDER_PRESETS: &[CylinderPreset] = &[
    CylinderPreset {
        name: "AL80",
        volume_l: 11.1,
        working_pressure_bar: 207.0,
    },
    CylinderPreset {
        name: "AL72",
        volume_l: 10.2,
        working_pressure_bar: 207.0,
    },
    CylinderPreset {
        name: "AL40",
        volume_l: 5.7,
        working_pressure_bar: 207.0,
    },
    CylinderPreset {
        name: "AL19",
        volume_l: 2.7,
        working_pressure_bar: 207.0,
    },
    CylinderPreset {
        name: "LP85",
        volume_l: 13.4,
        working_pressure_bar: 182.0,
    },
    CylinderPreset {
        name: "LP95",
        volume_l: 15.0,
        working_pressure_bar: 182.0,
    },
    CylinderPreset {
        name: "HP100",
        volume_l: 12.9,
        working_pressure_bar: 232.0,
    },
    CylinderPreset {
        name: "HP120",
        volume_l: 15.5,
        working_pressure_bar: 232.0,
    },
    CylinderPreset {
        name: "Steel 7L",
        volume_l: 7.0,
        working_pressure_bar: 232.0,
    },
    CylinderPreset {
        name: "Steel 12L",
        volume_l: 12.0,
        working_pressure_bar: 232.0,
    },
    CylinderPreset {
        name: "Steel 15L",
        volume_l: 15.0,
        working_pressure_bar: 232.0,
    },
    CylinderPreset {
        name: "Steel 18L",
        volume_l: 18.0,
        working_pressure_bar: 232.0,
    },
];

/// Look up a preset by (case-insensitive) name.
pub fn cylinder_preset(name: &str) -> Option<&'static CylinderPreset> {
    let name = name.trim();
    CYLINDER_PRESETS
        .iter()
        .find(|preset| preset.name.eq_ignore_ascii_case(name))
}

/// Whether `name` matches a known preset.
pub fn is_preset(name: &str) -> bool {
    cylinder_preset(name).is_some()
}

impl Cylinder {
    /// A cylinder configured from a preset (air, no measured pressures).
    pub fn from_preset(preset: &CylinderPreset) -> Self {
        Self {
            size: Some(Volume::from_liters(preset.volume_l)),
            working_pressure: Some(Pressure::from_bar(preset.working_pressure_bar)),
            description: preset.name.to_string(),
            ..Default::default()
        }
    }
}

/// Apply a preset to an existing cylinder, keeping its gas and measured
/// start/end pressures.
pub fn apply_preset(cylinder: &mut Cylinder, preset: &CylinderPreset) {
    let gas = cylinder.gas;
    let start = cylinder.start_pressure;
    let end = cylinder.end_pressure;
    *cylinder = Cylinder {
        gas,
        start_pressure: start,
        end_pressure: end,
        ..Cylinder::from_preset(preset)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::GasMix;

    #[test]
    fn preset_lookup_is_case_insensitive() {
        assert!(cylinder_preset("al80").is_some());
        assert_eq!(cylinder_preset("AL80").unwrap().volume_l, 11.1);
        assert!(cylinder_preset("nonexistent").is_none());
        assert!(is_preset("Steel 15L"));
        assert!(!is_preset("mystery tank"));
    }

    #[test]
    fn from_preset_sets_size_and_pressure() {
        let cyl = Cylinder::from_preset(cylinder_preset("HP100").unwrap());
        assert_eq!(cyl.description, "HP100");
        assert_eq!(cyl.size, Some(Volume::from_liters(12.9)));
        assert_eq!(cyl.working_pressure, Some(Pressure::from_bar(232.0)));
        assert!(cyl.gas.is_air());
    }

    #[test]
    fn apply_preset_preserves_gas_and_pressures() {
        let mut cyl = Cylinder {
            gas: GasMix::percent(32.0, 0.0),
            start_pressure: Some(Pressure::from_bar(200.0)),
            end_pressure: Some(Pressure::from_bar(60.0)),
            ..Default::default()
        };
        apply_preset(&mut cyl, cylinder_preset("AL80").unwrap());
        assert_eq!(cyl.description, "AL80");
        assert_eq!(cyl.size, Some(Volume::from_liters(11.1)));
        assert_eq!(cyl.gas.o2_permille, 320);
        assert_eq!(cyl.start_pressure, Some(Pressure::from_bar(200.0)));
        assert_eq!(cyl.end_pressure, Some(Pressure::from_bar(60.0)));
    }
}
