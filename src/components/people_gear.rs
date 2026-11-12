//! The global people & gear manager.
//!
//! Entries are ordered by most recent use, so the things a diver reaches for
//! on the next trip are at the top. Usage is derived from the dives, never
//! stored, which keeps the list correct after imports and edits.

use dioxus::prelude::*;

use benthic_core::{GearItem, GearKind, Person, Usage};

use crate::actions;
use crate::i18n;
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    People,
    Gear,
}

#[component]
pub fn PeopleGearDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_people = state.show_people;
    let log = (state.log)();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    let mut tab = use_signal(|| Tab::People);
    let mut new_person = use_signal(String::new);
    let mut new_gear_name = use_signal(String::new);
    let mut new_gear_kind = use_signal(|| GearKind::Wetsuit);

    let person_usage = log.person_usage();
    let mut people: Vec<(Person, Usage)> = log
        .people
        .iter()
        .map(|person| (person.clone(), person_usage[&person.id]))
        .collect();
    people.sort_by(|(a, a_usage), (b, b_usage)| {
        b_usage
            .last_used
            .cmp(&a_usage.last_used)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let gear_usage = log.gear_usage();
    let mut gear: Vec<(GearItem, Usage)> = log
        .gear
        .iter()
        .map(|item| (item.clone(), gear_usage[&item.id]))
        .collect();
    gear.sort_by(|(a, a_usage), (b, b_usage)| {
        b_usage
            .last_used
            .cmp(&a_usage.last_used)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let active = (tab)();
    let people_empty = people.is_empty();
    let gear_empty = gear.is_empty();
    let title = tr.people_gear;

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_people.set(false),
            div {
                class: "modal wide",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.people_gear}" }
                div { class: "dc-tabs",
                    button {
                        class: if active == Tab::People { "dc-tab active" } else { "dc-tab" },
                        onclick: move |_| tab.set(Tab::People),
                        "{tr.tab_people}"
                    }
                    button {
                        class: if active == Tab::Gear { "dc-tab active" } else { "dc-tab" },
                        onclick: move |_| tab.set(Tab::Gear),
                        "{tr.tab_gear}"
                    }
                }

                if active == Tab::People {
                    div { class: "catalog-add",
                        input {
                            class: "field",
                            placeholder: "{tr.person_name_placeholder}",
                            value: "{new_person}",
                            oninput: move |evt| new_person.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    actions::add_person(state, &new_person());
                                    new_person.set(String::new());
                                }
                            },
                        }
                        button {
                            class: "btn primary",
                            r#type: "button",
                            onclick: move |_| {
                                actions::add_person(state, &new_person());
                                new_person.set(String::new());
                            },
                            "{tr.add_person}"
                        }
                    }
                    if people_empty {
                        div { class: "muted", "{tr.people_empty}" }
                    }
                    for (person, usage) in people {
                        div { key: "{person.id}", class: "catalog-row",
                            input {
                                class: "field",
                                value: "{person.name}",
                                onchange: move |evt| actions::rename_person(state, person.id, &evt.value()),
                            }
                            span { class: "muted catalog-usage", "{usage_text(&prefs, tr, usage)}" }
                            button {
                                class: "icon-btn",
                                title: "{tr.delete}",
                                onclick: move |_| actions::delete_person(state, person.id),
                                "✕"
                            }
                        }
                    }
                } else {
                    div { class: "catalog-add",
                        input {
                            class: "field",
                            placeholder: "{tr.gear_name_placeholder}",
                            value: "{new_gear_name}",
                            oninput: move |evt| new_gear_name.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    actions::add_gear(state, (new_gear_kind)(), &new_gear_name());
                                    new_gear_name.set(String::new());
                                }
                            },
                        }
                        select {
                            class: "field catalog-kind-select",
                            value: "{gear_kind_index((new_gear_kind)())}",
                            onchange: move |evt| {
                                new_gear_kind.set(kind_from_index(
                                    evt.value().parse::<usize>().unwrap_or(0),
                                ));
                            },
                            for kind in GearKind::ALL {
                                option {
                                    key: "{gear_kind_index(kind)}",
                                    value: "{gear_kind_index(kind)}",
                                    selected: (new_gear_kind)() == kind,
                                    "{i18n::gear_kind(tr, kind)}"
                                }
                            }
                        }
                        button {
                            class: "btn primary",
                            r#type: "button",
                            onclick: move |_| {
                                actions::add_gear(state, (new_gear_kind)(), &new_gear_name());
                                new_gear_name.set(String::new());
                            },
                            "{tr.add_gear}"
                        }
                    }
                    if gear_empty {
                        div { class: "muted", "{tr.gear_empty}" }
                    }
                    for (item, usage) in gear {
                        div { key: "{item.id}", class: "catalog-row",
                            input {
                                class: "field",
                                value: "{item.name}",
                                onchange: move |evt| actions::rename_gear(state, item.id, &evt.value()),
                            }
                            select {
                                class: "field catalog-kind-select",
                                value: "{gear_kind_index(item.kind)}",
                                onchange: move |evt| {
                                    actions::update_gear_kind(
                                        state,
                                        item.id,
                                        kind_from_index(evt.value().parse::<usize>().unwrap_or(0)),
                                    );
                                },
                                for kind in GearKind::ALL {
                                    option {
                                        key: "{gear_kind_index(kind)}",
                                        value: "{gear_kind_index(kind)}",
                                        selected: item.kind == kind,
                                        "{i18n::gear_kind(tr, kind)}"
                                    }
                                }
                            }
                            span { class: "muted catalog-usage", "{usage_text(&prefs, tr, usage)}" }
                            button {
                                class: "icon-btn",
                                title: "{tr.delete}",
                                onclick: move |_| actions::delete_gear(state, item.id),
                                "✕"
                            }
                        }
                    }
                }

                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        onclick: move |_| show_people.set(false),
                        "{tr.done}"
                    }
                }
            }
        }
    }
}

fn usage_text(prefs: &benthic_core::Preferences, tr: &i18n::Strings, usage: Usage) -> String {
    match usage.last_used {
        Some(when) => i18n::t2(tr.last_used, prefs.date(when), usage.dives),
        None => tr.never_used.to_string(),
    }
}

fn gear_kind_index(kind: GearKind) -> usize {
    GearKind::ALL
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(0)
}

fn kind_from_index(index: usize) -> GearKind {
    GearKind::ALL.get(index).copied().unwrap_or_default()
}
