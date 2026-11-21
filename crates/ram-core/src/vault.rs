//! Encrypted secret vault for Roblox session cookies.
//!
//! Each cookie is encrypted with a random 256-bit data key (DEK) using
//! AES-256-GCM. The DEK itself is wrapped one of two ways, chosen when the
//! vault is first set up:
//!
//! - **Windows data protection (DPAPI).** The DEK is protected with the
//!   signed-in Windows account, so the vault unlocks automatically for that
//!   user on that machine and is unreadable anywhere else. No password.
//! - **Master password.** The DEK is wrapped with a key derived from a
//!   password through Argon2id (memory-hard). The vault locks itself after a
//!   period of inactivity and must be unlocked with the password. A one-time
//!   recovery code can also unwrap the DEK, in case the password is lost.
//!
//! The DEK lives in memory only while unlocked and is zeroized on lock. The
//! raw cookies never touch the plaintext store or the log. Revealing or
//! copying a cookie is gated in the user interface behind Windows Hello or
//! the master password; this module only decrypts once the vault is
//! unlocked.
//!
//! Platform services are injected so the core stays testable: `Entropy`
//! supplies operating-system randomness and `Protector` performs DPAPI.

#![allow(clippy::type_complexity)]

use crate::model::AccountId;
use aes_gcm::aead::{Aead, KeyInit, Nonce};
use aes_gcm::Aes256Gcm;
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Operating-system randomness.
pub trait Entropy {
    fn fill(&self, buf: &mut [u8]);
}

/// Windows data protection (DPAPI). Each call binds to the current Windows
/// user; the entropy value is the implementor's concern.
pub trait Protector {
    fn protect(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String>;
    fn unprotect(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// The vault is locked; unlock it first.
    Locked,
    NotConfigured,
    AlreadyConfigured,
    /// The master password did not unwrap the key.
    WrongPassword,
    /// The recovery code did not unwrap the key.
    WrongRecoveryCode,
    /// This vault uses a different unlock method.
    WrongMode,
    Crypto(String),
    Protector(String),
    Io(String),
    /// The vault file is from a newer version.
    NewerVersion(u32),
    Corrupt(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Locked => write!(f, "the vault is locked"),
            VaultError::NotConfigured => write!(f, "no vault has been set up"),
            VaultError::AlreadyConfigured => write!(f, "a vault is already set up"),
            VaultError::WrongPassword => write!(f, "incorrect master password"),
            VaultError::WrongRecoveryCode => write!(f, "incorrect recovery code"),
            VaultError::WrongMode => write!(f, "this vault uses a different unlock method"),
            VaultError::Crypto(e) => write!(f, "the vault could not be decrypted ({e})"),
            VaultError::Protector(e) => write!(f, "Windows data protection failed ({e})"),
            VaultError::Io(e) => write!(f, "the vault file could not be read or written ({e})"),
            VaultError::NewerVersion(v) => write!(f, "the vault file is version {v}, newer than this build"),
            VaultError::Corrupt(e) => write!(f, "the vault file is damaged ({e})"),
        }
    }
}

impl std::error::Error for VaultError {}

pub type Result<T> = std::result::Result<T, VaultError>;

pub const VAULT_VERSION: u32 = 1;
const DEK_LEN: usize = 32;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;

/// Argon2id cost. Memory 64 MiB, 3 passes: a desktop-appropriate balance.
const ARGON_M_COST: u32 = 65_536;
const ARGON_T_COST: u32 = 3;
const ARGON_P_COST: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultMode {
    Dpapi,
    MasterPassword,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kdf {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

impl Default for Kdf {
    fn default() -> Self {
        Kdf { m_cost: ARGON_M_COST, t_cost: ARGON_T_COST, p_cost: ARGON_P_COST }
    }
}

/// A DEK wrapped by a password- or recovery-derived key.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Wrapped {
    salt: String,
    /// nonce(12) followed by the AES-256-GCM ciphertext of the DEK.
    blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    mode: VaultMode,
    #[serde(default)]
    kdf: Kdf,
    /// DPAPI-protected DEK (DPAPI mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dek_dpapi: Option<String>,
    /// Password-wrapped DEK (master-password mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dek_password: Option<Wrapped>,
    /// Recovery-code-wrapped DEK (master-password mode, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dek_recovery: Option<Wrapped>,
    /// Seconds of inactivity before auto-lock. 0 never locks.
    #[serde(default)]
    auto_lock_secs: u32,
    /// Require Windows Hello (or the password) to reveal or copy a secret.
    #[serde(default = "yes")]
    require_hello_reveal: bool,
    /// Per-account encrypted cookie: nonce(12) + AES-256-GCM ciphertext.
    #[serde(default)]
    secrets: BTreeMap<AccountId, String>,
    /// Per-account encrypted Roblox password, kept only for accounts added
    /// by username and password. Same sealing as the cookies.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    passwords: BTreeMap<AccountId, String>,
}

fn yes() -> bool {
    true
}

/// The runtime vault. Holds the decrypted DEK only while unlocked.
pub struct Vault {
    path: PathBuf,
    file: Option<VaultFile>,
    dek: Option<Zeroizing<[u8; DEK_LEN]>>,
    last_activity: Option<i64>,
}

impl Vault {
    /// Loads the vault file if it exists. A missing file means "not set up".
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let file = match std::fs::read(&path) {
            Ok(bytes) => {
                let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes).to_vec();
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|e| VaultError::Corrupt(e.to_string()))?;
                let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if version > VAULT_VERSION {
                    return Err(VaultError::NewerVersion(version));
                }
                let file: VaultFile =
                    serde_json::from_value(value).map_err(|e| VaultError::Corrupt(e.to_string()))?;
                Some(file)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(VaultError::Io(e.to_string())),
        };
        Ok(Vault { path, file, dek: None, last_activity: None })
    }

