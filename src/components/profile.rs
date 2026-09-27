use dioxus::prelude::*;

use benthic_core::units::{format_duration, Depth, Duration, Pressure, Temperature};
use benthic_core::{Dive, DiveComputer, Preferences};

use crate::state::AppState;

const COLOR_DEPTH: &str = "var(--trace-depth)";
const COLOR_PRESSURE: &str = "var(--trace-pressure)";
const COLOR_TEMP: &str = "var(--trace-temp)";
const COLOR_NDL: &str = "var(--trace-ndl)";
const COLOR_TTS: &str = "var(--trace-tts)";
const COLOR_HEART: &str = "var(--trace-heart)";
const COLOR_CNS: &str = "var(--trace-cns)";
const COLOR_CEILING: &str = "var(--trace-ceiling)";

/// An interactive SVG depth profile with overlays, event markers, a scrubber
/// readout, pan/zoom and (when a dive has several computers) the other
/// computers' traces.
///
/// The plot keeps a vertical scale for every displayed metric: depth on the
/// left and one colour-coded scale per overlay on the right.
#[component]
pub fn DiveProfile(dive: Dive, dc_index: usize) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();

    // Which metrics this dive actually carries, so toggles can be disabled
    // when there is nothing to show. Computed before the hooks (which must run
    // unconditionally) and over the whole dive, not just the visible window.
    let active_opt = dive.computer(dc_index).or_else(|| dive.computers.first());
    let available = active_opt.map(metric_availability).unwrap_or_default();
    let has_pressure = available.pressure;
    let has_deco = available.ceiling;

    let mut show_pressure = use_signal(|| has_pressure);
    let mut show_temp = use_signal(|| false);
    let mut show_ndl = use_signal(|| false);
    let mut show_tts = use_signal(|| false);
    let mut show_heart = use_signal(|| false);
    let mut show_cns = use_signal(|| false);
    let mut show_deco = use_signal(|| has_deco);
    let mut cursor = use_signal(|| 0usize);
    let mut zoom = use_signal(|| 1.0f64);
    let mut pan = use_signal(|| 0.5f64);
    let mut hover = use_signal(|| None::<f64>);
    let mut plot_width = use_signal(|| 0.0f64);

    let Some(active) = active_opt else {
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

    // The depth trace is split into one path per vertical-speed band, so its
    // colour shows how fast the diver was descending or ascending. The ceiling
    // is a depth too and must line up with the depth scale.
    let depth_bands = speed_bands(&active.samples, max_d, max_t, win_start, win_end);
    let ceiling = scaled_series(
        active.samples.iter().filter_map(|s| {
            s.stop_depth
                .filter(|d| d.mm > 0)
                .map(|d| (s.time.seconds, d.mm))
        }),
        max_d,
        max_t,
        win_start,
        win_end,
    );
    let overlays: Vec<String> = dive
        .computers
        .iter()
        .enumerate()
        .filter(|(index, dc)| *index != dc_index && dc.samples.len() >= 2)
        .map(|(_, dc)| {
            scaled_series(
                dc.samples.iter().map(|s| (s.time.seconds, s.depth.mm)),
                max_d,
                max_t,
                win_start,
                win_end,
            )
        })
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

    let pressure_on = (show_pressure)();
    let temp_on = (show_temp)();
    let ndl_on = (show_ndl)();
    let tts_on = (show_tts)();
    let heart_on = (show_heart)();
    let cns_on = (show_cns)();
    let deco_on = (show_deco)();

    // Vertical scales: depth on the left, one column per active overlay on the
    // right. Each tick is `(y percent, style, label)`.
    let depth_ticks = depth_axis(max_d, &prefs);
    let depth_name_style = format!("color: {COLOR_DEPTH};");

    let mut right_axes: Vec<(&'static str, String, Vec<Tick>)> = Vec::new();
    if pressure_on {
        if let Some(Series { min, max, .. }) = &pressure {
            right_axes.push((
                "Pressure",
                format!("color: {COLOR_PRESSURE};"),
                value_axis(*min, *max, COLOR_PRESSURE, |v| {
                    prefs.pressure(Pressure::new(v))
                }),
            ));
        }
    }
    if temp_on {
        if let Some(Series { min, max, .. }) = &temperature {
            right_axes.push((
                "Temp",
                format!("color: {COLOR_TEMP};"),
                value_axis(*min, *max, COLOR_TEMP, |v| {
                    prefs.temperature(Temperature::new(v as u32))
                }),
            ));
        }
    }
    if ndl_on {
        if let Some(Series { min, max, .. }) = &ndl {
            right_axes.push((
                "NDL",
                format!("color: {COLOR_NDL};"),
                value_axis(*min, *max, COLOR_NDL, |v| format_duration(Duration::new(v))),
            ));
        }
    }
    if tts_on {
        if let Some(Series { min, max, .. }) = &tts {
            right_axes.push((
                "TTS",
                format!("color: {COLOR_TTS};"),
                value_axis(*min, *max, COLOR_TTS, |v| format_duration(Duration::new(v))),
            ));
        }
    }
    if heart_on {
        if let Some(Series { min, max, .. }) = &heart {
            right_axes.push((
                "Heart",
                format!("color: {COLOR_HEART};"),
                value_axis(*min, *max, COLOR_HEART, |v| format!("{v} bpm")),
            ));
        }
    }
    if cns_on {
        if let Some(Series { min, max, .. }) = &cns {
            right_axes.push((
                "CNS",
                format!("color: {COLOR_CNS};"),
                value_axis(*min, *max, COLOR_CNS, |v| format!("{v}%")),
            ));
        }
    }
    if deco_on && !ceiling.is_empty() {
        right_axes.push((
            "Ceiling",
            format!("color: {COLOR_CEILING};"),
            depth_ticks.clone(),
        ));
    }

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

    rsx! {
        div { class: "profile",
            div { class: "profile-plot",
                div { class: "axis-rail axis-left",
                    span { class: "axis-name", style: "{depth_name_style}", "Depth" }
                    for (_, style, label) in depth_ticks.iter() {
                        span { key: "depth-{label}", class: "axis-label", style: "{style}", "{label}" }
                    }
                }
                svg {
                    class: "profile-svg",
                    view_box: "0 0 100 100",
                    preserve_aspect_ratio: "none",
                    onmounted: move |evt: MountedEvent| async move {
                        if let Ok(rect) = evt.get_client_rect().await {
                            plot_width.set(rect.size.width.max(1.0));
                        }
                    },
                    // Pointer events cover mouse, touch and pen. Touching a
                    // point shows the same readout as hovering; unlike the
                    // mouse, a touch does not clear it on release so the
                    // values stay readable.
                    onpointerdown: move |evt: PointerEvent| {
                        let width = (plot_width)();
                        if width > 1.0 {
                            let fraction = (evt.element_coordinates().x / width).clamp(0.0, 1.0);
                            hover.set(Some(fraction));
                        }
                    },
                    onpointermove: move |evt: PointerEvent| {
                        let width = (plot_width)();
                        if width > 1.0 {
                            let fraction = (evt.element_coordinates().x / width).clamp(0.0, 1.0);
                            hover.set(Some(fraction));
                        }
                    },
                    onpointerup: move |evt: PointerEvent| {
                        if evt.pointer_type() == "mouse" {
                            hover.set(None);
                        }
                    },
                    onpointercancel: move |_| hover.set(None),
                    onpointerleave: move |evt: PointerEvent| {
                        if evt.pointer_type() == "mouse" {
                            hover.set(None);
                        }
                    },
                    for (y, _, _) in depth_ticks.iter() {
                        line {
                            key: "grid-{y}",
                            class: "profile-gridline",
                            x1: "0",
                            y1: "{y}",
                            x2: "100",
                            y2: "{y}",
                        }
                    }
                    for (n, points) in overlays.iter().enumerate() {
                        polyline {
                            key: "overlay-{n}",
                            points: "{points}",
                            style: "fill: none; stroke: #7f97ad; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.5;",
                        }
                    }
                    if pressure_on {
                        if let Some(Series { points, .. }) = &pressure {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_PRESSURE}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                            }
                        }
                    }
                    if temp_on {
                        if let Some(Series { points, .. }) = &temperature {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_TEMP}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                            }
                        }
                    }
                    if ndl_on {
                        if let Some(Series { points, .. }) = &ndl {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_NDL}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8; stroke-dasharray: 3 2;",
                            }
                        }
                    }
                    if tts_on {
                        if let Some(Series { points, .. }) = &tts {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_TTS}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                            }
                        }
                    }
                    if heart_on {
                        if let Some(Series { points, .. }) = &heart {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_HEART}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                            }
                        }
                    }
                    if cns_on {
                        if let Some(Series { points, .. }) = &cns {
                            polyline {
                                points: "{points}",
                                style: "fill: none; stroke: {COLOR_CNS}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.8;",
                            }
                        }
                    }
                    for (label, color, d) in depth_bands.iter() {
                        path {
                            key: "depth-{label}",
                            d: "{d}",
                            style: "fill: none; stroke: {color}; stroke-width: 1.5; vector-effect: non-scaling-stroke;",
                        }
                    }
                    if deco_on && !ceiling.is_empty() {
                        polyline {
                            points: "{ceiling}",
                            style: "fill: none; stroke: {COLOR_CEILING}; stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.9;",
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
                                "stroke: var(--trace-tts); stroke-width: 1.4; vector-effect: non-scaling-stroke;"
                            } else {
                                "stroke: var(--trace-ceiling); stroke-width: 1; vector-effect: non-scaling-stroke;"
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
                div { class: "axis-rail axis-right",
                    for (i, (name, name_style, ticks)) in right_axes.iter().enumerate() {
                        div { key: "axis-{i}", class: "axis-scale",
                            span { class: "axis-name", style: "{name_style}", "{name}" }
                            for (j, (_, style, label)) in ticks.iter().enumerate() {
                                span {
                                    key: "tick-{i}-{j}",
                                    class: "axis-label",
                                    style: "{style}",
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            div { class: "profile-legend",
                span { class: "legend-item", "Depth speed" }
                for (label, color, _) in SPEED_BANDS.iter() {
                    span { key: "legend-{label}", class: "legend-item",
                        span { class: "legend-swatch", style: "background: {color};" }
                        " {label}"
                    }
                }
            }
            div { class: "profile-caption", "{readout}" }
            div { class: "profile-controls",
                Toggle { label: "Pressure", color: COLOR_PRESSURE, on: pressure_on, disabled: !available.pressure, title: "Cylinder pressure during the dive", onclick: move |_| show_pressure.set(!pressure_on) }
                Toggle { label: "Temperature", color: COLOR_TEMP, on: temp_on, disabled: !available.temperature, title: "Water temperature during the dive", onclick: move |_| show_temp.set(!temp_on) }
                Toggle { label: "NDL", color: COLOR_NDL, on: ndl_on, disabled: !available.ndl, title: "No-decompression limit: how much longer you can stay at this depth without requiring a stop", onclick: move |_| show_ndl.set(!ndl_on) }
                Toggle { label: "TTS", color: COLOR_TTS, on: tts_on, disabled: !available.tts, title: "Time to surface: estimated ascent time including any decompression stops", onclick: move |_| show_tts.set(!tts_on) }
                Toggle { label: "Heart", color: COLOR_HEART, on: heart_on, disabled: !available.heart, title: "Heart rate in beats per minute", onclick: move |_| show_heart.set(!heart_on) }
                Toggle { label: "CNS", color: COLOR_CNS, on: cns_on, disabled: !available.cns, title: "Central nervous system oxygen toxicity, as a share of the NOAA limit", onclick: move |_| show_cns.set(!cns_on) }
                Toggle { label: "Deco", color: COLOR_CEILING, on: deco_on, disabled: !available.ceiling, title: "Decompression ceiling: the shallowest depth you may ascend to", onclick: move |_| show_deco.set(!deco_on) }
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
fn Toggle(
    label: &'static str,
    color: &'static str,
    title: &'static str,
    on: bool,
    disabled: bool,
    onclick: EventHandler<()>,
) -> Element {
    let name_style = format!("color: {color};");
    let box_style = format!("accent-color: {color};");
    let class = if disabled { "check disabled" } else { "check" };
    rsx! {
        label { class: "{class}", title: "{title}",
            input {
                r#type: "checkbox",
                checked: on,
                disabled,
                style: "{box_style}",
                onchange: move |_| onclick.call(()),
            }
            span { style: "{name_style}", " {label}" }
        }
    }
}

/// Which optional metrics a dive computer actually recorded.
#[derive(Clone, Copy, Default)]
struct Availability {
    pressure: bool,
    temperature: bool,
    ndl: bool,
    tts: bool,
    heart: bool,
    cns: bool,
    ceiling: bool,
}

/// Scan a computer's samples for the metrics that have data, so the matching
/// toggles can be enabled.
fn metric_availability(dc: &DiveComputer) -> Availability {
    let samples = &dc.samples;
    Availability {
        pressure: samples.iter().any(|s| !s.pressures.is_empty()),
        temperature: samples.iter().any(|s| s.temperature.is_some()),
        ndl: samples
            .iter()
            .any(|s| s.ndl.is_some_and(|d| d.seconds >= 0)),
        tts: samples.iter().any(|s| s.tts.is_some_and(|d| d.seconds > 0)),
        heart: samples.iter().any(|s| s.heartbeat.is_some()),
        cns: samples.iter().any(|s| s.cns.is_some()),
        ceiling: samples
            .iter()
            .any(|s| s.stop_depth.is_some_and(|d| d.mm > 0)),
    }
}

/// A normalized overlay series: the SVG point string plus the value range, so
/// the same range can be drawn as a y-axis scale.
struct Series {
    points: String,
    min: i32,
    max: i32,
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

/// Map `(seconds, value)` pairs onto the chart using a fixed value scale, from
/// `5%` at the surface to `95%` at `max_v`. Used for anything measured in
/// depth so it lines up with the depth trace.
fn scaled_series(
    values: impl Iterator<Item = (i32, i32)>,
    max_v: f64,
    max_t: f64,
    start: f64,
    end: f64,
) -> String {
    let span = (end - start).max(1e-9);
    values
        .filter_map(|(t, v)| {
            let fraction = t as f64 / max_t;
            if fraction < start || fraction > end {
                return None;
            }
            let x = (fraction - start) / span * 100.0;
            let y = v as f64 / max_v.max(1e-9) * 90.0 + 5.0;
            Some(format!("{x:.2},{y:.2}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Vertical-speed bands used to colour the depth trace, fastest first. `min`
/// is the lower bound on the *magnitude* of the speed in metres per minute,
/// so descents and ascents share one three-colour scale.
const SPEED_BANDS: [(&str, &str, f64); 3] = [
    ("Too fast", "var(--speed-toofast)", 20.0),
    ("A bit fast", "var(--speed-fast)", 10.0),
    ("Normal", COLOR_DEPTH, 0.0),
];

/// The band the magnitude of a vertical speed (m/min) falls into.
fn speed_band(speed: f64) -> usize {
    let speed = speed.abs();
    SPEED_BANDS
        .iter()
        .position(|(_, _, min)| speed >= *min)
        .unwrap_or(SPEED_BANDS.len() - 1)
}

/// Split the depth trace into segments and group them by the vertical speed
/// over each segment, returning `(label, colour, path data)` per non-empty
/// band. Paths are disjoint subpaths (`M … L …`), so one `<path>` per band
/// draws the whole trace in that band's colour.
fn speed_bands(
    samples: &[benthic_core::Sample],
    max_d: f64,
    max_t: f64,
    start: f64,
    end: f64,
) -> Vec<(&'static str, &'static str, String)> {
    let span = (end - start).max(1e-9);
    let mut bands: Vec<(&'static str, &'static str, String)> = SPEED_BANDS
        .iter()
        .map(|(label, color, _)| (*label, *color, String::new()))
        .collect();
    let mut previous: Option<(f64, f64, i32, f64)> = None;
    for sample in samples {
        let fraction = sample.time.seconds as f64 / max_t;
        if fraction < start || fraction > end {
            continue;
        }
        let x = (fraction - start) / span * 100.0;
        let y = sample.depth.mm as f64 / max_d.max(1e-9) * 90.0 + 5.0;
        if let Some((px, py, depth, time)) = previous {
            let minutes = (sample.time.seconds as f64 - time) / 60.0;
            let speed = if minutes > 0.0 {
                (sample.depth.mm - depth) as f64 / 1000.0 / minutes
            } else {
                0.0
            };
            bands[speed_band(speed)]
                .2
                .push_str(&format!("M {px:.2} {py:.2} L {x:.2} {y:.2} "));
        }
        previous = Some((x, y, sample.depth.mm, sample.time.seconds as f64));
    }
    bands.retain(|(_, _, d)| !d.is_empty());
    bands
}

/// Normalize a `(seconds, value)` series to its own range, inverting the value
/// axis so larger values sit higher on the chart.
fn series(
    values: impl Iterator<Item = (i32, i32)>,
    max_t: f64,
    start: f64,
    end: f64,
) -> Option<Series> {
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
    let value_span = (max - min) as f64;
    let points = visible
        .iter()
        .map(|(fraction, v)| {
            let x = (fraction - start) / span * 100.0;
            // A constant series has no range of its own; centre it so it lines
            // up with the single tick its scale draws.
            let y = if value_span <= 0.0 {
                50.0
            } else {
                92.0 - (*v - min) as f64 / value_span * 84.0
            };
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(Series { points, min, max })
}

/// A tick is `(y percent, inline style, label)`.
type Tick = (f64, String, String);

/// Evenly rounded depth ticks for the left axis, in the active unit.
fn depth_axis(max_d: f64, prefs: &Preferences) -> Vec<Tick> {
    let max_display = prefs.depth_value(Depth::new(max_d as i32));
    if !max_display.is_finite() || max_display <= 0.0 {
        return Vec::new();
    }
    let step = nice_step(max_display);
    let mut ticks = Vec::new();
    let mut value = 0.0;
    while value <= max_display + 1e-6 {
        let y = value / max_display * 90.0 + 5.0;
        ticks.push((
            y,
            format!("top: {y:.2}%; color: {COLOR_DEPTH};"),
            format!("{value:.0} {}", prefs.depth_unit()),
        ));
        value += step;
    }
    ticks
}

/// Three ticks (max, middle, min) for a normalized overlay, coloured to match
/// its trace.
fn value_axis(min: i32, max: i32, color: &str, fmt: impl Fn(i32) -> String) -> Vec<Tick> {
    if min == max {
        return vec![(50.0, format!("top: 50%; color: {color};"), fmt(min))];
    }
    let range = (max - min) as f64;
    let middle = min + (max - min) / 2;
    [max, middle, min]
        .into_iter()
        .map(|value| {
            let y = 92.0 - (value - min) as f64 / range * 84.0;
            (y, format!("top: {y:.2}%; color: {color};"), fmt(value))
        })
        .collect()
}

/// A round step that yields roughly five intervals over `max`.
fn nice_step(max: f64) -> f64 {
    let ideal = max / 5.0;
    const STEPS: [f64; 12] = [
        1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 25.0, 50.0, 100.0, 200.0, 500.0,
    ];
    STEPS
        .into_iter()
        .find(|step| *step >= ideal)
        .unwrap_or(500.0)
}
