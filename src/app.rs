//! The root component and application shell.

use dioxus::prelude::*;

use benthic_core::{DiveFilter, DiveLog, FilterPreset, History, Preferences};

use crate::components::{
    CompareDialog, DiveDetail, DiveList, FilterBar, MapDialog, PlannerDialog, PreferencesDialog,
    SelectionBar, SyncDialog, Toolbar, TripsDialog, WelcomeDialog,
};
use crate::state::AppState;

#[cfg(feature = "divecomputer")]
use crate::components::DeviceDownloadDialog;
#[cfg(target_arch = "wasm32")]
use crate::components::DeviceDownloadWebDialog;

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
            // No saved preference yet: start in the browser's language when we
            // can guess it, otherwise English.
            .unwrap_or_else(|| Preferences {
                language: crate::platform::preferred_language().unwrap_or_default(),
                ..Default::default()
            })
    });
    let show_prefs = use_signal(|| false);
    let show_trips = use_signal(|| false);
    let show_map = use_signal(|| false);
    let show_planner = use_signal(|| false);
    let show_sync = use_signal(|| false);
    let show_download = use_signal(|| false);
    // Assume available, then hide the Download action on web platforms that
    // have neither Web Serial nor Web Bluetooth (e.g. iOS).
    let download_available = use_signal(|| true);
    let show_compare = use_signal(|| false);
    let mut mobile_detail = use_signal(|| false);
    let presets = use_signal(|| {
        crate::storage::load_presets()
            .and_then(|text| serde_json::from_str::<Vec<FilterPreset>>(&text).ok())
            .unwrap_or_default()
    });
    // Autosave is gated until the initial load has completed, so we never
    // overwrite a stored log with the empty in-memory log on startup.
    let loaded = use_signal(|| false);
    // Set when a local write fails, so the UI never claims data is safe when
    // it isn't (quota exhausted, private mode, ...).
    let mut storage_error = use_signal(|| None::<String>);
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
        show_trips,
        show_map,
        show_planner,
        show_sync,
        show_download,
        download_available,
        show_compare,
        mobile_detail,
    };
    use_context_provider(|| state);

    // Preferences and filter presets are small and load synchronously.
    use_effect(move || {
        let mut storage_error = storage_error;
        let snapshot = prefs();
        if let Ok(text) = serde_json::to_string(&snapshot) {
            if let Err(error) = crate::storage::save_prefs(&text) {
                storage_error.set(Some(error));
            }
        }
    });
    use_effect(move || {
        let mut storage_error = storage_error;
        let snapshot = presets();
        if let Ok(text) = serde_json::to_string(&snapshot) {
            if let Err(error) = crate::storage::save_presets(&text) {
                storage_error.set(Some(error));
            }
        }
    });
    // Keep the page in sync with the chosen theme (the web build also classes
    // <html> so the background behind the app matches).
    use_effect(move || {
        let snapshot = prefs();
        crate::platform::set_theme(snapshot.theme == benthic_core::Theme::Light);
        crate::platform::set_language(snapshot.language.code());
    });
    // On narrow screens the detail screen replaces the list. With nothing
    // selected there is no detail to show, so return to the list.
    use_effect(move || {
        if (selected)().is_none() {
            mobile_detail.set(false);
        }
    });

    // Detect dive-computer transport support on the web so the Download button
    // is only offered where it can actually work.
    #[cfg(target_arch = "wasm32")]
    use_future(move || async move {
        let mut download_available = download_available;
        if !crate::platform::web_transport_available() {
            download_available.set(false);
        }
    });

    // Load the autosaved log once at startup.
    use_future(move || async move {
        let mut log = log;
        let mut selected = selected;
        let mut status = status;
        let mut loaded = loaded;
        crate::storage::init().await;
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
        let mut storage_error = storage_error;
        let snapshot = log();
        if let Ok(text) = benthic_core::io::json::to_string(&snapshot) {
            match crate::storage::save(&text) {
                Ok(()) => {
                    if storage_error().is_some() {
                        storage_error.set(None);
                    }
                }
                Err(error) => storage_error.set(Some(error)),
            }
        }
    });

    // Global undo/redo shortcuts.
    let on_keydown = move |evt: KeyboardEvent| {
        // Escape closes the open dialog, whichever it is.
        if evt.key() == Key::Escape {
            let mut show_prefs = show_prefs;
            let mut show_trips = show_trips;
            let mut show_map = show_map;
            let mut show_planner = show_planner;
            let mut show_sync = show_sync;
            let mut show_download = show_download;
            let mut show_compare = show_compare;
            let mut closed = true;
            if (show_prefs)() {
                show_prefs.set(false);
            } else if (show_trips)() {
                show_trips.set(false);
            } else if (show_map)() {
                show_map.set(false);
            } else if (show_planner)() {
                show_planner.set(false);
            } else if (show_sync)() {
                show_sync.set(false);
            } else if (show_compare)() {
                show_compare.set(false);
            } else if (show_download)() {
                show_download.set(false);
            } else {
                closed = false;
            }
            if closed {
                evt.prevent_default();
            }
            return;
        }

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

    let theme_light = (prefs)().theme == benthic_core::Theme::Light;
    let tr = crate::i18n::strings((prefs)().language);
    let mut panes_class = String::from("panes");
    if (mobile_detail)() {
        panes_class.push_str(" mobile-detail");
    }
    // The mobile bulk-action bar overlays the panes, so reserve space for it.
    if !(state.selection)().is_empty() {
        panes_class.push_str(" selection-active");
    }

    #[cfg(feature = "divecomputer")]
    let device_download = (show_download)().then(|| rsx! { DeviceDownloadDialog {} });
    #[cfg(target_arch = "wasm32")]
    let device_download = (show_download)().then(|| rsx! { DeviceDownloadWebDialog {} });
    #[cfg(not(any(feature = "divecomputer", target_arch = "wasm32")))]
    let device_download: Option<Element> = None;

    rsx! {
        style { dangerous_inner_html: CSS }
        link { rel: "icon", r#type: "image/svg+xml", href: "favicon.svg" }
        // A real heading gives search engines and screen readers something
        // meaningful to announce; it is visually hidden by `.sr-only`.
        h1 { class: "sr-only", "benthic — a modern, local-first dive log" }
        p { class: "sr-only",
            "benthic is an open-source dive log for scuba divers. Import and export \
             Subsurface-compatible logs, plan dives with a Bühlmann decompression model, \
             browse interactive dive profiles, and keep your data on your own device."
        }
        div {
            class: if theme_light { "app theme-light" } else { "app" },
            tabindex: "0",
            autofocus: true,
            onkeydown: on_keydown,
            Toolbar {}
            FilterBar {}
            if let Some(error) = (storage_error)() {
                div { class: "storage-warning",
                    span { {crate::i18n::t1(tr.welcome_storage_error, error)} }
                    button {
                        class: "btn",
                        onclick: move |_| storage_error.set(None),
                        "{tr.dismiss}"
                    }
                }
            }
            div { class: "{panes_class}",
                DiveList {}
                DiveDetail {}
            }
            SelectionBar {}
            if (loaded)() && (log)().dives.is_empty() && !(prefs)().seen_welcome {
                WelcomeDialog {}
            }
            if (show_prefs)() {
                PreferencesDialog {}
            }
            if (show_trips)() {
                TripsDialog {}
            }
            if (show_map)() {
                MapDialog {}
            }
            if (show_planner)() {
                PlannerDialog {}
            }
            if (show_compare)() {
                CompareDialog {}
            }
            if (show_sync)() {
                SyncDialog {}
            }
            {device_download}
        }
    }
}
