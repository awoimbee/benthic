use dioxus::prelude::*;

use benthic_core::UnitSystem;

use crate::actions;
use crate::state::AppState;

/// A modal for display preferences and data recovery.
#[component]
pub fn PreferencesDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let mut show_prefs = state.show_prefs;
    let current = (state.prefs)().units;

    let backup_age = crate::storage::backup_age_secs();
    let has_backup = backup_age.is_some();
    let backup_label = match backup_age {
        Some(age) => format!("Automatic backup: {}", human_age(age)),
        None => "No automatic backup yet".to_string(),
    };

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
                div { class: "pref-group",
                    div { class: "field-label", "Data" }
                    div { class: "muted", "{backup_label}" }
                    div { class: "detail-actions",
                        button {
                            class: "btn",
                            disabled: !has_backup,
                            onclick: move |_| actions::restore_backup(state),
                            "Restore last backup"
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

fn human_age(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{} h ago", secs / 3600)
    } else {
        format!("{} d ago", secs / 86_400)
    }
}

fn units_description(units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => "m, °C, bar, kg, L",
        UnitSystem::Imperial => "ft, °F, psi, lbs, cuft",
    }
}
