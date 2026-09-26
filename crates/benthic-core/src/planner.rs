//! Dive planning: turn a depth, bottom time and gas into a full plan
//! (decompression stops) and, optionally, a dive that can be saved.

use serde::{Deserialize, Serialize};

use crate::deco::{Buhlmann, DecoSegment, Stop};
use crate::gas::{ambient_mbar, GasMix};
use crate::model::{Cylinder, Dive, DiveComputer, Sample};
use crate::units::*;

/// A computed dive plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivePlan {
    pub depth: Depth,
    pub bottom_time: Duration,
    pub gas: GasMix,
    pub gf_low: f64,
    pub gf_high: f64,
    pub stops: Vec<Stop>,
    pub descent_rate: f64,
    pub ascent_rate: f64,
}

impl DivePlan {
    /// Compute a plan from a bottom segment.
    pub fn compute(
        depth: Depth,
        bottom_time: Duration,
        gas: GasMix,
        gf_low: f64,
        gf_high: f64,
        surface_bar: f64,
        salinity: i32,
    ) -> Self {
        let model = Buhlmann::new(surface_bar, salinity);
        let stops = model.deco_schedule(&DecoSegment {
            bottom_depth: depth,
            bottom_minutes: bottom_time.seconds as f64 / 60.0,
            gas,
            gf_low,
            gf_high,
            ..Default::default()
        });
        Self {
            depth,
            bottom_time,
            gas,
            gf_low,
            gf_high,
            stops,
            descent_rate: 20.0,
            ascent_rate: 10.0,
        }
    }

    /// The planned profile as dive samples.
    pub fn samples(&self) -> Vec<Sample> {
        let bottom_m = self.depth.meters();
        let mut samples = vec![Sample {
            time: Duration::new(0),
            depth: Depth::ZERO,
            ..Default::default()
        }];

        let mut seconds = 0.0f64;
        if bottom_m > 0.0 {
            let descent = bottom_m / self.descent_rate * 60.0;
            samples.push(Sample {
                time: Duration::new((descent / 2.0).round() as i32),
                depth: Depth::from_meters(bottom_m / 2.0),
                ..Default::default()
            });
            seconds += descent;
            samples.push(Sample {
                time: Duration::new(seconds.round() as i32),
                depth: self.depth,
                ..Default::default()
            });
        }

        seconds += self.bottom_time.seconds as f64;
        samples.push(Sample {
            time: Duration::new(seconds.round() as i32),
            depth: self.depth,
            ..Default::default()
        });

        let mut current = bottom_m;
        for stop in &self.stops {
            let stop_m = stop.depth.meters();
            seconds += (current - stop_m) / self.ascent_rate * 60.0;
            samples.push(Sample {
                time: Duration::new(seconds.round() as i32),
                depth: stop.depth,
                ..Default::default()
            });
            seconds += stop.duration.seconds as f64;
            samples.push(Sample {
                time: Duration::new(seconds.round() as i32),
                depth: stop.depth,
                ..Default::default()
            });
            current = stop_m;
        }

        seconds += current / self.ascent_rate * 60.0;
        samples.push(Sample {
            time: Duration::new(seconds.round() as i32),
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

    /// A human-readable plan summary, suitable for dive notes.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "Planned dive: {} for {} on {} (GF {:.0}/{:.0})\n",
            format_depth_m(self.depth),
            format_duration(self.bottom_time),
            self.gas.name(),
            self.gf_low * 100.0,
            self.gf_high * 100.0,
        );
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
    pub fn to_dive(&self, when: Timestamp, salinity: i32) -> Dive {
        let samples = self.samples();
        let duration = samples.last().map(|s| s.time).unwrap_or_default();
        Dive {
            when,
            duration: Some(duration),
            max_depth: Some(self.depth),
            salinity: Some(salinity),
            notes: self.summary(),
            tags: vec!["planned".to_string()],
            cylinders: vec![Cylinder {
                gas: self.gas,
                ..Default::default()
            }],
            computers: vec![DiveComputer {
                model: "Planner".to_string(),
                duration: Some(duration),
                max_depth: Some(self.depth),
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

    #[test]
    fn deep_plan_has_stops_and_a_profile() {
        let plan = DivePlan::compute(
            Depth::from_meters(40.0),
            Duration::from_minutes(40),
            AIR,
            1.0,
            1.0,
            1.01325,
            EN13319_SALINITY,
        );
        assert!(!plan.stops.is_empty());
        assert!(plan.deco_time().seconds > 0);
        assert!(plan.total_time().seconds > plan.bottom_time.seconds);

        let dive = plan.to_dive(1_000, EN13319_SALINITY);
        assert_eq!(dive.max_depth, Some(Depth::from_meters(40.0)));
        let computer = dive.primary_computer().unwrap();
        assert!(computer.samples.len() >= 4);
        assert_eq!(computer.samples.last().unwrap().depth, Depth::ZERO);
        assert!(dive.notes.contains("Total runtime"));
        assert!(dive.average_depth().is_some());
    }

    #[test]
    fn gas_needs_scale_with_depth_and_time() {
        let plan = DivePlan::compute(
            Depth::from_meters(30.0),
            Duration::from_minutes(20),
            AIR,
            1.0,
            1.0,
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
    fn shallow_plan_needs_no_stops() {
        let plan = DivePlan::compute(
            Depth::from_meters(15.0),
            Duration::from_minutes(20),
            AIR,
            1.0,
            1.0,
            1.01325,
            EN13319_SALINITY,
        );
        assert!(plan.stops.is_empty());
        assert!(plan.summary().contains("No decompression stops"));
    }
}
