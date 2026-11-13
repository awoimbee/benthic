//! Dive planning: turn a depth, bottom time and gas into a full plan
//! (decompression stops) and, optionally, a dive that can be saved.

use serde::{Deserialize, Serialize};

use crate::deco::vpmb::{Vpmb, VPMB_TIMESTEP_SECONDS};
use crate::deco::{BreathingMode, Buhlmann, DecoModel, DecoSegment, PlanPoint, Stop};
use crate::gas::{ambient_mbar, mod_depth_mm, GasMix};
use crate::model::{
    Cylinder, CylinderUse, Dive, DiveComputer, Divemode, Event, Sample, SensorPressure,
};
use crate::units::*;

/// Upper bound on chart samples, including the planner's waypoints and stops.
/// Very long or pathological plans are sampled at a coarser interval instead
/// of emitting one point per minute for hours.
pub const MAX_GRAPH_SAMPLES: usize = 100;

/// Expand a profile to a bounded number of points, preserving every original
/// waypoint/stop and adding interpolated points at a common interval.
fn densify_samples(base: Vec<Sample>, max_samples: usize) -> Vec<Sample> {
    let Some(first) = base.first().cloned() else {
        return Vec::new();
    };
    let span = base
        .last()
        .map(|sample| sample.time.seconds - first.time.seconds)
        .unwrap_or(0);
    let budget = max_samples.saturating_sub(base.len());
    let interval = if budget == 0 || span <= 0 {
        i32::MAX
    } else {
        ((span as f64 / budget as f64).ceil() as i32).max(60)
    };

    let mut samples = vec![first];
    for window in base.windows(2) {
        let (t0, d0) = (window[0].time.seconds, window[0].depth.mm);
        let (t1, d1) = (window[1].time.seconds, window[1].depth.mm);
        if t1 - t0 > interval && t1 > t0 {
            let mut time = t0 + interval;
            while time < t1 {
                let fraction = (time - t0) as f64 / (t1 - t0) as f64;
                samples.push(Sample {
                    time: Duration::new(time),
                    depth: Depth::new((d0 as f64 + (d1 - d0) as f64 * fraction).round() as i32),
                    ..Default::default()
                });
                time += interval;
            }
        }
        samples.push(window[1].clone());
    }
    samples
}

/// A gas cylinder the planner may breathe from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GasSupply {
    pub gas: GasMix,
    /// Water volume of the cylinder.
    pub size: Volume,
    pub start_pressure: Pressure,
}

impl GasSupply {
    pub fn available_liters(self) -> f64 {
        self.size.liters() * self.start_pressure.bar()
    }
}

/// Gas consumption for one cylinder in a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GasUse {
    pub gas: GasMix,
    pub size: Volume,
    pub start_pressure: Pressure,
    pub used_liters: f64,
    pub end_pressure: Pressure,
}

/// An automatic gas switch during the generated ascent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GasSwitch {
    pub depth: Depth,
    pub gas: GasMix,
    pub time: Duration,
    /// Index of the cylinder used after this switch, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cylinder: Option<usize>,
}

/// A computed dive plan: the entered waypoints plus the generated stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivePlan {
    /// The entered profile, deepest-or-last waypoint last.
    pub points: Vec<PlanPoint>,
    /// Generated decompression stops, deepest first.
    pub stops: Vec<Stop>,
    /// Ascent rate for the generated ascent, metres per minute.
    pub ascent_rate: f64,
    /// Distance between stops, metres.
    pub stop_step: f64,
    /// The chosen decompression model. `Buhlmann` carries the gradient factors;
    /// `Vpmb` carries the conservatism level.
    #[serde(default)]
    pub deco_model: DecoModel,
    /// Automatic gas switches during the generated ascent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gas_switches: Vec<GasSwitch>,
    /// Per-cylinder gas consumption, when the plan was built from supplies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gas_uses: Vec<GasUse>,
}

/// The no-decompression limit for a depth and breathing mode under the chosen
/// decompression model.
pub fn ndl(
    depth: Depth,
    mode: BreathingMode,
    deco_model: DecoModel,
    surface_bar: f64,
    salinity: i32,
) -> Option<Duration> {
    match deco_model {
        DecoModel::Buhlmann { gf_high, .. } => {
            Buhlmann::new(surface_bar, salinity).ndl_mode(depth, mode, gf_high)
        }
        DecoModel::Vpmb { conservatism } => {
            Vpmb::new(surface_bar, salinity, conservatism).ndl(depth, mode)
        }
    }
}

/// The dive mode implied by a breathing mode.
fn divemode_of(mode: BreathingMode) -> Divemode {
    match mode {
        BreathingMode::OpenCircuit(_) => Divemode::OpenCircuit,
        BreathingMode::ClosedCircuit { .. } => Divemode::Ccr,
        BreathingMode::PassiveSemiClosed { .. } => Divemode::Pscr,
    }
}

/// How a cylinder's gas is used, given the mode breathing it.
fn cylinder_use(mode: BreathingMode) -> CylinderUse {
    match mode {
        BreathingMode::OpenCircuit(_) => CylinderUse::OcGas,
        BreathingMode::ClosedCircuit { .. } | BreathingMode::PassiveSemiClosed { .. } => {
            CylinderUse::Diluent
        }
    }
}

fn switch_depth(
    gas: GasMix,
    max_ppo2_bar: f64,
    safety: Depth,
    surface_bar: f64,
    salinity: i32,
) -> Depth {
    let limit = (max_ppo2_bar.max(0.1) * 1000.0).round();
    let mod_mm = mod_depth_mm(gas, limit, surface_bar * 1000.0, salinity);
    let usable_mm = (mod_mm - safety.mm.max(0)).max(0);
    // Switch on a whole-metre boundary: 70% at ppO2 1.5 with a 1 m margin
    // switches at 10 m, not in the middle of the stop grid.
    Depth::new((usable_mm / 1000) * 1000)
}

fn sorted_supplies(
    supplies: &[GasSupply],
    max_ppo2_bar: f64,
    safety: Depth,
    surface_bar: f64,
    salinity: i32,
) -> Vec<(GasSupply, Depth)> {
    let mut sorted: Vec<(GasSupply, Depth)> = supplies
        .iter()
        .filter(|supply| supply.size.ml > 0 && supply.start_pressure.mbar > 0)
        .map(|supply| {
            (
                *supply,
                switch_depth(supply.gas, max_ppo2_bar, safety, surface_bar, salinity),
            )
        })
        .collect();
    sorted.sort_by(|a, b| {
        b.1.mm
            .cmp(&a.1.mm)
            .then_with(|| b.0.gas.o2_permille.cmp(&a.0.gas.o2_permille))
    });
    sorted
}

fn switch_modes(sorted: &[(GasSupply, Depth)], max_depth: Depth) -> Vec<(Depth, BreathingMode)> {
    let mut modes = Vec::new();
    for (supply, depth) in sorted.iter().skip(1) {
        if depth.mm > 0 && depth.mm < max_depth.mm {
            modes.push((*depth, BreathingMode::OpenCircuit(supply.gas)));
        }
    }
    modes
}

fn gas_at_depth(sorted: &[(GasSupply, Depth)], depth: Depth) -> GasMix {
    let mut chosen = sorted
        .first()
        .map(|(supply, _)| supply.gas)
        .unwrap_or_default();
    let mut best: Option<i32> = None;
    for (supply, switch_depth) in sorted {
        if depth.mm <= switch_depth.mm && best.is_none_or(|value| switch_depth.mm < value) {
            best = Some(switch_depth.mm);
            chosen = supply.gas;
        }
    }
    chosen
}

/// Round a ceiling up to the stop grid. The schedule lets the diver ascend to
/// a stop when the ceiling is within 5 cm of it, so the same tolerance keeps
/// the displayed ceiling from sitting one step below the diver.
fn stop_grid_ceiling(ceiling: Depth, step_mm: i32) -> Depth {
    const TOLERANCE_MM: i32 = 50;
    let mm = (ceiling.mm - TOLERANCE_MM).max(0);
    Depth::new((mm + step_mm - 1) / step_mm * step_mm)
}

/// Seconds to reach the surface from `start_m` through `stops`, rounded per
/// leg like the planned profile.
fn ascent_seconds(start_m: f64, stops: &[Stop], ascent_rate: f64) -> i32 {
    let rate = ascent_rate.max(0.1);
    let mut seconds = 0;
    let mut current = start_m;
    for stop in stops {
        seconds += ((current - stop.depth.meters()) / rate * 60.0).round() as i32;
        seconds += stop.duration.seconds;
        current = stop.depth.meters();
    }
    seconds + (current / rate * 60.0).round() as i32
}

