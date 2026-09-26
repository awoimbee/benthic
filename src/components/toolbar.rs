use dioxus::prelude::*;

use crate::actions;
use crate::components::ImportExport;
use crate::state::AppState;

#[component]
pub fn Toolbar() -> Element {
    let state = use_context::<AppState>();
    let status = (state.status)();
    let log = (state.log)();
    let can_undo = state.can_undo();
    let can_redo = state.can_redo();
    let mut filter = state.filter;

    rsx! {
        header { class: "toolbar",
            span { class: "brand", "benthic" }
            span { class: "muted", "{log.dives.len()} dives" }

            button {
                class: "btn primary",
                onclick: move |_| actions::new_dive(state),
                "+ New dive"
            }
            button {
                class: if log.autogroup { "btn active" } else { "btn" },
                title: "Automatically group nearby dives into trips",
                onclick: move |_| actions::toggle_autogroup(state),
                if log.autogroup { "Auto-group: on" } else { "Auto-group: off" }
            }

            ImportExport {}

            div { class: "spacer" }

            input {
                class: "search",
                r#type: "search",
                placeholder: "Search dives…",
                value: "{filter().query}",
                oninput: move |evt| filter.write().query = evt.value(),
            }
            button {
                class: "btn",
                disabled: !can_undo,
                title: "Undo (Ctrl/Cmd+Z)",
                onclick: move |_| state.undo(),
                "Undo"
            }
            button {
                class: "btn",
                disabled: !can_redo,
                title: "Redo (Ctrl/Cmd+Shift+Z)",
                onclick: move |_| state.redo(),
                "Redo"
            }
            span { class: "status", "{status}" }
        }
    }
}
