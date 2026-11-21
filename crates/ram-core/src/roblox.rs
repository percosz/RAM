//! Direct client launch via Roblox authentication tickets.
//!
//! Turns a signed-in account's `.ROBLOSECURITY` cookie into a running Roblox
//! client. The flow is: confirm the cookie and identity ([`whoami`]), exchange
//! the cookie for a one-time authentication ticket ([`request_ticket`]), then
//! build the `roblox-player:` launch URI ([`build_launch_uri`]).
//!
//! Everything goes through the injected [`HttpApi`], so the whole module is
//! unit-testable. The cookie is never stored or logged: it is a borrowed
//! parameter used only to build the `Cookie` request header.

#![allow(dead_code)]

use crate::platform::{HttpApi, NetError};
use crate::signin::CheckOutcome;

/// Why a launch step could not complete. These mirror the inconclusive and
/// conclusive signals Roblox gives, so they map cleanly onto
/// [`CheckOutcome`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    Offline,
    Timeout,
    RateLimited { retry_after_secs: Option<u64> },
    /// Roblox rejected the cookie (401, or an authenticated endpoint 403 that
    /// is not a CSRF challenge): the session is invalid.
    Unauthorized,
    /// Roblox returned an error that says nothing about the cookie.
    Service { status: u16 },
    /// The response was not in the documented shape.
    Malformed,
}

impl LaunchError {
    /// Maps this error onto the sign-in outcome the UI records.
    pub fn to_sign_in_outcome(&self) -> CheckOutcome {
        match self {
            LaunchError::Offline => CheckOutcome::Offline,
            LaunchError::Timeout => CheckOutcome::Timeout,
            LaunchError::RateLimited { retry_after_secs } => {
                CheckOutcome::RateLimited { retry_after_secs: *retry_after_secs }
            }
            LaunchError::Unauthorized => CheckOutcome::AuthorizationRejected,
            LaunchError::Service { status } => CheckOutcome::ServiceError { status: *status },
            LaunchError::Malformed => CheckOutcome::ServiceError { status: 0 },
        }
    }
}

/// Maps a transport error onto the offline/timeout launch errors. Anything
/// else is reported as an unreachable service.
fn net_error(e: NetError) -> LaunchError {
    match e {
        NetError::Offline => LaunchError::Offline,
        NetError::Timeout => LaunchError::Timeout,
        NetError::Other(_) => LaunchError::Service { status: 0 },
    }
}

/// The signed-in identity Roblox returns for a cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthedUser {
    pub id: u64,
    pub name: String,
    pub display_name: String,
}

const TIMEOUT_MS: u32 = 15_000;

/// Builds the `Cookie` header value for a `.ROBLOSECURITY` cookie. The cookie
/// is used only here and never stored or logged.
fn cookie_header(cookie: &str) -> String {
    format!(".ROBLOSECURITY={cookie}")
}

/// Confirms the cookie and returns the signed-in identity.
///
/// `GET https://users.roblox.com/v1/users/authenticated` with the cookie.
/// 200 parses `{"id","name","displayName"}`; 401 is [`LaunchError::Unauthorized`];
/// 429 is [`LaunchError::RateLimited`]; other non-success statuses are
/// [`LaunchError::Service`].
pub fn whoami(http: &dyn HttpApi, cookie: &str) -> Result<AuthedUser, LaunchError> {
    let cookie = cookie_header(cookie);
    let resp = http
        .request(
            "GET",
            "https://users.roblox.com/v1/users/authenticated",
            &[("Cookie", cookie.as_str())],
            None,
            TIMEOUT_MS,
        )
        .map_err(net_error)?;

    match resp.status {
        200 => parse_authed_user(&resp.body),
        401 => Err(LaunchError::Unauthorized),
        429 => Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
}

/// Parses the `authenticated` body into an [`AuthedUser`], or
/// [`LaunchError::Malformed`] when a field is missing or mistyped.
fn parse_authed_user(body: &[u8]) -> Result<AuthedUser, LaunchError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| LaunchError::Malformed)?;
    let id = v.get("id").and_then(|x| x.as_u64()).ok_or(LaunchError::Malformed)?;
    let name = v.get("name").and_then(|x| x.as_str()).ok_or(LaunchError::Malformed)?.to_string();
    let display_name = v
        .get("displayName")
        .and_then(|x| x.as_str())
        .ok_or(LaunchError::Malformed)?
        .to_string();
    Ok(AuthedUser { id, name, display_name })
}

