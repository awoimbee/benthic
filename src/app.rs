//! The root component and application shell.

use dioxus::prelude::*;

use benthic_core::{DiveFilter, DiveLog, History};

use crate::components::{DiveDetail, DiveList, FilterBar, Toolbar};
use crate::state::AppState;

const CSS: &str = include_str!("../assets/main.css");

#[component]
pub fn App() -> Element {
    let log = use_signal(DiveLog::new);
    let selected = use_signal(|| None::<u32>);
    let selection = use_signal(std::collections::BTreeSet::new);
    let status = use_signal(|| "Ready".to_string());
    let history = use_signal(History::new);
    let filter = use_signal(DiveFilter::default);
    // Autosave is gated until the initial load has completed, so we never
    // overwrite a stored log with the empty in-memory log on startup.
    let loaded = use_signal(|| false);
    let state = AppState {
        log,
        selected,
        selection,
        status,
        history,
        filter,
    };
    use_context_provider(|| state);

    // Load the autosaved log once at startup.
    use_future(move || async move {
        let mut log = log;
        let mut selected = selected;
        let mut status = status;
        let mut loaded = loaded;
        match crate::storage::load() {
            Some(text) => match benthic_core::io::parse_auto(&text) {
                Ok(parsed) => {
                    if let Some(first) = parsed.dives_sorted().first().map(|d| d.id) {
                        selected.set(Some(first));
                    }
                    let count = parsed.dives.len();
                    log.set(parsed);
                    loaded.set(true);
                    status.set(format!("Loaded {count} dives from local storage"));
                }
                // Leave autosave disabled so the unreadable log is preserved
                // for manual recovery instead of being overwritten.
                Err(e) => status.set(format!("Could not read local log: {e}")),
            },
            None => {
                loaded.set(true);
                status.set("No local log yet — import one to get started".to_string());
            }
        }
    });

    // Autosave whenever the log changes (once loading has finished).
    use_effect(move || {
        if !(loaded)() {
            return;
        }
        let snapshot = log();
        if let Ok(text) = benthic_core::io::json::to_string(&snapshot) {
            let _ = crate::storage::save(&text);
        }
    });

    // Global undo/redo shortcuts.
    let on_keydown = move |evt: KeyboardEvent| {
        let modifiers = evt.modifiers();
        let ctrl = modifiers.contains(Modifiers::CONTROL) || modifiers.contains(Modifiers::META);
        if !ctrl {
            return;
        }
        match evt.key() {
            Key::Character(ref c) if c.as_str() == "z" => {
                if modifiers.contains(Modifiers::SHIFT) {
                    state.redo();
                } else {
                    state.undo();
                }
                evt.prevent_default();
            }
            Key::Character(ref c) if c.as_str() == "y" => {
                state.redo();
                evt.prevent_default();
            }
            _ => {}
        }
    };

    rsx! {
        style { dangerous_inner_html: CSS }
        link { rel: "icon", r#type: "image/svg+xml", href: "favicon.svg" }
        div { class: "app", tabindex: "0", onkeydown: on_keydown,
            Toolbar {}
            FilterBar {}
            div { class: "panes",
                DiveList {}
                DiveDetail {}
            }
        }
    }
}
