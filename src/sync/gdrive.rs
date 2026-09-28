//! Google Drive backend, storing the log in the app's private `appDataFolder`.
//!
//! Sign-in differs by target: the web build uses Google Identity Services
//! (see `oauth_web.rs`), the desktop build uses the loopback + PKCE flow
//! (see `oauth_desktop.rs`).

use async_trait::async_trait;
use benthic_core::sync::RemoteFile;
use serde_json::json;

use super::backend::{AuthKind, Backend, Field, FieldKind, Platforms, Settings};
use super::http::{self, Method};

pub static GDRIVE: GoogleDrive = GoogleDrive;

pub struct GoogleDrive;

/// The Drive scope benthic needs: only its own private folder.
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";

/// OAuth client ID for the web build (a "Web application" client). This is a
/// public identifier, not a secret.
const WEB_CLIENT_ID: &str =
    "656029177705-9da78op3lrbstdof1a2q1pq0fb5014nf.apps.googleusercontent.com";

/// OAuth client for the desktop build: a "Desktop app" client, whose loopback
/// redirect needs its (non-confidential) secret at the token endpoint.
const DESKTOP_CLIENT_ID: &str = "";
#[cfg(not(target_arch = "wasm32"))]
const DESKTOP_CLIENT_SECRET: &str = "";

/// The built-in client ID for the target being compiled.
const DEFAULT_CLIENT_ID: &str = if cfg!(target_arch = "wasm32") {
    WEB_CLIENT_ID
} else {
    DESKTOP_CLIENT_ID
};

const FIELDS: &[Field] = &[
    Field {
        key: "client_id",
        label: "OAuth client ID",
        placeholder: DEFAULT_CLIENT_ID,
        kind: FieldKind::Text,
        advanced: true,
        platforms: Platforms::ALL,
    },
    Field {
        key: "client_secret",
        label: "OAuth client secret (desktop)",
        placeholder: "from the Desktop app client",
        kind: FieldKind::Password,
        advanced: true,
        platforms: Platforms::DESKTOP,
    },
    Field {
        key: "base_url",
        label: "API base (optional)",
        placeholder: "leave blank for googleapis.com",
        kind: FieldKind::Url,
        advanced: true,
        platforms: Platforms::ALL,
    },
];

impl GoogleDrive {
    fn client_id<'a>(&self, settings: &'a Settings) -> &'a str {
        let id = settings.text("client_id");
        if id.is_empty() {
            DEFAULT_CLIENT_ID
        } else {
            id
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn client_secret<'a>(&self, settings: &'a Settings) -> &'a str {
        let secret = settings.text("client_secret");
        if secret.is_empty() {
            DESKTOP_CLIENT_SECRET
        } else {
            secret
        }
    }

    fn api_base<'a>(&self, settings: &'a Settings) -> &'a str {
        let base = settings.text("base_url");
        if base.is_empty() {
            "https://www.googleapis.com"
        } else {
            base
        }
    }

    /// Overridable for tests; defaults to Google's authorization endpoint.
    #[cfg(not(target_arch = "wasm32"))]
    fn auth_endpoint<'a>(&self, settings: &'a Settings) -> &'a str {
        let url = settings.text("auth_url");
        if url.is_empty() {
            "https://accounts.google.com/o/oauth2/v2/auth"
        } else {
            url
        }
    }

    /// Overridable for tests; defaults to Google's token endpoint.
    #[cfg(not(target_arch = "wasm32"))]
    fn token_endpoint<'a>(&self, settings: &'a Settings) -> &'a str {
        let url = settings.text("token_url");
        if url.is_empty() {
            "https://oauth2.googleapis.com/token"
        } else {
            url
        }
    }

    async fn find(&self, settings: &Settings) -> Result<Option<(String, String)>, String> {
        let url = format!(
            "{}/drive/v3/files?spaces=appDataFolder&q=name%3D'benthic.json'&fields=files(id,version)",
            self.api_base(settings)
        );
        let authorization = format!("Bearer {}", settings.text("access_token"));
        let response = http::request(
            Method::Get,
            &url,
            &[("Authorization", authorization.as_str())],
            None,
        )
        .await?;
        if !response.ok() {
            return Err(http::error_message("Google Drive", &response));
        }
        let body: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        let Some(file) = body["files"].as_array().and_then(|files| files.first()) else {
            return Ok(None);
        };
        let id = file["id"].as_str().unwrap_or_default().to_string();
        let version = file["version"].as_str().unwrap_or_default().to_string();
        Ok(Some((id, version)))
    }
}

