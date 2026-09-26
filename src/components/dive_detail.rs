use dioxus::prelude::*;

use benthic_core::units::{format_duration, format_timestamp_utc};
use benthic_core::{Cylinder, WeightSystem};

use crate::components::DiveProfile;
use crate::state::AppState;

struct CylinderRow {
    description: String,
    gas: String,
    start: String,
    end: String,
}

struct WeightRow {
    description: String,
    weight: String,
}

#[component]
pub fn DiveDetail() -> Element {
    let state = use_context::<AppState>();
    let log = (state.log)();
    let selected = (state.selected)();

    let Some(dive) = selected.and_then(|id| log.dive_by_id(id).cloned()) else {
        return rsx! {
            section { class: "detail",
                div { class: "empty-hint", "Select a dive to see its details." }
            }
        };
    };

    let title = crate::format::dive_title(&dive, &log);
    let site_name = dive
        .site_id
        .and_then(|id| log.site_by_uuid(id))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "—".to_string());
    let when = format_timestamp_utc(dive.when);
    let duration = dive
        .duration()
        .map(format_duration)
        .unwrap_or_else(|| "—".to_string());
    let max_depth = dive
        .max_depth()
        .map(|d| format!("{:.1} m", d.meters()))
        .unwrap_or_else(|| "—".to_string());
    let water_temp = dive
        .water_temp
        .or_else(|| dive.primary_computer().and_then(|dc| dc.water_temp))
        .map(|t| format!("{:.1} °C", t.celsius()))
        .unwrap_or_else(|| "—".to_string());
    let air_temp = dive
        .air_temp
        .or_else(|| dive.primary_computer().and_then(|dc| dc.air_temp))
        .map(|t| format!("{:.1} °C", t.celsius()))
        .unwrap_or_else(|| "—".to_string());
    let computer = dive
        .primary_computer()
        .map(|dc| dc.model.clone())
        .unwrap_or_else(|| "—".to_string());
    let buddy = dive.buddy.clone();
    let divemaster = dive.diveguide.clone();
    let suit = dive.suit.clone();
    let notes = dive.notes.clone();
    let tags = dive.tags.clone();

    let cylinders: Vec<CylinderRow> = dive.cylinders.iter().map(cylinder_row).collect();
    let weights: Vec<WeightRow> = dive.weights.iter().map(weight_row).collect();

    rsx! {
        section { class: "detail",
            header { class: "detail-head",
                h1 { "{title}" }
                p { class: "muted", "{when} · {site_name}" }
            }

            DiveProfile { dive: dive.clone() }

            div { class: "facts",
                Fact { label: "Duration", value: duration }
                Fact { label: "Max depth", value: max_depth }
                Fact { label: "Water temp", value: water_temp }
                Fact { label: "Air temp", value: air_temp }
                Fact { label: "Computer", value: computer }
                Fact { label: "Buddy", value: buddy }
                Fact { label: "Dive master", value: divemaster }
                Fact { label: "Suit", value: suit }
            }

            if !tags.is_empty() {
                div { class: "tags",
                    for tag in tags {
                        span { key: "{tag}", class: "tag", "{tag}" }
                    }
                }
            }

            if !cylinders.is_empty() {
                div { class: "section-title", "Gas & equipment" }
                table { class: "data-table",
                    thead { tr { th { "Cylinder" } th { "Gas" } th { "Start" } th { "End" } } }
                    tbody {
                        for row in cylinders {
                            tr {
                                td { "{row.description}" }
                                td { "{row.gas}" }
                                td { "{row.start}" }
                                td { "{row.end}" }
                            }
                        }
                    }
                }
                if !weights.is_empty() {
                    table { class: "data-table",
                        thead { tr { th { "Weight" } th { "Description" } } }
                        tbody {
                            for row in weights {
                                tr { td { "{row.weight}" } td { "{row.description}" } }
                            }
                        }
                    }
                }
            }

            if !notes.is_empty() {
                div { class: "section-title", "Notes" }
                p { class: "notes", "{notes}" }
            }
        }
    }
}

#[component]
fn Fact(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "fact",
            span { class: "fact-label", "{label}" }
            span { class: "fact-value", "{value}" }
        }
    }
}

fn cylinder_row(cyl: &Cylinder) -> CylinderRow {
    CylinderRow {
        description: if cyl.description.is_empty() {
            cyl.size
                .map(|s| format!("{:.1} L", s.liters()))
                .unwrap_or_else(|| "—".to_string())
        } else {
            cyl.description.clone()
        },
        gas: cyl.gas.name(),
        start: cyl
            .start_pressure
            .map(|p| format!("{:.0} bar", p.bar()))
            .unwrap_or_else(|| "—".to_string()),
        end: cyl
            .end_pressure
            .map(|p| format!("{:.0} bar", p.bar()))
            .unwrap_or_else(|| "—".to_string()),
    }
}

fn weight_row(ws: &WeightSystem) -> WeightRow {
    WeightRow {
        description: if ws.description.is_empty() {
            "—".to_string()
        } else {
            ws.description.clone()
        },
        weight: format!("{:.2} kg", ws.weight.kg()),
    }
}
