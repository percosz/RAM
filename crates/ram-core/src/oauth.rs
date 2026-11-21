//! Verified sign-in checks through Roblox OAuth 2.0 (authorization code
//! with PKCE, public client).
//!
//! This is the only documented way for a desktop app to have Roblox confirm
//! an identity. The scopes are `openid profile`: they identify the user and
//! nothing else. They do not let the manager launch a client, read a browser
//! session or act as the user, so launches stay with the browser.
//!
//! The user registers their own OAuth application in the Roblox Creator
//! Dashboard and enters its client ID in Settings. The refresh token is the
//! only secret; it lives in Windows Credential Manager, never in the store.

use crate::model::AccountId;
use crate::platform::{HttpApi, HttpResponse, NetError, SecretStore};
use crate::signin::CheckOutcome;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::time::{Duration, Instant};

pub const AUTHORIZE_URL: &str = "https://apis.roblox.com/oauth/v1/authorize";
pub const TOKEN_URL: &str = "https://apis.roblox.com/oauth/v1/token";
pub const USERINFO_URL: &str = "https://apis.roblox.com/oauth/v1/userinfo";
pub const REVOKE_URL: &str = "https://apis.roblox.com/oauth/v1/token/revoke";
pub const SCOPES: &str = "openid profile";
const TIMEOUT_MS: u32 = 15_000;

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/callback")
}

pub fn secret_name(account: AccountId) -> String {
    format!("oauth-refresh-{account}")
}

/// URL-safe base64 without padding, as PKCE requires.
pub fn base64url(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

fn form_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One authorization attempt. `verifier` and `state` come from the system
/// random generator; `challenge` is base64url(SHA-256(verifier)).
#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub client_id: String,
    pub port: u16,
    pub state: String,
    pub verifier: String,
    pub challenge: String,
}

impl AuthRequest {
    /// The page to open in the account's browser profile.
    pub fn authorize_url(&self) -> String {
        format!(
            "{AUTHORIZE_URL}?client_id={}&redirect_uri={}&scope={}&response_type=code&state={}&code_challenge={}&code_challenge_method=S256",
            form_encode(&self.client_id),
            form_encode(&redirect_uri(self.port)),
            form_encode(SCOPES),
            form_encode(&self.state),
            form_encode(&self.challenge),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub user_id: u64,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    Net(NetError),
    RateLimited { retry_after: Option<u64> },
    /// Roblox explicitly refused the grant or token.
    Rejected,
    Service { status: u16 },
    /// The response was not in the documented shape.
    Malformed,
}

impl OAuthError {
    /// Maps a failure to a sign-in outcome. Only an explicit rejection says
    /// anything about the account.
    pub fn outcome(&self) -> CheckOutcome {
        match self {
            OAuthError::Net(NetError::Timeout) => CheckOutcome::Timeout,
            OAuthError::Net(NetError::Offline) => CheckOutcome::Offline,
            OAuthError::Net(NetError::Other(_)) => CheckOutcome::Offline,
            OAuthError::RateLimited { retry_after } => CheckOutcome::RateLimited { retry_after_secs: *retry_after },
            OAuthError::Rejected => CheckOutcome::AuthorizationRejected,
            OAuthError::Service { status } => CheckOutcome::ServiceError { status: *status },
            OAuthError::Malformed => CheckOutcome::ServiceError { status: 0 },
        }
    }
}

fn classify(resp: &HttpResponse, token_endpoint: bool) -> Result<serde_json::Value, OAuthError> {
    let json: Option<serde_json::Value> = serde_json::from_slice(&resp.body).ok();
    match resp.status {
        200 => json.ok_or(OAuthError::Malformed),
        429 => Err(OAuthError::RateLimited { retry_after: resp.retry_after }),
        400 if token_endpoint => {
            let error = json.as_ref().and_then(|j| j.get("error")).and_then(|e| e.as_str()).unwrap_or("");
            if error == "invalid_grant" {
                Err(OAuthError::Rejected)
            } else {
                Err(OAuthError::Service { status: 400 })
            }
        }
        401 if !token_endpoint => Err(OAuthError::Rejected),
        status => Err(OAuthError::Service { status }),
    }
}

fn token_request(http: &dyn HttpApi, body: String) -> Result<Tokens, OAuthError> {
    let resp = http
        .request(
            "POST",
            TOKEN_URL,
            &[("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")],
            Some(body.as_bytes()),
            TIMEOUT_MS,
        )
        .map_err(OAuthError::Net)?;
    let json = classify(&resp, true)?;
    let get = |k: &str| json.get(k).and_then(|v| v.as_str()).map(str::to_string);
    match (get("access_token"), get("refresh_token")) {
        (Some(access_token), Some(refresh_token)) => Ok(Tokens { access_token, refresh_token }),
        _ => Err(OAuthError::Malformed),
    }
}

pub fn exchange_code(http: &dyn HttpApi, req: &AuthRequest, code: &str) -> Result<Tokens, OAuthError> {
    token_request(
        http,
        format!(
            "grant_type=authorization_code&client_id={}&code={}&code_verifier={}&redirect_uri={}",
            form_encode(&req.client_id),
            form_encode(code),
            form_encode(&req.verifier),
            form_encode(&redirect_uri(req.port)),
        ),
    )
}

pub fn refresh(http: &dyn HttpApi, client_id: &str, refresh_token: &str) -> Result<Tokens, OAuthError> {
    token_request(
        http,
        format!(
            "grant_type=refresh_token&client_id={}&refresh_token={}",
            form_encode(client_id),
            form_encode(refresh_token)
        ),
    )
}

pub fn userinfo(http: &dyn HttpApi, access_token: &str) -> Result<UserInfo, OAuthError> {
    let bearer = format!("Bearer {access_token}");
    let resp = http
        .request("GET", USERINFO_URL, &[("Authorization", &bearer), ("Accept", "application/json")], None, TIMEOUT_MS)
        .map_err(OAuthError::Net)?;
    let json = classify(&resp, false)?;
    let user_id = json.get("sub").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).ok_or(OAuthError::Malformed)?;
    let username = json.get("preferred_username").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    Ok(UserInfo { user_id, username })
}

/// Best-effort revocation when the user removes an authorization.
pub fn revoke(http: &dyn HttpApi, client_id: &str, refresh_token: &str) {
    let body = format!("client_id={}&token={}", form_encode(client_id), form_encode(refresh_token));
    let _ = http.request(
        "POST",
        REVOKE_URL,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(body.as_bytes()),
        TIMEOUT_MS,
    );
}

/// Result of a verified check, with the identity Roblox reported when the
/// check succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub outcome: CheckOutcome,
    pub identity: Option<UserInfo>,
}

