//! Bühlmann ZH-L16C tissue model, ceilings and no-decompression limits.
//!
//! This is the foundation of the dive planner. It tracks nitrogen and helium
//! loading in the 16 ZH-L16 compartments and can answer the two questions a
//! planner needs most: "how long can I stay here?" (NDL) and "how deep is my
//! ceiling right now?" (decompression obligation).
//!
//! Gradient factors are applied the conventional way: `gf == 1.0` is the raw
//! M-value line, lower values are more conservative.

use serde::{Deserialize, Serialize};

use crate::gas::{ambient_mbar, he_fraction, n2_fraction, GasMix, SURFACE_PRESSURE_MBAR};
use crate::units::{Depth, Duration, EN13319_SALINITY};

const COMPARTMENTS: usize = 16;
const LN2: f64 = std::f64::consts::LN_2;
/// Alveolar water-vapour pressure in bar.
const WATER_VAPOUR_BAR: f64 = 0.0627;

// ZH-L16C nitrogen half-times (minutes).
const N2_HALF_TIMES: [f64; COMPARTMENTS] = [
    4.0, 8.0, 12.5, 18.5, 27.0, 38.3, 54.3, 77.0, 109.0, 146.0, 187.0, 239.0, 305.0, 390.0, 498.0,
    635.0,
];
// ZH-L16C helium half-times (minutes).
const HE_HALF_TIMES: [f64; COMPARTMENTS] = [
    1.88, 3.02, 4.72, 6.99, 10.21, 14.48, 20.53, 29.11, 41.20, 55.19, 70.69, 90.34, 115.29, 147.42,
    188.24, 240.03,
];
// ZH-L16C nitrogen `a` coefficients (bar).
const N2_A: [f64; COMPARTMENTS] = [
    1.2599, 1.0000, 0.8618, 0.7562, 0.6200, 0.5043, 0.4410, 0.4000, 0.3750, 0.3500, 0.3295, 0.3065,
    0.2835, 0.2610, 0.2480, 0.2327,
];
// ZH-L16C nitrogen `b` coefficients (dimensionless).
const N2_B: [f64; COMPARTMENTS] = [
    0.5050, 0.6514, 0.7222, 0.7825, 0.8126, 0.8434, 0.8693, 0.8910, 0.9092, 0.9222, 0.9319, 0.9403,
    0.9477, 0.9544, 0.9602, 0.9653,
];
// ZH-L16C helium `a` coefficients (bar).
const HE_A: [f64; COMPARTMENTS] = [
    1.7424, 1.3830, 1.1919, 1.0458, 0.9220, 0.8205, 0.7305, 0.6502, 0.5950, 0.5545, 0.5333, 0.5189,
    0.5181, 0.5176, 0.5172, 0.5119,
];
// ZH-L16C helium `b` coefficients (dimensionless).
const HE_B: [f64; COMPARTMENTS] = [
    0.4245, 0.5747, 0.6527, 0.7223, 0.7582, 0.7957, 0.8279, 0.8553, 0.8757, 0.8903, 0.8997, 0.9073,
    0.9122, 0.9171, 0.9217, 0.9267,
];

/// Inert-gas loading of the 16 Bühlmann compartments, in bar (partial
/// pressures).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tissues {
    pub n2: [f64; COMPARTMENTS],
    pub he: [f64; COMPARTMENTS],
}

impl Default for Tissues {
    /// A diver equilibrated at the surface on air.
    fn default() -> Self {
        let p_alv = (SURFACE_PRESSURE_MBAR / 1000.0 - WATER_VAPOUR_BAR) * 0.7902;
        Self {
            n2: [p_alv; COMPARTMENTS],
            he: [0.0; COMPARTMENTS],
        }
    }
}

/// A Bühlmann ZH-L16C model instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Buhlmann {
    pub tissues: Tissues,
    /// Surface pressure in bar.
    pub surface_bar: f64,
    /// Water salinity (grams per 10 litres).
    pub salinity: i32,
}

impl Default for Buhlmann {
    fn default() -> Self {
        Self::new(SURFACE_PRESSURE_MBAR / 1000.0, EN13319_SALINITY)
    }
}

impl Buhlmann {
    pub fn new(surface_bar: f64, salinity: i32) -> Self {
        Self {
            tissues: Tissues::default(),
            surface_bar,
            salinity,
        }
    }

    /// Off-gas / on-gas at a constant depth for a number of minutes.
    pub fn add_segment_minutes(&mut self, depth: Depth, minutes: f64, gas: GasMix) {
        if minutes <= 0.0 {
            return;
        }
        let p_amb = ambient_mbar(depth.mm, self.surface_bar * 1000.0, self.salinity) / 1000.0;
        let p_alv = (p_amb - WATER_VAPOUR_BAR).max(0.0);
        let p_alv_n2 = p_alv * n2_fraction(gas);
        let p_alv_he = p_alv * he_fraction(gas);

        for i in 0..COMPARTMENTS {
            let k_n2 = LN2 / N2_HALF_TIMES[i];
            let k_he = LN2 / HE_HALF_TIMES[i];
            self.tissues.n2[i] =
                p_alv_n2 + (self.tissues.n2[i] - p_alv_n2) * (-k_n2 * minutes).exp();
            self.tissues.he[i] =
                p_alv_he + (self.tissues.he[i] - p_alv_he) * (-k_he * minutes).exp();
        }
    }

