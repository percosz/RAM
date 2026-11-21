//! Reading `.ROBLOSECURITY` values out of what people paste: a bare value,
//! `.ROBLOSECURITY=…`, a `user:pass:cookie` line from another manager, or
//! a line that only has a username and password (which RAM never uses).

pub const WARNING_PREFIX: &str = "_|WARNING:-DO-NOT-SHARE-THIS";

/// What one pasted line turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PastedLine {
    Cookie(String),
    /// `username:password` with no cookie: RAM does not sign in with
    /// passwords, so the user is told to use the browser instead.
    Password { username: String },
    Blank,
    Unrecognised,
}

/// Pulls a cookie value out of one line.
pub fn parse_line(line: &str) -> PastedLine {
    let s = line.trim().trim_end_matches(';').trim();
    if s.is_empty() {
        return PastedLine::Blank;
    }
    if let Some(i) = s.find(WARNING_PREFIX) {
        let v = s[i..].split(|c: char| c.is_whitespace() || c == ';' || c == '"' || c == '\'').next().unwrap_or("");
        return if v.len() > WARNING_PREFIX.len() + 20 { PastedLine::Cookie(v.to_string()) } else { PastedLine::Unrecognised };
    }
    if let Some(rest) = s.strip_prefix(".ROBLOSECURITY=") {
        let v = rest.trim().trim_matches('"');
        return if v.len() >= 32 { PastedLine::Cookie(v.to_string()) } else { PastedLine::Unrecognised };
    }
    // A long opaque token with no spaces is taken as a cookie value.
    if !s.contains(':') && !s.contains(char::is_whitespace) && s.len() >= 200 {
        return PastedLine::Cookie(s.to_string());
    }
    // username:password (two short parts) → not supported.
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 && !parts[0].is_empty() && parts[0].len() <= 20 && parts[0].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return PastedLine::Password { username: parts[0].to_string() };
    }
    PastedLine::Unrecognised
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> String {
        format!("{WARNING_PREFIX}.--Sharing-this-will-allow-someone-to-log-in-as-you-and-to-steal-your-ROBUX-and-items.|_{}", "A".repeat(60))
    }

    #[test]
    fn bare_and_prefixed_cookies_parse() {
        let c = fake();
        assert_eq!(parse_line(&c), PastedLine::Cookie(c.clone()));
        assert_eq!(parse_line(&format!("  .ROBLOSECURITY={c}; ")), PastedLine::Cookie(c.clone()));
        assert_eq!(parse_line(&format!("user:pass:{c}")), PastedLine::Cookie(c.clone()));
        assert_eq!(parse_line(&format!("\"{c}\"")), PastedLine::Cookie(c));
    }

    #[test]
    fn passwords_and_junk_are_recognised() {
        assert_eq!(parse_line("bob_1:hunter2"), PastedLine::Password { username: "bob_1".into() });
        assert_eq!(parse_line(""), PastedLine::Blank);
        assert_eq!(parse_line("   "), PastedLine::Blank);
        assert_eq!(parse_line("hello world"), PastedLine::Unrecognised);
        assert_eq!(parse_line("_|WARNING:-DO-NOT-SHARE-THIS"), PastedLine::Unrecognised);
        assert_eq!(parse_line(".ROBLOSECURITY=short"), PastedLine::Unrecognised);
    }
}
