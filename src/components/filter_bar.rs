use dioxus::prelude::*;

use benthic_core::{DiveFilter, FilterPreset};

use crate::state::AppState;

/// A bar of structured filters (rating, tags, depth) and saved presets.
#[component]
pub fn FilterBar() -> Element {
    let state = use_context::<AppState>();
    let mut filter = state.filter;
    let mut presets_sig = state.presets;
    // Tags are edited as text and applied on change, to avoid re-formatting
    // the input while the user is still typing.
    let tags_text = use_signal(|| (state.filter)().tags.join(", "));
    let mut preset_name = use_signal(String::new);
    let mut loaded_preset = use_signal(|| None::<usize>);

    let current = (state.filter)();
    let prefs = (state.prefs)();
    let presets = (state.presets)();
    let selected = (loaded_preset)();
    let mut tags_text_signal = tags_text;

    let active = current.is_active();
    let ratings: Vec<(u8, String)> = (0..=5u8)
        .map(|r| {
            let label = if r == 0 {
                "Any rating".to_string()
            } else {
                format!("{r}+ stars")
            };
            (r, label)
        })
        .collect();

    let on_save_preset = move |_| {
        let name = preset_name().trim().to_string();
        if name.is_empty() {
            return;
        }
        let mut list = (state.presets)();
        let entry = FilterPreset::new(name, (state.filter)());
        match loaded_preset() {
            Some(index) if index < list.len() => list[index] = entry,
            _ => {
                list.push(entry);
                loaded_preset.set(Some(list.len() - 1));
            }
        }
        presets_sig.set(list);
        state.set_status("Saved filter preset");
    };

    let on_delete_preset = move |_| {
        let Some(index) = loaded_preset() else {
            return;
        };
        let mut list = (state.presets)();
        if index < list.len() {
            list.remove(index);
            presets_sig.set(list);
        }
        loaded_preset.set(None);
        preset_name.set(String::new());
    };

    // On narrow screens the fields collapse behind a toggle.
    let mut open = use_signal(|| false);
    let is_open = (open)();

    rsx! {
        div { class: "filter-bar",
            button {
                class: "btn filter-toggle",
                onclick: move |_| open.set(!is_open),
                if is_open { "Filters ▾" } else { "Filters ▸" }
            }
            div {
                class: if is_open { "filter-fields open" } else { "filter-fields" },
            span { class: "filter-label", "Filter" }
            select {
                class: "field",
                value: "{current.min_rating}",
                onchange: move |evt| {
                    filter.write().min_rating = evt.value().parse().unwrap_or(0);
                },
                for (value, label) in ratings {
                    option { key: "{value}", value: "{value}", "{label}" }
                }
            }
            input {
                class: "field",
                placeholder: "Tags (comma separated)",
                value: "{tags_text}",
                onchange: move |evt| {
                    let text = evt.value();
                    tags_text_signal.set(text.clone());
                    filter.write().tags = text
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                },
            }
            input {
                class: "field narrow",
                r#type: "number",
                placeholder: "Min {prefs.depth_unit()}",
                value: current.min_depth.map(|d| format!("{:.0}", prefs.depth_value(d))).unwrap_or_default(),
                oninput: move |evt| {
                    filter.write().min_depth = evt.value().parse::<f64>().ok().map(|v| prefs.depth_from_value(v));
                },
            }
            input {
                class: "field narrow",
                r#type: "number",
                placeholder: "Max {prefs.depth_unit()}",
                value: current.max_depth.map(|d| format!("{:.0}", prefs.depth_value(d))).unwrap_or_default(),
                oninput: move |evt| {
                    filter.write().max_depth = evt.value().parse::<f64>().ok().map(|v| prefs.depth_from_value(v));
                },
            }
            if active {
                button {
                    class: "btn",
                    onclick: move |_| {
                        tags_text_signal.set(String::new());
                        filter.set(DiveFilter::default());
                    },
                    "Clear filters"
                }
            }

            span { class: "filter-sep" }

            span { class: "filter-label", "Presets" }
            select {
                class: "field",
                value: "{selected.map(|i| i.to_string()).unwrap_or_default()}",
                onchange: move |evt| {
                    let value = evt.value();
                    if value.is_empty() {
                        loaded_preset.set(None);
                        return;
                    }
                    if let Ok(index) = value.parse::<usize>() {
                        let list = (state.presets)();
                        if let Some(preset) = list.get(index) {
                            filter.set(preset.filter.clone());
                            preset_name.set(preset.name.clone());
                            loaded_preset.set(Some(index));
                        }
                    }
                },
                option { value: "", "Load preset…" }
                for (index, preset) in presets.iter().enumerate() {
                    option { key: "{index}", value: "{index}", "{preset.name}" }
                }
            }
            input {
                class: "field",
                placeholder: "Preset name",
                value: "{preset_name()}",
                oninput: move |evt| preset_name.set(evt.value()),
            }
            button { class: "btn", onclick: on_save_preset, "Save" }
            button {
                class: "btn",
                disabled: selected.is_none(),
                onclick: on_delete_preset,
                "Delete"
            }
            }
        }
    }
}
