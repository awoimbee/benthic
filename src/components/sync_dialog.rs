use dioxus::prelude::*;

use benthic_core::sync::{self as core_sync, SyncPlan};

use crate::state::AppState;
use crate::sync::{self, Provider, SyncConfig};

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
async fn run(state: AppState, config: SyncConfig, mode: Mode) -> Result<Outcome, String> {
    let log = (state.log)();
    let local = benthic_core::io::json::to_string(&log).map_err(|e| e.to_string())?;
    let remote = sync::fetch(&config).await?;
    let mut bookmark = sync::load_state();
    let plan = match mode {
        Mode::Auto => core_sync::plan(&local, log.is_empty(), &bookmark, remote.as_ref()),
        Mode::ForcePush => SyncPlan::Push,
        Mode::ForcePull => SyncPlan::Pull,
    };
    match plan {
        SyncPlan::UpToDate => Ok(Outcome::Done("Already up to date.".to_string())),
        SyncPlan::Push => {
            let revision = remote.as_ref().map(|r| r.revision.clone());
            let pushed = sync::push(&config, &local, revision.as_deref()).await?;
            bookmark.remote_revision = Some(pushed.revision);
            bookmark.local_fingerprint = Some(core_sync::fingerprint(&local));
            bookmark.last_sync_secs = crate::platform::now_secs();
            sync::save_state(&bookmark);
            Ok(Outcome::Done("Uploaded the local log.".to_string()))
        }
        SyncPlan::Pull => {
            let remote = remote.ok_or_else(|| "The remote is empty.".to_string())?;
            let parsed =
                benthic_core::io::parse_auto(&remote.content).map_err(|e| e.to_string())?;
            let count = parsed.dives.len();
            if let Some(first) = parsed.dives_recent_first().first().map(|d| d.id) {
                let mut selected = state.selected;
                selected.set(Some(first));
            }
            let mut log = state.log;
            log.set(parsed);
            bookmark.remote_revision = Some(remote.revision);
            bookmark.local_fingerprint = Some(core_sync::fingerprint(&remote.content));
            bookmark.last_sync_secs = crate::platform::now_secs();
            sync::save_state(&bookmark);
            state.set_status(format!("Downloaded {count} dives from the remote"));
            Ok(Outcome::Done("Downloaded the remote log.".to_string()))
        }
        SyncPlan::Conflict => Ok(Outcome::Conflict),
    }
}

/// A dialog to configure and run a remote sync.
#[component]
pub fn SyncDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_sync = state.show_sync;
    let mut config = use_signal(sync::load_config);
    let mut status = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut conflict = use_signal(|| false);

    let cfg = (config)();
    let provider_index = match cfg.provider {
        Provider::GitHub => 0,
        Provider::GoogleDrive => 1,
    };
    let github = cfg.provider == Provider::GitHub;

    let mut launch = move |mode: Mode| {
        let snapshot = (config)();
        sync::save_config(&snapshot);
        busy.set(true);
        conflict.set(false);
        status.set(Some("Syncing…".to_string()));
        spawn(async move {
            match run(state, snapshot, mode).await {
                Ok(Outcome::Done(message)) => status.set(Some(message)),
                Ok(Outcome::Conflict) => {
                    conflict.set(true);
                    status.set(Some(
                        "Both the local and the remote logs changed. Choose which to keep."
                            .to_string(),
                    ));
                }
                Err(error) => {
                    status.set(Some(error));
                }
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| show_sync.set(false),
            div { class: "modal wide", onclick: move |evt| evt.stop_propagation(),
                h2 { "Sync" }
                p { class: "muted",
                    "Mirror the log to a file in a Git repository (GitHub, or any compatible API) or in Google Drive's app data folder."
                }
                div { class: "edit-form",
                    label { class: "field-label", "Service"
                        select {
                            class: "field",
                            value: "{provider_index}",
                            onchange: move |evt| {
                                let index: usize = evt.value().parse().unwrap_or(0);
                                config.write().provider = if index == 1 { Provider::GoogleDrive } else { Provider::GitHub };
                            },
                            option { value: "0", "GitHub" }
                            option { value: "1", "Google Drive" }
                        }
                    }
                    if github {
                        label { class: "field-label", "Repository (owner/name)"
                            input {
                                class: "field",
                                value: "{cfg.repo}",
                                placeholder: "awoimbee/benthic-dives",
                                oninput: move |evt| config.write().repo = evt.value(),
                            }
                        }
                        label { class: "field-label", "File path"
                            input {
                                class: "field",
                                value: "{cfg.path}",
                                oninput: move |evt| config.write().path = evt.value(),
                            }
                        }
                        label { class: "field-label", "Branch"
                            input {
                                class: "field",
                                value: "{cfg.branch}",
                                oninput: move |evt| config.write().branch = evt.value(),
                            }
                        }
                    }
                    label { class: "field-label", "Access token"
                        input {
                            class: "field",
                            r#type: "password",
                            value: "{cfg.token}",
                            oninput: move |evt| config.write().token = evt.value(),
                        }
                    }
                    label { class: "field-label", "API base (optional)"
                        input {
                            class: "field",
                            value: "{cfg.base_url}",
                            placeholder: "leave blank for the public API",
                            oninput: move |evt| config.write().base_url = evt.value(),
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
                            "Keep local (upload)"
                        }
                        button {
                            class: "btn",
                            disabled: (busy)(),
                            onclick: move |_| launch(Mode::ForcePull),
                            "Keep remote (download)"
                        }
                    } else {
                        button {
                            class: "btn primary",
                            disabled: (busy)() || !cfg.is_configured(),
                            onclick: move |_| launch(Mode::Auto),
                            "Sync now"
                        }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| show_sync.set(false),
                        "Close"
                    }
                }
            }
        }
    }
}
