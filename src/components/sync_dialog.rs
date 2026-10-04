use dioxus::prelude::*;

use benthic_core::sync::{self as core_sync, SyncPlan};

use crate::i18n;
use crate::state::AppState;
use crate::sync::backend::{self, AuthKind};
use crate::sync::SyncConfig;

/// What a sync attempt decided to do.
enum Mode {
    Auto,
    ForcePush,
    ForcePull,
}

enum Outcome {
    Done(String),
    Conflict,
}

/// Fetch, plan, then push or pull. Conflicting edits are never overwritten
/// silently; the caller offers the user a choice.
async fn run(state: AppState, mut config: SyncConfig, mode: Mode) -> Result<Outcome, String> {
    let log = (state.log)();
    let local = benthic_core::io::json::to_string(&log).map_err(|error| error.to_string())?;
    let remote = crate::sync::fetch(&mut config).await?;
    let mut bookmark = crate::sync::load_state();
    let plan = match mode {
        Mode::Auto => core_sync::plan(&local, log.is_empty(), &bookmark, remote.as_ref()),
        Mode::ForcePush => SyncPlan::Push,
        Mode::ForcePull => SyncPlan::Pull,
    };
    let t = state.strings();
    match plan {
        SyncPlan::UpToDate => Ok(Outcome::Done(t.sync_up_to_date.to_string())),
        SyncPlan::Push => {
            let revision = remote.as_ref().map(|r| r.revision.clone());
            let pushed = crate::sync::push(&mut config, &local, revision.as_deref()).await?;
            bookmark.remote_revision = Some(pushed.revision);
            bookmark.local_fingerprint = Some(core_sync::fingerprint(&local));
            bookmark.last_sync_secs = crate::platform::now_secs();
            crate::sync::save_state(&bookmark);
            Ok(Outcome::Done(t.sync_uploaded.to_string()))
        }
        SyncPlan::Pull => {
            let remote = remote.ok_or_else(|| t.sync_remote_empty.to_string())?;
            let parsed =
                benthic_core::io::parse_auto(&remote.content).map_err(|error| error.to_string())?;
            let count = parsed.dives.len();
            if let Some(first) = parsed.dives_recent_first().first().map(|d| d.id) {
                let mut selected = state.selected;
                selected.set(Some(first));
            }
            // Go through the command stack so "Keep remote" is undoable like
            // every other change, instead of silently replacing the log.
            let before = (state.log)();
            state.dispatch(benthic_core::Command::Snapshot {
                label: "Sync: download remote".into(),
                before: Box::new(before),
                after: Box::new(parsed),
            });
            bookmark.remote_revision = Some(remote.revision);
            bookmark.local_fingerprint = Some(core_sync::fingerprint(&remote.content));
            bookmark.last_sync_secs = crate::platform::now_secs();
            crate::sync::save_state(&bookmark);
            state.set_status(i18n::t1(t.downloaded_remote, count));
            Ok(Outcome::Done(t.sync_downloaded.to_string()))
        }
        SyncPlan::Conflict => Ok(Outcome::Conflict),
    }
}

/// A dialog to configure and run a remote sync.
#[component]
pub fn SyncDialog() -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    let mut show_sync = state.show_sync;
    let mut config = use_signal(crate::sync::load_config);
    let mut status = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut conflict = use_signal(|| false);

    let cfg = (config)();
    let backend = cfg.backend();
    let signed_in = backend.signed_in(cfg.settings());
    let oauth = backend.auth() == AuthKind::OAuth;
    let visible = backend
        .fields()
        .iter()
        .filter(|field| field.platforms.current())
        .collect::<Vec<_>>();
    let normal = visible
        .iter()
        .copied()
        .filter(|field| !field.advanced)
        .collect::<Vec<_>>();
    let advanced = visible
        .iter()
        .copied()
        .filter(|field| field.advanced)
        .collect::<Vec<_>>();

    let mut launch = move |mode: Mode| {
        let mut snapshot = (config)();
        busy.set(true);
        conflict.set(false);
        status.set(Some(tr.sync_syncing.to_string()));
        spawn(async move {
            // OAuth backends sign in interactively before the first request.
            let needs_sign_in = {
                let backend = snapshot.backend();
                backend.auth() == AuthKind::OAuth && !backend.signed_in(snapshot.settings())
            };
            if needs_sign_in {
                if let Err(error) = crate::sync::sign_in(&mut snapshot).await {
                    status.set(Some(error));
                    busy.set(false);
                    return;
                }
                config.set(snapshot.clone());
            }
            crate::sync::save_config(&snapshot);
            match run(state, snapshot, mode).await {
                Ok(Outcome::Done(message)) => status.set(Some(message)),
                Ok(Outcome::Conflict) => {
                    conflict.set(true);
                    status.set(Some(tr.sync_conflict.to_string()));
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
                    config.set(snapshot.clone());
                    crate::sync::save_config(&snapshot);
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
            config.set(snapshot.clone());
            crate::sync::save_config(&snapshot);
            status.set(Some(tr.sync_signed_out.to_string()));
        });
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| show_sync.set(false),
            div { class: "modal wide",
                role: "dialog",
            tabindex: "-1",
            autofocus: true,
                aria_modal: "true",
                aria_label: "{tr.sync_title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.sync_title}" }
                p { class: "muted", "{tr.sync_intro}" }
                div { class: "edit-form",
                    label { class: "field-label", "{tr.sync_service}"
                        select {
                            class: "field",
                            onchange: move |evt| config.write().provider = evt.value(),
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
                        label { class: "field-label", "{field.label}"
                            input {
                                class: "field",
                                r#type: field.kind.input_type(),
                                value: "{cfg.settings().text(field.key)}",
                                placeholder: field.placeholder,
                                oninput: move |evt| config.write().settings_mut().set(field.key, evt.value()),
                            }
                        }
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
                                class: "btn primary",
                                disabled: (busy)() || !cfg.is_configured(),
                                onclick: do_sign_in,
                                "{backend.sign_in_label()}"
                            }
                        }
                    }
                }
                if !advanced.is_empty() {
                    details { class: "sync-advanced",
                        summary { "{tr.sync_advanced}" }
                        div { class: "edit-form",
                            for field in advanced {
                                label { class: "field-label", "{field.label}"
                                    input {
                                        class: "field",
                                        r#type: field.kind.input_type(),
                                        value: "{cfg.settings().text(field.key)}",
                                        placeholder: field.placeholder,
                                        oninput: move |evt| config.write().settings_mut().set(field.key, evt.value()),
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(message) = (status)() {
                    p { class: "muted", "{message}" }
                }
                div { class: "detail-actions",
                    if (conflict)() {
                        button {
                            class: "btn primary",
                            disabled: (busy)(),
                            onclick: move |_| launch(Mode::ForcePush),
                            "{tr.sync_keep_local}"
                        }
                        button {
                            class: "btn",
                            disabled: (busy)(),
                            onclick: move |_| launch(Mode::ForcePull),
                            "{tr.sync_keep_remote}"
                        }
                    } else {
                        button {
                            class: "btn primary",
                            disabled: (busy)() || !cfg.is_configured(),
                            onclick: move |_| launch(Mode::Auto),
                            "{tr.sync_now}"
                        }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| show_sync.set(false),
                        "{tr.close}"
                    }
                }
            }
        }
    }
}