impl DivePlan {
    /// Plan from a sequence of waypoints.
    pub fn compute(
        points: Vec<PlanPoint>,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
    ) -> Self {
        Self::compute_with_switches(
            points,
            Vec::new(),
            Vec::new(),
            0.0,
            deco_model,
            surface_bar,
            salinity,
        )
    }

    fn compute_with_switches(
        points: Vec<PlanPoint>,
        switches: Vec<(Depth, BreathingMode)>,
        supplies: Vec<GasSupply>,
        rmv_l_per_min: f64,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
    ) -> Self {
        let segment = DecoSegment {
            points: points.clone(),
            ascent_rate: 10.0,
            stop_step: 3.0,
            gas_switches: switches.clone(),
            ..Default::default()
        };
        let stops = match deco_model {
            DecoModel::Buhlmann { gf_low, gf_high } => Buhlmann::new(surface_bar, salinity)
                .deco_schedule(&DecoSegment {
                    gf_low,
                    gf_high,
                    ..segment
                }),
            DecoModel::Vpmb { conservatism } => {
                crate::deco::vpmb::plan(
                    &segment,
                    surface_bar,
                    salinity,
                    conservatism,
                    VPMB_TIMESTEP_SECONDS,
                )
                .stops
            }
        };
        let mut plan = Self {
            points,
            stops,
            ascent_rate: 10.0,
            stop_step: 3.0,
            deco_model,
            gas_switches: Vec::new(),
            gas_uses: Vec::new(),
        };
        if !supplies.is_empty() && rmv_l_per_min > 0.0 {
            plan.account_gas(&supplies, &switches, rmv_l_per_min, surface_bar, salinity);
        }
        plan
    }

