use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

/// One row in the dive list; either a trip header or a dive.
struct Row {
    key: String,
    class: String,
    is_trip: bool,
    collapsed: bool,
    checked: bool,
    dive_id: Option<u32>,
    trip_id: Option<u32>,
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
    let mut renaming_trip = use_signal(|| None::<u32>);
    let mut rename_text = use_signal(String::new);
    let renaming = (renaming_trip)();
    // Trip ids whose dives are rolled up.
    let mut collapsed_trips = use_signal(std::collections::HashSet::<u32>::new);
    let collapsed = (collapsed_trips)();

    let filtered: Vec<&benthic_core::Dive> = log
        .dives_recent_first()
        .into_iter()
        .filter(|dive| filter.matches(dive, &log))
        .collect();

    let mut rows: Vec<Row> = Vec::new();
    let mut seen_trips: std::collections::HashSet<u32> = Default::default();
    for (index, dive) in filtered.iter().enumerate() {
        // The last dive of a trip is marked so the group reads as one block
        // and is visibly separated from the dives that follow.
        let trip_end = dive.trip_id.is_some()
            && filtered.get(index + 1).map(|next| next.trip_id) != Some(dive.trip_id);
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
                        class: "trip-row".to_string(),
                        is_trip: true,
                        collapsed: collapsed.contains(&trip_id),
                        checked: false,
                        dive_id: None,
                        trip_id: Some(trip_id),
                        number: String::new(),
                        title,
                        subtitle: format!("{count} dives"),
                        trailing: String::new(),
                    });
                }
            }
        }
        if dive
            .trip_id
            .is_some_and(|trip_id| collapsed.contains(&trip_id))
        {
            continue;
        }
        rows.push(Row {
            key: dive.id.to_string(),
            class: {
                let mut class = if current == Some(dive.id) {
                    String::from("dive-row selected")
                } else {
                    String::from("dive-row")
                };
                if dive.trip_id.is_some() {
                    class.push_str(" in-trip");
                }
                if trip_end {
                    class.push_str(" trip-end");
                }
                class
            },
            is_trip: false,
            collapsed: false,
            checked: checked_ids.contains(&dive.id),
            dive_id: Some(dive.id),
            trip_id: None,
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
                            } else if let Some(trip_id) = row.trip_id {
                                let mut set = collapsed_trips.write();
                                if !set.remove(&trip_id) {
                                    set.insert(trip_id);
                                }
                            }
                        },
                        if row.is_trip {
                            if let Some(trip_id) = row.trip_id {
                                if renaming == Some(trip_id) {
                                    input {
                                        class: "trip-rename",
                                        value: "{rename_text()}",
                                        onclick: move |evt| evt.stop_propagation(),
                                        oninput: move |evt| rename_text.set(evt.value()),
                                        onkeydown: move |evt| {
                                            if evt.key() == Key::Enter {
                                                actions::rename_trip(state, trip_id, &rename_text());
                                                renaming_trip.set(None);
                                            } else if evt.key() == Key::Escape {
                                                renaming_trip.set(None);
                                            }
                                        },
                                        onblur: move |_| {
                                            if renaming_trip() == Some(trip_id) {
                                                actions::rename_trip(state, trip_id, &rename_text());
                                                renaming_trip.set(None);
                                            }
                                        },
                                    }
                                } else {
                                    span { class: "trip-caret",
                                        if row.collapsed { "\u{25B8}" } else { "\u{25BE}" }
                                    }
                                    span { class: "trip-title", "{row.title}" }
                                    span { class: "trip-subtitle", "{row.subtitle}" }
                                    div { class: "trip-actions",
                                        button {
                                            class: "icon-btn",
                                            title: "Rename trip",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                let name = (state.log)()
                                                    .trip_by_id(trip_id)
                                                    .map(|t| t.location.clone())
                                                    .unwrap_or_default();
                                                rename_text.set(name);
                                                renaming_trip.set(Some(trip_id));
                                            },
                                            "✎"
                                        }
                                        button {
                                            class: "icon-btn",
                                            title: "Delete trip (dives are kept)",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                actions::delete_trip(state, trip_id);
                                            },
                                            "✕"
                                        }
                                    }
                                }
                            }
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
