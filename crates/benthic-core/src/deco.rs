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

use crate::gas::{
    ambient_mbar, he_fraction, n2_fraction, o2_fraction, GasMix, SURFACE_PRESSURE_MBAR,
};
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

/// Parameters of a passive semi-closed rebreather (pSCR).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PscrParams {
    /// Metabolic oxygen consumption, ml/min.
    pub o2_consumption_ml_min: f64,
    /// Surface ventilation (SAC), ml/min.
    pub sac_ml_min: f64,
    /// Dump ratio times 1000 (100 = 1:10), matching Subsurface's convention.
    pub dump_ratio: f64,
}

impl Default for PscrParams {
    fn default() -> Self {
        Self {
            o2_consumption_ml_min: 720.0,
            sac_ml_min: 20_000.0,
            dump_ratio: 100.0,
        }
    }
}

impl PscrParams {
    /// The steady-state loop pO2 in bar at a given ambient pressure.
    ///
    /// This mirrors Subsurface's model: the diluent's oxygen partial pressure
    /// is reduced by the metabolic consumption relative to the fresh-gas flow
    /// (`sac * dump_ratio`). The result is clamped at zero.
    pub fn loop_po2_bar(&self, diluent: GasMix, ambient_bar: f64) -> f64 {
        let o2_permille = o2_fraction(diluent) * 1000.0;
        let p_mbar = o2_permille * ambient_bar
            - (1.0 - o2_permille / 1000.0) * self.o2_consumption_ml_min
                / (self.sac_ml_min * self.dump_ratio)
                * 1_000_000.0;
        p_mbar.max(0.0) / 1000.0
    }
}

/// Build a loop gas from a diluent and a target oxygen fraction, preserving the
/// diluent's helium-to-nitrogen ratio.
fn loop_from_o2(diluent: GasMix, f_o2: f64) -> GasMix {
    let f_o2 = f_o2.clamp(0.0, 1.0);
    let f_o2_dil = o2_fraction(diluent);
    let f_he = if f_o2_dil < 1.0 {
        he_fraction(diluent) * (1.0 - f_o2) / (1.0 - f_o2_dil)
    } else {
        0.0
    };
    GasMix::new(
        (f_o2 * 1000.0).round() as u16,
        (f_he.max(0.0) * 1000.0).round() as u16,
    )
}

/// The closed-circuit loop gas at an ambient pressure and pO2 setpoint.
pub fn loop_gas(diluent: GasMix, ambient_bar: f64, setpoint_bar: f64) -> GasMix {
    loop_from_o2(diluent, setpoint_bar / ambient_bar.max(0.001))
}

/// The passive semi-closed loop gas at an ambient pressure, following
/// Subsurface's pSCR model.
pub fn pscr_loop_gas(diluent: GasMix, ambient_bar: f64, params: &PscrParams) -> GasMix {
    let p_amb = ambient_bar.max(0.001);
    let po2 = params.loop_po2_bar(diluent, p_amb);
    loop_from_o2(diluent, po2 / p_amb)
}

/// How the diver breathes during a segment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BreathingMode {
    /// Open circuit on a fixed mix.
    OpenCircuit(GasMix),
    /// Closed circuit on a diluent with a pO2 setpoint.
    ClosedCircuit { diluent: GasMix, setpoint_bar: f64 },
    /// Passive semi-closed rebreather on a diluent.
    PassiveSemiClosed { diluent: GasMix, params: PscrParams },
}

impl Default for BreathingMode {
    fn default() -> Self {
        BreathingMode::OpenCircuit(crate::gas::AIR)
    }
}

impl BreathingMode {
    /// The inspired gas at a given ambient pressure.
    pub fn gas_at(&self, ambient_bar: f64) -> GasMix {
        match self {
            BreathingMode::OpenCircuit(gas) => *gas,
            BreathingMode::ClosedCircuit {
                diluent,
                setpoint_bar,
            } => loop_gas(*diluent, ambient_bar, *setpoint_bar),
            BreathingMode::PassiveSemiClosed { diluent, params } => {
                pscr_loop_gas(*diluent, ambient_bar, params)
            }
        }
    }

    /// The gas carried in the cylinder (the mix itself, or the diluent).
    pub fn cylinder_gas(&self) -> GasMix {
        match self {
            BreathingMode::OpenCircuit(gas) => *gas,
            BreathingMode::ClosedCircuit { diluent, .. } => *diluent,
            BreathingMode::PassiveSemiClosed { diluent, .. } => *diluent,
        }
    }

    /// Whether this is a rebreather (closed or semi-closed) mode.
    pub fn is_rebreather(&self) -> bool {
        !matches!(self, BreathingMode::OpenCircuit(_))
    }
}

