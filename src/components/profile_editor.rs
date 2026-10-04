//! A planner-like editor for hand-entered dive profiles.
//!
//! Unlike the decompression planner, this screen does **not** compute stops or
//! apply any model: it simply records the depth/time trace the diver supplies
//! as a sequence of waypoints, interpolates it into samples, and returns them.
//! Max depth and bottom time are read *out* of that trace rather than typed in.

use dioxus::prelude::*;

use benthic_core::units::{format_duration, Depth, Duration};
use benthic_core::{Dive, DiveComputer, Sample};

use crate::components::DiveProfile;
use crate::i18n;
use crate::state::AppState;

/// How finely a segment is sampled when turning waypoints into a profile.
const SAMPLE_STEP_SECONDS: i32 = 15;

/// A waypoint is the end of one segment: the depth reached, reached over
/// `duration`.
#[component]
pub fn ProfileEditorDialog(
    /// Existing samples to start from; empty for a brand-new dive.
    initial: Vec<Sample>,
    on_save: EventHandler<Vec<Sample>>,
    on_close: EventHandler<()>,
) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);
    let mut points = use_signal(|| waypoints_from_samples(&initial));

    let current = (points)();
    let samples = samples_from_waypoints(&current);
    let (max_depth, duration) = profile_bounds(&samples);

    let graph_dive = Dive {
        max_depth,
        duration,
        computers: vec![DiveComputer {
            model: "Profile".to_string(),
            max_depth,
            duration,
            samples: samples.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let max_depth_text = max_depth
        .map(|depth| prefs.depth(depth))
        .unwrap_or_else(|| "—".to_string());
    let duration_text = duration
        .map(format_duration)
        .unwrap_or_else(|| "—".to_string());

    let add_point = move |_| {
        let mut list = points.write();
        let last = list
            .last()
            .copied()
            .unwrap_or((Depth::ZERO, Duration::ZERO));
        // A new point repeats the current depth for 5 more minutes; easy to
        // edit from there.
        list.push((last.0, Duration::from_minutes(5)));
    };

    let save = move |_| on_save.call(samples_from_waypoints(&(points)()));

    rsx! {
        section { class: "planner-screen profile-editor",
            div { class: "planner-inner",
                header { class: "detail-head",
                    div { class: "detail-title-row",
                        button {
                            class: "btn back-btn",
                            title: "{tr.back}",
                            onclick: move |_| on_close.call(()),
                            "\u{2039} {tr.back}"
                        }
                        h1 { "{tr.profile_title}" }
                        div { class: "detail-actions",
                            button { class: "btn primary", onclick: save, "{tr.use_profile}" }
                            button { class: "btn", onclick: move |_| on_close.call(()), "{tr.cancel}" }
                        }
                    }
                    p { class: "muted", "{tr.pe_explainer}" }
                }

                DiveProfile { dive: graph_dive, dc_index: 0 }

                div { class: "facts planner-results",
                    div { class: "fact",
                        span { class: "fact-label", "{tr.fact_max_depth}" }
                        span { class: "fact-value", "{max_depth_text}" }
                    }
                    div { class: "fact",
                        span { class: "fact-label", "{tr.bottom_time}" }
                        span { class: "fact-value", "{duration_text}" }
                    }
                    div { class: "fact",
                        span { class: "fact-label", "{tr.points}" }
                        span { class: "fact-value", "{current.len()}" }
                    }
                }

                div { class: "section-title", "{tr.waypoints}" }
                p { class: "muted profile-hint", "{tr.waypoints_hint}" }
                div { class: "plan-points",
                    {current.iter().enumerate().map(|(index, (depth, dur))| {
                        let depth_value = format!("{:.1}", prefs.depth_value(*depth));
                        let minutes_value = format!("{:.0}", dur.seconds.max(0) as f64 / 60.0);
                        let can_remove = current.len() > 1;
                        rsx! {
                            div { key: "{index}", class: "profile-point",
                                span { class: "plan-point-index", "{index + 1}" }
                                label { class: "field-label", {i18n::t1(tr.field_depth, prefs.depth_unit())}
                                    input {
                                        class: "field",
                                        r#type: "number",
                                        inputmode: "decimal",
                                        min: "0",
                                        step: "0.1",
                                        value: "{depth_value}",
                                        oninput: move |evt| {
                                            let mut list = points.write();
                                            if let Some(entry) = list.get_mut(index) {
                                                if let Ok(value) = evt.value().parse::<f64>() {
                                                    entry.0 = prefs.depth_from_value(value);
                                                }
                                            }
                                        },
                                    }
                                }
                                label { class: "field-label", "{tr.duration_min}"
                                    input {
                                        class: "field",
                                        r#type: "number",
                                        inputmode: "numeric",
                                        min: "0",
                                        step: "1",
                                        value: "{minutes_value}",
                                        oninput: move |evt| {
                                            let mut list = points.write();
                                            if let Some(entry) = list.get_mut(index) {
                                                if let Ok(value) = evt.value().parse::<f64>() {
                                                    entry.1 = Duration::new((value.max(0.0) * 60.0).round() as i32);
                                                }
                                            }
                                        },
                                    }
                                }
                                button {
                                    class: "icon-btn point-remove",
                                    title: "{tr.remove_waypoint}",
                                    disabled: !can_remove,
                                    onclick: move |_| {
                                        points.write().remove(index);
                                    },
                                    "\u{00d7}"
                                }
                            }
                        }
                    })}
                }
                button { class: "btn", onclick: add_point, "{tr.add_point}" }
            }
        }
    }
}

/// Turn a waypoint list into a sample trace. Linear on descent and ascent,
/// flat along a repeated depth.
pub fn samples_from_waypoints(waypoints: &[(Depth, Duration)]) -> Vec<Sample> {
    let mut nodes: Vec<(i32, Depth)> = vec![(0, Depth::ZERO)];
    let mut time = 0i32;
    for (depth, dur) in waypoints {
        let start_time = time;
        let start_depth = nodes.last().map(|(_, depth)| *depth).unwrap_or(Depth::ZERO);
        let end_time = time + dur.seconds.max(0);
        let span = end_time - start_time;
        if span > 0 {
            let mut t = start_time + SAMPLE_STEP_SECONDS.min(span);
            while t < end_time {
                let fraction = (t - start_time) as f64 / span as f64;
                nodes.push((t, lerp_depth(start_depth, *depth, fraction)));
                t += SAMPLE_STEP_SECONDS;
            }
        }
        nodes.push((end_time, *depth));
        time = end_time;
    }
    normalize_nodes(nodes)
}

/// Derive the waypoint list from an existing trace by keeping its turning
/// points (start, end, and local minima/maxima).
pub fn waypoints_from_samples(samples: &[Sample]) -> Vec<(Depth, Duration)> {
    if samples.is_empty() {
        return default_waypoints();
    }

    let mut nodes: Vec<(i32, Depth)> = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let keep = index == 0 || index + 1 == samples.len() || {
            let previous = samples[index - 1].depth.mm;
            let current = sample.depth.mm;
            let next = samples[index + 1].depth.mm;
            (current - previous).signum() != (next - current).signum()
        };
        if keep {
            nodes.push((sample.time.seconds, sample.depth));
        }
    }

    let mut result: Vec<(Depth, Duration)> = Vec::new();
    let mut previous_time = 0i32;
    for (index, (time, depth)) in nodes.iter().enumerate() {
        if index == 0 {
            previous_time = *time;
            continue;
        }
        result.push((*depth, Duration::new((time - previous_time).max(0))));
        previous_time = *time;
    }

    if result.is_empty() {
        default_waypoints()
    } else {
        result
    }
}

/// The deepest point (if any) and the last timestamp of a trace.
pub fn profile_bounds(samples: &[Sample]) -> (Option<Depth>, Option<Duration>) {
    let max_depth = samples
        .iter()
        .map(|sample| sample.depth.mm)
        .max()
        .filter(|mm| *mm > 0)
        .map(Depth::new);
    let duration = samples
        .last()
        .map(|sample| sample.time)
        .filter(|duration| duration.seconds > 0);
    (max_depth, duration)
}

fn default_waypoints() -> Vec<(Depth, Duration)> {
    vec![
        (Depth::from_meters(20.0), Duration::from_minutes(2)),
        (Depth::from_meters(20.0), Duration::from_minutes(25)),
        (Depth::ZERO, Duration::from_minutes(5)),
    ]
}

fn lerp_depth(start: Depth, end: Depth, fraction: f64) -> Depth {
    Depth::new((start.mm as f64 + (end.mm - start.mm) as f64 * fraction).round() as i32)
}

fn normalize_nodes(nodes: Vec<(i32, Depth)>) -> Vec<Sample> {
    let mut out: Vec<(i32, Depth)> = Vec::new();
    for (time, depth) in nodes {
        if let Some(last) = out.last_mut() {
            if last.0 == time {
                last.1 = depth;
                continue;
            }
        }
        out.push((time, depth));
    }
    out.into_iter()
        .map(|(time, depth)| Sample {
            time: Duration::new(time),
            depth,
            manually_entered: true,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trapezoid_waypoints_round_trip_bounds() {
        let waypoints = vec![
            (Depth::from_meters(20.0), Duration::from_minutes(2)),
            (Depth::from_meters(20.0), Duration::from_minutes(25)),
            (Depth::ZERO, Duration::from_minutes(5)),
        ];
        let samples = samples_from_waypoints(&waypoints);
        assert_eq!(samples.first().unwrap().depth, Depth::ZERO);
        assert_eq!(samples.last().unwrap().depth, Depth::ZERO);
        assert_eq!(samples.last().unwrap().time, Duration::from_minutes(32));

        let (max_depth, duration) = profile_bounds(&samples);
        assert_eq!(max_depth, Some(Depth::from_meters(20.0)));
        assert_eq!(duration, Some(Duration::from_minutes(32)));
    }

    #[test]
    fn waypoints_recover_turning_points() {
        let waypoints = vec![
            (Depth::from_meters(30.0), Duration::from_minutes(2)),
            (Depth::from_meters(30.0), Duration::from_minutes(18)),
            (Depth::ZERO, Duration::from_minutes(6)),
        ];
        let samples = samples_from_waypoints(&waypoints);
        let recovered = waypoints_from_samples(&samples);
        assert_eq!(recovered.len(), 3);
        assert_eq!(recovered[0], waypoints[0]);
        assert_eq!(recovered[1].0, waypoints[1].0);
        assert_eq!(recovered[2], waypoints[2]);
    }

    #[test]
    fn empty_trace_gets_a_sensible_default() {
        let waypoints = waypoints_from_samples(&[]);
        assert!(!waypoints.is_empty());
        let samples = samples_from_waypoints(&waypoints);
        assert!(profile_bounds(&samples).0.is_some());
    }
}
