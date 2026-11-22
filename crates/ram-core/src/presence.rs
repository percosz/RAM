//! Whether an account is online or in a game right now, from Roblox's
//! presence service.
//!
//! `POST https://presence.roblox.com/v1/presence/users` with
//! `{"userIds":[…]}` answers one entry per user: `userPresenceType`
//! (0 offline, 1 online, 2 in a game, 3 in Studio, 4 invisible), a
//! `lastLocation` text and, when the caller may see it, the place and
//! server. Asked with an account's own cookie, the answer about that account
//! is complete whatever its privacy settings, so RAM asks each account about
//! itself; without a cookie the same request is sent anonymously and Roblox
//! answers with what anyone may see.
//!
//! Roblox removed its "last online" endpoint (it answers 404 as of
//! 2026-11), so there is no way to ask when an account was last used. RAM
//! records that itself: every time presence shows an account online, that
//! moment is its last-online time.

use crate::platform::HttpApi;
use serde::{Deserialize, Serialize};

const URL: &str = "https://presence.roblox.com/v1/presence/users";
const TIMEOUT_MS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceKind {
    Offline,
    /// On the website or in the app, not in a game.
    Online,
    InGame,
    InStudio,
    /// Online but shown as offline to everyone else.
    Invisible,
}

impl PresenceKind {
    fn from_type(n: u64) -> PresenceKind {
        match n {
            1 => PresenceKind::Online,
            2 => PresenceKind::InGame,
            3 => PresenceKind::InStudio,
            4 => PresenceKind::Invisible,
            _ => PresenceKind::Offline,
        }
    }

    /// Someone is using the account right now.
    pub fn is_active(self) -> bool {
        self != PresenceKind::Offline
    }
}

/// One account's presence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub kind: PresenceKind,
    /// Roblox's description of where the user is (often the game's name);
    /// empty when not shared.
    pub location: String,
    pub place_id: Option<u64>,
    pub root_place_id: Option<u64>,
    pub universe_id: Option<u64>,
    /// The server (job) id, when shared.
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceError {
    /// The cookie was rejected.
    Unauthorized,
    RateLimited,
    Network(String),
    Service(u16),
    Malformed,
}

/// The JSON body asking about `user_ids`.
pub fn request_body(user_ids: &[u64]) -> String {
    serde_json::json!({ "userIds": user_ids }).to_string()
}

/// Reads the service's answer into (user id, presence) pairs. Entries
/// without a user id are skipped; unknown fields are ignored.
pub fn parse(body: &[u8]) -> Result<Vec<(u64, Presence)>, PresenceError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| PresenceError::Malformed)?;
    let list = v.get("userPresences").and_then(|l| l.as_array()).ok_or(PresenceError::Malformed)?;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        let Some(user) = p.get("userId").and_then(|u| u.as_u64()) else { continue };
        let num = |k: &str| p.get(k).and_then(|x| x.as_u64()).filter(|n| *n != 0);
        // The location text comes from Roblox and can carry a game's name:
        // keep it short and printable.
        let location: String = p
            .get("lastLocation")
            .and_then(|l| l.as_str())
            .unwrap_or("")
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        out.push((
            user,
            Presence {
                kind: PresenceKind::from_type(p.get("userPresenceType").and_then(|t| t.as_u64()).unwrap_or(0)),
                location,
                place_id: num("placeId"),
                root_place_id: num("rootPlaceId"),
                universe_id: num("universeId"),
                job_id: p.get("gameId").and_then(|g| g.as_str()).filter(|g| !g.is_empty()).map(str::to_string),
            },
        ));
    }
    Ok(out)
}

