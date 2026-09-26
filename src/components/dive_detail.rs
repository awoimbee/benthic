use dioxus::prelude::*;

use benthic_core::units::{format_duration, format_timestamp_utc};
use benthic_core::{Command, Dive, DiveSite, Location};

use crate::actions;
use crate::components::DiveProfile;
use crate::state::AppState;

#[component]
pub fn DiveDetail() -> Element {
    let state = use_context::<AppState>();
    let selected = (state.selected)();
    let log = (state.log)();

    let Some(dive) = selected.and_then(|id| log.dive_by_id(id).cloned()) else {
        return rsx! {
            section { class: "detail",
                div { class: "empty-hint", "Select a dive to see its details." }
            }
        };
    };

    // Remount on selection change so the edit form resets.
    rsx! { DiveDetailInner { key: "{dive.id}", dive } }
}

/// The editable subset of a dive, kept separate from the full [`Dive`] so that
/// typing does not clone the (potentially huge) sample arrays.
#[derive(Clone, PartialEq)]
struct DiveForm {
    number: i32,
    buddy: String,
    divemaster: String,
    suit: String,
    notes: String,
    rating: u8,
    tags: String,
    trip_id: Option<u32>,
    site_name: String,
    site_gps: String,
}

impl DiveForm {
    fn from_dive(dive: &Dive, site: Option<&DiveSite>) -> Self {
        Self {
            number: dive.number,
            buddy: dive.buddy.clone(),
            divemaster: dive.diveguide.clone(),
            suit: dive.suit.clone(),
            notes: dive.notes.clone(),
            rating: dive.rating,
            tags: dive.tags.join(", "),
            trip_id: dive.trip_id,
            site_name: site.map(|s| s.name.clone()).unwrap_or_default(),
            site_gps: site
                .and_then(|s| s.location)
                .map(|l| format!("{:.6}, {:.6}", l.lat, l.lon))
                .unwrap_or_default(),
        }
    }
}

