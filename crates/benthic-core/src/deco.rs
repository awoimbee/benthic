//! Bühlmann ZH-L16 tissue model, ceilings and no-decompression limits.
//!
//! We use the same coefficient tables as Subsurface's planner (the ZH-L16B
//! set for compartment 1, despite Subsurface labelling the model "ZHL-16C").
//!
//! This is the foundation of the dive planner. It tracks nitrogen and helium
//! loading in the 16 ZH-L16 compartments and can answer the two questions a
//! planner needs most: "how long can I stay here?" (NDL) and "how deep is my
//! ceiling right now?" (decompression obligation).
//!
//! Gradient factors are applied the conventional way: `gf == 1.0` is the raw
//! M-value line, lower values are more conservative.

pub mod vpmb;

use serde::{Deserialize, Serialize};

use crate::gas::{
    ambient_mbar, he_fraction, n2_fraction, o2_fraction, GasMix, SURFACE_PRESSURE_MBAR,
};
use crate::units::{Depth, Duration, EN13319_SALINITY};

const COMPARTMENTS: usize = 16;
const LN2: f64 = std::f64::consts::LN_2;
/// Alveolar water-vapour pressure in bar (Subsurface's `WV_PRESSURE` for the
/// Bühlmann model). The VPM-B model uses the lower CO2-corrected Schreiner
/// value instead.
const WATER_VAPOUR_BAR: f64 = 0.0627;
/// Smallest pressure (above the surface) at which the low gradient factor may
/// be anchored (Subsurface's `gf_low_position_min`, one bar). Without this a
/// shallow dive would anchor the gradient factor at its (very shallow) actual
/// ceiling and produce more deco than Subsurface.
const GF_LOW_POSITION_MIN_BAR: f64 = 1.0;
// Subsurface's nitrogen half-times (minutes). Note that Subsurface labels its
// planner "ZHL-16C" but ships the ZH-L16B coefficients (compartment 1 only).
const N2_HALF_TIMES: [f64; COMPARTMENTS] = [
    5.0, 8.0, 12.5, 18.5, 27.0, 38.3, 54.3, 77.0, 109.0, 146.0, 187.0, 239.0, 305.0, 390.0, 498.0,
    635.0,
];
// Subsurface's helium half-times (minutes).
const HE_HALF_TIMES: [f64; COMPARTMENTS] = [
    1.88, 3.02, 4.72, 6.99, 10.21, 14.48, 20.53, 29.11, 41.20, 55.19, 70.69, 90.34, 115.29, 147.42,
    188.24, 240.03,
];
// Subsurface's nitrogen `a` coefficients (bar).
const N2_A: [f64; COMPARTMENTS] = [
    1.1696, 1.0, 0.8618, 0.7562, 0.62, 0.5043, 0.441, 0.4, 0.375, 0.35, 0.3295, 0.3065, 0.2835,
    0.261, 0.248, 0.2327,
];
// Subsurface's nitrogen `b` coefficients (dimensionless).
const N2_B: [f64; COMPARTMENTS] = [
    0.5578, 0.6514, 0.7222, 0.7825, 0.8126, 0.8434, 0.8693, 0.8910, 0.9092, 0.9222, 0.9319, 0.9403,
    0.9477, 0.9544, 0.9602, 0.9653,
];
// Subsurface's helium `a` coefficients (bar).
const HE_A: [f64; COMPARTMENTS] = [
    1.6189, 1.383, 1.1919, 1.0458, 0.922, 0.8205, 0.7305, 0.6502, 0.595, 0.5545, 0.5333, 0.5189,
    0.5181, 0.5176, 0.5172, 0.5119,
];
// Subsurface's helium `b` coefficients (dimensionless).
const HE_B: [f64; COMPARTMENTS] = [
    0.4770, 0.5747, 0.6527, 0.7223, 0.7582, 0.7957, 0.8279, 0.8553, 0.8757, 0.8903, 0.8997, 0.9073,
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

/// The decompression model used to plan a dive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum DecoModel {
    /// Bühlmann ZH-L16 with gradient factors.
    Buhlmann { gf_low: f64, gf_high: f64 },
    /// VPM-B at a conservatism level 0..=4.
    Vpmb { conservatism: u8 },
}

