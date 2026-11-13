//! Cloud storage: the backend registry, the persisted configuration and the
//! open/save operations.
//!
//! The remote copy is used like a file: "open" replaces the local log with it
//! and "save" replaces it with the local log, optionally after every edit
//! (autosave). The decision logic lives in [`benthic_core::sync`]; this module
//! dispatches to the selected [`backend::Backend`] and persists the selection.
//!
//! Adding a backend means writing a module that implements
//! [`backend::Backend`] and listing it in [`backend::all`] — the dialog, the
//! persistence layer and the sync loop are all driven by that trait.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use benthic_core::sync::{self as plan, OpenPlan, RemoteFile, SavePlan, SyncState};
use benthic_core::DiveLog;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub mod backend;

mod gdrive;
mod git;
mod github;
mod http;

#[cfg(not(target_arch = "wasm32"))]
mod oauth_desktop;

#[cfg(target_arch = "wasm32")]
mod oauth_web;

mod subsurface_cloud;

#[cfg(test)]
mod test_server;

use backend::Settings;

/// The selected backend and its settings.
///
/// Settings are kept per backend so switching between services never discards
/// the other's tokens. The file name and revision bookkeeping are separate
/// ([`SyncState`]) because they are backend-independent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Id of the selected backend (see [`backend::all`]).
    pub provider: String,
    /// Per-backend settings, keyed by backend id.
    pub settings: BTreeMap<String, Settings>,
    /// Save to cloud storage automatically after every change.
    pub autosave: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            provider: backend::all()[0].id().to_string(),
            settings: BTreeMap::new(),
            autosave: false,
        }
    }
}

impl SyncConfig {
    /// The selected backend.
    pub fn backend(&self) -> &'static dyn backend::Backend {
        backend::by_id(&self.provider)
    }

    /// The selected backend's settings.
    pub fn settings(&self) -> &Settings {
        static EMPTY: OnceLock<Settings> = OnceLock::new();
        self.settings
            .get(&self.provider)
            .unwrap_or_else(|| EMPTY.get_or_init(Settings::default))
    }

    /// The selected backend's settings, created on demand.
    pub fn settings_mut(&mut self) -> &mut Settings {
        self.settings.entry(self.provider.clone()).or_default()
    }

    /// Whether the settings are complete enough to attempt a non-interactive
    /// request.
    pub fn is_configured(&self) -> bool {
        self.backend().ready(self.settings())
    }
}

/// The stored sync configuration, or the default.
pub fn load_config() -> SyncConfig {
    let mut config: SyncConfig = crate::storage::load_sync()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    // Fall back when the stored backend is not offered by this build (Drive
    // is development-only).
    if backend::all().iter().all(|b| b.id() != config.provider) {
        config.provider = SyncConfig::default().provider;
    }
    config
}

pub fn save_config(config: &SyncConfig) {
    if let Ok(text) = serde_json::to_string(config) {
        let _ = crate::storage::save_sync(&text);
    }
}

pub fn load_state() -> SyncState {
    crate::storage::load_sync_state()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Bumped whenever the open/save bookkeeping changes, so views that compare
/// the log with the cloud copy recompute.
pub static STATE_REVISION: GlobalSignal<u64> = Signal::global(|| 0);

pub fn save_state(state: &SyncState) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = crate::storage::save_sync_state(&text);
    }
    *STATE_REVISION.write() += 1;
}

/// Fetch the selected backend's remote log, or `None` when absent.
///
/// Refreshes credentials first, so callers never have to.
pub async fn fetch(config: &mut SyncConfig) -> Result<Option<RemoteFile>, String> {
    let id = config.provider.clone();
    let backend = backend::by_id(&id);
    let settings = config.settings_mut();
    backend.refresh(settings).await?;
    backend.fetch(settings).await
}

/// Upload `content` to the selected backend.
pub async fn push(
    config: &mut SyncConfig,
    content: &str,
    revision: Option<&str>,
) -> Result<RemoteFile, String> {
    let id = config.provider.clone();
    let backend = backend::by_id(&id);
    let settings = config.settings_mut();
    backend.refresh(settings).await?;
    backend.push(settings, content, revision).await
}

