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

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_trips.set(false),
            div {
                class: "modal wide",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.manage_trips}" }
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
                        button {
                            class: "icon-btn",
                            title: "{tr.delete_trip}",
                            onclick: move |_| actions::delete_trip(state, trip.id),
                            "✕"
                        }
                    }
                }
                div { class: "detail-actions",
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