impl Default for DecoModel {
    fn default() -> Self {
        Self::Buhlmann {
            gf_low: 0.30,
            gf_high: 0.70,
        }
    }
}

/// One user-entered waypoint of a planned profile.
///
/// The segment runs from the previous waypoint to `depth` over `duration`,
/// breathing `mode`. The first point is therefore the descent from the
/// surface.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlanPoint {
    /// Depth reached at the end of the segment.
    pub depth: Depth,
    /// Time taken by the segment.
    pub duration: Duration,
    /// How the diver breathes during the segment.
    pub mode: BreathingMode,
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

/// Load the tissues at a constant ambient pressure for a number of minutes,
/// using the given alveolar water-vapour pressure.
fn load_inert_gas(tissues: &mut Tissues, p_amb: f64, minutes: f64, gas: GasMix, water_vapour: f64) {
    if minutes <= 0.0 {
        return;
    }
    let p_alv = (p_amb - water_vapour).max(0.0);
    let p_alv_n2 = p_alv * n2_fraction(gas);
    let p_alv_he = p_alv * he_fraction(gas);

    for i in 0..COMPARTMENTS {
        let k_n2 = LN2 / N2_HALF_TIMES[i];
        let k_he = LN2 / HE_HALF_TIMES[i];
        tissues.n2[i] = p_alv_n2 + (tissues.n2[i] - p_alv_n2) * (-k_n2 * minutes).exp();
        tissues.he[i] = p_alv_he + (tissues.he[i] - p_alv_he) * (-k_he * minutes).exp();
    }
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

/// A Bühlmann ZH-L16 model instance.
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
        let p_amb = ambient_mbar(depth.mm, self.surface_bar * 1000.0, self.salinity) / 1000.0;
        load_inert_gas(&mut self.tissues, p_amb, minutes, gas, WATER_VAPOUR_BAR);
    }

    /// Off-gas / on-gas at a constant depth for a duration.
    pub fn add_segment(&mut self, depth: Depth, duration: Duration, gas: GasMix) {
        self.add_segment_minutes(depth, duration.seconds as f64 / 60.0, gas);
    }

    /// The current decompression ceiling at the given gradient factor.
    ///
    /// A ceiling at or below the surface is reported as 0.
    pub fn ceiling_depth(&self, gf: f64) -> Depth {
        self.ceiling_depth_of(&self.tissues, gf)
    }

    /// The ceiling for an explicit tissue state (used by the schedule).
    /// The tolerated ambient pressure (bar) for a tissue state at a gradient
    /// factor, using Bühlmann's M-line with Baker's tissue-axis interpolation
    /// of the gradient factor.
    pub fn ceiling_bar_of(&self, tissues: &Tissues, gf: f64) -> f64 {
        let mut ceiling_bar = self.surface_bar;
        for i in 0..COMPARTMENTS {
            let p_n2 = tissues.n2[i];
            let p_he = tissues.he[i];
            let p_total = p_n2 + p_he;
            if p_total <= 0.0 {
                continue;
            }
            let a = (N2_A[i] * p_n2 + HE_A[i] * p_he) / p_total;
            let b = (N2_B[i] * p_n2 + HE_B[i] * p_he) / p_total;
            // Bühlmann's M-line is `P_tissue_tol = a + P / b`, so the raw
            // tolerated ambient pressure is `(P_tissue - a) * b`. The
            // gradient factor interpolates on the tissue axis (Baker):
            //   P_tol_gf = b * (P_tissue - GF * a) / (b + GF * (1 - b))
            let denominator = b + gf * (1.0 - b);
            let p_gf = if denominator > 0.0 {
                b * (p_total - gf * a) / denominator
            } else {
                0.0
            };
            if p_gf > ceiling_bar {
                ceiling_bar = p_gf;
            }
        }
        ceiling_bar
    }

    /// The ceiling for an explicit tissue state (used by the schedule).
    pub fn ceiling_depth_of(&self, tissues: &Tissues, gf: f64) -> Depth {
        let ceiling_bar = self.ceiling_bar_of(tissues, gf);
        let mm = crate::gas::depth_mm_at(
            ceiling_bar * 1000.0,
            self.surface_bar * 1000.0,
            self.salinity,
        );
        Depth::new(mm.max(0))
    }

    /// The pressure at which the low gradient factor is anchored for this
    /// tissue state: the deepest ceiling, but never shallower than
    /// `surface + GF_LOW_POSITION_MIN_BAR` (Subsurface's `gf_low_pressure_this_dive`).
    pub fn gf_anchor_bar(&self, tissues: &Tissues, gf_low: f64) -> f64 {
        self.ceiling_bar_of(tissues, gf_low)
            .max(self.surface_bar + GF_LOW_POSITION_MIN_BAR)
    }

    /// The ceiling (bar) for a full gradient-factor descent from a deep anchor
    /// to the surface, following Subsurface's `tissue_tolerance_calc` exactly.
    ///
    /// `anchor_bar` is the deepest ceiling of the dive (Subsurface's
    /// `gf_low_pressure_this_dive`); the gradient factor runs from `gf_low`
    /// there to `gf_high` at the surface.
    pub fn gf_ceiling_bar_of(
        &self,
        tissues: &Tissues,
        gf_low: f64,
        gf_high: f64,
        anchor_bar: f64,
    ) -> f64 {
        let surface = self.surface_bar;
        let mut ret = 0.0f64;
        for i in 0..COMPARTMENTS {
            let p_n2 = tissues.n2[i];
            let p_he = tissues.he[i];
            let p_total = p_n2 + p_he;
            if p_total <= 0.0 {
                continue;
            }
            let a = (N2_A[i] * p_n2 + HE_A[i] * p_he) / p_total;
            let b = (N2_B[i] * p_n2 + HE_B[i] * p_he) / p_total;

            let surface_tol = (surface / b + a - surface) * gf_high + surface;
            let anchor_tol = (anchor_bar / b + a - anchor_bar) * gf_low + anchor_bar;

            let tolerated = if surface_tol < anchor_tol {
                (-a * b * (gf_high * anchor_bar - gf_low * surface)
                    - (1.0 - b) * (gf_high - gf_low) * anchor_bar * surface
                    + b * (anchor_bar - surface) * p_total)
                    / (-a * b * (gf_high - gf_low)
                        + (1.0 - b) * (gf_low * anchor_bar - gf_high * surface)
                        + b * (anchor_bar - surface))
            } else {
                ret
            };
            if tolerated >= ret {
                ret = tolerated;
            }
        }
        ret
    }

    /// As [`Self::gf_ceiling_bar_of`], but as a depth.
    pub fn gf_ceiling_depth_of(
        &self,
        tissues: &Tissues,
        gf_low: f64,
        gf_high: f64,
        anchor_bar: f64,
    ) -> Depth {
        let bar = self.gf_ceiling_bar_of(tissues, gf_low, gf_high, anchor_bar);
        let mm = crate::gas::depth_mm_at(bar * 1000.0, self.surface_bar * 1000.0, self.salinity);
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

    /// Load a sequence of waypoints, ramping between them.
    pub fn add_points(&mut self, points: &[PlanPoint]) {
        let mut previous_m = 0.0;
        for point in points {
            self.add_transition(
                previous_m,
                point.depth.meters(),
                point.duration.seconds as f64 / 60.0,
                point.mode,
            );
            previous_m = point.depth.meters();
        }
    }

    /// The tissue state at the end of a profile.
    pub fn tissues_at_profile(&self, points: &[PlanPoint]) -> Tissues {
        let mut model = self.clone();
        model.add_points(points);
        model.tissues
    }

    /// The tissue state at the start of the ascent. Exposed so tooling can
    /// compare the initial ceiling with Subsurface's `first_ceiling_pressure`.
    pub fn tissues_at_ascent(&self, plan: &DecoSegment) -> Tissues {
        self.tissues_at_profile(&plan.points)
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
    /// Load the tissues while ramping from one depth to another over a fixed
    /// time.
    ///
    /// Subsurface integrates the entered profile in 2-second steps; we do the
    /// same so that the inert-gas loading matches.
    fn add_transition(&mut self, from_m: f64, to_m: f64, minutes: f64, mode: BreathingMode) {
        if minutes <= 0.0 {
            return;
        }
        let step = 2.0 / 60.0;
        let steps = (minutes / step).ceil().max(1.0) as usize;
        let dt = minutes / steps as f64;
        for i in 0..steps {
            let mid = from_m + (to_m - from_m) * (i as f64 + 0.5) / steps as f64;
            let depth = Depth::from_meters(mid);
            let ambient = self.ambient_bar(depth);
            load_inert_gas(
                &mut self.tissues,
                ambient,
                dt,
                mode.gas_at(ambient),
                WATER_VAPOUR_BAR,
            );
        }
    }

    /// Compute a decompression schedule after a bottom segment.
    ///
    /// The descent, ascent and the stops themselves are integrated in
    /// 2-second steps (Subsurface's `base_timestep`), so a stop lasts exactly
    /// as long as it takes the ceiling to clear rather than a whole number of
    /// minutes. Stops are on a `stop_step` grid and the gradient factor
    /// interpolates from `gf_low` at the first stop to `gf_high` at the
    /// surface.
    pub fn deco_schedule(&self, plan: &DecoSegment) -> Vec<Stop> {
        let tissues = self.tissues_at_profile(&plan.points);
        let start_m = plan.points.last().map(|p| p.depth.meters()).unwrap_or(0.0);
        let mode = plan.points.last().map(|p| p.mode).unwrap_or_default();
        self.schedule_from(tissues, start_m, mode, plan)
    }

    /// Compute the ascent and stops from an already-loaded tissue state.
    fn schedule_from(
        &self,
        mut tissues: Tissues,
        start_m: f64,
        mode: BreathingMode,
        plan: &DecoSegment,
    ) -> Vec<Stop> {
        // The gradient factor is anchored at the deepest ceiling of the dive
        // (Subsurface's `gf_low_pressure_this_dive`) and interpolated to the
        // surface.
        let anchor_bar = self.gf_anchor_bar(&tissues, plan.gf_low);

        // Ascend continuously, trying each 3 m step on a copy of the tissues.
        // This mirrors Subsurface's `trial_ascent`: an initial ceiling that is
        // deeper than the first stop can still be cleared while ascending,
        // because the fast tissues off-gas on the way up.
        let timestep = 2.0 / 60.0;
        let mut stops: Vec<Stop> = Vec::new();
        let mut depth_m = start_m;
        let mut holding: Option<(f64, f64)> = None;

        let flush = |holding: &mut Option<(f64, f64)>, stops: &mut Vec<Stop>| {
            if let Some((depth, minutes)) = holding.take() {
                stops.push(Stop {
                    depth: Depth::from_meters(depth),
                    duration: Duration::new((minutes * 60.0).round() as i32),
                });
            }
        };

        while depth_m > 1e-9 {
            let next = ((depth_m - 1e-9) / plan.stop_step).floor() * plan.stop_step;
            let ascent_minutes = (depth_m - next) / plan.ascent_rate;
            let steps = (ascent_minutes / timestep).ceil().max(1.0) as usize;
            let dt = ascent_minutes / steps as f64;

            let mut trial = tissues.clone();
            let mut clear = true;
            let mut d = depth_m;
            for i in 0..steps {
                let d_next = depth_m + (next - depth_m) * ((i + 1) as f64 / steps as f64);
                let mid = (d + d_next) / 2.0;
                let amb = self.ambient_bar(Depth::from_meters(mid));
                load_inert_gas(&mut trial, amb, dt, mode.gas_at(amb), WATER_VAPOUR_BAR);
                if self
                    .gf_ceiling_depth_of(&trial, plan.gf_low, plan.gf_high, anchor_bar)
                    .meters()
                    > d_next + 0.05
                {
                    clear = false;
                    break;
                }
                d = d_next;
            }

            if clear {
                tissues = trial;
                depth_m = next;
                flush(&mut holding, &mut stops);
            } else {
                let amb = self.ambient_bar(Depth::from_meters(depth_m));
                load_inert_gas(
                    &mut tissues,
                    amb,
                    timestep,
                    mode.gas_at(amb),
                    WATER_VAPOUR_BAR,
                );
                match &mut holding {
                    Some((_, minutes)) => *minutes += timestep,
                    None => holding = Some((depth_m, timestep)),
                }
            }
        }
        flush(&mut holding, &mut stops);
        stops
    }
}

/// The inputs to a decompression schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct DecoSegment {
    /// The entered profile: a sequence of waypoints.
    pub points: Vec<PlanPoint>,
    pub gf_low: f64,
    pub gf_high: f64,
    /// Distance between stops, in metres.
    pub stop_step: f64,
    /// Ascent rate, in metres per minute.
    pub ascent_rate: f64,
    /// Descent rate, in metres per minute. Only used to build a square profile.
    pub descent_rate: f64,
}

