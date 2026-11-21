//! Reads the session cookies and saved passwords out of a legacy
//! `AccountData.json` so they can be moved into the vault. Kept apart from
//! the metadata importer, which deliberately never materialises secrets; this
//! one does, briefly, and only when the user asks for them to be imported.

use crate::import::{
    account_location, decrypt_password_file, detect_account_file, AccountFileKind, LegacyDecryptor, PasswordFileError,
    LEGACY_DPAPI_ENTROPY, MAX_FILE_BYTES,
};
use serde::{Deserialize, Deserializer};
use std::io::Read;
use std::path::Path;
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "Username", default, deserialize_with = "text")]
    username: Zeroizing<String>,
    #[serde(rename = "UserID", default, deserialize_with = "user_id")]
    user_id: u64,
    #[serde(rename = "SecurityToken", default, deserialize_with = "text")]
    security_token: Zeroizing<String>,
    #[serde(rename = "Password", default, deserialize_with = "text")]
    password: Zeroizing<String>,
}

/// A string, with `null` read as empty.
fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Zeroizing<String>, D::Error> {
    Ok(Zeroizing::new(Option::<String>::deserialize(deserializer)?.unwrap_or_default()))
}

/// The legacy ID is a signed 64-bit number; 0, negative and `null` all mean
/// "not known" and read as 0.
fn user_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    Ok(Option::<serde_json::Number>::deserialize(deserializer)?.and_then(|n| n.as_u64()).unwrap_or(0))
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

/// One legacy account's secrets: its session cookie and its saved password.
/// Either may be empty, never both.
pub struct LegacySecret {
    pub username: String,
    /// 0 when the file has none.
    pub user_id: u64,
    pub cookie: Zeroizing<String>,
    pub password: Zeroizing<String>,
}

impl std::fmt::Debug for LegacySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |secret: &str| if secret.is_empty() { "<empty>" } else { "<redacted>" };
        f.debug_struct("LegacySecret")
            .field("username", &self.username)
            .field("user_id", &self.user_id)
            .field("cookie", &format_args!("{}", shown(&self.cookie)))
            .field("password", &format_args!("{}", shown(&self.password)))
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CookieImportError {
    Missing,
    /// The file is password-protected and no password was given: ask for it
    /// and read again.
    PasswordProtected,
    /// The file is password-protected and the given password did not open it.
    WrongPassword,
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
                write!(f, "the account file is password-protected; enter its password to read it")
            }
            CookieImportError::WrongPassword => write!(f, "that password did not open the account file"),
            CookieImportError::Unrecognised => write!(f, "the account file format was not recognised"),
            CookieImportError::NeedsDecryptor => write!(f, "the account file is Windows-protected and cannot be read here"),
            CookieImportError::Decrypt(_) => write!(f, "the account file could not be decrypted on this PC"),
            CookieImportError::Parse(e) => write!(f, "the account file could not be parsed ({e})"),
            CookieImportError::Io(e) => write!(f, "the account file could not be read ({e})"),
        }
    }
}

/// Reads the legacy account file and returns the cookie and saved password
/// of every account that has a username and at least one of the two.
///
/// `path` is the legacy folder or an account file in it (`AccountData.json`,
/// one of its backups, or a copy under another name), as for
/// `import::scan_path`. `password` opens a password-protected file and is
/// ignored for the other variants; without one (`None` or empty) such a file
/// is `CookieImportError::PasswordProtected`, with one that does not open it
/// `CookieImportError::WrongPassword`. Every buffer that held the decrypted
/// file is zeroed before this returns.
pub fn read_secrets(
    path: &Path,
    decryptor: Option<&dyn LegacyDecryptor>,
    password: Option<&str>,
) -> Result<Vec<LegacySecret>, CookieImportError> {
    let (_, file) = account_location(path);
    let raw = read_capped(&file)?;
    let decrypted: Zeroizing<Vec<u8>>;
    let plain: &[u8] = match detect_account_file(&raw) {
        AccountFileKind::Plaintext => &raw,
        AccountFileKind::Dpapi => {
            let d = decryptor.ok_or(CookieImportError::NeedsDecryptor)?;
            decrypted = Zeroizing::new(d.unprotect(&raw, LEGACY_DPAPI_ENTROPY).map_err(CookieImportError::Decrypt)?);
            &decrypted
        }
        AccountFileKind::PasswordProtected => {
            let password = password.filter(|p| !p.is_empty()).ok_or(CookieImportError::PasswordProtected)?;
            decrypted = Zeroizing::new(decrypt_password_file(&raw, password).map_err(|e| match e {
                PasswordFileError::WrongPassword => CookieImportError::WrongPassword,
                other => CookieImportError::Parse(other.to_string()),
            })?);
            &decrypted
        }
        AccountFileKind::Missing => return Err(CookieImportError::Missing),
        AccountFileKind::Unrecognised => return Err(CookieImportError::Unrecognised),
    };
    let json = plain.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(plain);
    let rows: Vec<Row> =
        serde_json::from_slice(json).map_err(|e| CookieImportError::Parse(format!("line {}", e.line())))?;
    Ok(rows
        .into_iter()
        .filter(|r| !r.username.is_empty() && !(r.security_token.is_empty() && r.password.is_empty()))
        .map(|r| LegacySecret {
            username: r.username.to_string(),
            user_id: r.user_id,
            cookie: r.security_token,
            password: r.password,
        })
        .collect())
}

