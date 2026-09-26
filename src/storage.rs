//! Local persistence.
//!
//! The web build stores data in `localStorage`; the desktop build stores it in
//! the platform data directory. The dive log is persisted as native JSON
//! (lossless); preferences and filter presets are stored separately so they
//! survive replacing the log.
//!
//! A single, hourly automatic backup of the previous log is kept so an
//! accidental bad edit or a corrupt write can be recovered. Larger logs and
//! IndexedDB-backed storage are tracked on the roadmap.

/// How long to wait between automatic backups, in seconds.
const BACKUP_INTERVAL_SECS: i64 = 3600;

// Logical keys. The wasm backend uses them directly; the native backend maps
// them to files in the data directory.
const LOG: &str = "benthic.log";
const PREFS: &str = "benthic.prefs";
const PRESETS: &str = "benthic.presets";
const BACKUP: &str = "benthic.backup";
const BACKUP_TIME: &str = "benthic.backup_time";

#[cfg(target_arch = "wasm32")]
mod imp {
    use gloo_storage::{LocalStorage, Storage};

    pub fn get(key: &str) -> Option<String> {
        LocalStorage::get(key).ok()
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        LocalStorage::set(key, value).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        if let Some(dirs) = directories::ProjectDirs::from("io", "github", "benthic") {
            let dir = dirs.data_dir().to_path_buf();
            let _ = std::fs::create_dir_all(&dir);
            dir
        } else {
            PathBuf::new()
        }
    }

    fn path(key: &str) -> PathBuf {
        let dir = data_dir();
        if dir.as_os_str().is_empty() {
            PathBuf::from(key)
        } else {
            dir.join(key)
        }
    }

    pub fn get(key: &str) -> Option<String> {
        std::fs::read_to_string(path(key)).ok()
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        std::fs::write(path(key), value).map_err(|e| e.to_string())
    }
}

/// Load the dive log, if a stored one exists.
pub fn load() -> Option<String> {
    imp::get(LOG)
}

/// Save the dive log, taking an automatic backup first when one is due.
pub fn save(contents: &str) -> Result<(), String> {
    maybe_backup(contents);
    imp::set(LOG, contents)
}

/// Load the most recent automatic backup, if any.
pub fn load_backup() -> Option<String> {
    imp::get(BACKUP)
}

/// The age of the automatic backup in seconds, if one exists.
pub fn backup_age_secs() -> Option<i64> {
    let stored = imp::get(BACKUP_TIME).and_then(|s| s.parse::<i64>().ok())?;
    Some((crate::platform::now_secs() - stored).max(0))
}

/// Explicitly store `contents` as the backup (used when preserving a log that
/// failed to parse).
pub fn write_backup(contents: &str) -> Result<(), String> {
    imp::set(BACKUP, contents)?;
    imp::set(BACKUP_TIME, &crate::platform::now_secs().to_string())
}

pub fn load_prefs() -> Option<String> {
    imp::get(PREFS)
}

pub fn save_prefs(contents: &str) -> Result<(), String> {
    imp::set(PREFS, contents)
}

pub fn load_presets() -> Option<String> {
    imp::get(PRESETS)
}

pub fn save_presets(contents: &str) -> Result<(), String> {
    imp::set(PRESETS, contents)
}

/// Snapshot the current stored log into the backup slot, at most once per
/// [`BACKUP_INTERVAL_SECS`]. The age check happens before reading the log so
/// the common case costs nothing.
fn maybe_backup(new_contents: &str) {
    let due = match backup_age_secs() {
        Some(age) => age >= BACKUP_INTERVAL_SECS,
        None => true,
    };
    if !due {
        return;
    }
    let Some(current) = imp::get(LOG) else {
        return;
    };
    if current == new_contents {
        return;
    }
    let _ = write_backup(&current);
}
