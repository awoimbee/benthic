use dioxus::prelude::*;

use benthic_core::units::format_duration;
use benthic_core::Dive;

use crate::state::AppState;

/// An interactive SVG depth profile with overlays, event markers, a scrubber
/// readout, pan/zoom and (when a dive has several computers) the other
/// computers' traces.
#[component]
pub fn DiveProfile(dive: Dive, dc_index: usize) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();

    let mut show_pressure = use_signal(|| true);
    let mut show_temp = use_signal(|| false);
    let mut show_ndl = use_signal(|| false);
    let mut show_tts = use_signal(|| false);
    let mut show_heart = use_signal(|| false);
    let mut show_cns = use_signal(|| false);
    let mut show_deco = use_signal(|| true);
    let mut cursor = use_signal(|| 0usize);
    let mut zoom = use_signal(|| 1.0f64);
    let mut pan = use_signal(|| 0.5f64);
    let mut hover = use_signal(|| None::<f64>);
    let mut plot_width = use_signal(|| 0.0f64);

    let Some(active) = dive.computer(dc_index).or_else(|| dive.computers.first()) else {
        return rsx! { div { class: "profile empty-hint", "No profile data for this dive." } };
    };

    if active.samples.len() < 2 {
        return rsx! { div { class: "profile empty-hint", "No profile data for this dive." } };
    }

    // A common frame so multiple computers can be compared directly.
    let max_t = dive
        .computers
        .iter()
        .filter_map(|dc| dc.samples.last())
        .map(|s| s.time.seconds)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let max_d = dive
        .computers
        .iter()
        .flat_map(|dc| dc.samples.iter())
        .map(|s| s.depth.mm)
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    // Visible time window as fractions of the full profile.
    let zoom_value = (zoom)().max(1.0);
    let half = 0.5 / zoom_value;
    let pan_value = (pan)();
    let center = pan_value.clamp(half, 1.0 - half);
    let win_start = (center - half).clamp(0.0, 1.0 - 1.0 / zoom_value);
    let win_end = win_start + 1.0 / zoom_value;
    let win_span = (win_end - win_start).max(1e-9);

    let depth_points = depth_series(&active.samples, max_t, max_d, win_start, win_end);
    let overlays: Vec<String> = dive
        .computers
        .iter()
        .enumerate()
        .filter(|(index, dc)| *index != dc_index && dc.samples.len() >= 2)
        .map(|(_, dc)| depth_series(&dc.samples, max_t, max_d, win_start, win_end))
        .collect();

    let temperature = series(
        active
            .samples
            .iter()
            .filter_map(|s| s.temperature.map(|t| (s.time.seconds, t.mkelvin as i32))),
        max_t,
        win_start,
        win_end,
    );
    let pressure = series(
        active.samples.iter().filter_map(|s| {
            s.pressures
                .iter()
                .find(|p| p.sensor == 0)
                .or_else(|| s.pressures.first())
                .map(|p| (s.time.seconds, p.pressure.mbar))
        }),
        max_t,
        win_start,
        win_end,
    );
    let ndl = series(
        active.samples.iter().filter_map(|s| {
            s.ndl
                .filter(|d| d.seconds >= 0)
                .map(|d| (s.time.seconds, d.seconds))
        }),
        max_t,
        win_start,
        win_end,
    );
    let tts = series(
        active.samples.iter().filter_map(|s| {
            s.tts
                .filter(|d| d.seconds > 0)
                .map(|d| (s.time.seconds, d.seconds))
        }),
        max_t,
        win_start,
        win_end,
    );
    let heart = series(
        active
            .samples
            .iter()
            .filter_map(|s| s.heartbeat.map(|h| (s.time.seconds, h as i32))),
        max_t,
        win_start,
        win_end,
    );
    let cns = series(
        active
            .samples
            .iter()
            .filter_map(|s| s.cns.map(|c| (s.time.seconds, c as i32))),
        max_t,
        win_start,
        win_end,
    );
    let ceiling = series(
        active.samples.iter().filter_map(|s| {
            s.stop_depth
                .filter(|d| d.mm > 0)
                .map(|d| (s.time.seconds, d.mm))
        }),
        max_t,
        win_start,
        win_end,
    );

    // Event markers within the window, flagging gas switches.
    let event_marks: Vec<(f64, String, bool)> = active
        .events
        .iter()
        .filter(|e| e.time.seconds >= 0)
        .filter_map(|e| {
            let fraction = e.time.seconds as f64 / max_t;
            if fraction < win_start || fraction > win_end {
                return None;
            }
            let name = if e.name.is_empty() {
                "event".to_string()
            } else {
                e.name.clone()
            };
            Some((
                (fraction - win_start) / win_span * 100.0,
                name,
                e.is_gas_change(),
            ))
        })
        .collect();

    // The active point is the hovered sample when the pointer is over the
    // chart, otherwise the scrubber position.
    let hover_fraction = (hover)();
    let index = match hover_fraction {
        Some(fraction) => sample_index_at(win_start + fraction * win_span, &active.samples, max_t),
        None => (cursor)().min(active.samples.len() - 1),
    };
    let sample = &active.samples[index];
    let cursor_fraction = sample.time.seconds as f64 / max_t;
    let cursor_x = (cursor_fraction - win_start) / win_span * 100.0;
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
    if let Some(ndl) = sample.ndl.filter(|d| d.seconds >= 0) {
        readout.push(format!("NDL {}", format_duration(ndl)));
    }
    if let Some(tts) = sample.tts.filter(|d| d.seconds > 0) {
        readout.push(format!("TTS {}", format_duration(tts)));
    }
    if let Some(heart) = sample.heartbeat {
        readout.push(format!("{heart} bpm"));
    }
    if let Some(cns) = sample.cns {
        readout.push(format!("CNS {cns}%"));
    }
    if let Some(ceiling) = sample.stop_depth.filter(|d| d.mm > 0) {
        readout.push(format!("Ceiling {}", prefs.depth(ceiling)));
    }
    let readout = readout.join("  ·  ");

    let pressure_on = (show_pressure)();
    let temp_on = (show_temp)();
    let ndl_on = (show_ndl)();
    let tts_on = (show_tts)();
    let heart_on = (show_heart)();
    let cns_on = (show_cns)();
    let deco_on = (show_deco)();

    rsx! {
        div { class: "profile",
            svg {
                class: "profile-svg",
                view_box: "0 0 100 100",
                preserve_aspect_ratio: "none",
                onmounted: move |evt: MountedEvent| async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        plot_width.set(rect.size.width.max(1.0));
                    }
                },
                onmousemove: move |evt: MouseEvent| {
                    let width = (plot_width)();
                    if width > 1.0 {
                        let fraction = (evt.element_coordinates().x / width).clamp(0.0, 1.0);
                        hover.set(Some(fraction));
                    }
                },
                onmouseleave: move |_| hover.set(None),
                for (n, points) in overlays.iter().enumerate() {
                    polyline {
                        key: "overlay-{n}",
                        points: "{points}",
                        style: "fill: none; stroke: #7f97ad; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.5;",
                    }
                }
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
                if ndl_on {
                    if let Some(points) = &ndl {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #a8dadc; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8; stroke-dasharray: 3 2;",
                        }
                    }
                }
                if tts_on {
                    if let Some(points) = &tts {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #f28fad; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                        }
                    }
                }
                if heart_on {
                    if let Some(points) = &heart {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #d0ffb7; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                        }
                    }
                }
                if cns_on {
                    if let Some(points) = &cns {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #ffd166; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                        }
                    }
                }
                polyline {
                    points: "{depth_points}",
                    style: "fill: none; stroke: #4cc9f0; stroke-width: 1.5; vector-effect: non-scaling-stroke;",
                }
                if deco_on {
                    if let Some(points) = &ceiling {
                        polyline {
                            points: "{points}",
                            style: "fill: none; stroke: #c792ea; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.9;",
                        }
                    }
                }
                for (x, name, gas) in event_marks {
                    line {
                        key: "{name}-{x}",
                        x1: "{x}",
                        y1: "4",
                        x2: "{x}",
                        y2: "10",
                        style: if gas {
                            "stroke: #f28fad; stroke-width: 1.4; vector-effect: non-scaling-stroke;"
                        } else {
                            "stroke: #c792ea; stroke-width: 1; vector-effect: non-scaling-stroke;"
                        },
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
                Toggle { label: "Pressure", on: pressure_on, onclick: move |_| show_pressure.set(!pressure_on) }
                Toggle { label: "Temperature", on: temp_on, onclick: move |_| show_temp.set(!temp_on) }
                Toggle { label: "NDL", on: ndl_on, onclick: move |_| show_ndl.set(!ndl_on) }
                Toggle { label: "TTS", on: tts_on, onclick: move |_| show_tts.set(!tts_on) }
                Toggle { label: "Heart", on: heart_on, onclick: move |_| show_heart.set(!heart_on) }
                Toggle { label: "CNS", on: cns_on, onclick: move |_| show_cns.set(!cns_on) }
                Toggle { label: "Deco", on: deco_on, onclick: move |_| show_deco.set(!deco_on) }
                label { class: "check", "Zoom"
                    input {
                        class: "zoom",
                        r#type: "range",
                        min: "1",
                        max: "20",
                        step: "0.5",
                        value: "{zoom_value}",
                        oninput: move |evt| zoom.set(evt.value().parse().unwrap_or(1.0)),
                    }
                }
                label { class: "check", "Pan"
                    input {
                        class: "zoom",
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.01",
                        value: "{pan_value}",
                        oninput: move |evt| pan.set(evt.value().parse().unwrap_or(0.5)),
                    }
                }
                input {
                    class: "scrub",
                    r#type: "range",
                    min: "0",
                    max: "{active.samples.len() - 1}",
                    value: "{index}",
                    oninput: move |evt| cursor.set(evt.value().parse().unwrap_or(0)),
                }
            }
        }
    }
}

