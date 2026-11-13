//! Subsurface cloud storage, backed by its per-user git repository.
//!
//! The service is a normal smart-HTTP git server. The remote layout is
//! Subsurface's own tree format, so this backend converts to and from the
//! domain model in `benthic_core::io::subsurface_git` and reuses the generic
//! sync planner on top.
//!
//! The cloud server sends no CORS headers, so the web build goes through a
//! CORS proxy that forwards `/git/...` unchanged (see
//! `docs/ARCHITECTURE.md`). Desktop builds talk to the server directly.

use async_trait::async_trait;
use benthic_core::io::subsurface_git;
use benthic_core::sync::RemoteFile;
use std::collections::BTreeMap;

use super::backend::{Backend, Field, FieldHelp, FieldKind, Platforms, Settings};
use super::git;

pub static SUBSURFACE_CLOUD: SubsurfaceCloud = SubsurfaceCloud;

pub struct SubsurfaceCloud;

const DIRECT_BASE: &str = "https://cloud.subsurface-divelog.org";
const PROXY_BASE: &str = "https://mi-11i-termux.ghost-skate.ts.net/subsurface";
const DEFAULT_BASE: &str = if cfg!(target_arch = "wasm32") {
    PROXY_BASE
} else {
    DIRECT_BASE
};