/// A direct-launch target a ticket can serve. Private servers and share links
/// are deliberately absent: they need an access-code scrape, so the app
/// launches those through the browser instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    Place { place_id: u64 },
    Server { place_id: u64, job_id: String },
    Follow { user_id: u64 },
}

/// Maps a model [`Target`](crate::model::Target) onto a ticket-launchable
/// target, or `None` when it must go through the browser.
pub fn launch_target(target: &crate::model::Target) -> Option<LaunchTarget> {
    use crate::model::Target;
    match target {
        Target::Place { place_id } => Some(LaunchTarget::Place { place_id: *place_id }),
        Target::LiveServer { place_id, job_id } => {
            Some(LaunchTarget::Server { place_id: *place_id, job_id: job_id.clone() })
        }
        Target::PrivateServer { .. } | Target::ShareLink { .. } => None,
    }
}

/// Requests a one-time authentication ticket for the cookie.
///
/// Posts to `https://auth.roblox.com/v1/authentication-ticket/` with the
/// cookie and the fixed Roblox `Referer`/`Origin`. The first POST returns 403
/// with an `x-csrf-token` response header; the POST is repeated once with
/// `X-CSRF-TOKEN: <token>`, and a 200 carries the ticket in the
/// `rbx-authentication-ticket` response header. The retry happens at most
/// once.
///
/// 401, or a second 403 that still lacks a ticket, is
/// [`LaunchError::Unauthorized`]; 429 is [`LaunchError::RateLimited`]; a 200
/// with no ticket header is [`LaunchError::Malformed`].
pub fn request_ticket(http: &dyn HttpApi, cookie: &str) -> Result<String, LaunchError> {
    let cookie = cookie_header(cookie);
    const URL: &str = "https://auth.roblox.com/v1/authentication-ticket/";

    // First attempt without a CSRF token: Roblox answers 403 with the token.
    let first = http
        .request(
            "POST",
            URL,
            &[
                ("Cookie", cookie.as_str()),
                ("Referer", "https://www.roblox.com/"),
                ("Origin", "https://www.roblox.com"),
                ("Content-Type", "application/json"),
            ],
            Some(b"{}"),
            TIMEOUT_MS,
        )
        .map_err(net_error)?;

    match first.status {
        200 => return ticket_from(&first),
        401 => return Err(LaunchError::Unauthorized),
        429 => return Err(LaunchError::RateLimited { retry_after_secs: first.retry_after }),
        403 => {}
        status => return Err(LaunchError::Service { status }),
    }

    // A 403 is the CSRF challenge only when it carries a token; without one it
    // is a genuine rejection.
    let token = match first.header("x-csrf-token") {
        Some(t) => t.to_string(),
        None => return Err(LaunchError::Unauthorized),
    };

    // Exactly one retry, now carrying the CSRF token.
    let second = http
        .request(
            "POST",
            URL,
            &[
                ("Cookie", cookie.as_str()),
                ("Referer", "https://www.roblox.com/"),
                ("Origin", "https://www.roblox.com"),
                ("Content-Type", "application/json"),
                ("X-CSRF-TOKEN", token.as_str()),
            ],
            Some(b"{}"),
            TIMEOUT_MS,
        )
        .map_err(net_error)?;

    match second.status {
        200 => ticket_from(&second),
        401 => Err(LaunchError::Unauthorized),
        // A second 403 (even with a fresh token) is not retried: treat the
        // rejection as an invalid session rather than looping.
        403 => Err(LaunchError::Unauthorized),
        429 => Err(LaunchError::RateLimited { retry_after_secs: second.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
}

/// Extracts the authentication ticket from a 200 response, or
/// [`LaunchError::Malformed`] when the header is missing.
fn ticket_from(resp: &crate::platform::HttpResponse) -> Result<String, LaunchError> {
    resp.header("rbx-authentication-ticket")
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or(LaunchError::Malformed)
}

/// Builds the PlaceLauncher URL (PLU) for a target, before URL-encoding. See
/// `docs/LEGACY_AUDIT.md` section 2 (the launching section) for the shapes.
pub fn place_launcher_url(target: &LaunchTarget, browser_tracker_id: &str) -> String {
    const BASE: &str = "https://assetgame.roblox.com/game/PlaceLauncher.ashx";
    match target {
        LaunchTarget::Place { place_id } => format!(
            "{BASE}?request=RequestGame&browserTrackerId={browser_tracker_id}\
             &placeId={place_id}&isPlayTogetherGame=false"
        ),
        LaunchTarget::Server { place_id, job_id } => format!(
            "{BASE}?request=RequestGameJob&browserTrackerId={browser_tracker_id}\
             &placeId={place_id}&gameId={job_id}&isPlayTogetherGame=false"
        ),
        LaunchTarget::Follow { user_id } => format!(
            "{BASE}?request=RequestFollowUser&browserTrackerId={browser_tracker_id}\
             &userId={user_id}"
        ),
    }
}

/// Percent-encodes `s`, escaping everything except the unreserved set
/// `A-Za-z0-9-_.~`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Builds the full launch URI handed to the `roblox-player:` protocol handler.
/// The PlaceLauncher URL is percent-encoded before it is inserted. Shape per
/// `docs/LEGACY_AUDIT.md` section 2.
pub fn build_launch_uri(
    ticket: &str,
    target: &LaunchTarget,
    browser_tracker_id: &str,
    launch_time_ms: i64,
) -> String {
    let plu = place_launcher_url(target, browser_tracker_id);
    let plu_encoded = percent_encode(&plu);
    format!(
        "roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{launch_time_ms}\
         +placelauncherurl:{plu_encoded}+browsertrackerid:{browser_tracker_id}\
         +robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp"
    )
}

/// A fresh random 12-digit browser-tracker id (first digit non-zero), used to
/// tie a launch to a client by its command line later.
pub fn random_browser_tracker_id() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let mut s = String::with_capacity(12);
    s.push(char::from(b'0' + rng.random_range(1..10) as u8));
    for _ in 0..11 {
        s.push(char::from(b'0' + rng.random_range(0..10) as u8));
    }
    s
}

/// Confirms the cookie, requests a ticket and builds the launch URI for a
/// target. Returns `(launch_uri, browser_tracker_id)`, generating a tracker id
/// when none is supplied.
pub fn prepare_launch(
    http: &dyn HttpApi,
    cookie: &str,
    target: &LaunchTarget,
    browser_tracker_id: Option<&str>,
    launch_time_ms: i64,
) -> Result<(String, String), LaunchError> {
    whoami(http, cookie)?;
    let ticket = request_ticket(http, cookie)?;
    let btid = match browser_tracker_id {
        Some(b) => b.to_string(),
        None => random_browser_tracker_id(),
    };
    let uri = build_launch_uri(&ticket, target, &btid, launch_time_ms);
    Ok((uri, btid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeHttp;
    use crate::model::Target;
    use crate::platform::NetError;

    // --- whoami ---------------------------------------------------------

    #[test]
    fn whoami_parses_identity() {
        let http = FakeHttp::default();
        http.push(200, r#"{"id":123,"name":"neo","displayName":"Neo"}"#);
        let user = whoami(&http, "COOKIE").unwrap();
        assert_eq!(user, AuthedUser { id: 123, name: "neo".into(), display_name: "Neo".into() });
        // The cookie travels only in the Cookie header, in the documented form.
        assert_eq!(http.sent_header(0, "cookie").as_deref(), Some(".ROBLOSECURITY=COOKIE"));
        assert_eq!(http.requests.borrow()[0].1, "https://users.roblox.com/v1/users/authenticated");
    }

    #[test]
    fn whoami_401_is_unauthorized() {
        let http = FakeHttp::default();
        http.push(401, "");
        assert_eq!(whoami(&http, "COOKIE"), Err(LaunchError::Unauthorized));
    }

    #[test]
    fn whoami_timeout_is_timeout() {
        let http = FakeHttp::default();
        http.push_err(NetError::Timeout);
        assert_eq!(whoami(&http, "COOKIE"), Err(LaunchError::Timeout));
    }

    #[test]
    fn whoami_offline_is_offline() {
        let http = FakeHttp::default();
        http.push_err(NetError::Offline);
        assert_eq!(whoami(&http, "COOKIE"), Err(LaunchError::Offline));
    }

    #[test]
    fn whoami_429_is_rate_limited_with_retry_after() {
        let http = FakeHttp::default();
        http.responses.borrow_mut().push(Ok(crate::platform::HttpResponse {
            status: 429,
            retry_after: Some(42),
            headers: Vec::new(),
            body: Vec::new(),
        }));
        assert_eq!(
            whoami(&http, "COOKIE"),
            Err(LaunchError::RateLimited { retry_after_secs: Some(42) })
        );
    }

    #[test]
    fn whoami_malformed_body_is_malformed() {
        let http = FakeHttp::default();
        http.push(200, r#"{"id":"not a number","name":"x","displayName":"y"}"#);
        assert_eq!(whoami(&http, "COOKIE"), Err(LaunchError::Malformed));
    }

    #[test]
    fn whoami_other_status_is_service() {
        let http = FakeHttp::default();
        http.push(503, "");
        assert_eq!(whoami(&http, "COOKIE"), Err(LaunchError::Service { status: 503 }));
    }

    // --- request_ticket -------------------------------------------------

    #[test]
    fn request_ticket_does_csrf_handshake() {
        let http = FakeHttp::default();
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push_full(200, &[("rbx-authentication-ticket", "TICKET123")], "");

        let ticket = request_ticket(&http, "COOKIE").unwrap();
        assert_eq!(ticket, "TICKET123");

        // Exactly two requests, the second carrying the CSRF token.
        assert_eq!(http.requests.borrow().len(), 2);
        assert_eq!(http.sent_header(1, "x-csrf-token").as_deref(), Some("abc"));
        // The fixed Referer/Origin are present on the first request.
        assert_eq!(http.sent_header(0, "referer").as_deref(), Some("https://www.roblox.com/"));
        assert_eq!(http.sent_header(0, "origin").as_deref(), Some("https://www.roblox.com"));
        // First request must NOT carry a CSRF token.
        assert_eq!(http.sent_header(0, "x-csrf-token"), None);
    }

    #[test]
    fn request_ticket_200_without_header_is_malformed() {
        let http = FakeHttp::default();
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push(200, "");
        assert_eq!(request_ticket(&http, "COOKIE"), Err(LaunchError::Malformed));
    }

    #[test]
    fn request_ticket_403_then_401_is_unauthorized() {
        let http = FakeHttp::default();
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push(401, "");
        assert_eq!(request_ticket(&http, "COOKIE"), Err(LaunchError::Unauthorized));
        assert_eq!(http.requests.borrow().len(), 2);
    }

    #[test]
    fn request_ticket_403_twice_does_not_loop() {
        let http = FakeHttp::default();
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push_full(403, &[("x-csrf-token", "def")], "");
        // A third response would be consumed only by a buggy loop.
        http.push_full(200, &[("rbx-authentication-ticket", "SHOULD_NOT_REACH")], "");
        assert_eq!(request_ticket(&http, "COOKIE"), Err(LaunchError::Unauthorized));
        assert_eq!(http.requests.borrow().len(), 2);
    }

    #[test]
    fn request_ticket_403_without_token_is_unauthorized() {
        let http = FakeHttp::default();
        http.push(403, "");
        assert_eq!(request_ticket(&http, "COOKIE"), Err(LaunchError::Unauthorized));
        assert_eq!(http.requests.borrow().len(), 1);
    }

    #[test]
    fn request_ticket_401_up_front_is_unauthorized() {
        let http = FakeHttp::default();
        http.push(401, "");
        assert_eq!(request_ticket(&http, "COOKIE"), Err(LaunchError::Unauthorized));
        assert_eq!(http.requests.borrow().len(), 1);
    }

    #[test]
    fn request_ticket_429_is_rate_limited() {
        let http = FakeHttp::default();
        http.responses.borrow_mut().push(Ok(crate::platform::HttpResponse {
            status: 429,
            retry_after: Some(7),
            headers: Vec::new(),
            body: Vec::new(),
        }));
        assert_eq!(
            request_ticket(&http, "COOKIE"),
            Err(LaunchError::RateLimited { retry_after_secs: Some(7) })
        );
    }

    // --- launch_target --------------------------------------------------

    #[test]
    fn launch_target_maps_place_and_live_server() {
        assert_eq!(
            launch_target(&Target::Place { place_id: 42 }),
            Some(LaunchTarget::Place { place_id: 42 })
        );
        assert_eq!(
            launch_target(&Target::LiveServer { place_id: 42, job_id: "JOB".into() }),
            Some(LaunchTarget::Server { place_id: 42, job_id: "JOB".into() })
        );
    }

    #[test]
    fn launch_target_rejects_private_and_share() {
        assert_eq!(
            launch_target(&Target::PrivateServer { place_id: 1, link_code: "lc".into() }),
            None
        );
        assert_eq!(
            launch_target(&Target::ShareLink { code: "c".into(), link_type: "ExperienceDetails".into() }),
            None
        );
    }

    // --- URL builders ---------------------------------------------------

    #[test]
    fn place_launcher_url_place_is_exact() {
        let url = place_launcher_url(&LaunchTarget::Place { place_id: 55 }, "123456789012");
        assert_eq!(
            url,
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame\
             &browserTrackerId=123456789012&placeId=55&isPlayTogetherGame=false"
        );
    }

    #[test]
    fn place_launcher_url_server_is_exact() {
        let url = place_launcher_url(
            &LaunchTarget::Server { place_id: 55, job_id: "job-guid".into() },
            "123456789012",
        );
        assert_eq!(
            url,
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGameJob\
             &browserTrackerId=123456789012&placeId=55&gameId=job-guid&isPlayTogetherGame=false"
        );
    }

    #[test]
    fn place_launcher_url_follow_is_exact() {
        let url = place_launcher_url(&LaunchTarget::Follow { user_id: 99 }, "123456789012");
        assert_eq!(
            url,
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestFollowUser\
             &browserTrackerId=123456789012&userId=99"
        );
    }

    #[test]
    fn percent_encode_escapes_reserved_and_keeps_unreserved() {
        assert_eq!(percent_encode("https://a.com/x?y=1&z=2"), "https%3A%2F%2Fa.com%2Fx%3Fy%3D1%26z%3D2");
        assert_eq!(percent_encode("-_.~AZaz09"), "-_.~AZaz09");
    }

    #[test]
    fn build_launch_uri_place_is_exact() {
        let uri = build_launch_uri(
            "TICKET123",
            &LaunchTarget::Place { place_id: 55 },
            "123456789012",
            1700000000000,
        );
        assert_eq!(
            uri,
            "roblox-player:1+launchmode:play+gameinfo:TICKET123+launchtime:1700000000000\
             +placelauncherurl:https%3A%2F%2Fassetgame.roblox.com%2Fgame%2FPlaceLauncher.ashx\
             %3Frequest%3DRequestGame%26browserTrackerId%3D123456789012%26placeId%3D55\
             %26isPlayTogetherGame%3Dfalse+browsertrackerid:123456789012\
             +robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp"
        );
    }

    #[test]
    fn build_launch_uri_server_is_exact() {
        let uri = build_launch_uri(
            "TICKET123",
            &LaunchTarget::Server { place_id: 55, job_id: "job-guid".into() },
            "123456789012",
            1700000000000,
        );
        assert_eq!(
            uri,
            "roblox-player:1+launchmode:play+gameinfo:TICKET123+launchtime:1700000000000\
             +placelauncherurl:https%3A%2F%2Fassetgame.roblox.com%2Fgame%2FPlaceLauncher.ashx\
             %3Frequest%3DRequestGameJob%26browserTrackerId%3D123456789012%26placeId%3D55\
             %26gameId%3Djob-guid%26isPlayTogetherGame%3Dfalse+browsertrackerid:123456789012\
             +robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp"
        );
    }

    // --- random_browser_tracker_id --------------------------------------

    #[test]
    fn browser_tracker_id_is_twelve_digits_first_nonzero() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let id = random_browser_tracker_id();
            assert_eq!(id.len(), 12, "{id} is not 12 chars");
            assert!(id.chars().all(|c| c.is_ascii_digit()), "{id} has a non-digit");
            assert_ne!(id.as_bytes()[0], b'0', "{id} starts with zero");
            seen.insert(id);
        }
        // 200 draws of a 12-digit space must vary.
        assert!(seen.len() > 1);
    }

    // --- prepare_launch -------------------------------------------------

    #[test]
    fn prepare_launch_runs_whoami_then_ticket_then_builds() {
        let http = FakeHttp::default();
        http.push(200, r#"{"id":1,"name":"a","displayName":"A"}"#);
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push_full(200, &[("rbx-authentication-ticket", "TICK")], "");

        let (uri, btid) = prepare_launch(
            &http,
            "COOKIE",
            &LaunchTarget::Place { place_id: 7 },
            Some("111111111111"),
            1234,
        )
        .unwrap();
        assert_eq!(btid, "111111111111");
        assert!(uri.starts_with("roblox-player:1+launchmode:play+gameinfo:TICK+launchtime:1234+"));
        assert!(uri.contains("browsertrackerid:111111111111"));
    }

    #[test]
    fn prepare_launch_generates_btid_when_none() {
        let http = FakeHttp::default();
        http.push(200, r#"{"id":1,"name":"a","displayName":"A"}"#);
        http.push_full(403, &[("x-csrf-token", "abc")], "");
        http.push_full(200, &[("rbx-authentication-ticket", "TICK")], "");

        let (_uri, btid) =
            prepare_launch(&http, "COOKIE", &LaunchTarget::Place { place_id: 7 }, None, 1).unwrap();
        assert_eq!(btid.len(), 12);
    }

    #[test]
    fn prepare_launch_propagates_unauthorized() {
        let http = FakeHttp::default();
        http.push(401, "");
        assert_eq!(
            prepare_launch(&http, "COOKIE", &LaunchTarget::Place { place_id: 7 }, None, 1),
            Err(LaunchError::Unauthorized)
        );
    }

    // --- to_sign_in_outcome mapping -------------------------------------

    #[test]
    fn sign_in_outcome_mapping() {
        assert_eq!(LaunchError::Unauthorized.to_sign_in_outcome(), CheckOutcome::AuthorizationRejected);
        assert_eq!(LaunchError::Offline.to_sign_in_outcome(), CheckOutcome::Offline);
        assert_eq!(LaunchError::Timeout.to_sign_in_outcome(), CheckOutcome::Timeout);
        assert_eq!(
            LaunchError::RateLimited { retry_after_secs: Some(5) }.to_sign_in_outcome(),
            CheckOutcome::RateLimited { retry_after_secs: Some(5) }
        );
        assert_eq!(
            LaunchError::Service { status: 500 }.to_sign_in_outcome(),
            CheckOutcome::ServiceError { status: 500 }
        );
        assert_eq!(
            LaunchError::Malformed.to_sign_in_outcome(),
            CheckOutcome::ServiceError { status: 0 }
        );
    }
}
