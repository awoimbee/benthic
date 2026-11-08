//! Dive planning: turn a depth, bottom time and gas into a full plan
//! (decompression stops) and, optionally, a dive that can be saved.

use serde::{Deserialize, Serialize};

use crate::deco::vpmb::{Vpmb, VPMB_TIMESTEP_SECONDS};
use crate::deco::{BreathingMode, Buhlmann, DecoModel, DecoSegment, PlanPoint, Stop};
use crate::gas::{ambient_mbar, mod_depth_mm, GasMix};
use crate::model::{Cylinder, CylinderUse, Dive, DiveComputer, Divemode, Event, Sample};
use crate::units::*;

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
    /// of the automatic switches.
    fn account_gas(
        &mut self,
        supplies: &[GasSupply],
        switches: &[(Depth, BreathingMode)],
        rmv_l_per_min: f64,
        surface_bar: f64,
        salinity: i32,
    ) {
        let raw = self.samples_without_switch_events();
        if raw.len() < 2 {
            return;
        }
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
            let mut crossings: Vec<(i32, i32)> = switch_list
                .iter()
                .filter(|(depth, _)| depth.mm > d1 && depth.mm < d0)
                .map(|(depth, _)| (depth.mm, depth.mm))
                .collect();
            crossings.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
            for (_time, depth) in crossings {
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

        let surface_mbar = surface_bar * 1000.0;
        let mut used: std::collections::BTreeMap<(u16, u16), f64> = Default::default();
        let mut previous_gas = bottom_gas;
        for (index, window) in vertices.windows(2).enumerate() {
            let (t0, d0) = window[0];
            let (t1, d1) = window[1];
            if t1 <= t0 {
                continue;
            }
            let gas = if t0 < bottom_seconds {
                bottom_gas
            } else {
                gas_at(Depth::new(d0))
            };
            if t0 >= bottom_seconds && gas != previous_gas {
                self.gas_switches.push(GasSwitch {
                    depth: Depth::new(d0),
                    gas,
                    time: Duration::new(t0),
                });
                previous_gas = gas;
            }
            let minutes = (t1 - t0) as f64 / 60.0;
            let ambient = ambient_mbar((d0 + d1) / 2, surface_mbar, salinity) / surface_mbar;
            *used.entry((gas.o2_permille, gas.he_permille)).or_default() +=
                rmv_l_per_min * minutes * ambient;
            let _ = index;
        }

        let mut remaining = used;
        for supply in supplies {
            let key = (supply.gas.o2_permille, supply.gas.he_permille);
            let consumed = remaining.get_mut(&key).map(std::mem::take);
            let liters = consumed.unwrap_or(0.0);
            let available = supply.size.liters().max(0.001) * supply.start_pressure.bar();
            let used_from_this = liters.min(available.max(0.0));
            let start_bar = supply.start_pressure.bar();
            let end_bar = (start_bar - used_from_this / supply.size.liters().max(0.001)).max(0.0);
            self.gas_uses.push(GasUse {
                gas: supply.gas,
                size: supply.size,
                start_pressure: supply.start_pressure,
                used_liters: used_from_this,
                end_pressure: Pressure::from_bar(end_bar),
            });
            if let Some(leftover) = remaining.get_mut(&key) {
                *leftover = (liters - used_from_this).max(0.0);
            }
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

    /// Total run time including descent, bottom time, stops and ascent.
    pub fn total_time(&self) -> Duration {
        self.samples().last().map(|s| s.time).unwrap_or_default()
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
            let index = match cylinders.iter().position(|c| c.gas == switch.gas) {
                Some(index) => index,
                None => {
                    cylinders.push(Cylinder {
                        gas: switch.gas,
                        description: switch.gas.name(),
                        ..Default::default()
                    });
                    cylinders.len() - 1
                }
            };
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
}
