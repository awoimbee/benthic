//! VPM-B (Varying Permeability Model, Baker's "B" variant).
//!
//! A faithful port of Subsurface's `core/deco.cpp` plus the VPM-B parts of
//! `core/planner.cpp`, so the planner reproduces Subsurface's schedules. It is
//! only used by the planner: real dives still use the Bühlmann model.
//!
//! Units follow Subsurface: depths are integer millimetres internally (the
//! ascent integrator reproduces Subsurface's integer arithmetic exactly),
//! pressures are bar, and times are seconds or minutes. The 16-compartment
//! tissue state is shared with [`super::Buhlmann`], but VPM-B loads it with the
//! Schreiner alveolar water-vapour pressure (0.0493 bar) and additionally
//! tracks bubble nuclei.

use crate::gas::{ambient_mbar, depth_mm_at, GasMix};
use crate::units::{Depth, Duration};

use super::{
    load_inert_gas, BreathingMode, DecoSegment, Stop, Tissues, COMPARTMENTS, HE_HALF_TIMES, LN2,
    N2_HALF_TIMES,
};

/// Schreiner alveolar water-vapour pressure (bar), used by VPM-B because it
/// assumes a respiratory quotient of 0.8.
const WV_PRESSURE_SCHREINER: f64 = 0.0493;
/// Nitrogen fraction used for the surface inspired pressure.
const NITROGEN_FRACTION: f64 = 0.79;
/// Subsurface initialises the tissues with 781 per mille rather than air's 790.
const N2_IN_AIR: f64 = 781.0;
/// Always-present pressure of other gases in the tissues (bar).
const OTHER_GASES_PRESSURE: f64 = 0.1359888;
const CRIT_RADIUS_N2: f64 = 0.55;
const CRIT_RADIUS_HE: f64 = 0.45;
/// Constant corresponding to the critical gas volume (bar * min).
const CRIT_VOLUME_LAMBDA: f64 = 199.58;
/// Gradient after which bubbles become impermeable (bar).
const GRADIENT_OF_IMPERM: f64 = 8.30865;
/// Nuclei surface tension constant (N / bar = m2).
const SURFACE_TENSION_GAMMA: f64 = 0.18137175;
/// Skin compression `gammaC` (N / bar = m2).
const SKIN_COMPRESSION_GAMMA_C: f64 = 2.6040525;
/// Time for a bubble to regenerate to its starting radius (minutes).
const REGENERATION_TIME: f64 = 20160.0;
/// Conservatism multipliers, indexed by the 0..=4 conservatism level.
const CONSERVATISM_LEVELS: [f64; 5] = [1.0, 1.05, 1.12, 1.22, 1.35];
/// Subsurface's `base_timestep`: ascent integration step in seconds.
const BASE_TIMESTEP: i32 = 2;

/// A VPM-B model instance: the tissue state plus the bubble-nuclei state that
/// the Bühlmann model does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct Vpmb {
    pub tissues: Tissues,
    pub surface_bar: f64,
    pub salinity: i32,
    crit_radius_n2: f64,
    crit_radius_he: f64,
    max_n2_crushing: [f64; COMPARTMENTS],
    max_he_crushing: [f64; COMPARTMENTS],
    n2_regen_radius: [f64; COMPARTMENTS],
    he_regen_radius: [f64; COMPARTMENTS],
    crushing_onset_tension: [f64; COMPARTMENTS],
    max_ambient_pressure: f64,
    bottom_n2_gradient: [f64; COMPARTMENTS],
    bottom_he_gradient: [f64; COMPARTMENTS],
    initial_n2_gradient: [f64; COMPARTMENTS],
    initial_he_gradient: [f64; COMPARTMENTS],
    /// VPM-B tolerated ambient pressure at the start of the ascent (bar).
    first_ceiling_bar: f64,
    max_bottom_ceiling_bar: f64,
    /// Accumulated decompression time (seconds), used by the CVA loop.
    deco_time: f64,
    /// The decompression-stop grid in seconds (Subsurface's planner timestep).
    stop_timestep: f64,
}

/// The result of a VPM-B schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct VpmbPlan {
    /// The ceiling (metres) at the start of the ascent.
    pub first_ceiling_m: f64,
    /// Stops, deepest first.
    pub stops: Vec<Stop>,
}

