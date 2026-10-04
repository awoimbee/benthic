use dioxus::prelude::*;

use benthic_core::{DateFormat, Language, Salinity, Theme, TimeFormat, UnitSystem};

use crate::actions;
use crate::i18n;
use crate::state::AppState;

/// A modal for display preferences and data recovery.
#[component]
pub fn PreferencesDialog() -> Element {
    let state = use_context::<AppState>();
    let mut prefs = state.prefs;
    let mut show_prefs = state.show_prefs;
    let t = i18n::strings(prefs().language);
    let current = (state.prefs)().units;
    let current_theme = (state.prefs)().theme;
    let current_language = (state.prefs)().language;
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
        Some(age) => i18n::t1(t.backup_age, human_age(age, t)),
        None => t.no_backup_yet.to_string(),
    };

    // The policy page is a static web asset; it does not exist in the desktop
    // bundle, so only link to it on the web.
    #[cfg(target_arch = "wasm32")]
    let privacy = Some(rsx! {
        div { class: "pref-group",
            div { class: "field-label", "{t.about}" }
            div { class: "muted", "{t.about_text}" }
            a {
                class: "link",
                href: "privacy.html",
                target: "_blank",
                rel: "noopener",
                "{t.privacy_policy}"
            }
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let privacy: Option<Element> = None;

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_prefs.set(false),
            div {
                class: "modal",
                role: "dialog",
            tabindex: "-1",
            autofocus: true,
                aria_modal: "true",
                aria_label: "{t.preferences}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{t.preferences}" }
                div { class: "pref-group",
                    div { class: "field-label", "{t.language}" }
                    for language in Language::ALL {
                        label {
                            key: "{language.code()}",
                            class: "radio",
                            input {
                                r#type: "radio",
                                name: "language",
                                checked: current_language == language,
                                onchange: move |_| prefs.write().language = language,
                            }
                            span { "{language.label()}" }
                        }
                    }
                }
                div { class: "pref-group",
                    div { class: "field-label", "{t.theme}" }
                    for theme in Theme::ALL {
                        label {
                            key: "{theme.label()}",
                            class: "radio",
                            input {
                                r#type: "radio",
                                name: "theme",
                                checked: current_theme == theme,
                                onchange: move |_| prefs.write().theme = theme,
                            }
                            span { "{theme_label(t, theme)}" }
                        }
                    }
                }
                div { class: "pref-group",
                    div { class: "field-label", "{t.units}" }
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
                            span { "{unit_label(t, units)} — {units_description(units)}" }
                        }
                    }
                }
                div { class: "pref-group",
                    div { class: "field-label", "{t.date_format}" }
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
                    div { class: "field-label", "{t.time_format}" }
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
                    div { class: "field-label", "{t.default_salinity}" }
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
                    div { class: "field-label", "{t.default_cylinder}" }
                    select {
                        class: "field",
                        value: "{cylinder_value}",
                        onchange: move |evt| {
                            let value = evt.value();
                            prefs.write().default_cylinder = if value.is_empty() { None } else { value.parse().ok() };
                        },
                        option { value: "", "{t.none}" }
                        for (index, preset) in benthic_core::CYLINDER_PRESETS.iter().enumerate() {
                            option { key: "{index}", value: "{index}", "{preset.name}" }
                        }
                    }
                }
                div { class: "pref-group",
                    div { class: "field-label", "{t.data}" }
                    div { class: "muted", "{backup_label}" }
                    div { class: "detail-actions",
                        button {
                            class: "btn",
                            disabled: !has_backup,
                            onclick: move |_| actions::restore_backup(state),
                            "{t.restore_backup}"
                        }
                    }
                }
                {privacy}
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        onclick: move |_| show_prefs.set(false),
                        "{t.done}"
                    }
                }
            }
        }
    }
}

fn theme_label(t: &'static i18n::Strings, theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => t.theme_dark,
        Theme::Light => t.theme_light,
    }
}

fn unit_label(t: &'static i18n::Strings, units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => t.unit_metric,
        UnitSystem::Imperial => t.unit_imperial,
    }
}

fn human_age(secs: i64, t: &'static i18n::Strings) -> String {
    if secs < 60 {
        i18n::t1(t.ago_seconds, secs)
    } else if secs < 3600 {
        i18n::t1(t.ago_minutes, secs / 60)
    } else if secs < 86_400 {
        i18n::t1(t.ago_hours, secs / 3600)
    } else {
        i18n::t1(t.ago_days, secs / 86_400)
    }
}

fn units_description(units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Metric => "m, °C, bar, kg, L",
        UnitSystem::Imperial => "ft, °F, psi, lbs, cuft",
    }
}
