//! Preview-first importer for a legacy Roblox Account Manager folder.
//!
//! Only non-secret metadata is brought over: aliases, notes, groups, saved
//! destinations and the game list. Legacy cookies and passwords are never
//! deserialized: the account parser only materialises an allow-list of
//! properties and skips every other value without keeping it. The flow is
//! `scan` (read the folder) → `plan` (compare with the store) → the user
//! reviews the plan → `apply`.
//!
//! File formats are documented in `docs/LEGACY_AUDIT.md`, sections 4 to 6.
//!
//! All three storage variants of the account file are read: plaintext, DPAPI
//! (through a `LegacyDecryptor`) and password-protected (`scan_path` with the
//! password, see `decrypt_password_file`). A decrypted payload goes through
//! the same allow-list parser and is wiped afterwards.

use crate::links;
use crate::model::{Account, Destination, Game, ImportRecord, Store, Target};
use crate::signin;
use argon2::{Algorithm, Argon2, Params, Version};
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::XSalsa20Poly1305;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::collections::HashSet;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// The 64-byte header of a password-protected `AccountData.json`
/// (`Cryptography.RAMHeader` in the legacy source).
pub const PASSWORD_HEADER: &[u8; 64] = b"Roblox Account Manager created by ic3w0lf22 @ github.com .......";

/// The optional entropy the legacy app passes to DPAPI
/// (`AccountManager.Entropy` in the legacy source).
pub const LEGACY_DPAPI_ENTROPY: &[u8] = b"ROBLOX ACCOUNT MANAGER | :) | BROUGHT TO YOU BUY ic3w0lf";

/// Version and provider GUID that every DPAPI blob starts with.
const DPAPI_MAGIC: [u8; 20] = [
    0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF, 0x01, 0x15, 0xD1, 0x11, 0x8C, 0x7A, 0x00, 0xC0, 0x4F, 0xC2, 0x97,
    0xEB,
];

/// Files larger than this are skipped with a notice.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

const ACCOUNT_FILE: &str = "AccountData.json";

/// What the legacy app appends to `AccountData.json` for the copies it keeps
/// next to it (`AccountManager.cs` in the legacy source): `.backup` is the
/// previous contents, refreshed at most every 8 hours (lines 368-371); `.bak`
/// is the raw file when loading it failed (line 300); `.old` is the file a
/// restored password-locked backup replaced (lines 317-320). Each is stored
/// like an account file, in any of the three variants.
pub const ACCOUNT_BACKUP_SUFFIXES: [&str; 3] = [".backup", ".bak", ".old"];

/// Whether a file name is the legacy account file (`Some(false)`) or one of
/// its backups (`Some(true)`), ignoring ASCII case. `None` for anything else.
pub fn match_account_file_name(name: &str) -> Option<bool> {
    let split = ACCOUNT_FILE.len();
    if !name.is_char_boundary(split) || !name[..split].eq_ignore_ascii_case(ACCOUNT_FILE) {
        return None;
    }
    let suffix = &name[split..];
    if suffix.is_empty() {
        return Some(false);
    }
    ACCOUNT_BACKUP_SUFFIXES.iter().any(|s| suffix.eq_ignore_ascii_case(s)).then_some(true)
}
const SETTINGS_FILE: &str = "RAMSettings.ini";
const RECENT_FILE: &str = "RecentGames.json";
const FAVORITES_FILE: &str = "FavoriteGames.json";
const CONTROL_FILE: &str = "AccountControlData.json";

const LEGACY_DEFAULT_GROUP: &str = "Default";
const LINK_CODE_KEY: &str = "privateServerLinkCode=";
const VIP_REASON: &str =
    "saved with a private-server access code, which the browser hand-off cannot use; re-add it from its link";
const REST_IMPORTS: &str = "The other files still import.";

/// Decrypts the legacy DPAPI blob. Implemented by the Windows layer; when no
/// decryptor is supplied, DPAPI files cannot be read.
pub trait LegacyDecryptor {
    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String>;
}

/// How the legacy account file is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountFileKind {
    Missing,
    Plaintext,
    Dpapi,
    PasswordProtected,
    Unrecognised,
}

/// Detects the storage variant from the leading bytes. Never returns
/// `Missing`; that is for a file that does not exist.
pub fn detect_account_file(bytes: &[u8]) -> AccountFileKind {
    if bytes.len() >= PASSWORD_HEADER.len() && bytes[..PASSWORD_HEADER.len()] == PASSWORD_HEADER[..] {
        return AccountFileKind::PasswordProtected;
    }
    if bytes.starts_with(&DPAPI_MAGIC) {
        return AccountFileKind::Dpapi;
    }
    match strip_bom(bytes).iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'[') => AccountFileKind::Plaintext,
        _ => AccountFileKind::Unrecognised,
    }
}

// ---------------------------------------------------------------------------
// Password-protected account files
//
// `Classes/Cryptography.cs` in the legacy source, with libsodium-net 0.10.0
// (`RBX Alt Manager/packages.config`) on libsodium 1.0.11:
//
//   file = header (64) | salt (16) | nonce (24) | crypto_secretbox_easy(json)
//   key  = crypto_pwhash(32, pw, salt, opslimit 6, memlimit 134217728, alg 1)
//   pw   = crypto_hash(UTF-8 of the typed password)          (SHA-512, 64 bytes)
//
// `PasswordHash.ArgonHashBinary(pw, salt, StrengthArgon.Moderate, 32)` passes
// `ARGON_OPSLIMIT_MODERATE = 6`, `ARGON_MEMLIMIT_MODERATE = 134217728` and
// `ARGON_ALGORITHM_DEFAULT = 1`, which is `crypto_pwhash_ALG_ARGON2I13`:
// Argon2i version 1.3 with one lane (libsodium-net `PasswordHash.cs` at tag
// v0.10.0, lines 14-24, 134 and 178-180). The password bytes are
// `CryptoHash.Hash(PasswordTextBox.Text)` (`AccountManager.cs:422, 492`).

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const MAC_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// `StrengthArgon.Moderate`: 6 passes over 128 MiB (in KiB here).
const ARGON_MODERATE_PASSES: u32 = 6;
const ARGON_MODERATE_KIB: u32 = 134_217_728 / 1024;

/// Why a password-protected account file could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordFileError {
    /// The bytes do not start with the password-mode header.
    NotPasswordFile,
    /// The authentication check failed. That is what a wrong password looks
    /// like; a file altered after it was written fails the same way.
    WrongPassword,
    /// Too short to hold a salt, a nonce and an authentication tag.
    Malformed,
}

impl fmt::Display for PasswordFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PasswordFileError::NotPasswordFile => "the file is not password-protected",
            PasswordFileError::WrongPassword => "the password is wrong",
            PasswordFileError::Malformed => "the password-protected file is damaged or incomplete",
        })
    }
}

impl std::error::Error for PasswordFileError {}

/// Decrypts a password-protected `AccountData.json` and returns its payload,
/// the UTF-8 JSON account list **including cookies and passwords**. The
/// caller overwrites the returned buffer when done with it.
///
/// The password is used exactly as typed (the legacy app does not trim it).
/// Key derivation holds 128 MiB for its duration and takes on the order of a
/// second.
pub fn decrypt_password_file(bytes: &[u8], password: &str) -> Result<Vec<u8>, PasswordFileError> {
    let Some(rest) = bytes.strip_prefix(&PASSWORD_HEADER[..]) else {
        return Err(PasswordFileError::NotPasswordFile);
    };
    if rest.len() < SALT_LEN + NONCE_LEN + MAC_LEN {
        return Err(PasswordFileError::Malformed);
    }
    let (salt, rest) = rest.split_at(SALT_LEN);
    let (nonce, boxed) = rest.split_at(NONCE_LEN);

    let hashed = sha512(password.as_bytes());
    #[cfg(not(test))]
    let key = derive_key(&hashed[..], salt)?;
    // Unoptimised, a derivation takes many seconds: the tests share them.
    #[cfg(test)]
    let key = test_vectors::derive_key_once(&hashed[..], salt)?;

    // libsodium's "easy" layout, tag first, is what this cipher reads.
    XSalsa20Poly1305::new_from_slice(&key[..])
        .map_err(|_| PasswordFileError::Malformed)?
        .decrypt(crypto_secretbox::Nonce::from_slice(nonce), boxed)
        .map_err(|_| PasswordFileError::WrongPassword)
}

/// Argon2i v1.3 at `StrengthArgon.Moderate`, over the hashed password.
fn derive_key(hashed: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, PasswordFileError> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    let params = Params::new(ARGON_MODERATE_KIB, ARGON_MODERATE_PASSES, 1, Some(KEY_LEN))
        .map_err(|_| PasswordFileError::Malformed)?;
    Argon2::new(Algorithm::Argon2i, Version::V0x13, params)
        .hash_password_into(hashed, salt, &mut key[..])
        .map_err(|_| PasswordFileError::Malformed)?;
    Ok(key)
}

/// SHA-512 (FIPS 180-4), which is what libsodium's `crypto_hash` is. Only
/// used to turn the typed password into the bytes the legacy app feeds to
/// Argon2.
fn sha512(data: &[u8]) -> Zeroizing<[u8; 64]> {
    #[rustfmt::skip]
    const K: [u64; 80] = [
        0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
        0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
        0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
        0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
        0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
        0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
        0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
        0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
        0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
        0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
        0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
        0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
        0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
        0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
        0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
        0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
        0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
        0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
        0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
        0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
    ];
    let mut state = Zeroizing::new([
        0x6a09e667f3bcc908u64,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ]);

    // Sized up front so that the copy of the input is never reallocated.
    let mut padded = Zeroizing::new(Vec::with_capacity(data.len() + 144));
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 128 != 112 {
        padded.push(0);
    }
    padded.extend_from_slice(&(data.len() as u128 * 8).to_be_bytes());

    let mut w = Zeroizing::new([0u64; 80]);
    for block in padded.chunks_exact(128) {
        for (word, bytes) in w.iter_mut().zip(block.chunks_exact(8)) {
            *word = bytes.iter().fold(0u64, |acc, b| acc << 8 | u64::from(*b));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for (k, word) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(*k).wrapping_add(*word);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            (h, g, f, e) = (g, f, e, d.wrapping_add(t1));
            (d, c, b, a) = (c, b, a, t1.wrapping_add(s0.wrapping_add(maj)));
        }
        for (sum, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *sum = sum.wrapping_add(v);
        }
    }

    let mut out = Zeroizing::new([0u8; 64]);
    for (bytes, word) in out.chunks_exact_mut(8).zip(state.iter()) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// One legacy account, without its cookie or password.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyAccount {
    pub username: String,
    pub alias: String,
    pub description: String,
    /// Raw legacy group, `"Default"` when absent.
    pub group: String,
    /// `None` when the legacy ID is 0, absent or negative.
    pub user_id: Option<u64>,
    /// Unix seconds, when the legacy timestamp parses.
    pub last_use: Option<i64>,
    /// The account's saved place or server, from its `Fields`.
    pub saved: Option<Target>,
    /// Why a saved place or server exists but cannot be imported.
    pub saved_skip_reason: Option<String>,
}

impl fmt::Debug for LegacyAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LegacyAccount")
            .field("username", &self.username)
            .field("alias", &self.alias)
            .field("description", &self.description)
            .field("group", &self.group)
            .field("user_id", &self.user_id)
            .field("last_use", &self.last_use)
            .field("saved", &self.saved.as_ref().map(links::describe))
            .field("saved_skip_reason", &self.saved_skip_reason)
            .finish()
    }
}

/// A saved destination from `FavoriteGames.json` or
/// `AccountControlData.json`. Per-account saved destinations live on
/// `LegacyAccount::saved`.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDestination {
    pub name: String,
    pub target: Target,
    /// Name of the file it came from.
    pub source: &'static str,
}

impl fmt::Debug for LegacyDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LegacyDestination")
            .field("name", &self.name)
            .field("target", &links::describe(&self.target))
            .field("source", &self.source)
            .finish()
    }
}

/// A game list entry, unique by place ID within a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyGame {
    pub place_id: u64,
    pub name: String,
    pub universe_id: Option<u64>,
}

/// Everything importable that a legacy folder contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub folder: PathBuf,
    pub account_file: AccountFileKind,
    /// The account file is password-protected and no password was given, so
    /// its accounts are not in this scan. Scan again with the password.
    pub needs_password: bool,
    /// The account file is password-protected and the given password did not
    /// open it, so its accounts are not in this scan.
    pub wrong_password: bool,
    /// In legacy display order.
    pub accounts: Vec<LegacyAccount>,
    pub destinations: Vec<LegacyDestination>,
    pub games: Vec<LegacyGame>,
    /// Human-readable, one per problem or skipped file. Never contains
    /// secrets or private link codes.
    pub notices: Vec<String>,
}

/// Reads the legacy folder. Opens files read-only; never writes, renames or
/// deletes anything in the folder. A missing or malformed file produces a
/// notice and the rest still imports.
pub fn scan(folder: &Path, decryptor: Option<&dyn LegacyDecryptor>) -> Scan {
    let read = |name: &str| read_file(&folder.join(name));
    scan_inner(folder, ACCOUNT_FILE, read(ACCOUNT_FILE), &read, decryptor, None)
}

/// Like `scan`, for a path that is either the legacy folder or an account
/// file in it: `AccountData.json`, one of its backups (see
/// `ACCOUNT_BACKUP_SUFFIXES`) or a copy under any other name. The other
/// legacy files are looked up in the folder that holds the account file.
///
/// `password` opens a password-protected account file; it is ignored for the
/// other variants. Without one (`None` or empty) such a file sets
/// `Scan::needs_password`; with one that does not open it,
/// `Scan::wrong_password`. Either way the rest still imports.
pub fn scan_path(path: &Path, decryptor: Option<&dyn LegacyDecryptor>, password: Option<&str>) -> Scan {
    let (folder, file) = account_location(path);
    let name = file.file_name().map_or_else(|| ACCOUNT_FILE.to_string(), |n| n.to_string_lossy().into_owned());
    let read = |name: &str| read_file(&folder.join(name));
    scan_inner(&folder, &name, read_file(&file), &read, decryptor, password)
}

