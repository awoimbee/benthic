use dioxus::prelude::*;

use benthic_core::{DateFormat, Salinity, TimeFormat, UnitSystem};

use crate::actions;
use crate::state::AppState;

/// A modal for display preferences and data recovery.
#[component]
pub fn PreferencesDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let mut show_prefs = state.show_prefs;
    let current = (state.prefs)().units;
    let current_date = (state.prefs)().date_format;
    let current_time = (state.prefs)().time_format;
    let date_index = DateFormat::ALL
        .iter()
        .position(|f| *f == current_date)
        .unwrap_or(0);
    let time_index = TimeFormat::ALL
        .iter()
        .position(|f| *f == current_time)
        .unwrap_or(0);
    let salinity_index = Salinity::ALL
        .iter()
        .position(|s| *s == (state.prefs)().default_salinity)
        .unwrap_or(2);
    let cylinder_value = (state.prefs)()
        .default_cylinder
        .map(|i| i.to_string())
        .unwrap_or_default();

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
                    div { class: "field-label", "Date format" }
                    select {
                        class: "field",
                        value: "{date_index}",
                        onchange: move |evt| {
                            let index = evt.value().parse::<usize>().unwrap_or(0);
                            prefs.write().date_format = DateFormat::ALL[index.min(DateFormat::ALL.len() - 1)];
                        },
                        for (index, format) in DateFormat::ALL.iter().enumerate() {
                            option { key: "{index}", value: "{index}", "{format.label()}" }
                        }
                    }
                    div { class: "field-label", "Time format" }
                    select {
                        class: "field",
                        value: "{time_index}",
                        onchange: move |evt| {
                            let index = evt.value().parse::<usize>().unwrap_or(0);
                            prefs.write().time_format = TimeFormat::ALL[index.min(TimeFormat::ALL.len() - 1)];
                        },
                        for (index, format) in TimeFormat::ALL.iter().enumerate() {
                            option { key: "{index}", value: "{index}", "{format.label()}" }
                        }
                    }
                }
                div { class: "pref-group",
                    div { class: "field-label", "Default salinity" }
                    select {
                        class: "field",
                        value: "{salinity_index}",
                        onchange: move |evt| {
                            let index = evt.value().parse::<usize>().unwrap_or(0);
                            prefs.write().default_salinity = Salinity::ALL[index.min(Salinity::ALL.len() - 1)];
                        },
                        for (index, salinity) in Salinity::ALL.iter().enumerate() {
                            option { key: "{index}", value: "{index}", "{salinity.label()}" }
                        }
                    }
                    div { class: "field-label", "Default cylinder for new rows" }
                    select {
                        class: "field",
                        value: "{cylinder_value}",
                        onchange: move |evt| {
                            let value = evt.value();
                            prefs.write().default_cylinder = if value.is_empty() { None } else { value.parse().ok() };
                        },
                        option { value: "", "None" }
                        for (index, preset) in benthic_core::CYLINDER_PRESETS.iter().enumerate() {
                            option { key: "{index}", value: "{index}", "{preset.name}" }
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
