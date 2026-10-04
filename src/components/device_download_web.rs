//! Download dives from a dive computer in the browser over Web Serial or
//! Web Bluetooth.
#![cfg(target_arch = "wasm32")]

use dioxus::prelude::*;

use benthic_core::Dive;

use crate::actions;
use crate::divecomputer;
use crate::divecomputer_web::{self, WebModel};
use crate::i18n;
use crate::state::AppState;

/// A dialog to download dives from a web-connected dive computer.
#[component]
pub fn DeviceDownloadWebDialog() -> Element {
    let state = use_context::<AppState>();
    let tr = i18n::strings((state.prefs)().language);
    let mut show = state.show_download;

    let mut serial_ok = use_signal(|| false);
    let mut bluetooth_ok = use_signal(|| false);
    let mut models = use_signal(Vec::<WebModel>::new);
    let mut search = use_signal(String::new);
    let mut selected = use_signal(|| None::<WebModel>);
    let mut transport = use_signal(|| divecomputer_web::TRANSPORT_SERIAL);
    let mut connected = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut message = use_signal(|| Option::<String>::None);

    // Load the model table once the on-demand wasm module has come up.
    use_future(move || async move {
        let serial = divecomputer_web::supported().await;
        let bluetooth = divecomputer_web::bluetooth_supported().await;
        serial_ok.set(serial);
        bluetooth_ok.set(bluetooth);
        if !serial && !bluetooth {
            message.set(Some(tr.dc_no_transport_web.to_string()));
            return;
        }
        if !serial {
            transport.set(divecomputer_web::TRANSPORT_BLE);
        }
        match divecomputer_web::descriptors().await {
            Ok(list) => models.set(list),
            Err(error) => message.set(Some(i18n::t1(tr.dc_could_not_load, error))),
        }
    });

    let on_connect = move |_| {
        if (selected)().is_none() {
            message.set(Some(tr.dc_choose_model.to_string()));
            return;
        }
        let transport_now = (transport)();
        busy.set(true);
        message.set(Some(tr.dc_waiting.to_string()));
        spawn(async move {
            match divecomputer_web::connect(transport_now).await {
                Ok(true) => {
                    connected.set(true);
                    message.set(Some(tr.dc_selected.to_string()));
                }
                Ok(false) => message.set(Some(tr.dc_no_device.to_string())),
                Err(error) => message.set(Some(i18n::t1(tr.dc_could_not_connect, error))),
            }
            busy.set(false);
        });
    };

    let on_download = move |_| {
        let Some(model) = (selected)() else {
            return;
        };
        let transport_now = (transport)();
        busy.set(true);
        message.set(Some(tr.dc_downloading.to_string()));
        spawn(async move {
            let link = if transport_now == divecomputer_web::TRANSPORT_BLE {
                "bluetooth"
            } else {
                "serial"
            };
            let key = divecomputer::fingerprint_key(&model.vendor, &model.product, link);
            let fingerprint = divecomputer::load_fingerprint(&key);
            let hex = divecomputer::to_hex(&fingerprint);
            match divecomputer_web::download(&model.vendor, &model.product, transport_now, &hex)
                .await
            {
                Ok(result) => {
                    if let Some(error) = result.error {
                        message.set(Some(i18n::t1(tr.dc_download_failed, error)));
                    } else {
                        let latest = divecomputer::from_hex(&result.fingerprint);
                        if !latest.is_empty() {
                            divecomputer::save_fingerprint(&key, &latest);
                        }
                        let dives: Vec<Dive> =
                            result.dives.iter().map(|raw| raw.to_dive()).collect();
                        let count = dives.len();
                        let (added, skipped) = actions::merge_downloaded(state, dives);
                        state.set_status(i18n::t1(state.strings().downloading_dc, added));
                        message.set(Some(i18n::t3(tr.dc_read_summary, count, added, skipped)));
                        connected.set(false);
                    }
                }
                Err(error) => message.set(Some(i18n::t1(tr.dc_download_failed, error))),
            }
            busy.set(false);
        });
    };

    let query = (search)().to_lowercase();
    let selected_now = (selected)();
    let transport_now = (transport)();
    let models_now = (models)();
    let model_buttons = models_now
        .iter()
        .filter(|model| model.transports & transport_now != 0)
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

    let any_supported = (serial_ok)() || (bluetooth_ok)();

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| show.set(false),
            div { class: "modal wide",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{tr.dc_title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.dc_title}" }
                p { class: "muted", "{tr.dc_intro}" }
                if !any_supported {
                    p { class: "warn", "{tr.dc_no_transport}" }
                } else {
                    if (serial_ok)() && (bluetooth_ok)() {
                        label { class: "field-label", "{tr.dc_connection}"
                            select {
                                class: "field",
                                onchange: move |evt| {
                                    transport.set(if evt.value() == "bluetooth" {
                                        divecomputer_web::TRANSPORT_BLE
                                    } else {
                                        divecomputer_web::TRANSPORT_SERIAL
                                    });
                                    connected.set(false);
                                },
                                option {
                                    value: "serial",
                                    selected: transport_now == divecomputer_web::TRANSPORT_SERIAL,
                                    "{tr.dc_usb_serial}"
                                }
                                option {
                                    value: "bluetooth",
                                    selected: transport_now == divecomputer_web::TRANSPORT_BLE,
                                    "{tr.dc_bluetooth}"
                                }
                            }
                        }
                    } else if (bluetooth_ok)() {
                        p { class: "muted", "{tr.dc_using_bluetooth}" }
                    } else {
                        p { class: "muted", "{tr.dc_using_serial}" }
                    }
                    label { class: "field-label", "{tr.dc_model}"
                        input {
                            class: "field",
                            r#type: "search",
                            placeholder: "{tr.dc_search_models}",
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
                        if (connected)() { "{tr.dc_connected}" } else { "{tr.dc_connect}" }
                    }
                    button {
                        class: "btn primary",
                        disabled: (busy)() || !(connected)() || (selected)().is_none(),
                        onclick: on_download,
                        if (busy)() { "{tr.dc_downloading}" } else { "{tr.dc_download}" }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| show.set(false),
                        "{tr.close}"
                    }
                }
            }
        }
    }
}
