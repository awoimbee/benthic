//! Download dives from a dive computer (desktop only).
//!
//! libdivecomputer is blocking and can spend seconds on the bus, so scans and
//! downloads run on a worker thread and report back through a coroutine. The
//! dialog never blocks the UI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dioxus::prelude::*;
use futures_util::StreamExt;

use benthic_core::Dive;
use benthic_divecomputer::{
    descriptors, download, scan, DeviceDescriptor, DeviceEvent, DeviceId, DiscoveredDevice,
    Download, Transport,
};

use super::device_download_common::{
    finish, DownloadFrame, DownloadStatus, ModelEntry, ModelPicker,
};
use crate::divecomputer::{self as dc, MOCK_PRODUCT, MOCK_VENDOR};
use crate::i18n;
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
    let tr = state.strings();

    // The model table is static; enumerate it once, with the mock source first.
    let models = use_signal(|| {
        let mut list = descriptors().unwrap_or_default();
        list.insert(
            0,
            DeviceDescriptor {
                vendor: MOCK_VENDOR.to_string(),
                product: MOCK_PRODUCT.to_string(),
                family: 0,
                model: 0,
                transports: Transport::Serial.bit() | Transport::Ble.bit(),
            },
        );
        list
    });
    let search = use_signal(String::new);
    let mut selected = use_signal(|| None::<DeviceDescriptor>);
    let mut transport = use_signal(|| Transport::Serial);
    let mut devices = use_signal(Vec::<DiscoveredDevice>::new);
    let mut selected_device = use_signal(|| None::<usize>);
    let mut scanning = use_signal(|| false);
    let mut downloading = use_signal(|| false);
    let mut progress = use_signal(|| None::<(u32, u32)>);
    let mut message = use_signal(|| Option::<String>::None);
    let mut cancel = use_signal(|| None::<Arc<AtomicBool>>);

    // The chosen model and device, if both are set.
    let current = move || {
        let descriptor = (selected)()?;
        let device = (devices)().get((selected_device)()?)?.clone();
        Some((descriptor, device))
    };
    let mut show_mock = move || {
        devices.set(vec![mock_device()]);
        selected_device.set(Some(0));
        message.set(Some(tr.dc_mock_ready.to_string()));
    };

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
                            message.set(Some(i18n::t1(tr.dc_found_devices, count)));
                        }
                        Err(error) => message.set(Some(i18n::t1(tr.dc_scan_failed, error))),
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
                            let Some((descriptor, device)) = current() else {
                                continue;
                            };
                            let fingerprint = (!latest_fingerprint.is_empty()).then(|| {
                                (fingerprint_key(&descriptor, &device), latest_fingerprint)
                            });
                            let dives: Vec<Dive> = dives.into_iter().map(|d| d.dive).collect();
                            finish(state, dives, descriptor.name(), fingerprint);
                        }
                        Err(error) => message.set(Some(i18n::t1(tr.dc_download_failed, error))),
                    }
                }
            }
        }
    });

    let on_scan = move |_| {
        let Some(descriptor) = (selected)() else {
            message.set(Some(tr.dc_choose_model.to_string()));
            return;
        };
        if dc::is_mock(&descriptor.vendor, &descriptor.product) {
            show_mock();
            return;
        }
        let transport = (transport)();
        scanning.set(true);
        devices.set(Vec::new());
        selected_device.set(None);
        message.set(Some(tr.dc_scanning.to_string()));
        let sender = task.tx();
        std::thread::spawn(move || {
            let result = if transport == Transport::Ble {
                // libdivecomputer has no native BLE; we scan through BlueZ and
                // open the GATT stream ourselves.
                match benthic_divecomputer::ble::scan(std::time::Duration::from_secs(8)) {
                    Ok(found) => Ok(found
                        .into_iter()
                        .map(|device| DiscoveredDevice {
                            name: if device.name.is_empty() {
                                device.address.clone()
                            } else {
                                device.name
                            },
                            id: DeviceId::Ble {
                                address: device.address,
                            },
                        })
                        .collect()),
                    Err(error) => Err(error.to_string()),
                }
            } else {
                scan(&descriptor, transport).map_err(|e| e.to_string())
            };
            let _ = sender.unbounded_send(Task::Scanned(result));
        });
    };

    let on_download = move |_| {
        let Some(descriptor) = (selected)() else {
            message.set(Some(tr.dc_choose_model.to_string()));
            return;
        };
        if dc::is_mock(&descriptor.vendor, &descriptor.product) {
            finish(state, dc::mock_dives(), descriptor.name(), None);
            return;
        }
        let Some((descriptor, device)) = current() else {
            message.set(Some(tr.dc_choose_device.to_string()));
            return;
        };
        let id = device.id.clone();
        let fingerprint = dc::load_fingerprint(&fingerprint_key(&descriptor, &device));
        let flag = Arc::new(AtomicBool::new(false));
        cancel.set(Some(flag.clone()));
        downloading.set(true);
        progress.set(None);
        message.set(Some(tr.dc_downloading.to_string()));
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
        message.set(Some(tr.dc_cancelling.to_string()));
    };

    let selected_now = (selected)();
    let supported = selected_now
        .as_ref()
        .map(|d| d.transport_list())
        .unwrap_or_else(|| Transport::ALL.to_vec());

    let models_now = (models)();
    let entries: Vec<ModelEntry> = models_now
        .iter()
        .map(|model| ModelEntry {
            label: model.name(),
            detail: model
                .transport_list()
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(", "),
            selected: selected_now.as_ref() == Some(model),
            unavailable: None,
        })
        .collect();
    let on_pick = move |index: usize| {
        let Some(model) = (models)().get(index).cloned() else {
            return;
        };
        let supported = model.transport_list();
        if !supported.contains(&(transport)()) {
            transport.set(supported.first().copied().unwrap_or(Transport::Serial));
        }
        let mock = dc::is_mock(&model.vendor, &model.product);
        selected.set(Some(model));
        if mock {
            show_mock();
        } else {
            devices.set(Vec::new());
            selected_device.set(None);
            message.set(None);
        }
    };

    let on_forget_history = move |_| {
        let Some((descriptor, device)) = current() else {
            message.set(Some(tr.dc_choose_device.to_string()));
            return;
        };
        if dc::is_mock(&descriptor.vendor, &descriptor.product) {
            return;
        }
        dc::save_fingerprint(&fingerprint_key(&descriptor, &device), &[]);
        message.set(Some(tr.dc_history_cleared.to_string()));
    };

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

    let ready = !(downloading)() && current().is_some();

    rsx! {
        DownloadFrame { intro: tr.dc_intro_desktop,
            div { class: "device-columns",
                div { class: "device-column",
                    ModelPicker { search, models: entries, on_pick }
                }
                div { class: "device-column",
                    label { class: "field-label", "{tr.dc_transport}"
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
                            if (scanning)() { "{tr.dc_scanning}" } else { "{tr.dc_scan}" }
                        }
                    }
                    div { class: "device-devices", {device_buttons} }
                }
            }
            DownloadStatus { progress: (progress)(), message: (message)() }
            div { class: "detail-actions",
                button {
                    class: "btn",
                    disabled: !ready,
                    onclick: on_forget_history,
                    "{tr.dc_forget_history}"
                }
                button {
                    class: "btn primary",
                    disabled: !ready,
                    onclick: on_download,
                    if (downloading)() { "{tr.dc_downloading}" } else { "{tr.dc_download}" }
                }
                if (downloading)() {
                    button { class: "btn danger", onclick: on_cancel, "{tr.cancel}" }
                }
                button { class: "btn", onclick: move |_| state.close_dialog(), "{tr.close}" }
            }
        }
    }
}

fn mock_device() -> DiscoveredDevice {
    DiscoveredDevice {
        id: DeviceId::Serial("MOCK".to_string()),
        name: MOCK_PRODUCT.to_string(),
    }
}

fn fingerprint_key(descriptor: &DeviceDescriptor, device: &DiscoveredDevice) -> String {
    dc::fingerprint_key(
        &descriptor.vendor,
        &descriptor.product,
        &device.id.address(),
    )
}

fn transport_from_label(label: &str) -> Transport {
    match label {
        "usb" => Transport::Usb,
        "usbhid" => Transport::UsbHid,
        "bluetooth" => Transport::Bluetooth,
        _ => Transport::Serial,
    }
}