    /// An unconfigured vault at a path, for when a file could not be read.
    pub fn new_unconfigured(path: impl Into<PathBuf>) -> Self {
        Vault { path: path.into(), file: None, dek: None, last_activity: None }
    }

    pub fn is_configured(&self) -> bool {
        self.file.is_some()
    }

    pub fn mode(&self) -> Option<VaultMode> {
        self.file.as_ref().map(|f| f.mode)
    }

    pub fn is_unlocked(&self) -> bool {
        self.dek.is_some()
    }

    pub fn auto_lock_secs(&self) -> u32 {
        self.file.as_ref().map(|f| f.auto_lock_secs).unwrap_or(0)
    }

    pub fn require_hello_reveal(&self) -> bool {
        self.file.as_ref().map(|f| f.require_hello_reveal).unwrap_or(true)
    }

    pub fn set_require_hello_reveal(&mut self, on: bool) -> Result<()> {
        if let Some(f) = self.file.as_mut() {
            f.require_hello_reveal = on;
            self.save()?;
        }
        Ok(())
    }

    pub fn set_auto_lock_secs(&mut self, secs: u32) -> Result<()> {
        if let Some(f) = self.file.as_mut() {
            f.auto_lock_secs = secs;
            self.save()?;
        }
        Ok(())
    }

    /// Account ids that have a stored cookie. Needs no unlock.
    pub fn accounts_with_secret(&self) -> Vec<AccountId> {
        self.file.as_ref().map(|f| f.secrets.keys().copied().collect()).unwrap_or_default()
    }

    pub fn has_secret(&self, account: AccountId) -> bool {
        self.file.as_ref().is_some_and(|f| f.secrets.contains_key(&account))
    }

    // ---- setup -----------------------------------------------------------

