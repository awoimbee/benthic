//! GitHub backend, via the repository-contents API.
//!
//! The configurable API base means this also covers GitHub Enterprise, Gitea
//! and other compatible hosts.

use async_trait::async_trait;
use base64::Engine;
use benthic_core::sync::RemoteFile;
use serde_json::json;

use super::backend::{Backend, Field, FieldKind, Platforms, Settings};
use super::http::{self, Method};

pub static GITHUB: GitHub = GitHub;

pub struct GitHub;

const FIELDS: &[Field] = &[
    Field {
        key: "repo",
        label: "Repository (owner/name)",
        placeholder: "awoimbee/benthic-dives",
        kind: FieldKind::Text,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "path",
        label: "File path",
        placeholder: "benthic.json",
        kind: FieldKind::Text,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "branch",
        label: "Branch",
        placeholder: "main",
        kind: FieldKind::Text,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "token",
        label: "Access token",
        placeholder: "a personal access token with repo scope",
        kind: FieldKind::Password,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "base_url",
        label: "API base (optional)",
        placeholder: "leave blank for api.github.com",
        kind: FieldKind::Url,
        advanced: true,
        platforms: Platforms::ALL,
    },
];

fn api_base(settings: &Settings) -> &str {
    let base = settings.text("base_url");
    if base.is_empty() {
        "https://api.github.com"
    } else {
        base
    }
}

fn branch(settings: &Settings) -> &str {
    let branch = settings.text("branch");
    if branch.is_empty() {
        "main"
    } else {
        branch
    }
}

fn headers(authorization: &str) -> [(&'static str, &str); 3] {
    [
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
        ("Authorization", authorization),
    ]
}

#[async_trait(?Send)]
impl Backend for GitHub {
    fn id(&self) -> &'static str {
        "github"
    }

    fn name(&self) -> &'static str {
        "GitHub (or compatible)"
    }

    fn fields(&self) -> &'static [Field] {
        FIELDS
    }

    fn ready(&self, settings: &Settings) -> bool {
        !settings.text("repo").is_empty()
            && !settings.text("path").is_empty()
            && !settings.text("token").is_empty()
    }

    async fn fetch(&self, settings: &Settings) -> Result<Option<RemoteFile>, String> {
        let repo = settings.text("repo");
        let path = settings.text("path");
        if repo.is_empty() || path.is_empty() {
            return Err("Set the GitHub repository and path first.".to_string());
        }
        let url = format!(
            "{}/repos/{repo}/contents/{path}?ref={}",
            api_base(settings),
            branch(settings)
        );
        let authorization = format!("Bearer {}", settings.text("token"));
        let response = http::request(Method::Get, &url, &headers(&authorization), None).await?;
        if response.status == 404 {
            return Ok(None);
        }
        if !response.ok() {
            return Err(http::error_message("GitHub", &response));
        }
        let body: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        let revision = body["sha"].as_str().unwrap_or_default().to_string();
        let encoded = body["content"]
            .as_str()
            .unwrap_or_default()
            .replace('\n', "");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| error.to_string())?;
        let content = String::from_utf8(bytes).map_err(|error| error.to_string())?;
        Ok(Some(RemoteFile { content, revision }))
    }

    async fn push(
        &self,
        settings: &Settings,
        content: &str,
        revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        let url = format!(
            "{}/repos/{}/contents/{}",
            api_base(settings),
            settings.text("repo"),
            settings.text("path")
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let mut body = json!({
            "message": "benthic: sync dive log",
            "content": encoded,
            "branch": branch(settings),
        });
        if let Some(revision) = revision {
            body["sha"] = serde_json::Value::String(revision.to_string());
        }
        let authorization = format!("Bearer {}", settings.text("token"));
        let response = http::request(
            Method::Put,
            &url,
            &headers(&authorization),
            Some(&body.to_string()),
        )
        .await?;
        if !response.ok() {
            return Err(http::error_message("GitHub", &response));
        }
        let body: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        let revision = body["content"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(RemoteFile {
            content: content.to_string(),
            revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_server::MockServer;

    fn settings(server: &MockServer) -> Settings {
        let mut settings = Settings::default();
        settings.set("repo", "me/dives");
        settings.set("path", "benthic.json");
        settings.set("branch", "main");
        settings.set("token", "tok");
        settings.set("base_url", server.url.clone());
        settings
    }

    #[tokio::test]
    async fn fetch_decodes_the_contents_api() {
        let server = MockServer::start().await;
        let encoded = base64::engine::general_purpose::STANDARD.encode("hello");
        server.reply(200, format!(r#"{{"sha":"abc","content":"{encoded}"}}"#));

        let file = GITHUB.fetch(&settings(&server)).await.unwrap().unwrap();
        assert_eq!(file.content, "hello");
        assert_eq!(file.revision, "abc");

        let request = server.last();
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.path,
            "/repos/me/dives/contents/benthic.json?ref=main"
        );
    }

    #[tokio::test]
    async fn fetch_treats_404_as_absent() {
        let server = MockServer::start().await;
        server.reply(404, r#"{"message":"Not Found"}"#);
        assert!(GITHUB.fetch(&settings(&server)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn push_sends_the_payload_and_reads_the_new_sha() {
        let server = MockServer::start().await;
        server.reply(200, r#"{"content":{"sha":"def"}}"#);

        let file = GITHUB
            .push(&settings(&server), "payload", Some("old"))
            .await
            .unwrap();
        assert_eq!(file.revision, "def");

        let request = server.last();
        assert_eq!(request.method, "PUT");
        assert!(request.body.contains("\"sha\":\"old\""));
        let encoded = base64::engine::general_purpose::STANDARD.encode("payload");
        assert!(request.body.contains(&encoded));
    }

    #[test]
    fn ready_requires_a_repo_path_and_token() {
        let mut settings = Settings::default();
        assert!(!GITHUB.ready(&settings));
        settings.set("repo", "me/dives");
        settings.set("path", "benthic.json");
        assert!(!GITHUB.ready(&settings));
        settings.set("token", "tok");
        assert!(GITHUB.ready(&settings));
    }
}
