//! Batch editing for the ticked dives.
//!
//! Every field is optional: leaving it blank keeps the dives' current value,
//! so a batch edit only touches what the diver explicitly fills in.

use dioxus::prelude::*;

use crate::actions::{self, BatchEdit};
use crate::i18n;
use crate::state::AppState;

#[component]
pub fn BatchEditDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show = state.show_batch_edit;
    let log = (state.log)();
    let tr = i18n::strings((state.prefs)().language);
    let count = (state.selection)().len();

    let mut trip = use_signal(String::new);
    let mut buddy = use_signal(String::new);
    let mut diveguide = use_signal(String::new);
    let mut suit = use_signal(String::new);
    let mut rating = use_signal(String::new);
    let mut add_tags = use_signal(String::new);
    let mut remove_tags = use_signal(String::new);

    if count == 0 {
        return rsx! {};
    }

    let trips: Vec<(u32, String)> = log
        .trips
        .iter()
        .map(|trip| {
            let label = if trip.location.is_empty() {
                i18n::t1(tr.trip_fallback, trip.id)
            } else {
                trip.location.clone()
            };
            (trip.id, label)
        })
        .collect();

    let on_apply = move |_| {
        let trip_id = match trip().as_str() {
            "" => None,
            "none" => Some(None),
            value => value.parse::<u32>().ok().map(Some),
        };
        let edit = BatchEdit {
            trip_id,
            buddy: non_empty(&buddy()),
            diveguide: non_empty(&diveguide()),
            suit: non_empty(&suit()),
            rating: (!rating().is_empty()).then(|| rating().parse::<u8>().unwrap_or(0)),
            add_tags: parse_tags(&add_tags()),
            remove_tags: parse_tags(&remove_tags()),
        };
        actions::apply_batch_edit(state, edit);
    };

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show.set(false),
            div {
                class: "modal wide",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{i18n::t1(tr.batch_edit_title, count)}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { {i18n::t1(tr.batch_edit_title, count)} }
                p { class: "muted", "{tr.batch_edit_hint}" }

                div { class: "edit-form",
                    label { class: "field-label", "{tr.field_trip}"
                        select {
                            class: "field",
                            value: "{trip}",
                            onchange: move |evt| trip.set(evt.value()),
                            option { value: "", "{tr.batch_no_change}" }
                            option { value: "none", "{tr.no_trip}" }
                            for (id, label) in trips {
                                option { key: "{id}", value: "{id}", "{label}" }
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_rating}"
                        select {
                            class: "field",
                            value: "{rating}",
                            onchange: move |evt| rating.set(evt.value()),
                            option { value: "", "{tr.batch_no_change}" }
                            option { value: "0", "{tr.batch_clear_rating}" }
                            for stars in 1..=5u8 {
                                option { key: "{stars}", value: "{stars}", "{stars} ★" }
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_buddy}"
                        input {
                            class: "field",
                            value: "{buddy}",
                            oninput: move |evt| buddy.set(evt.value()),
                        }
                    }
                    label { class: "field-label", "{tr.field_dive_master}"
                        input {
                            class: "field",
                            value: "{diveguide}",
                            oninput: move |evt| diveguide.set(evt.value()),
                        }
                    }
                    label { class: "field-label", "{tr.field_suit}"
                        input {
                            class: "field",
                            value: "{suit}",
                            oninput: move |evt| suit.set(evt.value()),
                        }
                    }
                    label { class: "field-label full", "{tr.batch_add_tags}"
                        input {
                            class: "field",
                            value: "{add_tags}",
                            oninput: move |evt| add_tags.set(evt.value()),
                        }
                    }
                    label { class: "field-label full", "{tr.batch_remove_tags}"
                        input {
                            class: "field",
                            value: "{remove_tags}",
                            oninput: move |evt| remove_tags.set(evt.value()),
                        }
                    }
                }

                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        r#type: "button",
                        onclick: on_apply,
                        {i18n::t1(tr.batch_apply, count)}
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| show.set(false),
                        "{tr.cancel}"
                    }
                }
            }
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect();
    tags.sort();
    tags.dedup();
    tags
}