/// Asks Roblox about `user_ids`, as the owner of `cookie` when one is given.
/// A signed-in POST is answered 403 with a CSRF token first; it is repeated
/// once with the token.
pub fn fetch(http: &dyn HttpApi, cookie: Option<&str>, user_ids: &[u64]) -> Result<Vec<(u64, Presence)>, PresenceError> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let body = request_body(user_ids);
    let cookie_header = cookie.map(|c| format!(".ROBLOSECURITY={c}"));
    let send = |token: Option<&str>| {
        let mut headers: Vec<(&str, &str)> = vec![("Content-Type", "application/json"), ("Accept", "application/json")];
        if let Some(c) = &cookie_header {
            headers.push(("Cookie", c.as_str()));
            headers.push(("Referer", "https://www.roblox.com/"));
            headers.push(("Origin", "https://www.roblox.com"));
        }
        if let Some(t) = token {
            headers.push(("X-CSRF-TOKEN", t));
        }
        http.request("POST", URL, &headers, Some(body.as_bytes()), TIMEOUT_MS).map_err(|e| PresenceError::Network(format!("{e:?}")))
    };
    let mut resp = send(None)?;
    if resp.status == 403 {
        let Some(token) = resp.header("x-csrf-token").map(str::to_string) else {
            return Err(PresenceError::Unauthorized);
        };
        resp = send(Some(&token))?;
    }
    match resp.status {
        200 => parse(&resp.body),
        401 | 403 => Err(PresenceError::Unauthorized),
        429 => Err(PresenceError::RateLimited),
        status => Err(PresenceError::Service(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeHttp;

    #[test]
    fn parses_the_documented_answer() {
        // The first entry is what the service answered for a signed-out
        // request on 2026-11-22; the others follow the same shape.
        let body = br#"{"userPresences":[
            {"userPresenceType":0,"lastLocation":"Website","placeId":null,"rootPlaceId":null,"gameId":null,"universeId":null,"userId":1},
            {"userPresenceType":2,"lastLocation":"Rogue Lineage","placeId":5208655184,"rootPlaceId":3016661674,"gameId":"d9e92745-35ec-4fbe-8a2a-98e9fe15a8a3","universeId":1087859240,"userId":22},
            {"userPresenceType":1,"lastLocation":"Website","placeId":0,"userId":33,"somethingNew":true},
            {"userPresenceType":9,"userId":44},
            {"userPresenceType":2}
        ]}"#;
        let got = parse(body).unwrap();
        assert_eq!(got.len(), 4, "the entry without a user id is skipped");
        assert_eq!(got[0], (1, Presence { kind: PresenceKind::Offline, location: "Website".into(), place_id: None, root_place_id: None, universe_id: None, job_id: None }));
        let (id, p) = &got[1];
        assert_eq!(*id, 22);
        assert_eq!(p.kind, PresenceKind::InGame);
        assert_eq!(p.location, "Rogue Lineage");
        assert_eq!((p.place_id, p.root_place_id, p.universe_id), (Some(5208655184), Some(3016661674), Some(1087859240)));
        assert_eq!(p.job_id.as_deref(), Some("d9e92745-35ec-4fbe-8a2a-98e9fe15a8a3"));
        assert_eq!(got[2].1.kind, PresenceKind::Online);
        assert_eq!(got[2].1.place_id, None, "0 is not a place");
        assert_eq!(got[3].1.kind, PresenceKind::Offline, "unknown types count as offline");
        assert!(PresenceKind::InGame.is_active() && PresenceKind::Invisible.is_active() && !PresenceKind::Offline.is_active());
        assert_eq!(parse(b"{}"), Err(PresenceError::Malformed));
        assert_eq!(parse(b"nope"), Err(PresenceError::Malformed));
    }

    #[test]
    fn location_text_is_trimmed_and_bounded() {
        let long = "x".repeat(300);
        let body = format!(r#"{{"userPresences":[{{"userPresenceType":2,"lastLocation":"  A\tgame\n{long}","userId":5}}]}}"#);
        let got = parse(body.as_bytes()).unwrap();
        assert_eq!(got[0].1.location.chars().count(), 80);
        assert!(got[0].1.location.starts_with("Agame"));
    }

    #[test]
    fn signed_in_request_does_the_csrf_handshake() {
        let http = FakeHttp::default();
        http.push_full(403, &[("x-csrf-token", "tok")], "");
        http.push(200, r#"{"userPresences":[{"userPresenceType":2,"lastLocation":"A game","placeId":7,"userId":5}]}"#);
        let got = fetch(&http, Some("COOKIE"), &[5]).unwrap();
        assert_eq!(got[0].1.kind, PresenceKind::InGame);
        assert_eq!(http.sent_header(0, "cookie").as_deref(), Some(".ROBLOSECURITY=COOKIE"));
        assert_eq!(http.sent_header(0, "x-csrf-token"), None);
        assert_eq!(http.sent_header(1, "x-csrf-token").as_deref(), Some("tok"));
        assert_eq!(request_body(&[5, 6]), r#"{"userIds":[5,6]}"#);
    }

    #[test]
    fn anonymous_request_sends_no_cookie_and_errors_are_told_apart() {
        let http = FakeHttp::default();
        http.push(200, r#"{"userPresences":[{"userPresenceType":0,"userId":1}]}"#);
        assert_eq!(fetch(&http, None, &[1]).unwrap()[0].1.kind, PresenceKind::Offline);
        assert_eq!(http.sent_header(0, "cookie"), None);

        let http = FakeHttp::default();
        http.push(403, "");
        assert_eq!(fetch(&http, Some("C"), &[1]), Err(PresenceError::Unauthorized), "a 403 without a token is a rejection");
        let http = FakeHttp::default();
        http.push(401, "");
        assert_eq!(fetch(&http, Some("C"), &[1]), Err(PresenceError::Unauthorized));
        let http = FakeHttp::default();
        http.push(429, "");
        assert_eq!(fetch(&http, None, &[1]), Err(PresenceError::RateLimited));
        let http = FakeHttp::default();
        http.push(503, "");
        assert_eq!(fetch(&http, None, &[1]), Err(PresenceError::Service(503)));
        // Nothing to ask: no request at all.
        let http = FakeHttp::default();
        assert_eq!(fetch(&http, None, &[]), Ok(Vec::new()));
    }
}
