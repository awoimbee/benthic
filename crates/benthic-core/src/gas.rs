//! Breathing-gas mixtures.

use serde::{Deserialize, Serialize};

/// A breathing gas, stored as permille of oxygen and helium.
///
/// Air is represented explicitly as 21.0% O2 / 0% He so that the native JSON
/// format is never ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasMix {
    /// Oxygen fraction in permille.
    pub o2_permille: u16,
    /// Helium fraction in permille.
    pub he_permille: u16,
}

/// Standard air.
pub const AIR: GasMix = GasMix {
    o2_permille: 210,
    he_permille: 0,
};
/// Pure oxygen.
pub const OXYGEN: GasMix = GasMix {
    o2_permille: 1000,
    he_permille: 0,
};

impl Default for GasMix {
    fn default() -> Self {
        AIR
    }
}

impl GasMix {
    pub const fn new(o2_permille: u16, he_permille: u16) -> Self {
        Self {
            o2_permille,
            he_permille,
        }
    }

    /// Build a mix from percentages, e.g. `GasMix::percent(32.0, 0.0)`.
    pub fn percent(o2: f64, he: f64) -> Self {
        Self {
            o2_permille: (o2 * 10.0).round() as u16,
            he_permille: (he * 10.0).round() as u16,
        }
    }

    pub fn o2_percent(self) -> f64 {
        self.o2_permille as f64 / 10.0
    }

    pub fn he_percent(self) -> f64 {
        self.he_permille as f64 / 10.0
    }

    /// Nitrogen fraction in permille (remainder of O2 and He).
    pub fn n2_permille(self) -> i32 {
        1000 - self.o2_permille as i32 - self.he_permille as i32
    }

    pub fn is_air(self) -> bool {
        self.o2_permille == AIR.o2_permille && self.he_permille == 0
    }

    pub fn is_oxygen(self) -> bool {
        self.o2_permille >= 995 && self.he_permille == 0
    }

    pub fn is_trimix(self) -> bool {
        self.he_permille > 0
    }

    pub fn is_nitrox(self) -> bool {
        self.he_permille == 0 && self.o2_permille > AIR.o2_permille
    }

    /// A short human-readable name: "Air", "EAN32", "Tx21/35", "O2".
    pub fn name(self) -> String {
        if self.is_air() {
            "Air".into()
        } else if self.is_oxygen() {
            "O2".into()
        } else if self.is_trimix() {
            format!("Tx{}/{}", self.o2_permille / 10, self.he_permille / 10)
        } else if self.o2_permille > AIR.o2_permille {
            format!("EAN{}", self.o2_permille / 10)
        } else {
            format!("{}% O2", self.o2_percent())
        }
    }
}

// -- Planning maths --------------------------------------------------------
//
// These mirror Subsurface's gas calculations. All pressures are millibar and
// all depths millimetres, matching the rest of the model.

/// Standard sea-level surface pressure in millibar.
pub const SURFACE_PRESSURE_MBAR: f64 = 1013.25;
/// Default maximum pO2 for the working portion of a dive, in millibar.
pub const DEFAULT_PO2_LIMIT_MBAR: f64 = 1400.0;
/// Nitrogen fraction of air, in permille.
pub const N2_IN_AIR: f64 = 790.0;

/// Oxygen fraction of a mix in permille, treating a zero as air.
fn effective_o2(gas: GasMix) -> i32 {
    if gas.o2_permille == 0 {
        210
    } else {
        gas.o2_permille as i32
    }
}

/// Nitrogen fraction of a mix in permille, treating a zero O2 as air.
fn effective_n2(gas: GasMix) -> i32 {
    1000 - effective_o2(gas) - gas.he_permille as i32
}

/// Oxygen fraction as a 0..1 value, treating a zero as air.
pub fn o2_fraction(gas: GasMix) -> f64 {
    effective_o2(gas) as f64 / 1000.0
}

/// Nitrogen fraction as a 0..1 value, treating a zero O2 as air.
pub fn n2_fraction(gas: GasMix) -> f64 {
    effective_n2(gas) as f64 / 1000.0
}

