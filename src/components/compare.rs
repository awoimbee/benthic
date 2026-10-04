use dioxus::prelude::*;

use benthic_core::units::format_duration;
use benthic_core::Dive;

use crate::i18n;
use crate::state::AppState;

/// A modal that overlays two selected dives' profiles and compares their key
/// numbers. When one of the dives is a saved plan (tag `planned`), this is a
/// plan-versus-actual comparison.
#[component]
pub fn CompareDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_compare = state.show_compare;
    let log = (state.log)();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    let selected: Vec<u32> = (state.selection)().iter().copied().take(2).collect();
    let dives: Vec<Dive> = selected
        .iter()
        .filter_map(|id| log.dive_by_id(*id).cloned())
        .collect();

    if dives.len() < 2 {
        return rsx! {
            div {
                class: "modal-backdrop",
                onclick: move |_| show_compare.set(false),
                div {
                    class: "modal",
                    role: "dialog",
                    aria_modal: "true",
                    onclick: move |evt| evt.stop_propagation(),
                    h2 { "{tr.compare_dives}" }
                    p { class: "muted", "{tr.compare_hint}" }
                    div { class: "detail-actions",
                        button { class: "btn primary", onclick: move |_| show_compare.set(false), "{tr.close}" }
                    }
                }
            }
        };
    }

    let (a, b) = (&dives[0], &dives[1]);
    let max_t = a
        .duration()
        .map(|d| d.seconds)
        .unwrap_or(0)
        .max(b.duration().map(|d| d.seconds).unwrap_or(0))
        .max(1) as f64;
    let max_d = a
        .max_depth()
        .map(|d| d.mm)
        .unwrap_or(0)
        .max(b.max_depth().map(|d| d.mm).unwrap_or(0))
        .max(1) as f64;

    let points_a = depth_points(a, max_t, max_d);
    let points_b = depth_points(b, max_t, max_d);
    let title_a = crate::format::dive_title(a, &log);
    let title_b = crate::format::dive_title(b, &log);

    let rows: Vec<(&'static str, String, String)> = vec![
        (tr.row_duration, duration_text(a), duration_text(b)),
        (
            tr.row_max_depth,
            depth_text(a.max_depth(), &prefs),
            depth_text(b.max_depth(), &prefs),
        ),
        (
            tr.row_avg_depth,
            depth_text(a.average_depth(), &prefs),
            depth_text(b.average_depth(), &prefs),
        ),
        (tr.row_rmv, rmv_text(a), rmv_text(b)),
        (
            tr.row_water_temp,
            temp_text(a, &prefs),
            temp_text(b, &prefs),
        ),
        (tr.row_gas, gas_text(a), gas_text(b)),
        (tr.row_tags, a.tags.join(", "), b.tags.join(", ")),
    ];

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_compare.set(false),
            div {
                class: "modal wide",
                role: "dialog",
                aria_modal: "true",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.compare_dives}" }
                div { class: "profile",
                    svg {
                        class: "profile-svg",
                        view_box: "0 0 100 100",
                        preserve_aspect_ratio: "none",
                        polyline {
                            points: "{points_a}",
                            style: "fill: none; stroke: #4cc9f0; stroke-width: 1.5; vector-effect: non-scaling-stroke;",
                        }
                        polyline {
                            points: "{points_b}",
                            style: "fill: none; stroke: #f5a623; stroke-width: 1.5; vector-effect: non-scaling-stroke;",
                        }
                    }
                    div { class: "profile-caption",
                        span { style: "color: #4cc9f0;", "{title_a}" }
                        "  vs  "
                        span { style: "color: #f5a623;", "{title_b}" }
                    }
                }
                table { class: "data-table",
                    thead {
                        tr {
                            th { "" }
                            th { "{title_a}" }
                            th { "{title_b}" }
                        }
                    }
                    tbody {
                        for (label, left, right) in rows {
                            tr { td { "{label}" } td { "{left}" } td { "{right}" } }
                        }
                    }
                }
                div { class: "detail-actions",
                    button { class: "btn primary", onclick: move |_| show_compare.set(false), "{tr.close}" }
                }
            }
        }
    }
}

fn depth_points(dive: &Dive, max_t: f64, max_d: f64) -> String {
    let Some(dc) = dive.primary_computer() else {
        return String::new();
    };
    dc.samples
        .iter()
        .map(|s| {
            let x = s.time.seconds as f64 / max_t * 100.0;
            let y = s.depth.mm as f64 / max_d * 90.0 + 5.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn duration_text(dive: &Dive) -> String {
    dive.duration()
        .map(format_duration)
        .unwrap_or_else(|| "—".into())
}

fn depth_text(depth: Option<benthic_core::Depth>, prefs: &benthic_core::Preferences) -> String {
    depth.map(|d| prefs.depth(d)).unwrap_or_else(|| "—".into())
}

fn rmv_text(dive: &Dive) -> String {
    dive.rmv_l_per_min()
        .map(|v| format!("{v:.1} L/min"))
        .unwrap_or_else(|| "—".into())
}

fn temp_text(dive: &Dive, prefs: &benthic_core::Preferences) -> String {
    dive.water_temp
        .or_else(|| dive.primary_computer().and_then(|dc| dc.water_temp))
        .map(|t| prefs.temperature(t))
        .unwrap_or_else(|| "—".into())
}

fn gas_text(dive: &Dive) -> String {
    dive.cylinders
        .first()
        .map(|c| c.gas.name())
        .unwrap_or_else(|| "—".into())
}