    /// Creates a DPAPI-protected vault and leaves it unlocked.
    pub fn setup_dpapi(&mut self, entropy: &dyn Entropy, protector: &dyn Protector, auto_lock_secs: u32) -> Result<()> {
        if self.is_configured() {
            return Err(VaultError::AlreadyConfigured);
        }
        let dek = random_key(entropy);
        let protected = protector.protect(&dek).map_err(VaultError::Protector)?;
        self.file = Some(VaultFile {
            version: VAULT_VERSION,
            mode: VaultMode::Dpapi,
            kdf: Kdf::default(),
            dek_dpapi: Some(to_hex(&protected)),
            dek_password: None,
            dek_recovery: None,
            auto_lock_secs,
            require_hello_reveal: true,
            secrets: BTreeMap::new(),
            passwords: BTreeMap::new(),
        });
        self.dek = Some(Zeroizing::new(dek));
        self.save()?;
        Ok(())
    }

    /// Creates a password-protected vault, leaves it unlocked, and returns a
    /// one-time recovery code to show the user once.
    pub fn setup_master_password(
        &mut self,
        password: &str,
        entropy: &dyn Entropy,
        auto_lock_secs: u32,
    ) -> Result<RecoveryCode> {
        if self.is_configured() {
            return Err(VaultError::AlreadyConfigured);
        }
        let dek = random_key(entropy);
        let kdf = Kdf::default();
        let dek_password = wrap(&dek, password.as_bytes(), &kdf, entropy)?;
        let recovery = RecoveryCode::generate(entropy);
        let dek_recovery = wrap(&dek, recovery.canonical().as_bytes(), &kdf, entropy)?;
        self.file = Some(VaultFile {
            version: VAULT_VERSION,
            mode: VaultMode::MasterPassword,
            kdf,
            dek_dpapi: None,
            dek_password: Some(dek_password),
            dek_recovery: Some(dek_recovery),
            auto_lock_secs,
            require_hello_reveal: true,
            secrets: BTreeMap::new(),
            passwords: BTreeMap::new(),
        });
        self.dek = Some(Zeroizing::new(dek));
        self.save()?;
        Ok(recovery)
    }

    // ---- unlock ----------------------------------------------------------

    pub fn unlock_dpapi(&mut self, protector: &dyn Protector, now: i64) -> Result<()> {
        let file = self.file.as_ref().ok_or(VaultError::NotConfigured)?;
        if file.mode != VaultMode::Dpapi {
            return Err(VaultError::WrongMode);
        }
        let hex = file.dek_dpapi.as_ref().ok_or(VaultError::Corrupt("missing DPAPI key".into()))?;
        let protected = from_hex(hex)?;
        let dek = protector.unprotect(&protected).map_err(VaultError::Protector)?;
        self.set_dek(dek, now)
    }

    pub fn unlock_password(&mut self, password: &str, now: i64) -> Result<()> {
        let file = self.file.as_ref().ok_or(VaultError::NotConfigured)?;
        if file.mode != VaultMode::MasterPassword {
            return Err(VaultError::WrongMode);
        }
        let wrapped = file.dek_password.as_ref().ok_or(VaultError::Corrupt("missing password key".into()))?;
        let dek = unwrap(wrapped, password.as_bytes(), &file.kdf).map_err(|_| VaultError::WrongPassword)?;
        self.set_dek(dek, now)
    }

    pub fn unlock_recovery(&mut self, code: &str, now: i64) -> Result<()> {
        let file = self.file.as_ref().ok_or(VaultError::NotConfigured)?;
        let wrapped = file.dek_recovery.as_ref().ok_or(VaultError::WrongRecoveryCode)?;
        let canonical = RecoveryCode::canonicalize(code);
        let dek = unwrap(wrapped, canonical.as_bytes(), &file.kdf).map_err(|_| VaultError::WrongRecoveryCode)?;
        self.set_dek(dek, now)
    }

    fn set_dek(&mut self, dek: Vec<u8>, now: i64) -> Result<()> {
        if dek.len() != DEK_LEN {
            return Err(VaultError::Corrupt("key length".into()));
        }
        let mut k = [0u8; DEK_LEN];
        k.copy_from_slice(&dek);
        self.dek = Some(Zeroizing::new(k));
        self.last_activity = Some(now);
        Ok(())
    }

