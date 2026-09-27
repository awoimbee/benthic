//! Remote sync: GitHub and Google Drive transports.
//!
//! The decision logic lives in [`benthic_core::sync`]; this module only moves
//! bytes. Each provider stores the whole log as one file, so a sync is a
//! fetch, a plan, and either an upload or a download. HTTP is only available
//! in the web build.

use benthic_core::sync::{RemoteFile, SyncState};
use serde::{Deserialize, Serialize};

/// Which service the log is mirrored to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    GitHub,
    GoogleDrive,
}

/// Where and how to reach the remote copy of the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub provider: Provider,
    /// API base URL; empty means the provider's public default.
    #[serde(default)]
    pub base_url: String,
    /// GitHub: `owner/repo`.
    #[serde(default)]
    pub repo: String,
    /// GitHub: path to the log file inside the repo.
    #[serde(default)]
    pub path: String,
    /// GitHub: branch to read and write.
    #[serde(default)]
    pub branch: String,
    /// A personal access token (GitHub) or OAuth access token (Drive).
    #[serde(default)]
    pub token: String,
    /// Google Drive: the file id once known.
    #[serde(default)]
    pub file_id: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            provider: Provider::GitHub,
            base_url: String::new(),
            repo: String::new(),
            path: "benthic.json".to_string(),
            branch: "main".to_string(),
            token: String::new(),
            file_id: String::new(),
        }
    }
}

