//! Download dives from a dive computer in the browser over WebSerial.
#![cfg(target_arch = "wasm32")]

use dioxus::prelude::*;

use benthic_core::Dive;

use crate::actions;
use crate::divecomputer;
use crate::divecomputer_web::{self, WebModel};
use crate::state::AppState;

/// A dialog to download dives from a WebSerial dive computer.
#[component]
pub fn DeviceDownloadWebDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show = state.show_download;

    let mut supported = use_signal(|| None::<bool>);
    let mut models = use_signal(Vec::<WebModel>::new);
    let mut search = use_signal(String::new);
    let mut selected = use_signal(|| None::<WebModel>);
    let mut connected = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut message = use_signal(|| Option::<String>::None);

    // Load the model table once the on-demand wasm module has come up.
    use_future(move || async move {
        if !divecomputer_web::supported().await {
            supported.set(Some(false));
            message.set(Some(
                "This browser has no Web Serial support. Chrome, Edge and Opera on \
                 desktop do; Safari and Firefox do not yet."
                    .to_string(),
            ));
            return;
        }
        supported.set(Some(true));
        match divecomputer_web::descriptors().await {
            Ok(list) => models.set(list),
            Err(error) => message.set(Some(format!("Could not load the device list: {error}"))),
        }
    });

    let on_connect = move |_| {
        if (selected)().is_none() {
            message.set(Some("Choose a dive computer model first".to_string()));
            return;
        }
        busy.set(true);
        message.set(Some("Waiting for you to pick a port…".to_string()));
        spawn(async move {
            match divecomputer_web::request_port().await {
                Ok(true) => {
                    connected.set(true);
                    message.set(Some(
                        "Port selected. Put the dive computer in transfer mode, then download."
                            .to_string(),
                    ));
                }
                Ok(false) => message.set(Some("No port selected.".to_string())),
                Err(error) => message.set(Some(format!("Could not open the port: {error}"))),
            }
            busy.set(false);
        });
    };

    let on_download = move |_| {
        let Some(model) = (selected)() else {
            return;
        };
        busy.set(true);
        message.set(Some("Downloading…".to_string()));
        spawn(async move {
            let key = divecomputer::fingerprint_key(&model.vendor, &model.product, "");
            let fingerprint = divecomputer::load_fingerprint(&key);
            let hex = divecomputer::to_hex(&fingerprint);
            match divecomputer_web::download(&model.vendor, &model.product, &hex).await {
                Ok(result) => {
                    if let Some(error) = result.error {
                        message.set(Some(format!("Download failed: {error}")));
                    } else {
                        let latest = divecomputer::from_hex(&result.fingerprint);
                        if !latest.is_empty() {
                            divecomputer::save_fingerprint(&key, &latest);
                        }
                        let dives: Vec<Dive> =
                            result.dives.iter().map(|raw| raw.to_dive()).collect();
                        let count = dives.len();
                        let (added, skipped) = actions::merge_downloaded(state, dives);
                        state.set_status(format!(
                            "Downloaded {added} new dive(s) from the dive computer"
                        ));
                        message.set(Some(format!(
                            "Read {count} dive(s); added {added}, skipped {skipped} already in the log"
                        )));
                        connected.set(false);
                    }
                }
                Err(error) => message.set(Some(format!("Download failed: {error}"))),
            }
            busy.set(false);
        });
    };

    let query = (search)().to_lowercase();
    let selected_now = (selected)();
    let models_now = (models)();
    let model_buttons = models_now
        .iter()
        .filter(|model| query.is_empty() || model.name().to_lowercase().contains(&query))
        .take(200)
        .map(|model| {
            let click_model = model.clone();
            let is_selected = selected_now.as_ref() == Some(model);
            rsx! {
                button {
                    class: if is_selected { "device-model selected" } else { "device-model" },
                    onclick: move |_| {
                        selected.set(Some(click_model.clone()));
                        connected.set(false);
                        message.set(None);
                    },
                    span { "{model.vendor} {model.product}" }
                }
            }
        });

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| show.set(false),
            div { class: "modal wide", onclick: move |evt| evt.stop_propagation(),
                h2 { "Download from a dive computer" }
                p { class: "muted",
                    "Connect a USB dive computer and pick it when the browser asks. Nothing is imported twice."
                }
                if supported() == Some(false) {
                    p { class: "warn", "Web Serial is not available in this browser." }
                } else {
                    label { class: "field-label", "Model"
                        input {
                            class: "field",
                            r#type: "search",
                            placeholder: "Search models…",
                            value: "{search}",
                            oninput: move |evt| search.set(evt.value()),
                        }
                    }
                    div { class: "device-models", {model_buttons} }
                }
                if let Some(text) = (message)() {
                    p { class: "muted", "{text}" }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn",
                        disabled: (busy)() || (selected)().is_none(),
                        onclick: on_connect,
                        if (connected)() { "Port selected" } else { "Connect…" }
                    }
                    button {
                        class: "btn primary",
                        disabled: (busy)() || !(connected)() || (selected)().is_none(),
                        onclick: on_download,
                        if (busy)() { "Downloading…" } else { "Download" }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| show.set(false),
                        "Close"
                    }
                }
            }
        }
    }
}