/// Splits a path the user picked into the legacy folder and the account
/// file to read. A directory is the folder itself; an existing file, or a
/// missing one that is named like an account file, is that file in its
/// parent folder; anything else is taken as a (missing) folder.
pub(crate) fn account_location(path: &Path) -> (PathBuf, PathBuf) {
    let named_like_one =
        || path.file_name().and_then(|n| n.to_str()).is_some_and(|n| match_account_file_name(n).is_some());
    if !path.is_dir() && (path.is_file() || named_like_one()) {
        if let Some(parent) = path.parent() {
            return (parent.to_path_buf(), path.to_path_buf());
        }
    }
    (path.to_path_buf(), path.join(ACCOUNT_FILE))
}

/// Same as `scan`, from in-memory file contents (file name → bytes).
pub fn scan_files(
    folder: &Path,
    files: &dyn Fn(&str) -> Option<Vec<u8>>,
    decryptor: Option<&dyn LegacyDecryptor>,
) -> Scan {
    let read = |name: &str| files(name).map_or(FileRead::Missing, FileRead::Data);
    scan_inner(folder, ACCOUNT_FILE, read(ACCOUNT_FILE), &read, decryptor, None)
}

enum FileRead {
    Missing,
    Unreadable(String),
    Data(Vec<u8>),
}

fn read_file(path: &Path) -> FileRead {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileRead::Missing,
        Err(e) => return FileRead::Unreadable(e.kind().to_string()),
    };
    let mut buf = Vec::new();
    // One byte past the cap is enough to tell that the file is too large.
    match file.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut buf) {
        Ok(_) => FileRead::Data(buf),
        Err(e) => {
            wipe(&mut buf);
            FileRead::Unreadable(e.kind().to_string())
        }
    }
}

/// `account_name` and `account_data` are the account file, which need not be
/// called `AccountData.json`; `read` fetches the other files by name.
fn scan_inner(
    folder: &Path,
    account_name: &str,
    account_data: FileRead,
    read: &dyn Fn(&str) -> FileRead,
    decryptor: Option<&dyn LegacyDecryptor>,
    password: Option<&str>,
) -> Scan {
    let mut scan = Scan {
        folder: folder.to_path_buf(),
        account_file: AccountFileKind::Missing,
        needs_password: false,
        wrong_password: false,
        accounts: Vec::new(),
        destinations: Vec::new(),
        games: Vec::new(),
        notices: Vec::new(),
    };
    scan_accounts(&mut scan, account_name, account_data, decryptor, password);
    scan_recent(&mut scan, read);
    scan_favorites(&mut scan, read);
    scan_control(&mut scan, read);
    scan_settings(&mut scan, read);
    scan
}

// ---------------------------------------------------------------------------
// AccountData.json

/// The only account properties that are ever materialised. `SecurityToken`
/// and `Password` are deliberately absent.
const ACCOUNT_KEYS: &[&str] = &["Username", "LastUse", "Alias", "Description", "Group", "UserID", "Fields"];

/// `Fields` keys the legacy app itself defines; everything else is custom.
const KNOWN_FIELDS: &[&str] = &[
    "SavedPlaceId",
    "SavedJobId",
    "NoCookieRefresh",
    "Window_Position_X",
    "Window_Position_Y",
    "Window_Width",
    "Window_Height",
];

fn scan_accounts(
    scan: &mut Scan,
    name: &str,
    data: FileRead,
    decryptor: Option<&dyn LegacyDecryptor>,
    password: Option<&str>,
) {
    let mut bytes = match data {
        FileRead::Missing => {
            scan.notices.push(format!(
                "{name} was not found in this folder, so no accounts are imported. {REST_IMPORTS}"
            ));
            return;
        }
        FileRead::Unreadable(why) => {
            scan.notices.push(format!(
                "{name} could not be read ({why}), so no accounts are imported. {REST_IMPORTS}"
            ));
            return;
        }
        FileRead::Data(bytes) => bytes,
    };
    if bytes.len() > MAX_FILE_BYTES {
        scan.account_file = AccountFileKind::Unrecognised;
        scan.notices.push(format!(
            "{name} is larger than 64 MiB and was skipped, so no accounts are imported. {REST_IMPORTS}"
        ));
        wipe(&mut bytes);
        return;
    }

    scan.account_file = detect_account_file(&bytes);
    match scan.account_file {
        AccountFileKind::Plaintext => parse_accounts(scan, name, &bytes),
        AccountFileKind::Dpapi => match decryptor.map(|d| d.unprotect(&bytes, LEGACY_DPAPI_ENTROPY)) {
            Some(Ok(mut plain)) => {
                parse_accounts(scan, name, &plain);
                wipe(&mut plain);
            }
            // The decryptor's message is not shown: it is not ours to vouch for.
            Some(Err(_)) => scan.notices.push(format!(
                "{name} is encrypted by Windows and could not be decrypted. It can only be decrypted on \
                 the PC that wrote it, so no accounts are imported here. {REST_IMPORTS}"
            )),
            None => scan.notices.push(format!(
                "{name} is encrypted by Windows and can only be decrypted on the PC that wrote it. \
                 Decryption is not available here, so no accounts are imported. {REST_IMPORTS}"
            )),
        },
        AccountFileKind::PasswordProtected => match password.filter(|p| !p.is_empty()) {
            None => {
                scan.needs_password = true;
                scan.notices.push(format!(
                    "{name} is password-protected, so no accounts are imported yet. {REST_IMPORTS} To import the \
                     accounts, enter the password (or remove the password in the legacy app and import again)."
                ));
            }
            Some(password) => match decrypt_password_file(&bytes, password) {
                Ok(mut plain) => {
                    parse_accounts(scan, name, &plain);
                    wipe(&mut plain);
                }
                Err(PasswordFileError::WrongPassword) => {
                    scan.wrong_password = true;
                    scan.notices.push(format!(
                        "{name} is password-protected and that password did not open it, so no accounts are \
                         imported. {REST_IMPORTS}"
                    ));
                }
                Err(_) => scan.notices.push(format!(
                    "{name} is password-protected but damaged or incomplete, so no accounts are imported. \
                     {REST_IMPORTS}"
                )),
            },
        },
        AccountFileKind::Missing | AccountFileKind::Unrecognised => scan.notices.push(format!(
            "{name} is not in a recognised format, so no accounts are imported. {REST_IMPORTS}"
        )),
    }
    wipe(&mut bytes);
}

fn parse_accounts(scan: &mut Scan, name: &str, payload: &[u8]) {
    let rows = match parse_array(payload, ACCOUNT_KEYS) {
        Ok(rows) => rows,
        Err(problem) => {
            scan.notices.push(format!("{name} {problem}, so no accounts are imported. {REST_IMPORTS}"));
            return;
        }
    };
    let mut not_objects = 0usize;
    let mut custom_fields = 0usize;
    for row in rows {
        let Some(row) = row else {
            not_objects += 1;
            continue;
        };
        let group = row.text("Group").trim().to_string();
        let description = row.text("Description");
        let (mut saved, mut saved_skip_reason) = (None, None);
        if let Some(fields) = row.map("Fields") {
            custom_fields += fields.0.iter().filter(|(k, _)| !KNOWN_FIELDS.contains(&k.as_str())).count();
            let place_text = fields.text("SavedPlaceId");
            let job_text = fields.text("SavedJobId");
            if !place_text.trim().is_empty() || !job_text.trim().is_empty() {
                match build_target(parse_place_text(&place_text), &job_text) {
                    Ok(target) => saved = Some(target),
                    Err(reason) => saved_skip_reason = Some(reason),
                }
            }
        }
        scan.accounts.push(LegacyAccount {
            username: row.text("Username").trim().to_string(),
            alias: row.text("Alias").trim().to_string(),
            description: if description.trim().is_empty() { String::new() } else { description },
            group: if group.is_empty() { LEGACY_DEFAULT_GROUP.to_string() } else { group },
            user_id: row.id("UserID"),
            last_use: parse_iso8601(&row.text("LastUse")),
            saved,
            saved_skip_reason,
        });
    }
    note_non_objects(&mut scan.notices, name, not_objects);
    if custom_fields > 0 {
        scan.notices.push(format!(
            "{custom_fields} custom account field{} in {name} {} not imported.",
            plural(custom_fields),
            if custom_fields == 1 { "is" } else { "are" }
        ));
    }
}

// ---------------------------------------------------------------------------
// The other files

/// Reads an optional file, noting why when it is not usable.
fn load(name: &str, read: &dyn Fn(&str) -> FileRead, notices: &mut Vec<String>) -> Option<Vec<u8>> {
    match read(name) {
        FileRead::Missing => {
            notices.push(format!("{name} was not found; skipped."));
            None
        }
        FileRead::Unreadable(why) => {
            notices.push(format!("{name} could not be read ({why}); skipped."));
            None
        }
        FileRead::Data(bytes) if bytes.len() > MAX_FILE_BYTES => {
            notices.push(format!("{name} is larger than 64 MiB; skipped."));
            None
        }
        FileRead::Data(bytes) => Some(bytes),
    }
}

fn load_rows(
    name: &str,
    keys: &'static [&'static str],
    read: &dyn Fn(&str) -> FileRead,
    notices: &mut Vec<String>,
) -> Vec<Record> {
    let Some(bytes) = load(name, read, notices) else { return Vec::new() };
    match parse_array(&bytes, keys) {
        Ok(rows) => {
            let total = rows.len();
            let rows: Vec<Record> = rows.into_iter().flatten().collect();
            note_non_objects(notices, name, total - rows.len());
            rows
        }
        Err(problem) => {
            notices.push(format!("{name} {problem}; skipped. {REST_IMPORTS}"));
            Vec::new()
        }
    }
}

fn note_non_objects(notices: &mut Vec<String>, name: &str, count: usize) {
    if count > 0 {
        notices.push(format!(
            "{count} entr{} in {name} {} not in the expected shape and {} skipped.",
            if count == 1 { "y" } else { "ies" },
            if count == 1 { "is" } else { "are" },
            if count == 1 { "was" } else { "were" }
        ));
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn scan_recent(scan: &mut Scan, read: &dyn Fn(&str) -> FileRead) {
    for row in load_rows(RECENT_FILE, &["Details"], read, &mut scan.notices) {
        push_details_game(&mut scan.games, row.map("Details"));
    }
}

fn scan_favorites(scan: &mut Scan, read: &dyn Fn(&str) -> FileRead) {
    let rows = load_rows(FAVORITES_FILE, &["Name", "PrivateServer", "PlaceID", "Details"], read, &mut scan.notices);
    for (index, row) in rows.iter().enumerate() {
        let details = row.map("Details");
        push_details_game(&mut scan.games, details);

        let private = row.text("PrivateServer");
        let place = games_place_id(&private).or(details.and_then(|d| d.id("placeId"))).or(row.id("PlaceID"));
        let mut name = row.text("Name").trim().to_string();
        if name.is_empty() {
            name = details.and_then(|d| known_name(&d.text("name"))).unwrap_or_default();
        }
        if name.is_empty() || find_ci(&name, "privateServerLinkCode").is_some() {
            name = format!("Favourite {}", index + 1);
        }
        match build_target(place, &private) {
            Ok(target) => push_destination(&mut scan.destinations, name, target, FAVORITES_FILE),
            Err(reason) => scan.notices.push(format!("{FAVORITES_FILE}: \"{name}\" was skipped: {reason}.")),
        }
    }
}

fn scan_control(scan: &mut Scan, read: &dyn Fn(&str) -> FileRead) {
    // `AutoExecute` (script text) is not in the key list, so it is never kept.
    for row in load_rows(CONTROL_FILE, &["Username", "PlaceId", "JobId"], read, &mut scan.notices) {
        let Some(place) = row.id("PlaceId") else { continue };
        let username = row.text("Username").trim().to_string();
        let name = format!("{} relaunch target", if username.is_empty() { "Account" } else { &username });
        match build_target(Some(place), &row.text("JobId")) {
            Ok(target) => push_destination(&mut scan.destinations, name, target, CONTROL_FILE),
            Err(reason) => scan.notices.push(format!("{CONTROL_FILE}: \"{name}\" was skipped: {reason}.")),
        }
    }
}

fn scan_settings(scan: &mut Scan, read: &dyn Fn(&str) -> FileRead) {
    let Some(bytes) = load(SETTINGS_FILE, read, &mut scan.notices) else { return };
    let text = String::from_utf8_lossy(strip_bom(&bytes));
    if let Some(place) = ini_value(&text, "General", "SavedPlaceId").and_then(parse_place_text) {
        push_game(&mut scan.games, place, None, None);
    }
}

/// Looks up one key: `[Section]` headers, `key=value` split on the first
/// `=`, both sides trimmed, `;` and `#` comment lines.
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = name.trim().eq_ignore_ascii_case(section);
        } else if in_section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

/// The destination's name is made safe to display: a name that itself
/// carries the link code is replaced by the redacted description.
fn push_destination(list: &mut Vec<LegacyDestination>, name: String, target: Target, source: &'static str) {
    let leaks = match &target {
        Target::PrivateServer { link_code, .. } => name.contains(link_code.as_str()),
        Target::ShareLink { code, .. } => name.contains(code.as_str()),
        _ => false,
    };
    let name = if leaks { links::describe(&target) } else { name };
    list.push(LegacyDestination { name, target, source });
}

fn push_details_game(games: &mut Vec<LegacyGame>, details: Option<&Record>) {
    let Some(details) = details else { return };
    if let Some(place) = details.id("placeId") {
        push_game(games, place, known_name(&details.text("name")), details.id("universeId"));
    }
}

/// Adds a game, or improves the existing entry for the same place.
fn push_game(games: &mut Vec<LegacyGame>, place_id: u64, name: Option<String>, universe_id: Option<u64>) {
    let placeholder = format!("Place {place_id}");
    match games.iter_mut().find(|g| g.place_id == place_id) {
        Some(existing) => {
            if existing.name == placeholder {
                if let Some(name) = name {
                    existing.name = name;
                }
            }
            existing.universe_id = existing.universe_id.or(universe_id);
        }
        None => games.push(LegacyGame { place_id, name: name.unwrap_or(placeholder), universe_id }),
    }
}

/// A legacy game name, unless it is empty or the "Unknown" stub.
fn known_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name != "Unknown").then(|| name.to_string())
}

