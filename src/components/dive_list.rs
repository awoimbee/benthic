use dioxus::prelude::*;

use crate::state::AppState;

struct DiveRow {
    id: u32,
    class: &'static str,
    number: String,
    title: String,
    subtitle: String,
    gas: String,
}

#[component]
pub fn DiveList() -> Element {
    let state = use_context::<AppState>();
    let mut selected = state.selected;
    let log = (state.log)();
    let current = (state.selected)();

    let rows: Vec<DiveRow> = log
        .dives_sorted()
        .into_iter()
        .map(|dive| DiveRow {
            id: dive.id,
            class: if current == Some(dive.id) {
                "dive-row selected"
            } else {
                "dive-row"
            },
            number: if dive.number != 0 {
                dive.number.to_string()
            } else {
                "•".to_string()
            },
            title: crate::format::dive_title(dive, &log),
            subtitle: crate::format::dive_subtitle(dive),
            gas: dive
                .cylinders
                .first()
                .map(|c| c.gas.name())
                .unwrap_or_else(|| "—".to_string()),
        })
        .collect();

    let empty = rows.is_empty();

    rsx! {
        aside { class: "dive-list",
            div { class: "pane-title", "Dives" }
            if empty {
                div { class: "empty-hint", "No dives yet. Use Import to load a Subsurface log." }
            }
            ul {
                for row in rows {
                    li {
                        key: "{row.id}",
                        class: "{row.class}",
                        onclick: move |_| selected.set(Some(row.id)),
                        span { class: "dive-number", "{row.number}" }
                        div { class: "dive-main",
                            span { class: "dive-title", "{row.title}" }
                            span { class: "dive-subtitle", "{row.subtitle}" }
                        }
                        span { class: "dive-gas", "{row.gas}" }
                    }
                }
            }
        }
    }
}