const FIELDS: &[Field] = &[
    Field {
        key: "email",
        label: "Subsurface cloud email",
        placeholder: "you@example.com",
        kind: FieldKind::Text,
        advanced: false,
        platforms: Platforms::ALL,
        help: None,
    },
    Field {
        key: "password",
        label: "Password",
        placeholder: "your Subsurface cloud password",
        kind: FieldKind::Password,
        advanced: false,
        platforms: Platforms::ALL,
        help: None,
    },
    Field {
        key: "base_url",
        label: "Server (optional)",
        placeholder: DEFAULT_BASE,
        kind: FieldKind::Url,
        advanced: true,
        platforms: Platforms::ALL,
        help: Some(FieldHelp::SubsurfaceServer),
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

/// Keep files benthic does not own (filter presets, pictures of unknown
/// dives, future additions) while replacing the dive data it understands. A
/// file whose content is unchanged keeps its exact old bytes.
fn merge_files(
    mut old: BTreeMap<String, Vec<u8>>,
    new: BTreeMap<String, Vec<u8>>,
) -> BTreeMap<String, Vec<u8>> {
    let old_settings = old.get("00-Subsurface").cloned();
    let old_sites: BTreeMap<String, Vec<u8>> = old
        .iter()
        .filter(|(path, _)| path.starts_with("01-Divesites/"))
        .map(|(path, data)| (path.clone(), data.clone()))
        .collect();
    old.retain(|path, _| {
        if path == "00-Subsurface" || path.starts_with("01-Divesites/") {
            return false;
        }
        let root = path.split('/').next().unwrap_or("");
        !(root.len() == 4 && root.chars().all(|c| c.is_ascii_digit()))
    });
    old.extend(new);

    if let (Some(old_settings), Some(new_settings)) = (old_settings, old.get_mut("00-Subsurface")) {
        *new_settings = merge_settings(&old_settings, new_settings);
    }
    // Site taxonomy carries an "origin" benthic does not model; keep the old
    // file when the parts benthic writes are unchanged.
    for (path, data) in old.iter_mut() {
        if let Some(previous) = old_sites.get(path) {
            if same_ignoring_geo_origin(previous, data) {
                *data = previous.clone();
            }
        }
    }
    old
}

/// Rebuild `00-Subsurface` from the old file: keep its lines and their order
/// (fingerprints, preferences and keys benthic does not model), take
/// `autogroup` and the dive computers from benthic, and add new devices.
fn merge_settings(old: &[u8], new: &[u8]) -> Vec<u8> {
    let new = String::from_utf8_lossy(new);
    let old = String::from_utf8_lossy(old);
    let autogroup = new.lines().any(|line| line.trim() == "autogroup");
    let new_devices: Vec<&str> = new
        .lines()
        .filter(|line| line.starts_with("divecomputerid "))
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut devices_written = false;
    for line in old.lines() {
        let key = line.split_whitespace().next().unwrap_or("");
        match key {
            "autogroup" => continue,
            "divecomputerid" => {
                if !devices_written {
                    out.extend(new_devices.iter().map(|line| line.to_string()));
                    devices_written = true;
                }
            }
            "units" if autogroup => {
                out.push("autogroup".to_string());
                out.push(line.to_string());
            }
            _ => out.push(line.to_string()),
        }
    }
    if !devices_written && !new_devices.is_empty() {
        let at = out
            .iter()
            .position(|line| line.starts_with("version "))
            .map_or(0, |i| i + 1);
        for (offset, line) in new_devices.iter().enumerate() {
            out.insert(at + offset, line.to_string());
        }
    }
    if autogroup && !out.iter().any(|line| line == "autogroup") {
        out.push("autogroup".to_string());
    }
    let mut text = out.join("\n");
    text.push('\n');
    text.into_bytes()
}

fn same_ignoring_geo_origin(old: &[u8], new: &[u8]) -> bool {
    let normalize = |data: &[u8]| -> Vec<String> {
        String::from_utf8_lossy(data)
            .lines()
            .map(|line| {
                let mut tokens: Vec<&str> = line.splitn(6, ' ').collect();
                if tokens.first() == Some(&"geo") && tokens.get(3) == Some(&"origin") {
                    tokens[4] = "_";
                }
                tokens.join(" ")
            })
            .collect()
    };
    normalize(old) == normalize(new)
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
        old.insert(
            "00-Subsurface".to_string(),
            b"version 3\nfingerprint model=1 serial=2 deviceid=3 diveid=4 data=\"ff\"\nunits METRIC\nprefs DCCEILING\n".to_vec(),
        );
        old.insert("01-Divesites/Site-1".to_string(), b"old".to_vec());
        old.insert("2014/10/Dive-1".to_string(), b"old".to_vec());
        old.insert("02-Filterpresets/Preset-001".to_string(), b"keep".to_vec());
        old.insert(".gitignore".to_string(), b"keep".to_vec());
        let mut new = BTreeMap::new();
        new.insert(
            "00-Subsurface".to_string(),
            b"version 3\nautogroup\nunits METRIC\n".to_vec(),
        );
        new.insert("2024/05/Dive-2".to_string(), b"new".to_vec());
        let merged = merge_files(old, new);
        assert_eq!(
            String::from_utf8_lossy(&merged["00-Subsurface"]),
            "version 3\nfingerprint model=1 serial=2 deviceid=3 diveid=4 data=\"ff\"\nautogroup\nunits METRIC\nprefs DCCEILING\n"
        );
        assert!(merged.contains_key("02-Filterpresets/Preset-001"));
        assert!(merged.contains_key(".gitignore"));
        assert!(!merged.contains_key("01-Divesites/Site-1"));
        assert!(!merged.contains_key("2014/10/Dive-1"));
        assert!(merged.contains_key("2024/05/Dive-2"));
    }

    /// Round-trips a log through a real git server. Run with a smart-HTTP
    /// server at `SUBSURFACE_TEST_URL` (serving `/git/<email>`) and
    /// `SUBSURFACE_TEST_EMAIL` / `SUBSURFACE_TEST_PASSWORD`, e.g.
    /// `cargo test subsurface_cloud -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a git smart-HTTP server"]
    async fn round_trips_through_a_git_server() {
        let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
        let mut settings = Settings::default();
        settings.set("base_url", env("SUBSURFACE_TEST_URL"));
        settings.set("email", env("SUBSURFACE_TEST_EMAIL"));
        settings.set("password", env("SUBSURFACE_TEST_PASSWORD"));
        let backend = &SUBSURFACE_CLOUD;

        let demo = include_str!("../../dives/demo.ssrf");
        let log = benthic_core::io::parse_auto(demo).unwrap();
        let content = benthic_core::io::json::to_string(&log).unwrap();

        let existing = backend.fetch(&settings).await.unwrap();
        let first = backend
            .push(
                &settings,
                &content,
                existing.as_ref().map(|f| f.revision.as_str()),
            )
            .await
            .unwrap();
        let fetched = backend.fetch(&settings).await.unwrap().expect("pushed log");
        assert_eq!(fetched.revision, first.revision);
        let back = benthic_core::io::parse_auto(&fetched.content).unwrap();
        assert_eq!(back.dives.len(), log.dives.len());
        assert_eq!(back.sites.len(), log.sites.len());

        let second = backend
            .push(&settings, &fetched.content, Some(&fetched.revision))
            .await
            .unwrap();
        assert_ne!(second.revision, first.revision);
        let stale = backend
            .push(&settings, &content, Some(&first.revision))
            .await
            .unwrap_err();
        assert!(stale.contains("changed"), "{stale}");

        settings.set("password", "wrong");
        let error = backend.fetch(&settings).await.unwrap_err();
        assert!(error.contains("email or password"), "{error}");
    }
}
