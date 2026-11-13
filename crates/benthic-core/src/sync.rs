//! Cloud storage planning.
//!
//! The remote holds a single copy of the log, used like a file: "open" loads
//! it and "save" replaces it. Both are conservative: saving only replaces the
//! remote copy when it is the one last opened or saved here, and opening asks
//! before discarding local changes that were never saved to it. Nothing is
//! overwritten silently.
//!
//! The transport (HTTP calls to the storage services) lives in the app; this
//! module is the pure decision logic and is fully unit-tested.

use serde::{Deserialize, Serialize};

/// A copy of the log as fetched from the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    /// The file contents.
    pub content: String,
    /// An opaque revision token (Git blob SHA, Drive version, ...).
    pub revision: String,
}

/// Bookkeeping from the last successful sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SyncState {
    /// The remote revision seen at the last successful sync.
    pub remote_revision: Option<String>,
    /// The local log fingerprint at the last successful sync.
    pub local_fingerprint: Option<String>,
    /// Unix seconds of the last successful sync.
    pub last_sync_secs: i64,
}

/// What saving should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePlan {
    /// The remote already holds this log.
    UpToDate,
    /// Replace (or create) the remote copy.
    Save,
    /// The remote changed since it was last opened or saved here; ask before
    /// replacing it.
    Conflict,
}

/// What opening should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlan {
    /// Replace the local log with the remote copy.
    Open,
    /// The local log has changes that were never saved to the remote; ask
    /// before replacing them.
    ConfirmDiscard,
}

/// A stable, dependency-free fingerprint of the log contents. Used only to
/// detect whether the local copy changed since the last sync, so a non-
/// cryptographic hash is enough; the length guards against the tiny chance of
/// a 64-bit collision.
pub fn fingerprint(contents: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in contents.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}-{}", contents.len())
}

/// Decide what saving should do given the local log, the bookkeeping from
/// the last open/save and the current remote copy.
pub fn save_plan(
    local_json: &str,
    local_empty: bool,
    state: &SyncState,
    remote: Option<&RemoteFile>,
) -> SavePlan {
    let Some(remote) = remote else {
        // Nothing stored yet: create it, unless there is nothing to store.
        return if local_empty {
            SavePlan::UpToDate
        } else {
            SavePlan::Save
        };
    };
    if remote.content == local_json {
        return SavePlan::UpToDate;
    }
    if state.remote_revision.as_deref() != Some(remote.revision.as_str()) {
        return SavePlan::Conflict;
    }
    if state.local_fingerprint.as_deref() == Some(fingerprint(local_json).as_str()) {
        SavePlan::UpToDate
    } else {
        SavePlan::Save
    }
}

/// Decide whether opening the remote copy may replace the local log.
pub fn open_plan(local_json: &str, local_empty: bool, state: &SyncState) -> OpenPlan {
    if local_empty || state.local_fingerprint.as_deref() == Some(fingerprint(local_json).as_str()) {
        OpenPlan::Open
    } else {
        OpenPlan::ConfirmDiscard
    }
}

/// Whether the local log changed since it was last opened from or saved to
/// the remote.
pub fn has_unsaved_changes(local_json: &str, state: &SyncState) -> bool {
    state.local_fingerprint.as_deref() != Some(fingerprint(local_json).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(revision: Option<&str>, fingerprint: Option<&str>) -> SyncState {
        SyncState {
            remote_revision: revision.map(str::to_string),
            local_fingerprint: fingerprint.map(str::to_string),
            last_sync_secs: 0,
        }
    }

    fn remote(content: &str, revision: &str) -> RemoteFile {
        RemoteFile {
            content: content.to_string(),
            revision: revision.to_string(),
        }
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes_changes() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
        assert_ne!(fingerprint("abc"), fingerprint("abc "));
    }

    #[test]
    fn saving_an_empty_log_to_an_empty_remote_is_a_no_op() {
        assert_eq!(
            save_plan("{}", true, &state(None, None), None),
            SavePlan::UpToDate
        );
    }

    #[test]
    fn saving_creates_the_remote() {
        assert_eq!(
            save_plan("local", false, &state(None, None), None),
            SavePlan::Save
        );
    }

    #[test]
    fn saving_never_replaces_a_remote_it_has_not_seen() {
        let r = remote("remote", "sha1");
        assert_eq!(
            save_plan("local", false, &state(None, None), Some(&r)),
            SavePlan::Conflict
        );
        let stale = state(Some("sha0"), Some(&fingerprint("local")));
        assert_eq!(
            save_plan("local", false, &stale, Some(&r)),
            SavePlan::Conflict
        );
    }

    #[test]
    fn saving_replaces_the_remote_last_seen() {
        let r = remote("remote", "sha1");
        let s = state(Some("sha1"), Some(&fingerprint("older")));
        assert_eq!(save_plan("local", false, &s, Some(&r)), SavePlan::Save);
        let unchanged = state(Some("sha1"), Some(&fingerprint("local")));
        assert_eq!(
            save_plan("local", false, &unchanged, Some(&r)),
            SavePlan::UpToDate
        );
    }

    #[test]
    fn saving_identical_content_is_up_to_date() {
        let r = remote("same", "sha9");
        assert_eq!(
            save_plan("same", false, &state(None, None), Some(&r)),
            SavePlan::UpToDate
        );
    }

    #[test]
    fn opening_asks_before_discarding_unsaved_changes() {
        assert_eq!(open_plan("{}", true, &state(None, None)), OpenPlan::Open);
        assert_eq!(
            open_plan("local", false, &state(None, None)),
            OpenPlan::ConfirmDiscard
        );
        let saved = state(Some("sha1"), Some(&fingerprint("local")));
        assert_eq!(open_plan("local", false, &saved), OpenPlan::Open);
        assert_eq!(open_plan("edited", false, &saved), OpenPlan::ConfirmDiscard);
        assert!(has_unsaved_changes("edited", &saved));
        assert!(!has_unsaved_changes("local", &saved));
    }

    #[test]
    fn state_round_trips_through_json() {
        let s = state(Some("sha"), Some("fp"));
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<SyncState>(&text).unwrap(), s);
    }
}
