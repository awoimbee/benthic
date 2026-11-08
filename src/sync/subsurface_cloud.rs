//! Subsurface cloud storage, backed by its per-user git repository.
//!
//! The service is a normal smart-HTTP git server. The remote layout is
//! Subsurface's own tree format, so this backend converts to and from the
//! domain model in `benthic_core::io::subsurface_git` and reuses the generic
//! sync planner on top.

use async_trait::async_trait;
use benthic_core::io::subsurface_git;
use benthic_core::sync::RemoteFile;
use std::collections::BTreeMap;

use super::backend::{Backend, Field, FieldKind, Platforms, Settings};
use super::git;

pub static SUBSURFACE_CLOUD: SubsurfaceCloud = SubsurfaceCloud;

pub struct SubsurfaceCloud;

const DEFAULT_BASE: &str = "https://cloud.subsurface-divelog.org";

const FIELDS: &[Field] = &[
    Field {
        key: "email",
        label: "Subsurface cloud email",
        placeholder: "you@example.com",
        kind: FieldKind::Text,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "password",
        label: "Password",
        placeholder: "your Subsurface cloud password",
        kind: FieldKind::Password,
        advanced: false,
        platforms: Platforms::ALL,
    },
    Field {
        key: "base_url",
        label: "Server (optional)",
        placeholder: DEFAULT_BASE,
        kind: FieldKind::Url,
        advanced: true,
        platforms: Platforms::ALL,
    },
];

impl SubsurfaceCloud {
    fn base(&self, settings: &Settings) -> String {
        let base = settings.text("base_url");
        let base = if base.is_empty() { DEFAULT_BASE } else { base };
        format!(
            "{}/git/{}",
            base.trim_end_matches('/'),
            self.account(settings)
        )
    }

    /// Subsurface lower-cases the account email and uses it verbatim as both
    /// the repository name and the branch name.
    fn account(&self, settings: &Settings) -> String {
        settings.text("email").trim().to_lowercase()
    }
}

/// Keep files benthic does not own (filter presets, fingerprints, future
/// additions) while replacing the dive data it understands.
fn merge_files(
    mut old: BTreeMap<String, Vec<u8>>,
    new: BTreeMap<String, Vec<u8>>,
) -> BTreeMap<String, Vec<u8>> {
    let old_settings = old.get("00-Subsurface").cloned();
    old.retain(|path, _| {
        if path == "00-Subsurface" || path.starts_with("01-Divesites/") {
            return false;
        }
        let root = path.split('/').next().unwrap_or("");
        !(root.len() == 4 && root.chars().all(|c| c.is_ascii_digit()))
    });
    old.extend(new);

    // Keep settings benthic does not model itself (dive-computer fingerprints,
    // user id, future additions) instead of dropping them on every push.
    if let (Some(old_settings), Some(new_settings)) = (old_settings, old.get_mut("00-Subsurface")) {
        let known = ["version", "divecomputerid", "autogroup", "units", "prefs"];
        for line in String::from_utf8_lossy(&old_settings).lines() {
            let key = line.split_whitespace().next().unwrap_or("");
            if key.is_empty() || known.contains(&key) {
                continue;
            }
            new_settings.push(b'\n');
            new_settings.extend_from_slice(line.as_bytes());
        }
    }
    old
}

#[async_trait(?Send)]
impl Backend for SubsurfaceCloud {
    fn id(&self) -> &'static str {
        "subsurface"
    }

    fn name(&self) -> &'static str {
        "Subsurface cloud"
    }

    fn fields(&self) -> &'static [Field] {
        FIELDS
    }

    fn ready(&self, settings: &Settings) -> bool {
        !settings.text("email").is_empty() && !settings.text("password").is_empty()
    }

    async fn fetch(&self, settings: &Settings) -> Result<Option<RemoteFile>, String> {
        if !self.ready(settings) {
            return Err("Set your Subsurface cloud email and password first.".to_string());
        }
        let account = self.account(settings);
        let Some(fetched) = git::fetch_branch(
            &self.base(settings),
            &account,
            &account,
            settings.text("password"),
        )
        .await?
        else {
            return Ok(None);
        };
        let log = subsurface_git::parse(&fetched.files)
            .map_err(|error| format!("Could not read the Subsurface cloud log: {error}"))?;
        let content = benthic_core::io::json::to_string(&log)
            .map_err(|error| format!("Could not serialize the Subsurface cloud log: {error}"))?;
        Ok(Some(RemoteFile {
            content,
            revision: fetched.commit,
        }))
    }

    async fn push(
        &self,
        settings: &Settings,
        content: &str,
        revision: Option<&str>,
    ) -> Result<RemoteFile, String> {
        let log = benthic_core::io::parse_auto(content)
            .map_err(|error| format!("Could not parse the local log: {error}"))?;
        let new_files = subsurface_git::write(&log);
        let account = self.account(settings);
        let old_files = match git::fetch_branch(
            &self.base(settings),
            &account,
            &account,
            settings.text("password"),
        )
        .await?
        {
            Some(fetched) => fetched.files,
            None => BTreeMap::new(),
        };
        let files = merge_files(old_files, new_files);
        let commit = git::push_branch(
            &self.base(settings),
            &account,
            &account,
            settings.text("password"),
            revision,
            files,
        )
        .await?;
        Ok(RemoteFile {
            content: content.to_string(),
            revision: commit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_unknown_files_and_replaces_dive_data() {
        let mut old = BTreeMap::new();
        old.insert("00-Subsurface".to_string(), b"old".to_vec());
        old.insert("01-Divesites/Site-1".to_string(), b"old".to_vec());
        old.insert("2014/10/Dive-1".to_string(), b"old".to_vec());
        old.insert("02-Filterpresets/Preset-001".to_string(), b"keep".to_vec());
        old.insert(".gitignore".to_string(), b"keep".to_vec());
        let mut new = BTreeMap::new();
        new.insert("00-Subsurface".to_string(), b"new".to_vec());
        new.insert("2024/05/Dive-2".to_string(), b"new".to_vec());
        let merged = merge_files(old, new);
        assert!(merged.get("00-Subsurface").unwrap().starts_with(b"new"));
        assert!(merged.contains_key("02-Filterpresets/Preset-001"));
        assert!(merged.contains_key(".gitignore"));
        assert!(!merged.contains_key("01-Divesites/Site-1"));
        assert!(!merged.contains_key("2014/10/Dive-1"));
        assert!(merged.contains_key("2024/05/Dive-2"));
    }
}
