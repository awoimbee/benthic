//! The root component and application shell.

use dioxus::prelude::*;

use benthic_core::{DiveFilter, DiveLog, FilterPreset, History, Preferences};

use crate::components::{
    AiDialog, BatchEditDialog, CompareDialog, ConfirmDialog, DiveDetail, DiveList, FilterBar,
    ImportReviewDialog, MapDialog, PeopleGearDialog, PlannerDialog, PreferencesDialog,
    SelectionBar, SyncDialog, Toolbar, TripView, TripsDialog, WelcomeDialog,
};
use crate::routing::{self, Route};
use crate::state::{AppState, Dialog};

#[cfg(feature = "divecomputer")]
use crate::components::DeviceDownloadDialog;
#[cfg(target_arch = "wasm32")]
use crate::components::DeviceDownloadWebDialog;

const CSS: &str = include_str!("../assets/main.css");

#[component]
pub fn App() -> Element {
    let initial_route = routing::current_route();
    let log = use_signal(DiveLog::new);
    let selected = use_signal(|| None::<u32>);
    let selection = use_signal(std::collections::BTreeSet::new);
    let status = use_signal(String::new);
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
    let dialog = use_signal(|| initial_route.dialog());
    let map_focus = use_signal(|| None::<(f64, f64)>);
    // Assume available, then hide the Download action on web platforms that
    // have neither Web Serial nor Web Bluetooth (e.g. iOS).
    let download_available = use_signal(|| true);
    let import_review = use_signal(|| None::<crate::state::PendingImport>);
    let show_trip = use_signal(|| match initial_route {
        Route::Trip(id) => Some(id),
        _ => None,
    });
    let mut mobile_detail = use_signal(|| matches!(initial_route, Route::Trip(_)));
    let route = use_signal(|| initial_route);
    let presets = use_signal(|| {
        crate::storage::load_presets()
            .and_then(|text| serde_json::from_str::<Vec<FilterPreset>>(&text).ok())
            .unwrap_or_default()
    });
    let show_filters = use_signal(|| false);
    // Autosave is gated until the initial load has completed, so we never
    // overwrite a stored log with the empty in-memory log on startup.
    let loaded = use_signal(|| false);
    // Set when the stored log was unreadable and had no usable backup: once
    // the unreadable copy is preserved, the first edit turns autosave on.
    let autosave_on_edit = use_signal(|| false);
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
        show_filters,
        prefs,
        dialog,
        map_focus,
        download_available,
        import_review,
        show_trip,
        mobile_detail,
        route,
    };
    use_context_provider(|| state);

    // Keep the browser URL in sync with the open pane or dialog. This is the
    // single place that pushes history for UI-driven navigation.
    use_effect(move || {
        let desired = Route::from_ui((dialog)(), (show_trip)());
        if desired == (state.route)() {
            return;
        }
        let mut route_signal = state.route;
        route_signal.set(desired);
        routing::push(desired);
    });

    // Browser Back/Forward applies the URL route to the UI without pushing a
    // new entry.
    #[cfg(target_arch = "wasm32")]
    use_future(move || async move {
        let mut route_signal = state.route;
        let mut eval = document::eval(
            "window.addEventListener('popstate', () => dioxus.send(window.location.pathname));",
        );
        while let Ok(path) = eval.recv::<String>().await {
            let next = routing::route_from_path(&path);
            route_signal.set(next);
            routing::apply_route(state, next);
        }
    });

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
        if (selected)().is_none() && (show_trip)().is_none() {
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
        let t = crate::i18n::strings((prefs)().language);
        let mut log = log;
        let mut selected = selected;
        let mut status = status;
        let mut loaded = loaded;
        let mut autosave_on_edit = autosave_on_edit;
        let mut storage_error = storage_error;
        let mut adopt = move |mut parsed: DiveLog| {
            parsed.prepare();
            selected.set(parsed.newest_dive_id());
            let count = parsed.dives.len();
            log.set(parsed);
            count
        };
        // Autosave stays off on every failure path below, so a log we could
        // not read is never overwritten.
        if let Err(error) = crate::storage::init().await {
            let message = crate::i18n::t1(t.status_could_not_read, error);
            status.set(message.clone());
            storage_error.set(Some(message));
            return;
        }
        let Some(text) = crate::storage::load() else {
            loaded.set(true);
            status.set(t.status_no_log.to_string());
            return;
        };
        let error = match benthic_core::io::parse_auto(&text) {
            Ok(parsed) => {
                let count = adopt(parsed);
                loaded.set(true);
                status.set(crate::i18n::t1(t.status_loaded, count));
                return;
            }
            Err(error) => error,
        };
        let kept = match crate::storage::preserve_unreadable(&text).await {
            Ok(key) => key,
            Err(preserve_error) => {
                let message = crate::i18n::t1(
                    t.status_could_not_read,
                    format!("{error}; {preserve_error}"),
                );
                status.set(message.clone());
                storage_error.set(Some(message));
                return;
            }
        };
        match crate::storage::load_backup().map(|backup| benthic_core::io::parse_auto(&backup)) {
            Some(Ok(parsed)) => {
                let count = adopt(parsed);
                loaded.set(true);
                status.set(crate::i18n::t3(t.status_recovered, error, count, kept));
            }
            _ => {
                // The unreadable log is safely copied, so the next edit may
                // start a fresh log in its place.
                autosave_on_edit.set(true);
                let message = crate::i18n::t2(t.status_unreadable_kept, error, kept);
                status.set(message.clone());
                storage_error.set(Some(message));
            }
        }
    });

    // Report background write failures (and recoveries) on the web, where
    // saves complete asynchronously.
    #[cfg(target_arch = "wasm32")]
    use_future(move || async move {
        let mut storage_error = storage_error;
        loop {
            storage_error.set(crate::storage::next_write_status().await);
        }
    });

    // Autosave whenever the log changes (once loading has finished).
    use_effect(move || {
        if !(loaded)() {
            if !((autosave_on_edit)() && (history)().can_undo()) {
                return;
            }
            let mut loaded = loaded;
            loaded.set(true);
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
            if (dialog)().is_some() {
                state.close_dialog();
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
    let show_welcome = (loaded)() && (log)().dives.is_empty() && !(prefs)().seen_welcome;
    // While a dialog is open, the page behind it is inert, so focus and
    // assistive technology stay inside the dialog.
    let modal_open = show_welcome || (dialog)().is_some() || (import_review)().is_some();

    let dialog_view = (dialog)().map(|open| match open {
        Dialog::Preferences => rsx! { PreferencesDialog {} },
        Dialog::Ai => rsx! { AiDialog {} },
        Dialog::Trips => rsx! { TripsDialog {} },
        Dialog::Map => rsx! { MapDialog {} },
        Dialog::People => rsx! { PeopleGearDialog {} },
        Dialog::Planner => rsx! { PlannerDialog {} },
        Dialog::Sync => rsx! { SyncDialog {} },
        Dialog::Compare => rsx! { CompareDialog {} },
        Dialog::BatchEdit => rsx! { BatchEditDialog {} },
        Dialog::ConfirmDeleteSelected => rsx! {
            ConfirmDialog {
                title: tr.confirm_delete_dives_title.to_string(),
                body: crate::i18n::t1(tr.confirm_delete_dives_body, (selection)().len()),
                confirm_label: tr.delete.to_string(),
                cancel_label: tr.cancel.to_string(),
                on_confirm: move |_| {
                    crate::actions::delete_selected(state);
                    state.close_dialog();
                },
                on_cancel: move |_| state.close_dialog(),
            }
        },
        #[cfg(feature = "divecomputer")]
        Dialog::Download => rsx! { DeviceDownloadDialog {} },
        #[cfg(target_arch = "wasm32")]
        Dialog::Download => rsx! { DeviceDownloadWebDialog {} },
        #[cfg(not(any(feature = "divecomputer", target_arch = "wasm32")))]
        Dialog::Download => rsx! {},
    });

    rsx! {
        style { dangerous_inner_html: CSS }
        link { rel: "icon", r#type: "image/svg+xml", href: "favicon.svg" }
        // A real heading gives search engines and screen readers something
        // meaningful to announce; it is visually hidden by `.sr-only`.
        h1 { class: "sr-only", "{tr.app_h1}" }
        p { class: "sr-only", "{tr.app_description}" }
        // The key handler must see events from the dialogs too, so it lives
        // on this non-inert wrapper rather than on the inert app shell.
        div { class: "app-root", onkeydown: on_keydown,
        div {
            class: if theme_light { "app theme-light" } else { "app" },
            tabindex: "0",
            autofocus: true,
            // Only render the attribute when true: `inert` is a boolean
            // attribute, so `inert="false"` would still disable the page.
            inert: if modal_open { Some("true") } else { None },
            Toolbar {}
            if (show_filters)() {
                FilterBar {}
            }
            if let Some(error) = (storage_error)() {
                div { class: "storage-warning", role: "alert",
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
                if let Some(trip_id) = (show_trip)() {
                    TripView { key: "{trip_id}", trip_id }
                } else {
                    DiveDetail {}
                }
            }
            SelectionBar {}
        }

        // Dialogs sit outside the inert shell.
        if show_welcome {
            WelcomeDialog {}
        }
        {dialog_view}
        if let Some(pending) = (import_review)() {
            ImportReviewDialog { key: "{pending.nonce}", pending }
        }
        }
    }
}
