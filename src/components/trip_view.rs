//! The detail pane for one trip: where it was, when, its notes and its dives.
//!
//! Location, start date and notes are edited here and committed with a single
//! undoable `UpdateTrip`. The date range and totals are derived from the dives
//! in the trip, so they are always in sync. Selecting a trip in the list shows
//! this in the same pane as a dive.

use dioxus::prelude::*;

use benthic_core::units::{format_duration, Duration, Timestamp};
use benthic_core::{Command, DiveTrip};

use crate::i18n;
use crate::state::AppState;

#[component]
pub fn TripView(trip_id: u32) -> Element {
    let state = use_context::<AppState>();
    let mut show_trip = state.show_trip;
    let mut mobile_detail = state.mobile_detail;
    let log = (state.log)();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    let Some(trip) = log.trip_by_id(trip_id).cloned() else {
        return rsx! {};
    };

    let mut dives: Vec<benthic_core::Dive> = log
        .dives
        .iter()
        .filter(|dive| dive.trip_id == Some(trip_id))
        .cloned()
        .collect();
    dives.sort_by_key(|dive| dive.when);

    let title = if trip.location.is_empty() {
        i18n::t1(tr.trip_fallback, trip.id)
    } else {
        trip.location.clone()
    };

    let mut location = use_signal(|| trip.location.clone());
    let mut notes = use_signal(|| trip.notes.clone());
    let mut start_date = use_signal(|| trip.date.map(date_input_value).unwrap_or_default());

    // Derived from the dives: a trip spans a range, not a single day.
    let first = dives.first().map(|dive| dive.when);
    let last = dives.last().map(|dive| dive.when);
    let days = match (first, last) {
        (Some(start), Some(end)) => ((end - start) / 86_400 + 1).max(1),
        _ => 0,
    };
    let dates_text = match (first, last) {
        (Some(start), Some(end)) if start != end => {
            format!("{} – {}", prefs.date(start), prefs.date(end))
        }
        (Some(start), _) => prefs.date(start),
        _ => "—".to_string(),
    };
    let dates_value = if days > 1 {
        format!("{dates_text} · {}", i18n::t1(tr.days_count, days))
    } else {
        dates_text
    };
    let total_seconds: i32 = dives
        .iter()
        .filter_map(|dive| dive.duration())
        .map(|duration| duration.seconds)
        .sum();
    let total_time = if total_seconds > 0 {
        format_duration(Duration::new(total_seconds))
    } else {
        "—".to_string()
    };
    let max_depth = dives
        .iter()
        .filter_map(|dive| dive.max_depth())
        .max_by_key(|depth| depth.mm)
        .map(|depth| prefs.depth(depth))
        .unwrap_or_else(|| "—".to_string());

    let on_save = {
        let trip = trip.clone();
        move |_| {
            let after = DiveTrip {
                location: location().trim().to_string(),
                notes: notes(),
                date: parse_date_input(&start_date()),
                ..trip.clone()
            };
            state.dispatch(Command::UpdateTrip {
                before: trip.clone(),
                after,
            });
            state.set_status(tr.saved_trip);
        }
    };

    // Discard edits and restore the stored values.
    let on_revert = {
        let trip = trip.clone();
        move |_| {
            location.set(trip.location.clone());
            notes.set(trip.notes.clone());
            start_date.set(trip.date.map(date_input_value).unwrap_or_default());
        }
    };

    rsx! {
        section { class: "detail trip-view",
            header { class: "detail-head",
                div { class: "detail-title-row",
                    button {
                        class: "btn back-btn",
                        title: "{tr.back}",
                        onclick: move |_| mobile_detail.set(false),
                        "\u{2039} {tr.back}"
                    }
                    h1 { "{title}" }
                    div { class: "detail-actions",
                        button { class: "btn primary", onclick: on_save, "{tr.save}" }
                        button { class: "btn", onclick: on_revert, "{tr.cancel}" }
                    }
                }
                p { class: "muted", "{dates_value}" }
            }

            div { class: "facts planner-results",
                TripFact { label: tr.cat_dives, value: dives.len().to_string() }
                TripFact { label: tr.dates, value: dates_value.clone() }
                TripFact { label: tr.bottom_time, value: total_time }
                TripFact { label: tr.fact_max_depth, value: max_depth }
            }

            div { class: "section-title", "{tr.trip}" }
            div { class: "edit-form",
                label { class: "field-label", "{tr.location}"
                    input {
                        class: "field",
                        value: "{location()}",
                        oninput: move |evt| location.set(evt.value()),
                    }
                }
                label { class: "field-label", "{tr.start_date}"
                    input {
                        class: "field",
                        r#type: "date",
                        value: "{start_date()}",
                        oninput: move |evt| start_date.set(evt.value()),
                    }
                }
                label { class: "field-label full", "{tr.field_notes}"
                    textarea {
                        class: "field",
                        rows: "4",
                        value: "{notes()}",
                        oninput: move |evt| notes.set(evt.value()),
                    }
                }
            }

            div { class: "section-title", "{tr.cat_dives}" }
            if dives.is_empty() {
                p { class: "muted", "{tr.no_dives_in_trip}" }
            } else {
                div { class: "trip-dive-list",
                    for dive in dives {
                        {
                            let dive_id = dive.id;
                            let dive_title = crate::format::dive_title(&dive, &log);
                            let dive_subtitle = crate::format::dive_subtitle(&dive, &prefs);
                            rsx! {
                                button {
                                    key: "{dive_id}",
                                    class: "trip-dive-row",
                                    r#type: "button",
                                    onclick: move |_| {
                                        let mut selected = state.selected;
                                        selected.set(Some(dive_id));
                                        show_trip.set(None);
                                        mobile_detail.set(true);
                                    },
                                    span { class: "trip-dive-date", "{prefs.date(dive.when)}" }
                                    span { class: "trip-dive-main",
                                        span { class: "dive-title", "{dive_title}" }
                                        span { class: "dive-subtitle", "{dive_subtitle}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TripFact(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "fact",
            span { class: "fact-label", "{label}" }
            span { class: "fact-value", "{value}" }
        }
    }
}

/// `YYYY-MM-DD` for a `<input type="date">`, in UTC (matching the rest of the UI).
fn date_input_value(timestamp: Timestamp) -> String {
    benthic_core::datetime_local(timestamp)
        .chars()
        .take(10)
        .collect()
}

/// Parse a `<input type="date">` value into a UTC timestamp; empty means "no
/// date".
fn parse_date_input(value: &str) -> Option<Timestamp> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    benthic_core::parse_datetime_local(&format!("{value}T00:00"))
}
