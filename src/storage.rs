//! Local persistence.
//!
//! The web build stores data in `localStorage`; the desktop build stores it in
//! the platform data directory. The dive log is persisted as native JSON
//! (lossless); preferences are stored separately so they survive replacing the
//! log. Larger logs and IndexedDB-backed storage are tracked on the roadmap.

/// Key / filename for the autosaved log and the preferences.
#[cfg(target_arch = "wasm32")]
mod imp {
    use gloo_storage::{LocalStorage, Storage};

    const LOG_KEY: &str = "benthic.log";
    const PREFS_KEY: &str = "benthic.prefs";

    pub fn load() -> Option<String> {
        LocalStorage::get(LOG_KEY).ok()
    }

    pub fn save(contents: &str) -> Result<(), String> {
        LocalStorage::set(LOG_KEY, contents).map_err(|e| e.to_string())
    }

    pub fn load_prefs() -> Option<String> {
        LocalStorage::get(PREFS_KEY).ok()
    }

    pub fn save_prefs(contents: &str) -> Result<(), String> {
        LocalStorage::set(PREFS_KEY, contents).map_err(|e| e.to_string())
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

    fn log_path() -> PathBuf {
        let dir = data_dir();
        if dir.as_os_str().is_empty() {
            PathBuf::from("benthic.log.json")
        } else {
            dir.join("log.benthic.json")
        }
    }

    fn prefs_path() -> PathBuf {
        let dir = data_dir();
        if dir.as_os_str().is_empty() {
            PathBuf::from("benthic.prefs.json")
        } else {
            dir.join("prefs.json")
        }
    }

    pub fn load() -> Option<String> {
        std::fs::read_to_string(log_path()).ok()
    }

    pub fn save(contents: &str) -> Result<(), String> {
        std::fs::write(log_path(), contents).map_err(|e| e.to_string())
    }

    pub fn load_prefs() -> Option<String> {
        std::fs::read_to_string(prefs_path()).ok()
    }

    pub fn save_prefs(contents: &str) -> Result<(), String> {
        std::fs::write(prefs_path(), contents).map_err(|e| e.to_string())
    }
}

#[allow(unused_imports)]
pub use imp::{load, load_prefs, save, save_prefs};
