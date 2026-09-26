use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

struct TripRow {
    id: u32,
    name: String,
    count: usize,
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
                        format!("Trip #{}", other.id)
                    } else {
                        other.location.clone()
                    };
                    (other.id, name)
                })
                .collect();
            TripRow {
                id: trip.id,
                name: trip.location.clone(),
                count,
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
                h2 { "Trips" }
                if empty {
                    div { class: "muted", "No trips yet. Select dives and choose 'New trip' to create one." }
                }
                for trip in trips {
                    div { key: "{trip.id}", class: "trip-edit-row",
                        input {
                            class: "field",
                            placeholder: "Trip name",
                            value: "{trip.name}",
                            onchange: move |evt| actions::rename_trip(state, trip.id, &evt.value()),
                        }
                        span { class: "muted trip-edit-meta", "{trip.count} dives {trip.date}" }
                        select {
                            class: "field",
                            value: "",
                            onchange: move |evt| {
                                if let Ok(target) = evt.value().parse::<u32>() {
                                    actions::merge_trips(state, target, trip.id);
                                }
                            },
                            option { value: "", "Merge into…" }
                            for (id, name) in trip.others.iter() {
                                option { key: "{id}", value: "{id}", "{name}" }
                            }
                        }
                        button {
                            class: "icon-btn",
                            title: "Delete trip (dives are kept)",
                            onclick: move |_| actions::delete_trip(state, trip.id),
                            "✕"
                        }
                    }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        onclick: move |_| show_trips.set(false),
                        "Done"
                    }
                }
            }
        }
    }
}
