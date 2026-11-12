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
//! are flushed to IndexedDB in order in the background. A failed flush is kept
//! queued for retry and reported through [`next_write_status`] and subsequent
//! [`save`] calls, so the UI never claims unsaved data is safe.
//!
//! On the desktop every write goes to a temporary file that is synced and then
//! renamed over the target, so a crash never leaves a truncated log behind.

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
const AI: &str = "benthic.ai";
/// Prefix for copies of a log that failed to parse at startup.
const UNREADABLE_PREFIX: &str = "benthic.unreadable.";
#[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
const DEVICE_FINGERPRINTS: &str = "benthic.device_fingerprints";

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
    use std::task::{Poll, Waker};

    /// Keys stored in the bulk (IndexedDB) backend.
    const BULK: [&str; 3] = [super::LOG, super::BACKUP, super::BACKUP_TIME];
    const STORE: &str = "kv";
    const DB_NAME: &str = "benthic";

    thread_local! {
        static CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static PENDING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static WRITING: Cell<bool> = const { Cell::new(false) };
        static DB: RefCell<Option<Rc<Database>>> = const { RefCell::new(None) };
        static LOCAL_ONLY: Cell<bool> = const { Cell::new(false) };
        static WRITE_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
        static STATUS_GENERATION: Cell<u64> = const { Cell::new(0) };
        static STATUS_WAKERS: RefCell<Vec<Waker>> = const { RefCell::new(Vec::new()) };
    }

    fn is_bulk(key: &str) -> bool {
        BULK.contains(&key) || key.starts_with(super::UNREADABLE_PREFIX)
    }

    fn set_write_error(error: Option<String>) {
        let changed = WRITE_ERROR.with(|current| {
            let mut current = current.borrow_mut();
            if *current == error {
                false
            } else {
                *current = error;
                true
            }
        });
        if changed {
            STATUS_GENERATION.with(|generation| generation.set(generation.get() + 1));
            for waker in STATUS_WAKERS.with(|wakers| std::mem::take(&mut *wakers.borrow_mut())) {
                waker.wake();
            }
        }
    }

    /// Resolves with the current write error whenever it changes.
    pub async fn next_write_status() -> Option<String> {
        let start = STATUS_GENERATION.with(Cell::get);
        std::future::poll_fn(|context| {
            if STATUS_GENERATION.with(Cell::get) != start {
                Poll::Ready(WRITE_ERROR.with(|error| error.borrow().clone()))
            } else {
                STATUS_WAKERS.with(|wakers| wakers.borrow_mut().push(context.waker().clone()));
                Poll::Pending
            }
        })
        .await
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
    ///
    /// Fails when IndexedDB opened but a stored key could not be read: the
    /// caller must then keep autosave off, or it would overwrite a log it
    /// never saw.
    pub async fn init() -> Result<(), String> {
        let Some(db) = db().await else {
            // No IndexedDB: fall back to `localStorage` for everything.
            LOCAL_ONLY.with(|f| f.set(true));
            return Ok(());
        };
        let mut loaded = HashMap::new();
        for key in BULK {
            if let Some(value) = idb_get(&db, key).await? {
                loaded.insert(key.to_string(), value);
            }
        }
        CACHE.with(|c| *c.borrow_mut() = loaded);
        Ok(())
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
        match WRITE_ERROR.with(|error| error.borrow().clone()) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Write a key and wait until it is persisted, bypassing the queue.
    pub async fn set_durable(key: &str, value: &str) -> Result<(), String> {
        if !is_bulk(key) || LOCAL_ONLY.with(Cell::get) {
            return LocalStorage::set(key, value).map_err(|e| e.to_string());
        }
        let db = db().await.ok_or("IndexedDB is unavailable")?;
        idb_put(&db, key, value).await?;
        CACHE.with(|c| c.borrow_mut().insert(key.to_string(), value.to_string()));
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

    /// Flush the pending writes, coalescing repeats of the same key.
    ///
    /// On failure the unwritten values go back into the queue (unless a newer
    /// value for the same key arrived meanwhile) and the drain stops; the next
    /// write retries them.
    async fn drain() {
        loop {
            let batch: Vec<(String, String)> = PENDING.with(|p| p.borrow_mut().drain().collect());
            if batch.is_empty() {
                // No await between the empty check and clearing the flag, so a
                // concurrent enqueue cannot be lost.
                WRITING.with(|w| w.set(false));
                break;
            }
            let failure = match db().await {
                Some(db) => {
                    let mut failure = None;
                    let mut remaining = batch.into_iter();
                    for (key, value) in remaining.by_ref() {
                        if let Err(error) = idb_put(&db, &key, &value).await {
                            failure = Some((error, Some((key, value))));
                            break;
                        }
                    }
                    failure.map(|(error, failed)| {
                        (error, failed.into_iter().chain(remaining).collect())
                    })
                }
                None => Some(("IndexedDB is unavailable".to_string(), batch)),
            };
            match failure {
                None => set_write_error(None),
                Some((error, unwritten)) => {
                    PENDING.with(|pending| {
                        let mut pending = pending.borrow_mut();
                        for (key, value) in unwritten {
                            pending.entry(key).or_insert(value);
                        }
                    });
                    WRITING.with(|w| w.set(false));
                    set_write_error(Some(error));
                    break;
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    fn data_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            directories::ProjectDirs::from("io", "github", "benthic")
                .map(|dirs| dirs.data_dir().to_path_buf())
                .unwrap_or_default()
        })
    }

    fn path(key: &str) -> PathBuf {
        data_dir().join(key)
    }

    /// Nothing to prepare: the file backend reads and writes directly.
    pub async fn init() -> Result<(), String> {
        Ok(())
    }

    pub fn get(key: &str) -> Option<String> {
        std::fs::read_to_string(path(key)).ok()
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        write_atomic(&path(key), value.as_bytes()).map_err(|e| e.to_string())
    }

    pub async fn set_durable(key: &str, value: &str) -> Result<(), String> {
        set(key, value)
    }

    /// Write `contents` to a sibling temporary file, flush it to disk, then
    /// rename it over `target`, so readers see either the old or the new file.
    pub(super) fn write_atomic(target: &Path, contents: &[u8]) -> std::io::Result<()> {
        let dir = target.parent().filter(|dir| !dir.as_os_str().is_empty());
        if let Some(dir) = dir {
            std::fs::create_dir_all(dir)?;
        }
        let mut name = target.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        let temporary = target.with_file_name(name);
        let result = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return result;
        }
        // Persist the rename itself. Directories cannot be opened for syncing
        // on every platform, so this is best effort.
        #[cfg(unix)]
        if let Some(dir) = dir {
            if let Ok(dir) = std::fs::File::open(dir) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

/// Prepare the storage backend (opens IndexedDB on the web). Call once at
/// startup before the first read. On error, stored data could not be read and
/// must not be overwritten.
pub async fn init() -> Result<(), String> {
    imp::init().await
}

/// Wait until the background write status changes; `Some` carries the error
/// of the last failed flush, `None` means pending writes have been persisted.
#[cfg(target_arch = "wasm32")]
pub async fn next_write_status() -> Option<String> {
    imp::next_write_status().await
}

/// Durably keep a copy of a log that failed to parse under a new key, so
/// later saves and backups cannot destroy it. Returns the key it was stored
/// under.
pub async fn preserve_unreadable(contents: &str) -> Result<String, String> {
    let key = format!("{UNREADABLE_PREFIX}{}", crate::platform::now_secs());
    imp::set_durable(&key, contents).await?;
    Ok(key)
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

/// Local-only AI settings. The API key must never enter the dive log or sync.
pub fn load_ai() -> Option<String> {
    imp::get(AI)
}

pub fn save_ai(contents: &str) -> Result<(), String> {
    imp::set(AI, contents)
}

/// Per-device download fingerprints, keyed by model and connection.
#[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
pub fn load_device_fingerprints() -> Option<String> {
    imp::get(DEVICE_FINGERPRINTS)
}

#[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
pub fn save_device_fingerprints(contents: &str) -> Result<(), String> {
    imp::set(DEVICE_FINGERPRINTS, contents)
}

pub fn load_presets() -> Option<String> {
    imp::get(PRESETS)
}

pub fn save_presets(contents: &str) -> Result<(), String> {
    imp::set(PRESETS, contents)
}

/// Snapshot the current stored log into the backup slot, at most once per
/// [`BACKUP_INTERVAL_SECS`]. The age check happens before reading the log so
/// the common case costs nothing. A stored log that no longer parses never
/// replaces the backup.
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
    if current == new_contents || benthic_core::io::parse_auto(&current).is_err() {
        return;
    }
    let _ = write_backup(&current);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::imp::write_atomic;

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!("benthic-storage-{}", std::process::id()));
        let target = dir.join("benthic.log");
        write_atomic(&target, b"first").unwrap();
        write_atomic(&target, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_atomic_write_keeps_the_previous_file() {
        let dir = std::env::temp_dir().join(format!("benthic-storage-fail-{}", std::process::id()));
        let target = dir.join("benthic.log");
        write_atomic(&target, b"kept").unwrap();
        // A directory where the temporary file should go makes creation fail.
        std::fs::create_dir(dir.join("benthic.log.tmp")).unwrap();
        assert!(write_atomic(&target, b"lost").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