/// Runs a verified check for one account using its stored authorization.
///
/// Refresh tokens are single-use, so the replacement is stored before
/// anything else can fail.
pub fn check_account(
    http: &dyn HttpApi,
    secrets: &dyn SecretStore,
    client_id: &str,
    account: AccountId,
    expected_user_id: Option<u64>,
) -> CheckResult {
    let none = |outcome| CheckResult { outcome, identity: None };
    if client_id.trim().is_empty() {
        return none(CheckOutcome::NotConfigured);
    }
    let name = secret_name(account);
    let stored = match secrets.get(&name) {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => return none(CheckOutcome::NotAuthorized),
        Err(_) => return none(CheckOutcome::ServiceError { status: 0 }),
    };
    let tokens = match refresh(http, client_id, &stored) {
        Ok(t) => t,
        Err(OAuthError::Rejected) => {
            let _ = secrets.delete(&name);
            return none(CheckOutcome::AuthorizationRejected);
        }
        Err(e) => return none(e.outcome()),
    };
    let _ = secrets.set(&name, tokens.refresh_token.as_bytes());
    finish_check(http, &tokens, expected_user_id)
}

/// Completes a new authorization: exchanges the code, stores the refresh
/// token and confirms the identity.
pub fn complete_authorization(
    http: &dyn HttpApi,
    secrets: &dyn SecretStore,
    req: &AuthRequest,
    code: &str,
    account: AccountId,
    expected_user_id: Option<u64>,
) -> CheckResult {
    let tokens = match exchange_code(http, req, code) {
        Ok(t) => t,
        Err(e) => return CheckResult { outcome: e.outcome(), identity: None },
    };
    let result = finish_check(http, &tokens, expected_user_id);
    // Keep the authorization only when it belongs to this account.
    if result.outcome == CheckOutcome::Confirmed {
        let _ = secrets.set(&secret_name(account), tokens.refresh_token.as_bytes());
    }
    result
}