#[async_trait(?Send)]
impl Backend for GoogleDrive {
    fn id(&self) -> &'static str {
        "gdrive"
    }

    fn name(&self) -> &'static str {
        "Google Drive"
    }

    fn fields(&self) -> &'static [Field] {
        FIELDS
    }

    fn auth(&self) -> AuthKind {
        AuthKind::OAuth
    }

    fn sign_in_label(&self) -> &'static str {
        "Sign in with Google"
    }

    fn ready(&self, settings: &Settings) -> bool {
        !self.client_id(settings).is_empty()
    }

    fn signed_in(&self, settings: &Settings) -> bool {
        !settings.text("access_token").is_empty()
            && settings.i64("expires_at") > crate::platform::now_secs()
    }

    async fn refresh(&self, settings: &mut Settings) -> Result<(), String> {
        // The web build signs in through the browser, so there is nothing to
        // refresh here; expired tokens trigger an interactive sign-in instead.
        #[cfg(target_arch = "wasm32")]
        let _ = settings;
        #[cfg(not(target_arch = "wasm32"))]
        if !self.signed_in(settings) && !settings.text("refresh_token").is_empty() {
            let tokens = super::oauth_desktop::refresh(
                self.client_id(settings),
                self.client_secret(settings),
                self.token_endpoint(settings),
                settings.text("refresh_token"),
            )
            .await?;
            store_tokens(settings, tokens);
        }
        Ok(())
    }

    async fn sign_in(&self, settings: &mut Settings) -> Result<(), String> {
        let client_id = self.client_id(settings).to_string();
        if client_id.is_empty() {
            return Err("Set a Google OAuth client ID first.".to_string());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (token, expires_at) = super::oauth_web::sign_in(&client_id, DRIVE_SCOPE).await?;
            settings.set("access_token", token);
            settings.set("expires_at", expires_at);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tokens = super::oauth_desktop::sign_in(
                &client_id,
                self.client_secret(settings),
                self.auth_endpoint(settings),
                self.token_endpoint(settings),
                DRIVE_SCOPE,
            )
            .await?;
            store_tokens(settings, tokens);
        }
        Ok(())
    }

    async fn sign_out(&self, settings: &mut Settings) {
        #[cfg(target_arch = "wasm32")]
        super::oauth_web::sign_out(settings.text("access_token"));
        settings.clear("access_token");
        settings.clear("refresh_token");
        settings.clear("expires_at");
    }

    async fn fetch(&self, settings: &Settings) -> Result<Option<RemoteFile>, String> {
        let Some((id, version)) = self.find(settings).await? else {
            return Ok(None);
        };
        let url = format!(
            "{}/drive/v3/files/{}?alt=media",
            self.api_base(settings),
            id
        );
        let authorization = format!("Bearer {}", settings.text("access_token"));
        let response = http::request(
            Method::Get,
            &url,
            &[("Authorization", authorization.as_str())],
            None,
        )
        .await?;
        if !response.ok() {
            return Err(http::error_message("Google Drive", &response));
        }
        Ok(Some(RemoteFile {
            content: response.body,
            revision: version,
        }))
    }

    async fn push(
        &self,
        settings: &Settings,
        content: &str,
        _revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        let existing = if settings.text("file_id").is_empty() {
            self.find(settings).await?.map(|(id, _)| id)
        } else {
            Some(settings.text("file_id").to_string())
        };
        let id = match existing {
            Some(id) => id,
            None => {
                let url = format!("{}/drive/v3/files?fields=id", self.api_base(settings));
                let metadata = json!({
                    "name": "benthic.json",
                    "parents": ["appDataFolder"],
                });
                let authorization = format!("Bearer {}", settings.text("access_token"));
                let response = http::request(
                    Method::Post,
                    &url,
                    &[("Authorization", authorization.as_str())],
                    Some(&metadata.to_string()),
                )
                .await?;
                if !response.ok() {
                    return Err(http::error_message("Google Drive", &response));
                }
                let body: serde_json::Value =
                    serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
                body["id"].as_str().unwrap_or_default().to_string()
            }
        };
        let url = format!(
            "{}/upload/drive/v3/files/{}?uploadType=media&fields=version",
            self.api_base(settings),
            id
        );
        let authorization = format!("Bearer {}", settings.text("access_token"));
        let response = http::request(
            Method::Patch,
            &url,
            &[("Authorization", authorization.as_str())],
            Some(content),
        )
        .await?;
        if !response.ok() {
            return Err(http::error_message("Google Drive", &response));
        }
        let body: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        let version = body["version"].as_str().unwrap_or_default().to_string();
        Ok(RemoteFile {
            content: content.to_string(),
            revision: version,
        })
    }
}

