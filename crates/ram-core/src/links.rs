//! Roblox link validation and launch URLs.
//!
//! Pasted text is accepted only when it is a bare place ID or a link on a
//! Roblox host in one of the known shapes. Launches are always plain
//! `https://www.roblox.com/...` URLs opened in the user's browser; the
//! manager never builds `roblox-player:` URIs or authentication tickets.

use crate::model::Target;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    /// The text is a URL, but not on a Roblox host.
    NotRoblox,
    /// A Roblox link in a shape the manager does not know how to launch.
    Unrecognised,
    BadPlaceId,
    /// The private server or share code contains unexpected characters.
    BadCode,
    /// A server (job) ID that is not a GUID.
    BadJobId,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkError::Empty => "Paste a Roblox link or a place ID.",
            LinkError::NotRoblox => "That link is not on roblox.com.",
            LinkError::Unrecognised => "That Roblox link is not a game, private server or share link.",
            LinkError::BadPlaceId => "The place ID must be a positive number.",
            LinkError::BadCode => "The link code contains characters Roblox does not use.",
            LinkError::BadJobId => "The server ID is not in Roblox's format.",
        })
    }
}

impl std::error::Error for LinkError {}

/// Parses pasted text into a launch target.
pub fn parse_destination(text: &str) -> Result<Target, LinkError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(LinkError::Empty);
    }
    if text.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(Target::Place { place_id: parse_place_id(text)? });
    }

    let (scheme, rest) = match text.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        // "www.roblox.com/games/1" without a scheme.
        None => ("https".to_string(), text),
    };
    let (location, query) = match rest.split_once('?') {
        Some((l, q)) => (l, q.split('#').next().unwrap_or("")),
        None => (rest.split('#').next().unwrap_or(""), ""),
    };
    let params = parse_query(query);
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };

    match scheme.as_str() {
        "roblox" => {
            // roblox://experiences/start?placeId=...  and the older
            // roblox://placeId=... form.
            let place = get("placeId").or_else(|| location.strip_prefix("placeId="));
            let Some(place) = place else { return Err(LinkError::Unrecognised) };
            target_from_parts(parse_place_id(place)?, get("linkCode"), get("gameInstanceId"))
        }
        "http" | "https" => {
            let (host, path) = match location.split_once('/') {
                Some((h, p)) => (h, p),
                None => (location, ""),
            };
            if !is_roblox_host(host) {
                return Err(LinkError::NotRoblox);
            }
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            // Optional locale prefix such as /en-us/ or /de/.
            let segments: &[&str] = match segments.first() {
                Some(first) if is_locale(first) => &segments[1..],
                _ => &segments[..],
            };
            match segments {
                ["share"] | ["share-links"] => {
                    let code = get("code").ok_or(LinkError::Unrecognised)?;
                    let link_type = get("type").unwrap_or("Server");
                    if !is_code(code) || !link_type.bytes().all(|b| b.is_ascii_alphabetic()) {
                        return Err(LinkError::BadCode);
                    }
                    Ok(Target::ShareLink { code: code.to_string(), link_type: link_type.to_string() })
                }
                ["games", "start"] => {
                    let place = get("placeId").ok_or(LinkError::Unrecognised)?;
                    target_from_parts(
                        parse_place_id(place)?,
                        get("linkCode").or(get("privateServerLinkCode")),
                        get("gameInstanceId").or(get("gameId")),
                    )
                }
                ["games", place, ..] => target_from_parts(
                    parse_place_id(place)?,
                    get("privateServerLinkCode"),
                    get("gameInstanceId").or(get("gameId")),
                ),
                _ => Err(LinkError::Unrecognised),
            }
        }
        _ => Err(LinkError::NotRoblox),
    }
}

fn target_from_parts(place_id: u64, link_code: Option<&str>, job_id: Option<&str>) -> Result<Target, LinkError> {
    if let Some(code) = link_code {
        if !is_code(code) {
            return Err(LinkError::BadCode);
        }
        return Ok(Target::PrivateServer { place_id, link_code: code.to_string() });
    }
    if let Some(job) = job_id {
        if !is_job_id(job) {
            return Err(LinkError::BadJobId);
        }
        return Ok(Target::LiveServer { place_id, job_id: job.to_ascii_lowercase() });
    }
    Ok(Target::Place { place_id })
}

fn parse_place_id(text: &str) -> Result<u64, LinkError> {
    match text.parse::<u64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(LinkError::BadPlaceId),
    }
}

fn is_roblox_host(host: &str) -> bool {
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    host == "roblox.com" || host == "www.roblox.com" || host == "web.roblox.com" || host == "ro.blox.com"
}

fn is_locale(segment: &str) -> bool {
    let b = segment.as_bytes();
    let alpha = |x: u8| x.is_ascii_alphabetic();
    (b.len() == 2 && alpha(b[0]) && alpha(b[1]))
        || (b.len() == 5 && alpha(b[0]) && alpha(b[1]) && b[2] == b'-' && alpha(b[3]) && alpha(b[4]))
}

