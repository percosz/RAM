//! One-file backup of everything RAM keeps: the account list and settings,
//! and every stored cookie and password.
//!
//! The vault on disk cannot simply be copied, because in its default mode
//! it is tied to one Windows user on one PC. A backup therefore carries the
//! secrets themselves, sealed with a password the user chooses for that
//! file: Argon2id (the vault's cost) and AES-256-GCM over the whole payload,
//! so nothing in the file is readable without the password, not even the
//! usernames.
//!
//! Layout: [`MAGIC`] || salt(16) || nonce(12) || ciphertext of the JSON
//! [`Backup`].

use crate::model::{AccountId, Store};
use crate::vault::{password_open, password_seal, Entropy};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::Zeroizing;

/// First bytes of every backup file.
pub const MAGIC: &[u8] = b"RAM-BACKUP-1\n";
/// Shortest password accepted for a backup.
pub const MIN_PASSWORD_LEN: usize = 8;
/// Version of the payload layout.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupError {
    /// The file does not start with [`MAGIC`].
    NotABackup,
    /// Decryption failed: the password is wrong or the file was changed.
    WrongPassword,
    /// It decrypted but the contents are not a backup this version reads.
    Damaged(String),
    /// The password is shorter than [`MIN_PASSWORD_LEN`].
    WeakPassword,
    Crypto(String),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::NotABackup => write!(f, "That file is not a RAM backup."),
            BackupError::WrongPassword => write!(f, "Wrong password, or the backup file is damaged."),
            BackupError::Damaged(e) => write!(f, "The backup could not be read: {e}"),
            BackupError::WeakPassword => write!(f, "Use a backup password of at least {MIN_PASSWORD_LEN} characters."),
            BackupError::Crypto(e) => write!(f, "Encryption failed: {e}"),
        }
    }
}

/// Everything in a backup, in the clear. Lives only in memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub format: u32,
    pub app_version: String,
    /// Unix seconds.
    pub created: i64,
    pub store: Store,
    pub cookies: BTreeMap<AccountId, String>,
    pub passwords: BTreeMap<AccountId, String>,
}

impl Backup {
    /// A backup of `store` and the given secrets. State that only means
    /// something while this RAM is running (links to client processes,
    /// overrides in place, a pending settings restore) is left out.
    pub fn new(store: &Store, cookies: BTreeMap<AccountId, String>, passwords: BTreeMap<AccountId, String>, app_version: &str, created: i64) -> Backup {
        let mut store = store.clone();
        store.applied.clear();
        store.links.clear();
        store.xml_undo = None;
        Backup { format: FORMAT, app_version: app_version.to_string(), created, store, cookies, passwords }
    }
}

/// Encrypts a backup with `password`.
pub fn seal(backup: &Backup, password: &str, entropy: &dyn Entropy) -> Result<Vec<u8>, BackupError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(BackupError::WeakPassword);
    }
    let json = Zeroizing::new(serde_json::to_vec(backup).map_err(|e| BackupError::Crypto(e.to_string()))?);
    let sealed = password_seal(password, &json, entropy).map_err(|e| BackupError::Crypto(e.to_string()))?;
    let mut out = Vec::with_capacity(MAGIC.len() + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypts a backup file.
pub fn open(bytes: &[u8], password: &str) -> Result<Backup, BackupError> {
    let Some(sealed) = bytes.strip_prefix(MAGIC) else {
        return Err(BackupError::NotABackup);
    };
    let json = password_open(password, sealed).map_err(|_| BackupError::WrongPassword)?;
    let backup: Backup = serde_json::from_slice(&json).map_err(|e| BackupError::Damaged(e.to_string()))?;
    if backup.format > FORMAT {
        return Err(BackupError::Damaged(format!("it was made by a newer RAM ({})", backup.app_version)));
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Account;
    use std::cell::Cell;

    /// Deterministic bytes; never used outside tests.
    struct Counter(Cell<u8>);
    impl Entropy for Counter {
        fn fill(&self, buf: &mut [u8]) {
            for b in buf {
                self.0.set(self.0.get().wrapping_add(7));
                *b = self.0.get();
            }
        }
    }

    fn sample() -> Backup {
        let mut store = Store::default();
        let id = store.alloc_id();
        store.accounts.push(Account { id, username: "Thornwick_Eli".into(), alias: "main char".into(), ..Account::default() });
        store.main_account = Some(id);
        store.xml_undo = Some(Default::default());
        let cookies = BTreeMap::from([(id, "_|WARNING:-DO-NOT-SHARE-THIS.--fake-cookie-value".to_string())]);
        let passwords = BTreeMap::from([(id, "p\"a'ss wörd".to_string())]);
        Backup::new(&store, cookies, passwords, "0.6.0", 1_795_300_000)
    }

    #[test]
    fn round_trip_keeps_everything_and_hides_everything() {
        let backup = sample();
        // Runtime-only state is not carried.
        assert!(backup.store.xml_undo.is_none());
        let bytes = seal(&backup, "correct horse", &Counter(Cell::new(0))).unwrap();
        assert!(bytes.starts_with(MAGIC));
        // Nothing readable in the file: not the name, not the cookie.
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("Thornwick") && !text.contains("WARNING") && !text.contains("main char"));
        let back = open(&bytes, "correct horse").unwrap();
        assert_eq!(back, backup);
        assert_eq!(back.store.accounts[0].username, "Thornwick_Eli");
        assert_eq!(back.passwords.values().next().map(String::as_str), Some("p\"a'ss wörd"));
    }

    #[test]
    fn wrong_password_tampering_and_other_files_are_refused() {
        let backup = sample();
        let bytes = seal(&backup, "correct horse", &Counter(Cell::new(3))).unwrap();
        assert_eq!(open(&bytes, "correct horsf"), Err(BackupError::WrongPassword));
        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert_eq!(open(&flipped, "correct horse"), Err(BackupError::WrongPassword));
        assert_eq!(open(&bytes[..MAGIC.len() + 10], "correct horse"), Err(BackupError::WrongPassword));
        assert_eq!(open(b"{\"accounts\":[]}", "correct horse"), Err(BackupError::NotABackup));
        assert_eq!(open(b"", "correct horse"), Err(BackupError::NotABackup));
        assert_eq!(seal(&backup, "short", &Counter(Cell::new(0))), Err(BackupError::WeakPassword));
    }

    #[test]
    fn a_newer_format_is_refused() {
        let mut backup = sample();
        backup.format = FORMAT + 1;
        let bytes = seal(&backup, "correct horse", &Counter(Cell::new(9))).unwrap();
        assert!(matches!(open(&bytes, "correct horse"), Err(BackupError::Damaged(_))));
    }
}