impl Default for DecoSegment {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            gf_low: 0.30,
            gf_high: 0.70,
            stop_step: 3.0,
            ascent_rate: 10.0,
            descent_rate: 20.0,
        }
    }
}

impl DecoSegment {
    /// A single square bottom segment: descend to `depth` at the default rate,
    /// then hold there for `minutes`.
    pub fn square(depth: Depth, minutes: f64, mode: BreathingMode) -> Self {
        let descent = Self::default();
        let descent_seconds = (depth.meters() / descent.descent_rate * 60.0).round() as i32;
        Self {
            points: vec![
                PlanPoint {
                    depth,
                    duration: Duration::new(descent_seconds),
                    mode,
                },
                PlanPoint {
                    depth,
                    duration: Duration::from_minutes(minutes.round() as i32),
                    mode,
                },
            ],
            ..descent
        }
    }

    /// The breathing mode of the last (deepest) waypoint.
    pub fn bottom_mode(&self) -> BreathingMode {
        self.points.last().map(|p| p.mode).unwrap_or_default()
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
    fn ceiling_follows_the_baker_gradient_factor_formula() {
        let mut model = Buhlmann::default();
        for i in 0..16 {
            model.tissues.n2[i] = 0.5;
            model.tissues.he[i] = 0.0;
        }
        // Make compartment 3 the controlling one.
        model.tissues.n2[3] = 3.0;

        let surface_mbar = model.surface_bar * 1000.0;
        let (a, b) = (N2_A[3], N2_B[3]);

        // Raw M-line (GF = 1): P_tol = (P - a) * b.
        let raw_bar = (3.0 - a) * b;
        let raw_mm = crate::gas::depth_mm_at(raw_bar * 1000.0, surface_mbar, model.salinity).max(0);
        assert_eq!(model.ceiling_depth(1.0).mm, raw_mm);

        // Baker's gradient factor: P_tol_gf = b * (P - GF*a) / (b + GF*(1-b)).
        let gf = 0.3;
        let gf_bar = b * (3.0 - gf * a) / (b + gf * (1.0 - b));
        let gf_mm = crate::gas::depth_mm_at(gf_bar * 1000.0, surface_mbar, model.salinity).max(0);
        assert_eq!(model.ceiling_depth(gf).mm, gf_mm);
        // A gradient factor below 1 must give a deeper ceiling.
        assert!(model.ceiling_depth(gf).mm > model.ceiling_depth(1.0).mm);
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
    fn ndl_matches_subsurface_values() {
        // Representative no-decompression limits on air, in minutes, matching
        // dive computers. Regression guard.
        let model = Buhlmann::default();
        for (depth_m, expected_minutes) in [(18.0, 60), (24.0, 29), (30.0, 17), (40.0, 9)] {
            let ndl = minutes(model.ndl(Depth::from_meters(depth_m), AIR, 1.0));
            assert!(
                (ndl - expected_minutes as f64).abs() <= 2.0,
                "{depth_m} m NDL was {ndl} min, expected ~{expected_minutes}"
            );
        }
    }

    #[test]
    fn matches_subsurface_reference_plans() {
        // Golden plans from Subsurface 6.0.5504's planner (planner-cli), sea
        // water, 1013 mbar surface, 20 m/min descent, 10 m/min ascent, 3 m
        // stop grid, last stop at 3 m, open circuit. The reference rounds each
        // stop's absolute end time up to a 60 s grid, so its durations are up
        // to a few minutes longer than the true minimum we compute. The
        // (exact) first ceiling is therefore compared tightly and the stop
        // durations within three minutes.
        struct Reference {
            depth: f64,
            minutes: f64,
            gf_low: f64,
            gf_high: f64,
            gas: GasMix,
            first_ceiling_m: f64,
            stops: &'static [(f64, f64)],
        }

        let ean32 = GasMix::new(320, 0);
        let cases = [
            Reference {
                depth: 40.0,
                minutes: 40.0,
                gf_low: 1.0,
                gf_high: 1.0,
                gas: AIR,
                first_ceiling_m: 9.65,
                stops: &[(9.0, 6.9), (6.0, 14.7), (3.0, 28.7)],
            },
            Reference {
                depth: 40.0,
                minutes: 40.0,
                gf_low: 0.3,
                gf_high: 0.7,
                gas: AIR,
                first_ceiling_m: 20.27,
                stops: &[
                    (21.0, 2.1),
                    (18.0, 3.7),
                    (15.0, 6.7),
                    (12.0, 9.7),
                    (9.0, 16.7),
                    (6.0, 28.7),
                    (3.0, 54.7),
                ],
            },
            Reference {
                depth: 30.0,
                minutes: 30.0,
                gf_low: 0.3,
                gf_high: 0.7,
                gas: ean32,
                first_ceiling_m: 8.69,
                stops: &[(9.0, 1.37), (6.0, 2.7), (3.0, 6.7)],
            },
            Reference {
                depth: 18.0,
                minutes: 60.0,
                gf_low: 0.3,
                gf_high: 0.7,
                gas: AIR,
                first_ceiling_m: 4.41,
                stops: &[(6.0, 2.87), (3.0, 16.7)],
            },
            Reference {
                depth: 60.0,
                minutes: 20.0,
                gf_low: 0.3,
                gf_high: 0.7,
                gas: AIR,
                first_ceiling_m: 29.5,
                stops: &[
                    (27.0, 1.67),
                    (24.0, 1.7),
                    (21.0, 3.7),
                    (18.0, 3.7),
                    (15.0, 5.7),
                    (12.0, 8.7),
                    (9.0, 14.7),
                    (6.0, 26.7),
                    (3.0, 51.7),
                ],
            },
        ];

        // The reference plans use sea water (10300 g per 10 L) and a 1013 mbar
        // surface.
        let model = Buhlmann::new(SURFACE_PRESSURE_MBAR / 1000.0, 10_300);
        for c in &cases {
            let plan = DecoSegment {
                points: DecoSegment::square(
                    Depth::from_meters(c.depth),
                    c.minutes,
                    BreathingMode::OpenCircuit(c.gas),
                )
                .points,
                gf_low: c.gf_low,
                gf_high: c.gf_high,
                ..Default::default()
            };

            let tissues = model.tissues_at_ascent(&plan);
            let ceiling = model.gf_ceiling_depth_of(
                &tissues,
                c.gf_low,
                c.gf_high,
                model.gf_anchor_bar(&tissues, c.gf_low),
            );
            assert!(
                (ceiling.meters() - c.first_ceiling_m).abs() < 0.05,
                "{} m/{} min: first ceiling {:.3} m, Subsurface {:.2} m",
                c.depth,
                c.minutes,
                ceiling.meters(),
                c.first_ceiling_m
            );

            let stops = model.deco_schedule(&plan);
            assert_eq!(
                stops.len(),
                c.stops.len(),
                "{} m/{} min: stop count differs from Subsurface",
                c.depth,
                c.minutes
            );
            for (stop, (exp_depth, exp_minutes)) in stops.iter().zip(c.stops.iter()) {
                assert_eq!(
                    stop.depth,
                    Depth::from_meters(*exp_depth),
                    "{} m/{} min: stop depth",
                    c.depth,
                    c.minutes
                );
                let minutes = stop.duration.seconds as f64 / 60.0;
                assert!(
                    (minutes - exp_minutes).abs() <= 3.0,
                    "{} m/{} min: {exp_depth} m stop was {minutes} min, Subsurface {exp_minutes}",
                    c.depth,
                    c.minutes
                );
            }
        }
    }

    #[test]
    fn forty_metre_gf100_schedule_is_stable() {
        let stops = Buhlmann::default().deco_schedule(&DecoSegment {
            points: DecoSegment::square(
                Depth::from_meters(40.0),
                40.0,
                BreathingMode::OpenCircuit(AIR),
            )
            .points,
            gf_low: 1.0,
            gf_high: 1.0,
            ..Default::default()
        });
        let total: i32 = stops.iter().map(|s| s.duration.seconds).sum();
        assert_eq!(
            stops.first().map(|s| s.depth),
            Some(Depth::from_meters(9.0))
        );
        assert_eq!(total, 47 * 60 + 54);
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
            points: DecoSegment::square(
                Depth::from_meters(40.0),
                40.0,
                BreathingMode::OpenCircuit(AIR),
            )
            .points,
            gf_low: 1.0,
            gf_high: 1.0,
            ..Default::default()
        });
        let ccr = model.deco_schedule(&DecoSegment {
            points: DecoSegment::square(
                Depth::from_meters(40.0),
                40.0,
                BreathingMode::ClosedCircuit {
                    diluent: AIR,
                    setpoint_bar: 1.3,
                },
            )
            .points,
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
            points: DecoSegment::square(
                Depth::from_meters(40.0),
                40.0,
                BreathingMode::OpenCircuit(AIR),
            )
            .points,
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
        // Deeper stops are shorter than the shallow ones on a square profile.
        assert!(stops[0].duration.seconds <= stops[stops.len() - 1].duration.seconds);
    }

    #[test]
    fn shallow_dive_needs_no_stops() {
        let model = Buhlmann::default();
        assert!(model
            .deco_schedule(&DecoSegment {
                points: DecoSegment::square(
                    Depth::from_meters(15.0),
                    20.0,
                    BreathingMode::OpenCircuit(AIR)
                )
                .points,
                gf_low: 1.0,
                gf_high: 1.0,
                ..Default::default()
            })
            .is_empty());
    }

    #[test]
    fn conservative_gradient_factors_add_deco() {
        let model = Buhlmann::default();
        let square = DecoSegment::square(
            Depth::from_meters(40.0),
            35.0,
            BreathingMode::OpenCircuit(AIR),
        );
        let liberal = model.deco_schedule(&DecoSegment {
            points: square.points.clone(),
            gf_low: 0.9,
            gf_high: 0.9,
            ..Default::default()
        });
        let conservative = model.deco_schedule(&DecoSegment {
            points: square.points,
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