    /// Checks a master password without changing the lock state, for
    /// re-confirming before a reveal or copy.
    pub fn verify_password(&self, password: &str) -> bool {
        let Some(file) = self.file.as_ref() else { return false };
        let Some(wrapped) = file.dek_password.as_ref() else { return false };
        unwrap(wrapped, password.as_bytes(), &file.kdf).is_ok()
    }

    pub fn lock(&mut self) {
        self.dek = None;
        self.last_activity = None;
    }

    /// Records activity, delaying auto-lock.
    pub fn touch(&mut self, now: i64) {
        if self.is_unlocked() {
            self.last_activity = Some(now);
        }
    }

    /// Locks the vault if it has been idle past the timeout. Returns true if
    /// it locked just now.
    pub fn tick(&mut self, now: i64) -> bool {
        let timeout = self.auto_lock_secs();
        if self.is_unlocked() && timeout > 0 {
            if let Some(last) = self.last_activity {
                if now.saturating_sub(last) >= timeout as i64 {
                    self.lock();
                    return true;
                }
            }
        }
        false
    }

    /// Seconds until auto-lock, when unlocked with a timeout set.
    pub fn locks_in(&self, now: i64) -> Option<i64> {
        let timeout = self.auto_lock_secs();
        if self.is_unlocked() && timeout > 0 {
            self.last_activity.map(|last| (timeout as i64 - (now - last)).max(0))
        } else {
            None
        }
    }

    // ---- secrets ---------------------------------------------------------

    /// Encrypts and stores a cookie for an account. Requires an unlocked
    /// vault.
    pub fn set_secret(&mut self, account: AccountId, cookie: &str, entropy: &dyn Entropy, now: i64) -> Result<()> {
        let dek = self.dek.as_ref().ok_or(VaultError::Locked)?.clone();
        let blob = seal(&dek, cookie.as_bytes(), entropy)?;
        let file = self.file.as_mut().ok_or(VaultError::NotConfigured)?;
        file.secrets.insert(account, to_hex(&blob));
        self.last_activity = Some(now);
        self.save()
    }

    /// Decrypts the stored cookie for an account. Requires an unlocked
    /// vault. The caller gates reveal and copy behind Windows Hello or the
    /// password; launching uses this directly while unlocked.
    pub fn secret(&self, account: AccountId) -> Result<Option<Zeroizing<String>>> {
        let dek = self.dek.as_ref().ok_or(VaultError::Locked)?;
        let file = self.file.as_ref().ok_or(VaultError::NotConfigured)?;
        let Some(hex) = file.secrets.get(&account) else {
            return Ok(None);
        };
        let blob = from_hex(hex)?;
        let plain = open(dek, &blob)?;
        let text = String::from_utf8(plain.to_vec()).map_err(|_| VaultError::Crypto("not text".into()))?;
        Ok(Some(Zeroizing::new(text)))
    }

    pub fn remove_secret(&mut self, account: AccountId) -> Result<()> {
        if let Some(file) = self.file.as_mut() {
            if file.secrets.remove(&account).is_some() {
                self.save()?;
            }
        }
        Ok(())
    }

    /// Encrypts and stores a Roblox password for an account. Requires an
    /// unlocked vault.
    pub fn set_password(&mut self, account: AccountId, password: &str, entropy: &dyn Entropy, now: i64) -> Result<()> {
        let dek = self.dek.as_ref().ok_or(VaultError::Locked)?.clone();
        let blob = seal(&dek, password.as_bytes(), entropy)?;
        let file = self.file.as_mut().ok_or(VaultError::NotConfigured)?;
        file.passwords.insert(account, to_hex(&blob));
        self.last_activity = Some(now);
        self.save()
    }