impl Vpmb {
    /// A diver equilibrated at the surface on air, for the given conservatism.
    pub fn new(surface_bar: f64, salinity: i32, conservatism: u8) -> Self {
        let level = CONSERVATISM_LEVELS[conservatism.min(4) as usize];
        let crit_radius_n2 = CRIT_RADIUS_N2 * level;
        let crit_radius_he = CRIT_RADIUS_HE * level;
        let p_alv = (surface_bar - WV_PRESSURE_SCHREINER) * N2_IN_AIR / 1000.0;
        Self {
            tissues: Tissues {
                n2: [p_alv; COMPARTMENTS],
                he: [0.0; COMPARTMENTS],
            },
            surface_bar,
            salinity,
            crit_radius_n2,
            crit_radius_he,
            max_n2_crushing: [0.0; COMPARTMENTS],
            max_he_crushing: [0.0; COMPARTMENTS],
            n2_regen_radius: [crit_radius_n2; COMPARTMENTS],
            he_regen_radius: [crit_radius_he; COMPARTMENTS],
            crushing_onset_tension: [0.0; COMPARTMENTS],
            max_ambient_pressure: 0.0,
            bottom_n2_gradient: [0.0; COMPARTMENTS],
            bottom_he_gradient: [0.0; COMPARTMENTS],
            initial_n2_gradient: [0.0; COMPARTMENTS],
            initial_he_gradient: [0.0; COMPARTMENTS],
            first_ceiling_bar: 0.0,
            max_bottom_ceiling_bar: 0.0,
            deco_time: 0.0,
            stop_timestep: 60.0,
        }
    }

    /// Ambient pressure (bar) at a depth in millimetres.
    fn ambient_bar_mm(&self, depth_mm: i32) -> f64 {
        ambient_mbar(depth_mm, self.surface_bar * 1000.0, self.salinity) / 1000.0
    }

    /// The depth (millimetres) of a tolerated ambient pressure.
    fn ceiling_depth_mm(&self, tolerance_bar: f64) -> i32 {
        depth_mm_at(
            tolerance_bar * 1000.0,
            self.surface_bar * 1000.0,
            self.salinity,
        )
        .max(0)
    }

    /// On-gas/off-gas a gas for a number of seconds at a fixed ambient
    /// pressure, then update the bubble crushing pressure.
    fn add_segment_bar(&mut self, ambient_bar: f64, seconds: f64, gas: GasMix) {
        if seconds <= 0.0 {
            return;
        }
        load_inert_gas(
            &mut self.tissues,
            ambient_bar,
            seconds / 60.0,
            gas,
            WV_PRESSURE_SCHREINER,
        );
        self.calc_crushing_pressure(ambient_bar);
    }

    /// Subsurface's `interpolate_transition`: ramp the depth linearly over one
    /// second steps, then crush at the final depth.
    fn interpolate(&mut self, from_mm: i32, to_mm: i32, seconds: i32, mode: BreathingMode) {
        for j in 0..seconds {
            let depth = if seconds > 0 {
                ((from_mm as f64 * (seconds - j) as f64 + to_mm as f64 * j as f64) / seconds as f64)
                    .round() as i32
            } else {
                (from_mm + to_mm) / 2
            };
            let amb = self.ambient_bar_mm(depth);
            self.add_segment_bar(amb, 1.0, mode.gas_at(amb));
        }
        if to_mm > from_mm {
            let amb = self.ambient_bar_mm(to_mm);
            self.calc_crushing_pressure(amb);
        }
    }

    /// A constant-depth segment, integrated in one-second steps.
    fn hold(&mut self, depth_mm: i32, seconds: i32, mode: BreathingMode) {
        let amb = self.ambient_bar_mm(depth_mm);
        for _ in 0..seconds {
            self.add_segment_bar(amb, 1.0, mode.gas_at(amb));
        }
    }

