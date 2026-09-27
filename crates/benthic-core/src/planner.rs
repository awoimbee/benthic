//! Dive planning: turn a depth, bottom time and gas into a full plan
//! (decompression stops) and, optionally, a dive that can be saved.

use serde::{Deserialize, Serialize};

use crate::deco::vpmb::{Vpmb, VPMB_TIMESTEP_SECONDS};
use crate::deco::{BreathingMode, Buhlmann, DecoModel, DecoSegment, PlanPoint, Stop};
use crate::gas::{ambient_mbar, GasMix};
use crate::model::{Cylinder, Dive, DiveComputer, Divemode, Sample};
use crate::units::*;

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

impl DivePlan {
    /// Compute a plan from a bottom segment.
    /// Plan from a sequence of waypoints.
    pub fn compute(
        points: Vec<PlanPoint>,
        deco_model: DecoModel,
        surface_bar: f64,
        salinity: i32,
    ) -> Self {
        let segment = DecoSegment {
            points: points.clone(),
            ascent_rate: 10.0,
            stop_step: 3.0,
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
        Self {
            points,
            stops,
            ascent_rate: 10.0,
            stop_step: 3.0,
            deco_model,
        }
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
        let ascent_rate = self.ascent_rate.max(0.1);
        let mut samples = vec![Sample {
            time: Duration::new(0),
            depth: Depth::ZERO,
            ..Default::default()
        }];

        // The entered waypoints; the chart draws the ramps between them.
        let mut seconds = 0i32;
        for point in &self.points {
            seconds += point.duration.seconds;
            samples.push(Sample {
                time: Duration::new(seconds),
                depth: point.depth,
                ..Default::default()
            });
        }

        // The generated ascent and stops.
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
        out.push_str(&format!(
            "Total runtime: {}",
            format_duration(self.total_time())
        ));
        out
    }

    /// Build a dive from this plan. The caller assigns the id and number and
    /// may adjust the start time.
    ///
    /// Only the bottom gas is written as a cylinder; gas switches between
    /// waypoints are not yet recorded as events.
    pub fn to_dive(&self, when: Timestamp, salinity: i32) -> Dive {
        let (gas, setpoint, divemode) = match self.bottom_mode() {
            BreathingMode::OpenCircuit(gas) => (gas, None, Divemode::OpenCircuit),
            BreathingMode::ClosedCircuit {
                diluent,
                setpoint_bar,
            } => (diluent, Some(setpoint_bar), Divemode::Ccr),
            BreathingMode::PassiveSemiClosed { diluent, .. } => (diluent, None, Divemode::Pscr),
        };

        let mut samples = self.samples();
        if let Some(setpoint) = setpoint {
            for sample in &mut samples {
                sample.setpoint = Some(O2Pressure::from_bar(setpoint));
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
            cylinders: vec![Cylinder {
                gas,
                ..Default::default()
            }],
            computers: vec![DiveComputer {
                model: "Planner".to_string(),
                divemode,
                duration: Some(duration),
                max_depth: Some(self.max_depth()),
                samples,
                ..Default::default()
            }],
            ..Default::default()
        }
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
