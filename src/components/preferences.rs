use dioxus::prelude::*;

use benthic_core::UnitSystem;

use crate::state::AppState;

/// A modal for display preferences.
#[component]
pub fn PreferencesDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let mut show_prefs = state.show_prefs;
    let current = (state.prefs)().units;

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_prefs.set(false),
            div {
                class: "modal",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "Preferences" }
                div { class: "pref-group",
                    div { class: "field-label", "Units" }
                    for units in UnitSystem::ALL {
                        label {
                            key: "{units.label()}",
                            class: "radio",
                            input {
                                r#type: "radio",
                                name: "units",
                                checked: current == units,
                                onchange: move |_| prefs.write().units = units,
                            }
                            span { "{units.label()} — {units_description(units)}" }
                        }
                    }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        onclick: move |_| show_prefs.set(false),
                        "Done"
                    }
                }
            }
        }
    }
}

fn units_description(units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => "m, °C, bar, kg, L",
        UnitSystem::Imperial => "ft, °F, psi, lbs, cuft",
    }
}
