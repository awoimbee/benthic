use dioxus::prelude::*;

use benthic_core::units::{format_duration, Depth, Duration, Pressure, Temperature};
use benthic_core::{Dive, DiveComputer, Event, Preferences};

use crate::i18n;
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
///
/// `planned` is for generated profiles: the depth trace is a single colour
/// (ascent speed is fixed by the plan) and unavailable metrics are hidden.
#[component]
pub fn DiveProfile(
    dive: Dive,
    dc_index: usize,
    #[props(default)] show_all: bool,
    #[props(default)] pressure_colors: Vec<String>,
    #[props(default)] on_add_event: Option<EventHandler<(usize, Duration)>>,
    #[props(default)] on_remove_event: Option<EventHandler<(usize, usize)>>,
    #[props(default)] planned: bool,
) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    // Which metrics this dive actually carries, so toggles can be disabled
    // when there is nothing to show. Computed before the hooks (which must run
    // unconditionally) and over the whole dive, not just the visible window.
    let active_opt = dive.computer(dc_index).or_else(|| dive.computers.first());
    let available = active_opt.map(metric_availability).unwrap_or_default();
    let has_pressure = available.pressure;

    let mut overlays_on = use_signal(|| Overlays {
        pressure: has_pressure,
        ndl: show_all && available.ndl,
        tts: show_all && available.tts,
        ..Default::default()
    });
    let mut cursor = use_signal(|| 0usize);
    let mut zoom = use_signal(|| 1.0f64);
    let mut pan = use_signal(|| 0.5f64);
    let mut hover = use_signal(|| None::<f64>);
    let mut drag = use_signal(|| None::<(f64, f64)>);
    let mut plot_width = use_signal(|| 0.0f64);
    let mut plot_left = use_signal(|| 0.0f64);
    let mut touches = use_signal(Vec::<(i32, f64, f64)>::new);
    let mut touch_gesture = use_signal(|| None::<TouchGesture>);
    let mut selected_event = use_signal(|| None::<usize>);
    let Some(active) = active_opt else {
        return rsx! { div { class: "profile empty-hint", "{tr.no_profile}" } };
    };

    if active.samples.len() < 2 {
        return rsx! { div { class: "profile empty-hint", "{tr.no_profile}" } };
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

    // Drag-to-select zoom, Grafana style. `selection` is `(x, width, shown)`
    // in plot percentages.
    let (sel_x, sel_w, sel_on) = (drag)()
        .map(|(a, b)| {
            let (lo, hi) = (a.min(b), a.max(b));
            (lo * 100.0, (hi - lo) * 100.0, hi - lo > MIN_SELECTION)
        })
        .unwrap_or((0.0, 0.0, false));

    // The depth trace is split into one path per vertical-speed band, so its
    // colour shows how fast the diver was descending or ascending.
    let depth_bands = if planned {
        single_band(&active.samples, max_d, max_t, win_start, win_end)
    } else {
        speed_bands(&active.samples, max_d, max_t, win_start, win_end)
    };
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
    // Pressure always uses a zero baseline, so 0 bar lines up with the same
    // bottom edge as 0 NDL and max depth.
    let pressure_values: Vec<i32> = active
        .samples
        .iter()
        .flat_map(|s| s.pressures.iter().map(|p| p.pressure.mbar))
        .collect();
    let pressure_max = pressure_values.iter().copied().max().unwrap_or(0);
    let pressure = (pressure_max > 0).then(|| Series {
        points: String::new(),
        min: 0,
        max: pressure_max,
    });
    let pressure_sensors: Vec<i16> = {
        let mut sensors: Vec<i16> = active
            .samples
            .iter()
            .flat_map(|s| s.pressures.iter().map(|p| p.sensor))
            .collect();
        sensors.sort_unstable();
        sensors.dedup();
        sensors
    };
    let pressure_lines: Vec<(String, Option<String>)> = pressure
        .as_ref()
        .map(|Series { min, max, .. }| {
            pressure_sensors
                .iter()
                .filter_map(|sensor| {
                    let points = series_points(
                        active.samples.iter().filter_map(|sample| {
                            sample
                                .pressures
                                .iter()
                                .find(|pressure| pressure.sensor == *sensor)
                                .map(|pressure| (sample.time.seconds, pressure.pressure.mbar))
                        }),
                        *min,
                        *max,
                        max_t,
                        win_start,
                        win_end,
                    )?;
                    Some((points, pressure_colors.get(*sensor as usize).cloned()))
                })
                .collect()
        })
        .unwrap_or_default();
    // The NDL scale starts at one minute so a zero NDL (in decompression)
    // falls below the plot instead of lying along its bottom edge.
    let ndl = clipped_series(
        active.samples.iter().filter_map(|s| {
            s.ndl
                .filter(|d| d.seconds >= 0)
                .map(|d| (s.time.seconds, d.seconds))
        }),
        60,
        90 * 60,
        max_t,
        win_start,
        win_end,
    );
    let tts = series_fixed(
        active.samples.iter().filter_map(|s| {
            s.tts
                .filter(|d| d.seconds > 0)
                .map(|d| (s.time.seconds, d.seconds))
        }),
        0,
        active
            .samples
            .iter()
            .filter_map(|s| s.tts.map(|d| d.seconds))
            .max()
            .unwrap_or(0),
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
    let Overlays {
        pressure: pressure_on,
        temp: temp_on,
        ndl: ndl_on,
        tts: tts_on,
        heart: heart_on,
        cns: cns_on,
    } = overlays_on();
    // Subsurface-style water column: a depth gradient under the trace whose
    // top edge drops to the decompression ceiling.
    let (depth_area, ceiling_zone) = depth_area(&active.samples, max_d, max_t, win_start, win_end);

    // Vertical scales: depth on the left, one column per active overlay on the
    // right. Each tick is `(y percent, style, label)`.
    let depth_ticks = depth_axis(max_d, &prefs);
    let depth_name_style = format!("color: {COLOR_DEPTH};");

    let mut right_axes: Vec<(&'static str, String, Vec<Tick>)> = Vec::new();
    if pressure_on {
        if let Some(Series { min, max, .. }) = &pressure {
            right_axes.push((
                tr.pressure,
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
                value_axis_ticks(
                    *min,
                    *max,
                    &[*max, (*min + *max) / 2 / 60 * 60, *min],
                    COLOR_NDL,
                    |v| format_duration(Duration::new(v)),
                ),
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
                tr.heart,
                format!("color: {COLOR_HEART};"),
                value_axis(*min, *max, COLOR_HEART, |v| format!("{v} bpm")),
            ));
        }
    }
    if cns_on {
        if let Some(Series { min, max, .. }) = &cns {
            right_axes.push((
                tr.cns,
                format!("color: {COLOR_CNS};"),
                value_axis(*min, *max, COLOR_CNS, |v| format!("{v}%")),
            ));
        }
    }

    // Event markers within the window. The icon/colour identifies the event
    // type so gas switches and bookmarks are not just anonymous ticks.
    let last_sample_seconds = active
        .samples
        .last()
        .map(|sample| sample.time.seconds)
        .unwrap_or(0);
    let event_marks: Vec<EventMark> = active
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.time.seconds >= 0)
        .filter(|(_, event)| !boundary_gas_switch(event, last_sample_seconds))
        .filter_map(|(event_index, event)| {
            let fraction = event.time.seconds as f64 / max_t;
            if fraction < win_start || fraction > win_end {
                return None;
            }
            let kind = event_kind(event);
            let gas_color = event
                .gas
                .and_then(|(cylinder, _)| usize::try_from(cylinder).ok())
                .and_then(|cylinder| pressure_colors.get(cylinder).cloned());
            let label = match event.gas {
                Some((_, gas)) => format!("{} · {}", kind.label(tr), gas.name()),
                None => kind.label(tr).to_string(),
            };
            Some(EventMark {
                event_index,
                x: (fraction - win_start) / win_span * 100.0,
                y: depth_at(&active.samples, event.time.seconds) / max_d * 90.0 + 5.0,
                label,
                time: format_duration(event.time),
                icon: kind.icon(),
                color: gas_color.unwrap_or_else(|| kind.color().to_string()),
                gas_change: kind == EventKind::GasChange,
                sample_index: sample_index_at(fraction, &active.samples, max_t),
            })
        })
        .collect();
    let event_buttons: Vec<Element> = event_marks
        .iter()
        .enumerate()
        .map(|(index, mark)| {
            let sample_index = mark.sample_index;
            let x = mark.x;
            let icon = mark.icon;
            let color = mark.color.clone();
            let label = mark.label.clone();
            let time = mark.time.clone();
            let event_index = mark.event_index;
            rsx! {
                button {
                    key: "event-{index}",
                    class: "profile-event",
                    style: "left: {x:.2}%; --event-color: {color};",
                    title: "{label} · {time}",
                    aria_label: "{label} · {time}",
                    onclick: move |_| {
                        cursor.set(sample_index);
                        hover.set(None);
                        let current = selected_event();
                        selected_event.set(if current == Some(event_index) {
                            None
                        } else {
                            Some(event_index)
                        });
                    },
                    span { class: "profile-event-icon", "{icon}" }
                    span { class: "profile-event-tip", "{label} · {time}" }
                }
            }
        })
        .collect();

    let selected_event_index = (selected_event)();
    let selected_mark = selected_event_index
        .and_then(|index| event_marks.iter().find(|mark| mark.event_index == index));
    let selected_x = selected_mark.map(|mark| mark.x.clamp(9.0, 91.0));
    let selected_remove_index = selected_mark.map(|mark| mark.event_index);
    let selected_event_data = selected_event_index.and_then(|index| active.events.get(index));
    let selected_details = selected_event_data
        .map(|event| event_details(event, tr))
        .unwrap_or_default();
    let selected_icon = selected_event_data.map(|event| event_kind(event).icon());
    let selected_color = selected_mark.map(|mark| mark.color.clone());
    let selected_label = selected_mark.map(|mark| mark.label.clone());
    let selected_time = selected_event_data.map(|event| format_duration(event.time));
    let selected_can_remove = selected_event_data
        .is_some_and(|event| event_kind(event) == EventKind::Bookmark && on_remove_event.is_some());

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

    let speed = speed_at(&active.samples, index);
    let mut readout = vec![
        format_duration(sample.time),
        prefs.depth(sample.depth),
        speed_text(speed),
    ];
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
        readout.push(format!("{} {cns}%", tr.cns));
    }
    if let Some(ceiling) = sample.stop_depth.filter(|d| d.mm > 0) {
        readout.push(format!("{} {}", tr.axis_ceiling, prefs.depth(ceiling)));
    }
    let readout = readout.join("  ·  ");
    let cursor_time = sample.time;

    rsx! {
        div { class: "profile",
            div { class: "profile-plot",
                div { class: "axis-rail axis-left",
                    span { class: "axis-name", style: "{depth_name_style}", "{tr.axis_depth}" }
                    for (_, style, label) in depth_ticks.iter() {
                        span { key: "depth-{label}", class: "axis-label", style: "{style}", "{label}" }
                    }
                }
                div { class: "profile-canvas",
                    if let Some(zone) = ceiling_zone.as_deref() {
                        div {
                            class: "profile-ceiling-zone",
                            style: "clip-path: polygon({zone});",
                        }
                    }
                    svg {
                        class: "profile-svg",
                    view_box: "0 0 100 100",
                    preserve_aspect_ratio: "none",
                    onmounted: move |evt: MountedEvent| async move {
                        if let Ok(rect) = evt.get_client_rect().await {
                            plot_left.set(rect.origin.x);
                            plot_width.set(rect.size.width.max(1.0));
                        }
                    },
                    // The plot narrows when metric scales are added on the
                    // right; keep the pixel width the pointer maps against in
                    // sync so the crosshair stays under the pointer.
                    onresize: move |evt: ResizeEvent| {
                        if let Ok(size) = evt.get_border_box_size() {
                            plot_width.set(size.width.max(1.0));
                        }
                    },
                    // Pointer events cover mouse, touch and pen. Touching a
                    // point shows the same readout as hovering; unlike the
                    // mouse, a touch does not clear it on release so the
                    // values stay readable.
                    onpointerdown: move |evt: PointerEvent| {
                        let width = (plot_width)();
                        if width <= 1.0 {
                            return;
                        }
                        if evt.pointer_type() == "touch" {
                            let x = evt.element_coordinates().x;
                            let y = evt.element_coordinates().y;
                            let id = evt.pointer_id();
                            let mut points = touches.write();
                            points.retain(|(existing, _, _)| *existing != id);
                            points.push((id, x, y));
                            let points = points.clone();
                            let zoom_value = (zoom)().max(1.0);
                            let half = 0.5 / zoom_value;
                            let center = (pan)().clamp(half, 1.0 - half);
                            touch_gesture.set(match points.as_slice() {
                                [(_, x, _)] => Some(TouchGesture::Pan {
                                    start_center: center,
                                    start_x: *x,
                                }),
                                [(_, x1, y1), (_, x2, y2), ..] => {
                                    let mid = ((x1 + x2) / 2.0 / width).clamp(0.0, 1.0);
                                    let anchor = win_start + mid * win_span;
                                    Some(TouchGesture::Pinch {
                                        start_zoom: zoom_value,
                                        start_center: center,
                                        anchor,
                                        start_distance: distance(*x1, *y1, *x2, *y2).max(1.0),
                                    })
                                }
                                _ => None,
                            });
                            return;
                        }
                        let fraction = (evt.element_coordinates().x / width).clamp(0.0, 1.0);
                        hover.set(Some(fraction));
                        drag.set(Some((fraction, fraction)));
                    },
                    onpointermove: move |evt: PointerEvent| {
                        let width = (plot_width)();
                        if width <= 1.0 {
                            return;
                        }
                        if evt.pointer_type() == "touch" {
                            let id = evt.pointer_id();
                            let x = evt.element_coordinates().x;
                            let y = evt.element_coordinates().y;
                            {
                                let mut points = touches.write();
                                if let Some(point) = points.iter_mut().find(|(existing, _, _)| *existing == id) {
                                    point.1 = x;
                                    point.2 = y;
                                }
                            }
                            match (touch_gesture)() {
                                Some(TouchGesture::Pan { start_center, start_x }) => {
                                    let zoom_value = (zoom)().max(1.0);
                                    let half = 0.5 / zoom_value;
                                    let moved = -(x - start_x) / width / zoom_value;
                                    pan.set((start_center + moved).clamp(half, 1.0 - half));
                                }
                                Some(TouchGesture::Pinch {
                                    start_zoom,
                                    start_center,
                                    anchor,
                                    start_distance,
                                }) => {
                                    let points = (touches)();
                                    if points.len() >= 2 {
                                        let distance = distance(
                                            points[0].1,
                                            points[0].2,
                                            points[1].1,
                                            points[1].2,
                                        )
                                        .max(1.0);
                                        let next_zoom =
                                            (start_zoom * distance / start_distance).clamp(1.0, MAX_ZOOM);
                                        let half = 0.5 / next_zoom;
                                        let center = anchor
                                            + (start_center - anchor) * (start_zoom / next_zoom);
                                        zoom.set(next_zoom);
                                        pan.set(center.clamp(half, 1.0 - half));
                                    }
                                }
                                None => {}
                            }
                            return;
                        }
                        let fraction = (evt.element_coordinates().x / width).clamp(0.0, 1.0);
                        hover.set(Some(fraction));
                        if let Some((start, _)) = (drag)() {
                            drag.set(Some((start, fraction)));
                        }
                    },
                    onpointerup: move |evt: PointerEvent| {
                        if evt.pointer_type() == "touch" {
                            finish_touch(
                                evt.pointer_id(),
                                touches,
                                touch_gesture,
                                plot_width,
                                zoom,
                                pan,
                                win_start,
                                win_span,
                            );
                            return;
                        }
                        if let Some((a, b)) = (drag)() {
                            drag.set(None);
                            let (lo, hi) = (a.min(b), a.max(b));
                            if hi - lo > MIN_SELECTION {
                                // The selection is a slice of the visible
                                // window; map it back to full-profile fractions.
                                let f0 = win_start + lo * win_span;
                                let f1 = win_start + hi * win_span;
                                let span = (f1 - f0).max(1e-9);
                                zoom.set((1.0 / span).clamp(1.0, MAX_ZOOM));
                                pan.set((f0 + f1) / 2.0);
                            }
                        }
                    },
                    onpointercancel: move |evt: PointerEvent| {
                        if evt.pointer_type() == "touch" {
                            finish_touch(
                                evt.pointer_id(),
                                touches,
                                touch_gesture,
                                plot_width,
                                zoom,
                                pan,
                                win_start,
                                win_span,
                            );
                            return;
                        }
                        drag.set(None);
                        hover.set(None);
                    },
                    // Keep the readout pinned where the pointer left; the
                    // scrubber (or a new hover) moves it.

                    ondoubleclick: move |_| {
                        zoom.set(1.0);
                        pan.set(0.5);
                    },
                    defs {
                        linearGradient {
                            id: "profile-depth-gradient",
                            gradient_units: "userSpaceOnUse",
                            x1: "0",
                            y1: "5",
                            x2: "0",
                            y2: "95",
                            stop { offset: "0", class: "profile-depth-stop-top" }
                            stop { offset: "1", class: "profile-depth-stop-bottom" }
                        }
                    }
                    polygon {
                        class: "profile-depth-area",
                        points: "{depth_area}",
                    }
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
                        for (index, (points, color)) in pressure_lines.iter().enumerate() {
                            polyline {
                                key: "pressure-{index}",
                                points: "{points}",
                                style: "fill: none; stroke: {color.as_deref().unwrap_or(COLOR_PRESSURE)}; stroke-width: 1.6; stroke-linecap: round; vector-effect: non-scaling-stroke; opacity: 0.95;",
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
                            path {
                                d: "{points}",
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
                    for (index, mark) in event_marks.iter().enumerate() {
                        line {
                            key: "event-line-{index}",
                            x1: "{mark.x}",
                            y1: "0",
                            x2: "{mark.x}",
                            y2: "100",
                            style: "stroke: {mark.color}; stroke-width: 0.8; vector-effect: non-scaling-stroke; opacity: 0.32; pointer-events: none;",
                        }
                    }
                    for (index, mark) in event_marks.iter().filter(|mark| mark.gas_change).enumerate() {
                        line {
                            key: "gas-switch-{index}",
                            class: "profile-gas-switch",
                            x1: "{mark.x}",
                            y1: "{mark.y}",
                            x2: "{mark.x}",
                            y2: "{mark.y}",
                            style: "stroke: {mark.color};",
                        }
                    }
                    line {
                        class: "profile-cursor",
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
                    if sel_on {
                        rect {
                            x: "{sel_x}",
                            y: "0",
                            width: "{sel_w}",
                            height: "100",
                            class: "profile-selection",
                        }
                    }
                    }
                    div { class: "profile-events",
                        for button in event_buttons {
                            {button}
                        }
                        if let (
                            Some(x),
                            Some(event_index),
                            Some(icon),
                            Some(color),
                            Some(label),
                            Some(time),
                        ) = (
                            selected_x,
                            selected_remove_index,
                            selected_icon,
                            selected_color.as_deref(),
                            selected_label.as_deref(),
                            selected_time.as_deref(),
                        ) {
                            div {
                                class: "profile-event-popover",
                                style: "left: {x:.2}%; --event-color: {color};",
                                div { class: "profile-event-popover-head",
                                    span { class: "profile-event-icon", "{icon}" }
                                    strong { "{label}" }
                                    button {
                                        class: "icon-btn",
                                        r#type: "button",
                                        title: "{tr.close}",
                                        aria_label: "{tr.close}",
                                        onclick: move |_| selected_event.set(None),
                                        "✕"
                                    }
                                }
                                div { class: "profile-event-popover-meta", "{time}" }
                                for (detail_label, detail_value) in selected_details.iter() {
                                    div { class: "profile-event-detail",
                                        span { class: "muted", "{detail_label}" }
                                        span { "{detail_value}" }
                                    }
                                }
                                if selected_can_remove {
                                    if let Some(remove) = on_remove_event {
                                        button {
                                            class: "btn danger profile-event-remove",
                                            r#type: "button",
                                            onclick: move |_| {
                                                selected_event.set(None);
                                                remove.call((dc_index, event_index));
                                            },
                                            "{tr.remove_bookmark}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                AxisRail { axes: right_axes }
            }
            div { class: "profile-caption", "{readout}" }
            div { class: "profile-controls",
                if !planned || available.pressure {
                    Toggle { label: tr.pressure, color: COLOR_PRESSURE, on: pressure_on, disabled: !available.pressure, title: tr.tip_pressure, onclick: move |_| overlays_on.write().pressure ^= true }
                }
                if !planned || available.temperature {
                    Toggle { label: tr.temperature, color: COLOR_TEMP, on: temp_on, disabled: !available.temperature, title: tr.tip_temperature, onclick: move |_| overlays_on.write().temp ^= true }
                }
                if !planned || available.ndl {
                    Toggle { label: tr.ndl, color: COLOR_NDL, on: ndl_on, disabled: !available.ndl, title: tr.tip_ndl_toggle, onclick: move |_| overlays_on.write().ndl ^= true }
                }
                if !planned || available.tts {
                    Toggle { label: tr.tts, color: COLOR_TTS, on: tts_on, disabled: !available.tts, title: tr.tip_tts, onclick: move |_| overlays_on.write().tts ^= true }
                }
                if !planned || available.heart {
                    Toggle { label: tr.heart, color: COLOR_HEART, on: heart_on, disabled: !available.heart, title: tr.tip_heart, onclick: move |_| overlays_on.write().heart ^= true }
                }
                if !planned || available.cns {
                    Toggle { label: tr.cns, color: COLOR_CNS, on: cns_on, disabled: !available.cns, title: tr.tip_cns, onclick: move |_| overlays_on.write().cns ^= true }
                }
                if let Some(add_event) = on_add_event {
                    button {
                        class: "btn profile-add-event",
                        title: "{tr.add_bookmark}",
                        aria_label: "{tr.add_bookmark}",
                        onclick: move |_| add_event.call((dc_index, cursor_time)),
                        "+"
                    }
                }
                if zoom_value > 1.01 {
                    button {
                        class: "btn",
                        title: "Reset the zoom and pan (or double-click the graph)",
                        onclick: move |_| {
                            zoom.set(1.0);
                            pan.set(0.5);
                        },
                        "{tr.reset_zoom}"
                    }
                }
                input {
                    class: "scrub",
                    r#type: "range",
                    min: "0",
                    max: "{active.samples.len() - 1}",
                    value: "{index}",
                    oninput: move |evt| {
                        hover.set(None);
                        cursor.set(evt.value().parse().unwrap_or(0));
                    },
                }
            }
            // While a selection drag is active, this full-viewport layer keeps
            // receiving the moves (and the release) even past the chart edges,
            // so the selection stays clamped to the ends instead of stopping.
            if (drag)().is_some() {
                div {
                    class: "profile-drag-overlay",
                    onpointermove: move |evt: PointerEvent| {
                        let width = (plot_width)();
                        if width > 1.0 {
                            let fraction = ((evt.element_coordinates().x - (plot_left)()) / width)
                                .clamp(0.0, 1.0);
                            hover.set(Some(fraction));
                            if let Some((start, _)) = (drag)() {
                                drag.set(Some((start, fraction)));
                            }
                        }
                    },
                    onpointerup: move |_evt: PointerEvent| {
                        if let Some((a, b)) = (drag)() {
                            drag.set(None);
                            let (lo, hi) = (a.min(b), a.max(b));
                            if hi - lo > MIN_SELECTION {
                                let f0 = win_start + lo * win_span;
                                let f1 = win_start + hi * win_span;
                                let span = (f1 - f0).max(1e-9);
                                zoom.set((1.0 / span).clamp(1.0, MAX_ZOOM));
                                pan.set((f0 + f1) / 2.0);
                            }
                        }
                    },
                    onpointercancel: move |_| {
                        drag.set(None);
                        hover.set(None);
                    },
                }
            }
        }
    }
}

/// Which overlays are shown on the profile.
#[derive(Clone, Copy, Default, PartialEq)]
struct Overlays {
    pressure: bool,
    temp: bool,
    ndl: bool,
    tts: bool,
    heart: bool,
    cns: bool,
}

/// One colour-coded value scale per active overlay, right of the plot.
#[component]
fn AxisRail(axes: Vec<(&'static str, String, Vec<Tick>)>) -> Element {
    rsx! {
        div { class: "axis-rail axis-right",
            for (i, (name, name_style, ticks)) in axes.iter().enumerate() {
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
    }
}

/// A marker rendered above the profile for one dive event.
struct EventMark {
    event_index: usize,
    x: f64,
    /// Depth of the trace at the event, in plot percent.
    y: f64,
    label: String,
    time: String,
    icon: &'static str,
    color: String,
    gas_change: bool,
    sample_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    GasChange,
    Bookmark,
    Marker,
    ModeChange,
    Deco,
    Alarm,
    Other,
}

impl EventKind {
    fn label(self, tr: &'static i18n::Strings) -> &'static str {
        match self {
            EventKind::GasChange => tr.event_gas_change,
            EventKind::Bookmark => tr.event_bookmark,
            EventKind::Marker => tr.event_marker,
            EventKind::ModeChange => tr.event_mode_change,
            EventKind::Deco => tr.event_deco,
            EventKind::Alarm => tr.event_alarm,
            EventKind::Other => tr.event_other,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            EventKind::GasChange => "⇄",
            EventKind::Bookmark => "★",
            EventKind::Marker => "◆",
            EventKind::ModeChange => "↻",
            EventKind::Deco => "↑",
            EventKind::Alarm => "!",
            EventKind::Other => "•",
        }
    }

    fn color(self) -> &'static str {
        match self {
            EventKind::GasChange => COLOR_TTS,
            EventKind::Bookmark => COLOR_CNS,
            EventKind::Marker => COLOR_DEPTH,
            EventKind::ModeChange => COLOR_HEART,
            EventKind::Deco => COLOR_CEILING,
            EventKind::Alarm => "var(--speed-toofast)",
            EventKind::Other => "var(--muted)",
        }
    }
}

fn event_kind(event: &Event) -> EventKind {
    if event.is_gas_change() {
        return EventKind::GasChange;
    }
    if event.is_divemode_change() || event.divemode.is_some() {
        return EventKind::ModeChange;
    }
    let name = event.name.to_ascii_lowercase();
    if name.contains("bookmark") {
        EventKind::Bookmark
    } else if name.contains("marker") {
        EventKind::Marker
    } else if name.contains("deco") || name.contains("ceiling") || name.contains("stop") {
        EventKind::Deco
    } else if name.contains("alarm") || name.contains("violation") || name.contains("warning") {
        EventKind::Alarm
    } else {
        EventKind::Other
    }
}

/// Gas switches at the very start or end of a recording are bookkeeping:
/// they just mark the initial gas and the final switch back to it.
fn boundary_gas_switch(event: &Event, last_sample_seconds: i32) -> bool {
    event.is_gas_change()
        && (event.time.seconds <= 0 || event.time.seconds >= last_sample_seconds.saturating_sub(5))
}

fn event_details(event: &Event, tr: &'static i18n::Strings) -> Vec<(String, String)> {
    let mut details = Vec::new();
    if let Some((cylinder, gas)) = event.gas {
        details.push((tr.cyl_gas.to_string(), gas.name()));
        if cylinder >= 0 {
            details.push((tr.cyl_cylinder.to_string(), format!("#{}", cylinder + 1)));
        }
    }
    if let Some(mode) = event.divemode {
        details.push((tr.dive_mode.to_string(), mode.label().to_string()));
    }
    details
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
    let all: Vec<(f64, i32)> = values.map(|(t, v)| (t as f64 / max_t, v)).collect();
    let (lo, hi) = value_window(&all, start, end);
    all[lo..hi]
        .iter()
        .map(|(fraction, v)| {
            let x = (fraction - start) / span * 100.0;
            let y = *v as f64 / max_v.max(1e-9) * 90.0 + 5.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The water column under the depth trace as a closed polygon, plus the zone
/// above the decompression ceiling (`None` without a ceiling) as CSS
/// `polygon()` points.
///
/// The column's top edge is the ceiling (the surface when there is none),
/// never deeper than the diver.
fn depth_area(
    samples: &[benthic_core::Sample],
    max_d: f64,
    max_t: f64,
    start: f64,
    end: f64,
) -> (String, Option<String>) {
    if samples.is_empty() {
        return (String::new(), None);
    }
    let span = (end - start).max(1e-9);
    let first = samples.partition_point(|s| (s.time.seconds as f64 / max_t) < start);
    let after = samples.partition_point(|s| (s.time.seconds as f64 / max_t) <= end);
    let window = &samples[first.saturating_sub(1)..(after + 1).min(samples.len())];
    let x_of =
        |sample: &benthic_core::Sample| (sample.time.seconds as f64 / max_t - start) / span * 100.0;
    let y_of = |mm: i32| mm as f64 / max_d.max(1e-9) * 90.0 + 5.0;
    let ceiling = |s: &benthic_core::Sample| s.stop_depth.map_or(0, |d| d.mm.clamp(0, s.depth.mm));
    // The ceiling moves in stop steps: between two samples, hold the shallower
    // of the two so the edge never dips below the diver.
    let mut top: Vec<(f64, f64)> = vec![(x_of(&window[0]), y_of(ceiling(&window[0])))];
    for pair in window.windows(2) {
        let level = y_of(ceiling(&pair[0]).min(ceiling(&pair[1])));
        top.push((x_of(&pair[0]), level));
        top.push((x_of(&pair[1]), level));
    }
    if let Some(last) = window.last() {
        top.push((x_of(last), y_of(ceiling(last))));
    }
    top.dedup();

    let area = window
        .iter()
        .map(|s| (x_of(s), y_of(s.depth.mm)))
        .chain(top.iter().rev().copied())
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect::<Vec<_>>()
        .join(" ");

    let surface = y_of(0);
    let zone = top.iter().any(|(_, y)| *y > surface + 1e-6).then(|| {
        let (first_x, last_x) = (top[0].0, top[top.len() - 1].0);
        std::iter::once((first_x, surface))
            .chain(top.iter().copied())
            .chain(std::iter::once((last_x, surface)))
            .map(|(x, y)| format!("{x:.2}% {y:.2}%"))
            .collect::<Vec<_>>()
            .join(", ")
    });
    (area, zone)
}

/// The trace depth (mm) at a time, interpolated between samples.
fn depth_at(samples: &[benthic_core::Sample], seconds: i32) -> f64 {
    let index = samples.partition_point(|s| s.time.seconds < seconds);
    match (
        index.checked_sub(1).and_then(|i| samples.get(i)),
        samples.get(index),
    ) {
        (Some(a), Some(b)) if b.time.seconds > a.time.seconds => {
            let fraction =
                (seconds - a.time.seconds) as f64 / (b.time.seconds - a.time.seconds) as f64;
            a.depth.mm as f64 + (b.depth.mm - a.depth.mm) as f64 * fraction
        }
        (_, Some(sample)) | (Some(sample), None) => sample.depth.mm as f64,
        (None, None) => 0.0,
    }
}

/// A drag narrower than this fraction of the plot is treated as a click, not a
/// zoom selection.
const MIN_SELECTION: f64 = 0.02;

#[derive(Clone, Copy)]
enum TouchGesture {
    Pan {
        start_center: f64,
        start_x: f64,
    },
    Pinch {
        start_zoom: f64,
        start_center: f64,
        anchor: f64,
        start_distance: f64,
    },
}

fn distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt()
}

/// Remove a lifted touch and restart a one-finger pan or pinch if fingers
/// remain. This keeps gestures continuous when a second finger joins or
/// leaves mid-gesture.
#[allow(clippy::too_many_arguments)]
fn finish_touch(
    id: i32,
    mut touches: Signal<Vec<(i32, f64, f64)>>,
    mut touch_gesture: Signal<Option<TouchGesture>>,
    plot_width: Signal<f64>,
    zoom: Signal<f64>,
    pan: Signal<f64>,
    win_start: f64,
    win_span: f64,
) {
    let points = {
        let mut points = touches.write();
        points.retain(|(existing, _, _)| *existing != id);
        points.clone()
    };
    let width = (plot_width)();
    let zoom_value = (zoom)().max(1.0);
    let half = 0.5 / zoom_value;
    let center = (pan)().clamp(half, 1.0 - half);
    touch_gesture.set(match points.as_slice() {
        [] => None,
        [(_, x, _)] => Some(TouchGesture::Pan {
            start_center: center,
            start_x: *x,
        }),
        [(_, x1, y1), (_, x2, y2), ..] => {
            let mid = ((x1 + x2) / 2.0 / width.max(1.0)).clamp(0.0, 1.0);
            Some(TouchGesture::Pinch {
                start_zoom: zoom_value,
                start_center: center,
                anchor: win_start + mid * win_span,
                start_distance: distance(*x1, *y1, *x2, *y2).max(1.0),
            })
        }
    });
}
/// The largest zoom a drag selection may request.
const MAX_ZOOM: f64 = 100.0;

/// Vertical-speed bands used to colour the depth trace, fastest first. `min`
/// is the lower bound on the *magnitude* of the speed in metres per minute,
/// so descents and ascents share one three-colour scale.
const SPEED_BANDS: [(&str, &str, f64); 3] = [
    ("Too fast", "var(--speed-toofast)", 20.0),
    ("A bit fast", "var(--speed-fast)", 10.0),
    ("Normal", COLOR_DEPTH, 0.0),
];

/// The depth speed (m/min, positive when descending) arriving at a sample,
/// taken from the segment leading into it (or leaving it, for the first
/// sample). The sign matches the depth trace's speed bands.
fn speed_at(samples: &[benthic_core::Sample], index: usize) -> f64 {
    let (a, b) = if index > 0 {
        (index - 1, index)
    } else if samples.len() > 1 {
        (0, 1)
    } else {
        return 0.0;
    };
    let minutes = (samples[b].time.seconds - samples[a].time.seconds) as f64 / 60.0;
    if minutes <= 0.0 {
        return 0.0;
    }
    (samples[b].depth.mm - samples[a].depth.mm) as f64 / 1000.0 / minutes
}

/// Format a vertical speed for the readout.
fn speed_text(speed: f64) -> String {
    if speed.abs() < 0.5 {
        "0 m/min".to_string()
    } else {
        format!("{speed:.0} m/min")
    }
}

/// The band the magnitude of a vertical speed (m/min) falls into.
fn speed_band(speed: f64) -> usize {
    let speed = speed.abs();
    SPEED_BANDS
        .iter()
        .position(|(_, _, min)| speed >= *min)
        .unwrap_or(SPEED_BANDS.len() - 1)
}

/// The slice `[lo, hi)` of a `(fraction, value)` series covering the visible
/// window plus one sample on each side, so a segment that crosses the window
/// edge is still drawn when the window is sparse.
fn value_window<T>(all: &[(f64, T)], start: f64, end: f64) -> (usize, usize) {
    let first = all.partition_point(|(f, _)| *f < start);
    let after = all.partition_point(|(f, _)| *f <= end);
    (first.saturating_sub(1), (after + 1).min(all.len()))
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
    if samples.is_empty() {
        return bands;
    }
    let first = samples.partition_point(|s| (s.time.seconds as f64 / max_t) < start);
    let after = samples.partition_point(|s| (s.time.seconds as f64 / max_t) <= end);
    let lo = first.saturating_sub(1);
    let hi = (after + 1).min(samples.len());
    let mut previous: Option<(f64, f64, i32, f64)> = None;
    for sample in &samples[lo..hi] {
        let fraction = sample.time.seconds as f64 / max_t;
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

/// The whole depth trace as one path in the depth colour.
fn single_band(
    samples: &[benthic_core::Sample],
    max_d: f64,
    max_t: f64,
    start: f64,
    end: f64,
) -> Vec<(&'static str, &'static str, String)> {
    let points = scaled_series(
        samples.iter().map(|s| (s.time.seconds, s.depth.mm)),
        max_d,
        max_t,
        start,
        end,
    );
    if points.is_empty() {
        return Vec::new();
    }
    vec![(
        "Depth",
        COLOR_DEPTH,
        format!("M {}", points.replace(' ', " L ")),
    )]
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
    let all: Vec<(f64, i32)> = values.map(|(t, v)| (t as f64 / max_t, v)).collect();
    if all.is_empty() {
        return None;
    }
    let (lo, hi) = value_window(&all, start, end);
    let first = all.partition_point(|(f, _)| *f < start);
    let after = all.partition_point(|(f, _)| *f <= end);
    // Scale the axis to the samples actually in the window; only fall back to
    // the bracketing samples when the window is too sparse to have a range.
    let strict = &all[first..after];
    let range = if strict.len() >= 2 {
        strict
    } else {
        &all[lo..hi]
    };
    let min = range.iter().map(|(_, v)| *v).min()?;
    let max = range.iter().map(|(_, v)| *v).max()?;
    let value_span = (max - min) as f64;
    let points = all[lo..hi]
        .iter()
        .map(|(fraction, v)| {
            let x = (fraction - start) / span * 100.0;
            // A constant series has no range of its own; centre it so it lines
            // up with the single tick its scale draws.
            // Use the same 5%..95% vertical band as depth so every right-hand
            // scale starts and ends on the same plot lines.
            let y = if value_span <= 0.0 {
                50.0
            } else {
                95.0 - (*v - min) as f64 / value_span * 90.0
            };
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(Series { points, min, max })
}

/// Normalize a series to a fixed value range, using the same 5%..95% band as
/// the axis ticks so the trace and its labels line up.
fn series_fixed(
    values: impl Iterator<Item = (i32, i32)>,
    min: i32,
    max: i32,
    max_t: f64,
    start: f64,
    end: f64,
) -> Option<Series> {
    let span = (end - start).max(1e-9);
    let all: Vec<(f64, i32)> = values.map(|(t, v)| (t as f64 / max_t, v)).collect();
    if all.is_empty() {
        return None;
    }
    let (lo, hi) = value_window(&all, start, end);
    let value_span = (max - min) as f64;
    let points = all[lo..hi]
        .iter()
        .map(|(fraction, value)| {
            let x = (fraction - start) / span * 100.0;
            let value = (*value).clamp(min, max);
            let y = if value_span <= 0.0 {
                50.0
            } else {
                95.0 - (value - min) as f64 / value_span * 90.0
            };
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(Series { points, min, max })
}

/// Normalize a series to an explicitly supplied value range so several
/// pressure traces share one axis.
fn series_points(
    values: impl Iterator<Item = (i32, i32)>,
    min: i32,
    max: i32,
    max_t: f64,
    start: f64,
    end: f64,
) -> Option<String> {
    let span = (end - start).max(1e-9);
    let all: Vec<(f64, i32)> = values.map(|(t, v)| (t as f64 / max_t, v)).collect();
    if all.is_empty() {
        return None;
    }
    let (lo, hi) = value_window(&all, start, end);
    let value_span = (max - min) as f64;
    Some(
        all[lo..hi]
            .iter()
            .map(|(fraction, value)| {
                let x = (fraction - start) / span * 100.0;
                let y = if value_span <= 0.0 {
                    50.0
                } else {
                    92.0 - (*value - min) as f64 / value_span * 84.0
                };
                format!("{x:.2},{y:.2}")
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Like [`series_fixed`], but values below `min` are placed under the plot
/// rather than clamped onto its edge, and runs of them are left out. The
/// points are SVG path data.
fn clipped_series(
    values: impl Iterator<Item = (i32, i32)>,
    min: i32,
    max: i32,
    max_t: f64,
    start: f64,
    end: f64,
) -> Option<Series> {
    const BELOW: f64 = 105.0;
    let span = (end - start).max(1e-9);
    let all: Vec<(f64, i32)> = values.map(|(t, v)| (t as f64 / max_t, v)).collect();
    if all.is_empty() {
        return None;
    }
    let (lo, hi) = value_window(&all, start, end);
    let value_span = (max - min).max(1) as f64;
    let mut d = String::new();
    let mut previous: Option<(f64, f64, bool)> = None;
    for (fraction, value) in &all[lo..hi] {
        let x = (fraction - start) / span * 100.0;
        let visible = *value >= min;
        let y = if visible {
            95.0 - ((*value).min(max) - min) as f64 / value_span * 90.0
        } else {
            BELOW
        };
        if let Some((px, py, previous_visible)) = previous {
            if visible || previous_visible {
                if !d.ends_with(&format!("{px:.2},{py:.2} ")) {
                    d.push_str(&format!("M {px:.2},{py:.2} "));
                }
                d.push_str(&format!("L {x:.2},{y:.2} "));
            }
        }
        previous = Some((x, y, visible));
    }
    Some(Series {
        points: d,
        min,
        max,
    })
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
    value_axis_ticks(min, max, &[max, min + (max - min) / 2, min], color, fmt)
}

/// A value scale with ticks at the given values.
fn value_axis_ticks(
    min: i32,
    max: i32,
    values: &[i32],
    color: &str,
    fmt: impl Fn(i32) -> String,
) -> Vec<Tick> {
    if min == max {
        return vec![(50.0, format!("top: 50%; color: {color};"), fmt(min))];
    }
    let range = (max - min) as f64;
    values
        .iter()
        .map(|&value| {
            let y = 95.0 - (value - min) as f64 / range * 90.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use benthic_core::gas::AIR;
    use benthic_core::Divemode;

    #[test]
    fn recognises_event_types() {
        let gas = Event {
            gas: Some((0, AIR)),
            ..Default::default()
        };
        assert_eq!(event_kind(&gas), EventKind::GasChange);
        assert_eq!(
            event_kind(&Event {
                name: "bookmark".into(),
                ..Default::default()
            }),
            EventKind::Bookmark
        );
        assert_eq!(
            event_kind(&Event {
                name: "Marker".into(),
                ..Default::default()
            }),
            EventKind::Marker
        );
        assert_eq!(
            event_kind(&Event {
                name: "modechange".into(),
                divemode: Some(Divemode::Ccr),
                ..Default::default()
            }),
            EventKind::ModeChange
        );
        assert_eq!(
            event_kind(&Event {
                name: "deco stop".into(),
                ..Default::default()
            }),
            EventKind::Deco
        );
        assert_eq!(
            event_kind(&Event {
                name: "alarm".into(),
                ..Default::default()
            }),
            EventKind::Alarm
        );
    }
    #[test]
    fn boundary_gas_switches_are_not_markers() {
        use benthic_core::units::Duration;
        let gas = Event {
            gas: Some((0, AIR)),
            ..Default::default()
        };
        assert!(boundary_gas_switch(&gas, 1800));
        let mut end = gas.clone();
        end.time = Duration::new(1800);
        assert!(boundary_gas_switch(&end, 1800));
        let mut mid = gas.clone();
        mid.time = Duration::new(900);
        assert!(!boundary_gas_switch(&mid, 1800));
        let bookmark = Event {
            time: Duration::new(1800),
            name: "bookmark".into(),
            ..Default::default()
        };
        assert!(!boundary_gas_switch(&bookmark, 1800));
    }
}
