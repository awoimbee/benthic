use dioxus::prelude::*;

use crate::state::AppState;

/// One row in the dive list; either a trip header or a dive.
struct Row {
    key: String,
    class: &'static str,
    is_trip: bool,
    checked: bool,
    dive_id: Option<u32>,
    number: String,
    title: String,
    subtitle: String,
    trailing: String,
}

#[component]
pub fn DiveList() -> Element {
    let state = use_context::<AppState>();
    let mut selected = state.selected;
    let mut selection = state.selection;
    let log = (state.log)();
    let filter = (state.filter)();
    let prefs = (state.prefs)();
    let current = (state.selected)();
    let checked_ids = (state.selection)();

    let filtered: Vec<&benthic_core::Dive> = log
        .dives_sorted()
        .into_iter()
        .filter(|dive| filter.matches(dive, &log))
        .collect();

    let mut rows: Vec<Row> = Vec::new();
    let mut seen_trips: std::collections::HashSet<u32> = Default::default();
    for dive in &filtered {
        if let Some(trip_id) = dive.trip_id {
            if seen_trips.insert(trip_id) {
                if let Some(trip) = log.trip_by_id(trip_id) {
                    let count = filtered
                        .iter()
                        .filter(|d| d.trip_id == Some(trip_id))
                        .count();
                    let title = if trip.location.is_empty() {
                        format!("Trip #{trip_id}")
                    } else {
                        trip.location.clone()
                    };
                    rows.push(Row {
                        key: format!("trip-{trip_id}"),
                        class: "trip-row",
                        is_trip: true,
                        checked: false,
                        dive_id: None,
                        number: String::new(),
                        title,
                        subtitle: format!("{count} dives"),
                        trailing: String::new(),
                    });
                }
            }
        }
        rows.push(Row {
            key: dive.id.to_string(),
            class: if current == Some(dive.id) {
                "dive-row selected"
            } else {
                "dive-row"
            },
            is_trip: false,
            checked: checked_ids.contains(&dive.id),
            dive_id: Some(dive.id),
            number: if dive.number != 0 {
                dive.number.to_string()
            } else {
                "•".to_string()
            },
            title: crate::format::dive_title(dive, &log),
            subtitle: crate::format::dive_subtitle(dive, &prefs),
            trailing: dive
                .cylinders
                .first()
                .map(|c| c.gas.name())
                .unwrap_or_else(|| "—".to_string()),
        });
    }

    let total = log.dives.len();
    let shown = filtered.len();
    let empty = rows.is_empty();

    rsx! {
        aside { class: "dive-list",
            div { class: "pane-title", "Dives {shown}/{total}" }
            if empty {
                div { class: "empty-hint",
                    if total == 0 {
                        "No dives yet. Use Import to load a Subsurface log, or add a new dive."
                    } else {
                        "No dives match your search."
                    }
                }
            }
            ul {
                for row in rows {
                    li {
                        key: "{row.key}",
                        class: "{row.class}",
                        onclick: move |_| {
                            if let Some(id) = row.dive_id {
                                selected.set(Some(id));
                            }
                        },
                        if row.is_trip {
                            span { class: "trip-title", "{row.title}" }
                            span { class: "trip-subtitle", "{row.subtitle}" }
                        } else {
                            input {
                                r#type: "checkbox",
                                class: "row-check",
                                checked: row.checked,
                                onclick: move |evt| evt.stop_propagation(),
                                onchange: move |_| {
                                    if let Some(id) = row.dive_id {
                                        let mut set = selection.write();
                                        if set.contains(&id) {
                                            set.remove(&id);
                                        } else {
                                            set.insert(id);
                                        }
                                    }
                                },
                            }
                            span { class: "dive-number", "{row.number}" }
                            div { class: "dive-main",
                                span { class: "dive-title", "{row.title}" }
                                span { class: "dive-subtitle", "{row.subtitle}" }
                            }
                            span { class: "dive-gas", "{row.trailing}" }
                        }
                    }
                }
            }
        }
    }
}
