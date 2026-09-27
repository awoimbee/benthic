//! Sync planning.
//!
//! The remote holds a single copy of the log. Syncing is deliberately
//! conservative: it only pushes when the remote is unchanged since the last
//! sync, only pulls when the local copy is unchanged, and otherwise reports a
//! conflict so the user chooses. Nothing is overwritten silently.
//!
//! The transport (HTTP calls to GitHub or Google Drive) lives in the app; this
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

/// What a sync should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlan {
    /// Both sides already agree.
    UpToDate,
    /// Upload the local log to the remote.
    Push,
    /// Replace the local log with the remote's.
    Pull,
    /// Both sides changed since the last sync; ask the user.
    Conflict,
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

/// Decide what to do given the local log, the last-synced state and the remote.
pub fn plan(
    local_json: &str,
    local_empty: bool,
    state: &SyncState,
    remote: Option<&RemoteFile>,
) -> SyncPlan {
    let Some(remote) = remote else {
        // Nothing on the remote yet: create it, unless there is nothing to
        // send either.
        return if local_empty {
            SyncPlan::UpToDate
        } else {
            SyncPlan::Push
        };
    };

    let local_fingerprint = fingerprint(local_json);
    let remote_unchanged = state.remote_revision.as_deref() == Some(remote.revision.as_str());
    let local_unchanged = state.local_fingerprint.as_deref() == Some(local_fingerprint.as_str());

    // First ever sync: adopt the remote if there is nothing local to lose,
    // otherwise the two sides have diverged and the user must choose.
    if state.remote_revision.is_none() && state.local_fingerprint.is_none() {
        return if local_empty || local_json == remote.content {
            SyncPlan::Pull
        } else {
            SyncPlan::Conflict
        };
    }

    match (remote_unchanged, local_unchanged) {
        (true, true) => SyncPlan::UpToDate,
        (true, false) => SyncPlan::Push,
        (false, true) => SyncPlan::Pull,
        (false, false) => SyncPlan::Conflict,
    }
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
    fn empty_remote_and_empty_local_is_a_no_op() {
        assert_eq!(
            plan("{}", true, &state(None, None), None),
            SyncPlan::UpToDate
        );
    }

    #[test]
    fn empty_remote_gets_the_local_log() {
        assert_eq!(
            plan("{\"dives\":[]}", false, &state(None, None), None),
            SyncPlan::Push
        );
    }

    #[test]
    fn first_sync_adopts_a_remote_when_local_is_empty() {
        let r = remote("remote", "sha1");
        assert_eq!(
            plan("{}", true, &state(None, None), Some(&r)),
            SyncPlan::Pull
        );
        assert_eq!(
            plan("remote", false, &state(None, None), Some(&r)),
            SyncPlan::Pull
        );
    }

    #[test]
    fn first_sync_with_both_sides_filled_conflicts() {
        let r = remote("remote", "sha1");
        assert_eq!(
            plan("local", false, &state(None, None), Some(&r)),
            SyncPlan::Conflict
        );
    }

    #[test]
    fn pushes_only_when_the_remote_is_unchanged() {
        let local = "local";
        let r = remote("remote", "sha1");
        let s = state(Some("sha1"), Some(&fingerprint(local)));
        assert_eq!(plan(local, false, &s, Some(&r)), SyncPlan::UpToDate);

        let changed = state(Some("sha1"), Some(&fingerprint("older")));
        assert_eq!(plan(local, false, &changed, Some(&r)), SyncPlan::Push);
    }

    #[test]
    fn pulls_only_when_the_local_is_unchanged() {
        let local = "local";
        let r = remote("remote", "sha2");
        let s = state(Some("sha1"), Some(&fingerprint(local)));
        assert_eq!(plan(local, false, &s, Some(&r)), SyncPlan::Pull);
    }

    #[test]
    fn both_sides_changed_conflicts() {
        let r = remote("remote", "sha2");
        let s = state(Some("sha1"), Some(&fingerprint("older-local")));
        assert_eq!(plan("local", false, &s, Some(&r)), SyncPlan::Conflict);
    }

    #[test]
    fn state_round_trips_through_json() {
        let s = state(Some("sha"), Some("fp"));
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<SyncState>(&text).unwrap(), s);
    }
}