/// Link and share codes are URL-safe tokens.
pub fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 128
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Server (job) IDs are GUIDs: 8-4-4-4-12 hexadecimal digits.
pub fn is_job_id(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() {
            if let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The browser URL that starts a launch for this target. Every component is
/// validated when the target is created, so no escaping is needed here.
pub fn launch_url(target: &Target) -> String {
    match target {
        Target::Place { place_id } => format!("https://www.roblox.com/games/start?placeId={place_id}"),
        Target::PrivateServer { place_id, link_code } => {
            format!("https://www.roblox.com/games/{place_id}?privateServerLinkCode={link_code}")
        }
        Target::ShareLink { code, link_type } => {
            format!("https://www.roblox.com/share?code={code}&type={link_type}")
        }
        Target::LiveServer { place_id, job_id } => {
            format!("https://www.roblox.com/games/start?placeId={place_id}&gameInstanceId={job_id}")
        }
    }
}

/// The experience's page, for when the user wants to look before joining.
pub fn game_page_url(place_id: u64) -> String {
    format!("https://www.roblox.com/games/{place_id}")
}

/// A description that is safe to show in lists and logs: private codes are
/// never included.
pub fn describe(target: &Target) -> String {
    match target {
        Target::Place { place_id } => format!("Place {place_id}"),
        Target::PrivateServer { place_id, .. } => format!("Private server, place {place_id}"),
        Target::ShareLink { link_type, .. } => format!("Share link ({link_type})"),
        Target::LiveServer { place_id, job_id } => {
            format!("Server {}… of place {place_id}", &job_id[..job_id.len().min(8)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOB: &str = "0f6ad7a2-3c1b-4c0e-9a57-0123456789ab";

    #[test]
    fn bare_place_id() {
        assert_eq!(parse_destination(" 3016661674 "), Ok(Target::Place { place_id: 3016661674 }));
        assert_eq!(parse_destination("0"), Err(LinkError::BadPlaceId));
        assert_eq!(parse_destination(""), Err(LinkError::Empty));
    }

    #[test]
    fn game_page_links() {
        for link in [
            "https://www.roblox.com/games/3016661674/Rogue-Lineage",
            "http://roblox.com/games/3016661674",
            "www.roblox.com/games/3016661674/Rogue-Lineage#!/about",
            "https://www.roblox.com/en-us/games/3016661674/x",
        ] {
            assert_eq!(parse_destination(link), Ok(Target::Place { place_id: 3016661674 }), "{link}");
        }
    }

    #[test]
    fn private_server_links() {
        let t = parse_destination("https://www.roblox.com/games/3016661674/Rogue-Lineage?privateServerLinkCode=AbC_123-xyz").unwrap();
        assert_eq!(t, Target::PrivateServer { place_id: 3016661674, link_code: "AbC_123-xyz".into() });
        assert!(t.is_private());
        assert_eq!(
            launch_url(&t),
            "https://www.roblox.com/games/3016661674?privateServerLinkCode=AbC_123-xyz"
        );
    }

    #[test]
    fn share_links() {
        let t = parse_destination("https://www.roblox.com/share?code=0123abcd&type=Server").unwrap();
        assert_eq!(t, Target::ShareLink { code: "0123abcd".into(), link_type: "Server".into() });
        assert_eq!(launch_url(&t), "https://www.roblox.com/share?code=0123abcd&type=Server");
    }

    #[test]
    fn live_server_links_and_deep_links() {
        let want = Target::LiveServer { place_id: 5, job_id: JOB.into() };
        assert_eq!(parse_destination(&format!("https://www.roblox.com/games/start?placeId=5&gameInstanceId={JOB}")), Ok(want.clone()));
        assert_eq!(parse_destination(&format!("roblox://experiences/start?placeId=5&gameInstanceId={}", JOB.to_uppercase())), Ok(want.clone()));
        assert_eq!(parse_destination("roblox://placeId=5"), Ok(Target::Place { place_id: 5 }));
        assert_eq!(want.durability(), crate::model::Durability::Live);
        assert!(launch_url(&want).starts_with("https://www.roblox.com/games/start?placeId=5&gameInstanceId="));
    }

    #[test]
    fn rejects_other_hosts_and_injection() {
        assert_eq!(parse_destination("https://roblox.com.evil.example/games/5"), Err(LinkError::NotRoblox));
        assert_eq!(parse_destination("https://evil.example/?x=roblox.com/games/5"), Err(LinkError::NotRoblox));
        assert_eq!(parse_destination("javascript://www.roblox.com/games/5"), Err(LinkError::NotRoblox));
        assert_eq!(parse_destination("https://www.roblox.com/users/1/profile"), Err(LinkError::Unrecognised));
        assert_eq!(
            parse_destination("https://www.roblox.com/games/5?privateServerLinkCode=a%22%20--flag"),
            Err(LinkError::BadCode)
        );
        assert_eq!(
            parse_destination("https://www.roblox.com/games/start?placeId=5&gameInstanceId=nope"),
            Err(LinkError::BadJobId)
        );
    }

    #[test]
    fn every_launch_url_is_https_roblox() {
        for t in [
            Target::Place { place_id: 1 },
            Target::PrivateServer { place_id: 1, link_code: "c".into() },
            Target::ShareLink { code: "c".into(), link_type: "Server".into() },
            Target::LiveServer { place_id: 1, job_id: JOB.into() },
        ] {
            let url = launch_url(&t);
            assert!(url.starts_with("https://www.roblox.com/"));
            assert_eq!(parse_destination(&url), Ok(t));
        }
    }

    #[test]
    fn descriptions_never_contain_private_codes() {
        let t = Target::PrivateServer { place_id: 1, link_code: "SECRETCODE".into() };
        assert!(!describe(&t).contains("SECRETCODE"));
        let s = Target::ShareLink { code: "SECRETCODE".into(), link_type: "Server".into() };
        assert!(!describe(&s).contains("SECRETCODE"));
    }
}
