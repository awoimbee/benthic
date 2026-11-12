//! Remote sync: the backend registry and the persisted configuration.
//!
//! The decision logic lives in [`benthic_core::sync`]; this module dispatches
//! to the selected [`backend::Backend`] and persists the selection.
//!
//! Adding a backend means writing a module that implements
//! [`backend::Backend`] and listing it in [`backend::all`] — the dialog, the
//! persistence layer and the sync loop are all driven by that trait.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use benthic_core::sync::{RemoteFile, SyncState};
use serde::{Deserialize, Serialize};

pub mod backend;

mod gdrive;
mod github;
mod http;

#[cfg(not(target_arch = "wasm32"))]
mod oauth_desktop;

#[cfg(target_arch = "wasm32")]
mod oauth_web;

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
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            provider: backend::all()[0].id().to_string(),
            settings: BTreeMap::new(),
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

pub fn save_state(state: &SyncState) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = crate::storage::save_sync_state(&text);
    }
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