/// Run the selected backend's interactive sign-in, if it has one.
pub async fn sign_in(config: &mut SyncConfig) -> Result<(), String> {
    let id = config.provider.clone();
    let backend = backend::by_id(&id);
    backend.sign_in(config.settings_mut()).await
}

/// Drop the selected backend's stored credentials.
pub async fn sign_out(config: &mut SyncConfig) {
    let id = config.provider.clone();
    let backend = backend::by_id(&id);
    backend.sign_out(config.settings_mut()).await;
}

/// Forget which remote copy the local log was last opened from or saved to,
/// e.g. after switching services.
pub fn reset_state() {
    save_state(&SyncState::default());
}

fn remember(revision: String, local_json: &str) {
    save_state(&SyncState {
        remote_revision: Some(revision),
        local_fingerprint: Some(plan::fingerprint(local_json)),
        last_sync_secs: crate::platform::now_secs(),
    });
}

/// The result of saving to cloud storage.
pub enum SaveOutcome {
    Written,
    UpToDate,
    /// The remote changed since it was last opened or saved here.
    Conflict,
}

/// Save `log` to cloud storage. Unless `force`, a remote copy that changed
/// since it was last opened or saved here is left alone and reported as a
/// conflict.
pub async fn save(
    config: &mut SyncConfig,
    log: &DiveLog,
    force: bool,
) -> Result<SaveOutcome, String> {
    let local = benthic_core::io::json::to_string(log).map_err(|error| error.to_string())?;
    let remote = fetch(config).await?;
    let state = load_state();
    let decision = if force {
        SavePlan::Save
    } else {
        plan::save_plan(&local, log.is_empty(), &state, remote.as_ref())
    };
    match decision {
        SavePlan::UpToDate => {
            if let Some(remote) = remote {
                remember(remote.revision, &local);
            }
            Ok(SaveOutcome::UpToDate)
        }
        SavePlan::Conflict => Ok(SaveOutcome::Conflict),
        SavePlan::Save => {
            let revision = remote.as_ref().map(|r| r.revision.clone());
            let pushed = push(config, &local, revision.as_deref()).await?;
            remember(pushed.revision, &local);
            Ok(SaveOutcome::Written)
        }
    }
}

/// The result of opening from cloud storage.
pub enum Opened {
    Log(Box<DiveLog>),
    Empty,
    /// The local log has changes that were never saved to cloud storage.
    ConfirmDiscard,
}

/// Fetch the cloud copy to open. Unless `force`, unsaved local changes are
/// reported instead of being discarded. Call [`opened`] once the returned log
/// has replaced the local one.
pub async fn open(config: &mut SyncConfig, log: &DiveLog, force: bool) -> Result<Opened, String> {
    if !force {
        let local = benthic_core::io::json::to_string(log).map_err(|error| error.to_string())?;
        if plan::open_plan(&local, log.is_empty(), &load_state()) == OpenPlan::ConfirmDiscard {
            return Ok(Opened::ConfirmDiscard);
        }
    }
    let Some(remote) = fetch(config).await? else {
        return Ok(Opened::Empty);
    };
    let parsed =
        benthic_core::io::parse_auto(&remote.content).map_err(|error| error.to_string())?;
    save_state(&SyncState {
        remote_revision: Some(remote.revision),
        ..SyncState::default()
    });
    Ok(Opened::Log(Box::new(parsed)))
}

/// Record that the local log now matches the copy just opened.
pub fn opened(log: &DiveLog) {
    let mut state = load_state();
    if let (Some(revision), Ok(local)) = (
        state.remote_revision.take(),
        benthic_core::io::json::to_string(log),
    ) {
        remember(revision, &local);
    }
}

/// Whether the local log has changes not yet saved to cloud storage.
pub fn has_unsaved_changes(log: &DiveLog) -> bool {
    benthic_core::io::json::to_string(log)
        .map(|local| plan::has_unsaved_changes(&local, &load_state()))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut config = SyncConfig {
            provider: "gdrive".to_string(),
            ..SyncConfig::default()
        };
        config.settings_mut().set("file_id", "abc");
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<SyncConfig>(&text).unwrap(), config);
    }
}