    /// Plan a multi-level open-circuit profile using a list of cylinders.
    ///
    /// Each entered waypoint is assigned the deepest gas that is safe at its
    /// depth, and the generated ascent switches automatically at the configured
    /// safety depths.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_with_gases(
        points: Vec<PlanPoint>,
        supplies: &[GasSupply],
        max_ppo2_bar: f64,
        switch_safety: Depth,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
        rmv_l_per_min: f64,
    ) -> Self {
        let sorted = sorted_supplies(supplies, max_ppo2_bar, switch_safety, surface_bar, salinity);
        if sorted.is_empty() {
            return Self::compute(points, deco_model, surface_bar, salinity);
        }
        let max_depth = points
            .iter()
            .map(|point| point.depth)
            .max()
            .unwrap_or_default();
        let switches = switch_modes(&sorted, max_depth);
        let points: Vec<PlanPoint> = points
            .into_iter()
            .map(|mut point| {
                point.mode = BreathingMode::OpenCircuit(gas_at_depth(&sorted, point.depth));
                point
            })
            .collect();
        let supplies: Vec<GasSupply> = sorted.iter().map(|(supply, _)| *supply).collect();
        Self::compute_with_switches(
            points,
            switches,
            supplies,
            rmv_l_per_min,
            deco_model,
            surface_bar,
            salinity,
        )
    }

    /// Plan a square dive (descent, hold, generated ascent) from a cylinder
    /// list. The bottom portion always uses the deepest-capable gas.
    #[allow(clippy::too_many_arguments)]
    pub fn square_with_gases(
        depth: Depth,
        bottom_time: Duration,
        supplies: &[GasSupply],
        max_ppo2_bar: f64,
        switch_safety: Depth,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
        rmv_l_per_min: f64,
    ) -> Self {
        let sorted = sorted_supplies(supplies, max_ppo2_bar, switch_safety, surface_bar, salinity);
        if sorted.is_empty() {
            return Self::square(
                depth,
                bottom_time,
                BreathingMode::OpenCircuit(GasMix::default()),
                deco_model,
                surface_bar,
                salinity,
            );
        }
        let bottom_gas = BreathingMode::OpenCircuit(sorted[0].0.gas);
        let descent_seconds = (depth.meters() / 20.0 * 60.0).round() as i32;
        let points = vec![
            PlanPoint {
                depth,
                duration: Duration::new(descent_seconds.max(0)),
                mode: bottom_gas,
            },
            PlanPoint {
                depth,
                duration: bottom_time,
                mode: bottom_gas,
            },
        ];
        let switches = switch_modes(&sorted, depth);
        let supplies: Vec<GasSupply> = sorted.iter().map(|(supply, _)| *supply).collect();
        Self::compute_with_switches(
            points,
            switches,
            supplies,
            rmv_l_per_min,
            deco_model,
            surface_bar,
            salinity,
        )
    }

    /// The longest square bottom time whose gas consumption still leaves
    /// `reserve` bar in every cylinder (bounded to 24 hours).
    #[allow(clippy::too_many_arguments)]
    pub fn max_bottom_time_for_reserve(
        depth: Depth,
        supplies: &[GasSupply],
        max_ppo2_bar: f64,
        switch_safety: Depth,
        reserve: Pressure,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
        rmv_l_per_min: f64,
    ) -> Duration {
        if sorted_supplies(supplies, max_ppo2_bar, switch_safety, surface_bar, salinity).is_empty()
        {
            return Duration::ZERO;
        }
        let mut low = 0i32;
        let mut high = 24 * 60;
        let mut best = 0i32;
        while low <= high {
            let minutes = (low + high) / 2;
            let plan = Self::square_with_gases(
                depth,
                Duration::from_minutes(minutes),
                supplies,
                max_ppo2_bar,
                switch_safety,
                deco_model,
                surface_bar,
                salinity,
                rmv_l_per_min,
            );
            let enough = plan
                .gas_uses
                .iter()
                .all(|gas| gas.end_pressure.mbar >= reserve.mbar);
            if enough {
                best = minutes;
                low = minutes + 1;
            } else {
                high = minutes - 1;
            }
        }
        Duration::from_minutes(best)
    }

    /// Account for every cylinder's gas consumption and record the exact times
    /// of the automatic switches. Cylinders of the same gas are used in order;
    /// when one reaches ambient pressure plus a small regulator margin the next
    /// usable cylinder takes over.
    fn account_gas(
        &mut self,
        supplies: &[GasSupply],
        switches: &[(Depth, BreathingMode)],
        rmv_l_per_min: f64,
        surface_bar: f64,
        salinity: i32,
    ) {
        let raw = self.samples_without_switch_events();
        if raw.len() < 2 || supplies.is_empty() {
            return;
        }
        let surface_mbar = surface_bar * 1000.0;
        let bottom_seconds = self.points.iter().map(|point| point.duration.seconds).sum();
        let bottom_gas = supplies
            .first()
            .map(|supply| supply.gas)
            .unwrap_or_default();
        let switch_list: Vec<(Depth, GasMix)> = switches
            .iter()
            .map(|(depth, mode)| (*depth, mode.cylinder_gas()))
            .collect();
        let gas_at = |depth: Depth| -> GasMix {
            let mut chosen = bottom_gas;
            let mut best = None;
            for (switch_depth, gas) in &switch_list {
                if depth.mm <= switch_depth.mm
                    && best.is_none_or(|best: i32| switch_depth.mm < best)
                {
                    best = Some(switch_depth.mm);
                    chosen = *gas;
                }
            }
            chosen
        };

        // Build profile vertices, including depth-based gas switch crossings.
        let mut vertices: Vec<(i32, i32)> = raw
            .iter()
            .map(|sample| (sample.time.seconds, sample.depth.mm))
            .collect();
        for window in raw.windows(2) {
            let (t0, d0) = (window[0].time.seconds, window[0].depth.mm);
            let (t1, d1) = (window[1].time.seconds, window[1].depth.mm);
            if t1 <= bottom_seconds || d1 >= d0 {
                continue;
            }
            let mut crossings: Vec<i32> = switch_list
                .iter()
                .filter(|(depth, _)| depth.mm > d1 && depth.mm < d0)
                .map(|(depth, _)| depth.mm)
                .collect();
            crossings.sort_by(|a, b| b.cmp(a));
            for depth in crossings {
                let fraction = if d1 != d0 {
                    (depth - d0) as f64 / (d1 - d0) as f64
                } else {
                    0.0
                };
                vertices.push((t0 + ((t1 - t0) as f64 * fraction).round() as i32, depth));
            }
        }
        vertices.sort_by_key(|(time, _)| *time);
        vertices.dedup();

        // Discretise long intervals to at most one minute so gas exhaustion is
        // detected at a useful resolution.
        let mut dense = vec![vertices[0]];
        for window in vertices.windows(2) {
            let (t0, d0) = window[0];
            let (t1, d1) = window[1];
            if t1 - t0 > 60 && t1 > t0 {
                let mut time = t0 + 60;
                while time < t1 {
                    let fraction = (time - t0) as f64 / (t1 - t0) as f64;
                    dense.push((
                        time,
                        (d0 as f64 + (d1 - d0) as f64 * fraction).round() as i32,
                    ));
                    time += 60;
                }
            }
            dense.push(window[1]);
        }

        #[derive(Clone)]
        struct SupplyState {
            supply: GasSupply,
            remaining: f64,
            used: f64,
        }
        let mut states: Vec<SupplyState> = supplies
            .iter()
            .map(|supply| SupplyState {
                supply: *supply,
                remaining: supply.available_liters(),
                used: 0.0,
            })
            .collect();
        let ambient_bar = |depth: Depth| ambient_mbar(depth.mm, surface_mbar, salinity) / 1000.0;
        let usable = |state: &SupplyState, depth: Depth| {
            state.remaining / state.supply.size.liters().max(0.001) > ambient_bar(depth) + 2.0
        };
        let safe = |gas: GasMix, depth: Depth| {
            if gas == bottom_gas {
                return true;
            }
            switch_list
                .iter()
                .find(|(_, candidate)| *candidate == gas)
                .is_none_or(|(switch_depth, _)| depth.mm <= switch_depth.mm)
        };
        let choose = |states: &[SupplyState], depth: Depth, desired: GasMix| -> usize {
            if let Some(index) = states
                .iter()
                .position(|state| state.supply.gas == desired && usable(state, depth))
            {
                return index;
            }
            if let Some((index, _)) = states
                .iter()
                .enumerate()
                .filter(|(_, state)| safe(state.supply.gas, depth) && usable(state, depth))
                .max_by(|a, b| a.1.remaining.total_cmp(&b.1.remaining))
            {
                return index;
            }
            // Not enough usable gas: keep accounting so the deficit is visible.
            states
                .iter()
                .enumerate()
                .filter(|(_, state)| state.supply.gas == desired)
                .max_by(|a, b| a.1.remaining.total_cmp(&b.1.remaining))
                .map(|(index, _)| index)
                .unwrap_or(0)
        };

        let mut previous_supply: Option<usize> = None;
        for window in dense.windows(2) {
            let (t0, d0) = window[0];
            let (t1, d1) = window[1];
            if t1 <= t0 {
                continue;
            }
            let desired = if t0 < bottom_seconds {
                bottom_gas
            } else {
                gas_at(Depth::new(d0))
            };
            let index = choose(&states, Depth::new(d0), desired);
            if let Some(previous) = previous_supply {
                if previous != index {
                    let supply = &states[index].supply;
                    self.gas_switches.push(GasSwitch {
                        depth: Depth::new(d0),
                        gas: supply.gas,
                        time: Duration::new(t0),
                        cylinder: Some(index),
                    });
                }
            }
            let minutes = (t1 - t0) as f64 / 60.0;
            let ambient = ambient_mbar((d0 + d1) / 2, surface_mbar, salinity) / surface_mbar;
            let used = rmv_l_per_min * minutes * ambient;
            states[index].used += used;
            states[index].remaining -= used;
            previous_supply = Some(index);
        }

        for state in states {
            let size = state.supply.size.liters().max(0.001);
            let start_bar = state.supply.start_pressure.bar();
            let end_bar = (start_bar - state.used / size).max(0.0);
            self.gas_uses.push(GasUse {
                gas: state.supply.gas,
                size: state.supply.size,
                start_pressure: state.supply.start_pressure,
                used_liters: state.used,
                end_pressure: Pressure::from_bar(end_bar),
            });
        }
    }

    fn samples_without_switch_events(&self) -> Vec<Sample> {
        let ascent_rate = self.ascent_rate.max(0.1);
        let mut samples = vec![Sample {
            time: Duration::new(0),
            depth: Depth::ZERO,
            ..Default::default()
        }];
        let mut seconds = 0i32;
        for point in &self.points {
            seconds += point.duration.seconds;
            samples.push(Sample {
                time: Duration::new(seconds),
                depth: point.depth,
                ..Default::default()
            });
        }
        let mut current = self.points.last().map(|p| p.depth.meters()).unwrap_or(0.0);
        for stop in &self.stops {
            let stop_m = stop.depth.meters();
            seconds += ((current - stop_m) / ascent_rate * 60.0).round() as i32;
            samples.push(Sample {
                time: Duration::new(seconds),
                depth: stop.depth,
                ..Default::default()
            });
            seconds += stop.duration.seconds;
            samples.push(Sample {
                time: Duration::new(seconds),
                depth: stop.depth,
                ..Default::default()
            });
            current = stop_m;
        }
        seconds += (current / ascent_rate * 60.0).round() as i32;
        samples.push(Sample {
            time: Duration::new(seconds),
            depth: Depth::ZERO,
            ..Default::default()
        });
        samples
    }

    /// A single square bottom segment (descent, hold, then the generated
    /// ascent). A convenience for callers that do not build waypoints.
    pub fn square(
        depth: Depth,
        bottom_time: Duration,
        mode: BreathingMode,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
    ) -> Self {
        let descent = DecoSegment::square(depth, bottom_time.seconds as f64 / 60.0, mode);
        Self::compute(descent.points, deco_model, surface_bar, salinity)
    }

    /// The no-decompression limit at the deepest waypoint.
    pub fn ndl(&self, surface_bar: f64, salinity: i32) -> Option<Duration> {
        ndl(
            self.max_depth(),
            self.bottom_mode(),
            self.deco_model,
            surface_bar,
            salinity,
        )
    }

    /// The deepest entered depth.
    pub fn max_depth(&self) -> Depth {
        self.points
            .iter()
            .map(|p| p.depth)
            .max()
            .unwrap_or(Depth::ZERO)
    }

    /// Total time spent in the entered profile.
    pub fn bottom_time(&self) -> Duration {
        Duration::new(self.points.iter().map(|p| p.duration.seconds).sum())
    }

    /// The breathing mode of the deepest-or-last waypoint.
    pub fn bottom_mode(&self) -> BreathingMode {
        self.points.last().map(|p| p.mode).unwrap_or_default()
    }

    /// The gas carried in the cylinder (the mix, or the diluent).
    pub fn bottom_gas(&self) -> GasMix {
        self.bottom_mode().cylinder_gas()
    }

    /// The planned profile as dive samples.
    pub fn samples(&self) -> Vec<Sample> {
        let mut samples = self.samples_without_switch_events();
        if self.gas_switches.is_empty() {
            return samples;
        }
        samples.extend(self.gas_switches.iter().map(|switch| Sample {
            time: switch.time,
            depth: switch.depth,
            ..Default::default()
        }));
        samples.sort_by_key(|sample| sample.time);
        samples.dedup_by_key(|sample| sample.time);
        samples
    }

    /// Samples for the planner chart: depth, NDL, the decompression ceiling
    /// and the pressure of each cylinder while it is in use.
    pub fn graph_samples(
        &self,
        surface_bar: f64,
        salinity: i32,
        rmv_l_per_min: f64,
    ) -> Vec<Sample> {
        let mut samples = densify_samples(self.samples(), MAX_GRAPH_SAMPLES);
        match self.deco_model {
            DecoModel::Buhlmann { gf_low, gf_high } => {
                self.fill_buhlmann(&mut samples, gf_low, gf_high, surface_bar, salinity);
            }
            DecoModel::Vpmb { .. } => {
                self.fill_vpmb(&mut samples, surface_bar, salinity);
            }
        }
        if rmv_l_per_min > 0.0 {
            self.fill_pressures(&mut samples, rmv_l_per_min, surface_bar, salinity);
        }
        samples
    }

    /// NDL and gradient-factor ceiling along the profile. The low gradient
    /// factor is anchored at the deepest ceiling reached so far. Like a dive
    /// computer, the ceiling is shown as the next stop on the stop grid.
    fn fill_buhlmann(
        &self,
        samples: &mut [Sample],
        gf_low: f64,
        gf_high: f64,
        surface_bar: f64,
        salinity: i32,
    ) {
        const STEP_SECONDS: f64 = 2.0;
        let mut model = Buhlmann::new(surface_bar, salinity);
        let mut anchor_bar = model.gf_anchor_bar(&model.tissues, gf_low);
        let step_mm = (self.stop_step.max(0.1) * 1000.0).round() as i32;
        let bottom_seconds = self.bottom_time().seconds;
        let mut previous: Option<(i32, f64)> = None;
        for sample in samples.iter_mut() {
            let time = sample.time.seconds;
            let depth_m = sample.depth.meters();
            if let Some((t0, d0)) = previous {
                let seconds = (time - t0) as f64;
                if seconds > 0.0 {
                    let mode = self.mode_at_time(t0);
                    let steps = (seconds / STEP_SECONDS).ceil().max(1.0);
                    let minutes = seconds / 60.0 / steps;
                    for step in 0..steps as usize {
                        let mid = d0 + (depth_m - d0) * (step as f64 + 0.5) / steps;
                        let depth = Depth::from_meters(mid);
                        let gas = mode.gas_at(model.ambient_bar(depth));
                        model.add_segment_minutes(depth, minutes, gas);
                    }
                }
            }
            // The schedule anchors the low gradient factor at the start of the
            // ascent, so the anchor stops moving there.
            if time <= bottom_seconds {
                anchor_bar = anchor_bar.max(model.gf_anchor_bar(&model.tissues, gf_low));
            }
            let raw = model.gf_ceiling_depth_of(&model.tissues, gf_low, gf_high, anchor_bar);
            sample.stop_depth = Some(stop_grid_ceiling(raw, step_mm));
            if sample.depth.mm > 0 {
                let mode = self.mode_at_time(time);
                let gas = mode.gas_at(model.ambient_bar(sample.depth));
                sample.ndl =
                    Some(model.ndl_capped(sample.depth, gas, gf_high, Duration::new(99 * 60)));
            }
            previous = Some((time, depth_m));
        }
    }

    /// VPM-B only exposes its schedule, so the ceiling follows the planned
    /// stops during the ascent. The NDL is the extra time at the current depth
    /// before the profile so far would need a stop, and zero while the diver is
    /// in decompression.
    fn fill_vpmb(&self, samples: &mut [Sample], surface_bar: f64, salinity: i32) {
        let DecoModel::Vpmb { conservatism } = self.deco_model else {
            return;
        };
        let bottom_seconds = self.bottom_time().seconds;
        let mut stop_ends = Vec::with_capacity(self.stops.len());
        let mut time = bottom_seconds;
        let mut depth_m = self.points.last().map(|p| p.depth.meters()).unwrap_or(0.0);
        let rate = self.ascent_rate.max(0.1);
        for stop in &self.stops {
            time += ((depth_m - stop.depth.meters()).max(0.0) / rate * 60.0).round() as i32;
            time += stop.duration.seconds;
            stop_ends.push((time, stop.depth));
            depth_m = stop.depth.meters();
        }

        let base = DecoSegment {
            ascent_rate: self.ascent_rate,
            stop_step: self.stop_step,
            gas_switches: self.ascent_switches(),
            ..Default::default()
        };
        let has_stops = |points: Vec<PlanPoint>| {
            !crate::deco::vpmb::plan(
                &DecoSegment {
                    points,
                    ..base.clone()
                },
                surface_bar,
                salinity,
                conservatism,
                VPMB_TIMESTEP_SECONDS,
            )
            .stops
            .is_empty()
        };
        let in_deco_ascent = !self.stops.is_empty();
        let mut profile: Vec<PlanPoint> = Vec::new();
        let mut previous: Option<i32> = None;
        // Staying at or below the depth where a stop became necessary keeps it
        // necessary, which skips most schedule runs on the bottom.
        let mut deco_since: Option<i32> = None;
        for sample in samples.iter_mut() {
            let time = sample.time.seconds;
            if let Some(t0) = previous.filter(|t0| time > *t0) {
                profile.push(PlanPoint {
                    depth: sample.depth,
                    duration: Duration::new(time - t0),
                    mode: self.mode_at_time(t0),
                });
            }
            previous = Some(time);
            if time >= bottom_seconds {
                sample.stop_depth = Some(
                    stop_ends
                        .iter()
                        .find(|(end, _)| *end > time)
                        .map(|(_, depth)| *depth)
                        .unwrap_or(Depth::ZERO),
                );
            }
            if sample.depth.mm <= 0 {
                continue;
            }
            if (in_deco_ascent && time >= bottom_seconds)
                || deco_since.is_some_and(|depth| sample.depth.mm >= depth)
            {
                sample.ndl = Some(Duration::ZERO);
                continue;
            }
            let mode = self.mode_at_time(time);
            let with_extra = |minutes: i32| {
                let mut points = profile.clone();
                if minutes > 0 {
                    points.push(PlanPoint {
                        depth: sample.depth,
                        duration: Duration::from_minutes(minutes),
                        mode,
                    });
                }
                has_stops(points)
            };
            const CAP_MINUTES: i32 = 99;
            sample.ndl = Some(if with_extra(0) {
                deco_since = Some(sample.depth.mm);
                Duration::ZERO
            } else if !with_extra(CAP_MINUTES) {
                Duration::from_minutes(CAP_MINUTES)
            } else {
                let (mut lo, mut hi) = (0, CAP_MINUTES);
                while hi - lo > 1 {
                    let mid = (lo + hi) / 2;
                    if with_extra(mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                Duration::from_minutes(lo)
            });
        }
    }

    /// The ascent gas switches as the schedule sees them: the deepest switch
    /// to each gas after the entered profile.
    fn ascent_switches(&self) -> Vec<(Depth, BreathingMode)> {
        let bottom_seconds = self.bottom_time().seconds;
        let bottom_gas = self.bottom_gas();
        let mut switches: Vec<(Depth, BreathingMode)> = Vec::new();
        for switch in &self.gas_switches {
            if switch.time.seconds < bottom_seconds || switch.gas == bottom_gas {
                continue;
            }
            match switches
                .iter_mut()
                .find(|(_, mode)| mode.cylinder_gas() == switch.gas)
            {
                Some((depth, _)) => *depth = (*depth).max(switch.depth),
                None => switches.push((switch.depth, BreathingMode::OpenCircuit(switch.gas))),
            }
        }
        switches
    }

    /// Per-cylinder remaining pressure. Switch events carry the cylinder
    /// index, so this replays each bottle independently rather than
    /// collapsing cylinders that contain the same gas. A cylinder only has a
    /// pressure from the moment it is first breathed until it is last used.
    fn fill_pressures(
        &self,
        samples: &mut [Sample],
        rmv_l_per_min: f64,
        surface_bar: f64,
        salinity: i32,
    ) {
        if self.gas_uses.is_empty() || samples.is_empty() {
            return;
        }
        let surface_mbar = surface_bar * 1000.0;
        let sizes: Vec<f64> = self
            .gas_uses
            .iter()
            .map(|gas| gas.size.liters().max(0.001))
            .collect();
        let mut remaining: Vec<f64> = self
            .gas_uses
            .iter()
            .zip(&sizes)
            .map(|(gas, size)| size * gas.start_pressure.bar())
            .collect();
        let mut history: Vec<Vec<f64>> = Vec::with_capacity(samples.len());
        let mut used_span: Vec<Option<(usize, usize)>> = vec![None; remaining.len()];
        let mut active = 0usize;
        let mut switch_index = 0usize;
        for index in 0..samples.len() {
            while let Some(switch) = self.gas_switches.get(switch_index) {
                if switch.time.seconds > samples[index].time.seconds {
                    break;
                }
                if let Some(cylinder) = switch.cylinder {
                    active = cylinder.min(remaining.len() - 1);
                }
                switch_index += 1;
            }
            history.push(remaining.clone());
            let Some(next) = samples.get(index + 1) else {
                break;
            };
            let dt = (next.time.seconds - samples[index].time.seconds) as f64 / 60.0;
            if dt <= 0.0 {
                continue;
            }
            let span = used_span[active].get_or_insert((index, index + 1));
            span.1 = index + 1;
            let avg_depth = (samples[index].depth.mm + next.depth.mm) as f64 / 2.0;
            let ambient =
                ambient_mbar(avg_depth.round() as i32, surface_mbar, salinity) / surface_mbar;
            remaining[active] = (remaining[active] - rmv_l_per_min * dt * ambient).max(0.0);
        }
        for (index, sample) in samples.iter_mut().enumerate() {
            let Some(liters) = history.get(index) else {
                continue;
            };
            sample.pressures = used_span
                .iter()
                .enumerate()
                .filter(|(_, span)| span.is_some_and(|(from, to)| (from..=to).contains(&index)))
                .map(|(cylinder, _)| SensorPressure {
                    sensor: cylinder as i16,
                    pressure: Pressure::new(
                        (liters[cylinder] / sizes[cylinder] * 1000.0).max(0.0) as i32
                    ),
                })
                .collect();
        }
    }

    /// The largest TTS in the plan: the ascent time from the end of the
    /// entered profile, where the decompression obligation peaks.
    pub fn max_tts(&self) -> Duration {
        Duration::new((self.total_time().seconds - self.bottom_time().seconds).max(0))
    }

    /// The largest TTS as a dive computer that ignores future gas switches
    /// would show it: the ascent from the end of the entered profile computed
    /// on the bottom gas only.
    pub fn max_tts_current_gas(&self, surface_bar: f64, salinity: i32) -> Duration {
        let segment = DecoSegment {
            points: self.points.clone(),
            ascent_rate: self.ascent_rate,
            stop_step: self.stop_step,
            gas_switches: Vec::new(),
            ..Default::default()
        };
        let stops = match self.deco_model {
            DecoModel::Buhlmann { gf_low, gf_high } => Buhlmann::new(surface_bar, salinity)
                .deco_schedule(&DecoSegment {
                    gf_low,
                    gf_high,
                    ..segment
                }),
            DecoModel::Vpmb { conservatism } => {
                crate::deco::vpmb::plan(
                    &segment,
                    surface_bar,
                    salinity,
                    conservatism,
                    VPMB_TIMESTEP_SECONDS,
                )
                .stops
            }
        };
        let start = self.points.last().map(|p| p.depth.meters()).unwrap_or(0.0);
        Duration::new(ascent_seconds(start, &stops, self.ascent_rate))
    }

    /// The breathing mode at an elapsed time: the waypoint's mode during the
    /// entered profile, then the automatic switches during the ascent.
    fn mode_at_time(&self, seconds: i32) -> BreathingMode {
        if seconds < self.bottom_time().seconds {
            return self.mode_at(seconds);
        }
        let mut mode = self.bottom_mode();
        if matches!(mode, BreathingMode::OpenCircuit(_)) {
            for switch in &self.gas_switches {
                if switch.time.seconds <= seconds {
                    mode = BreathingMode::OpenCircuit(switch.gas);
                }
            }
        }
        mode
    }

    /// Total run time including descent, bottom time, stops and ascent.
    pub fn total_time(&self) -> Duration {
        self.samples().last().map(|s| s.time).unwrap_or_default()
    }

    /// Cumulative gas used over the planned profile, as `(elapsed, litres)`.
    /// Used by the planner chart; the gas mix does not matter for the total.
    pub fn gas_usage_series(
        &self,
        rmv_l_per_min: f64,
        surface_bar: f64,
        salinity: i32,
    ) -> Vec<(i32, f64)> {
        let surface_mbar = surface_bar * 1000.0;
        let samples = self.samples();
        let mut out = vec![(0, 0.0)];
        let mut liters = 0.0;
        for window in samples.windows(2) {
            let dt_min = (window[1].time.seconds - window[0].time.seconds) as f64 / 60.0;
            if dt_min > 0.0 {
                let avg_depth_mm = (window[0].depth.mm + window[1].depth.mm) / 2;
                let ambient = ambient_mbar(avg_depth_mm, surface_mbar, salinity) / surface_mbar;
                liters += rmv_l_per_min * dt_min * ambient;
            }
            out.push((window[1].time.seconds, liters));
        }
        out
    }

    /// Total time spent at decompression stops.
    pub fn deco_time(&self) -> Duration {
        Duration::new(self.stops.iter().map(|s| s.duration.seconds).sum())
    }

    /// Litres of gas needed to run the plan at a given surface RMV
    /// (respiratory minute volume).
    ///
    /// Integrates the planned profile: for each segment the average ambient
    /// pressure scales the surface consumption.
    pub fn gas_needs_liters(&self, rmv_l_per_min: f64, surface_bar: f64, salinity: i32) -> f64 {
        let surface_mbar = surface_bar * 1000.0;
        let samples = self.samples();
        let mut liters = 0.0;
        for window in samples.windows(2) {
            let dt_min = (window[1].time.seconds - window[0].time.seconds) as f64 / 60.0;
            if dt_min <= 0.0 {
                continue;
            }
            let avg_depth_mm = (window[0].depth.mm + window[1].depth.mm) / 2;
            let ambient_ata = ambient_mbar(avg_depth_mm, surface_mbar, salinity) / surface_mbar;
            liters += rmv_l_per_min * dt_min * ambient_ata;
        }
        liters
    }

    /// Litres of open-circuit gas needed to ascend from the end of the bottom
    /// time to the surface, following the schedule.
    ///
    /// This is the bailout requirement for a rebreather plan, and the ascent
    /// requirement for an open-circuit one.
    pub fn bailout_liters(&self, rmv_l_per_min: f64, surface_bar: f64, salinity: i32) -> f64 {
        let surface_mbar = surface_bar * 1000.0;
        let samples = self.samples();
        let bottom_mm = self.max_depth().mm;
        let start = samples
            .iter()
            .rposition(|s| s.depth.mm >= bottom_mm)
            .unwrap_or(0);
        let mut liters = 0.0;
        for window in samples[start..].windows(2) {
            let dt_min = (window[1].time.seconds - window[0].time.seconds) as f64 / 60.0;
            if dt_min <= 0.0 {
                continue;
            }
            let avg_depth_mm = (window[0].depth.mm + window[1].depth.mm) / 2;
            let ambient_ata = ambient_mbar(avg_depth_mm, surface_mbar, salinity) / surface_mbar;
            liters += rmv_l_per_min * dt_min * ambient_ata;
        }
        liters
    }

    /// A human-readable plan summary, suitable for dive notes.
    pub fn summary(&self) -> String {
        let mode = match self.bottom_mode() {
            BreathingMode::OpenCircuit(_) => "OC".to_string(),
            BreathingMode::ClosedCircuit { setpoint_bar, .. } => {
                format!("CCR, setpoint {setpoint_bar:.1} bar")
            }
            BreathingMode::PassiveSemiClosed { .. } => "pSCR".to_string(),
        };
        let deco = match self.deco_model {
            DecoModel::Buhlmann { gf_low, gf_high } => {
                format!("GF {:.0}/{:.0}", gf_low * 100.0, gf_high * 100.0)
            }
            DecoModel::Vpmb { conservatism } => format!("VPM-B +{conservatism}"),
        };
        let mut out = format!(
            "Planned dive: max {} for {} on {} ({mode}, {deco})\n",
            format_depth_m(self.max_depth()),
            format_duration(self.bottom_time()),
            self.bottom_gas().name(),
        );
        if self.points.len() > 2 {
            out.push_str("Profile:\n");
            let mut elapsed = 0;
            for point in &self.points {
                elapsed += point.duration.seconds;
                out.push_str(&format!(
                    "  {} at {}\n",
                    format_depth_m(point.depth),
                    format_duration(Duration::new(elapsed))
                ));
            }
        }
        if self.stops.is_empty() {
            out.push_str("No decompression stops required.\n");
        } else {
            out.push_str("Decompression stops:\n");
            for stop in &self.stops {
                out.push_str(&format!(
                    "  {} for {}\n",
                    format_depth_m(stop.depth),
                    format_duration(stop.duration)
                ));
            }
        }
        if !self.gas_switches.is_empty() {
            out.push_str("Gas switches:\n");
            for switch in &self.gas_switches {
                out.push_str(&format!(
                    "  {} at {} ({})\n",
                    switch.gas.name(),
                    format_depth_m(switch.depth),
                    format_duration(switch.time)
                ));
            }
        }
        if !self.gas_uses.is_empty() {
            out.push_str("Gas left:\n");
            for gas in &self.gas_uses {
                out.push_str(&format!(
                    "  {}: {:.0} bar ({:.0} L used)\n",
                    gas.gas.name(),
                    gas.end_pressure.bar(),
                    gas.used_liters
                ));
            }
        }
        out.push_str(&format!(
            "Total runtime: {}",
            format_duration(self.total_time())
        ));
        out
    }

    /// Build a dive from this plan. The caller assigns the id and number and
    /// may adjust the start time.
    ///
    /// Each distinct gas becomes a cylinder and every change of gas between
    /// waypoints becomes a gas-switch event, so the saved dive shows the same
    /// switches the planner does.
    pub fn to_dive(&self, when: Timestamp, salinity: i32) -> Dive {
        let (cylinders, events) = self.cylinders_and_events();

        // A CCR sample carries its segment's setpoint.
        let mut samples = self.samples();
        for sample in &mut samples {
            if let BreathingMode::ClosedCircuit { setpoint_bar, .. } =
                self.mode_at(sample.time.seconds)
            {
                sample.setpoint = Some(O2Pressure::from_bar(setpoint_bar));
            }
        }
        let duration = samples.last().map(|s| s.time).unwrap_or_default();

        Dive {
            when,
            duration: Some(duration),
            max_depth: Some(self.max_depth()),
            salinity: Some(salinity),
            notes: self.summary(),
            tags: vec!["planned".to_string()],
            cylinders,
            computers: vec![DiveComputer {
                model: "Planner".to_string(),
                divemode: divemode_of(self.bottom_mode()),
                duration: Some(duration),
                max_depth: Some(self.max_depth()),
                samples,
                events,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Gas-switch and dive-mode events, with cylinder indices matching
    /// [`Self::to_dive`].
    pub fn events(&self) -> Vec<Event> {
        self.cylinders_and_events().1
    }

    fn cylinders_and_events(&self) -> (Vec<Cylinder>, Vec<Event>) {
        let mut cylinders: Vec<Cylinder> = Vec::new();
        if !self.gas_uses.is_empty() {
            cylinders = self
                .gas_uses
                .iter()
                .map(|gas| Cylinder {
                    gas: gas.gas,
                    size: Some(gas.size),
                    start_pressure: Some(gas.start_pressure),
                    end_pressure: Some(gas.end_pressure),
                    description: gas.gas.name(),
                    use_: CylinderUse::OcGas,
                    ..Default::default()
                })
                .collect();
        }

        let mut events: Vec<Event> = Vec::new();
        let mut previous: Option<BreathingMode> = None;
        let mut time = 0i32;
        for point in &self.points {
            let gas = point.mode.cylinder_gas();
            let index = match cylinders.iter().position(|c| c.gas == gas) {
                Some(index) => index,
                None => {
                    cylinders.push(Cylinder {
                        gas,
                        use_: cylinder_use(point.mode),
                        description: gas.name(),
                        ..Default::default()
                    });
                    cylinders.len() - 1
                }
            };
            if let Some(prev) = previous {
                if gas != prev.cylinder_gas() {
                    events.push(Event {
                        time: Duration::new(time),
                        name: "gaschange".to_string(),
                        gas: Some((index as i32, gas)),
                        ..Default::default()
                    });
                }
                if divemode_of(point.mode) != divemode_of(prev) {
                    events.push(Event {
                        time: Duration::new(time),
                        name: "modechange".to_string(),
                        divemode: Some(divemode_of(point.mode)),
                        ..Default::default()
                    });
                }
            }
            previous = Some(point.mode);
            time += point.duration.seconds;
        }
        for switch in &self.gas_switches {
            let index = switch
                .cylinder
                .filter(|index| *index < cylinders.len())
                .or_else(|| cylinders.iter().position(|c| c.gas == switch.gas))
                .unwrap_or_else(|| {
                    cylinders.push(Cylinder {
                        gas: switch.gas,
                        description: switch.gas.name(),
                        ..Default::default()
                    });
                    cylinders.len() - 1
                });
            if !events
                .iter()
                .any(|event| event.is_gas_change() && event.time == switch.time)
            {
                events.push(Event {
                    time: switch.time,
                    name: "gaschange".to_string(),
                    gas: Some((index as i32, switch.gas)),
                    ..Default::default()
                });
            }
        }
        if cylinders.is_empty() {
            cylinders.push(Cylinder::default());
        }
        events.sort_by_key(|event| event.time);
        (cylinders, events)
    }

    /// The breathing mode used at a given elapsed time. A switch happens at the
    /// start of the segment that uses the new mode.
    fn mode_at(&self, seconds: i32) -> BreathingMode {
        let mut cumulative = 0;
        for point in &self.points {
            cumulative += point.duration.seconds;
            if seconds < cumulative {
                return point.mode;
            }
        }
        self.bottom_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::AIR;

    fn oc() -> BreathingMode {
        BreathingMode::OpenCircuit(AIR)
    }

    #[test]
    fn deep_plan_has_stops_and_a_profile() {
        let plan = DivePlan::square(
            Depth::from_meters(40.0),
            Duration::from_minutes(40),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        assert!(!plan.stops.is_empty());
        assert!(plan.deco_time().seconds > 0);
        assert!(plan.total_time().seconds > plan.bottom_time().seconds);

        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        assert_eq!(dive.max_depth, Some(Depth::from_meters(40.0)));
        let computer = dive.primary_computer().unwrap();
        assert!(computer.samples.len() >= 4);
        assert_eq!(computer.samples.last().unwrap().depth, Depth::ZERO);
        assert!(dive.notes.contains("Total runtime"));
        assert!(dive.average_depth().is_some());
    }

    #[test]
    fn ccr_plan_saves_a_setpoint_dive() {
        let plan = DivePlan::square(
            Depth::from_meters(30.0),
            Duration::from_minutes(30),
            BreathingMode::ClosedCircuit {
                diluent: AIR,
                setpoint_bar: 1.3,
            },
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        let computer = dive.primary_computer().unwrap();
        assert_eq!(computer.divemode, Divemode::Ccr);
        assert!(computer.samples.iter().all(|s| s.setpoint.is_some()));
        assert!(dive.notes.contains("CCR"));
    }

    #[test]
    fn gas_needs_scale_with_depth_and_time() {
        let plan = DivePlan::square(
            Depth::from_meters(30.0),
            Duration::from_minutes(20),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        // About 20 min at ~4 ATA at 20 L/min is ~1600 L, plus descent/ascent.
        let liters = plan.gas_needs_liters(20.0, 1.01325, EN13319_SALINITY);
        assert!((1_500.0..2_200.0).contains(&liters), "gas was {liters}");
        // A lower RMV needs proportionally less gas.
        let half = plan.gas_needs_liters(10.0, 1.01325, EN13319_SALINITY);
        assert!((half * 2.0 - liters).abs() < 1e-6);
    }

    #[test]
    fn bailout_is_less_than_total_gas() {
        let plan = DivePlan::square(
            Depth::from_meters(40.0),
            Duration::from_minutes(40),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let total = plan.gas_needs_liters(20.0, 1.01325, EN13319_SALINITY);
        let bailout = plan.bailout_liters(20.0, 1.01325, EN13319_SALINITY);
        assert!(bailout > 0.0);
        assert!(
            bailout < total,
            "bailout {bailout} should be below total {total}"
        );
        // A no-stop dive still needs gas to get back up.
        let shallow = DivePlan::square(
            Depth::from_meters(18.0),
            Duration::from_minutes(20),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        assert!(shallow.bailout_liters(20.0, 1.01325, EN13319_SALINITY) > 0.0);
    }

    #[test]
    fn shallow_plan_needs_no_stops() {
        let plan = DivePlan::square(
            Depth::from_meters(15.0),
            Duration::from_minutes(20),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        assert!(plan.stops.is_empty());
        assert!(plan.summary().contains("No decompression stops"));
    }

    #[test]
    fn vpmb_plan_has_stops_and_a_summary() {
        let plan = DivePlan::square(
            Depth::from_meters(40.0),
            Duration::from_minutes(40),
            oc(),
            DecoModel::Vpmb { conservatism: 3 },
            1.01325,
            EN13319_SALINITY,
        );
        assert!(!plan.stops.is_empty());
        assert!(plan.summary().contains("VPM-B +3"));
        assert!(plan.ndl(1.01325, EN13319_SALINITY).is_some());
    }

    #[test]
    fn multilevel_vpmb_matches_subsurface() {
        // Subsurface planner CLI (sea water, 1013 mbar, air, 10 m/min ascent):
        // descend to 40 m over 2 min, 20 min at 40 m, ascend to 20 m over
        // 2 min, then 10 min at 20 m, followed by the generated ascent.
        let oc = BreathingMode::OpenCircuit(AIR);
        let point = |depth: f64, seconds: i32| PlanPoint {
            depth: Depth::from_meters(depth),
            duration: Duration::new(seconds),
            mode: oc,
        };
        let plan = DivePlan::compute(
            vec![
                point(40.0, 120),
                point(40.0, 1200),
                point(20.0, 120),
                point(20.0, 600),
            ],
            DecoModel::Vpmb { conservatism: 3 },
            1.013,
            10_300,
        );
        let stops: Vec<(f64, i32)> = plan
            .stops
            .iter()
            .map(|s| (s.depth.meters(), s.duration.seconds))
            .collect();
        assert_eq!(stops, vec![(12.0, 72), (9.0, 402), (6.0, 582), (3.0, 1122)]);
        assert_eq!(plan.total_time().seconds, 4338);
    }

    #[test]
    fn to_dive_records_gas_switches() {
        let air = BreathingMode::OpenCircuit(AIR);
        let ean50 = BreathingMode::OpenCircuit(GasMix::new(500, 0));
        let point = |depth: f64, seconds: i32, mode| PlanPoint {
            depth: Depth::from_meters(depth),
            duration: Duration::new(seconds),
            mode,
        };
        let plan = DivePlan::compute(
            vec![
                point(30.0, 90, air),
                point(30.0, 1200, air),
                point(21.0, 120, ean50),
            ],
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        assert_eq!(dive.cylinders.len(), 2);
        assert_eq!(dive.cylinders[0].gas, AIR);
        assert_eq!(dive.cylinders[1].gas, GasMix::new(500, 0));
        let dc = dive.primary_computer().unwrap();
        let switches: Vec<_> = dc.events.iter().filter(|e| e.is_gas_change()).collect();
        assert_eq!(switches.len(), 1);
        // The switch to EAN50 happens at the waypoint after the 20 min bottom.
        assert_eq!(switches[0].time.seconds, 90 + 1200);
        assert_eq!(switches[0].gas, Some((1, GasMix::new(500, 0))));
    }

    #[test]
    fn to_dive_sets_setpoints_per_segment() {
        let oc = BreathingMode::OpenCircuit(AIR);
        let ccr = BreathingMode::ClosedCircuit {
            diluent: AIR,
            setpoint_bar: 1.3,
        };
        let point = |seconds: i32, mode| PlanPoint {
            depth: Depth::from_meters(30.0),
            duration: Duration::new(seconds),
            mode,
        };
        let plan = DivePlan::compute(
            vec![point(90, oc), point(600, ccr)],
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        let dc = dive.primary_computer().unwrap();
        // The first, open-circuit segment has no setpoint; the CCR segment does.
        assert!(dc
            .samples
            .iter()
            .find(|s| s.time.seconds < 90)
            .unwrap()
            .setpoint
            .is_none());
        assert_eq!(
            dc.samples
                .iter()
                .find(|s| s.time.seconds >= 90)
                .unwrap()
                .setpoint,
            Some(O2Pressure::from_bar(1.3))
        );
        assert_eq!(dive.cylinders.len(), 1);
        assert_eq!(dive.cylinders[0].use_, CylinderUse::OcGas);
    }

    #[test]
    fn multi_gas_square_switches_and_accounts_for_gas() {
        let supplies = vec![
            GasSupply {
                gas: AIR,
                size: Volume::from_liters(15.0),
                start_pressure: Pressure::from_bar(200.0),
            },
            GasSupply {
                gas: GasMix::new(500, 0),
                size: Volume::from_liters(11.0),
                start_pressure: Pressure::from_bar(200.0),
            },
            GasSupply {
                gas: GasMix::new(1000, 0),
                size: Volume::from_liters(11.0),
                start_pressure: Pressure::from_bar(200.0),
            },
        ];
        let plan = DivePlan::square_with_gases(
            Depth::from_meters(40.0),
            Duration::from_minutes(25),
            &supplies,
            1.5,
            Depth::from_meters(1.0),
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        assert!(!plan.stops.is_empty());
        assert_eq!(plan.gas_switches.len(), 2, "{:?}", plan.gas_switches);
        assert_eq!(plan.gas_uses.len(), 3);
        assert_eq!(plan.gas_uses[0].gas, AIR);
        assert!(plan.gas_uses[0].used_liters > plan.gas_uses[1].used_liters);
        assert!(plan.gas_uses.iter().all(|gas| gas.used_liters > 0.0));
        assert!(plan
            .gas_uses
            .iter()
            .all(|gas| gas.end_pressure.mbar < gas.start_pressure.mbar));
        // The 70% gas switches around 10 m and oxygen at the shallow stop.
        assert!(plan
            .gas_switches
            .iter()
            .any(|switch| switch.gas.o2_permille == 500));
        assert!(plan
            .gas_switches
            .iter()
            .any(|switch| switch.gas.o2_permille == 1000));
        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        assert_eq!(dive.cylinders.len(), 3);
        assert!(dive
            .primary_computer()
            .unwrap()
            .events
            .iter()
            .any(|event| event.is_gas_change()));
    }

    #[test]
    fn two_air_bottles_switch_when_the_first_one_empties() {
        let supplies = vec![
            GasSupply {
                gas: AIR,
                size: Volume::from_liters(10.0),
                start_pressure: Pressure::from_bar(200.0),
            },
            GasSupply {
                gas: AIR,
                size: Volume::from_liters(10.0),
                start_pressure: Pressure::from_bar(200.0),
            },
        ];
        let plan = DivePlan::square_with_gases(
            Depth::from_meters(40.0),
            Duration::from_minutes(25),
            &supplies,
            1.6,
            Depth::from_meters(1.0),
            DecoModel::Buhlmann {
                gf_low: 0.85,
                gf_high: 0.85,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        assert_eq!(plan.gas_uses.len(), 2);
        assert!(plan.gas_uses[0].used_liters > 0.0);
        assert!(plan.gas_uses[1].used_liters > 0.0);
        assert!(plan.gas_uses[0].end_pressure.mbar > 0);
        let bottle_switch = plan
            .gas_switches
            .iter()
            .find(|switch| switch.cylinder == Some(1))
            .expect("switch to the second bottle");
        assert_eq!(bottle_switch.gas, AIR);
        let samples = plan.graph_samples(1.01325, EN13319_SALINITY, 20.0);
        let at_switch = samples
            .iter()
            .find(|sample| sample.time == bottle_switch.time)
            .expect("switch sample");
        assert_eq!(at_switch.pressures.len(), 2);
        assert!(at_switch.pressures[1].pressure.mbar > at_switch.pressures[0].pressure.mbar);
        let last = samples.last().expect("surface sample");
        assert_eq!(last.pressures.len(), 1);
        assert_eq!(last.pressures[0].sensor, 1);
    }

    #[test]
    fn seventy_percent_switches_at_ten_metres() {
        let supplies = vec![
            GasSupply {
                gas: AIR,
                size: Volume::from_liters(15.0),
                start_pressure: Pressure::from_bar(200.0),
            },
            GasSupply {
                gas: GasMix::new(700, 0),
                size: Volume::from_liters(11.0),
                start_pressure: Pressure::from_bar(200.0),
            },
        ];
        let plan = DivePlan::square_with_gases(
            Depth::from_meters(40.0),
            Duration::from_minutes(20),
            &supplies,
            1.5,
            Depth::from_meters(1.0),
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        let switch = plan
            .gas_switches
            .iter()
            .find(|switch| switch.gas.o2_permille == 700)
            .expect("70% switch");
        assert_eq!(switch.depth, Depth::from_meters(10.0));
    }

    #[test]
    fn graph_samples_are_minutely_and_have_live_ndl() {
        let plan = DivePlan::square(
            Depth::from_meters(18.0),
            Duration::from_minutes(20),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let samples = plan.graph_samples(1.01325, EN13319_SALINITY, 20.0);
        assert!(samples
            .windows(2)
            .all(|pair| pair[1].time.seconds - pair[0].time.seconds <= 60));
        let bottom = samples
            .iter()
            .find(|sample| sample.time.seconds >= plan.bottom_time().seconds)
            .expect("bottom sample");
        assert!(bottom.ndl.is_some());
        assert_eq!(samples.last().unwrap().depth, Depth::ZERO);
        // The NDL shrinks as the diver stays on the bottom.
        let early = samples
            .iter()
            .find(|sample| sample.time.seconds >= 120)
            .and_then(|sample| sample.ndl)
            .expect("early NDL");
        let late = bottom.ndl.expect("late NDL");
        assert!(late.seconds < early.seconds, "{late:?} < {early:?}");
    }

    #[test]
    fn ceiling_rises_during_the_bottom_and_stays_above_the_diver() {
        let plan = DivePlan::square(
            Depth::from_meters(45.0),
            Duration::from_minutes(25),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
        );
        assert!(!plan.stops.is_empty());
        let samples = plan.graph_samples(1.01325, EN13319_SALINITY, 18.0);
        let ceiling_at = |seconds| {
            samples
                .iter()
                .find(|sample| sample.time.seconds >= seconds)
                .and_then(|sample| sample.stop_depth)
                .expect("ceiling")
        };
        assert_eq!(ceiling_at(0), Depth::ZERO);
        let early = ceiling_at(300);
        let late = ceiling_at(plan.bottom_time().seconds);
        assert!(late.mm > early.mm, "{early:?} -> {late:?}");
        assert!(late.mm > 0);
        for sample in &samples {
            let ceiling = sample.stop_depth.expect("ceiling");
            assert!(
                ceiling.mm <= (sample.depth.mm + 2999) / 3000 * 3000,
                "ceiling {ceiling:?} below the diver at {:?}",
                sample.depth
            );
        }
        assert!(samples
            .iter()
            .all(|sample| sample.stop_depth.unwrap().mm % 3000 == 0));
        assert_eq!(samples.last().unwrap().stop_depth, Some(Depth::ZERO));
        // While holding a stop, the ceiling is that stop until it clears.
        let first_stop = plan.stops[0].depth;
        assert!(samples
            .iter()
            .any(|sample| sample.depth == first_stop && sample.stop_depth == Some(first_stop)));
    }

    #[test]
    fn current_gas_tts_ignores_deco_gases() {
        let bottom = GasSupply {
            gas: AIR,
            size: Volume::from_liters(24.0),
            start_pressure: Pressure::from_bar(200.0),
        };
        let deco = GasSupply {
            gas: GasMix::new(500, 0),
            size: Volume::from_liters(11.0),
            start_pressure: Pressure::from_bar(200.0),
        };
        let model = DecoModel::Buhlmann {
            gf_low: 0.3,
            gf_high: 0.7,
        };
        let plan = |supplies: &[GasSupply]| {
            DivePlan::square_with_gases(
                Depth::from_meters(45.0),
                Duration::from_minutes(25),
                supplies,
                1.6,
                Depth::ZERO,
                model,
                1.01325,
                EN13319_SALINITY,
                20.0,
            )
        };
        let air_only = plan(&[bottom]);
        assert_eq!(
            air_only.max_tts_current_gas(1.01325, EN13319_SALINITY),
            air_only.max_tts()
        );
        let with_deco = plan(&[bottom, deco]);
        let current = with_deco.max_tts_current_gas(1.01325, EN13319_SALINITY);
        assert_eq!(current, air_only.max_tts());
        assert!(current.seconds > with_deco.max_tts().seconds);

        let vpmb = DivePlan::square_with_gases(
            Depth::from_meters(45.0),
            Duration::from_minutes(25),
            &[bottom, deco],
            1.6,
            Depth::ZERO,
            DecoModel::Vpmb { conservatism: 3 },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        assert!(
            vpmb.max_tts_current_gas(1.01325, EN13319_SALINITY).seconds > vpmb.max_tts().seconds
        );
    }

    #[test]
    fn ndl_never_recovers_during_a_deco_ascent() {
        for model in [
            DecoModel::Vpmb { conservatism: 3 },
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
        ] {
            let plan = DivePlan::square(
                Depth::from_meters(45.0),
                Duration::from_minutes(25),
                oc(),
                model,
                1.01325,
                EN13319_SALINITY,
            );
            assert!(!plan.stops.is_empty());
            let samples = plan.graph_samples(1.01325, EN13319_SALINITY, 18.0);
            let bottom = plan.bottom_time().seconds;
            let mut previous = Duration::new(i32::MAX);
            for sample in samples.iter().filter(|s| s.depth.mm > 0) {
                let ndl = sample.ndl.expect("NDL");
                if sample.time.seconds >= bottom {
                    assert_eq!(ndl, Duration::ZERO, "{model:?} at {:?}", sample.time);
                } else if sample.time.seconds > 180 {
                    assert!(ndl <= previous, "{model:?} at {:?}", sample.time);
                    previous = ndl;
                }
            }
            let first_bottom = samples
                .iter()
                .find(|s| s.time.seconds > 180)
                .and_then(|s| s.ndl)
                .expect("bottom NDL");
            assert!(first_bottom.seconds > 0, "{model:?}");
        }
    }

    #[test]
    fn deco_gas_pressure_only_appears_while_in_use() {
        let supplies = vec![
            GasSupply {
                gas: AIR,
                size: Volume::from_liters(15.0),
                start_pressure: Pressure::from_bar(200.0),
            },
            GasSupply {
                gas: GasMix::new(500, 0),
                size: Volume::from_liters(7.0),
                start_pressure: Pressure::from_bar(200.0),
            },
        ];
        let plan = DivePlan::square_with_gases(
            Depth::from_meters(40.0),
            Duration::from_minutes(20),
            &supplies,
            1.6,
            Depth::from_meters(1.0),
            DecoModel::Buhlmann {
                gf_low: 0.3,
                gf_high: 0.7,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        let switch = plan
            .gas_switches
            .iter()
            .find(|switch| switch.gas.o2_permille == 500)
            .expect("EAN50 switch");
        let samples = plan.graph_samples(1.01325, EN13319_SALINITY, 20.0);
        let sensors =
            |sample: &Sample| -> Vec<i16> { sample.pressures.iter().map(|p| p.sensor).collect() };
        for sample in &samples {
            if sample.time.seconds < switch.time.seconds {
                assert_eq!(sensors(sample), vec![0], "at {:?}", sample.time);
            } else if sample.time.seconds > switch.time.seconds {
                assert_eq!(sensors(sample), vec![1], "at {:?}", sample.time);
            }
        }
        assert!(samples
            .iter()
            .any(|sample| sample.time == switch.time && sensors(sample) == vec![0, 1]));
    }

    #[test]
    fn gas_usage_series_is_monotonic_and_matches_the_total() {
        let plan = DivePlan::square(
            Depth::from_meters(30.0),
            Duration::from_minutes(20),
            oc(),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
        );
        let series = plan.gas_usage_series(20.0, 1.01325, EN13319_SALINITY);
        assert!(series.len() >= 2);
        assert_eq!(series.first().unwrap().0, 0);
        assert!(series.windows(2).all(|pair| pair[1].1 >= pair[0].1));
        let total = plan.gas_needs_liters(20.0, 1.01325, EN13319_SALINITY);
        assert!((series.last().unwrap().1 - total).abs() < 1e-6);
    }

    #[test]
    fn reserve_optimizer_stops_at_the_gas_limit() {
        let supplies = vec![GasSupply {
            gas: AIR,
            size: Volume::from_liters(12.0),
            start_pressure: Pressure::from_bar(200.0),
        }];
        let time = DivePlan::max_bottom_time_for_reserve(
            Depth::from_meters(30.0),
            &supplies,
            1.5,
            Depth::from_meters(1.0),
            Pressure::from_bar(50.0),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        assert!(time.seconds > 0);
        let plan = DivePlan::square_with_gases(
            Depth::from_meters(30.0),
            time,
            &supplies,
            1.5,
            Depth::from_meters(1.0),
            DecoModel::Buhlmann {
                gf_low: 1.0,
                gf_high: 1.0,
            },
            1.01325,
            EN13319_SALINITY,
            20.0,
        );
        assert!(plan.gas_uses[0].end_pressure.mbar >= 50_000);
    }

    #[test]
    fn vpmb_ndl_ignores_bottom_time() {
        // The NDL is a property of the depth and gas, not the planned bottom
        // time, so it must not change when the bottom time does.
        let make = |minutes: i32| {
            DivePlan::square(
                Depth::from_meters(30.0),
                Duration::from_minutes(minutes),
                oc(),
                DecoModel::Vpmb { conservatism: 3 },
                1.01325,
                EN13319_SALINITY,
            )
            .ndl(1.01325, EN13319_SALINITY)
        };
        assert_eq!(make(10), make(40));
    }

    #[test]
    fn long_profiles_are_bounded_and_keep_their_endpoints() {
        let base = vec![
            Sample {
                time: Duration::ZERO,
                ..Default::default()
            },
            Sample {
                time: Duration::new(36_000),
                depth: Depth::from_meters(30.0),
                ..Default::default()
            },
            Sample {
                time: Duration::new(72_000),
                ..Default::default()
            },
        ];
        let samples = densify_samples(base, MAX_GRAPH_SAMPLES);
        assert!(samples.len() <= MAX_GRAPH_SAMPLES);
        assert_eq!(samples.first().unwrap().time, Duration::ZERO);
        assert_eq!(samples.last().unwrap().time, Duration::new(72_000));
    }
}
