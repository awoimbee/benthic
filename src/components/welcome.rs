use dioxus::prelude::*;

use benthic_core::UnitSystem;

use crate::i18n;
use crate::state::AppState;

/// The demo log shipped inside the binary, so a first-time user can see the
/// app with real data without hunting for a file.
const DEMO_LOG: &str = include_str!("../../dives/demo.ssrf");

/// A first-run welcome that gives a brand-new log somewhere to go: load the
/// demo, pick units, and learn the local-first promise. It only appears while
/// the log is empty and has never been dismissed.
#[component]
pub fn WelcomeDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let mut status = state.status;
    let t = i18n::strings(prefs().language);

    let load_demo = move |_| {
        match benthic_core::io::parse_auto(DEMO_LOG) {
            Ok(parsed) => {
                let mut log = state.log;
                let mut selected = state.selected;
                if let Some(first) = parsed.dives_recent_first().first().map(|d| d.id) {
                    selected.set(Some(first));
                }
                let count = parsed.dives.len();
                log.set(parsed);
                status.set(i18n::t1(t.loaded_demo, count));
            }
            Err(error) => status.set(format!("Could not load the demo: {error}")),
        }
        prefs.write().seen_welcome = true;
    };

    let dismiss = move |_| {
        prefs.write().seen_welcome = true;
    };

    rsx! {
        div { class: "modal-backdrop welcome-backdrop",
            div { class: "modal welcome",
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
