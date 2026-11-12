//! Pieces shared by the desktop and web dive-computer download dialogs.
#![cfg(any(feature = "divecomputer", target_arch = "wasm32"))]

use dioxus::prelude::*;

use benthic_core::Dive;

use crate::state::AppState;

/// One entry of the model list.
#[derive(Clone, PartialEq)]
pub struct ModelEntry {
    pub label: String,
    pub detail: String,
    pub selected: bool,
    /// Why the model cannot be chosen right now, if it cannot.
    pub unavailable: Option<&'static str>,
}

/// The modal frame, title and intro of a download dialog.
#[component]
pub fn DownloadFrame(intro: &'static str, children: Element) -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| state.close_dialog(),
            div {
                class: "modal wide",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{tr.dc_title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.dc_title}" }
                p { class: "muted", "{intro}" }
                {children}
            }
        }
    }
}

/// A searchable list of models. `on_pick` receives the index into `models`.
#[component]
pub fn ModelPicker(
    search: Signal<String>,
    models: Vec<ModelEntry>,
    on_pick: EventHandler<usize>,
) -> Element {
    let tr = use_context::<AppState>().strings();
    let query = search().to_lowercase();
    rsx! {
        label { class: "field-label", "{tr.dc_model}"
            input {
                class: "field",
                r#type: "search",
                placeholder: "{tr.dc_search_models}",
                value: "{search}",
                oninput: move |evt| search.set(evt.value()),
            }
        }
        div { class: "device-models",
            for (index, model) in models.into_iter().enumerate().filter(|(_, model)| {
                query.is_empty() || model.label.to_lowercase().contains(&query)
            }) {
                button {
                    key: "{model.label}",
                    class: match (model.unavailable.is_some(), model.selected) {
                        (true, _) => "device-model unavailable",
                        (false, true) => "device-model selected",
                        (false, false) => "device-model",
                    },
                    disabled: model.unavailable.is_some(),
                    title: model.unavailable.unwrap_or_default(),
                    onclick: move |_| on_pick.call(index),
                    span { "{model.label}" }
                    if !model.detail.is_empty() {
                        span { class: "muted", "{model.detail}" }
                    }
                }
            }
        }
    }
}

/// The progress bar and status line.
#[component]
pub fn DownloadStatus(progress: Option<(u32, u32)>, message: Option<String>) -> Element {
    rsx! {
        if let Some((current, maximum)) = progress {
            div { class: "download-progress",
                progress { value: "{current}", max: "{maximum}" }
                span { class: "muted", "{current} / {maximum}" }
            }
        }
        if let Some(text) = message {
            p { class: "muted", "{text}" }
        }
    }
}

/// Close the dialog and hand the downloaded dives to the import review.
pub fn finish(
    state: AppState,
    dives: Vec<Dive>,
    source: String,
    fingerprint: Option<(String, Vec<u8>)>,
) {
    state.close_dialog();
    crate::actions::review_downloaded(state, dives, source, fingerprint);
}