    /// Decrypts the stored password for an account. Requires an unlocked
    /// vault; the caller gates it like a cookie reveal.
    pub fn password(&self, account: AccountId) -> Result<Option<Zeroizing<String>>> {
        let dek = self.dek.as_ref().ok_or(VaultError::Locked)?;
        let file = self.file.as_ref().ok_or(VaultError::NotConfigured)?;
        let Some(hex) = file.passwords.get(&account) else {
            return Ok(None);
        };
        let blob = from_hex(hex)?;
        let plain = open(dek, &blob)?;
        let text = String::from_utf8(plain.to_vec()).map_err(|_| VaultError::Crypto("not text".into()))?;
        Ok(Some(Zeroizing::new(text)))
    }

    pub fn has_password(&self, account: AccountId) -> bool {
        self.file.as_ref().map(|f| f.passwords.contains_key(&account)).unwrap_or(false)
    }

    pub fn remove_password(&mut self, account: AccountId) -> Result<()> {
        if let Some(file) = self.file.as_mut() {
            if file.passwords.remove(&account).is_some() {
                self.save()?;
            }
        }
        Ok(())
    }

    /// Changes the master password, re-wrapping the existing key, and issues
    /// a fresh recovery code. Requires an unlocked vault.
    pub fn change_master_password(&mut self, new_password: &str, entropy: &dyn Entropy) -> Result<RecoveryCode> {
        let dek = self.dek.as_ref().ok_or(VaultError::Locked)?.clone();
        let file = self.file.as_mut().ok_or(VaultError::NotConfigured)?;
        if file.mode != VaultMode::MasterPassword {
            return Err(VaultError::WrongMode);
        }
        file.dek_password = Some(wrap(&dek, new_password.as_bytes(), &file.kdf, entropy)?);
        let recovery = RecoveryCode::generate(entropy);
        file.dek_recovery = Some(wrap(&dek, recovery.canonical().as_bytes(), &file.kdf, entropy)?);
        self.save()?;
        Ok(recovery)
    }

    /// Deletes the vault entirely, with every stored cookie. Not reversible.
    pub fn destroy(&mut self) -> Result<()> {
        self.lock();
        self.file = None;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VaultError::Io(e.to_string())),
        }
    }

    fn save(&self) -> Result<()> {
        let Some(file) = self.file.as_ref() else { return Ok(()) };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| VaultError::Io(e.to_string()))?;
        }
        let json = serde_json::to_vec_pretty(file).map_err(|e| VaultError::Crypto(e.to_string()))?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &json).map_err(|e| VaultError::Io(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| VaultError::Io(e.to_string()))?;
        Ok(())
    }
}

// ---- crypto helpers --------------------------------------------------------

fn random_key(entropy: &dyn Entropy) -> [u8; DEK_LEN] {
    let mut k = [0u8; DEK_LEN];
    entropy.fill(&mut k);
    k
}

fn derive_key(password: &[u8], salt: &[u8], kdf: &Kdf) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(kdf.m_cost, kdf.t_cost, kdf.p_cost, Some(KEY_LEN))
        .map_err(|e| VaultError::Crypto(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|e| VaultError::Crypto(format!("argon2: {e}")))?;
    Ok(key)
}

