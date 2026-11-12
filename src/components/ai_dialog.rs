//! Minimal in-app LLM prompt panel with a local search tool.

use dioxus::prelude::*;

use crate::ai::AiProvider;
use crate::i18n;
use crate::state::AppState;

#[component]
pub fn AiDialog() -> Element {
    let state = use_context::<AppState>();
    let t = i18n::strings((state.prefs)().language);

    let mut config = use_signal(crate::ai::load_config);
    let mut prompt = use_signal(String::new);
    let mut answer = use_signal(|| None::<String>);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut saved = use_signal(|| false);
    let current = (config)();
    let provider_index = AiProvider::ALL
        .iter()
        .position(|provider| *provider == current.provider)
        .unwrap_or(0);

    let on_save = move |_| {
        let cfg = (config)();
        match crate::ai::save_config(&cfg) {
            Ok(()) => {
                saved.set(true);
                error.set(None);
            }
            Err(err) => error.set(Some(err)),
        }
    };

    let on_send = move |_| {
        let cfg = (config)();
        if cfg.api_key.trim().is_empty() {
            error.set(Some(t.ai_need_key.to_string()));
            return;
        }
        let text = (prompt)().trim().to_string();
        if text.is_empty() || (busy)() {
            return;
        }
        let _ = crate::ai::save_config(&cfg);
        saved.set(true);
        busy.set(true);
        answer.set(None);
        error.set(None);
        let log = (state.log)();
        let mut answer = answer;
        let mut error = error;
        let mut busy = busy;
        spawn(async move {
            match crate::ai::run_prompt(&cfg, log, text).await {
                Ok(text) => answer.set(Some(text)),
                Err(err) => error.set(Some(err)),
            }
            busy.set(false);
        });
    };

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| state.close_dialog(),
            div {
                class: "modal wide ai-dialog",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{t.ai_assistant}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{t.ai_assistant}" }
                p { class: "muted", "{t.ai_intro}" }
                div { class: "edit-form",
                    label { class: "field-label", "{t.ai_provider}"
                        select {
                            class: "field",
                            value: "{provider_index}",
                            onchange: move |evt| {
                                let index = evt.value().parse::<usize>().unwrap_or(0);
                                if let Some(provider) = AiProvider::ALL.get(index).copied() {
                                    let mut cfg = config.write();
                                    cfg.provider = provider;
                                    cfg.model = provider.default_model().to_string();
                                }
                                saved.set(false);
                            },
                            for (index, provider) in AiProvider::ALL.iter().enumerate() {
                                option {
                                    key: "{index}",
                                    value: "{index}",
                                    selected: current.provider == *provider,
                                    "{provider.label()}"
                                }
                            }
                        }
                    }
                    label { class: "field-label", "{t.ai_model}"
                        input {
                            class: "field",
                            value: "{current.model}",
                            oninput: move |evt| {
                                config.write().model = evt.value();
                                saved.set(false);
                            },
                        }
                    }
                    label { class: "field-label full", "{t.ai_api_key}"
                        input {
                            class: "field",
                            r#type: "password",
                            autocomplete: "off",
                            placeholder: "{t.ai_api_key_placeholder}",
                            value: "{current.api_key}",
                            oninput: move |evt| {
                                config.write().api_key = evt.value();
                                saved.set(false);
                            },
                        }
                    }
                    label { class: "field-label full", "{t.ai_prompt}"
                        textarea {
                            class: "field",
                            rows: "3",
                            placeholder: "{t.ai_prompt_placeholder}",
                            value: "{prompt}",
                            oninput: move |evt| prompt.set(evt.value()),
                        }
                    }
                }
                p { class: "muted small-note", "{t.ai_cors_note}" }
                p { class: "muted small-note", "{t.ai_privacy_note}" }
                div { class: "detail-actions",
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: on_save,
                        "{t.ai_save}"
                    }
                    button {
                        class: "btn primary",
                        r#type: "button",
                        disabled: (busy)(),
                        onclick: on_send,
                        if (busy)() { "…" } else { "{t.ai_send}" }
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| state.close_dialog(),
                        "{t.close}"
                    }
                }
                if saved() {
                    p { class: "muted", "{t.ai_saved}" }
                }
                if let Some(err) = (error)() {
                    p { class: "warn", "{err}" }
                }
                if let Some(text) = (answer)() {
                    div { class: "ai-answer",
                        div { class: "field-label", "{t.ai_answer}" }
                        pre { "{text}" }
                    }
                }
            }
        }
    }
}
