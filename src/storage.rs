//! Local persistence.
//!
//! The web build keeps the (potentially large) dive log in IndexedDB and the
//! small preference blobs in `localStorage`; the desktop build keeps
//! everything in the platform data directory. The dive log is persisted as
//! native JSON (lossless); preferences and filter presets are stored
//! separately so they survive replacing the log.
//!
//! A single, hourly automatic backup of the previous log is kept so an
//! accidental bad edit or a corrupt write can be recovered.
//!
//! On the web the log lives behind an in-memory cache plus a coalescing write
//! queue: reads are synchronous (after [`init`]), writes update the cache and
//! are flushed to IndexedDB in order in the background.

/// How long to wait between automatic backups, in seconds.
const BACKUP_INTERVAL_SECS: i64 = 3600;

// Logical keys. The wasm backend uses them directly; the native backend maps
// them to files in the data directory.
const LOG: &str = "benthic.log";
const PREFS: &str = "benthic.prefs";
const PRESETS: &str = "benthic.presets";
const BACKUP: &str = "benthic.backup";
const BACKUP_TIME: &str = "benthic.backup_time";
const SYNC: &str = "benthic.sync";
const SYNC_STATE: &str = "benthic.sync_state";

#[cfg(target_arch = "wasm32")]
mod imp {
    //! IndexedDB-backed bulk storage with a synchronous cache in front, plus
    //! `localStorage` for the small keys and as a fallback when IndexedDB is
    //! unavailable (e.g. some private-browsing modes).

    use gloo_storage::{LocalStorage, Storage};
    use indexed_db_futures::database::Database;
    use indexed_db_futures::prelude::*;
    use indexed_db_futures::transaction::TransactionMode;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    /// Keys stored in the bulk (IndexedDB) backend.
    const BULK: [&str; 3] = [super::LOG, super::BACKUP, super::BACKUP_TIME];
    const STORE: &str = "kv";
    const DB_NAME: &str = "benthic";

    thread_local! {
        static CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static PENDING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static WRITING: Cell<bool> = const { Cell::new(false) };
        static DB: RefCell<Option<Rc<Database>>> = RefCell::new(None);
        static LOCAL_ONLY: Cell<bool> = const { Cell::new(false) };
    }

    fn is_bulk(key: &str) -> bool {
        BULK.contains(&key)
    }

    async fn open() -> Result<Rc<Database>, String> {
        let db = Database::open(DB_NAME)
            .with_version(1u8)
            .with_on_upgrade_needed(|_, db| {
                let _ = db.create_object_store(STORE).build();
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(Rc::new(db))
    }

    /// The cached database handle, opening it on first use.
    async fn db() -> Option<Rc<Database>> {
        if let Some(db) = DB.with(|d| d.borrow().clone()) {
            return Some(db);
        }
        let db = open().await.ok()?;
        DB.with(|d| *d.borrow_mut() = Some(db.clone()));
        Some(db)
    }

    async fn idb_get(db: &Database, key: &str) -> Result<Option<String>, String> {
        let tx = db.transaction(STORE).build().map_err(|e| e.to_string())?;
        let store = tx.object_store(STORE).map_err(|e| e.to_string())?;
        store
            .get(key.to_string())
            .primitive()
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())
    }

    async fn idb_put(db: &Database, key: &str, value: &str) -> Result<(), String> {
        let tx = db
            .transaction(STORE)
            .with_mode(TransactionMode::Readwrite)
            .build()
            .map_err(|e| e.to_string())?;
        let store = tx.object_store(STORE).map_err(|e| e.to_string())?;
        store
            .put(value.to_string())
            .with_key(key.to_string())
            .primitive()
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    /// Load the bulk keys into memory. Call once before the first read.
    pub async fn init() {
        let Some(db) = db().await else {
            // No IndexedDB: fall back to `localStorage` for everything.
            LOCAL_ONLY.with(|f| f.set(true));
            return;
        };
        let mut loaded = HashMap::new();
        for key in BULK {
            if let Ok(Some(value)) = idb_get(&db, key).await {
                loaded.insert(key.to_string(), value);
            }
        }
        let had_log = loaded.contains_key(super::LOG);
        CACHE.with(|c| *c.borrow_mut() = loaded);
        // One-time migration from the old `localStorage` log.
        if !had_log {
            if let Ok(old) = LocalStorage::get::<String>(super::LOG) {
                CACHE.with(|c| c.borrow_mut().insert(super::LOG.to_string(), old.clone()));
                enqueue(super::LOG, old);
            }
        }
    }

    pub fn get(key: &str) -> Option<String> {
        if !is_bulk(key) || LOCAL_ONLY.with(Cell::get) {
            return LocalStorage::get(key).ok();
        }
        CACHE.with(|c| c.borrow().get(key).cloned())
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        if !is_bulk(key) || LOCAL_ONLY.with(Cell::get) {
            return LocalStorage::set(key, value).map_err(|e| e.to_string());
        }
        CACHE.with(|c| c.borrow_mut().insert(key.to_string(), value.to_string()));
        enqueue(key, value.to_string());
        Ok(())
    }

    /// Queue a write, starting the background drain if it is not running.
    /// Later writes to the same key supersede earlier ones.
    fn enqueue(key: &str, value: String) {
        PENDING.with(|p| p.borrow_mut().insert(key.to_string(), value));
        if WRITING.with(|w| w.get()) {
            return;
        }
        WRITING.with(|w| w.set(true));
        wasm_bindgen_futures::spawn_local(drain());
    }

    /// Flush the pending writes in order, coalescing repeats of the same key.
    async fn drain() {
        loop {
            let batch: Vec<(String, String)> = PENDING.with(|p| p.borrow_mut().drain().collect());
            if batch.is_empty() {
                // No await between the empty check and clearing the flag, so a
                // concurrent enqueue cannot be lost.
                WRITING.with(|w| w.set(false));
                break;
            }
            if let Some(db) = db().await {
                for (key, value) in batch {
                    let _ = idb_put(&db, &key, &value).await;
                }
            }
        }
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

    /// Nothing to prepare: the file backend reads and writes directly.
    pub async fn init() {}

    pub fn get(key: &str) -> Option<String> {
        std::fs::read_to_string(path(key)).ok()
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        std::fs::write(path(key), value).map_err(|e| e.to_string())
    }
}

/// Prepare the storage backend (opens IndexedDB on the web). Call once at
/// startup before the first read.
pub async fn init() {
    imp::init().await;
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

/// The remote-sync configuration (provider, coordinates, token).
pub fn load_sync() -> Option<String> {
    imp::get(SYNC)
}

pub fn save_sync(contents: &str) -> Result<(), String> {
    imp::set(SYNC, contents)
}

/// The last-synced bookkeeping.
pub fn load_sync_state() -> Option<String> {
    imp::get(SYNC_STATE)
}

pub fn save_sync_state(contents: &str) -> Result<(), String> {
    imp::set(SYNC_STATE, contents)
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
