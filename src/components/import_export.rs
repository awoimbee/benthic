use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::{AppState, ImportMode, PendingImport};

/// Open (replace the log from a file), import (merge a file into the log) and
/// export (SSRF) controls. Formats are auto-detected from the file name.
#[component]
pub fn ImportExport() -> Element {
    let state = use_context::<AppState>();
    let t = i18n::strings((state.prefs)().language);

    let on_import = move |evt: Event<FormData>| async move {
        let Some((name, incoming)) = read_log(state, evt, t.import_failed).await else {
            return;
        };
        let duplicates = actions::duplicate_dive_ids(&(state.log)(), &incoming);
        let mut import_review = state.import_review;
        import_review.set(Some(PendingImport {
            log: incoming,
            file_name: name,
            mode: ImportMode::Merge,
            duplicates,
            fingerprint: None,
            nonce: crate::platform::now_secs() as u64,
        }));
    };

    let on_open = move |evt: Event<FormData>| async move {
        let Some((name, incoming)) = read_log(state, evt, t.open_failed).await else {
            return;
        };
        let (dives, sites) = actions::open_log(state, incoming);
        state.set_status(i18n::t3(t.opened_file, dives, sites, name));
    };

    let on_export = move |_| actions::export_ssrf(state);

    rsx! {
        label { class: "btn",
            title: "{t.open_title}",
            // Keep the toolbar menu open while the file dialog is up.
            onclick: move |evt| evt.stop_propagation(),
            "{t.open}"
            input {
                r#type: "file",
                accept: ".ssrf,.xml,.json,.gpx,.csv,.tsv",
                class: "hidden-input",
                onchange: on_open,
            }
        }
        label { class: "btn",
            onclick: move |evt| evt.stop_propagation(),
            "{t.import}"
            input {
                r#type: "file",
                accept: ".ssrf,.xml,.json,.gpx,.csv,.tsv",
                class: "hidden-input",
                onchange: on_import,
            }
        }
        button {
            class: "btn",
            onclick: move |evt| {
                evt.stop_propagation();
                on_export(evt);
            },
            "{t.export}"
        }
    }
}

/// Read and parse the chosen file, reporting failures in the status bar.
async fn read_log(
    state: AppState,
    evt: Event<FormData>,
    parse_failed: &'static str,
) -> Option<(String, benthic_core::DiveLog)> {
    let file = evt.files().into_iter().next()?;
    let name = file.name();
    let t = state.strings();
    let text = match file.read_string().await {
        Ok(text) => text,
        Err(e) => {
            state.set_status(i18n::t2(t.could_not_read, name, e));
            return None;
        }
    };
    match benthic_core::io::parse_named(&name, &text) {
        Ok(log) => Some((name, log)),
        Err(e) => {
            state.set_status(i18n::t1(parse_failed, e));
            None
        }
    }
}
