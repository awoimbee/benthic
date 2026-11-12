//! Download dives from a dive computer in the browser over Web Serial or
//! Web Bluetooth.
#![cfg(target_arch = "wasm32")]

use dioxus::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use benthic_core::Dive;

use super::device_download_common::{
    finish, DownloadFrame, DownloadStatus, ModelEntry, ModelPicker,
};
use crate::divecomputer;
use crate::divecomputer_web::{self, WebModel};
use crate::i18n;
use crate::state::AppState;

/// A dialog to download dives from a web-connected dive computer.
#[component]
pub fn DeviceDownloadWebDialog() -> Element {
    let state = use_context::<AppState>();
    let tr = i18n::strings((state.prefs)().language);

    let mut serial_ok = use_signal(|| false);
    let mut bluetooth_ok = use_signal(|| false);
    let mut models = use_signal(Vec::<WebModel>::new);
    let search = use_signal(String::new);
    let mut selected = use_signal(|| None::<WebModel>);
    let mut transport = use_signal(|| divecomputer_web::TRANSPORT_SERIAL);
    let mut connected = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut progress = use_signal(|| None::<(u32, u32)>);
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
            Ok(mut list) => {
                list.insert(
                    0,
                    WebModel {
                        vendor: divecomputer::MOCK_VENDOR.to_string(),
                        product: divecomputer::MOCK_PRODUCT.to_string(),
                        transports: divecomputer_web::TRANSPORT_SERIAL
                            | divecomputer_web::TRANSPORT_BLE,
                    },
                );
                models.set(list);
            }
            Err(error) => message.set(Some(i18n::t1(tr.dc_could_not_load, error))),
        }
    });

    let on_connect = move |_| {
        let Some(model) = (selected)() else {
            message.set(Some(tr.dc_choose_model.to_string()));
            return;
        };
        if divecomputer::is_mock(&model.vendor, &model.product) {
            connected.set(true);
            message.set(Some(tr.dc_mock_ready.to_string()));
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

    let on_forget_history = move |_| {
        let Some(model) = (selected)() else {
            message.set(Some(tr.dc_choose_model.to_string()));
            return;
        };
        if divecomputer::is_mock(&model.vendor, &model.product) {
            return;
        }
        divecomputer::save_fingerprint(&fingerprint_key(&model, (transport)()), &[]);
        message.set(Some(tr.dc_history_cleared.to_string()));
    };

    let on_download = move |_| {
        let Some(model) = (selected)() else {
            return;
        };
        if divecomputer::is_mock(&model.vendor, &model.product) {
            finish(state, divecomputer::mock_dives(), model.name(), None);
            return;
        }
        let transport_now = (transport)();
        busy.set(true);
        progress.set(None);
        message.set(Some(tr.dc_downloading.to_string()));
        spawn(async move {
            let key = fingerprint_key(&model, transport_now);
            let fingerprint = divecomputer::load_fingerprint(&key);
            let hex = divecomputer::to_hex(&fingerprint);
            let mut progress_signal = progress;
            let callback_closure = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
                let event = js_sys::Reflect::get(&value, &JsValue::from_str("event"))
                    .ok()
                    .and_then(|event| event.as_string());
                if event.as_deref() != Some("progress") {
                    return;
                }
                let current = js_sys::Reflect::get(&value, &JsValue::from_str("current"))
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as u32;
                let maximum = js_sys::Reflect::get(&value, &JsValue::from_str("maximum"))
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as u32;
                if maximum > 0 {
                    progress_signal.set(Some((current, maximum)));
                }
            });
            let callback_function: &js_sys::Function = callback_closure.as_ref().unchecked_ref();
            let result = divecomputer_web::download(
                &model.vendor,
                &model.product,
                transport_now,
                &hex,
                Some(callback_function),
            )
            .await;
            drop(callback_closure);
            match result {
                Ok(result) => {
                    if let Some(error) = result.error {
                        message.set(Some(i18n::t1(tr.dc_download_failed, error)));
                    } else {
                        let latest = divecomputer::from_hex(&result.fingerprint);
                        let fingerprint = (!latest.is_empty()).then(|| (key.clone(), latest));
                        let dives: Vec<Dive> =
                            result.dives.iter().map(|raw| raw.to_dive()).collect();
                        finish(state, dives, model.name(), fingerprint);
                    }
                }
                Err(error) => message.set(Some(i18n::t1(tr.dc_download_failed, error))),
            }
            progress.set(None);
            busy.set(false);
        });
    };

    let selected_now = (selected)();
    let transport_now = (transport)();
    let models_now = (models)();
    let entries: Vec<ModelEntry> = models_now
        .iter()
        .map(|model| {
            let serial = model.transports & divecomputer_web::TRANSPORT_SERIAL != 0;
            let bluetooth = model.transports & divecomputer_web::TRANSPORT_BLE != 0;
            ModelEntry {
                label: model.name(),
                detail: match (serial, bluetooth) {
                    (true, true) => format!("{} + {}", tr.dc_usb_serial, tr.dc_bluetooth),
                    (true, false) => tr.dc_usb_serial.to_string(),
                    (false, true) => tr.dc_bluetooth.to_string(),
                    (false, false) => String::new(),
                },
                selected: selected_now.as_ref() == Some(model),
                unavailable: (model.transports & transport_now == 0)
                    .then_some(tr.dc_transport_unavailable),
            }
        })
        .collect();
    let on_pick = move |index: usize| {
        let Some(model) = (models)().get(index).cloned() else {
            return;
        };
        let mock = divecomputer::is_mock(&model.vendor, &model.product);
        selected.set(Some(model));
        connected.set(mock);
        message.set(mock.then(|| tr.dc_mock_ready.to_string()));
    };

    let any_supported = (serial_ok)() || (bluetooth_ok)();

    rsx! {
        DownloadFrame { intro: tr.dc_intro,
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
                    ModelPicker { search, models: entries, on_pick }
                }
                DownloadStatus { progress: (progress)(), message: (message)() }
                div { class: "detail-actions",
                    button {
                        class: "btn",
                        disabled: (busy)() || (selected)().is_none(),
                        onclick: on_forget_history,
                        "{tr.dc_forget_history}"
                    }
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
                        onclick: move |_| state.close_dialog(),
                        "{tr.close}"
                    }
                }
        }
    }
}

/// The fingerprint key for a model over a web transport.
fn fingerprint_key(model: &WebModel, transport: u32) -> String {
    let link = if transport == divecomputer_web::TRANSPORT_BLE {
        "bluetooth"
    } else {
        "serial"
    };
    divecomputer::fingerprint_key(&model.vendor, &model.product, link)
}
