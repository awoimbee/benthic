//! The root component and application shell.

use dioxus::prelude::*;

use benthic_core::{DiveFilter, DiveLog, FilterPreset, History, Preferences};

use crate::components::{
    CommandPalette, CompareDialog, DiveDetail, DiveList, FilterBar, PlannerDialog,
    PreferencesDialog, Toolbar, TripsDialog,
};
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
    let prefs = use_signal(|| {
        crate::storage::load_prefs()
            .and_then(|text| serde_json::from_str::<Preferences>(&text).ok())
            .unwrap_or_default()
    });
    let show_prefs = use_signal(|| false);
    let show_palette = use_signal(|| false);
    let show_trips = use_signal(|| false);
    let show_planner = use_signal(|| false);
    let show_compare = use_signal(|| false);
    let presets = use_signal(|| {
        crate::storage::load_presets()
            .and_then(|text| serde_json::from_str::<Vec<FilterPreset>>(&text).ok())
            .unwrap_or_default()
    });
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
        presets,
        prefs,
        show_prefs,
        show_palette,
        show_trips,
        show_planner,
        show_compare,
    };
    use_context_provider(|| state);

    // Preferences and filter presets are small and load synchronously.
    use_effect(move || {
        let snapshot = prefs();
        if let Ok(text) = serde_json::to_string(&snapshot) {
            let _ = crate::storage::save_prefs(&text);
        }
    });
    use_effect(move || {
        let snapshot = presets();
        if let Ok(text) = serde_json::to_string(&snapshot) {
            let _ = crate::storage::save_presets(&text);
        }
    });

    // Load the autosaved log once at startup.
    use_future(move || async move {
        let mut log = log;
        let mut selected = selected;
        let mut status = status;
        let mut loaded = loaded;
        match crate::storage::load() {
            Some(text) => match benthic_core::io::parse_auto(&text) {
                Ok(parsed) => {
                    if let Some(first) = parsed.dives_recent_first().first().map(|d| d.id) {
                        selected.set(Some(first));
                    }
                    let count = parsed.dives.len();
                    log.set(parsed);
                    loaded.set(true);
                    status.set(format!("Loaded {count} dives from local storage"));
                }
                // Leave autosave disabled so the unreadable log is preserved
                // for manual recovery instead of being overwritten.
                Err(e) => match crate::storage::load_backup() {
                    Some(backup) => match benthic_core::io::parse_auto(&backup) {
                        Ok(parsed) => {
                            if let Some(first) = parsed.dives_recent_first().first().map(|d| d.id) {
                                selected.set(Some(first));
                            }
                            let count = parsed.dives.len();
                            log.set(parsed);
                            loaded.set(true);
                            status.set(format!(
                                "Primary log was unreadable ({e}); recovered {count} dives from the backup",
                            ));
                        }
                        Err(_) => status.set(format!("Could not read local log: {e}")),
                    },
                    None => status.set(format!("Could not read local log: {e}")),
                },
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
            Key::Character(ref c) if c.as_str() == "k" => {
                let mut palette = state.show_palette;
                palette.set(!(state.show_palette)());
                evt.prevent_default();
            }
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
        div { class: "app", tabindex: "0", autofocus: true, onkeydown: on_keydown,
            Toolbar {}
            FilterBar {}
            div { class: "panes",
                DiveList {}
                DiveDetail {}
            }
            if (show_prefs)() {
                PreferencesDialog {}
            }
            if (show_palette)() {
                CommandPalette {}
            }
            if (show_trips)() {
                TripsDialog {}
            }
            if (show_planner)() {
                PlannerDialog {}
            }
            if (show_compare)() {
                CompareDialog {}
            }
        }
    }
}