fn finish_check(http: &dyn HttpApi, tokens: &Tokens, expected_user_id: Option<u64>) -> CheckResult {
    match userinfo(http, &tokens.access_token) {
        Ok(info) => {
            let outcome = match expected_user_id {
                Some(expected) if expected != info.user_id => {
                    CheckOutcome::DifferentAccount { username: info.username.clone() }
                }
                _ => CheckOutcome::Confirmed,
            };
            CheckResult { outcome, identity: Some(info) }
        }
        Err(e) => CheckResult { outcome: e.outcome(), identity: None },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectError {
    /// The local port could not be opened.
    Bind(String),
    TimedOut,
    Cancelled,
    /// Roblox reported an error instead of a code, e.g. `access_denied`.
    Denied(String),
}

/// Waits on the loopback interface for Roblox's redirect and returns the
/// authorization code. Only requests carrying the expected `state` are
/// accepted. `cancelled` is polled so the UI can abort the wait.
pub fn wait_for_redirect(
    port: u16,
    expected_state: &str,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, RedirectError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| RedirectError::Bind(e.to_string()))?;
    listener.set_nonblocking(true).map_err(|e| RedirectError::Bind(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled() {
            return Err(RedirectError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(RedirectError::TimedOut);
        }
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let parsed = parse_redirect(&request, expected_state);
        let page = match &parsed {
            Some(Ok(_)) => "Authorization received. You can close this tab and return to RAM.",
            Some(Err(_)) => "Roblox did not grant the authorization. You can close this tab.",
            None => "This address only receives the Roblox authorization redirect.",
        };
        let body = format!("<!doctype html><meta charset=utf-8><title>RAM</title><p style=\"font:16px system-ui;margin:3em\">{page}</p>");
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        match parsed {
            Some(Ok(code)) => return Ok(code),
            Some(Err(reason)) => return Err(RedirectError::Denied(reason)),
            None => continue,
        }
    }
}

/// Extracts the code from the redirect request line. `None` means the
/// request is not the redirect (wrong path or state) and is ignored.
fn parse_redirect(request: &str, expected_state: &str) -> Option<Result<String, String>> {
    let line = request.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let (path, query) = parts.next()?.split_once('?')?;
    if path != "/callback" {
        return None;
    }
    let param = |name: &str| {
        query.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    };
    if param("state").as_deref() != Some(expected_state) {
        return None;
    }
    if let Some(code) = param("code") {
        if !code.is_empty() && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'%')) {
            return Some(Ok(code));
        }
    }
    Some(Err(param("error").unwrap_or_else(|| "no code returned".to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeHttp, FakeSecrets};

    const TOKENS: &str = r#"{"access_token":"at","refresh_token":"rt2","token_type":"Bearer","expires_in":900}"#;

    fn secrets_with_token() -> FakeSecrets {
        let s = FakeSecrets::default();
        s.set(&secret_name(7), b"rt1").unwrap();
        s
    }

    #[test]
    fn base64url_matches_rfc_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn authorize_url_uses_pkce_and_identity_scopes_only() {
        let req = AuthRequest { client_id: "123".into(), port: 53682, state: "st".into(), verifier: "v".into(), challenge: "ch".into() };
        let url = req.authorize_url();
        assert!(url.starts_with("https://apis.roblox.com/oauth/v1/authorize?client_id=123&"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53682%2Fcallback"));
        assert!(url.contains("scope=openid%20profile&"));
        assert!(url.contains("code_challenge=ch&code_challenge_method=S256"));
        assert!(!url.contains("client_secret"));
    }

    #[test]
    fn confirmed_check_rotates_the_refresh_token() {
        let (http, secrets) = (FakeHttp::default(), secrets_with_token());
        http.push(200, TOKENS);
        http.push(200, r#"{"sub":"42","preferred_username":"bob"}"#);
        let r = check_account(&http, &secrets, "cid", 7, Some(42));
        assert_eq!(r.outcome, CheckOutcome::Confirmed);
        assert_eq!(r.identity, Some(UserInfo { user_id: 42, username: "bob".into() }));
        assert_eq!(secrets.get(&secret_name(7)).unwrap().unwrap(), b"rt2");
        assert!(http.requests.borrow()[0].2.contains("refresh_token=rt1"));
    }

    #[test]
    fn network_failures_are_inconclusive_and_keep_the_token() {
        for (err, want) in [(NetError::Timeout, CheckOutcome::Timeout), (NetError::Offline, CheckOutcome::Offline)] {
            let (http, secrets) = (FakeHttp::default(), secrets_with_token());
            http.push_err(err);
            let r = check_account(&http, &secrets, "cid", 7, None);
            assert_eq!(r.outcome, want);
            assert!(r.outcome.is_inconclusive());
            assert_eq!(secrets.get(&secret_name(7)).unwrap().unwrap(), b"rt1");
        }
    }

    #[test]
    fn rate_limit_and_server_errors_are_inconclusive() {
        let (http, secrets) = (FakeHttp::default(), secrets_with_token());
        http.responses.borrow_mut().push(Ok(HttpResponse { status: 429, retry_after: Some(30), headers: Vec::new(), body: vec![] }));
        assert_eq!(check_account(&http, &secrets, "cid", 7, None).outcome, CheckOutcome::RateLimited { retry_after_secs: Some(30) });
        http.push(503, "busy");
        assert_eq!(check_account(&http, &secrets, "cid", 7, None).outcome, CheckOutcome::ServiceError { status: 503 });
        http.push(400, r#"{"error":"invalid_request"}"#);
        assert!(check_account(&http, &secrets, "cid", 7, None).outcome.is_inconclusive());
        assert!(secrets.get(&secret_name(7)).unwrap().is_some());
    }

    #[test]
    fn only_invalid_grant_signs_out_and_forgets_the_token() {
        let (http, secrets) = (FakeHttp::default(), secrets_with_token());
        http.push(400, r#"{"error":"invalid_grant"}"#);
        assert_eq!(check_account(&http, &secrets, "cid", 7, None).outcome, CheckOutcome::AuthorizationRejected);
        assert!(secrets.get(&secret_name(7)).unwrap().is_none());
    }

    #[test]
    fn unconfigured_and_unauthorized_are_distinct_and_inconclusive() {
        let (http, secrets) = (FakeHttp::default(), FakeSecrets::default());
        assert_eq!(check_account(&http, &secrets, "", 7, None).outcome, CheckOutcome::NotConfigured);
        assert_eq!(check_account(&http, &secrets, "cid", 7, None).outcome, CheckOutcome::NotAuthorized);
        assert!(http.requests.borrow().is_empty());
    }

    #[test]
    fn authorization_for_another_account_is_not_stored() {
        let (http, secrets) = (FakeHttp::default(), FakeSecrets::default());
        let req = AuthRequest { client_id: "cid".into(), port: 1, state: "s".into(), verifier: "v".into(), challenge: "c".into() };
        http.push(200, TOKENS);
        http.push(200, r#"{"sub":"99","preferred_username":"other"}"#);
        let r = complete_authorization(&http, &secrets, &req, "code", 7, Some(42));
        assert_eq!(r.outcome, CheckOutcome::DifferentAccount { username: "other".into() });
        assert!(secrets.get(&secret_name(7)).unwrap().is_none());

        http.push(200, TOKENS);
        http.push(200, r#"{"sub":"42","preferred_username":"bob"}"#);
        let r = complete_authorization(&http, &secrets, &req, "code", 7, Some(42));
        assert_eq!(r.outcome, CheckOutcome::Confirmed);
        assert_eq!(secrets.get(&secret_name(7)).unwrap().unwrap(), b"rt2");
        assert!(http.requests.borrow()[2].2.contains("code_verifier=v"));
    }

    #[test]
    fn redirect_parsing_requires_path_and_state() {
        assert_eq!(parse_redirect("GET /callback?code=abc&state=s1 HTTP/1.1\r\n", "s1"), Some(Ok("abc".into())));
        assert_eq!(parse_redirect("GET /callback?state=s1&code=abc HTTP/1.1\r\n", "s1"), Some(Ok("abc".into())));
        assert_eq!(parse_redirect("GET /callback?code=abc&state=evil HTTP/1.1\r\n", "s1"), None);
        assert_eq!(parse_redirect("GET /favicon.ico HTTP/1.1\r\n", "s1"), None);
        assert_eq!(parse_redirect("POST /callback?code=abc&state=s1 HTTP/1.1\r\n", "s1"), None);
        assert_eq!(
            parse_redirect("GET /callback?error=access_denied&state=s1 HTTP/1.1\r\n", "s1"),
            Some(Err("access_denied".into()))
        );
    }

    #[test]
    fn loopback_listener_returns_the_code_and_can_be_cancelled() {
        let port = 53999;
        let t = std::thread::spawn(move || wait_for_redirect(port, "st", Duration::from_secs(10), &|| false));
        let mut ok = false;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(50));
            if let Ok(mut s) = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
                s.write_all(b"GET /callback?code=the-code&state=st HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut resp = String::new();
                let _ = s.read_to_string(&mut resp);
                ok = resp.contains("200 OK");
                break;
            }
        }
        assert!(ok);
        assert_eq!(t.join().unwrap(), Ok("the-code".to_string()));
        assert_eq!(wait_for_redirect(port + 1, "st", Duration::from_secs(5), &|| true), Err(RedirectError::Cancelled));
    }
}
