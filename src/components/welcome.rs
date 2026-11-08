use dioxus::prelude::*;

use benthic_core::UnitSystem;

use crate::actions;
use crate::i18n;
use crate::state::AppState;

/// A first-run welcome that gives a brand-new log somewhere to go: load the
/// example logbook, pick units, and learn the local-first promise. It only
/// appears while the log is empty and has never been dismissed.
#[component]
pub fn WelcomeDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let t = i18n::strings(prefs().language);

    let load_demo = move |_| {
        actions::open_example_log(state);
        prefs.write().seen_welcome = true;
    };

    let dismiss = move |_| {
        prefs.write().seen_welcome = true;
    };

    rsx! {
        div { class: "modal-backdrop welcome-backdrop",
            div { class: "modal welcome",
                role: "dialog",
            tabindex: "-1",
            autofocus: true,
                aria_modal: "true",
                aria_label: "{t.welcome_title}",
                h2 { "{t.welcome_title}" }
                p { class: "welcome-lead", "{t.welcome_lead}" }
                ul { class: "welcome-points",
                    li { "{t.welcome_point_1}" }
                    li { "{t.welcome_point_2}" }
                    li { "{t.welcome_point_3}" }
                }
                div { class: "pref-group",
                    div { class: "field-label", "{t.preferred_units}" }
                    for units in UnitSystem::ALL {
                        label {
                            key: "{units.label()}",
                            class: "radio",
                            input {
                                r#type: "radio",
                                name: "welcome-units",
                                checked: prefs().units == units,
                                onchange: move |_| prefs.write().units = units,
                            }
                            span {
                                "{unit_label(t, units)} — {units_description(units)}"
                            }
                        }
                    }
                }
                div { class: "detail-actions welcome-actions",
                    button { class: "btn primary", onclick: load_demo, "{t.try_demo}" }
                    button { class: "btn", onclick: dismiss, "{t.start_empty}" }
                }
            }
        }
    }
}

/// The localized name for a unit system.
fn unit_label(t: &'static i18n::Strings, units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => t.unit_metric,
        UnitSystem::Imperial => t.unit_imperial,
    }
}

fn units_description(units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => "m, °C, bar, kg, L",
        UnitSystem::Imperial => "ft, °F, psi, lbs, cuft",
    }
}