/// A decompression stop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// Stop depth.
    pub depth: Depth,
    /// Time spent at the stop.
    pub duration: Duration,
}

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

    /// Compute a decompression schedule after a bottom segment.
    ///
    /// Ascends in steps of `stop_step` metres at `ascent_rate`, holding at each
    /// stop until the gradient-factor-adjusted ceiling allows the next step.
    /// The gradient factor interpolates linearly from `gf_low` at the first
    /// stop to `gf_high` at the surface. Returns the stops deepest-first; the
    /// Ambient pressure at a depth, in bar.
    pub fn ambient_bar(&self, depth: Depth) -> f64 {
        ambient_mbar(depth.mm, self.surface_bar * 1000.0, self.salinity) / 1000.0
    }

    /// Off-gas / on-gas at a constant depth for a number of minutes in the
    /// given breathing mode.
    pub fn add_segment_mode(&mut self, depth: Depth, minutes: f64, mode: BreathingMode) {
        let gas = mode.gas_at(self.ambient_bar(depth));
        self.add_segment_minutes(depth, minutes, gas);
    }

    /// The no-decompression limit at a constant depth in the given breathing
    /// mode.
    pub fn ndl_mode(&self, depth: Depth, mode: BreathingMode, gf: f64) -> Option<Duration> {
        let gas = mode.gas_at(self.ambient_bar(depth));
        self.ndl(depth, gas, gf)
    }

    /// Compute a decompression schedule after a bottom segment.
    ///
    /// Ascends in steps of `stop_step` metres at `ascent_rate`, holding at each
    /// stop until the gradient-factor-adjusted ceiling allows the next step.
    /// The gradient factor interpolates linearly from `gf_low` at the first
    /// stop to `gf_high` at the surface. Returns the stops deepest-first; the
    /// bottom itself is not included.
    pub fn deco_schedule(&self, plan: &DecoSegment) -> Vec<Stop> {
        let mut tissues = self.clone();
        let bottom_gas = plan.mode.gas_at(self.ambient_bar(plan.bottom_depth));
        tissues.add_segment_minutes(plan.bottom_depth, plan.bottom_minutes, bottom_gas);

        let ceiling = tissues.ceiling_depth(plan.gf_low).meters();
        if ceiling <= 0.0 {
            return Vec::new();
        }
        let first_stop = (ceiling / plan.stop_step).ceil() * plan.stop_step;

        let gf_at = |depth: f64| {
            if first_stop <= 0.0 {
                plan.gf_high
            } else {
                let frac = (depth / first_stop).clamp(0.0, 1.0);
                plan.gf_high + (plan.gf_low - plan.gf_high) * frac
            }
        };

        let mut stops = Vec::new();
        let mut depth = first_stop;
        while depth >= plan.stop_step - 1e-9 {
            let next = (depth - plan.stop_step).max(0.0);
            let gf_next = gf_at(next);
            let mut minutes: f64 = 0.0;
            while tissues.ceiling_depth(gf_next).meters() > next + 0.05 {
                let gas = plan
                    .mode
                    .gas_at(self.ambient_bar(Depth::from_meters(depth)));
                tissues.add_segment_minutes(Depth::from_meters(depth), 1.0, gas);
                minutes += 1.0;
                if minutes > 600.0 {
                    break;
                }
            }
            if minutes >= 1.0 {
                stops.push(Stop {
                    depth: Depth::from_meters(depth),
                    duration: Duration::from_minutes(minutes.round() as i32),
                });
            }
            let mid = (depth + next) / 2.0;
            let ascent_minutes = (depth - next) / plan.ascent_rate;
            let gas = plan.mode.gas_at(self.ambient_bar(Depth::from_meters(mid)));
            tissues.add_segment_minutes(Depth::from_meters(mid), ascent_minutes, gas);
            depth = next;
        }
        stops
    }
}

/// The inputs to a decompression schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecoSegment {
    pub bottom_depth: Depth,
    pub bottom_minutes: f64,
    pub mode: BreathingMode,
    pub gf_low: f64,
    pub gf_high: f64,
    /// Distance between stops, in metres.
    pub stop_step: f64,
    /// Ascent rate, in metres per minute.
    pub ascent_rate: f64,
}

