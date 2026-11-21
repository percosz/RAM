//! Redaction for logs and diagnostics.
//!
//! Every line written to the log file or copied into a diagnostics report
//! passes through `redact`, so a credential or private server link cannot
//! leak even when an error message happens to contain one.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub const MASK: &str = "[redacted]";

/// Names whose values are always masked, in `name=value`, `name: value` and
/// `"name": "value"` forms. Matching is case-insensitive.
const SECRET_KEYS: &[&str] = &[
    "privateserverlinkcode",
    "linkcode",
    "accesscode",
    "access_token",
    "refresh_token",
    "id_token",
    "code_verifier",
    "code_challenge",
    "client_secret",
    "securitytoken",
    ".roblosecurity",
    "roblosecurity",
    "password",
    "authorization",
    "cookie",
    "state",
    "code",
];

/// Returns `text` with credentials and private link codes masked.
pub fn redact(text: &str) -> String {
    let mut out = mask_bearer(&mask_cookie_bodies(text));
    for key in SECRET_KEYS {
        out = mask_key(&out, key);
    }
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

fn mask_key(text: &str, key: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = lower[from..].find(key) {
        let start = from + found;
        let mut i = start + key.len();
        from = i;
        // The key must be a whole word: "code" must not match "barcode" or
        // "codes".
        if start > 0 && is_word(bytes[start - 1]) {
            continue;
        }
        if i < bytes.len() && is_word(bytes[i]) {
            continue;
        }
        while i < bytes.len() && matches!(bytes[i], b'"' | b'\'' | b' ') {
            i += 1;
        }
        if i >= bytes.len() || !matches!(bytes[i], b'=' | b':') {
            continue;
        }
        i += 1;
        while i < bytes.len() && matches!(bytes[i], b'"' | b'\'' | b' ') {
            i += 1;
        }
        let value_start = i;
        while i < bytes.len() && !matches!(bytes[i], b'&' | b'"' | b'\'' | b' ' | b',' | b';' | b'}' | b')' | b'\r' | b'\n' | b'\t' | b'#') {
            i += 1;
        }
        if i == value_start {
            continue;
        }
        out.push_str(&text[copied..value_start]);
        out.push_str(MASK);
        copied = i;
        from = i;
    }
    out.push_str(&text[copied..]);
    out
}

/// Masks Roblox cookie values wherever they appear, keyed or not.
fn mask_cookie_bodies(text: &str) -> String {
    const MARK: &str = "_|WARNING:-DO-NOT-SHARE-THIS";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(MARK) {
        out.push_str(&rest[..pos]);
        out.push_str(MASK);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ';' | ','))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn mask_bearer(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = lower[from..].find("bearer ") {
        let value_start = from + found + "bearer ".len();
        let tail = &text[value_start..];
        let len = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',')).unwrap_or(tail.len());
        from = value_start + len;
        if len == 0 || tail.starts_with(MASK) {
            continue;
        }
        out.push_str(&text[copied..value_start]);
        out.push_str(MASK);
        copied = from;
    }
    out.push_str(&text[copied..]);
    out
}

/// Severity of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    /// Parses the `[LEVEL]` tag of a stored line.
    pub fn of_line(line: &str) -> Option<Level> {
        let tag = line.split("][").nth(2)?.split(']').next()?;
        match tag {
            "DEBUG" => Some(Level::Debug),
            "INFO" => Some(Level::Info),
            "WARN" => Some(Level::Warn),
            "ERROR" => Some(Level::Error),
            _ => None,
        }
    }
}

/// `[yyyy-mm-dd][hh:mm:ss]` in UTC for a unix time.
pub fn stamp(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("[{y:04}-{m:02}-{d:02}][{:02}:{:02}:{:02}]", secs / 3600, (secs / 60) % 60, secs % 60)
}

/// Append-only log file with a size cap. Every line is redacted and looks
/// like `[2026-11-21][13:26:09][INFO] message`.
pub struct Log {
    path: PathBuf,
    max_bytes: u64,
}

impl Log {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Log { path: path.into(), max_bytes: 2 * 1024 * 1024 }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn unix_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Writes one line at Info. Kept for callers that pass their own clock.
    pub fn line(&self, now: i64, message: &str) {
        self.write(now, Level::Info, message);
    }

    pub fn debug(&self, message: &str) {
        self.write(Self::unix_now(), Level::Debug, message);
    }
    pub fn info(&self, message: &str) {
        self.write(Self::unix_now(), Level::Info, message);
    }
    pub fn warn(&self, message: &str) {
        self.write(Self::unix_now(), Level::Warn, message);
    }
    pub fn error(&self, message: &str) {
        self.write(Self::unix_now(), Level::Error, message);
    }

