use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

/// Open (replace the log from a file), import (merge a file into the log) and
/// export (SSRF) controls. Formats are auto-detected from the file name.
#[component]
pub fn ImportExport() -> Element {
    let state = use_context::<AppState>();

    let on_import = move |evt: Event<FormData>| {
        let state = state;
        async move {
            let mut selected = state.selected;
            let mut status = state.status;

            let Some(file) = evt.files().into_iter().next() else {
                return;
            };
            let name = file.name();

            match file.read_string().await {
                Ok(text) => match benthic_core::io::parse_named(&name, &text) {
                    Ok(incoming) => {
                        let (dives, sites) = actions::merge_log(state, incoming);
                        if selected().is_none() {
                            let first = (state.log)().dives_recent_first().first().map(|d| d.id);
                            selected.set(first);
                        }
                        status.set(format!(
                            "Imported {dives} dives and {sites} sites from {name}"
                        ));
                    }
                    Err(e) => status.set(format!("Import failed: {e}")),
                },
                Err(e) => status.set(format!("Could not read {name}: {e}")),
            }
        }
    };

    let on_open = move |evt: Event<FormData>| {
        let state = state;
        async move {
            let mut selected = state.selected;
            let mut status = state.status;

            let Some(file) = evt.files().into_iter().next() else {
                return;
            };
            let name = file.name();

            match file.read_string().await {
                Ok(text) => match benthic_core::io::parse_named(&name, &text) {
                    Ok(incoming) => {
                        let (dives, sites) = actions::open_log(state, incoming);
                        selected.set((state.log)().dives_recent_first().first().map(|d| d.id));
                        status.set(format!(
                            "Opened {dives} dives and {sites} sites from {name} (undo to restore the previous log)"
                        ));
                    }
                    Err(e) => status.set(format!("Open failed: {e}")),
                },
                Err(e) => status.set(format!("Could not read {name}: {e}")),
            }
        }
    };

    let on_export = move |_| actions::export_ssrf(state);

    rsx! {
        label { class: "btn",
            title: "Replace the current log with a file (undoable)",
            "Open"
            input {
                r#type: "file",
                accept: ".ssrf,.xml,.json,.gpx,.csv,.tsv",
                class: "hidden-input",
                onchange: on_open,
            }
        }
        label { class: "btn",
            "Import"
            input {
                r#type: "file",
                accept: ".ssrf,.xml,.json,.gpx,.csv,.tsv",
                class: "hidden-input",
                onchange: on_import,
            }
        }
        button { class: "btn", onclick: on_export, "Export" }
    }
}