    /// Update the maximum crushing pressure seen so far, from Subsurface's
    /// `calc_crushing_pressure`.
    fn calc_crushing_pressure(&mut self, pressure: f64) {
        for ci in 0..COMPARTMENTS {
            let gas_tension = self.tissues.n2[ci] + self.tissues.he[ci] + OTHER_GASES_PRESSURE;
            let gradient = pressure - gas_tension;
            let (n2_crushing, he_crushing);
            if gradient <= GRADIENT_OF_IMPERM {
                n2_crushing = gradient;
                he_crushing = gradient;
                self.crushing_onset_tension[ci] = gas_tension;
            } else {
                if self.max_ambient_pressure >= pressure {
                    return;
                }
                let n2_inner = self.calc_inner_pressure(
                    self.crit_radius_n2,
                    self.crushing_onset_tension[ci],
                    pressure,
                );
                let he_inner = self.calc_inner_pressure(
                    self.crit_radius_he,
                    self.crushing_onset_tension[ci],
                    pressure,
                );
                n2_crushing = pressure - n2_inner;
                he_crushing = pressure - he_inner;
            }
            self.max_n2_crushing[ci] = self.max_n2_crushing[ci].max(n2_crushing);
            self.max_he_crushing[ci] = self.max_he_crushing[ci].max(he_crushing);
        }
        self.max_ambient_pressure = self.max_ambient_pressure.max(pressure);
    }

    fn calc_inner_pressure(&self, crit_radius: f64, onset_tension: f64, pressure: f64) -> f64 {
        let onset_radius = 1.0
            / (GRADIENT_OF_IMPERM / (2.0 * (SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA))
                + 1.0 / crit_radius);
        let a = pressure - GRADIENT_OF_IMPERM
            + (2.0 * (SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA)) / onset_radius;
        let b = 2.0 * (SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA);
        let c = onset_tension * onset_radius.powi(3);
        let current_radius = solve_cubic(a, b, c);
        onset_tension * onset_radius.powi(3) / current_radius.powi(3)
    }

    /// Subsurface's `nuclear_regeneration`.
    fn nuclear_regeneration(&mut self, seconds: f64) {
        let time = seconds / 60.0;
        for ci in 0..COMPARTMENTS {
            let crushing_n2 = 1.0
                / (self.max_n2_crushing[ci]
                    / (2.0 * (SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA))
                    + 1.0 / self.crit_radius_n2);
            let crushing_he = 1.0
                / (self.max_he_crushing[ci]
                    / (2.0 * (SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA))
                    + 1.0 / self.crit_radius_he);
            self.n2_regen_radius[ci] = crushing_n2
                + (self.crit_radius_n2 - crushing_n2) * (1.0 - (-time / REGENERATION_TIME).exp());
            self.he_regen_radius[ci] = crushing_he
                + (self.crit_radius_he - crushing_he) * (1.0 - (-time / REGENERATION_TIME).exp());
        }
    }

    /// Subsurface's `vpmb_start_gradient`.
    fn start_gradient(&mut self) {
        let k = 2.0 * (SURFACE_TENSION_GAMMA / SKIN_COMPRESSION_GAMMA_C);
        for ci in 0..COMPARTMENTS {
            let n2 =
                k * ((SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA) / self.n2_regen_radius[ci]);
            let he =
                k * ((SKIN_COMPRESSION_GAMMA_C - SURFACE_TENSION_GAMMA) / self.he_regen_radius[ci]);
            self.initial_n2_gradient[ci] = n2;
            self.bottom_n2_gradient[ci] = n2;
            self.initial_he_gradient[ci] = he;
            self.bottom_he_gradient[ci] = he;
        }
    }

    /// Subsurface's `calc_surface_phase`.
    fn calc_surface_phase(
        &self,
        he_pressure: f64,
        n2_pressure: f64,
        he_time_constant: f64,
        n2_time_constant: f64,
    ) -> f64 {
        let inspired_n2 = (self.surface_bar - WV_PRESSURE_SCHREINER) * NITROGEN_FRACTION;
        if n2_pressure > inspired_n2 {
            return (he_pressure / he_time_constant
                + (n2_pressure - inspired_n2) / n2_time_constant)
                / (he_pressure + n2_pressure - inspired_n2);
        }
        if he_pressure + n2_pressure >= inspired_n2 {
            let gradient_decay_time = 1.0 / (n2_time_constant - he_time_constant)
                * ((inspired_n2 - n2_pressure) / he_pressure).ln();
            let gradients_integral = he_pressure / he_time_constant
                * (1.0 - (-he_time_constant * gradient_decay_time).exp())
                + (n2_pressure - inspired_n2) / n2_time_constant
                    * (1.0 - (-n2_time_constant * gradient_decay_time).exp());
            return gradients_integral / (he_pressure + n2_pressure - inspired_n2);
        }
        0.0
    }

