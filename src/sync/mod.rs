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
#[derive(Debug, Clone, PartialEq, Serialize)]
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

// `SyncConfig` is deserialized by hand so old flat configs keep working.
impl<'de> Deserialize<'de> for SyncConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(from_value(value))
    }
}

/// True for the provider names used before the backend registry existed.
fn is_legacy_provider(provider: &str) -> bool {
    matches!(provider, "GitHub" | "GoogleDrive")
}

fn from_value(value: serde_json::Value) -> SyncConfig {
    let provider = value.get("provider").and_then(|p| p.as_str()).unwrap_or("");
    if is_legacy_provider(provider) {
        if let Ok(legacy) = serde_json::from_value::<LegacyConfig>(value.clone()) {
            return legacy.into();
        }
    }
    #[derive(Deserialize)]
    struct Fresh {
        #[serde(default)]
        provider: String,
        #[serde(default)]
        settings: BTreeMap<String, Settings>,
    }
    let fresh: Fresh = serde_json::from_value(value).unwrap_or(Fresh {
        provider: String::new(),
        settings: BTreeMap::new(),
    });
    let mut config = SyncConfig {
        provider: fresh.provider,
        settings: fresh.settings,
    };
    if backend::all().iter().all(|b| b.id() != config.provider) {
        config.provider = backend::all()[0].id().to_string();
    }
    config
}

/// The configuration format used before the backend registry.
#[derive(Debug, Default, Deserialize)]
struct LegacyConfig {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    token_expiry: i64,
    #[serde(default)]
    file_id: String,
}

impl From<LegacyConfig> for SyncConfig {
    fn from(legacy: LegacyConfig) -> Self {
        let mut config = SyncConfig::default();
        if legacy.provider == "GoogleDrive" {
            config.provider = "gdrive".to_string();
            let settings = config.settings_mut();
            if !legacy.client_id.is_empty() {
                settings.set("client_id", legacy.client_id);
            }
            if !legacy.token.is_empty() {
                settings.set("access_token", legacy.token);
            }
            if legacy.token_expiry != 0 {
                settings.set("expires_at", legacy.token_expiry);
            }
            if !legacy.file_id.is_empty() {
                settings.set("file_id", legacy.file_id);
            }
            if !legacy.base_url.is_empty() {
                settings.set("base_url", legacy.base_url);
            }
        } else {
            config.provider = "github".to_string();
            let settings = config.settings_mut();
            if !legacy.repo.is_empty() {
                settings.set("repo", legacy.repo);
            }
            if !legacy.path.is_empty() {
                settings.set("path", legacy.path);
            }
            if !legacy.branch.is_empty() {
                settings.set("branch", legacy.branch);
            }
            if !legacy.token.is_empty() {
                settings.set("token", legacy.token);
            }
            if !legacy.base_url.is_empty() {
                settings.set("base_url", legacy.base_url);
            }
        }
        config
    }
}

pub fn load_config() -> SyncConfig {
    crate::storage::load_sync()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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
    fn round_trips_the_new_format() {
        let mut config = SyncConfig {
            provider: "gdrive".to_string(),
            ..SyncConfig::default()
        };
        config.settings_mut().set("file_id", "abc");
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<SyncConfig>(&text).unwrap(), config);
    }

    #[test]
    fn migrates_a_legacy_github_config() {
        let legacy = r#"{"provider":"GitHub","base_url":"","repo":"me/dives","path":"benthic.json","branch":"main","token":"tok","client_id":"","token_expiry":0,"file_id":""}"#;
        let config: SyncConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(config.provider, "github");
        let settings = config.settings();
        assert_eq!(settings.text("repo"), "me/dives");
        assert_eq!(settings.text("token"), "tok");
        assert_eq!(settings.text("branch"), "main");
    }

    #[test]
    fn migrates_a_legacy_drive_config() {
        let legacy = r#"{"provider":"GoogleDrive","base_url":"","repo":"","path":"","branch":"","token":"access","client_id":"cid","token_expiry":123,"file_id":"fid"}"#;
        let config: SyncConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(config.provider, "gdrive");
        let settings = config.settings();
        assert_eq!(settings.text("client_id"), "cid");
        assert_eq!(settings.text("access_token"), "access");
        assert_eq!(settings.i64("expires_at"), 123);
        assert_eq!(settings.text("file_id"), "fid");
    }

    #[test]
    fn unknown_providers_fall_back_to_the_first_backend() {
        let config: SyncConfig = serde_json::from_str(r#"{"provider":"ftp"}"#).unwrap();
        assert_eq!(config.provider, backend::all()[0].id());
    }
}
