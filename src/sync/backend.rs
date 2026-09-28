//! The backend abstraction.
//!
//! A backend is one remote service (GitHub, Google Drive, OneDrive, FTP, ...).
//! It owns three things:
//!
//! * the **fields** the settings dialog should collect,
//! * the optional **interactive sign-in** flow, and
//! * the **transport**: fetching and pushing the single log file.
//!
//! Adding a backend is meant to be self-contained: implement [`Backend`] in a
//! new module, then list its instance in [`all`]. The dialog, the persistence
//! layer and the sync loop are all driven by this trait, so none of them need
//! to change.

use benthic_core::sync::RemoteFile;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{gdrive, github};

/// Per-backend settings, kept as a JSON object so a backend can persist
/// whatever it needs (including tokens) without changing the config schema.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Settings(pub Map<String, Value>);

impl Settings {
    /// A string setting, or `""` when absent.
    pub fn text(&self, key: &str) -> &str {
        self.0.get(key).and_then(Value::as_str).unwrap_or("")
    }

    /// An integer setting, or `0` when absent.
    pub fn i64(&self, key: &str) -> i64 {
        self.0.get(key).and_then(Value::as_i64).unwrap_or(0)
    }

    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.0.insert(key.to_string(), value.into());
    }

    pub fn clear(&mut self, key: &str) {
        self.0.remove(key);
    }
}

/// How a backend obtains its credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Credentials are typed in directly (e.g. a personal access token).
    Fields,
    /// An interactive "sign in" button drives an OAuth flow.
    OAuth,
}

/// The kind of HTML input a settings field maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Password,
    Url,
}

impl FieldKind {
    pub fn input_type(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Password => "password",
            FieldKind::Url => "url",
        }
    }
}

/// Which targets a field applies to (some credentials only exist on desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platforms {
    pub web: bool,
    pub desktop: bool,
}

impl Platforms {
    pub const ALL: Self = Self {
        web: true,
        desktop: true,
    };
    pub const DESKTOP: Self = Self {
        web: false,
        desktop: true,
    };
    /// Whether the field applies to the target this build is compiled for.
    pub fn current(self) -> bool {
        if cfg!(target_arch = "wasm32") {
            self.web
        } else {
            self.desktop
        }
    }
}

/// One settings input, rendered generically by the dialog.
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub kind: FieldKind,
    /// Shown under the collapsible "Advanced" section.
    pub advanced: bool,
    pub platforms: Platforms,
}

/// A remote service.
///
/// The futures are `?Send` because the web transport (`gloo-net`) is not
/// `Send`, and Dioxus polls tasks on a single thread.
#[async_trait::async_trait(?Send)]
pub trait Backend: Send + Sync {
    /// Stable id persisted in the config.
    fn id(&self) -> &'static str;
    /// Human-readable name shown in the picker.
    fn name(&self) -> &'static str;
    /// Settings fields the dialog should render, in order.
    fn fields(&self) -> &'static [Field];

    /// How credentials are provided.
    fn auth(&self) -> AuthKind {
        AuthKind::Fields
    }

    /// Label for the interactive sign-in button.
    fn sign_in_label(&self) -> &'static str {
        "Sign in"
    }

    /// Whether the typed settings are complete enough to attempt a request.
    fn ready(&self, settings: &Settings) -> bool;

    /// Whether there are currently valid credentials.
    fn signed_in(&self, settings: &Settings) -> bool {
        self.ready(settings)
    }

    /// Refresh credentials if possible. Called before every fetch/push.
    async fn refresh(&self, _settings: &mut Settings) -> Result<(), String> {
        Ok(())
    }

    /// Fetch the remote log, or `None` when it does not exist yet.
    async fn fetch(&self, settings: &Settings) -> Result<Option<RemoteFile>, String>;

    /// Upload `content`, replacing the file at `revision` (or creating it).
    async fn push(
        &self,
        settings: &Settings,
        content: &str,
        revision: Option<&str>,
    ) -> Result<RemoteFile, String>;

    /// Interactive sign-in. Only called when [`Backend::auth`] is
    /// [`AuthKind::OAuth`].
    async fn sign_in(&self, _settings: &mut Settings) -> Result<(), String> {
        Err("This service does not support interactive sign-in.".to_string())
    }

    /// Drop any stored credentials.
    async fn sign_out(&self, _settings: &mut Settings) {}
}

/// Every registered backend, in the order shown to the user.
pub fn all() -> &'static [&'static dyn Backend] {
    static REGISTRY: &[&dyn Backend] = &[&github::GITHUB, &gdrive::GDRIVE];
    REGISTRY
}

/// Look a backend up by id, falling back to the first one.
pub fn by_id(id: &str) -> &'static dyn Backend {
    all()
        .iter()
        .copied()
        .find(|b| b.id() == id)
        .unwrap_or(all()[0])
}
