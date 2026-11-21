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
    headshots(http, &[user_id]).into_iter().next().map(|(_, url)| url)
}

/// Headshot URLs for up to 100 user ids in one call
/// (`GET thumbnails.roblox.com/v1/users/avatar-headshot`, no cookie). Only
/// entries Roblox reports as `Completed` are returned; `Pending` ones are
/// left out so the caller retries them later.
pub fn headshots(http: &dyn HttpApi, user_ids: &[u64]) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for chunk in user_ids.chunks(100) {
        let ids: Vec<String> = chunk.iter().map(|i| i.to_string()).collect();
        let url = format!(
            "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={}&size=150x150&format=Png&isCircular=false",
            ids.join(",")
        );
        let Ok(resp) = http.request("GET", &url, &[("Accept", "application/json")], None, TIMEOUT_MS) else { continue };
        if resp.status != 200 {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&resp.body) else { continue };
        for item in v.get("data").and_then(|d| d.as_array()).into_iter().flatten() {
            let completed = item.get("state").and_then(|s| s.as_str()) == Some("Completed");
            let id = item.get("targetId").and_then(|i| i.as_u64());
            let image = item.get("imageUrl").and_then(|u| u.as_str()).filter(|u| u.starts_with("https://"));
            if let (true, Some(id), Some(image)) = (completed, id, image) {
                out.push((id, image.to_string()));
            }
        }
    }
    out
}

/// What the public games API says about a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceInfo {
    pub place_id: u64,
    pub universe_id: u64,
    pub name: String,
    pub creator: String,
}

/// Resolves a place id to its experience name and creator, through
/// `apis.roblox.com/universes/v1/places/{id}/universe` and
/// `games.roblox.com/v1/games?universeIds=`. No cookie is needed.
/// `Err(Malformed)` means the id is not a place Roblox knows.
pub fn place_info(http: &dyn HttpApi, place_id: u64) -> Result<PlaceInfo, LaunchError> {
    let url = format!("https://apis.roblox.com/universes/v1/places/{place_id}/universe");
    let resp = http.request("GET", &url, &[("Accept", "application/json")], None, TIMEOUT_MS).map_err(map_net)?;
    let universe_id = match resp.status {
        200 => {
            let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|_| LaunchError::Malformed)?;
            v.get("universeId").and_then(|u| u.as_u64()).ok_or(LaunchError::Malformed)?
        }
        400 | 404 => return Err(LaunchError::Malformed),
        429 => return Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => return Err(LaunchError::Service { status }),
    };
    let url = format!("https://games.roblox.com/v1/games?universeIds={universe_id}");
    let resp = http.request("GET", &url, &[("Accept", "application/json")], None, TIMEOUT_MS).map_err(map_net)?;
    match resp.status {
        200 => {
            let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|_| LaunchError::Malformed)?;
            let g = v.get("data").and_then(|d| d.as_array()).and_then(|a| a.first()).ok_or(LaunchError::Malformed)?;
            let name = g.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                return Err(LaunchError::Malformed);
            }
            let creator = g.get("creator").and_then(|c| c.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string();
            Ok(PlaceInfo { place_id, universe_id, name, creator })
        }
        429 => Err(LaunchError::RateLimited { retry_after_secs: resp.retry_after }),
        status => Err(LaunchError::Service { status }),
    }
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

    #[test]
    fn headshots_keep_completed_entries_only() {
        let http = FakeHttp::default();
        http.push(
            200,
            r#"{"data":[{"targetId":1,"state":"Completed","imageUrl":"https://tr.rbxcdn.com/a.png"},{"targetId":2,"state":"Pending","imageUrl":""},{"targetId":3,"state":"Completed","imageUrl":"http://plain"}]}"#,
        );
        let got = headshots(&http, &[1, 2, 3]);
        assert_eq!(got, vec![(1, "https://tr.rbxcdn.com/a.png".to_string())]);
        assert!(http.requests.borrow()[0].1.contains("userIds=1,2,3"));
        assert!(http.sent_header(0, "cookie").is_none());
    }

    #[test]
    fn headshots_are_batched_by_hundred() {
        let http = FakeHttp::default();
        http.push(200, r#"{"data":[]}"#);
        http.push(200, r#"{"data":[]}"#);
        let ids: Vec<u64> = (1..=150).collect();
        assert!(headshots(&http, &ids).is_empty());
        assert_eq!(http.requests.borrow().len(), 2);
    }

    #[test]
    fn place_info_resolves_universe_then_game() {
        let http = FakeHttp::default();
        http.push(200, r#"{"universeId":1087859240}"#);
        http.push(200, r#"{"data":[{"id":1087859240,"name":"Rogue Lineage","creator":{"name":"Rogue Lineage Group"}}]}"#);
        let info = place_info(&http, 3016661674).unwrap();
        assert_eq!(info, PlaceInfo { place_id: 3016661674, universe_id: 1087859240, name: "Rogue Lineage".into(), creator: "Rogue Lineage Group".into() });
        assert!(http.requests.borrow()[1].1.ends_with("universeIds=1087859240"));
    }

    #[test]
    fn unknown_place_is_malformed() {
        let http = FakeHttp::default();
        http.push(400, r#"{"errors":[{"code":2,"message":"Place not found"}]}"#);
        assert_eq!(place_info(&http, 5), Err(LaunchError::Malformed));
        let http = FakeHttp::default();
        http.push(200, r#"{"universeId":9}"#);
        http.push(200, r#"{"data":[]}"#);
        assert_eq!(place_info(&http, 5), Err(LaunchError::Malformed));
    }
}
