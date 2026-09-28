//! Download dives from a dive computer (desktop only).
//!
//! libdivecomputer is blocking and can spend seconds on the bus, so scans and
//! downloads run on a worker thread and report back through a coroutine. The
//! dialog never blocks the UI.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dioxus::prelude::*;
use futures_util::StreamExt;

use benthic_core::Dive;
use benthic_divecomputer::{
    descriptors, download, scan, DeviceDescriptor, DeviceEvent, DiscoveredDevice, Download,
    Transport,
};

use crate::actions;
use crate::state::AppState;

/// Messages from the worker thread back to the UI.
enum Task {
    Scanned(Result<Vec<DiscoveredDevice>, String>),
    Event(DeviceEvent),
    Finished(Result<Download, String>),
}

/// A dialog to find and download from a dive computer.
#[component]
pub fn DeviceDownloadDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show = state.show_download;

    // The model table is static; enumerate it once.
    let models = use_signal(|| descriptors().unwrap_or_default());
    let mut search = use_signal(String::new);
    let mut selected = use_signal(|| None::<DeviceDescriptor>);
    let mut transport = use_signal(|| Transport::Serial);
    let mut devices = use_signal(Vec::<DiscoveredDevice>::new);
    let mut selected_device = use_signal(|| None::<usize>);
    let mut scanning = use_signal(|| false);
    let mut downloading = use_signal(|| false);
    let mut progress = use_signal(|| None::<(u32, u32)>);
    let mut message = use_signal(|| Option::<String>::None);
    let mut cancel = use_signal(|| None::<Arc<AtomicBool>>);

    let task = use_coroutine(move |mut rx: UnboundedReceiver<Task>| async move {
        while let Some(task) = rx.next().await {
            match task {
                Task::Scanned(result) => {
                    scanning.set(false);
                    match result {
                        Ok(found) => {
                            let count = found.len();
                            selected_device.set((!found.is_empty()).then_some(0));
                            devices.set(found);
                            message.set(Some(format!("Found {count} device(s)")));
                        }
                        Err(error) => message.set(Some(format!("Scan failed: {error}"))),
                    }
                }
                Task::Event(DeviceEvent::Progress { current, maximum }) => {
                    progress.set(Some((current, maximum)));
                }
                Task::Event(_) => {}
                Task::Finished(result) => {
                    downloading.set(false);
                    cancel.set(None);
                    progress.set(None);
                    match result {
                        Ok(downloaded) => {
                            let Download {
                                dives,
                                latest_fingerprint,
                                ..
                            } = downloaded;
                            if let (Some(descriptor), Some(device)) = (
                                (selected)(),
                                (selected_device)()
                                    .and_then(|index| (devices)().get(index).cloned()),
                            ) {
                                if !latest_fingerprint.is_empty() {
                                    save_fingerprint(
                                        &fingerprint_key(&descriptor, &device),
                                        &latest_fingerprint,
                                    );
                                }
                            }
                            let dives: Vec<Dive> = dives.into_iter().map(|d| d.dive).collect();
                            let (added, skipped) = actions::merge_downloaded(state, dives);
                            state.set_status(format!(
                                "Downloaded {added} new dive(s) from the dive computer"
                            ));
                            message.set(Some(format!(
                                "Added {added} dive(s); skipped {skipped} already in the log"
                            )));
                        }
                        Err(error) => message.set(Some(format!("Download failed: {error}"))),
                    }
                }
            }
        }
    });

    let on_scan = move |_| {
        let Some(descriptor) = (selected)() else {
            message.set(Some("Choose a dive computer model first".to_string()));
            return;
        };
        let transport = (transport)();
        scanning.set(true);
        devices.set(Vec::new());
        selected_device.set(None);
        message.set(Some("Scanning…".to_string()));
        let sender = task.tx();
        std::thread::spawn(move || {
            let result = scan(&descriptor, transport).map_err(|e| e.to_string());
            let _ = sender.unbounded_send(Task::Scanned(result));
        });
    };

    let on_download = move |_| {
        let Some(descriptor) = (selected)() else {
            message.set(Some("Choose a dive computer model first".to_string()));
            return;
        };
        let Some(device) = (selected_device)().and_then(|index| (devices)().get(index).cloned())
        else {
            message.set(Some("Scan and choose a device first".to_string()));
            return;
        };
        let id = device.id.clone();
        let fingerprint = load_fingerprint(&fingerprint_key(&descriptor, &device));
        let flag = Arc::new(AtomicBool::new(false));
        cancel.set(Some(flag.clone()));
        downloading.set(true);
        progress.set(None);
        message.set(Some("Downloading…".to_string()));
        let sender = task.tx();
        std::thread::spawn(move || {
            let result = download(&descriptor, &id, &fingerprint, flag, |event| {
                let _ = sender.unbounded_send(Task::Event(event));
            })
            .map_err(|e| e.to_string());
            let _ = sender.unbounded_send(Task::Finished(result));
        });
    };

    let on_cancel = move |_| {
        if let Some(flag) = (cancel)() {
            flag.store(true, Ordering::Relaxed);
        }
        message.set(Some("Cancelling…".to_string()));
    };

    let selected_now = (selected)();
    let supported = selected_now
        .as_ref()
        .map(|d| d.transport_list())
        .unwrap_or_else(|| Transport::ALL.to_vec());

    let query = (search)().to_lowercase();
    let models_now = (models)();
    let model_buttons = models_now
        .iter()
        .filter(|model| query.is_empty() || model.name().to_lowercase().contains(&query))
        .take(200)
        .map(|model| {
            let click_model = model.clone();
            let is_selected = selected_now.as_ref() == Some(model);
            let transports = model
                .transport_list()
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(", ");
            rsx! {
                button {
                    class: if is_selected { "device-model selected" } else { "device-model" },
                    onclick: move |_| {
                        selected.set(Some(click_model.clone()));
                        let supported = click_model.transport_list();
                        if !supported.contains(&(transport)()) {
                            transport.set(supported.first().copied().unwrap_or(Transport::Serial));
                        }
                        devices.set(Vec::new());
                        selected_device.set(None);
                        message.set(None);
                    },
                    span { "{model.vendor} {model.product}" }
                    span { class: "muted", "{transports}" }
                }
            }
        });

    let devices_now = (devices)();
    let device_buttons = devices_now.iter().enumerate().map(|(index, device)| {
        let label = device.label();
        let is_selected = (selected_device)() == Some(index);
        rsx! {
            button {
                class: if is_selected { "device-entry selected" } else { "device-entry" },
                onclick: move |_| selected_device.set(Some(index)),
                "{label}"
            }
        }
    });

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| show.set(false),
            div { class: "modal wide", onclick: move |evt| evt.stop_propagation(),
                h2 { "Download from a dive computer" }
                p { class: "muted",
                    "Choose a model, scan the bus, then download the dives that are not in the log yet. Nothing is imported twice."
                }
                div { class: "device-columns",
                    div { class: "device-column",
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
                    div { class: "device-column",
                        label { class: "field-label", "Transport"
                            select {
                                class: "field",
                                onchange: move |evt| transport.set(transport_from_label(&evt.value())),
                                for option in supported {
                                    option {
                                        value: "{option.label()}",
                                        selected: option == (transport)(),
                                        "{option.label()}"
                                    }
                                }
                            }
                        }
                        div { class: "device-actions",
                            button {
                                class: "btn",
                                disabled: (scanning)() || (downloading)() || (selected)().is_none(),
                                onclick: on_scan,
                                if (scanning)() { "Scanning…" } else { "Scan" }
                            }
                        }
                        div { class: "device-devices", {device_buttons} }
                    }
                }
                if let Some((current, maximum)) = (progress)() {
                    div { class: "download-progress",
                        progress { value: "{current}", max: "{maximum}" }
                        span { class: "muted", "{current} / {maximum}" }
                    }
                }
                if let Some(text) = (message)() {
                    p { class: "muted", "{text}" }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        disabled: (downloading)() || (selected)().is_none() || (selected_device)().is_none(),
                        onclick: on_download,
                        if (downloading)() { "Downloading…" } else { "Download" }
                    }
                    if (downloading)() {
                        button { class: "btn danger", onclick: on_cancel, "Cancel" }
                    }
                    button { class: "btn", onclick: move |_| show.set(false), "Close" }
                }
            }
        }
    }
}

fn transport_from_label(label: &str) -> Transport {
    match label {
        "usb" => Transport::Usb,
        "usbhid" => Transport::UsbHid,
        "bluetooth" => Transport::Bluetooth,
        _ => Transport::Serial,
    }
}

/// Fingerprints are remembered per model and connection so a second download
/// only fetches new dives.
fn fingerprint_key(descriptor: &DeviceDescriptor, device: &DiscoveredDevice) -> String {
    format!("{} @ {}", descriptor.name(), device.id.address())
}

fn fingerprints() -> Option<BTreeMap<String, String>> {
    crate::storage::load_device_fingerprints().and_then(|text| serde_json::from_str(&text).ok())
}

fn load_fingerprint(key: &str) -> Vec<u8> {
    fingerprints()
        .and_then(|map| map.get(key).map(|hex| from_hex(hex)))
        .unwrap_or_default()
}

fn save_fingerprint(key: &str, value: &[u8]) {
    let mut map = fingerprints().unwrap_or_default();
    map.insert(key.to_string(), to_hex(value));
    if let Ok(text) = serde_json::to_string(&map) {
        let _ = crate::storage::save_device_fingerprints(&text);
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn from_hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}