    /// Subsurface's `vpmb_next_gradient`.
    fn next_gradient(&mut self, deco_time_seconds: f64) {
        let deco_time = deco_time_seconds / 60.0;
        for ci in 0..COMPARTMENTS {
            let he_tc = LN2 / HE_HALF_TIMES[ci];
            let n2_tc = LN2 / N2_HALF_TIMES[ci];
            let desat_time = deco_time
                + self.calc_surface_phase(self.tissues.he[ci], self.tissues.n2[ci], he_tc, n2_tc);
            let n2_b = self.initial_n2_gradient[ci]
                + (CRIT_VOLUME_LAMBDA * SURFACE_TENSION_GAMMA)
                    / (SKIN_COMPRESSION_GAMMA_C * desat_time);
            let he_b = self.initial_he_gradient[ci]
                + (CRIT_VOLUME_LAMBDA * SURFACE_TENSION_GAMMA)
                    / (SKIN_COMPRESSION_GAMMA_C * desat_time);
            let n2_c =
                SURFACE_TENSION_GAMMA.powi(2) * CRIT_VOLUME_LAMBDA * self.max_n2_crushing[ci]
                    / (SKIN_COMPRESSION_GAMMA_C.powi(2) * desat_time);
            let he_c =
                SURFACE_TENSION_GAMMA.powi(2) * CRIT_VOLUME_LAMBDA * self.max_he_crushing[ci]
                    / (SKIN_COMPRESSION_GAMMA_C.powi(2) * desat_time);
            self.bottom_n2_gradient[ci] = 0.5 * (n2_b + (n2_b * n2_b - 4.0 * n2_c).max(0.0).sqrt());
            self.bottom_he_gradient[ci] = 0.5 * (he_b + (he_b * he_b - 4.0 * he_c).max(0.0).sqrt());
        }
    }

    /// Boyle's-law compensated gradient at a reference pressure.
    fn update_gradient(&self, next_stop_bar: f64, first_gradient: f64) -> f64 {
        let b = first_gradient.powi(3) / (self.first_ceiling_bar + first_gradient);
        let c = next_stop_bar * b;
        solve_cubic2(b, c).max(0.0)
    }

    fn tolerated_ambient_pressure(&self, reference: f64, ci: usize) -> f64 {
        let (n2_gradient, he_gradient) =
            if reference >= self.first_ceiling_bar || self.first_ceiling_bar == 0.0 {
                (self.bottom_n2_gradient[ci], self.bottom_he_gradient[ci])
            } else {
                (
                    self.update_gradient(reference, self.bottom_n2_gradient[ci]),
                    self.update_gradient(reference, self.bottom_he_gradient[ci]),
                )
            };
        let n2 = self.tissues.n2[ci];
        let he = self.tissues.he[ci];
        let total = n2 + he;
        if total <= 0.0 {
            return 0.0;
        }
        let total_gradient = (n2_gradient * n2 + he_gradient * he) / total;
        total + OTHER_GASES_PRESSURE - total_gradient
    }

    /// Subsurface's `tissue_tolerance_calc` VPM-B branch.
    fn tolerance_bar(&self, initial_pressure: f64) -> f64 {
        let mut ret = initial_pressure;
        loop {
            let reference = ret;
            ret = 0.0;
            for ci in 0..COMPARTMENTS {
                ret = ret.max(self.tolerated_ambient_pressure(reference, ci));
            }
            if (ret - reference).abs() <= 0.01 {
                break;
            }
        }
        ret
    }

    /// Ascent rate at a depth: Subsurface's `ascent_velocity`, in mm/s.
    fn ascent_rate_mm_s(&self, plan: &DecoSegment) -> i32 {
        (plan.ascent_rate * 1000.0 / 60.0).round() as i32
    }

    /// Subsurface's `trial_ascent`: can the diver ascend from `from_mm` to
    /// `target_mm` without breaking the ceiling? `wait_seconds` is spent at
    /// `from_mm` first.
    fn trial_ascent(
        &self,
        plan: &DecoSegment,
        from_mm: i32,
        target_mm: i32,
        wait_seconds: f64,
    ) -> bool {
        let mut ds = self.clone();
        if wait_seconds > 0.0 {
            let amb = ds.ambient_bar_mm(from_mm);
            ds.add_segment_bar(amb, wait_seconds, plan.mode.gas_at(amb));
        }
        // Consistency with other VPM-B implementations: do not start the ascent
        // while the ceiling is already deeper than the next stop.
        let tolerance = ds.tolerance_bar(ds.ambient_bar_mm(target_mm));
        if ds.ceiling_depth_mm(tolerance) > target_mm {
            return false;
        }
        let rate = self.ascent_rate_mm_s(plan);
        let mut depth = from_mm;
        while depth > target_mm {
            let mut deltad = rate * BASE_TIMESTEP;
            if deltad > depth {
                deltad = depth;
            }
            let amb = ds.ambient_bar_mm(depth);
            ds.add_segment_bar(amb, BASE_TIMESTEP as f64, plan.mode.gas_at(amb));
            let tolerance = ds.tolerance_bar(ds.ambient_bar_mm(depth));
            if ds.ceiling_depth_mm(tolerance) > depth - deltad {
                return false;
            }
            depth -= deltad;
        }
        true
    }