/// Persist freshly minted tokens into the backend settings.
#[cfg(not(target_arch = "wasm32"))]
fn store_tokens(settings: &mut Settings, tokens: super::oauth_desktop::Tokens) {
    settings.set("access_token", tokens.access_token);
    settings.set("expires_at", tokens.expires_at);
    if let Some(refresh) = tokens.refresh_token {
        settings.set("refresh_token", refresh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_server::MockServer;

    fn settings(server: &MockServer) -> Settings {
        let mut settings = Settings::default();
        settings.set("access_token", "tok");
        settings.set("expires_at", crate::platform::now_secs() + 3600);
        settings.set("base_url", server.url.clone());
        settings
    }

    #[tokio::test]
    async fn fetch_finds_then_downloads() {
        let server = MockServer::start().await;
        server.reply(200, r#"{"files":[{"id":"f1","version":"7"}]}"#);
        server.reply(200, "the log");

        let file = GDRIVE.fetch(&settings(&server)).await.unwrap().unwrap();
        assert_eq!(file.content, "the log");
        assert_eq!(file.revision, "7");

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .path
            .starts_with("/drive/v3/files?spaces=appDataFolder"));
        assert_eq!(requests[1].path, "/drive/v3/files/f1?alt=media");
    }

    #[tokio::test]
    async fn fetch_returns_none_when_no_file_exists() {
        let server = MockServer::start().await;
        server.reply(200, r#"{"files":[]}"#);
        assert!(GDRIVE.fetch(&settings(&server)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn push_creates_the_file_then_uploads() {
        let server = MockServer::start().await;
        server.reply(200, r#"{"files":[]}"#);
        server.reply(200, r#"{"id":"f1"}"#);
        server.reply(200, r#"{"version":"9"}"#);

        let file = GDRIVE.push(&settings(&server), "log", None).await.unwrap();
        assert_eq!(file.revision, "9");

        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].method, "POST");
        assert!(requests[1].body.contains("appDataFolder"));
        assert_eq!(requests[2].method, "PATCH");
        assert!(requests[2].path.contains("/upload/drive/v3/files/f1"));
        assert_eq!(requests[2].body, "log");
    }

    #[test]
    fn signed_in_requires_a_live_token() {
        let mut settings = Settings::default();
        assert!(!GDRIVE.signed_in(&settings));
        settings.set("access_token", "tok");
        settings.set("expires_at", crate::platform::now_secs() - 1);
        assert!(!GDRIVE.signed_in(&settings));
        settings.set("expires_at", crate::platform::now_secs() + 60);
        assert!(GDRIVE.signed_in(&settings));
    }
}