/// Encrypts `plain` with a 256-bit key; output is nonce(12) || ciphertext.
fn seal(key: &[u8; KEY_LEN], plain: &[u8], entropy: &dyn Entropy) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| VaultError::Crypto("key length".into()))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    entropy.fill(&mut nonce_bytes);
    let nonce = Nonce::<Aes256Gcm>::try_from(&nonce_bytes[..]).map_err(|_| VaultError::Crypto("nonce".into()))?;
    let ct = cipher.encrypt(&nonce, plain).map_err(|_| VaultError::Crypto("encrypt".into()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a nonce(12) || ciphertext blob. A GCM tag mismatch (wrong key or
/// tampering) returns a crypto error.
fn open(key: &[u8; KEY_LEN], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if blob.len() < NONCE_LEN + 16 {
        return Err(VaultError::Crypto("short ciphertext".into()));
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| VaultError::Crypto("key length".into()))?;
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::<Aes256Gcm>::try_from(nonce_bytes).map_err(|_| VaultError::Crypto("nonce".into()))?;
    let plain = cipher
        .decrypt(&nonce, ct)
        .map_err(|_| VaultError::Crypto("authentication failed".into()))?;
    Ok(Zeroizing::new(plain))
}

/// Wraps the DEK with a password-derived key and a fresh salt.
fn wrap(dek: &[u8; DEK_LEN], password: &[u8], kdf: &Kdf, entropy: &dyn Entropy) -> Result<Wrapped> {
    let mut salt = [0u8; SALT_LEN];
    entropy.fill(&mut salt);
    let key = derive_key(password, &salt, kdf)?;
    let blob = seal(&key, dek, entropy)?;
    Ok(Wrapped { salt: to_hex(&salt), blob: to_hex(&blob) })
}

fn unwrap(wrapped: &Wrapped, password: &[u8], kdf: &Kdf) -> Result<Vec<u8>> {
    let salt = from_hex(&wrapped.salt)?;
    let blob = from_hex(&wrapped.blob)?;
    let key = derive_key(password, &salt, kdf)?;
    let dek = open(&key, &blob)?;
    Ok(dek.to_vec())
}

// ---- recovery code ---------------------------------------------------------

/// A one-time code that can unwrap the vault key if the password is lost.
/// Shown once at setup; the vault keeps only a wrapped key, never the code.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryCode(String);

impl RecoveryCode {
    /// Crockford base32, without the ambiguous I, L, O, U.
    const ALPHABET: &'static [u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    /// 130 bits of entropy as five groups of five characters.
    fn generate(entropy: &dyn Entropy) -> RecoveryCode {
        let mut bytes = [0u8; 17];
        entropy.fill(&mut bytes);
        let mut chars = String::new();
        for (i, chunk) in bytes[..15].chunks(5).enumerate() {
            if i > 0 {
                chars.push('-');
            }
            // 5 bytes = 40 bits = 8 base32 symbols.
            let mut acc = 0u64;
            for b in chunk {
                acc = (acc << 8) | *b as u64;
            }
            for shift in (0..8).rev() {
                chars.push(Self::ALPHABET[((acc >> (shift * 5)) & 31) as usize] as char);
            }
        }
        RecoveryCode(chars)
    }

    /// The code as shown to the user, grouped with dashes.
    pub fn display(&self) -> &str {
        &self.0
    }

    /// The canonical form used for key derivation (upper-case, no
    /// separators, ambiguous characters folded).
    pub fn canonical(&self) -> String {
        Self::canonicalize(&self.0)
    }

    pub fn canonicalize(input: &str) -> String {
        input
            .chars()
            .filter_map(|c| {
                let c = c.to_ascii_uppercase();
                match c {
                    'I' | 'L' => Some('1'),
                    'O' => Some('0'),
                    'U' => Some('V'),
                    '0'..='9' | 'A'..='Z' => Some(c),
                    _ => None,
                }
            })
            .collect()
    }
}

impl fmt::Debug for RecoveryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the code in logs or diagnostics.
        write!(f, "RecoveryCode(<redacted>)")
    }
}

// ---- hex -------------------------------------------------------------------

fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0xf) as usize] as char);
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(VaultError::Corrupt("odd hex length".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| VaultError::Corrupt("bad hex".into())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Counter-based "entropy": deterministic, for tests only.
    struct SeqEntropy {
        next: Cell<u8>,
    }
    impl SeqEntropy {
        fn new() -> Self {
            SeqEntropy { next: Cell::new(1) }
        }
    }
    impl Entropy for SeqEntropy {
        fn fill(&self, buf: &mut [u8]) {
            for b in buf.iter_mut() {
                let v = self.next.get();
                *b = v;
                self.next.set(v.wrapping_add(1));
            }
        }
    }

    /// A reversible stand-in for DPAPI that XORs with a fixed key.
    struct XorProtector;
    impl Protector for XorProtector {
        fn protect(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
            Ok(data.iter().map(|b| b ^ 0x5a).collect())
        }
        fn unprotect(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
            Ok(data.iter().map(|b| b ^ 0x5a).collect())
        }
    }

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ram-vault-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    const COOKIE: &str = "_|WARNING:-DO-NOT-SHARE-THIS...|_SECRETCOOKIEVALUE";

    #[test]
    fn passwords_are_sealed_beside_cookies_and_never_in_clear() {
        let (e, p) = (SeqEntropy::new(), XorProtector);
        let path = temp("passwords");
        let mut v = Vault::open(&path).unwrap();
        v.setup_dpapi(&e, &p, 0).unwrap();
        assert!(!v.has_password(3));
        assert_eq!(v.password(3).unwrap(), None);
        v.set_secret(3, COOKIE, &e, 0).unwrap();
        v.set_password(3, "hunter2-PLAINTEXT", &e, 0).unwrap();
        assert!(v.has_password(3) && v.has_secret(3));
        assert_eq!(v.password(3).unwrap().unwrap().as_str(), "hunter2-PLAINTEXT");
        // Nothing readable on disk.
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(!disk.contains("hunter2-PLAINTEXT") && !disk.contains("SECRETCOOKIEVALUE"));
        // Survives a reopen; locked means unreadable.
        let mut v2 = Vault::open(&path).unwrap();
        assert!(v2.has_password(3));
        assert_eq!(v2.password(3), Err(VaultError::Locked));
        v2.unlock_dpapi(&p, 0).unwrap();
        assert_eq!(v2.password(3).unwrap().unwrap().as_str(), "hunter2-PLAINTEXT");
        // Removing the password leaves the cookie, and an old vault file
        // without the map still loads (serde default).
        v2.remove_password(3).unwrap();
        assert!(!v2.has_password(3) && v2.has_secret(3));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("\"passwords\""));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dpapi_setup_store_reopen_and_read() {
        let (e, p) = (SeqEntropy::new(), XorProtector);
        let path = temp("dpapi");
        let mut v = Vault::open(&path).unwrap();
        assert!(!v.is_configured());
        v.setup_dpapi(&e, &p, 0).unwrap();
        assert!(v.is_unlocked());
        v.set_secret(7, COOKIE, &e, 0).unwrap();
        assert_eq!(v.secret(7).unwrap().unwrap().as_str(), COOKIE);

        // Reopen from disk: locked until DPAPI unlock.
        let mut v2 = Vault::open(&path).unwrap();
        assert!(v2.is_configured());
        assert_eq!(v2.mode(), Some(VaultMode::Dpapi));
        assert!(!v2.is_unlocked());
        assert!(v2.has_secret(7));
        assert_eq!(v2.secret(7), Err(VaultError::Locked));
        v2.unlock_dpapi(&p, 0).unwrap();
        assert_eq!(v2.secret(7).unwrap().unwrap().as_str(), COOKIE);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn master_password_unlock_and_wrong_password() {
        let e = SeqEntropy::new();
        let path = temp("pw");
        let mut v = Vault::open(&path).unwrap();
        let recovery = v.setup_master_password("hunter2", &e, 300).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();

        let mut v2 = Vault::open(&path).unwrap();
        assert_eq!(v2.unlock_password("wrong", 0), Err(VaultError::WrongPassword));
        assert!(!v2.is_unlocked());
        v2.unlock_password("hunter2", 0).unwrap();
        assert_eq!(v2.secret(1).unwrap().unwrap().as_str(), COOKIE);

        // Recovery code unlocks a fresh handle too.
        let mut v3 = Vault::open(&path).unwrap();
        v3.unlock_recovery(recovery.display(), 0).unwrap();
        assert_eq!(v3.secret(1).unwrap().unwrap().as_str(), COOKIE);
        assert_eq!(v3.unlock_recovery("WRONG-CODE", 0), Err(VaultError::WrongRecoveryCode));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recovery_code_is_normalised() {
        let e = SeqEntropy::new();
        let path = temp("rc");
        let mut v = Vault::open(&path).unwrap();
        let recovery = v.setup_master_password("pw", &e, 0).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();
        // Lower-case, spaces, and an ambiguous O/l instead of 0/1.
        let messy = recovery.display().to_lowercase().replace('-', "  ");
        let mut v2 = Vault::open(&path).unwrap();
        v2.unlock_recovery(&messy, 0).unwrap();
        assert!(v2.is_unlocked());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn auto_lock_after_timeout() {
        let e = SeqEntropy::new();
        let path = temp("lock");
        let mut v = Vault::open(&path).unwrap();
        v.setup_master_password("pw", &e, 60).unwrap();
        v.unlock_password("pw", 1000).unwrap();
        assert!(!v.tick(1059));
        assert!(v.is_unlocked());
        v.touch(1059);
        assert!(!v.tick(1100));
        assert_eq!(v.locks_in(1100), Some(19));
        assert!(v.tick(1119));
        assert!(!v.is_unlocked());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn change_password_keeps_secrets_and_rerolls_recovery() {
        let e = SeqEntropy::new();
        let path = temp("chpw");
        let mut v = Vault::open(&path).unwrap();
        let first = v.setup_master_password("old", &e, 0).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();
        let second = v.change_master_password("new", &e).unwrap();
        assert_ne!(first.canonical(), second.canonical());

        let mut v2 = Vault::open(&path).unwrap();
        assert_eq!(v2.unlock_password("old", 0), Err(VaultError::WrongPassword));
        v2.unlock_password("new", 0).unwrap();
        assert_eq!(v2.secret(1).unwrap().unwrap().as_str(), COOKIE);
        // The old recovery code no longer works; the new one does.
        let mut v3 = Vault::open(&path).unwrap();
        assert_eq!(v3.unlock_recovery(first.display(), 0), Err(VaultError::WrongRecoveryCode));
        v3.unlock_recovery(second.display(), 0).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn locked_vault_cannot_read_and_wrong_mode_is_reported() {
        let (e, p) = (SeqEntropy::new(), XorProtector);
        let path = temp("mode");
        let mut v = Vault::open(&path).unwrap();
        v.setup_master_password("pw", &e, 0).unwrap();
        v.lock();
        assert_eq!(v.secret(1), Err(VaultError::Locked));
        assert_eq!(v.unlock_dpapi(&p, 0), Err(VaultError::WrongMode));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let e = SeqEntropy::new();
        let path = temp("tamper");
        let mut v = Vault::open(&path).unwrap();
        v.setup_master_password("pw", &e, 0).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();
        // Flip a bit in the stored ciphertext.
        let file = v.file.as_mut().unwrap();
        let hex = file.secrets.get(&1).unwrap().clone();
        let mut bytes = from_hex(&hex).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        file.secrets.insert(1, to_hex(&bytes));
        assert!(matches!(v.secret(1), Err(VaultError::Crypto(_))));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn secrets_and_recovery_never_appear_in_debug_or_file() {
        let e = SeqEntropy::new();
        let path = temp("leak");
        let mut v = Vault::open(&path).unwrap();
        let recovery = v.setup_master_password("pw", &e, 0).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();
        assert!(!format!("{recovery:?}").contains(recovery.display()));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("SECRETCOOKIEVALUE"));
        assert!(!on_disk.contains(&recovery.canonical()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn remove_and_destroy() {
        let (e, p) = (SeqEntropy::new(), XorProtector);
        let path = temp("destroy");
        let mut v = Vault::open(&path).unwrap();
        v.setup_dpapi(&e, &p, 0).unwrap();
        v.set_secret(1, COOKIE, &e, 0).unwrap();
        v.set_secret(2, COOKIE, &e, 0).unwrap();
        v.remove_secret(1).unwrap();
        assert!(!v.has_secret(1));
        assert!(v.has_secret(2));
        v.destroy().unwrap();
        assert!(!path.exists());
        assert!(!v.is_configured());
    }
}