/// Helium fraction as a 0..1 value.
pub fn he_fraction(gas: GasMix) -> f64 {
    gas.he_permille as f64 / 1000.0
}

/// Ambient pressure at `depth_mm`, in millibar.
///
/// `salinity` is grams of salt per 10 litres, which equals the water density in
/// kg/m³, so freshwater is 1000, EN13319 is 1020 and sea water is 1030.
pub fn ambient_mbar(depth_mm: i32, surface_mbar: f64, salinity: i32) -> f64 {
    let density = salinity.max(1) as f64 / 10.0;
    surface_mbar + density * 9.806_65 * (depth_mm as f64 / 1000.0) / 100.0
}

/// The depth (in millimetres) at which the ambient pressure reaches
/// `ambient_mbar`; the inverse of [`ambient_mbar`].
pub fn depth_mm_at(ambient_mbar: f64, surface_mbar: f64, salinity: i32) -> i32 {
    let density = salinity.max(1) as f64 / 10.0;
    let mbar_per_m = density * 9.806_65 / 100.0;
    (((ambient_mbar - surface_mbar) / mbar_per_m) * 1000.0).round() as i32
}

/// Maximum operating depth for a mix at a pO2 limit, in millimetres.
pub fn mod_depth_mm(gas: GasMix, po2_limit_mbar: f64, surface_mbar: f64, salinity: i32) -> i32 {
    let fo2 = effective_o2(gas) as f64 / 1000.0;
    if fo2 <= 0.0 {
        return 0;
    }
    depth_mm_at(po2_limit_mbar / fo2, surface_mbar, salinity).max(0)
}

/// The richest oxygen fraction (in permille) that stays within `po2_limit_mbar`
/// at the given depth.
pub fn best_o2_permille(
    depth_mm: i32,
    po2_limit_mbar: f64,
    surface_mbar: f64,
    salinity: i32,
) -> u16 {
    let ambient = ambient_mbar(depth_mm, surface_mbar, salinity);
    ((po2_limit_mbar / ambient) * 1000.0)
        .round()
        .clamp(0.0, 1000.0) as u16
}

/// Equivalent narcotic depth, in millimetres.
///
/// When `o2_narcotic` is set, oxygen is treated as narcotic (the more
/// conservative assumption) and contributes to the narcotic partial pressure.
pub fn end_depth_mm(
    gas: GasMix,
    depth_mm: i32,
    surface_mbar: f64,
    salinity: i32,
    o2_narcotic: bool,
) -> i32 {
    let ambient = ambient_mbar(depth_mm, surface_mbar, salinity);
    let narcotic = if o2_narcotic {
        (effective_n2(gas) + effective_o2(gas)).clamp(0, 1000) as f64 / 1000.0
    } else {
        effective_n2(gas) as f64 / 1000.0
    };
    depth_mm_at(
        narcotic * ambient / (N2_IN_AIR / 1000.0),
        surface_mbar,
        salinity,
    )
}

/// Equivalent air depth, in millimetres: the air depth with the same nitrogen
/// partial pressure.
pub fn ead_depth_mm(gas: GasMix, depth_mm: i32, surface_mbar: f64, salinity: i32) -> i32 {
    let ambient = ambient_mbar(depth_mm, surface_mbar, salinity);
    let f_n2 = effective_n2(gas) as f64 / 1000.0;
    depth_mm_at(
        f_n2 * ambient / (N2_IN_AIR / 1000.0),
        surface_mbar,
        salinity,
    )
}

/// The result of an isobaric counterdiffusion (ICD) check between two gases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcdResult {
    /// Change in nitrogen fraction, in permille (positive = more nitrogen).
    pub d_n2: i32,
    /// Change in helium fraction, in permille (positive = more helium).
    pub d_he: i32,
    /// Whether the switch risks isobaric counterdiffusion.
    pub warning: bool,
}

