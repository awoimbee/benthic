use dioxus::prelude::*;

use benthic_core::units::{format_duration, Depth};
use benthic_core::Dive;

use crate::state::AppState;

/// A lightweight SVG depth profile for the selected dive.
#[component]
pub fn DiveProfile(dive: Dive) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let samples = dive
        .primary_computer()
        .map(|dc| dc.samples.clone())
        .unwrap_or_default();

    if samples.len() < 2 {
        return rsx! { div { class: "profile empty-hint", "No profile data for this dive." } };
    }

    let max_t = samples.last().map(|s| s.time.seconds).unwrap_or(1).max(1) as f64;
    let max_d = samples.iter().map(|s| s.depth.mm).max().unwrap_or(1).max(1) as f64;

    let points: String = samples
        .iter()
        .map(|s| {
            let x = s.time.seconds as f64 / max_t * 100.0;
            let y = s.depth.mm as f64 / max_d * 90.0 + 5.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let caption = format!(
        "{} · max {} · {} samples",
        dive.duration()
            .map(format_duration)
            .unwrap_or_else(|| "—".to_string()),
        prefs.depth(Depth::new(max_d as i32)),
        samples.len()
    );

    rsx! {
        div { class: "profile",
            svg {
                class: "profile-svg",
                view_box: "0 0 100 100",
                preserve_aspect_ratio: "none",
                polyline {
                    points: "{points}",
                    style: "fill: none; stroke: #4cc9f0; stroke-width: 1; vector-effect: non-scaling-stroke;",
                }
            }
            div { class: "profile-caption", "{caption}" }
        }
    }
}
