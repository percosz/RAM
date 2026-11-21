//! Roblox lookups beyond the login check: username to id (for Follow), and
//! the Robux balance and Premium flag shown next to an account. All calls go
//! through the injected `HttpApi` and never store the cookie.

use crate::platform::HttpApi;
use crate::roblox::LaunchError;

const TIMEOUT_MS: u32 = 15_000;

fn cookie_header(cookie: &str) -> String {
    format!(".ROBLOSECURITY={cookie}")
}

fn map_net(e: crate::platform::NetError) -> LaunchError {
    match e {
        crate::platform::NetError::Offline => LaunchError::Offline,
        crate::platform::NetError::Timeout => LaunchError::Timeout,
        crate::platform::NetError::Other(_) => LaunchError::Service { status: 0 },
    }
}

/// Resolves a username to a user id with the public lookup endpoint
/// (`POST users.roblox.com/v1/usernames/users`). No cookie is needed.
pub fn resolve_username(http: &dyn HttpApi, username: &str) -> Result<u64, LaunchError> {
    let body = serde_json::json!({ "usernames": [username.trim()], "excludeBannedUsers": false }).to_string();
    let resp = http
        .request(
            "POST",
            "https://users.roblox.com/v1/usernames/users",
            &[("Content-Type", "application/json"), ("Accept", "application/json")],
            Some(body.as_bytes()),
            TIMEOUT_MS,
        )
        .map_err(map_net)?;
    match resp.status {
        200 => {
            let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|_| LaunchError::Malformed)?;
            v.get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|u| u.get("id"))
                .and_then(|i| i.as_u64())
                .ok_or(LaunchError::Malformed)
        }
        429 => Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
}

/// The signed-in account's Robux balance
/// (`GET economy.roblox.com/v1/users/{id}/currency`, cookie required).
pub fn robux(http: &dyn HttpApi, cookie: &str, user_id: u64) -> Result<i64, LaunchError> {
    let cookie = cookie_header(cookie);
    let url = format!("https://economy.roblox.com/v1/users/{user_id}/currency");
    let resp = http
        .request("GET", &url, &[("Cookie", cookie.as_str()), ("Accept", "application/json")], None, TIMEOUT_MS)
        .map_err(map_net)?;
    match resp.status {
        200 => {
            let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|_| LaunchError::Malformed)?;
            v.get("robux").and_then(|r| r.as_i64()).ok_or(LaunchError::Malformed)
        }
        401 => Err(LaunchError::Unauthorized),
        429 => Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
}

/// Whether the signed-in account has Premium
/// (`GET premiumfeatures.roblox.com/v1/users/{id}/validate-membership`).
pub fn premium(http: &dyn HttpApi, cookie: &str, user_id: u64) -> Result<bool, LaunchError> {
    let cookie = cookie_header(cookie);
    let url = format!("https://premiumfeatures.roblox.com/v1/users/{user_id}/validate-membership");
    let resp = http
        .request("GET", &url, &[("Cookie", cookie.as_str()), ("Accept", "application/json")], None, TIMEOUT_MS)
        .map_err(map_net)?;
    match resp.status {
        200 => {
            let text = String::from_utf8_lossy(&resp.body);
            Ok(text.trim() == "true")
        }
        401 => Err(LaunchError::Unauthorized),
        429 => Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
}

/// Headshot thumbnail URL for a user id (public thumbnails API).
pub fn headshot_url(http: &dyn HttpApi, user_id: u64) -> Option<String> {
    let url = format!(
        "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={user_id}&size=150x150&format=Png&isCircular=false"
    );
    let resp = http.request("GET", &url, &[("Accept", "application/json")], None, TIMEOUT_MS).ok()?;
    if resp.status != 200 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&resp.body).ok()?;
    v.get("data")?.as_array()?.first()?.get("imageUrl")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeHttp;

    #[test]
    fn resolves_username_to_id() {
        let http = FakeHttp::default();
        http.push(200, r#"{"data":[{"requestedUsername":"bob","id":42,"name":"bob"}]}"#);
        assert_eq!(resolve_username(&http, " bob ").unwrap(), 42);
        assert!(http.requests.borrow()[0].2.contains("\"bob\""));
    }

    #[test]
    fn unknown_username_is_malformed() {
        let http = FakeHttp::default();
        http.push(200, r#"{"data":[]}"#);
        assert_eq!(resolve_username(&http, "nobody"), Err(LaunchError::Malformed));
    }

    #[test]
    fn robux_and_premium_use_the_cookie() {
        let http = FakeHttp::default();
        http.push(200, r#"{"robux":1234}"#);
        http.push(200, "true");
        assert_eq!(robux(&http, "ck", 7).unwrap(), 1234);
        assert!(premium(&http, "ck", 7).unwrap());
        assert_eq!(http.sent_header(0, "cookie").as_deref(), Some(".ROBLOSECURITY=ck"));
        assert!(http.requests.borrow()[0].1.contains("/users/7/currency"));
    }

    #[test]
    fn expired_cookie_is_unauthorized() {
        let http = FakeHttp::default();
        http.push(401, "");
        assert_eq!(robux(&http, "ck", 7), Err(LaunchError::Unauthorized));
    }
}