    /// Subsurface's `wait_until`: binary-search the absolute hold time on the
    /// planner's timestep grid that lets the ascent start. `clock` is the
    /// current time, `min` the lower bound of the search and `leap` the guess.
    fn wait_until(
        &self,
        plan: &DecoSegment,
        depth_mm: i32,
        target_mm: i32,
        clock: i64,
        min: i64,
        leap: i64,
    ) -> i64 {
        let stepsize = self.stop_timestep as i64;
        if min >= 48 * 3600 {
            return 50 * 3600;
        }
        let upper = min + leap + stepsize - 1 - ((min + leap - 1).rem_euclid(stepsize));
        if !self.trial_ascent(plan, depth_mm, target_mm, (upper - clock) as f64) {
            return self.wait_until(plan, depth_mm, target_mm, clock, upper, leap);
        }
        if upper - min <= stepsize {
            return upper;
        }
        self.wait_until(plan, depth_mm, target_mm, clock, min, leap / 2)
    }

    /// Build the schedule for one CVA iteration. Returns the stops and the
    /// resulting decompression time in seconds.
    fn run(&mut self, plan: &DecoSegment, bottom_time: i64) -> (Vec<Stop>, f64) {
        let bottom_mm = (plan.bottom_depth.meters() * 1000.0).round() as i32;
        let rate = self.ascent_rate_mm_s(plan);

        // The first ceiling is the VPM-B tolerance at the start of the ascent,
        // round-tripped through Subsurface's integer mbar pressure.
        let tolerance = self.tolerance_bar(self.ambient_bar_mm(bottom_mm));
        let ceiling_mm = self.ceiling_depth_mm(tolerance);
        let first_ceiling_mbar =
            ambient_mbar(ceiling_mm, self.surface_bar * 1000.0, self.salinity).round();
        self.first_ceiling_bar = (first_ceiling_mbar / 1000.0).max(self.max_bottom_ceiling_bar);

        let mut depth = bottom_mm;
        let mut clock = bottom_time;
        let mut stops: Vec<Stop> = Vec::new();

        // Stop levels on the stop grid, deepest first, ending at the surface.
        let stop_mm = (plan.stop_step * 1000.0).round().max(1.0) as i32;
        let mut levels: Vec<i32> = Vec::new();
        let mut level = (bottom_mm / stop_mm) * stop_mm;
        while level > 0 {
            levels.push(level);
            level -= stop_mm;
        }
        levels.push(0);

        let mut laststoptime = self.stop_timestep as i64;
        for i in 0..levels.len() {
            let level = levels[i];
            // Ascend to this level. Subsurface uses a do-while, so it always
            // integrates one base-timestep segment even if it is already there.
            loop {
                let mut deltad = rate * BASE_TIMESTEP;
                if depth - deltad < level {
                    deltad = depth - level;
                }
                let amb = self.ambient_bar_mm(depth);
                self.add_segment_bar(amb, BASE_TIMESTEP as f64, plan.mode.gas_at(amb));
                depth -= deltad;
                clock += BASE_TIMESTEP as i64;
                if depth <= 0 || depth <= level {
                    break;
                }
            }
            if level <= 0 {
                break;
            }
            let target = levels[i + 1];
            if !self.trial_ascent(plan, depth, target, 0.0) {
                let new_clock =
                    self.wait_until(plan, depth, target, clock, clock, laststoptime * 2 + 1);
                laststoptime = new_clock - clock;
                if laststoptime > 0 {
                    let amb = self.ambient_bar_mm(depth);
                    self.add_segment_bar(amb, laststoptime as f64, plan.mode.gas_at(amb));
                    clock = new_clock;
                    stops.push(Stop {
                        depth: Depth::new(depth),
                        duration: Duration::new(laststoptime as i32),
                    });
                }
            }
        }

        // Pseudo deco time for the next CVA iteration (Subsurface assumes the
        // final ascent always takes the same time).
        // Subsurface divides integer millimetres by the integer mm/s rate.
        let deco_time = (clock - bottom_time - 3000 / rate as i64 + 20) as f64;
        (stops, deco_time)
    }