// ---------------------------------------------------------------------------
// Targets

/// Builds a target from a place ID and the legacy "Job ID" text. The error
/// is a reason phrased to follow "skipped: " and never contains the text.
fn build_target(place: Option<u64>, job_text: &str) -> Result<Target, String> {
    const NO_PLACE: &str = "it has no valid place ID";
    let job = job_text.trim();
    if job.len() >= 4 && job.as_bytes()[..4].eq_ignore_ascii_case(b"VIP:") {
        return Err(VIP_REASON.to_string());
    }
    if let Some(pos) = find_ci(job, LINK_CODE_KEY) {
        let after = &job[pos + LINK_CODE_KEY.len()..];
        let end = after.find(|c: char| c == '&' || c == '#' || c.is_whitespace()).unwrap_or(after.len());
        let code = &after[..end];
        if !links::is_code(code) {
            return Err("its private-server link code is not in Roblox's format".to_string());
        }
        let place_id = games_place_id(job).or(place).ok_or(NO_PLACE)?;
        return Ok(Target::PrivateServer { place_id, link_code: code.to_string() });
    }
    let place_id = place.ok_or(NO_PLACE)?;
    if job.is_empty() {
        Ok(Target::Place { place_id })
    } else if links::is_job_id(job) {
        Ok(Target::LiveServer { place_id, job_id: job.to_ascii_lowercase() })
    } else {
        Err("its server ID is not in a recognised format".to_string())
    }
}

/// A place ID from the legacy PlaceId box: a number, or a `/games/<id>` link.
fn parse_place_text(text: &str) -> Option<u64> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse::<u64>().ok().filter(|n| *n > 0);
    }
    games_place_id(text)
}

/// The first positive ID following `/games/` in the text.
fn games_place_id(text: &str) -> Option<u64> {
    const MARKER: &str = "/games/";
    let mut rest = text;
    while let Some(pos) = find_ci(rest, MARKER) {
        rest = &rest[pos + MARKER.len()..];
        let digits = &rest[..rest.bytes().take_while(u8::is_ascii_digit).count()];
        if let Some(id) = digits.parse::<u64>().ok().filter(|n| *n > 0) {
            return Some(id);
        }
    }
    None
}

/// ASCII case-insensitive substring search. `needle` must be ASCII, which
/// makes the returned offset (and the end of the match) a char boundary.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