#[component]
fn DiveDetailInner(dive: Dive) -> Element {
    let state = use_context::<AppState>();
    let log = (state.log)();
    let site = dive.site_id.and_then(|id| log.site_by_uuid(id)).cloned();

    let mut editing = use_signal(|| false);
    let mut form = use_signal(|| DiveForm::from_dive(&dive, site.as_ref()));
    let confirm_delete = use_signal(|| false);

    let dive_id = dive.id;
    let is_editing = (editing)();
    let is_confirming = (confirm_delete)();

    // --- actions ---------------------------------------------------------

    let on_save = {
        let dive = dive.clone();
        let site = site.clone();
        let log = log.clone();
        move |_| {
            let form = form;
            let mut editing = editing;
            let f = form();
            let mut after = dive.clone();
            after.number = f.number;
            after.buddy = f.buddy;
            after.diveguide = f.divemaster;
            after.suit = f.suit;
            after.notes = f.notes;
            after.rating = f.rating;
            after.tags = parse_tags(&f.tags);
            after.trip_id = f.trip_id;

            let mut commands: Vec<Command> = Vec::new();
            let name = f.site_name.trim().to_string();
            let location = parse_location(&f.site_gps);
            let site_id = match (after.site_id, site.clone()) {
                (Some(id), Some(existing)) => {
                    let mut updated = existing.clone();
                    updated.name = name;
                    updated.location = location;
                    commands.push(Command::UpdateSite {
                        before: existing,
                        after: updated,
                    });
                    Some(id)
                }
                _ if !name.is_empty() || location.is_some() => {
                    let uuid = log.next_site_uuid();
                    commands.push(Command::AddSite {
                        site: DiveSite {
                            uuid,
                            name,
                            location,
                            ..Default::default()
                        },
                    });
                    Some(uuid)
                }
                _ => None,
            };
            after.site_id = site_id;
            commands.push(Command::UpdateDive {
                before: dive.clone(),
                after,
            });
            state.dispatch_all("Edit dive", commands);
            editing.set(false);
            state.set_status("Saved dive");
        }
    };

    let on_cancel = {
        let dive = dive.clone();
        let site = site.clone();
        move |_| {
            let mut form = form;
            let mut editing = editing;
            form.set(DiveForm::from_dive(&dive, site.as_ref()));
            editing.set(false);
        }
    };

    let on_delete = move |_| {
        let mut confirm = confirm_delete;
        if !confirm() {
            confirm.set(true);
            return;
        }
        actions::delete_dive(state, dive_id);
    };

    let on_duplicate = move |_| actions::duplicate_dive(state, dive_id);

    // --- read-only values -------------------------------------------------

    let title = crate::format::dive_title(&dive, &log);
    let site_name = site
        .as_ref()
        .map(|s| s.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "—".to_string());
    let trip_name = dive
        .trip_id
        .and_then(|id| log.trip_by_id(id))
        .map(|t| {
            if t.location.is_empty() {
                format!("Trip #{}", t.id)
            } else {
                t.location.clone()
            }
        })
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

    // --- edit-form values -------------------------------------------------

    let f = (form)();
    let stars: Vec<(u8, &'static str)> = (1..=5u8)
        .map(|s| (s, if f.rating >= s { "star active" } else { "star" }))
        .collect();
    let trips: Vec<(u32, String)> = log
        .trips
        .iter()
        .map(|t| {
            let label = if t.location.is_empty() {
                format!("Trip #{}", t.id)
            } else {
                t.location.clone()
            };
            (t.id, label)
        })
        .collect();
    let current_trip = f.trip_id.map(|id| id.to_string()).unwrap_or_default();

    let cylinders: Vec<CylinderRow> = dive.cylinders.iter().map(cylinder_row).collect();
    let weights: Vec<WeightRow> = dive.weights.iter().map(weight_row).collect();

    rsx! {
        section { class: "detail",
            header { class: "detail-head",
                div { class: "detail-title-row",
                    h1 { "{title}" }
                    div { class: "detail-actions",
                        if is_editing {
                            button { class: "btn primary", onclick: on_save, "Save" }
                            button { class: "btn", onclick: on_cancel, "Cancel" }
                        } else {
                            button { class: "btn", onclick: move |_| editing.set(true), "Edit" }
                            button { class: "btn", onclick: on_duplicate, "Duplicate" }
                            button {
                                class: if is_confirming { "btn danger" } else { "btn" },
                                onclick: on_delete,
                                if is_confirming { "Confirm delete" } else { "Delete" }
                            }
                        }
                    }
                }
                p { class: "muted", "{when} · {site_name} · {trip_name}" }
            }

            DiveProfile { dive: dive.clone() }

            if is_editing {
                div { class: "edit-form",
                    label { class: "field-label", "Number"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{f.number}",
                            oninput: move |evt| form.write().number = evt.value().parse().unwrap_or(0),
                        }
                    }
                    label { class: "field-label", "Rating"
                        div { class: "stars",
                            for (star, class) in stars {
                                button {
                                    key: "{star}",
                                    class: "{class}",
                                    onclick: move |_| form.write().rating = star,
                                    "★"
                                }
                            }
                        }
                    }
                    label { class: "field-label", "Buddy"
                        input {
                            class: "field",
                            value: "{f.buddy}",
                            oninput: move |evt| form.write().buddy = evt.value(),
                        }
                    }
                    label { class: "field-label", "Dive master"
                        input {
                            class: "field",
                            value: "{f.divemaster}",
                            oninput: move |evt| form.write().divemaster = evt.value(),
                        }
                    }
                    label { class: "field-label", "Suit"
                        input {
                            class: "field",
                            value: "{f.suit}",
                            oninput: move |evt| form.write().suit = evt.value(),
                        }
                    }
                    label { class: "field-label", "Trip"
                        select {
                            class: "field",
                            value: "{current_trip}",
                            onchange: move |evt| {
                                let value = evt.value();
                                form.write().trip_id = if value.is_empty() { None } else { value.parse().ok() };
                            },
                            option { value: "", "No trip" }
                            for (id, label) in trips {
                                option { key: "{id}", value: "{id}", "{label}" }
                            }
                        }
                    }
                    label { class: "field-label", "Site name"
                        input {
                            class: "field",
                            value: "{f.site_name}",
                            oninput: move |evt| form.write().site_name = evt.value(),
                        }
                    }
                    label { class: "field-label", "Site GPS (lat, lon)"
                        input {
                            class: "field",
                            placeholder: "28.572100, 34.536700",
                            value: "{f.site_gps}",
                            oninput: move |evt| form.write().site_gps = evt.value(),
                        }
                    }
                    label { class: "field-label full", "Tags (comma separated)"
                        input {
                            class: "field",
                            value: "{f.tags}",
                            oninput: move |evt| form.write().tags = evt.value(),
                        }
                    }
                    label { class: "field-label full", "Notes"
                        textarea {
                            class: "field",
                            rows: "6",
                            value: "{f.notes}",
                            oninput: move |evt| form.write().notes = evt.value(),
                        }
                    }
                }
            } else {
                div { class: "facts",
                    Fact { label: "Duration", value: duration }
                    Fact { label: "Max depth", value: max_depth }
                    Fact { label: "Water temp", value: water_temp }
                    Fact { label: "Air temp", value: air_temp }
                    Fact { label: "Computer", value: computer }
                    Fact { label: "Buddy", value: dive.buddy.clone() }
                    Fact { label: "Dive master", value: dive.diveguide.clone() }
                    Fact { label: "Suit", value: dive.suit.clone() }
                }

                if !dive.tags.is_empty() {
                    div { class: "tags",
                        for tag in dive.tags.clone() {
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

                if !dive.notes.is_empty() {
                    div { class: "section-title", "Notes" }
                    p { class: "notes", "{dive.notes}" }
                }
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

fn cylinder_row(cyl: &benthic_core::Cylinder) -> CylinderRow {
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

fn weight_row(ws: &benthic_core::WeightSystem) -> WeightRow {
    WeightRow {
        description: if ws.description.is_empty() {
            "—".to_string()
        } else {
            ws.description.clone()
        },
        weight: format!("{:.2} kg", ws.weight.kg()),
    }
}

/// Parse a comma-separated tag list into a sorted, de-duplicated vector.
fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = text
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Parse "lat, lon" or "lat lon" into a valid [`Location`].
fn parse_location(text: &str) -> Option<Location> {
    let cleaned = text.replace(',', " ");
    let mut parts = cleaned.split_whitespace();
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    let location = Location::new(lat, lon);
    location.is_valid().then_some(location)
}