#[component]
fn Toggle(label: &'static str, on: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        label { class: "check",
            input { r#type: "checkbox", checked: on, onchange: move |_| onclick.call(()) }
            " {label}"
        }
    }
}

/// The index of the sample whose time is closest to `fraction` of the full
/// profile. Samples are assumed to be ordered by time.
fn sample_index_at(fraction: f64, samples: &[benthic_core::Sample], max_t: f64) -> usize {
    if samples.is_empty() {
        return 0;
    }
    let target = fraction * max_t;
    let index = samples.partition_point(|s| (s.time.seconds as f64) < target);
    if index == 0 {
        return 0;
    }
    if index >= samples.len() {
        return samples.len() - 1;
    }
    let before = samples[index - 1].time.seconds as f64;
    let after = samples[index].time.seconds as f64;
    if (target - before).abs() <= (after - target).abs() {
        index - 1
    } else {
        index
    }
}

fn depth_series(
    samples: &[benthic_core::Sample],
    max_t: f64,
    max_d: f64,
    start: f64,
    end: f64,
) -> String {
    let span = (end - start).max(1e-9);
    samples
        .iter()
        .filter_map(|s| {
            let fraction = s.time.seconds as f64 / max_t;
            if fraction < start || fraction > end {
                return None;
            }
            let x = (fraction - start) / span * 100.0;
            let y = s.depth.mm as f64 / max_d * 90.0 + 5.0;
            Some(format!("{x:.2},{y:.2}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize a `(seconds, value)` series into SVG points within the visible
/// window, inverting the value axis so larger values sit higher on the chart.
fn series(
    values: impl Iterator<Item = (i32, i32)>,
    max_t: f64,
    start: f64,
    end: f64,
) -> Option<String> {
    let span = (end - start).max(1e-9);
    let visible: Vec<(f64, i32)> = values
        .filter_map(|(t, v)| {
            let fraction = t as f64 / max_t;
            if fraction < start || fraction > end {
                None
            } else {
                Some((fraction, v))
            }
        })
        .collect();
    if visible.len() < 2 {
        return None;
    }
    let min = visible.iter().map(|(_, v)| *v).min()?;
    let max = visible.iter().map(|(_, v)| *v).max()?;
    let range = (max - min).max(1) as f64;
    Some(
        visible
            .iter()
            .map(|(fraction, v)| {
                let x = (fraction - start) / span * 100.0;
                let y = 92.0 - (*v - min) as f64 / range * 84.0;
                format!("{x:.2},{y:.2}")
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}