    /// The no-decompression limit for a square profile: how long the diver
    /// can stay at the bottom before a VPM-B ceiling appears.
    pub fn ndl(&self, plan: &DecoSegment) -> Option<Duration> {
        let bottom_mm = (plan.bottom_depth.meters() * 1000.0).round() as i32;
        let descent_seconds = descent_seconds(plan);
        let mut ds = self.clone();
        ds.interpolate(0, bottom_mm, descent_seconds, plan.mode);
        ds.nuclear_regeneration(descent_seconds as f64);
        ds.start_gradient();
        ds.first_ceiling_bar = ds.tolerance_bar(ds.ambient_bar_mm(bottom_mm));
        let mut minutes = 0.0f64;
        loop {
            let tolerance = ds.tolerance_bar(ds.ambient_bar_mm(bottom_mm));
            if ds.ceiling_depth_mm(tolerance) > 0 {
                return Some(Duration::new((minutes * 60.0).round() as i32));
            }
            if minutes >= 24.0 * 60.0 {
                return None;
            }
            ds.hold(bottom_mm, 60, plan.mode);
            minutes += 1.0;
        }
    }

    /// Reproduce Subsurface's planner: replay the square profile, then iterate
    /// the CVA until the decompression time converges.
    fn schedule(&self, plan: &DecoSegment, timestep: f64) -> VpmbPlan {
        let mut ds = self.clone();
        ds.stop_timestep = timestep;

        let bottom_mm = (plan.bottom_depth.meters() * 1000.0).round() as i32;
        let descent_seconds = descent_seconds(plan);
        let bottom_seconds = (plan.bottom_minutes * 60.0).round() as i32;

        // Replay the entered profile: descent, then bottom.
        ds.interpolate(0, bottom_mm, descent_seconds, plan.mode);
        ds.hold(bottom_mm, bottom_seconds, plan.mode);

        let bottom_time = (descent_seconds + bottom_seconds) as i64;
        ds.nuclear_regeneration(bottom_time as f64);
        ds.start_gradient();

        let cached = ds.clone();
        let mut previous_deco_time = 1.0e8;
        ds.deco_time = 1.0e7;

        let mut result;
        loop {
            let is_final = (previous_deco_time - ds.deco_time).abs() < 10.0;
            if ds.deco_time != 1.0e7 {
                ds.next_gradient(ds.deco_time);
            }
            previous_deco_time = ds.deco_time;

            // Restore the bottom tissues, keeping the gradient state.
            let mut trial = cached.clone();
            trial.bottom_n2_gradient = ds.bottom_n2_gradient;
            trial.bottom_he_gradient = ds.bottom_he_gradient;
            trial.initial_n2_gradient = ds.initial_n2_gradient;
            trial.initial_he_gradient = ds.initial_he_gradient;
            trial.first_ceiling_bar = ds.first_ceiling_bar;
            trial.max_bottom_ceiling_bar = ds.max_bottom_ceiling_bar;
            trial.stop_timestep = timestep;

            let (stops, deco_time) = trial.run(plan, bottom_time);
            let first_ceiling_m = trial.ceiling_depth_mm(trial.first_ceiling_bar) as f64 / 1000.0;
            result = (first_ceiling_m, stops);
            ds = trial;
            ds.deco_time = deco_time;

            if is_final {
                break;
            }
        }

        let (first_ceiling_m, stops) = result;
        VpmbPlan {
            first_ceiling_m,
            stops,
        }
    }
}

// Solve x^3 - B x - C == 0 for the positive root.
fn solve_cubic2(b: f64, c: f64) -> f64 {
    let discriminant = 27.0 * c * c - 4.0 * b.powi(3);
    if discriminant < 0.0 {
        return 2.0
            * (b / 3.0).sqrt()
            * ((3.0 * c * (3.0 / b).sqrt() / (2.0 * b)).acos() / 3.0).cos();
    }
    let denominator = (9.0 * c + (3.0 * discriminant).sqrt()).cbrt();
    (2.0 / 3.0f64).cbrt() * b / denominator + denominator / 18.0f64.cbrt()
}

