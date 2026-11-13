use dioxus::prelude::*;

use crate::components::InfoTip;
use crate::i18n;
use crate::state::AppState;
use crate::sync::backend::{self, AuthKind, Field, FieldHelp};
use crate::sync::{Opened, SaveOutcome, SyncConfig};

/// A pending question after an open or save was refused.
#[derive(Clone, Copy, PartialEq)]
pub enum Confirm {
    /// Opening would discard local changes never saved to cloud storage.
    Discard,
    /// The cloud copy changed since it was last opened or saved here.
    Overwrite,
}

/// Sign in to an OAuth backend if needed, then persist the settings.
async fn prepare(mut config: SyncConfig) -> Result<SyncConfig, String> {
    let needs_sign_in = {
        let backend = config.backend();
        backend.auth() == AuthKind::OAuth && !backend.signed_in(config.settings())
    };
    if needs_sign_in {
        crate::sync::sign_in(&mut config).await?;
    }
    crate::sync::save_config(&config);
    Ok(config)
}

/// Open the cloud copy, replacing the local log as one undoable step.
pub async fn open(
    state: AppState,
    config: SyncConfig,
    force: bool,
) -> Result<Result<String, Confirm>, String> {
    let t = state.strings();
    let mut config = prepare(config).await?;
    match crate::sync::open(&mut config, &(state.log)(), force).await? {
        Opened::ConfirmDiscard => Ok(Err(Confirm::Discard)),
        Opened::Empty => Ok(Ok(t.cloud_empty.to_string())),
        Opened::Log(log) => {
            let sites = log.sites.len();
            let label = i18n::t2(t.undo_open_cloud, log.dives.len(), sites);
            let count = crate::actions::replace_log(state, *log, label);
            crate::sync::opened(&(state.log)());
            let message = i18n::t1(t.cloud_opened, count);
            state.set_status(message.clone());
            Ok(Ok(message))
        }
    }
}

/// Save the local log to the cloud copy.
pub async fn save(
    state: AppState,
    config: SyncConfig,
    force: bool,
) -> Result<Result<String, Confirm>, String> {
    let t = state.strings();
    let mut config = prepare(config).await?;
    match crate::sync::save(&mut config, &(state.log)(), force).await? {
        SaveOutcome::Conflict => Ok(Err(Confirm::Overwrite)),
        SaveOutcome::UpToDate => Ok(Ok(t.cloud_up_to_date.to_string())),
        SaveOutcome::Written => Ok(Ok(t.cloud_saved.to_string())),
    }
}