// ---------------------------------------------------------------------------
// Small parsers and helpers

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Overwrites a buffer that held legacy file content. Best effort: copies
/// the allocator or the OS made are out of reach.
fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        *b = 0;
    }
    std::hint::black_box(&*buf);
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fraction][Z|±HH:MM]` into Unix seconds. A
/// timestamp without an offset is taken as UTC. Dates before 1970 (the
/// legacy "never" value is year 1) give `None`.
fn parse_iso8601(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = &b[from..from + len];
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(part.iter().fold(0i64, |acc, d| acc * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let separators = b[4] == b'-' && b[7] == b'-' && matches!(b[10], b'T' | b't' | b' ') && b[13] == b':' && b[16] == b':';
    let in_range = (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24 && minute < 60 && second <= 60;
    if !separators || !in_range || year < 1970 {
        return None;
    }
    let mut i = 19;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    let offset = match &b[i..] {
        [] | [b'Z'] | [b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] | [sign @ (b'+' | b'-'), h1, h2, m1, m2]
            if [h1, h2, m1, m2].iter().all(|d| d.is_ascii_digit()) =>
        {
            let digit = |d: &u8| i64::from(*d - b'0');
            let secs = (digit(h1) * 10 + digit(h2)) * 3600 + (digit(m1) * 10 + digit(m2)) * 60;
            if *sign == b'-' {
                -secs
            } else {
                secs
            }
        }
        _ => return None,
    };
    // Days since 1970-01-01 (proleptic Gregorian).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Display name for a legacy group: strips an anchored `^\d{1,3}\s?` sort
/// prefix, so "001 Farm" shows as "Farm". When stripping leaves nothing, the
/// original is returned.
pub fn group_display_name(raw: &str) -> &str {
    let digits = raw.bytes().take(3).take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return raw;
    }
    let mut rest = &raw[digits..];
    if let Some(c) = rest.chars().next().filter(|c| c.is_whitespace()) {
        rest = &rest[c.len_utf8()..];
    }
    if rest.is_empty() {
        raw
    } else {
        rest
    }
}

/// The group an imported account gets: the legacy default means ungrouped.
fn import_group(raw: &str) -> &str {
    if raw == LEGACY_DEFAULT_GROUP {
        ""
    } else {
        raw
    }
}

// ---------------------------------------------------------------------------
// Tolerant JSON reading
//
// Files are read as "a list of objects, keeping only these keys". A value
// under any other key is skipped by the parser without being stored, and a
// value of an unexpected type never fails the file.

/// A JSON value reduced to what the importer can use.
enum Loose {
    Str(String),
    Int(i64),
    UInt(u64),
    Float(f64),
    Map(Record),
    /// Booleans, nulls and arrays.
    Other,
}

impl Loose {
    fn text(&self) -> String {
        match self {
            Loose::Str(s) => s.clone(),
            Loose::Int(n) => n.to_string(),
            Loose::UInt(n) => n.to_string(),
            _ => String::new(),
        }
    }

    /// A positive whole number, also accepted as numeric text.
    fn id(&self) -> Option<u64> {
        let id = match self {
            Loose::UInt(n) => Some(*n),
            Loose::Int(n) => u64::try_from(*n).ok(),
            Loose::Float(f) if f.fract() == 0.0 && *f > 0.0 && *f < 9_007_199_254_740_992.0 => Some(*f as u64),
            Loose::Str(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        };
        id.filter(|n| *n > 0)
    }
}

/// The kept properties of one JSON object, in file order.
struct Record(Vec<(String, Loose)>);

impl Record {
    /// Like Newtonsoft, a repeated key takes its last value.
    fn get(&self, key: &str) -> Option<&Loose> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn text(&self, key: &str) -> String {
        self.get(key).map(Loose::text).unwrap_or_default()
    }

    fn id(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Loose::id)
    }

    fn map(&self, key: &str) -> Option<&Record> {
        match self.get(key) {
            Some(Loose::Map(record)) => Some(record),
            _ => None,
        }
    }
}

/// Visitor methods that map every scalar to one fixed value.
macro_rules! scalars_as {
    ($value:expr) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok($value)
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok($value)
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok($value)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok($value)
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok($value)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok($value)
        }
    };
}

fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

impl<'de> Deserialize<'de> for Loose {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LooseVisitor;
        impl<'de> Visitor<'de> for LooseVisitor {
            type Value = Loose;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Loose, E> {
                Ok(Loose::Other)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Loose, E> {
                Ok(Loose::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Loose, E> {
                Ok(Loose::UInt(v))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Loose, E> {
                Ok(Loose::Float(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Loose, E> {
                Ok(Loose::Str(v.to_string()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Loose, E> {
                Ok(Loose::Other)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Loose, A::Error> {
                drain_seq(seq).map(|()| Loose::Other)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Loose, A::Error> {
                let mut entries = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    entries.push((key, map.next_value::<Loose>()?));
                }
                Ok(Loose::Map(Record(entries)))
            }
        }
        deserializer.deserialize_any(LooseVisitor)
    }
}

/// Reads one object, keeping only the listed keys. Anything that is not an
/// object reads as `None`.
struct Pick(&'static [&'static str]);

impl<'de> DeserializeSeed<'de> for Pick {
    type Value = Option<Record>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Pick {
    type Value = Option<Record>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }
    scalars_as!(None);
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        drain_seq(seq).map(|()| None)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if self.0.contains(&key.as_str()) {
                entries.push((key, map.next_value::<Loose>()?));
            } else {
                // Skipped without being stored. This is where legacy
                // cookies, passwords and scripts end.
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(Some(Record(entries)))
    }
}

/// Reads a list of objects with `Pick`. Anything that is not a list reads
/// as `None`.
struct PickSeq(&'static [&'static str]);

impl<'de> DeserializeSeed<'de> for PickSeq {
    type Value = Option<Vec<Option<Record>>>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for PickSeq {
    type Value = Option<Vec<Option<Record>>>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }
    scalars_as!(None);
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut rows = Vec::new();
        while let Some(row) = seq.next_element_seed(Pick(self.0))? {
            rows.push(row);
        }
        Ok(Some(rows))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        drain_map(map).map(|()| None)
    }
}

/// Why a JSON file is unusable. Carries a position only, never content.
enum JsonProblem {
    Syntax { line: usize, column: usize },
    NotList,
}

impl fmt::Display for JsonProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonProblem::Syntax { line, column } => {
                write!(f, "is damaged or incomplete (not valid JSON at line {line}, column {column})")
            }
            JsonProblem::NotList => f.write_str("does not contain a list"),
        }
    }
}

fn parse_array(bytes: &[u8], keys: &'static [&'static str]) -> Result<Vec<Option<Record>>, JsonProblem> {
    let syntax = |e: serde_json::Error| JsonProblem::Syntax { line: e.line(), column: e.column() };
    let mut de = serde_json::Deserializer::from_slice(strip_bom(bytes));
    let rows = PickSeq(keys).deserialize(&mut de).map_err(syntax)?;
    de.end().map_err(syntax)?;
    rows.ok_or(JsonProblem::NotList)
}

// ---------------------------------------------------------------------------
// Plan

/// What applying one plan item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    Add,
    /// Fills the listed empty fields ("alias", "notes", "group", "user ID")
    /// of an existing account.
    Update { changes: Vec<String> },
    Unchanged,
    Skip { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    Account,
    Destination,
    Game,
}

/// One row of the import preview.
#[derive(Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub kind: PlanKind,
    /// Safe to display: never contains a private code.
    pub label: String,
    /// Safe to display.
    pub detail: String,
    pub action: PlanAction,
    /// True by default for Add and Update. The preview lets the user untick
    /// an item; `apply` ignores the flag on Unchanged and Skip items.
    pub selected: bool,
    payload: Payload,
}

impl fmt::Debug for PlanItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlanItem")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("detail", &self.detail)
            .field("action", &self.action)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

/// What `apply` needs for an item. Not shown anywhere.
#[derive(Clone, PartialEq, Eq)]
enum Payload {
    None,
    Account(LegacyAccount),
    Destination { target: Target, source: &'static str },
    Game(LegacyGame),
}

/// The preview of an import: what each legacy entry would do to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    /// The legacy folder.
    pub source: String,
    /// Accounts, then destinations, then games.
    pub items: Vec<PlanItem>,
    pub notices: Vec<String>,
}

impl ImportPlan {
    /// Number of items that add, update, are unchanged and are skipped,
    /// regardless of selection.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for item in &self.items {
            match item.action {
                PlanAction::Add => counts.0 += 1,
                PlanAction::Update { .. } => counts.1 += 1,
                PlanAction::Unchanged => counts.2 += 1,
                PlanAction::Skip { .. } => counts.3 += 1,
            }
        }
        counts
    }
}

fn item(kind: PlanKind, label: String, detail: String, action: PlanAction, payload: Payload) -> PlanItem {
    let selected = matches!(action, PlanAction::Add | PlanAction::Update { .. });
    PlanItem { kind, label, detail, action, selected, payload }
}

fn skip(reason: &str) -> PlanAction {
    PlanAction::Skip { reason: reason.to_string() }
}

/// Compares the scan with the store and says what applying would do. Does
/// not modify the store.
pub fn plan(store: &Store, scan: &Scan) -> ImportPlan {
    let mut items = Vec::new();

    let mut seen_ids: HashSet<u64> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    for legacy in &scan.accounts {
        let label = match (legacy.alias.is_empty(), legacy.username.is_empty()) {
            (false, _) => legacy.alias.clone(),
            (true, false) => legacy.username.clone(),
            (true, true) => "(no username)".to_string(),
        };
        let mut detail = account_detail(legacy);
        let lower = legacy.username.to_lowercase();
        let action = if legacy.username.is_empty() {
            skip("no username")
        } else if legacy.user_id.is_some_and(|id| seen_ids.contains(&id)) || seen_names.contains(&lower) {
            skip("duplicate in source")
        } else {
            seen_ids.extend(legacy.user_id);
            seen_names.insert(lower);
            match find_account(store, legacy) {
                None => PlanAction::Add,
                Some(index) => {
                    let changes = fill(&mut store.accounts[index].clone(), legacy);
                    if changes.is_empty() {
                        PlanAction::Unchanged
                    } else {
                        detail = format!("fills {} of {}", changes.join(", "), store.accounts[index].username);
                        PlanAction::Update { changes }
                    }
                }
            }
        };
        items.push(item(PlanKind::Account, label, detail, action, Payload::Account(legacy.clone())));
    }

    let in_store: HashSet<&Target> = store.destinations.iter().map(|d| &d.target).collect();
    let mut seen: HashSet<Target> = HashSet::new();
    let mut destination = |name: String, target: &Target, source: &'static str| {
        let action = if in_store.contains(target) {
            PlanAction::Unchanged
        } else if !seen.insert(target.clone()) {
            skip("duplicate in source")
        } else {
            PlanAction::Add
        };
        let detail = format!("{} · from {source}", links::describe(target));
        item(PlanKind::Destination, name, detail, action, Payload::Destination { target: target.clone(), source })
    };
    for legacy in &scan.accounts {
        let owner = [&legacy.alias, &legacy.username].into_iter().find(|s| !s.is_empty()).map_or("Account", |s| s);
        match (&legacy.saved, &legacy.saved_skip_reason) {
            (Some(target), _) => {
                let what = if matches!(target, Target::Place { .. }) { "saved place" } else { "saved server" };
                items.push(destination(format!("{owner} {what}"), target, ACCOUNT_FILE));
            }
            (None, Some(reason)) => items.push(item(
                PlanKind::Destination,
                format!("{owner} saved server"),
                format!("from {ACCOUNT_FILE}"),
                skip(reason),
                Payload::None,
            )),
            (None, None) => {}
        }
    }
    for legacy in &scan.destinations {
        items.push(destination(legacy.name.clone(), &legacy.target, legacy.source));
    }

    let mut seen_places: HashSet<u64> = HashSet::new();
    for game in &scan.games {
        let action = if store.games.iter().any(|g| g.place_id == game.place_id) {
            PlanAction::Unchanged
        } else if !seen_places.insert(game.place_id) {
            skip("duplicate in source")
        } else {
            PlanAction::Add
        };
        let detail = format!("Place {} · game list", game.place_id);
        items.push(item(PlanKind::Game, game.name.clone(), detail, action, Payload::Game(game.clone())));
    }

    ImportPlan { source: scan.folder.display().to_string(), items, notices: scan.notices.clone() }
}

fn account_detail(legacy: &LegacyAccount) -> String {
    let mut parts = Vec::new();
    if !legacy.alias.is_empty() && !legacy.username.is_empty() {
        parts.push(legacy.username.clone());
    }
    let group = import_group(&legacy.group);
    if !group.is_empty() {
        parts.push(format!("group {}", group_display_name(group)));
    }
    if let Some(id) = legacy.user_id {
        parts.push(format!("user ID {id}"));
    }
    if !legacy.description.is_empty() {
        parts.push("has notes".to_string());
    }
    parts.join(" · ")
}

/// The store account a legacy account corresponds to: by Roblox user ID
/// first (confirmed or not), else by case-insensitive username.
fn find_account(store: &Store, legacy: &LegacyAccount) -> Option<usize> {
    if let Some(id) = legacy.user_id {
        if let Some(index) = store.accounts.iter().position(|a| a.roblox_user_id == Some(id)) {
            return Some(index);
        }
    }
    let lower = legacy.username.to_lowercase();
    if lower.is_empty() {
        return None;
    }
    store.accounts.iter().position(|a| a.username.to_lowercase() == lower)
}

/// Fills fields that are empty in the store account and returns what it
/// filled. Never overwrites a value and touches nothing else.
fn fill(account: &mut Account, legacy: &LegacyAccount) -> Vec<String> {
    let mut changes = Vec::new();
    if account.alias.trim().is_empty() && !legacy.alias.is_empty() {
        account.alias = legacy.alias.clone();
        changes.push("alias".to_string());
    }
    if account.notes.trim().is_empty() && !legacy.description.is_empty() {
        account.notes = legacy.description.clone();
        changes.push("notes".to_string());
    }
    let group = import_group(&legacy.group);
    if account.group.trim().is_empty() && !group.is_empty() {
        account.group = group.to_string();
        changes.push("group".to_string());
    }
    if account.roblox_user_id.is_none() {
        if let Some(id) = legacy.user_id {
            account.roblox_user_id = Some(id);
            account.user_id_confirmed = false;
            changes.push("user ID".to_string());
        }
    }
    changes
}

/// Applies the selected Add and Update items, appends an `ImportRecord` to
/// `store.imports` and returns it. The caller saves the store.
///
/// Every item is checked against the store again, so a plan that is stale or
/// applied twice cannot create duplicates or overwrite anything.
pub fn apply(store: &mut Store, plan: &ImportPlan, now: i64) -> ImportRecord {
    let mut record = ImportRecord {
        at: now,
        source: plan.source.clone(),
        accounts_added: 0,
        accounts_updated: 0,
        destinations_added: 0,
        skipped: 0,
    };
    for item in &plan.items {
        match item.action {
            PlanAction::Add | PlanAction::Update { .. } if item.selected => {}
            PlanAction::Unchanged => continue,
            // Skipped by the plan, or unticked by the user.
            _ => {
                record.skipped += 1;
                continue;
            }
        }
        match &item.payload {
            Payload::None => {}
            Payload::Account(legacy) => match find_account(store, legacy) {
                Some(index) => {
                    if !fill(&mut store.accounts[index], legacy).is_empty() {
                        record.accounts_updated += 1;
                    }
                }
                None => {
                    let import_key = match legacy.user_id {
                        Some(id) => format!("legacy:user:{id}"),
                        None => format!("legacy:name:{}", legacy.username.to_lowercase()),
                    };
                    let mut account = Account {
                        id: store.alloc_id(),
                        username: legacy.username.clone(),
                        alias: legacy.alias.clone(),
                        // From a file, not from Roblox.
                        roblox_user_id: legacy.user_id,
                        user_id_confirmed: false,
                        group: import_group(&legacy.group).to_string(),
                        notes: legacy.description.clone(),
                        created: now,
                        last_used: legacy.last_use,
                        import_key: Some(import_key),
                        ..Account::default()
                    };
                    signin::require_fresh_sign_in(&mut account.sign_in);
                    store.accounts.push(account);
                    record.accounts_added += 1;
                }
            },
            Payload::Destination { target, source } => {
                if !store.destinations.iter().any(|d| &d.target == target) {
                    let id = store.alloc_id();
                    store.destinations.push(Destination {
                        id,
                        name: item.label.clone(),
                        target: target.clone(),
                        notes: String::new(),
                        created: now,
                        last_used: None,
                        import_key: Some(format!("legacy:{source}")),
                    });
                    record.destinations_added += 1;
                }
            }
            Payload::Game(game) => {
                if !store.games.iter().any(|g| g.place_id == game.place_id) {
                    store.games.push(Game {
                        place_id: game.place_id,
                        universe_id: game.universe_id,
                        name: game.name.clone(),
                        pinned: false,
                        builtin: false,
                        ..Default::default()
                    });
                }
            }
        }
    }
    store.imports.push(record.clone());
    record
}

/// A password-protected account file produced by the legacy app's own
/// native library, for the tests here and in `import_cookies`.
#[cfg(test)]
pub(crate) mod test_vectors {
    use super::{derive_key, PasswordFileError, KEY_LEN};
    use std::sync::Mutex;
    use zeroize::Zeroizing;

    /// `derive_key`, one call at a time and once per input, for every test in
    /// the crate: a derivation holds 128 MiB and is slow in a test build.
    pub(crate) fn derive_key_once(hashed: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, PasswordFileError> {
        type Derived = (Vec<u8>, Vec<u8>, [u8; KEY_LEN]);
        static DERIVED: Mutex<Vec<Derived>> = Mutex::new(Vec::new());
        let mut derived = DERIVED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, key)) = derived.iter().find(|(h, s, _)| h == hashed && s == salt) {
            return Ok(Zeroizing::new(*key));
        }
        let key = derive_key(hashed, salt)?;
        derived.push((hashed.to_vec(), salt.to_vec(), *key));
        Ok(key)
    }

    pub(crate) const VECTOR_PASSWORD: &str = "correct horse";

    /// Differs by a trailing space: the password is taken as typed. The
    /// tests that need a wrong password all use this one.
    pub(crate) const WRONG_PASSWORD: &str = "correct horse ";

    /// The (made-up) account list inside the file.
    pub(crate) const VECTOR_JSON: &str = r#"[{"Username":"demo_user","UserID":42,"SecurityToken":"_|WARNING:-DO-NOT-SHARE-THIS.--Sharing-this-will-allow-someone-to-log-in-as-you-and-to-steal-your-ROBUX-and-items.|_FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE","Password":"hunter2","Alias":"a","Description":"d","Group":"G"}]"#;

    /// Written the way `Cryptography.Encrypt` does it, by calling the
    /// `libsodium.dll` that ships with the legacy app (libsodium 1.0.11):
    /// `pw = crypto_hash("correct horse")`,
    /// `key = crypto_pwhash(32, pw, salt, 6, 134217728, 1)`,
    /// `crypto_secretbox_easy(VECTOR_JSON, nonce, key)`, with the salt fixed
    /// to `a0..af` and the nonce to `10..27` (the app draws both at random).
    const VECTOR_HEX: &str = "\
        526f626c6f78204163636f756e74204d616e6167657220637265617465642062792069633377306c663232204020676974687562\
        2e636f6d202e2e2e2e2e2e2ea0a1a2a3a4a5a6a7a8a9aaabacadaeaf101112131415161718191a1b1c1d1e1f2021222324252627\
        eed152882f108c7e58185a7d49467b71e336bf09e43481c8805ca4f689043cf6b8444701472d1f270d36bbdb0346eda752be0706\
        fe01046602328d60eb3fb00435656b32b6ddf67bd72e08c903e3386ee98b6ef9d320ab839202021ca9e462b64e76300a005e82fb\
        63636a9b8b193c35f877507458c6167c238f42792fd66611bf17b949367111e05aa38d65446098eb85ffac90c9068c2ddf7424de\
        373e7cb263e8ad3790fde50a5a6886dce8f803b50dd91d39e42c3d9070575da208ffed28cbc01df556fc0c6205e7d6294a5edf88\
        8813c9a57301fd7e0cfea3af147088ceeacd24e314645b1076a6e4f62794add74db16ff83cbc93e4ad69d3244d156d1384c767f2\
        9d9d0d254dbc780613ac243cfda4016a9d308dea97c9e811aafb2b37e53bf959b55b142615f3ed8456afe0f39ebd99b820a3496c";

    pub(crate) fn libsodium_file() -> Vec<u8> {
        let digit = |c: u8| (c as char).to_digit(16).unwrap() as u8;
        VECTOR_HEX.as_bytes().chunks(2).map(|pair| digit(pair[0]) << 4 | digit(pair[1])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_vectors::{libsodium_file, VECTOR_JSON, VECTOR_PASSWORD, WRONG_PASSWORD};
    use super::*;
    use crate::model::{SignInState, ROGUE_LINEAGE_PLACE_ID};

    const TOKEN: &str = "_|WARNING:-DO-NOT-SHARE-THIS.--COOKIEZZ9917";
    const PASSWORD: &str = "hunter2-PWZZ4471";
    const CODE: &str = "LinkCodeZZ7733abcdef";
    const CODE2: &str = "OtherCodeZZ5522";
    const SCRIPT: &str = "EXPLOITZZ6601";
    const INI_SECRET: &str = "WSPWZZ3308";
    const JOB: &str = "0F6AD7A2-3C1B-4C0E-9A57-0123456789AB";
    const NOW: i64 = 1_800_000_000;

    fn accounts_json() -> String {
        format!(
            r#"[
 {{"Valid":true,"SecurityToken":"{TOKEN}","Username":"AlphaFarmer","LastUse":"2023-08-14T21:05:33.1234567+02:00","Alias":"Main farm","Description":"Trades on weekends","Password":"{PASSWORD}","Group":"001 Farm","UserID":1001,"Fields":{{"SavedPlaceId":"3016661674","SavedJobId":"https://www.roblox.com/games/3016661674/Rogue-Lineage?privateServerLinkCode={CODE}","Window_Position_X":"10","MyNote":"x"}},"LastAttemptedRefresh":"0001-01-01T00:00:00","BrowserTrackerID":"123456789012"}},
 {{"Valid":true,"SecurityToken":"{TOKEN}-2","Username":"BetaAlt","LastUse":"2024-01-02T03:04:05Z","Alias":"","Description":"","Password":"","UserID":1002,"Fields":{{"SavedPlaceId":"606849621","SavedJobId":"{JOB}"}}}},
 {{"Valid":false,"SecurityToken":"{TOKEN}-3","Username":"GammaNoId","Alias":"Gamma","Description":"no id","Password":"{PASSWORD}","Group":"Default","UserID":0,"Fields":{{}}}}
]"#
        )
    }

    fn favorites_json() -> String {
        format!(
            r#"[
 {{"Name":"Lineage VIP (VIP)","PrivateServer":"https://www.roblox.com/games/3016661674/Rogue-Lineage?privateServerLinkCode={CODE2}","PlaceID":5315046213,"Details":{{"placeId":3016661674,"name":"Rogue Lineage","universeId":1087859240,"filteredName":"Rogue Lineage"}},"ImageUrl":"x"}},
 {{"Name":"Tower","PrivateServer":"","PlaceID":1962086868,"Details":{{"placeId":1962086868,"name":"Tower of Hell","universeId":703124385}},"ImageUrl":null}}
]"#
        )
    }

    const RECENT: &str = r#"[{"Details":{"placeId":606849621,"name":"Jailbreak","universeId":245662005,"filteredName":"Jailbreak"},"ImageUrl":"u"},{"Details":{"placeId":4442272183,"name":"Unknown"},"ImageUrl":null}]"#;

    fn control_json() -> String {
        format!(
            r#"[{{"Username":"AlphaFarmer","AutoExecute":"loadstring(game:HttpGet('{SCRIPT}'))()","PlaceId":920587237,"JobId":"","RelaunchDelay":30.0,"AutoRelaunch":true}},{{"Username":"BetaAlt","AutoExecute":"","PlaceId":0,"JobId":""}}]"#
        )
    }

    fn ini() -> String {
        format!("; settings\n[General]\nSavedPlaceId = 5315046213\nNopechaKey={INI_SECRET}\n[WebServer]\nPassword={INI_SECRET}\nSavedPlaceId=1\n")
    }

    type Files = Vec<(&'static str, Vec<u8>)>;

    fn full_files() -> Files {
        vec![
            (ACCOUNT_FILE, accounts_json().into_bytes()),
            (FAVORITES_FILE, favorites_json().into_bytes()),
            (RECENT_FILE, RECENT.as_bytes().to_vec()),
            (CONTROL_FILE, control_json().into_bytes()),
            (SETTINGS_FILE, ini().into_bytes()),
        ]
    }

    fn with(mut files: Files, name: &'static str, bytes: Vec<u8>) -> Files {
        files.retain(|(n, _)| *n != name);
        files.push((name, bytes));
        files
    }

    fn scan_of(files: &Files, decryptor: Option<&dyn LegacyDecryptor>) -> Scan {
        scan_files(
            Path::new("C:/legacy"),
            &|name| files.iter().find(|(n, _)| *n == name).map(|(_, b)| b.clone()),
            decryptor,
        )
    }

    fn scan_accounts_only(json: &str) -> Scan {
        scan_of(&vec![(ACCOUNT_FILE, json.as_bytes().to_vec())], None)
    }

    fn import(store: &mut Store, scan: &Scan) -> ImportRecord {
        let plan = plan(store, scan);
        apply(store, &plan, NOW)
    }

    fn find<'a>(plan: &'a ImportPlan, kind: PlanKind, label: &str) -> &'a PlanItem {
        plan.items
            .iter()
            .find(|i| i.kind == kind && i.label == label)
            .unwrap_or_else(|| panic!("no {kind:?} item labelled {label}"))
    }

    struct FakeDecryptor(Result<Vec<u8>, String>);
    impl LegacyDecryptor for FakeDecryptor {
        fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
            assert!(data.starts_with(&DPAPI_MAGIC));
            assert_eq!(entropy, LEGACY_DPAPI_ENTROPY);
            self.0.clone()
        }
    }

    fn dpapi_blob() -> Vec<u8> {
        let mut blob = DPAPI_MAGIC.to_vec();
        blob.extend_from_slice(&[0xAB; 200]);
        blob
    }

    fn password_blob() -> Vec<u8> {
        let mut blob = PASSWORD_HEADER.to_vec();
        blob.extend_from_slice(&[0x5A; 120]);
        blob
    }

    // --- literals -----------------------------------------------------------

    #[test]
    fn literals_match_the_legacy_source_bytes() {
        // Classes/Cryptography.cs:10 (RAMHeader), copied as written there.
        let header: [u8; 64] = [
            82, 111, 98, 108, 111, 120, 32, 65, 99, 99, 111, 117, 110, 116, 32, 77, 97, 110, 97, 103, 101, 114, 32,
            99, 114, 101, 97, 116, 101, 100, 32, 98, 121, 32, 105, 99, 51, 119, 48, 108, 102, 50, 50, 32, 64, 32, 103,
            105, 116, 104, 117, 98, 46, 99, 111, 109, 32, 46, 46, 46, 46, 46, 46, 46,
        ];
        assert_eq!(PASSWORD_HEADER, &header);
        // AccountManager.cs:93 (Entropy), copied as written there.
        let entropy: [u8; 56] = [
            0x52, 0x4f, 0x42, 0x4c, 0x4f, 0x58, 0x20, 0x41, 0x43, 0x43, 0x4f, 0x55, 0x4e, 0x54, 0x20, 0x4d, 0x41,
            0x4e, 0x41, 0x47, 0x45, 0x52, 0x20, 0x7c, 0x20, 0x3a, 0x29, 0x20, 0x7c, 0x20, 0x42, 0x52, 0x4f, 0x55,
            0x47, 0x48, 0x54, 0x20, 0x54, 0x4f, 0x20, 0x59, 0x4f, 0x55, 0x20, 0x42, 0x55, 0x59, 0x20, 0x69, 0x63,
            0x33, 0x77, 0x30, 0x6c, 0x66,
        ];
        assert_eq!(LEGACY_DPAPI_ENTROPY, &entropy);
    }

    // --- detection ----------------------------------------------------------

    #[test]
    fn detects_each_variant() {
        assert_eq!(detect_account_file(&password_blob()), AccountFileKind::PasswordProtected);
        assert_eq!(detect_account_file(&PASSWORD_HEADER[..]), AccountFileKind::PasswordProtected);
        assert_eq!(detect_account_file(&dpapi_blob()), AccountFileKind::Dpapi);
        assert_eq!(detect_account_file(&DPAPI_MAGIC), AccountFileKind::Dpapi);
        assert_eq!(detect_account_file(b"[]"), AccountFileKind::Plaintext);
        assert_eq!(detect_account_file(b"\xEF\xBB\xBF \r\n\t[{}]"), AccountFileKind::Plaintext);
        assert_eq!(detect_account_file(accounts_json().as_bytes()), AccountFileKind::Plaintext);
        assert_eq!(detect_account_file(b"{\"a\":1}"), AccountFileKind::Unrecognised);
        assert_eq!(detect_account_file(b"hello"), AccountFileKind::Unrecognised);
    }

    #[test]
    fn short_files_do_not_crash_detection() {
        assert_eq!(detect_account_file(b""), AccountFileKind::Unrecognised);
        assert_eq!(detect_account_file(b"   "), AccountFileKind::Unrecognised);
        assert_eq!(detect_account_file(b"["), AccountFileKind::Plaintext);
        assert_eq!(detect_account_file(b"\xEF\xBB"), AccountFileKind::Unrecognised);
        for len in 0..64 {
            assert_eq!(detect_account_file(&PASSWORD_HEADER[..len]), AccountFileKind::Unrecognised, "{len}");
        }
        for len in 0..20 {
            assert_eq!(detect_account_file(&DPAPI_MAGIC[..len]), AccountFileKind::Unrecognised, "{len}");
        }
        // A header with one byte changed is not password mode.
        let mut almost = password_blob();
        almost[63] = b'!';
        assert_eq!(detect_account_file(&almost), AccountFileKind::Unrecognised);
    }

    // --- scanning -----------------------------------------------------------

    #[test]
    fn plaintext_scan_reads_everything() {
        let scan = scan_of(&full_files(), None);
        assert_eq!(scan.folder, PathBuf::from("C:/legacy"));
        assert_eq!(scan.account_file, AccountFileKind::Plaintext);
        assert_eq!(scan.accounts.len(), 3);

        let a = &scan.accounts[0];
        assert_eq!((a.username.as_str(), a.alias.as_str()), ("AlphaFarmer", "Main farm"));
        assert_eq!(a.description, "Trades on weekends");
        assert_eq!(a.group, "001 Farm");
        assert_eq!(a.user_id, Some(1001));
        assert_eq!(a.last_use, Some(1_692_039_933));
        assert_eq!(a.saved, Some(Target::PrivateServer { place_id: 3016661674, link_code: CODE.into() }));
        assert_eq!(a.saved_skip_reason, None);

        let b = &scan.accounts[1];
        assert_eq!(b.group, "Default", "absent group is the legacy default");
        assert_eq!(b.last_use, Some(1_704_164_645));
        assert_eq!(b.saved, Some(Target::LiveServer { place_id: 606849621, job_id: JOB.to_lowercase() }));

        let c = &scan.accounts[2];
        assert_eq!((c.user_id, c.last_use, &c.saved, &c.saved_skip_reason), (None, None, &None, &None));

        assert_eq!(
            scan.destinations,
            vec![
                LegacyDestination {
                    name: "Lineage VIP (VIP)".into(),
                    target: Target::PrivateServer { place_id: 3016661674, link_code: CODE2.into() },
                    source: FAVORITES_FILE,
                },
                LegacyDestination {
                    name: "Tower".into(),
                    target: Target::Place { place_id: 1962086868 },
                    source: FAVORITES_FILE,
                },
                LegacyDestination {
                    name: "AlphaFarmer relaunch target".into(),
                    target: Target::Place { place_id: 920587237 },
                    source: CONTROL_FILE,
                },
            ]
        );
        let game = |place_id, name: &str, universe_id| LegacyGame { place_id, name: name.into(), universe_id };
        assert_eq!(
            scan.games,
            vec![
                game(606849621, "Jailbreak", Some(245662005)),
                game(4442272183, "Place 4442272183", None),
                game(3016661674, "Rogue Lineage", Some(1087859240)),
                game(1962086868, "Tower of Hell", Some(703124385)),
                game(5315046213, "Place 5315046213", None),
            ]
        );
        // The only notice is the custom field count ("MyNote").
        assert_eq!(scan.notices.len(), 1, "{:?}", scan.notices);
        assert!(scan.notices[0].contains("1 custom account field"));
    }

    #[test]
    fn missing_files_each_produce_a_notice() {
        let scan = scan_of(&Vec::new(), None);
        assert_eq!(scan.account_file, AccountFileKind::Missing);
        assert!(scan.accounts.is_empty() && scan.destinations.is_empty() && scan.games.is_empty());
        for name in [ACCOUNT_FILE, SETTINGS_FILE, RECENT_FILE, FAVORITES_FILE, CONTROL_FILE] {
            assert_eq!(scan.notices.iter().filter(|n| n.contains(name)).count(), 1, "{name}");
        }
        let plan = plan(&Store::default(), &scan);
        assert!(plan.items.is_empty());
        assert_eq!(plan.notices, scan.notices);
    }

    #[test]
    fn dpapi_file_decrypts_through_the_decryptor() {
        let files = with(full_files(), ACCOUNT_FILE, dpapi_blob());
        let ok = FakeDecryptor(Ok(accounts_json().into_bytes()));
        let scan = scan_of(&files, Some(&ok));
        assert_eq!(scan.account_file, AccountFileKind::Dpapi);
        assert_eq!(scan.accounts, scan_of(&full_files(), None).accounts);

        // A BOM in the decrypted payload is tolerated.
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(accounts_json().as_bytes());
        assert_eq!(scan_of(&files, Some(&FakeDecryptor(Ok(bom)))).accounts.len(), 3);
    }

    #[test]
    fn dpapi_without_decryptor_or_with_failure_is_a_partial_import() {
        let files = with(full_files(), ACCOUNT_FILE, dpapi_blob());
        let failing = FakeDecryptor(Err(format!("key not valid {TOKEN}")));
        let garbage = FakeDecryptor(Ok(b"not json at all".to_vec()));
        for decryptor in [None, Some(&failing as &dyn LegacyDecryptor)] {
            let scan = scan_of(&files, decryptor);
            assert_eq!(scan.account_file, AccountFileKind::Dpapi);
            assert!(scan.accounts.is_empty());
            assert_eq!(scan.destinations.len(), 3);
            assert_eq!(scan.games.len(), 5);
            let notice = scan.notices.iter().find(|n| n.contains(ACCOUNT_FILE)).unwrap();
            assert!(notice.contains("the PC that wrote it"), "{notice}");
            assert!(!format!("{scan:?}").contains(TOKEN));
        }
        let scan = scan_of(&files, Some(&garbage));
        assert!(scan.accounts.is_empty());
        assert!(scan.notices.iter().any(|n| n.contains(ACCOUNT_FILE) && n.contains("no accounts")));
        assert_eq!(scan.destinations.len(), 3);
    }

    #[test]
    fn password_protected_file_still_imports_the_rest() {
        let files = with(full_files(), ACCOUNT_FILE, password_blob());
        let scan = scan_of(&files, Some(&FakeDecryptor(Err("must not matter".into()))));
        assert_eq!(scan.account_file, AccountFileKind::PasswordProtected);
        assert!(scan.needs_password && !scan.wrong_password);
        assert!(scan.accounts.is_empty());
        let notice = scan.notices.iter().find(|n| n.contains(ACCOUNT_FILE)).unwrap();
        assert!(notice.contains("password-protected"));
        assert!(notice.contains("enter the password"));
        assert!(notice.contains("other files still import"));
        assert!(notice.contains("remove the password"));

        let mut store = Store::default();
        let record = import(&mut store, &scan);
        assert_eq!((record.accounts_added, record.destinations_added), (0, 3));
        assert!(store.accounts.is_empty());
        assert!(store.games.iter().any(|g| g.name == "Jailbreak"));
        assert!(store.games.iter().any(|g| g.name == "Tower of Hell"));
    }

    #[test]
    fn malformed_account_file_with_valid_other_files() {
        for bad in ["[{\"Username\":\"x\",", "[1,2", "{\"Username\":\"x\"}", "[] trailing", "[\"\\ud800\"]"] {
            let scan = scan_of(&with(full_files(), ACCOUNT_FILE, bad.as_bytes().to_vec()), None);
            assert!(scan.accounts.is_empty(), "{bad}");
            assert!(scan.notices.iter().any(|n| n.contains(ACCOUNT_FILE) && n.contains("no accounts")), "{bad}");
            assert_eq!(scan.destinations.len(), 3, "{bad}");
            assert_eq!(scan.games.len(), 5, "{bad}");
        }
        // Not JSON and not any known variant.
        let scan = scan_of(&with(full_files(), ACCOUNT_FILE, b"\x00\x01\x02binary".to_vec()), None);
        assert_eq!(scan.account_file, AccountFileKind::Unrecognised);
        assert!(scan.notices.iter().any(|n| n.contains("not in a recognised format")));
        assert_eq!(scan.destinations.len(), 3);
    }

    #[test]
    fn malformed_other_files_do_not_stop_accounts() {
        let mut files = full_files();
        files = with(files, FAVORITES_FILE, b"[{\"Name\":".to_vec());
        files = with(files, RECENT_FILE, b"{}".to_vec());
        files = with(files, CONTROL_FILE, b"\xff\xfe\x00".to_vec());
        files = with(files, SETTINGS_FILE, b"\xff\xfe[General\nSavedPlaceId\n=5\n".to_vec());
        let scan = scan_of(&files, None);
        assert_eq!(scan.accounts.len(), 3);
        assert!(scan.destinations.is_empty() && scan.games.is_empty());
        for name in [FAVORITES_FILE, RECENT_FILE, CONTROL_FILE] {
            assert!(scan.notices.iter().any(|n| n.contains(name) && n.contains("skipped")), "{name}");
        }
    }

    #[test]
    fn wrong_types_and_huge_numbers_are_tolerated() {
        let scan = scan_accounts_only(
            r#"[1, "x", null, [[]],
                {"Username":5,"UserID":"77","Alias":null,"Group":[],"Fields":"nope","LastUse":12,"Description":{"a":1}},
                {"Username":"big","UserID":99999999999999999999999999,"Fields":{"SavedPlaceId":5,"SavedJobId":null}},
                {"Username":"neg","UserID":-4,"LastUse":"yesterday","Fields":null},
                {"Username":"frac","UserID":12.5,"Fields":[1,2,3]},
                {"Username":"dupkey","Alias":"first","Alias":"second"}]"#,
        );
        assert_eq!(scan.accounts.len(), 5, "{:?}", scan.notices);
        let a = &scan.accounts[0];
        assert_eq!((a.username.as_str(), a.user_id, a.alias.as_str(), a.group.as_str()), ("5", Some(77), "", "Default"));
        assert_eq!((a.description.as_str(), a.last_use), ("", None));
        assert_eq!(scan.accounts[1].user_id, None);
        assert_eq!(scan.accounts[1].saved, Some(Target::Place { place_id: 5 }));
        assert_eq!((scan.accounts[2].user_id, scan.accounts[2].last_use), (None, None));
        assert_eq!(scan.accounts[3].user_id, None);
        assert_eq!(scan.accounts[4].alias, "second");
        assert!(scan.notices.iter().any(|n| n.starts_with("4 entries")));
    }

    #[test]
    fn truncated_and_corrupted_input_never_panics() {
        let json = accounts_json();
        let store = Store::default();
        for end in 0..=json.len() {
            let scan = scan_of(&vec![(ACCOUNT_FILE, json.as_bytes()[..end].to_vec())], None);
            let _ = plan(&store, &scan);
            if end < json.trim_end().len() {
                assert!(scan.accounts.is_empty(), "prefix of {end} bytes parsed");
            }
        }
        let favorites = favorites_json();
        for end in 0..=favorites.len() {
            let files: Files = vec![
                (FAVORITES_FILE, favorites.as_bytes()[..end].to_vec()),
                (RECENT_FILE, RECENT.as_bytes()[..end.min(RECENT.len())].to_vec()),
                (SETTINGS_FILE, ini().as_bytes()[..end.min(ini().len())].to_vec()),
            ];
            let _ = plan(&store, &scan_of(&files, None));
        }
        // Every byte value at a few positions, including invalid UTF-8.
        for position in [0, 1, 40, 300] {
            for byte in [0x00u8, 0x22, 0x5C, 0x7B, 0x80, 0xC3, 0xFF] {
                let mut bytes = json.clone().into_bytes();
                bytes[position] = byte;
                let _ = plan(&store, &scan_of(&vec![(ACCOUNT_FILE, bytes)], None));
            }
        }
        // Deep nesting hits the parser's depth limit instead of the stack.
        let deep = format!("[{}{}]", "[".repeat(5000), "]".repeat(5000));
        assert!(scan_accounts_only(&deep).accounts.is_empty());
        let deep_fields = format!("[{{\"Username\":\"a\",\"Fields\":{}1{}}}]", "{\"k\":".repeat(5000), "}".repeat(5000));
        assert!(scan_accounts_only(&deep_fields).accounts.is_empty());
    }

    #[test]
    fn oversized_files_are_skipped_with_a_notice() {
        let scan = scan_files(
            Path::new("x"),
            &|name| match name {
                RECENT_FILE => Some(vec![b' '; MAX_FILE_BYTES + 1]),
                FAVORITES_FILE => Some(favorites_json().into_bytes()),
                _ => None,
            },
            None,
        );
        assert!(scan.notices.iter().any(|n| n.contains(RECENT_FILE) && n.contains("larger than 64 MiB")));
        assert_eq!(scan.destinations.len(), 2);
    }

    // --- secrets ------------------------------------------------------------

    #[test]
    fn secrets_never_leave_the_parser() {
        let mut files = full_files();
        // Favourites that carry the private code in awkward places: as the
        // name itself, and in an entry that is skipped with a notice.
        let favorites = format!(
            r#"[{{"Name":"https://www.roblox.com/games/77?privateServerLinkCode={CODE}","PrivateServer":"https://www.roblox.com/games/77/x?privateServerLinkCode={CODE}","PlaceID":77,"Details":null}},
                {{"Name":"{CODE}","PrivateServer":"privateServerLinkCode={CODE}&x=1","PlaceID":78,"Details":{{"placeId":78,"name":"G"}}}},
                {{"Name":"Broken","PrivateServer":"privateServerLinkCode={CODE}","PlaceID":0,"Details":null}},
                {{"Name":"Bad code","PrivateServer":"https://www.roblox.com/games/5?privateServerLinkCode={CODE}%22","PlaceID":5,"Details":null}}]"#
        );
        files = with(files, FAVORITES_FILE, favorites.into_bytes());
        let scan = scan_of(&files, None);
        assert_eq!(scan.destinations.len(), 3, "{:?}", scan.notices);
        assert_eq!(scan.notices.iter().filter(|n| n.contains("was skipped")).count(), 2);

        let secrets = [TOKEN, PASSWORD, SCRIPT, INI_SECRET, "COOKIEZZ", "PWZZ"];
        let scan_debug = format!("{scan:?}");
        let mut store = Store::default();
        let plan = plan(&store, &scan);
        let plan_debug = format!("{plan:?}");
        let mut shown = vec![scan_debug, plan_debug];
        shown.extend(scan.notices.iter().cloned());
        shown.extend(plan.notices.iter().cloned());
        for item in &plan.items {
            shown.push(item.label.clone());
            shown.push(item.detail.clone());
            shown.push(format!("{:?}", item.action));
            shown.push(format!("{item:?}"));
        }
        for text in &shown {
            for secret in secrets {
                assert!(!text.contains(secret), "secret {secret} in: {text}");
            }
            assert!(!text.contains(CODE), "link code in: {text}");
            assert!(!text.contains(CODE2), "link code in: {text}");
            assert!(!text.contains(&JOB.to_lowercase()), "full job ID in: {text}");
        }

        let record = apply(&mut store, &plan, NOW);
        let saved = serde_json::to_string(&store).unwrap();
        for secret in secrets {
            assert!(!saved.contains(secret), "secret {secret} in the store");
        }
        assert!(!format!("{record:?}").contains(CODE));
        // The link code legitimately lives in the destination's target, and
        // nowhere else in the store.
        assert!(saved.contains(CODE));
        for d in &store.destinations {
            assert!(!d.name.contains(CODE) && !d.notes.contains(CODE), "{}", d.name);
            assert!(!d.import_key.as_deref().unwrap_or("").contains(CODE));
        }
        assert!(store
            .destinations
            .iter()
            .any(|d| d.target == Target::PrivateServer { place_id: 77, link_code: CODE.into() }));
    }

    #[test]
    fn wipe_zeroes_the_buffer() {
        let mut buf = TOKEN.as_bytes().to_vec();
        wipe(&mut buf);
        assert_eq!(buf.len(), TOKEN.len());
        assert!(buf.iter().all(|b| *b == 0));
    }

    // --- plan and apply -----------------------------------------------------

    #[test]
    fn first_import_adds_everything() {
        let scan = scan_of(&full_files(), None);
        let mut store = Store::default();
        let before = store.clone();
        let plan = plan(&store, &scan);
        assert_eq!(store, before, "plan must not modify the store");
        assert_eq!(plan.source, scan.folder.display().to_string());
        // 3 accounts + 5 destinations + 4 games; Rogue Lineage is built in.
        assert_eq!(plan.counts(), (12, 0, 1, 0));
        assert!(plan.items.iter().all(|i| i.selected == (i.action == PlanAction::Add)));
        assert_eq!(find(&plan, PlanKind::Game, "Rogue Lineage").action, PlanAction::Unchanged);
        assert_eq!(find(&plan, PlanKind::Account, "Main farm").detail, "AlphaFarmer · group Farm · user ID 1001 · has notes");
        assert_eq!(
            find(&plan, PlanKind::Destination, "Main farm saved server").detail,
            "Private server, place 3016661674 · from AccountData.json"
        );
        find(&plan, PlanKind::Destination, "BetaAlt saved server");

        let record = apply(&mut store, &plan, NOW);
        assert_eq!(
            record,
            ImportRecord {
                at: NOW,
                source: "C:/legacy".into(),
                accounts_added: 3,
                accounts_updated: 0,
                destinations_added: 5,
                skipped: 0
            }
        );
        assert_eq!(store.imports, vec![record]);

        // Accounts keep the legacy display order.
        let names: Vec<&str> = store.accounts.iter().map(|a| a.username.as_str()).collect();
        assert_eq!(names, ["AlphaFarmer", "BetaAlt", "GammaNoId"]);
        let a = &store.accounts[0];
        assert_eq!((a.alias.as_str(), a.notes.as_str(), a.group.as_str()), ("Main farm", "Trades on weekends", "001 Farm"));
        assert_eq!((a.roblox_user_id, a.user_id_confirmed), (Some(1001), false));
        assert_eq!((a.created, a.last_used), (NOW, Some(1_692_039_933)));
        assert_eq!(a.import_key.as_deref(), Some("legacy:user:1001"));
        assert_eq!(a.sign_in.state, SignInState::Unknown);
        assert_eq!(a.sign_in.last_check, None);
        assert!(a.sign_in.detail.contains("Sign in again"));
        assert!(!a.favorite && a.tags.is_empty() && a.browser.is_none());
        let c = &store.accounts[2];
        assert_eq!(c.import_key.as_deref(), Some("legacy:name:gammanoid"));
        assert_eq!((c.roblox_user_id, c.last_used), (None, None));

        let mut ids: Vec<u64> = store.accounts.iter().map(|a| a.id).chain(store.destinations.iter().map(|d| d.id)).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8, "every new entry gets its own ID");
        assert!(store.next_id > *ids.last().unwrap());

        let d = store.destinations.iter().find(|d| d.name == "Lineage VIP (VIP)").unwrap();
        assert_eq!(d.import_key.as_deref(), Some("legacy:FavoriteGames.json"));
        assert_eq!(d.created, NOW);
        let d = store.destinations.iter().find(|d| d.name == "AlphaFarmer relaunch target").unwrap();
        assert_eq!(d.import_key.as_deref(), Some("legacy:AccountControlData.json"));
        let d = store.destinations.iter().find(|d| d.name == "Main farm saved server").unwrap();
        assert_eq!(d.import_key.as_deref(), Some("legacy:AccountData.json"));

        let jailbreak = store.games.iter().find(|g| g.place_id == 606849621).unwrap();
        assert_eq!((jailbreak.name.as_str(), jailbreak.universe_id, jailbreak.pinned, jailbreak.builtin), ("Jailbreak", Some(245662005), false, false));
        assert_eq!(store.game_name(5315046213), "Place 5315046213");
        let lineage = store.games.iter().find(|g| g.place_id == ROGUE_LINEAGE_PLACE_ID).unwrap();
        assert!(lineage.builtin && lineage.pinned && lineage.universe_id.is_none(), "existing games are left alone");
    }

    #[test]
    fn default_group_becomes_ungrouped() {
        let mut store = Store::default();
        import(&mut store, &scan_of(&full_files(), None));
        assert_eq!(store.accounts[1].group, "", "absent group");
        assert_eq!(store.accounts[2].group, "", "explicit Default");
        assert_eq!(store.groups(), vec!["001 Farm".to_string()]);
    }

    #[test]
    fn repeated_import_is_idempotent() {
        let scan = scan_of(&full_files(), None);
        let mut store = Store::default();
        import(&mut store, &scan);
        let after_first = serde_json::to_string(&store).unwrap();

        let second = plan(&store, &scan);
        let (add, update, unchanged, skipped) = second.counts();
        assert_eq!((add, update, skipped), (0, 0, 0), "{second:?}");
        assert_eq!(unchanged, second.items.len());
        assert!(second.items.iter().all(|i| !i.selected));

        let record = apply(&mut store, &second, NOW + 5);
        assert_eq!((record.accounts_added, record.accounts_updated, record.destinations_added, record.skipped), (0, 0, 0, 0));
        assert_eq!(store.imports.len(), 2);
        store.imports.pop();
        assert_eq!(serde_json::to_string(&store).unwrap(), after_first);
    }

    #[test]
    fn applying_a_stale_plan_twice_creates_no_duplicates() {
        let scan = scan_of(&full_files(), None);
        let mut store = Store::default();
        let stale = plan(&store, &scan);
        apply(&mut store, &stale, NOW);
        let after_first = serde_json::to_string(&store).unwrap();
        let record = apply(&mut store, &stale, NOW);
        assert_eq!((record.accounts_added, record.accounts_updated, record.destinations_added), (0, 0, 0));
        store.imports.pop();
        assert_eq!(serde_json::to_string(&store).unwrap(), after_first);
    }

    #[test]
    fn dedupes_by_user_id_when_the_username_changed() {
        let mut store = Store::default();
        let id = store.alloc_id();
        let mut existing = Account {
            id,
            username: "OldName".into(),
            alias: "Mine".into(),
            roblox_user_id: Some(1001),
            user_id_confirmed: true,
            favorite: true,
            tags: vec!["keep".into()],
            created: 7,
            last_used: Some(9),
            ..Account::default()
        };
        existing.sign_in.state = SignInState::Verified;
        existing.sign_in.last_check = Some(8);
        store.accounts.push(existing.clone());

        let scan = scan_of(&full_files(), None);
        let plan = plan(&store, &scan);
        let item = find(&plan, PlanKind::Account, "Main farm");
        assert_eq!(item.action, PlanAction::Update { changes: vec!["notes".into(), "group".into()] });
        assert!(item.selected);
        assert_eq!(item.detail, "fills notes, group of OldName");

        let record = apply(&mut store, &plan, NOW);
        assert_eq!((record.accounts_added, record.accounts_updated), (2, 1));
        assert_eq!(store.accounts.len(), 3);
        let updated = store.account(id).unwrap();
        let expected = Account { notes: "Trades on weekends".into(), group: "001 Farm".into(), ..existing };
        assert_eq!(updated, &expected, "only the empty fields are filled");
    }

    #[test]
    fn dedupes_by_username_and_fills_only_empty_fields() {
        let mut store = Store::default();
        let gamma = store.alloc_id();
        store.accounts.push(Account { id: gamma, username: "gammaNOID".into(), notes: "my own note".into(), ..Account::default() });
        let beta = store.alloc_id();
        store.accounts.push(Account { id: beta, username: "betaalt".into(), group: "Mine".into(), ..Account::default() });
        // Same ID as AlphaFarmer but unconfirmed, with everything filled in.
        let alpha = store.alloc_id();
        store.accounts.push(Account {
            id: alpha,
            username: "Renamed".into(),
            alias: "A".into(),
            notes: "N".into(),
            group: "G".into(),
            roblox_user_id: Some(1001),
            ..Account::default()
        });
        let before = store.clone();

        let scan = scan_of(&full_files(), None);
        let plan = plan(&store, &scan);
        assert_eq!(find(&plan, PlanKind::Account, "Gamma").action, PlanAction::Update { changes: vec!["alias".into()] });
        assert_eq!(find(&plan, PlanKind::Account, "BetaAlt").action, PlanAction::Update { changes: vec!["user ID".into()] });
        let unchanged = find(&plan, PlanKind::Account, "Main farm");
        assert_eq!(unchanged.action, PlanAction::Unchanged);
        assert!(!unchanged.selected);

        let record = apply(&mut store, &plan, NOW);
        assert_eq!((record.accounts_added, record.accounts_updated), (0, 2));
        assert_eq!(store.accounts.len(), 3);
        assert_eq!(store.account(alpha), before.account(alpha));
        let g = store.account(gamma).unwrap();
        assert_eq!((g.alias.as_str(), g.notes.as_str(), g.group.as_str(), g.username.as_str()), ("Gamma", "my own note", "", "gammaNOID"));
        assert_eq!(g.import_key, None);
        let b = store.account(beta).unwrap();
        assert_eq!((b.roblox_user_id, b.user_id_confirmed, b.group.as_str()), (Some(1002), false, "Mine"));
        assert_eq!(b.sign_in, before.account(beta).unwrap().sign_in);
        assert_eq!(b.last_used, None);

        // And now nothing is left to do.
        let (add, update, _, _) = super::plan(&store, &scan).counts();
        assert_eq!((add, update), (0, 0));
    }

    #[test]
    fn duplicates_within_the_source_are_skipped() {
        let scan = scan_accounts_only(
            r#"[{"Username":"One","UserID":5,"Alias":"first"},
                {"Username":"OneRenamed","UserID":5,"Alias":"same id"},
                {"Username":"two","UserID":0},
                {"Username":"TWO","Alias":"same name"},
                {"Username":"  ","Alias":"nameless","UserID":9},
                {"Username":"Three","UserID":6}]"#,
        );
        let mut store = Store::default();
        let plan = plan(&store, &scan);
        let actions: Vec<&PlanAction> = plan.items.iter().map(|i| &i.action).collect();
        let dup = PlanAction::Skip { reason: "duplicate in source".into() };
        let nameless = PlanAction::Skip { reason: "no username".into() };
        assert_eq!(actions, [&PlanAction::Add, &dup, &PlanAction::Add, &dup, &nameless, &PlanAction::Add]);
        assert_eq!(plan.counts(), (3, 0, 0, 3));
        assert!(plan.items.iter().all(|i| i.selected == (i.action == PlanAction::Add)));

        // Ticking a skipped item does not import it.
        let mut forced = plan.clone();
        forced.items.iter_mut().for_each(|i| i.selected = true);
        let record = apply(&mut store, &forced, NOW);
        assert_eq!((record.accounts_added, record.skipped), (3, 3));
        let names: Vec<&str> = store.accounts.iter().map(|a| a.username.as_str()).collect();
        assert_eq!(names, ["One", "two", "Three"]);
    }

    #[test]
    fn destinations_dedupe_against_store_and_source() {
        let mut store = Store::default();
        let id = store.alloc_id();
        store.destinations.push(Destination {
            id,
            name: "My tower".into(),
            target: Target::Place { place_id: 1962086868 },
            notes: String::new(),
            created: 1,
            last_used: None,
            import_key: None,
        });
        let files = with(
            full_files(),
            CONTROL_FILE,
            format!(
                r#"[{{"Username":"A","PlaceId":920587237,"JobId":""}},{{"Username":"B","PlaceId":920587237,"JobId":""}},
                    {{"Username":"C","PlaceId":606849621,"JobId":"{}"}},{{"Username":"D","PlaceId":4,"JobId":"VIP:abc"}}]"#,
                JOB.to_lowercase()
            )
            .into_bytes(),
        );
        let scan = scan_of(&files, None);
        assert!(scan.notices.iter().any(|n| n.contains("\"D relaunch target\" was skipped") && n.contains("access code")));
        let plan = plan(&store, &scan);
        assert_eq!(find(&plan, PlanKind::Destination, "Tower").action, PlanAction::Unchanged);
        assert_eq!(find(&plan, PlanKind::Destination, "A relaunch target").action, PlanAction::Add);
        let dup = PlanAction::Skip { reason: "duplicate in source".into() };
        assert_eq!(find(&plan, PlanKind::Destination, "B relaunch target").action, dup);
        // Same live server as BetaAlt's saved one, in different letter case.
        assert_eq!(find(&plan, PlanKind::Destination, "C relaunch target").action, dup);

        let record = apply(&mut store, &plan, NOW);
        assert_eq!((record.destinations_added, record.skipped), (4, 2));
        assert_eq!(store.destinations.len(), 5);
        assert_eq!(store.destinations[0].name, "My tower");
    }

    #[test]
    fn unticking_an_item_excludes_it() {
        let scan = scan_of(&full_files(), None);
        let mut store = Store::default();
        let mut plan = plan(&store, &scan);
        for label in ["Main farm", "Tower", "Jailbreak"] {
            plan.items.iter_mut().find(|i| i.label == label).unwrap().selected = false;
        }
        let record = apply(&mut store, &plan, NOW);
        assert_eq!((record.accounts_added, record.destinations_added, record.skipped), (2, 4, 3));
        assert!(!store.accounts.iter().any(|a| a.username == "AlphaFarmer"));
        assert!(!store.destinations.iter().any(|d| d.name == "Tower"));
        assert!(!store.games.iter().any(|g| g.place_id == 606849621));
        // The account's saved server is its own item and still imports.
        assert!(store.destinations.iter().any(|d| d.name == "Main farm saved server"));

        // The unticked items are offered again next time.
        let again = super::plan(&store, &scan);
        assert_eq!(again.counts().0, 3);
    }

    #[test]
    fn partial_import_then_full_import_later() {
        let mut store = Store::default();
        let partial = scan_of(&with(full_files(), ACCOUNT_FILE, password_blob()), None);
        let first = import(&mut store, &partial);
        assert_eq!((first.accounts_added, first.destinations_added), (0, 3));

        let full = scan_of(&full_files(), None);
        let plan = plan(&store, &full);
        // 3 accounts and their 2 saved servers are new; the rest is known.
        assert_eq!(plan.counts(), (5, 0, 8, 0));
        let second = apply(&mut store, &plan, NOW + 1);
        assert_eq!((second.accounts_added, second.destinations_added, second.skipped), (3, 2, 0));
        assert_eq!((store.accounts.len(), store.destinations.len(), store.imports.len()), (3, 5, 2));

        let mut direct = Store::default();
        import(&mut direct, &full);
        let targets = |s: &Store| s.destinations.iter().map(|d| d.target.clone()).collect::<HashSet<_>>();
        assert_eq!(targets(&store), targets(&direct));
        assert_eq!(store.games.len(), direct.games.len());
    }

    // --- field shapes -------------------------------------------------------

    #[test]
    fn every_saved_job_id_shape() {
        let lower = JOB.to_lowercase();
        let private = |place_id: u64, code: &str| Ok(Target::PrivateServer { place_id, link_code: code.into() });
        // Empty: the place alone.
        assert_eq!(build_target(Some(5), ""), Ok(Target::Place { place_id: 5 }));
        assert_eq!(build_target(Some(5), "  \t"), Ok(Target::Place { place_id: 5 }));
        assert!(build_target(None, "").is_err());
        // A server job GUID.
        assert_eq!(build_target(Some(5), JOB), Ok(Target::LiveServer { place_id: 5, job_id: lower.clone() }));
        assert_eq!(build_target(Some(5), &format!(" {lower} ")), Ok(Target::LiveServer { place_id: 5, job_id: lower }));
        assert!(build_target(None, JOB).is_err());
        assert!(build_target(Some(5), "not-a-guid").unwrap_err().contains("server ID"));
        // Text with a link code: the place in the link wins over the box.
        assert_eq!(build_target(Some(5), "https://www.roblox.com/games/77/Some-Game?privateServerLinkCode=Ab-c_9"), private(77, "Ab-c_9"));
        assert_eq!(build_target(None, "https://www.roblox.com/games/77?privateServerLinkCode=abc&x=1"), private(77, "abc"));
        assert_eq!(build_target(Some(5), "https://www.roblox.com/games/77?a=1&privateServerLinkCode=abc#frag"), private(77, "abc"));
        assert_eq!(build_target(Some(5), "privateServerLinkCode=abc def"), private(5, "abc"));
        assert_eq!(build_target(Some(5), "https://www.roblox.com/games/0/x?privateserverlinkcode=abc"), private(5, "abc"));
        assert!(build_target(None, "privateServerLinkCode=abc").unwrap_err().contains("place ID"));
        assert!(build_target(Some(5), "privateServerLinkCode=").unwrap_err().contains("link code"));
        let bad = build_target(Some(5), "privateServerLinkCode=SECRET%22x").unwrap_err();
        assert!(bad.contains("link code") && !bad.contains("SECRET"));
        assert!(build_target(Some(5), &format!("privateServerLinkCode={}", "a".repeat(200))).is_err());
        // An access code cannot be used.
        assert_eq!(build_target(Some(5), "VIP:0f6ad7a2-3c1b-4c0e-9a57-0123456789ab"), Err(VIP_REASON.to_string()));
        assert_eq!(build_target(Some(5), " vip:x"), Err(VIP_REASON.to_string()));
        // Non-ASCII text is handled without slicing inside a character.
        assert!(build_target(Some(5), "VIé").is_err());
        assert!(build_target(Some(5), "é/games/éprivateServerLinkCode=é").is_err());
    }

    #[test]
    fn saved_fields_flow_into_the_plan() {
        let scan = scan_accounts_only(&format!(
            r#"[{{"Username":"place","Fields":{{"SavedPlaceId":"11"}}}},
                {{"Username":"job","Fields":{{"SavedPlaceId":"12","SavedJobId":"{JOB}"}}}},
                {{"Username":"link","Alias":"Linky","Fields":{{"SavedPlaceId":"13","SavedJobId":"https://www.roblox.com/games/14/n?privateServerLinkCode={CODE}"}}}},
                {{"Username":"vip","Fields":{{"SavedPlaceId":"15","SavedJobId":"VIP:{CODE2}"}}}},
                {{"Username":"junk","Fields":{{"SavedPlaceId":"abc","SavedJobId":""}}}},
                {{"Username":"url","Fields":{{"SavedPlaceId":"https://www.roblox.com/games/16/Name"}}}},
                {{"Username":"none","Fields":{{"SavedPlaceId":"","SavedJobId":"","Window_Width":"800"}}}}]"#
        ));
        let saved: Vec<Option<Target>> = scan.accounts.iter().map(|a| a.saved.clone()).collect();
        assert_eq!(
            saved,
            [
                Some(Target::Place { place_id: 11 }),
                Some(Target::LiveServer { place_id: 12, job_id: JOB.to_lowercase() }),
                Some(Target::PrivateServer { place_id: 14, link_code: CODE.into() }),
                None,
                None,
                Some(Target::Place { place_id: 16 }),
                None,
            ]
        );
        assert_eq!(scan.accounts[3].saved_skip_reason.as_deref(), Some(VIP_REASON));
        assert!(scan.accounts[4].saved_skip_reason.as_deref().unwrap().contains("place ID"));
        assert_eq!(scan.accounts[6].saved_skip_reason, None);
        let about_accounts = scan.notices.iter().filter(|n| n.contains(ACCOUNT_FILE)).count();
        assert_eq!(about_accounts, 0, "known field keys are not custom: {:?}", scan.notices);

        let mut store = Store::default();
        let plan = plan(&store, &scan);
        assert_eq!(find(&plan, PlanKind::Destination, "place saved place").action, PlanAction::Add);
        assert_eq!(find(&plan, PlanKind::Destination, "job saved server").action, PlanAction::Add);
        assert_eq!(find(&plan, PlanKind::Destination, "Linky saved server").action, PlanAction::Add);
        let vip = find(&plan, PlanKind::Destination, "vip saved server");
        assert_eq!(vip.action, PlanAction::Skip { reason: VIP_REASON.into() });
        assert!(!vip.selected);
        assert!(!format!("{plan:?}").contains(CODE2), "the access code is not shown");
        assert!(matches!(find(&plan, PlanKind::Destination, "junk saved server").action, PlanAction::Skip { .. }));
        let destinations = plan.items.iter().filter(|i| i.kind == PlanKind::Destination).count();
        assert_eq!(destinations, 6);

        let record = apply(&mut store, &plan, NOW);
        assert_eq!((record.accounts_added, record.destinations_added, record.skipped), (7, 4, 2));
    }

    #[test]
    fn favourites_place_id_precedence() {
        let files: Files = vec![(
            FAVORITES_FILE,
            br#"[{"Name":"from link","PrivateServer":"https://www.roblox.com/games/111/x?privateServerLinkCode=aaa","PlaceID":333,"Details":{"placeId":222,"name":"Two"}},
                 {"Name":"from details","PrivateServer":"privateServerLinkCode=bbb","PlaceID":333,"Details":{"placeId":222,"name":"Two"}},
                 {"Name":"from PlaceID","PrivateServer":"privateServerLinkCode=ccc","PlaceID":333,"Details":null},
                 {"Name":"place from details","PrivateServer":"","PlaceID":333,"Details":{"placeId":444,"name":"Unknown"}},
                 {"Name":"","PrivateServer":"","PlaceID":555},
                 {"Name":"","PrivateServer":"","PlaceID":0,"Details":{"placeId":666,"name":"Six"}},
                 {"Name":"nothing","PrivateServer":"","PlaceID":0}]"#
                .to_vec(),
        )];
        let scan = scan_of(&files, None);
        let got: Vec<(&str, &Target)> = scan.destinations.iter().map(|d| (d.name.as_str(), &d.target)).collect();
        let private = |place_id, code: &str| Target::PrivateServer { place_id, link_code: code.into() };
        assert_eq!(
            got,
            [
                ("from link", &private(111, "aaa")),
                ("from details", &private(222, "bbb")),
                ("from PlaceID", &private(333, "ccc")),
                ("place from details", &Target::Place { place_id: 444 }),
                ("Favourite 5", &Target::Place { place_id: 555 }),
                ("Six", &Target::Place { place_id: 666 }),
            ]
        );
        assert!(scan.notices.iter().any(|n| n.contains("\"nothing\" was skipped")));
        // Games come from Details only.
        let games: Vec<(u64, &str)> = scan.games.iter().map(|g| (g.place_id, g.name.as_str())).collect();
        assert_eq!(games, [(222, "Two"), (444, "Place 444"), (666, "Six")]);
    }

    #[test]
    fn games_merge_names_across_files() {
        let files: Files = vec![
            (RECENT_FILE, br#"[{"Details":{"placeId":9,"name":"Unknown"}},{"Details":{"placeId":9,"name":"Nine","universeId":90}},{"Details":{"placeId":0,"name":"zero"}},{"Details":null},{}]"#.to_vec()),
            (SETTINGS_FILE, b"[general]\r\n# comment\r\nsavedplaceid=9\r\n".to_vec()),
        ];
        let scan = scan_of(&files, None);
        assert_eq!(scan.games, vec![LegacyGame { place_id: 9, name: "Nine".into(), universe_id: Some(90) }]);

        // An existing game is never renamed.
        let mut store = Store::default();
        store.games.push(Game { place_id: 9, universe_id: None, name: "My name".into(), pinned: true, builtin: false, ..Default::default() });
        let before = store.games.clone();
        let plan = plan(&store, &scan);
        assert_eq!(find(&plan, PlanKind::Game, "Nine").action, PlanAction::Unchanged);
        apply(&mut store, &plan, NOW);
        assert_eq!(store.games, before);
    }

    #[test]
    fn ini_lookup() {
        let text = "SavedPlaceId=1\n[Other]\nSavedPlaceId=2\n; [General]\n[General]\n#SavedPlaceId=3\nSavedPlaceId = 4=5 \nSavedPlaceId=6\n";
        assert_eq!(ini_value(text, "General", "SavedPlaceId"), Some("4=5"));
        assert_eq!(ini_value(text, "Other", "SavedPlaceId"), Some("2"));
        assert_eq!(ini_value(text, "Missing", "SavedPlaceId"), None);
        assert_eq!(ini_value("[General]\nOther=1", "General", "SavedPlaceId"), None);
        assert_eq!(parse_place_text(" 42 "), Some(42));
        assert_eq!(parse_place_text("0"), None);
        assert_eq!(parse_place_text("99999999999999999999999"), None);
        assert_eq!(parse_place_text("Jailbreak"), None);
        assert_eq!(parse_place_text("https://www.roblox.com/games/606849621/Jailbreak"), Some(606849621));
    }

    #[test]
    fn group_display_names() {
        assert_eq!(group_display_name("001 Farm"), "Farm");
        assert_eq!(group_display_name("1Apple"), "Apple");
        assert_eq!(group_display_name("12\tTabbed"), "Tabbed");
        assert_eq!(group_display_name("Team 12"), "Team 12");
        assert_eq!(group_display_name("123"), "123");
        assert_eq!(group_display_name("12 "), "12 ");
        assert_eq!(group_display_name("1234 Four"), "4 Four");
        assert_eq!(group_display_name("7  Two spaces"), " Two spaces");
        assert_eq!(group_display_name("Default"), "Default");
        assert_eq!(group_display_name(""), "");
        assert_eq!(group_display_name("9é"), "é");
        assert_eq!(group_display_name("1\u{3000}Wide"), "Wide");
    }

    #[test]
    fn legacy_timestamps() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2023-08-14T21:05:33.1234567+02:00"), Some(1_692_039_933));
        assert_eq!(parse_iso8601("2023-08-14T19:05:33"), Some(1_692_039_933));
        assert_eq!(parse_iso8601("2023-08-14 14:35:33-0430"), Some(1_692_039_933));
        assert_eq!(parse_iso8601("2024-02-29T12:00:00.5Z"), Some(1_709_208_000));
        assert_eq!(parse_iso8601("2000-03-01T00:00:00Z"), Some(951_868_800));
        for bad in ["", "0001-01-01T00:00:00", "2023-08-14", "2023-13-01T00:00:00", "2023-08-14T25:00:00", "2023-08-14T21:05:33+2", "2023-08-14T21:05:33 garbage", "20230814T210533Zxxxxxx", "ééééééééééééééééééééé"] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
    }

    // --- disk ---------------------------------------------------------------

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let dir = std::env::temp_dir().join(format!("RAM-import-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let modified = e.metadata().unwrap().modified().unwrap();
                (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap(), modified)
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    #[test]
    fn scanning_a_folder_leaves_it_untouched() {
        let dir = TempDir::new("untouched");
        for (name, bytes) in full_files() {
            std::fs::write(dir.0.join(name), bytes).unwrap();
        }
        std::fs::write(dir.0.join("AccountData.json.backup"), b"left alone").unwrap();
        let before = snapshot(&dir.0);
        assert_eq!(before.len(), 6);

        let scanned = scan(&dir.0, None);
        let mut store = Store::default();
        let record = import(&mut store, &scanned);

        assert_eq!(snapshot(&dir.0), before, "files, contents and modification times are unchanged");
        assert_eq!(scanned.folder, dir.0);
        assert_eq!(record.source, dir.0.display().to_string());
        // Disk and in-memory scans agree.
        let memory = scan_of(&full_files(), None);
        assert_eq!((scanned.account_file, scanned.accounts, scanned.destinations), (memory.account_file, memory.accounts, memory.destinations));
        assert_eq!((scanned.games, scanned.notices), (memory.games, memory.notices));
    }

    #[test]
    fn scanning_odd_folders_reports_instead_of_failing() {
        let missing = scan(Path::new("Z:/definitely/not/a/real/folder-RAM"), None);
        assert_eq!(missing.account_file, AccountFileKind::Missing);
        assert_eq!(missing.notices.len(), 5);

        // A directory where a file is expected, and an empty account file.
        let dir = TempDir::new("odd");
        std::fs::create_dir(dir.0.join(RECENT_FILE)).unwrap();
        std::fs::write(dir.0.join(ACCOUNT_FILE), b"").unwrap();
        std::fs::write(dir.0.join(FAVORITES_FILE), favorites_json()).unwrap();
        let odd = scan(&dir.0, None);
        assert_eq!(odd.account_file, AccountFileKind::Unrecognised);
        assert!(odd.notices.iter().any(|n| n.contains(RECENT_FILE)));
        assert_eq!(odd.destinations.len(), 2);
    }

    // --- password-protected files -------------------------------------------

    #[test]
    fn sha512_matches_known_answers() {
        let hex = |data: &[u8]| sha512(data).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(b""), "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e");
        assert_eq!(hex(b"abc"), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
        // Either side of the point where the padding needs a second block, and more than one block.
        assert_eq!(hex(&[b'a'; 111]), "fa9121c7b32b9e01733d034cfc78cbf67f926c7ed83e82200ef86818196921760b4beff48404df811b953828274461673c68d04e297b0eb7b2b4d60fc6b566a2");
        assert_eq!(hex(&[b'a'; 112]), "c01d080efd492776a1c43bd23dd99d0a2e626d481e16782e75d54c2503b5dc32bd05f0f1ba33e568b88fd2d970929b719ecbb152f58f130a407c8830604b70ca");
        assert_eq!(hex(&[b'a'; 200]), "4b11459c33f52a22ee8236782714c150a3b2c60994e9acee17fe68947a3e6789f31e7668394592da7bef827cddca88c4e6f86e4df7ed1ae6cba71f3e98faee9f");
        // What libsodium's crypto_hash returned for the vector's password.
        assert_eq!(hex(VECTOR_PASSWORD.as_bytes()), "56b698defedb5a435b634afe3320bbaf3fdcd920b6c503a446fc7b7a776b298d479d1ba6a8b617808eb0bf579ce9a95d668347bcab7149085ac93cb27995197b");
    }

    #[test]
    fn opens_a_file_written_by_libsodium() {
        let file = libsodium_file();
        assert_eq!(file.len(), 64 + 16 + 24 + 16 + VECTOR_JSON.len());
        assert_eq!(detect_account_file(&file), AccountFileKind::PasswordProtected);

        let plain = decrypt_password_file(&file, VECTOR_PASSWORD).expect("the right password opens it");
        assert_eq!(String::from_utf8(plain).unwrap(), VECTOR_JSON);
        assert_eq!(decrypt_password_file(&file, WRONG_PASSWORD), Err(PasswordFileError::WrongPassword));

        // The right password does not open a file that was altered, in the box or in the nonce.
        for at in [64 + 16, 64 + 16 + 24, file.len() - 1] {
            let mut altered = file.clone();
            altered[at] ^= 0x01;
            assert_eq!(decrypt_password_file(&altered, VECTOR_PASSWORD), Err(PasswordFileError::WrongPassword), "{at}");
        }
    }

    #[test]
    fn rejects_what_cannot_be_a_password_file() {
        // None of these gets as far as key derivation.
        let not_one: [&[u8]; 4] = [b"", b"[]", &DPAPI_MAGIC, &PASSWORD_HEADER[..63]];
        for bytes in not_one {
            assert_eq!(decrypt_password_file(bytes, "pw"), Err(PasswordFileError::NotPasswordFile));
        }
        let file = libsodium_file();
        for len in [64, 64 + 16, 64 + 16 + 24, 64 + 16 + 24 + 15] {
            assert_eq!(decrypt_password_file(&file[..len], "pw"), Err(PasswordFileError::Malformed), "{len}");
        }
    }

    #[test]
    fn account_file_names_and_backups() {
        assert_eq!(match_account_file_name("AccountData.json"), Some(false));
        assert_eq!(match_account_file_name("accountdata.JSON"), Some(false));
        for name in ["AccountData.json.backup", "ACCOUNTDATA.JSON.BAK", "AccountData.json.old"] {
            assert_eq!(match_account_file_name(name), Some(true), "{name}");
        }
        let others = [
            "",
            "AccountData",
            "AccountData.jsonx",
            "AccountData.json.tmp",
            "xAccountData.json",
            CONTROL_FILE,
            "AccountData.jso\u{144}",
        ];
        for name in others {
            assert_eq!(match_account_file_name(name), None, "{name}");
        }
    }

    #[test]
    fn scan_path_takes_a_folder_or_an_account_file() {
        let dir = TempDir::new("path");
        for (name, bytes) in full_files() {
            std::fs::write(dir.0.join(name), bytes).unwrap();
        }
        let by_folder = scan_path(&dir.0, None, None);
        assert_eq!(by_folder, scan(&dir.0, None));
        assert_eq!(by_folder.accounts.len(), 3);
        let by_file = scan_path(&dir.0.join(ACCOUNT_FILE), None, Some("ignored for a plaintext file"));
        assert_eq!(by_file, by_folder);
        assert!(!by_file.needs_password && !by_file.wrong_password);

        // A backup is read by its own path; the other files come from its folder.
        let backup = dir.0.join("AccountData.json.backup");
        std::fs::write(&backup, r#"[{"Username":"FromBackup","UserID":7}]"#).unwrap();
        let from_backup = scan_path(&backup, None, None);
        assert_eq!(from_backup.folder, dir.0);
        assert_eq!(from_backup.account_file, AccountFileKind::Plaintext);
        assert_eq!(from_backup.accounts.iter().map(|a| a.username.as_str()).collect::<Vec<_>>(), vec!["FromBackup"]);
        assert_eq!((&from_backup.games, &from_backup.destinations), (&by_folder.games, &by_folder.destinations));
        let mut store = Store::default();
        assert_eq!(import(&mut store, &from_backup).accounts_added, 1);

        // So is a copy under any other name.
        let renamed = dir.0.join("my accounts.txt");
        std::fs::write(&renamed, accounts_json()).unwrap();
        assert_eq!(scan_path(&renamed, None, None).accounts, by_folder.accounts);

        // A backup that is not there is reported under its own name.
        let missing = scan_path(&dir.0.join("AccountData.json.bak"), None, None);
        assert_eq!(missing.account_file, AccountFileKind::Missing);
        assert!(missing.notices.iter().any(|n| n.starts_with("AccountData.json.bak was not found")));
        assert_eq!(missing.games, by_folder.games);

        let nowhere = Path::new("Z:/definitely/not/a/real/folder-RAM");
        assert_eq!(scan_path(nowhere, None, None), scan(nowhere, None));
    }

    #[test]
    fn scan_path_opens_a_password_protected_file() {
        let dir = TempDir::new("locked");
        let file = dir.0.join("AccountData.json.backup");
        std::fs::write(&file, libsodium_file()).unwrap();
        std::fs::write(dir.0.join(RECENT_FILE), RECENT).unwrap();

        for password in [None, Some("")] {
            let locked = scan_path(&file, None, password);
            assert_eq!(locked.account_file, AccountFileKind::PasswordProtected);
            assert!(locked.needs_password && !locked.wrong_password);
            assert!(locked.accounts.is_empty());
            assert_eq!(locked.games.len(), 2, "the rest still imports");
            let notice = locked.notices.iter().find(|n| n.contains("password")).unwrap();
            assert!(notice.starts_with("AccountData.json.backup is password-protected"), "{notice}");
            assert!(notice.contains("enter the password"), "{notice}");
        }

        let wrong = scan_path(&file, None, Some(WRONG_PASSWORD));
        assert_eq!(wrong.account_file, AccountFileKind::PasswordProtected);
        assert!(wrong.wrong_password && !wrong.needs_password);
        assert!(wrong.accounts.is_empty());
        assert_eq!(wrong.games.len(), 2);
        assert!(wrong.notices.iter().any(|n| n.contains("that password did not open it")));

        let open = scan_path(&file, None, Some(VECTOR_PASSWORD));
        assert_eq!(open.account_file, AccountFileKind::PasswordProtected);
        assert!(!open.needs_password && !open.wrong_password);
        assert_eq!(open.accounts.len(), 1);
        let a = &open.accounts[0];
        assert_eq!((a.username.as_str(), a.user_id), ("demo_user", Some(42)));
        assert_eq!((a.alias.as_str(), a.description.as_str(), a.group.as_str()), ("a", "d", "G"));
        assert!(!open.notices.iter().any(|n| n.contains("password")), "{:?}", open.notices);
        let shown = format!("{open:?}");
        assert!(!shown.contains("FAKE") && !shown.contains("hunter2"), "{shown}");

        let mut store = Store::default();
        let record = import(&mut store, &open);
        assert_eq!(record.accounts_added, 1);
        assert_eq!((store.accounts[0].username.as_str(), store.accounts[0].roblox_user_id), ("demo_user", Some(42)));
        assert!(store.games.iter().any(|g| g.name == "Jailbreak"));
    }
}
