use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::AppState;

struct TripRow {
    id: u32,
    name: String,
    dives: String,
    date: String,
    others: Vec<(u32, String)>,
}

/// A modal for renaming, merging and deleting trips.
#[component]
pub fn TripsDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_trips = state.show_trips;
    let log = (state.log)();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    let trips: Vec<TripRow> = log
        .trips
        .iter()
        .map(|trip| {
            let count = log
                .dives
                .iter()
                .filter(|d| d.trip_id == Some(trip.id))
                .count();
            let date = trip.date.map(|d| prefs.date(d)).unwrap_or_default();
            let others = log
                .trips
                .iter()
                .filter(|other| other.id != trip.id)
                .map(|other| {
                    let name = if other.location.is_empty() {
                        i18n::t1(tr.trip_fallback, other.id)
                    } else {
                        other.location.clone()
                    };
                    (other.id, name)
                })
                .collect();
            TripRow {
                id: trip.id,
                name: trip.location.clone(),
                dives: i18n::t1(tr.n_dives, count),
                date,
                others,
            }
        })
        .collect();

    let empty = trips.is_empty();
    let selected_count = (state.selection)().len();
    let mut new_trip_name = use_signal(String::new);

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_trips.set(false),
            div {
                class: "modal wide",
                role: "dialog",
            tabindex: "-1",
            autofocus: true,
                aria_modal: "true",
                aria_label: "{tr.manage_trips}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.manage_trips}" }
                div { class: "trip-edit-row trip-new-row",
                    input {
                        class: "field",
                        placeholder: "{tr.trip_name_placeholder}",
                        value: "{new_trip_name}",
                        oninput: move |evt| new_trip_name.set(evt.value()),
                    }
                    button {
                        class: "btn primary",
                        onclick: move |_| {
                            actions::create_trip_with_selection(state, &new_trip_name());
                            new_trip_name.set(String::new());
                        },
                        "{tr.create_trip}"
                    }
                    if selected_count > 0 {
                        span { class: "muted trip-edit-meta",
                            "{selected_count}"
                        }
                    }
                }
                if empty {
                    div { class: "muted", "{tr.trips_empty}" }
                }
                for trip in trips {
                    div { key: "{trip.id}", class: "trip-edit-row",
                        input {
                            class: "field",
                            placeholder: "{tr.trip_name_placeholder}",
                            value: "{trip.name}",
                            onchange: move |evt| actions::rename_trip(state, trip.id, &evt.value()),
                        }
                        span { class: "muted trip-edit-meta", "{trip.dives} {trip.date}" }
                        select {
                            class: "field",
                            value: "",
                            onchange: move |evt| {
                                if let Ok(target) = evt.value().parse::<u32>() {
                                    actions::merge_trips(state, target, trip.id);
                                }
                            },
                            option { value: "", "{tr.merge_into}" }
                            for (id, name) in trip.others.iter() {
                                option { key: "{id}", value: "{id}", "{name}" }
                            }
                        }
                        if selected_count > 0 {
                            button {
                                class: "btn trip-move-btn",
                                onclick: move |_| {
                                    actions::move_selection_to_trip(state, Some(trip.id));
                                },
                                "{tr.move_selected_here}"
                            }
                        }
                        button {
                            class: "icon-btn",
                            title: "{tr.delete_trip}",
                            onclick: move |_| actions::delete_trip(state, trip.id),
                            "✕"
                        }
                    }
                }
                div { class: "detail-actions",
                    if selected_count > 0 {
                        button {
                            class: "btn",
                            onclick: move |_| actions::move_selection_to_trip(state, None),
                            "{tr.detach_from_trip}"
                        }
                    }
                    button {
                        class: "btn primary",
                        onclick: move |_| show_trips.set(false),
                        "{tr.done}"
                    }
                }
            }
        }
    }
}
