use dioxus::prelude::*;

use crate::state::AppState;

/// Import (SSRF/XML/JSON, auto-detected) and export (SSRF) controls.
#[component]
pub fn ImportExport() -> Element {
    let state = use_context::<AppState>();

    let on_import = move |evt: Event<FormData>| {
        let state = state;
        async move {
            let mut log = state.log;
            let mut selected = state.selected;
            let mut status = state.status;

            let Some(file) = evt.files().into_iter().next() else {
                return;
            };
            let name = file.name();

            match file.read_string().await {
                Ok(text) => match benthic_core::io::parse_auto(&text) {
                    Ok(mut incoming) => {
                        incoming.fixup_all();
                        let count = incoming.dives.len();
                        let mut current = log();
                        current.merge(incoming);
                        let first = current.dives_sorted().first().map(|d| d.id);
                        log.set(current);
                        if selected().is_none() {
                            selected.set(first);
                        }
                        status.set(format!("Imported {count} dives from {name}"));
                    }
                    Err(e) => status.set(format!("Import failed: {e}")),
                },
                Err(e) => status.set(format!("Could not read {name}: {e}")),
            }
        }
    };

    let on_export = move |_| {
        let mut status = state.status;
        let log = (state.log)();
        let text = benthic_core::io::ssrf::write_string(&log);
        match crate::platform::save_file("benthic.ssrf", &text) {
            Ok(message) => status.set(message),
            Err(e) => status.set(format!("Export failed: {e}")),
        }
    };

    rsx! {
        label { class: "btn",
            "Import"
            input {
                r#type: "file",
                accept: ".ssrf,.xml,.json",
                class: "hidden-input",
                onchange: on_import,
            }
        }
        button { class: "btn", onclick: on_export, "Export" }
    }
}