impl SyncConfig {
    #[cfg(target_arch = "wasm32")]
    pub fn github_base(&self) -> &str {
        if self.base_url.is_empty() {
            "https://api.github.com"
        } else {
            &self.base_url
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn drive_base(&self) -> &str {
        if self.base_url.is_empty() {
            "https://www.googleapis.com"
        } else {
            &self.base_url
        }
    }

    pub fn is_configured(&self) -> bool {
        match self.provider {
            Provider::GitHub => {
                !self.repo.is_empty() && !self.path.is_empty() && !self.token.is_empty()
            }
            Provider::GoogleDrive => !self.token.is_empty(),
        }
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

/// Fetch the remote log, or `None` when it does not exist yet.
pub async fn fetch(config: &SyncConfig) -> Result<Option<RemoteFile>, String> {
    net::fetch(config).await
}

/// Upload `content`, replacing the file at `revision` (or creating it).
pub async fn push(
    config: &SyncConfig,
    content: &str,
    revision: Option<&str>,
) -> Result<RemoteFile, String> {
    net::push(config, content, revision).await
}

#[cfg(not(target_arch = "wasm32"))]
mod net {
    use super::*;

    pub async fn fetch(_config: &SyncConfig) -> Result<Option<RemoteFile>, String> {
        Err("Sync is only available in the web build.".to_string())
    }

    pub async fn push(
        _config: &SyncConfig,
        _content: &str,
        _revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        Err("Sync is only available in the web build.".to_string())
    }
}

#[cfg(target_arch = "wasm32")]
mod net {
    use super::{Provider, RemoteFile, SyncConfig};
    use base64::Engine;
    use gloo_net::http::Request;
    use serde_json::json;

    pub async fn fetch(config: &SyncConfig) -> Result<Option<RemoteFile>, String> {
        match config.provider {
            Provider::GitHub => github_fetch(config).await,
            Provider::GoogleDrive => drive_fetch(config).await,
        }
    }

    pub async fn push(
        config: &SyncConfig,
        content: &str,
        revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        match config.provider {
            Provider::GitHub => github_push(config, content, revision).await,
            Provider::GoogleDrive => drive_push(config, content, revision).await,
        }
    }

    // ---- GitHub ----------------------------------------------------------

    fn github_headers(request: gloo_net::http::RequestBuilder) -> gloo_net::http::RequestBuilder {
        request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    async fn github_fetch(config: &SyncConfig) -> Result<Option<RemoteFile>, String> {
        if config.repo.is_empty() || config.path.is_empty() {
            return Err("Set the GitHub repository and path first.".to_string());
        }
        let url = format!(
            "{}/repos/{}/contents/{}?ref={}",
            config.github_base(),
            config.repo,
            config.path,
            config.branch
        );
        let response = github_headers(Request::get(&url))
            .header("Authorization", &format!("Bearer {}", config.token))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status() == 404 {
            return Ok(None);
        }
        if !response.ok() {
            return Err(http_error("GitHub", response).await);
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let sha = body["sha"].as_str().unwrap_or_default().to_string();
        let encoded = body["content"]
            .as_str()
            .unwrap_or_default()
            .replace('\n', "");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| e.to_string())?;
        let content = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        Ok(Some(RemoteFile {
            content,
            revision: sha,
        }))
    }

    async fn github_push(
        config: &SyncConfig,
        content: &str,
        revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        let url = format!(
            "{}/repos/{}/contents/{}",
            config.github_base(),
            config.repo,
            config.path
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let mut body = json!({
            "message": "benthic: sync dive log",
            "content": encoded,
            "branch": config.branch,
        });
        if let Some(revision) = revision {
            body["sha"] = serde_json::Value::String(revision.to_string());
        }
        let response = github_headers(Request::put(&url))
            .header("Authorization", &format!("Bearer {}", config.token))
            .json(&body)
            .map_err(|e| e.to_string())?
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.ok() {
            return Err(http_error("GitHub", response).await);
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let sha = body["content"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(RemoteFile {
            content: content.to_string(),
            revision: sha,
        })
    }

    // ---- Google Drive ----------------------------------------------------

    async fn drive_find(config: &SyncConfig) -> Result<Option<(String, String)>, String> {
        let url = format!(
            "{}/drive/v3/files?spaces=appDataFolder&q=name%3D'benthic.json'&fields=files(id,version)",
            config.drive_base()
        );
        let response = Request::get(&url)
            .header("Authorization", &format!("Bearer {}", config.token))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.ok() {
            return Err(http_error("Google Drive", response).await);
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let Some(file) = body["files"].as_array().and_then(|files| files.first()) else {
            return Ok(None);
        };
        let id = file["id"].as_str().unwrap_or_default().to_string();
        let version = file["version"].as_str().unwrap_or_default().to_string();
        Ok(Some((id, version)))
    }

    async fn drive_fetch(config: &SyncConfig) -> Result<Option<RemoteFile>, String> {
        let Some((id, version)) = drive_find(config).await? else {
            return Ok(None);
        };
        let url = format!("{}/drive/v3/files/{}?alt=media", config.drive_base(), id);
        let response = Request::get(&url)
            .header("Authorization", &format!("Bearer {}", config.token))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.ok() {
            return Err(http_error("Google Drive", response).await);
        }
        let content = response.text().await.map_err(|e| e.to_string())?;
        Ok(Some(RemoteFile {
            content,
            revision: version,
        }))
    }

    async fn drive_push(
        config: &SyncConfig,
        content: &str,
        _revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        // Find or create the file, then replace its contents.
        let existing = if config.file_id.is_empty() {
            drive_find(config).await?.map(|(id, _)| id)
        } else {
            Some(config.file_id.clone())
        };
        let id = match existing {
            Some(id) => id,
            None => {
                let url = format!("{}/drive/v3/files?fields=id", config.drive_base());
                let metadata = json!({
                    "name": "benthic.json",
                    "parents": ["appDataFolder"],
                });
                let response = Request::post(&url)
                    .header("Authorization", &format!("Bearer {}", config.token))
                    .json(&metadata)
                    .map_err(|e| e.to_string())?
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.ok() {
                    return Err(http_error("Google Drive", response).await);
                }
                let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
                body["id"].as_str().unwrap_or_default().to_string()
            }
        };
        let url = format!(
            "{}/upload/drive/v3/files/{}?uploadType=media&fields=version",
            config.drive_base(),
            id
        );
        let response = Request::patch(&url)
            .header("Authorization", &format!("Bearer {}", config.token))
            .header("Content-Type", "application/json")
            .body(content.to_string())
            .map_err(|e| e.to_string())?
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.ok() {
            return Err(http_error("Google Drive", response).await);
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let version = body["version"].as_str().unwrap_or_default().to_string();
        Ok(RemoteFile {
            content: content.to_string(),
            revision: version,
        })
    }

    /// Read a response body for an error message, keeping it short.
    async fn http_error(service: &str, response: gloo_net::http::Response) -> String {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(300).collect();
        format!("{service} returned {status}: {body}")
    }
}