/// Check whether switching from `old` to `new` risks isobaric
/// counterdiffusion, which happens when nitrogen and helium move in opposite
/// directions across the switch.
pub fn isobaric_counterdiffusion(old: GasMix, new: GasMix) -> IcdResult {
    let d_n2 = effective_n2(new) - effective_n2(old);
    let d_he = new.he_permille as i32 - old.he_permille as i32;
    IcdResult {
        d_n2,
        d_he,
        warning: (d_n2 > 0 && d_he < 0) || (d_n2 < 0 && d_he > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Salinity;

    const SEA: i32 = 10_300;

    #[test]
    fn ambient_pressure_round_trips() {
        let ambient = ambient_mbar(30_000, SURFACE_PRESSURE_MBAR, SEA);
        // 30 m of sea water is roughly 4.04 bar = 4043 mbar.
        assert!((3900.0..4100.0).contains(&ambient), "ambient was {ambient}");
        assert_eq!(depth_mm_at(ambient, SURFACE_PRESSURE_MBAR, SEA), 30_000);
    }

    #[test]
    fn mod_of_air_and_nitrox() {
        // Air at 1.4 bar pO2 tops out around 56 m.
        let air = mod_depth_mm(AIR, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR, SEA);
        assert!((55_000..57_000).contains(&air), "air MOD was {air}");
        // EAN32 around 33 m.
        let ean32 = mod_depth_mm(
            GasMix::percent(32.0, 0.0),
            DEFAULT_PO2_LIMIT_MBAR,
            SURFACE_PRESSURE_MBAR,
            SEA,
        );
        assert!((32_000..35_000).contains(&ean32), "EAN32 MOD was {ean32}");
        // Pure O2 at a 1.4 bar limit tops out around 3.8 m.
        let o2 = mod_depth_mm(OXYGEN, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR, SEA);
        assert!((3_000..5_000).contains(&o2), "O2 MOD was {o2}");
    }

    #[test]
    fn best_mix_matches_mod_inverse() {
        let depth = 30_000;
        let fo2 = best_o2_permille(depth, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR, SEA);
        assert!((340..350).contains(&fo2), "best O2 was {fo2}");
        let gas = GasMix::new(fo2, 0);
        let modd = mod_depth_mm(gas, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR, SEA);
        assert!((depth - 500..depth + 500).contains(&modd));
    }

    #[test]
    fn ead_of_nitrox() {
        // EAN32 at 30 m has an EAD of about 24 m.
        let ead = ead_depth_mm(
            GasMix::percent(32.0, 0.0),
            30_000,
            SURFACE_PRESSURE_MBAR,
            SEA,
        );
        assert!((23_000..26_000).contains(&ead), "EAD was {ead}");
        // Air's EAD is its depth.
        assert_eq!(
            ead_depth_mm(AIR, 20_000, SURFACE_PRESSURE_MBAR, SEA),
            20_000
        );
    }

    #[test]
    fn end_of_trimix() {
        // Tx21/35 at 60 m has an END of roughly 29 m (oxygen not narcotic).
        let end = end_depth_mm(
            GasMix::percent(21.0, 35.0),
            60_000,
            SURFACE_PRESSURE_MBAR,
            SEA,
            false,
        );
        assert!((27_000..31_000).contains(&end), "END was {end}");
        // Air's END is its depth.
        let air_end = end_depth_mm(AIR, 40_000, SURFACE_PRESSURE_MBAR, SEA, false);
        assert!((39_500..40_500).contains(&air_end), "air END was {air_end}");
    }

    #[test]
    fn icd_detects_opposite_gradients() {
        // Air -> trimix lowers N2 while raising He: a warning.
        let icd = isobaric_counterdiffusion(AIR, GasMix::percent(21.0, 35.0));
        assert!(icd.warning);
        assert!(icd.d_n2 < 0 && icd.d_he > 0);
        // Air -> nitrox only lowers N2: no warning.
        assert!(!isobaric_counterdiffusion(AIR, GasMix::percent(32.0, 0.0)).warning);
    }

    #[test]
    fn salinity_values_are_densities() {
        // Fresh water is less dense, so the same depth is less pressure.
        let fresh = ambient_mbar(30_000, SURFACE_PRESSURE_MBAR, Salinity::Freshwater.value());
        let sea = ambient_mbar(30_000, SURFACE_PRESSURE_MBAR, Salinity::Seawater.value());
        assert!(fresh < sea);
    }
}