/// Cloud storage: choose a service, open the log stored there or save to it,
/// and optionally save after every change.
#[component]
pub fn SyncDialog() -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    let mut config = state.cloud;
    let mut status = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut confirm = use_signal(|| None::<Confirm>);

    let cfg = (config)();
    let backend = cfg.backend();
    let signed_in = backend.signed_in(cfg.settings());
    let oauth = backend.auth() == AuthKind::OAuth;
    let ready = cfg.is_configured();
    let (advanced, normal): (Vec<&Field>, Vec<&Field>) = backend
        .fields()
        .iter()
        .filter(|field| field.platforms.current())
        .partition(|field| field.advanced);
    let field_input = move |field: &'static Field| {
        let help = field.help.map(|help| match help {
            FieldHelp::SubsurfaceServer => tr.tip_subsurface_server,
        });
        // The "?" is a button, so it must stay outside the <label>: a label
        // forwards clicks to its first button, which would reopen the tip.
        let id = format!("cloud-{}", field.key);
        rsx! {
            div { class: "field-label",
                span { class: "label-row",
                    label { r#for: "{id}", "{field.label}" }
                    if let Some(text) = help {
                        InfoTip { text }
                    }
                }
                input {
                    id: "{id}",
                    class: "field",
                    r#type: field.kind.input_type(),
                    value: "{config.read().settings().text(field.key)}",
                    placeholder: field.placeholder,
                    oninput: move |evt| config.write().settings_mut().set(field.key, evt.value()),
                }
            }
        }
    };

    let mut launch = move |action: Confirm, force: bool| {
        let snapshot = (config)();
        busy.set(true);
        confirm.set(None);
        status.set(Some(
            match action {
                Confirm::Discard => tr.cloud_opening,
                Confirm::Overwrite => tr.cloud_saving,
            }
            .to_string(),
        ));
        spawn(async move {
            let result = match action {
                Confirm::Discard => open(state, snapshot, force).await,
                Confirm::Overwrite => save(state, snapshot, force).await,
            };
            config.set(crate::sync::load_config());
            match result {
                Ok(Ok(message)) => status.set(Some(message)),
                Ok(Err(question)) => {
                    confirm.set(Some(question));
                    status.set(Some(
                        match question {
                            Confirm::Discard => tr.cloud_confirm_discard,
                            Confirm::Overwrite => tr.cloud_confirm_overwrite,
                        }
                        .to_string(),
                    ));
                }
                Err(error) => status.set(Some(error)),
            }
            busy.set(false);
        });
    };

    let do_sign_in = move |_| {
        let mut snapshot = (config)();
        busy.set(true);
        status.set(Some(tr.sync_signing_in.to_string()));
        spawn(async move {
            match crate::sync::sign_in(&mut snapshot).await {
                Ok(()) => {
                    crate::sync::save_config(&snapshot);
                    config.set(snapshot);
                    status.set(Some(tr.sync_signed_in_status.to_string()));
                }
                Err(error) => status.set(Some(error)),
            }
            busy.set(false);
        });
    };

    let do_sign_out = move |_| {
        let mut snapshot = (config)();
        spawn(async move {
            crate::sync::sign_out(&mut snapshot).await;
            snapshot.autosave = false;
            crate::sync::save_config(&snapshot);
            config.set(snapshot);
            status.set(Some(tr.sync_signed_out.to_string()));
        });
    };

    let on_provider = move |evt: Event<FormData>| {
        let mut next = (config)();
        if next.provider != evt.value() {
            next.provider = evt.value();
            next.autosave = false;
            crate::sync::save_config(&next);
            crate::sync::reset_state();
            config.set(next);
            confirm.set(None);
            status.set(None);
        }
    };

    let on_autosave = move |evt: Event<FormData>| {
        let mut next = (config)();
        next.autosave = evt.checked();
        crate::sync::save_config(&next);
        config.set(next);
    };

    let save_settings = move |_| crate::sync::save_config(&(config)());

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| state.close_dialog(),
            div { class: "modal wide",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{tr.cloud_title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.cloud_title}" }
                p { class: "muted", "{tr.cloud_intro}" }
                div { class: "edit-form", onfocusout: save_settings,
                    label { class: "field-label", "{tr.sync_service}"
                        select { class: "field", onchange: on_provider,
                            for candidate in backend::all() {
                                option {
                                    value: "{candidate.id()}",
                                    selected: candidate.id() == cfg.provider,
                                    "{candidate.name()}"
                                }
                            }
                        }
                    }
                    for field in normal {
                        {field_input(field)}
                    }
                    if oauth {
                        if signed_in {
                            div { class: "sync-account",
                                span { class: "muted", {i18n::t1(tr.sync_signed_in, backend.name())} }
                                button {
                                    class: "btn",
                                    disabled: (busy)(),
                                    onclick: do_sign_out,
                                    "{tr.sync_sign_out}"
                                }
                            }
                        } else {
                            button {
                                class: "btn",
                                disabled: (busy)() || !cfg.is_configured(),
                                onclick: do_sign_in,
                                "{backend.sign_in_label()}"
                            }
                        }
                    }
                }
                if !advanced.is_empty() {
                    details { class: "sync-advanced", onfocusout: save_settings,
                        summary { "{tr.sync_advanced}" }
                        div { class: "edit-form",
                            for field in advanced {
                                {field_input(field)}
                            }
                        }
                    }
                }
                div { class: "cloud-actions",
                    button {
                        class: "btn",
                        disabled: (busy)() || !ready,
                        onclick: move |_| launch(Confirm::Discard, false),
                        "{tr.cloud_open}"
                    }
                    button {
                        class: "btn primary",
                        disabled: (busy)() || !ready,
                        onclick: move |_| launch(Confirm::Overwrite, false),
                        "{tr.cloud_save}"
                    }
                    label { class: "check", title: "{tr.cloud_autosave_hint}",
                        input {
                            r#type: "checkbox",
                            checked: cfg.autosave,
                            disabled: !cfg.is_configured(),
                            onchange: on_autosave,
                        }
                        " {tr.cloud_autosave}"
                    }
                }
                if let Some(message) = (status)() {
                    p { class: "muted", role: "status", "{message}" }
                }
                if let Some(question) = (confirm)() {
                    div { class: "detail-actions",
                        button {
                            class: "btn danger",
                            disabled: (busy)(),
                            onclick: move |_| launch(question, true),
                            match question {
                                Confirm::Discard => tr.cloud_discard_and_open,
                                Confirm::Overwrite => tr.cloud_overwrite,
                            }
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                confirm.set(None);
                                status.set(None);
                            },
                            "{tr.cancel}"
                        }
                    }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn",
                        onclick: move |_| state.close_dialog(),
                        "{tr.close}"
                    }
                }
            }
        }
    }
}
