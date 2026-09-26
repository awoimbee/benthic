use dioxus::prelude::*;

use benthic_core::units::format_duration;
use benthic_core::Dive;

use crate::state::AppState;

/// An interactive SVG depth profile with temperature/pressure overlays, event
/// markers and a scrubber readout.
#[component]
pub fn DiveProfile(dive: Dive) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();

    let mut show_temp = use_signal(|| false);
    let mut show_pressure = use_signal(|| true);
    let mut cursor = use_signal(|| 0usize);

    let samples = dive
        .primary_computer()
        .map(|dc| dc.samples.clone())
        .unwrap_or_default();
    let events = dive
        .primary_computer()
        .map(|dc| dc.events.clone())
        .unwrap_or_default();

    if samples.len() < 2 {
        return rsx! { div { class: "profile empty-hint", "No profile data for this dive." } };
    }

    let max_t = samples.last().map(|s| s.time.seconds).unwrap_or(1).max(1) as f64;
    let max_d = samples.iter().map(|s| s.depth.mm).max().unwrap_or(1).max(1) as f64;

    let depth_points: String = samples
        .iter()
        .map(|s| {
            let x = s.time.seconds as f64 / max_t * 100.0;
            let y = s.depth.mm as f64 / max_d * 90.0 + 5.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let temperature = series(
        samples
            .iter()
            .filter_map(|s| s.temperature.map(|t| (s.time.seconds, t.mkelvin as i32))),
        max_t,
        false,
    );
    let pressure = series(
        samples.iter().filter_map(|s| {
            s.pressures
                .iter()
                .find(|p| p.sensor == 0)
                .or_else(|| s.pressures.first())
                .map(|p| (s.time.seconds, p.pressure.mbar))
        }),
        max_t,
        false,
    );

    let event_marks: Vec<(f64, String)> = events
        .iter()
        .filter(|e| e.time.seconds >= 0)
        .map(|e| {
            (
                e.time.seconds as f64 / max_t * 100.0,
                if e.name.is_empty() {
                    "event".to_string()
                } else {
                    e.name.clone()
                },
            )
        })
        .collect();

    let index = (cursor)().min(samples.len() - 1);
    let sample = &samples[index];
    let cursor_x = sample.time.seconds as f64 / max_t * 100.0;
    let cursor_y = sample.depth.mm as f64 / max_d * 90.0 + 5.0;

    let mut readout = vec![format_duration(sample.time), prefs.depth(sample.depth)];
    if let Some(t) = sample.temperature {
        readout.push(prefs.temperature(t));
    }
    if let Some(p) = sample
        .pressures
        .iter()
        .find(|p| p.sensor == 0)
        .or_else(|| sample.pressures.first())
    {
        readout.push(prefs.pressure(p.pressure));
    }
    if let Some(ndl) = sample.ndl {
        readout.push(format!("NDL {}", format_duration(ndl)));
    }
    let readout = readout.join("  ·  ");

    let temp_on = (show_temp)();
    let pressure_on = (show_pressure)();

    rsx! {
        div { class: "profile",
            svg {
                class: "profile-svg",
                view_box: "0 0 100 100",
                preserve_aspect_ratio: "none",
                if pressure_on {
                    if let Some(points) = &pressure {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #7ee787; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                        }
                    }
                }
                if temp_on {
                    if let Some(points) = &temperature {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #f5a623; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                        }
                    }
                }
                polyline {
                    points: "{depth_points}",
                    style: "fill: none; stroke: #4cc9f0; stroke-width: 1.5; vector-effect: non-scaling-stroke;",
                }
                for (x, name) in event_marks {
                    line {
                        key: "{name}-{x}",
                        x1: "{x}",
                        y1: "4",
                        x2: "{x}",
                        y2: "10",
                        style: "stroke: #c792ea; stroke-width: 1; vector-effect: non-scaling-stroke;",
                    }
                }
                line {
                    x1: "{cursor_x}",
                    y1: "0",
                    x2: "{cursor_x}",
                    y2: "100",
                    style: "stroke: #dce8f2; stroke-width: 0.5; vector-effect: non-scaling-stroke; opacity: 0.6;",
                }
                circle {
                    cx: "{cursor_x}",
                    cy: "{cursor_y}",
                    r: "1.4",
                    style: "fill: #dce8f2;",
                }
            }
            div { class: "profile-caption", "{readout}" }
            div { class: "profile-controls",
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: pressure_on,
                        onchange: move |_| show_pressure.set(!pressure_on),
                    }
                    " Pressure"
                }
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: temp_on,
                        onchange: move |_| show_temp.set(!temp_on),
                    }
                    " Temperature"
                }
                input {
                    class: "scrub",
                    r#type: "range",
                    min: "0",
                    max: "{samples.len() - 1}",
                    value: "{index}",
                    oninput: move |evt| cursor.set(evt.value().parse().unwrap_or(0)),
                }
            }
        }
    }
}

/// Normalize a `(seconds, value)` series into SVG points, inverting the value
/// axis so larger values sit higher on the chart.
fn series(values: impl Iterator<Item = (i32, i32)>, max_t: f64, _invert: bool) -> Option<String> {
    let values: Vec<(i32, i32)> = values.collect();
    if values.len() < 2 {
        return None;
    }
    let min = values.iter().map(|(_, v)| *v).min()?;
    let max = values.iter().map(|(_, v)| *v).max()?;
    let range = (max - min).max(1) as f64;
    Some(
        values
            .iter()
            .map(|(t, v)| {
                let x = *t as f64 / max_t * 100.0;
                let y = 92.0 - (*v - min) as f64 / range * 84.0;
                format!("{x:.2},{y:.2}")
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}