impl Default for DecoSegment {
    fn default() -> Self {
        Self {
            bottom_depth: Depth::from_meters(30.0),
            bottom_minutes: 20.0,
            mode: BreathingMode::default(),
            gf_low: 0.30,
            gf_high: 0.70,
            stop_step: 3.0,
            ascent_rate: 10.0,
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

    #[test]
    fn ccr_loop_is_oxygen_rich() {
        // At 30 m (about 4 bar) with a 1.3 bar setpoint, the loop is ~32% O2.
        let gas = loop_gas(AIR, 4.0, 1.3);
        let fo2 = gas.o2_permille as f64 / 1000.0;
        assert!((0.30..0.35).contains(&fo2), "loop O2 was {fo2}");
        assert_eq!(gas.he_permille, 0);
        // A trimix diluent keeps some helium in the loop.
        let gas = loop_gas(GasMix::percent(18.0, 45.0), 6.0, 1.3);
        assert!(gas.he_permille > 0, "expected helium in the loop");
    }

    #[test]
    fn ccr_ndl_exceeds_open_circuit() {
        let model = Buhlmann::default();
        let oc = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        let ccr = minutes(model.ndl_mode(
            Depth::from_meters(30.0),
            BreathingMode::ClosedCircuit {
                diluent: AIR,
                setpoint_bar: 1.3,
            },
            1.0,
        ));
        assert!(ccr > oc, "CCR {ccr} should exceed OC {oc}");
    }

    #[test]
    fn pscr_loop_dilutes_oxygen_at_depth() {
        // At 30 m the metabolic consumption dilutes the air diluent below 21%.
        let params = PscrParams::default();
        let ambient = 4.0;
        let po2 = params.loop_po2_bar(AIR, ambient);
        let fo2 = po2 / ambient;
        assert!(fo2 > 0.0 && fo2 < 0.21, "pSCR loop O2 was {fo2}");
        // And the loop gas reflects that.
        let gas = pscr_loop_gas(AIR, ambient, &params);
        assert!(gas.o2_permille < 210);
        // At the surface the model clamps the depleted loop at zero.
        assert_eq!(params.loop_po2_bar(AIR, 1.0), 0.0);
    }

    #[test]
    fn pscr_ndl_is_shorter_than_open_circuit_on_the_same_mix() {
        // Dilution means the loop carries more inert gas than the diluent.
        let model = Buhlmann::default();
        let oc = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        let pscr = minutes(model.ndl_mode(
            Depth::from_meters(30.0),
            BreathingMode::PassiveSemiClosed {
                diluent: AIR,
                params: PscrParams::default(),
            },
            1.0,
        ));
        assert!(pscr < oc, "pSCR {pscr} should be below OC {oc}");
        assert!(pscr > 0.0);
    }

    #[test]
    fn pscr_with_nitrox_beats_open_circuit_air() {
        let model = Buhlmann::default();
        let oc_air = minutes(model.ndl(Depth::from_meters(30.0), AIR, 1.0));
        let pscr_ean36 = minutes(model.ndl_mode(
            Depth::from_meters(30.0),
            BreathingMode::PassiveSemiClosed {
                diluent: GasMix::percent(36.0, 0.0),
                params: PscrParams::default(),
            },
            1.0,
        ));
        assert!(
            pscr_ean36 > oc_air,
            "pSCR EAN36 {pscr_ean36} should exceed OC air {oc_air}"
        );
    }

    #[test]
    fn ccr_schedule_uses_the_loop_gas() {
        let model = Buhlmann::default();
        let oc = model.deco_schedule(&DecoSegment {
            bottom_depth: Depth::from_meters(40.0),
            bottom_minutes: 40.0,
            mode: BreathingMode::OpenCircuit(AIR),
            gf_low: 1.0,
            gf_high: 1.0,
            ..Default::default()
        });
        let ccr = model.deco_schedule(&DecoSegment {
            bottom_depth: Depth::from_meters(40.0),
            bottom_minutes: 40.0,
            mode: BreathingMode::ClosedCircuit {
                diluent: AIR,
                setpoint_bar: 1.3,
            },
            gf_low: 1.0,
            gf_high: 1.0,
            ..Default::default()
        });
        assert!(
            total_deco(&ccr) < total_deco(&oc),
            "CCR {} should be less than OC {}",
            total_deco(&ccr),
            total_deco(&oc)
        );
    }

    fn total_deco(stops: &[Stop]) -> i32 {
        stops.iter().map(|s| s.duration.seconds).sum()
    }

    #[test]
    fn deep_dive_gets_a_schedule() {
        let model = Buhlmann::default();
        let segment = DecoSegment {
            bottom_depth: Depth::from_meters(40.0),
            bottom_minutes: 40.0,
            gf_low: 1.0,
            gf_high: 1.0,
            ..Default::default()
        };
        let stops = model.deco_schedule(&segment);
        assert!(!stops.is_empty());
        for stop in &stops {
            assert_eq!(
                stop.depth.mm % 3_000,
                0,
                "stop {} is not a 3 m multiple",
                stop.depth.mm
            );
            assert!(stop.duration.seconds >= 60);
        }
        // Deepest first, and never deeper than the bottom.
        assert!(stops[0].depth.mm >= stops[stops.len() - 1].depth.mm);
        assert!(stops[0].depth.mm <= 40_000);
    }

    #[test]
    fn shallow_dive_needs_no_stops() {
        let model = Buhlmann::default();
        assert!(model
            .deco_schedule(&DecoSegment {
                bottom_depth: Depth::from_meters(15.0),
                bottom_minutes: 20.0,
                gf_low: 1.0,
                gf_high: 1.0,
                ..Default::default()
            })
            .is_empty());
    }

    #[test]
    fn conservative_gradient_factors_add_deco() {
        let model = Buhlmann::default();
        let liberal = model.deco_schedule(&DecoSegment {
            bottom_depth: Depth::from_meters(40.0),
            bottom_minutes: 35.0,
            gf_low: 0.9,
            gf_high: 0.9,
            ..Default::default()
        });
        let conservative = model.deco_schedule(&DecoSegment {
            bottom_depth: Depth::from_meters(40.0),
            bottom_minutes: 35.0,
            gf_low: 0.3,
            gf_high: 0.3,
            ..Default::default()
        });
        assert!(
            total_deco(&conservative) > total_deco(&liberal),
            "conservative {} should exceed liberal {}",
            total_deco(&conservative),
            total_deco(&liberal)
        );
    }
}