    /// Off-gas / on-gas at a constant depth for a duration.
    pub fn add_segment(&mut self, depth: Depth, duration: Duration, gas: GasMix) {
        self.add_segment_minutes(depth, duration.seconds as f64 / 60.0, gas);
    }

    /// The current decompression ceiling at the given gradient factor.
    ///
    /// A ceiling at or below the surface is reported as 0.
    pub fn ceiling_depth(&self, gf: f64) -> Depth {
        let mut ceiling_bar = self.surface_bar;
        for i in 0..COMPARTMENTS {
            let p_n2 = self.tissues.n2[i];
            let p_he = self.tissues.he[i];
            let p_total = p_n2 + p_he;
            if p_total <= 0.0 {
                continue;
            }
            let a = (N2_A[i] * p_n2 + HE_A[i] * p_he) / p_total;
            let b = (N2_B[i] * p_n2 + HE_B[i] * p_he) / p_total;
            // Ambient pressure tolerated by this compartment at the raw limit.
            let p_tol = (p_total - a) * b;
            // Gradient-factor-adjusted tolerated ambient pressure.
            let p_gf = p_total - gf * (p_total - p_tol);
            if p_gf > ceiling_bar {
                ceiling_bar = p_gf;
            }
        }
        let mm = crate::gas::depth_mm_at(
            ceiling_bar * 1000.0,
            self.surface_bar * 1000.0,
            self.salinity,
        );
        Depth::new(mm.max(0))
    }

    /// The no-decompression limit at a constant depth, or `None` when it
    /// exceeds 24 hours.
    pub fn ndl(&self, depth: Depth, gas: GasMix, gf: f64) -> Option<Duration> {
        let mut tissues = self.clone();
        let step: f64 = 1.0 / 6.0; // 10 seconds
        let mut minutes: f64 = 0.0;
        loop {
            if tissues.ceiling_depth(gf).mm > 0 {
                return Some(Duration::new((minutes * 60.0).round() as i32));
            }
            if minutes >= 24.0 * 60.0 {
                return None;
            }
            tissues.add_segment_minutes(depth, step, gas);
            minutes += step;
        }
    }

    /// Run a sequence of constant-depth segments.
    pub fn add_profile(&mut self, segments: &[(Depth, Duration, GasMix)]) {
        for (depth, duration, gas) in segments {
            self.add_segment(*depth, *duration, *gas);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::AIR;

    fn minutes(limit: Option<Duration>) -> f64 {
        limit
            .map(|d| d.seconds as f64 / 60.0)
            .unwrap_or(f64::INFINITY)
    }

    #[test]
    fn surface_is_equilibrated() {
        let model = Buhlmann::default();
        assert_eq!(model.ceiling_depth(1.0), Depth::ZERO);
    }

    #[test]
    fn ndl_decreases_with_depth() {
        let model = Buhlmann::default();
        let shallow = minutes(model.ndl(Depth::from_meters(18.0), AIR, 1.0));
        let medium = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        let deep = minutes(model.ndl(Depth::from_meters(40.0), AIR, 1.0));
        assert!(shallow > medium, "{shallow} should exceed {medium}");
        assert!(medium > deep, "{medium} should exceed {deep}");
    }

    #[test]
    fn ndl_is_in_a_sensible_range() {
        let model = Buhlmann::default();
        // Air at 30 m is a classic "around 20 minutes" NDL on the raw M-line.
        let ndl = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        assert!((15.0..30.0).contains(&ndl), "30 m NDL was {ndl}");
        // At 40 m it drops to roughly ten minutes.
        let ndl = minutes(model.ndl(Depth::from_meters(40.0), AIR, 1.0));
        assert!((6.0..15.0).contains(&ndl), "40 m NDL was {ndl}");
    }

    #[test]
    fn lower_gradient_factor_shortens_ndl() {
        let model = Buhlmann::default();
        let conservative = minutes(model.ndl(Depth::from_meters(30.0), AIR, 0.7));
        let raw = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        assert!(conservative < raw, "{conservative} should be below {raw}");
        assert!(conservative > 0.0);
    }

    #[test]
    fn nitrox_extends_ndl() {
        let model = Buhlmann::default();
        let air = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        let ean36 = minutes(model.ndl(Depth::from_meters(30.0), GasMix::percent(36.0, 0.0), 1.0));
        assert!(ean36 > air, "EAN36 {ean36} should exceed air {air}");
    }

    #[test]
    fn long_deep_exposure_creates_a_ceiling() {
        let mut model = Buhlmann::default();
        model.add_segment(Depth::from_meters(40.0), Duration::from_minutes(40), AIR);
        assert!(
            model.ceiling_depth(1.0).mm > 0,
            "expected a ceiling after a long deep dive"
        );
    }

    #[test]
    fn off_gassing_clears_the_ceiling() {
        let mut model = Buhlmann::default();
        model.add_segment(Depth::from_meters(40.0), Duration::from_minutes(40), AIR);
        for _ in 0..12 {
            model.add_segment_minutes(Depth::from_meters(3.0), 10.0, AIR);
        }
        assert_eq!(model.ceiling_depth(1.0), Depth::ZERO);
    }
}