    /// Logging failures are ignored: the log must never stop the app.
    pub fn write(&self, now: i64, level: Level, message: &str) {
        if let Ok(meta) = fs::metadata(&self.path) {
            if meta.len() > self.max_bytes {
                let _ = fs::rename(&self.path, self.path.with_extension("old.log"));
            }
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}[{}] {}", stamp(now), level.tag(), redact(message).replace(['\r', '\n'], " "));
        }
    }

    /// The last `lines` lines of the log (the current file, preceded by the
    /// rotated one when the current file is short).
    pub fn tail(&self, lines: usize) -> Vec<String> {
        let mut text = fs::read_to_string(&self.path).unwrap_or_default();
        if text.lines().count() < lines {
            if let Ok(old) = fs::read_to_string(self.path.with_extension("old.log")) {
                text = old + &text;
            }
        }
        let all: Vec<&str> = text.lines().collect();
        let start = all.len().saturating_sub(lines);
        all[start..].iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_server_links_are_masked() {
        let s = redact("opening https://www.roblox.com/games/3016661674?privateServerLinkCode=SUPERSECRET123 now");
        assert!(!s.contains("SUPERSECRET123"));
        assert!(s.contains("privateServerLinkCode=[redacted] now"));
        let s = redact("https://www.roblox.com/share?code=abcdef0123&type=Server");
        assert_eq!(s, "https://www.roblox.com/share?code=[redacted]&type=Server");
        let s = redact("https://www.roblox.com/games/start?placeId=5&linkCode=LC99&accessCode=AC77");
        assert!(!s.contains("LC99") && !s.contains("AC77"));
        assert!(s.contains("placeId=5"));
    }

    #[test]
    fn oauth_material_is_masked() {
        let s = redact(r#"{"access_token":"eyJabc.def","refresh_token": "rt-1","token_type":"Bearer","expires_in":900}"#);
        assert!(!s.contains("eyJabc") && !s.contains("rt-1"));
        assert!(s.contains("expires_in"));
        let s = redact("GET /callback?code=AUTHCODE&state=STATE1 HTTP/1.1");
        assert!(!s.contains("AUTHCODE") && !s.contains("STATE1"));
        let s = redact("Authorization: Bearer abc.def.ghi");
        assert!(!s.contains("abc.def.ghi"));
        let s = redact("code_verifier=VERIFIER&client_id=123");
        assert!(!s.contains("VERIFIER") && s.contains("client_id=123"));
    }

    #[test]
    fn legacy_credentials_are_masked() {
        let s = redact(r#"{"Username":"bob","SecurityToken":"_|WARNING:-DO-NOT-SHARE-THIS.--Sharing-this|_ABCDEF","Password":"hunter2"}"#);
        assert!(!s.contains("ABCDEF") && !s.contains("hunter2"));
        assert!(s.contains("bob"));
        let s = redact("cookie jar had _|WARNING:-DO-NOT-SHARE-THIS.--x|_DEADBEEF; path=/");
        assert!(!s.contains("DEADBEEF"));
        let s = redact(".ROBLOSECURITY=XYZ123; other=1");
        assert!(!s.contains("XYZ123") && s.contains("other=1"));
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for s in [
            "Launched account 12 to place 3016661674",
            "error code 5: access denied",
            "status codes: 200, 429",
            "barcode=1234 stays",
            "the state of the client is Running",
        ] {
            assert_eq!(redact(s), s);
        }
    }

    #[test]
    fn log_lines_are_redacted_on_disk() {
        let dir = std::env::temp_dir().join(format!("ram-core-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log = Log::new(dir.join("RAM.log"));
        log.line(1, "open https://www.roblox.com/games/1?privateServerLinkCode=LEAKME\nsecond");
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains("LEAKME"));
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("[1970-01-01][00:00:01][INFO] "));
        log.warn("careful");
        let tail = log.tail(1);
        assert_eq!(tail.len(), 1);
        assert!(tail[0].ends_with("[WARN] careful"));
        assert_eq!(Level::of_line(&tail[0]), Some(Level::Warn));
        assert_eq!(Level::of_line("garbage"), None);
    }

    #[test]
    fn stamps_are_utc_calendar_dates() {
        assert_eq!(stamp(0), "[1970-01-01][00:00:00]");
        assert_eq!(stamp(951_782_400), "[2000-02-29][00:00:00]");
        assert_eq!(stamp(1_795_229_780), "[2026-11-21][02:56:20]");
    }
}
