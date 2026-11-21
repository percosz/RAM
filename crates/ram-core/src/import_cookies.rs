//! Reads the session cookies out of a legacy `AccountData.json` so they can
//! be moved into the vault. Kept apart from the metadata importer, which
//! deliberately never materialises secrets; this one does, briefly, and only
//! when the user asks for cookies to be imported.

use crate::import::{detect_account_file, AccountFileKind, LegacyDecryptor, LEGACY_DPAPI_ENTROPY};
use serde::Deserialize;
use std::path::Path;
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "Username", default)]
    username: String,
    #[serde(rename = "UserID", default)]
    user_id: u64,
    #[serde(rename = "SecurityToken", default)]
    security_token: String,
}

/// One legacy account's cookie, keyed by its username (and user id when
/// the file has one).
pub struct LegacyCookie {
    pub username: String,
    pub user_id: u64,
    pub cookie: Zeroizing<String>,
}

impl std::fmt::Debug for LegacyCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LegacyCookie({}, <redacted>)", self.username)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CookieImportError {
    Missing,
    PasswordProtected,
    Unrecognised,
    NeedsDecryptor,
    Decrypt(String),
    Parse(String),
    Io(String),
}

impl std::fmt::Display for CookieImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookieImportError::Missing => write!(f, "AccountData.json was not found"),
            CookieImportError::PasswordProtected => {
                write!(f, "the account file is password-protected; remove the password in the legacy app first")
            }
            CookieImportError::Unrecognised => write!(f, "the account file format was not recognised"),
            CookieImportError::NeedsDecryptor => write!(f, "the account file is Windows-protected and cannot be read here"),
            CookieImportError::Decrypt(_) => write!(f, "the account file could not be decrypted on this PC"),
            CookieImportError::Parse(e) => write!(f, "the account file could not be parsed ({e})"),
            CookieImportError::Io(e) => write!(f, "the account file could not be read ({e})"),
        }
    }
}

/// Reads `<folder>/AccountData.json` and returns the (username, cookie)
/// pairs that have a non-empty cookie. The decrypted buffer is zeroed after
/// parsing.
pub fn read_cookies(folder: &Path, decryptor: Option<&dyn LegacyDecryptor>) -> Result<Vec<LegacyCookie>, CookieImportError> {
    let path = folder.join("AccountData.json");
    let raw = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CookieImportError::Missing),
        Err(e) => return Err(CookieImportError::Io(e.to_string())),
    };
    let mut plain: Zeroizing<Vec<u8>> = match detect_account_file(&raw) {
        AccountFileKind::Plaintext => Zeroizing::new(raw),
        AccountFileKind::Dpapi => {
            let d = decryptor.ok_or(CookieImportError::NeedsDecryptor)?;
            Zeroizing::new(d.unprotect(&raw, LEGACY_DPAPI_ENTROPY).map_err(CookieImportError::Decrypt)?)
        }
        AccountFileKind::PasswordProtected => return Err(CookieImportError::PasswordProtected),
        AccountFileKind::Missing => return Err(CookieImportError::Missing),
        AccountFileKind::Unrecognised => return Err(CookieImportError::Unrecognised),
    };
    let text = plain.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&plain).to_vec();
    let rows: Vec<Row> = serde_json::from_slice(&text).map_err(|e| CookieImportError::Parse(format!("line {}", e.line())))?;
    plain.iter_mut().for_each(|b| *b = 0);
    Ok(rows
        .into_iter()
        .filter(|r| !r.username.is_empty() && !r.security_token.is_empty())
        .map(|r| LegacyCookie { username: r.username, user_id: r.user_id, cookie: Zeroizing::new(r.security_token) })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plaintext_cookies_and_skips_empty() {
        let dir = std::env::temp_dir().join(format!("ram-cookies-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("AccountData.json"),
            r#"[{"Username":"a","UserID":77,"SecurityToken":"_|WARNING:-X|_AAA"},{"Username":"b","SecurityToken":""},{"Username":"","SecurityToken":"zzz"}]"#,
        )
        .unwrap();
        let got = read_cookies(&dir, None).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].username, "a");
        assert_eq!(got[0].user_id, 77);
        assert_eq!(got[0].cookie.as_str(), "_|WARNING:-X|_AAA");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dpapi_without_decryptor_is_reported() {
        let dir = std::env::temp_dir().join(format!("ram-cookies-dpapi-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut blob = vec![0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF, 0x01, 0x15, 0xD1, 0x11, 0x8C, 0x7A, 0x00, 0xC0, 0x4F, 0xC2, 0x97, 0xEB];
        blob.extend_from_slice(&[0u8; 64]);
        std::fs::write(dir.join("AccountData.json"), &blob).unwrap();
        assert_eq!(read_cookies(&dir, None).unwrap_err(), CookieImportError::NeedsDecryptor);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
