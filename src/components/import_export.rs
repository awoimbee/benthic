use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

/// Import (SSRF/XML/JSON, auto-detected) and export (SSRF) controls.
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

    let on_export = move |_| actions::export_ssrf(state);

    rsx! {
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
