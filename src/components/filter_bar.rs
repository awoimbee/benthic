use dioxus::prelude::*;

use benthic_core::DiveFilter;

use crate::state::AppState;

/// A bar of structured filters (rating, tags, depth) below the toolbar.
#[component]
pub fn FilterBar() -> Element {
    let state = use_context::<AppState>();
    let mut filter = state.filter;
    // Tags are edited as text and applied on change, to avoid re-formatting
    // the input while the user is still typing.
    let tags_text = use_signal(|| (state.filter)().tags.join(", "));
    let current = (state.filter)();
    let prefs = (state.prefs)();
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

    rsx! {
        div { class: "filter-bar",
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
        }
    }
}
