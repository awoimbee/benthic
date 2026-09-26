//! Local persistence.
//!
//! The web build stores the log in `localStorage`; the desktop build stores it
//! in the platform data directory. Both persist the native JSON format, which
//! is lossless. Larger logs and IndexedDB-backed storage are tracked on the
//! roadmap.

/// Key / filename used for the autosaved log.
#[cfg(target_arch = "wasm32")]
mod imp {
    use gloo_storage::{LocalStorage, Storage};

    const KEY: &str = "benthic.log";

    pub fn load() -> Option<String> {
        LocalStorage::get(KEY).ok()
    }

    pub fn save(contents: &str) -> Result<(), String> {
        LocalStorage::set(KEY, contents).map_err(|e| e.to_string())
    }
}
#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::PathBuf;

    fn path() -> PathBuf {
        if let Some(dirs) = directories::ProjectDirs::from("io", "github", "benthic") {
            let dir = dirs.data_dir().to_path_buf();
            let _ = std::fs::create_dir_all(&dir);
            dir.join("log.benthic.json")
        } else {
            PathBuf::from("benthic.log.json")
        }
    }

    pub fn load() -> Option<String> {
        std::fs::read_to_string(path()).ok()
    }

    pub fn save(contents: &str) -> Result<(), String> {
        std::fs::write(path(), contents).map_err(|e| e.to_string())
    }
}

#[allow(unused_imports)]
pub use imp::{load, save};
