//! Versioned store with atomic updates.
//!
//! The store is one JSON file. A save writes a sibling temporary file,
//! flushes it to disk, keeps the previous file as `.bak`, then renames the
//! temporary file over the original. A crash at any point leaves either the
//! old or the new file intact.

use crate::model::{Store, STORE_VERSION};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    /// The file was written by a newer version of the app. It is left
    /// untouched.
    NewerVersion { found: u32, supported: u32 },
    /// The file is not valid; it was preserved at `preserved_as`.
    Corrupt { preserved_as: PathBuf, reason: String },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "could not read or write the data file: {e}"),
            StoreError::NewerVersion { found, supported } => write!(
                f,
                "the data file is version {found}, but this build supports up to version {supported}"
            ),
            StoreError::Corrupt { preserved_as, reason } => write!(
                f,
                "the data file could not be read ({reason}); it was kept as {}",
                preserved_as.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

/// Result of loading: the store plus how it was obtained.
#[derive(Debug)]
pub struct Loaded {
    pub store: Store,
    pub origin: LoadOrigin,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadOrigin {
    /// No file existed; a fresh store was created in memory.
    Fresh,
    File,
    /// The main file was unreadable and the backup was used instead.
    Backup { preserved_as: PathBuf },
}

pub struct StoreFile {
    path: PathBuf,
}

impl StoreFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        StoreFile { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(suffix);
        self.path.with_file_name(name)
    }

    pub fn load(&self) -> Result<Loaded, StoreError> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A crash between "move old to .bak" and "rename new" leaves
                // only the backup.
                let bak = self.sibling(".bak");
                if let Ok(b) = fs::read(&bak) {
                    if let Ok(store) = parse(&b) {
                        return Ok(Loaded { store, origin: LoadOrigin::Backup { preserved_as: bak } });
                    }
                }
                return Ok(Loaded { store: Store::default(), origin: LoadOrigin::Fresh });
            }
            Err(e) => return Err(e.into()),
        };
        match parse(&bytes) {
            Ok(store) => Ok(Loaded { store, origin: LoadOrigin::File }),
            Err(ParseError::Newer(found)) => {
                Err(StoreError::NewerVersion { found, supported: STORE_VERSION })
            }
            Err(ParseError::Invalid(reason)) => {
                let preserved = self.sibling(&format!(".corrupt-{}", unix_now()));
                fs::copy(&self.path, &preserved)?;
                let bak = self.sibling(".bak");
                if let Ok(b) = fs::read(&bak) {
                    if let Ok(store) = parse(&b) {
                        return Ok(Loaded { store, origin: LoadOrigin::Backup { preserved_as: preserved } });
                    }
                }
                Err(StoreError::Corrupt { preserved_as: preserved, reason })
            }
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(store)
            .map_err(|e| StoreError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let tmp = self.sibling(".tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        if self.path.exists() {
            // Best effort: a failed backup must not block the save.
            let _ = fs::copy(&self.path, self.sibling(".bak"));
        }
        // `rename` replaces the destination atomically on Windows
        // (MoveFileExW with MOVEFILE_REPLACE_EXISTING) and on POSIX.
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

enum ParseError {
    Newer(u32),
    Invalid(String),
}

fn parse(bytes: &[u8]) -> Result<Store, ParseError> {
    // Tolerate a UTF-8 byte order mark from hand edits.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| ParseError::Invalid(e.to_string()))?;
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version > STORE_VERSION {
        return Err(ParseError::Newer(version));
    }
    let value = migrate(value, version);
    let mut store: Store =
        serde_json::from_value(value).map_err(|e| ParseError::Invalid(e.to_string()))?;
    store.version = STORE_VERSION;
    normalise(&mut store);
    Ok(store)
}

/// Upgrades older schema versions step by step. Version 1 is the first
/// released schema, so there is nothing to convert yet; version 0 (a file
/// without a version field) is read with defaults.
fn migrate(value: serde_json::Value, _from: u32) -> serde_json::Value {
    value
}

/// Repairs invariants that a hand-edited or partially written file may break.
fn normalise(store: &mut Store) {
    let max_id = store
        .accounts
        .iter()
        .map(|a| a.id)
        .chain(store.destinations.iter().map(|d| d.id))
        .max()
        .unwrap_or(0);
    if store.next_id <= max_id {
        store.next_id = max_id + 1;
    }
    if let Some(main) = store.main_account {
        if !store.accounts.iter().any(|a| a.id == main) {
            store.main_account = None;
        }
    }
    store.ensure_builtin_games();
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Account;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ram-core-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_gives_fresh_store_with_rogue_lineage() {
        let dir = temp_dir("fresh");
        let loaded = StoreFile::new(dir.join("store.json")).load().unwrap();
        assert_eq!(loaded.origin, LoadOrigin::Fresh);
        assert!(loaded.store.games.iter().any(|g| g.builtin && g.name == "Rogue Lineage"));
    }

    #[test]
    fn save_then_load_round_trips_and_keeps_backup() {
        let dir = temp_dir("roundtrip");
        let file = StoreFile::new(dir.join("store.json"));
        let mut store = Store::default();
        let id = store.alloc_id();
        store.accounts.push(Account { id, username: "one".into(), ..Default::default() });
        file.save(&store).unwrap();
        store.accounts[0].alias = "first".into();
        file.save(&store).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.origin, LoadOrigin::File);
        assert_eq!(loaded.store, store);
        let bak: Store = serde_json::from_slice(&fs::read(dir.join("store.json.bak")).unwrap()).unwrap();
        assert_eq!(bak.accounts[0].alias, "");
        assert!(!dir.join("store.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_preserved_and_backup_is_used() {
        let dir = temp_dir("corrupt");
        let file = StoreFile::new(dir.join("store.json"));
        let mut store = Store::default();
        let id = store.alloc_id();
        store.accounts.push(Account { id, username: "kept".into(), ..Default::default() });
        file.save(&store).unwrap();
        file.save(&store).unwrap(); // creates .bak
        fs::write(dir.join("store.json"), b"{ truncated").unwrap();

        let loaded = file.load().unwrap();
        match loaded.origin {
            LoadOrigin::Backup { preserved_as } => {
                assert_eq!(fs::read(preserved_as).unwrap(), b"{ truncated");
            }
            other => panic!("unexpected origin {other:?}"),
        }
        assert_eq!(loaded.store.accounts[0].username, "kept");
    }

    #[test]
    fn corrupt_file_without_backup_is_an_error_and_preserved() {
        let dir = temp_dir("corrupt-nobak");
        fs::write(dir.join("store.json"), b"not json").unwrap();
        match StoreFile::new(dir.join("store.json")).load() {
            Err(StoreError::Corrupt { preserved_as, .. }) => assert!(preserved_as.exists()),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(fs::read(dir.join("store.json")).unwrap(), b"not json");
    }

    #[test]
    fn newer_version_is_refused_and_left_alone() {
        let dir = temp_dir("newer");
        let text = format!("{{\"version\": {}}}", STORE_VERSION + 1);
        fs::write(dir.join("store.json"), &text).unwrap();
        assert!(matches!(
            StoreFile::new(dir.join("store.json")).load(),
            Err(StoreError::NewerVersion { .. })
        ));
        assert_eq!(fs::read_to_string(dir.join("store.json")).unwrap(), text);
    }

    #[test]
    fn interrupted_save_leaves_previous_file_readable() {
        let dir = temp_dir("interrupted");
        let file = StoreFile::new(dir.join("store.json"));
        let mut store = Store::default();
        let id = store.alloc_id();
        store.accounts.push(Account { id, username: "stable".into(), ..Default::default() });
        file.save(&store).unwrap();
        // Simulates a crash after the temporary file was partly written.
        fs::write(dir.join("store.json.tmp"), b"{\"version\": 1, \"acc").unwrap();
        let loaded = file.load().unwrap();
        assert_eq!(loaded.store.accounts[0].username, "stable");
    }

    #[test]
    fn dangling_main_and_low_next_id_are_repaired() {
        let dir = temp_dir("normalise");
        fs::write(
            dir.join("store.json"),
            br#"{"version":1,"next_id":1,"main_account":99,"accounts":[{"id":7,"username":"a"}]}"#,
        )
        .unwrap();
        let s = StoreFile::new(dir.join("store.json")).load().unwrap().store;
        assert_eq!(s.main_account, None);
        assert_eq!(s.next_id, 8);
    }
}