/// Reads `<folder>/AccountData.json` and returns the (username, cookie)
/// pairs that have a non-empty cookie. A password-protected file is
/// `CookieImportError::PasswordProtected`; use `read_secrets` to open one.
pub fn read_cookies(folder: &Path, decryptor: Option<&dyn LegacyDecryptor>) -> Result<Vec<LegacyCookie>, CookieImportError> {
    Ok(read_secrets(folder, decryptor, None)?
        .into_iter()
        .filter(|s| !s.cookie.is_empty())
        .map(|s| LegacyCookie { username: s.username, user_id: s.user_id, cookie: s.cookie })
        .collect())
}

/// The whole file, refused when it is larger than the importer's cap.
fn read_capped(path: &Path) -> Result<Zeroizing<Vec<u8>>, CookieImportError> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CookieImportError::Missing),
        Err(e) => return Err(CookieImportError::Io(e.to_string())),
    };
    let mut buf = Zeroizing::new(Vec::new());
    file.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut buf).map_err(|e| CookieImportError::Io(e.to_string()))?;
    if buf.len() > MAX_FILE_BYTES {
        return Err(CookieImportError::Io("larger than 64 MiB".to_string()));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::test_vectors::{libsodium_file, VECTOR_PASSWORD, WRONG_PASSWORD};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ram-cookies-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_plaintext_cookies_and_skips_empty() {
        let dir = temp_dir("plain");
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
        let dir = temp_dir("dpapi");
        let mut blob = vec![0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF, 0x01, 0x15, 0xD1, 0x11, 0x8C, 0x7A, 0x00, 0xC0, 0x4F, 0xC2, 0x97, 0xEB];
        blob.extend_from_slice(&[0u8; 64]);
        std::fs::write(dir.join("AccountData.json"), &blob).unwrap();
        assert_eq!(read_cookies(&dir, None).unwrap_err(), CookieImportError::NeedsDecryptor);
        assert_eq!(read_secrets(&dir, None, Some("unused")).unwrap_err(), CookieImportError::NeedsDecryptor);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secrets_keep_rows_with_a_cookie_or_a_password() {
        let dir = temp_dir("secrets");
        let file = dir.join("AccountData.json.backup");
        std::fs::write(
            &file,
            "\u{FEFF}[\
             {\"Username\":\"both\",\"UserID\":1,\"SecurityToken\":\"COOKIE-ONE\",\"Password\":\"pw-one\"},\
             {\"Username\":\"cookie_only\",\"UserID\":-5,\"SecurityToken\":\"COOKIE-TWO\",\"Password\":null},\
             {\"Username\":\"password_only\",\"UserID\":null,\"SecurityToken\":null,\"Password\":\"pw-three\"},\
             {\"Username\":\"neither\",\"UserID\":4,\"SecurityToken\":\"\",\"Password\":\"\"},\
             {\"Username\":\"\",\"SecurityToken\":\"COOKIE-FIVE\",\"Password\":\"pw-five\"},\
             {\"Username\":\"bare\"}]",
        )
        .unwrap();

        // The backup is read when it is the path given; the folder has no AccountData.json.
        assert_eq!(read_secrets(&dir, None, None).unwrap_err(), CookieImportError::Missing);
        let got = read_secrets(&file, None, None).unwrap();
        let rows: Vec<_> =
            got.iter().map(|s| (s.username.as_str(), s.user_id, s.cookie.as_str(), s.password.as_str())).collect();
        assert_eq!(
            rows,
            vec![
                ("both", 1, "COOKIE-ONE", "pw-one"),
                ("cookie_only", 0, "COOKIE-TWO", ""),
                ("password_only", 0, "", "pw-three"),
            ]
        );

        let shown = format!("{got:?}");
        assert!(shown.contains("both") && shown.contains("<redacted>") && shown.contains("<empty>"), "{shown}");
        for secret in ["COOKIE", "pw-one", "pw-three"] {
            assert!(!shown.contains(secret), "{shown}");
        }

        // The cookie-only view of the same file, by its path.
        let cookies = read_cookies(&file, None).unwrap();
        assert_eq!(cookies.iter().map(|c| c.username.as_str()).collect::<Vec<_>>(), vec!["both", "cookie_only"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_protected_file_needs_then_checks_the_password() {
        let dir = temp_dir("locked");
        std::fs::write(dir.join("AccountData.json"), libsodium_file()).unwrap();

        // No key derivation happens without a password.
        assert_eq!(read_cookies(&dir, None).unwrap_err(), CookieImportError::PasswordProtected);
        assert_eq!(read_secrets(&dir, None, None).unwrap_err(), CookieImportError::PasswordProtected);
        assert_eq!(read_secrets(&dir, None, Some("")).unwrap_err(), CookieImportError::PasswordProtected);

        assert_eq!(read_secrets(&dir, None, Some(WRONG_PASSWORD)).unwrap_err(), CookieImportError::WrongPassword);
        let got = read_secrets(&dir, None, Some(VECTOR_PASSWORD)).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].username.as_str(), got[0].user_id), ("demo_user", 42));
        assert!(got[0].cookie.starts_with("_|WARNING:-DO-NOT-SHARE-THIS."));
        assert!(got[0].cookie.ends_with("FAKEFAKE"));
        assert_eq!(got[0].password.as_str(), "hunter2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_password_file_is_a_parse_error() {
        let dir = temp_dir("damaged");
        let file = libsodium_file();
        std::fs::write(dir.join("AccountData.json"), &file[..64 + 16 + 24 + 15]).unwrap();
        assert!(matches!(read_secrets(&dir, None, Some("anything")), Err(CookieImportError::Parse(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