// Solve A r^3 - B r^2 - C == 0 for the positive root.
fn solve_cubic(a: f64, b: f64, c: f64) -> f64 {
    let ba = b / a;
    let ca = c / a;
    let discriminant = ca * (4.0 * ba.powi(3) + 27.0 * ca);
    if discriminant < 0.0 {
        return 0.0;
    }
    let denominator = (ba.powi(3) + 1.5 * (9.0 * ca + 3.0f64.sqrt() * discriminant.sqrt())).cbrt();
    (ba + ba * ba / denominator + denominator) / 3.0
}

/// The descent time in seconds for a square profile.
fn descent_seconds(plan: &DecoSegment) -> i32 {
    if plan.descent_rate <= 0.0 {
        return 0;
    }
    let bottom_mm = (plan.bottom_depth.meters() * 1000.0).round();
    (bottom_mm / (plan.descent_rate * 1000.0 / 60.0)).round() as i32
}

/// Plan a square profile with VPM-B, mirroring Subsurface's planner.
///
/// `timestep` is the decompression-stop grid in seconds: Subsurface's planner
/// CLI uses 60, which is why its published stop times are not minute-aligned.
pub fn plan(
    segment: &DecoSegment,
    surface_bar: f64,
    salinity: i32,
    conservatism: u8,
    timestep: f64,
) -> VpmbPlan {
    Vpmb::new(surface_bar, salinity, conservatism).schedule(segment, timestep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::AIR;

    /// A golden VPM-B plan from Subsurface 6.0.5504's planner CLI (sea water,
    /// 1013 mbar surface). Stop depths and times must match exactly; the CLI
    /// rounds the ceiling to two decimals, so it is compared within 2 cm.
    struct Reference {
        depth: f64,
        minutes: f64,
        conservatism: u8,
        o2: u16,
        he: u16,
        descent: f64,
        ascent: f64,
        first_ceiling_m: f64,
        stops: &'static [(f64, i32)],
    }

    impl Reference {
        fn segment(&self) -> DecoSegment {
            DecoSegment {
                bottom_depth: Depth::from_meters(self.depth),
                bottom_minutes: self.minutes,
                mode: BreathingMode::OpenCircuit(GasMix::new(self.o2, self.he)),
                descent_rate: self.descent,
                ascent_rate: self.ascent,
                ..Default::default()
            }
        }
    }

    fn base(
        depth: f64,
        minutes: f64,
        first_ceiling_m: f64,
        stops: &'static [(f64, i32)],
    ) -> Reference {
        Reference {
            depth,
            minutes,
            conservatism: 3,
            o2: 210,
            he: 0,
            descent: 20.0,
            ascent: 10.0,
            first_ceiling_m,
            stops,
        }
    }

    #[test]
    fn matches_subsurface_vpmb_reference_plans() {
        let cases = [
            base(18.0, 60.0, 2.11, &[(3.0, 514)]),
            base(30.0, 20.0, 9.49, &[(9.0, 142), (6.0, 222), (3.0, 462)]),
            base(
                40.0,
                40.0,
                21.85,
                &[
                    (21.0, 186),
                    (18.0, 222),
                    (15.0, 342),
                    (12.0, 522),
                    (9.0, 762),
                    (6.0, 1242),
                    (3.0, 2202),
                ],
            ),
            base(
                45.0,
                30.0,
                25.12,
                &[
                    (24.0, 97),
                    (21.0, 162),
                    (18.0, 222),
                    (15.0, 342),
                    (12.0, 462),
                    (9.0, 642),
                    (6.0, 1122),
                    (3.0, 2022),
                ],
            ),
            base(
                60.0,
                20.0,
                34.41,
                &[
                    (33.0, 76),
                    (30.0, 102),
                    (27.0, 102),
                    (24.0, 162),
                    (21.0, 162),
                    (18.0, 282),
                    (15.0, 342),
                    (12.0, 522),
                    (9.0, 702),
                    (6.0, 1182),
                    (3.0, 2022),
                ],
            ),
            Reference {
                conservatism: 0,
                first_ceiling_m: 20.40,
                ..base(
                    40.0,
                    40.0,
                    0.0,
                    &[
                        (21.0, 66),
                        (18.0, 162),
                        (15.0, 282),
                        (12.0, 402),
                        (9.0, 582),
                        (6.0, 882),
                        (3.0, 1662),
                    ],
                )
            },
            Reference {
                conservatism: 1,
                first_ceiling_m: 20.79,
                ..base(
                    40.0,
                    40.0,
                    0.0,
                    &[
                        (21.0, 126),
                        (18.0, 162),
                        (15.0, 282),
                        (12.0, 402),
                        (9.0, 642),
                        (6.0, 1002),
                        (3.0, 1782),
                    ],
                )
            },
            Reference {
                conservatism: 2,
                first_ceiling_m: 21.27,
                ..base(
                    40.0,
                    40.0,
                    0.0,
                    &[
                        (21.0, 126),
                        (18.0, 222),
                        (15.0, 282),
                        (12.0, 462),
                        (9.0, 702),
                        (6.0, 1062),
                        (3.0, 1962),
                    ],
                )
            },
            Reference {
                o2: 320,
                first_ceiling_m: 15.33,
                ..base(
                    40.0,
                    40.0,
                    0.0,
                    &[(15.0, 90), (12.0, 222), (9.0, 402), (6.0, 582), (3.0, 1062)],
                )
            },
            Reference {
                o2: 180,
                he: 450,
                first_ceiling_m: 38.54,
                ..base(
                    60.0,
                    25.0,
                    0.0,
                    &[
                        (36.0, 34),
                        (33.0, 102),
                        (30.0, 102),
                        (27.0, 102),
                        (24.0, 222),
                        (21.0, 222),
                        (18.0, 342),
                        (15.0, 462),
                        (12.0, 702),
                        (9.0, 1002),
                        (6.0, 1842),
                        (3.0, 3642),
                    ],
                )
            },
            Reference {
                descent: 18.0,
                ascent: 9.0,
                first_ceiling_m: 21.88,
                ..base(
                    40.0,
                    40.0,
                    0.0,
                    &[
                        (21.0, 159),
                        (18.0, 220),
                        (15.0, 340),
                        (12.0, 520),
                        (9.0, 760),
                        (6.0, 1240),
                        (3.0, 2200),
                    ],
                )
            },
        ];

        for c in &cases {
            let plan = plan(&c.segment(), 1.013, 10_300, c.conservatism, 60.0);
            assert!(
                (plan.first_ceiling_m - c.first_ceiling_m).abs() < 0.02,
                "{} m/{} min cons{}: first ceiling {:.3}, Subsurface {:.2}",
                c.depth,
                c.minutes,
                c.conservatism,
                plan.first_ceiling_m,
                c.first_ceiling_m
            );
            assert_eq!(
                plan.stops.len(),
                c.stops.len(),
                "{} m/{} min cons{}: stop count",
                c.depth,
                c.minutes,
                c.conservatism
            );
            for (stop, (exp_depth, exp_seconds)) in plan.stops.iter().zip(c.stops.iter()) {
                assert_eq!(
                    stop.depth,
                    Depth::from_meters(*exp_depth),
                    "{} m/{} min cons{}: stop depth",
                    c.depth,
                    c.minutes,
                    c.conservatism
                );
                assert_eq!(
                    stop.duration.seconds, *exp_seconds,
                    "{} m/{} min cons{}: {} m stop duration",
                    c.depth, c.minutes, c.conservatism, exp_depth
                );
            }
        }
    }

    #[test]
    fn deep_vpmb_dive_has_a_ceiling() {
        let plan = plan(
            &base(60.0, 20.0, 0.0, &[]).segment(),
            1.013,
            10_300,
            3,
            60.0,
        );
        assert!(plan.first_ceiling_m > 30.0);
        assert!(!plan.stops.is_empty());
    }

    #[test]
    fn more_conservatism_means_more_stop_time() {
        let total = |c: u8| -> i32 {
            plan(
                &base(40.0, 40.0, 0.0, &[]).segment(),
                1.013,
                10_300,
                c,
                60.0,
            )
            .stops
            .iter()
            .map(|s| s.duration.seconds)
            .sum()
        };
        assert!(total(0) < total(2));
        assert!(total(2) < total(4));
    }

    #[test]
    fn no_decompression_for_a_short_shallow_dive() {
        let plan = plan(
            &base(12.0, 20.0, 0.0, &[]).segment(),
            1.013,
            10_300,
            3,
            60.0,
        );
        assert!(plan.stops.is_empty());
    }

    #[test]
    fn air_is_the_default_gas() {
        assert_eq!(
            base(30.0, 20.0, 0.0, &[]).segment().mode.cylinder_gas(),
            AIR
        );
    }
}
