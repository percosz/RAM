//! Credential Manager round trip and one live HTTPS request.

use ram_core::platform::{HttpApi, NetError, SecretStore};
use ram_win::{WinHttp, WinSecrets};

/// Deletes the test credential even when an assertion fails.
struct Cleanup<'a>(&'a WinSecrets, &'a str);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = self.0.delete(self.1);
    }
}

#[test]
fn secrets_round_trip_with_test_prefix() {
    let store = WinSecrets::with_prefix("WaypointTest/");
    let name = format!("round-trip-{}", std::process::id());
    let _cleanup = Cleanup(&store, &name);

    assert_eq!(store.get(&name), Ok(None));
    assert_eq!(store.delete(&name), Ok(()), "deleting a missing secret is fine");

    let value: Vec<u8> = (0u8..=255).collect();
    store.set(&name, &value).unwrap();
    assert_eq!(store.get(&name), Ok(Some(value)));

    store.set(&name, b"replaced").unwrap();
    assert_eq!(store.get(&name), Ok(Some(b"replaced".to_vec())));

    // A different prefix is a different namespace.
    let elsewhere = WinSecrets::with_prefix("WaypointTestOther/");
    assert_eq!(elsewhere.get(&name), Ok(None));

    assert!(store.set(&name, &vec![0u8; 2561]).is_err(), "over the blob limit");
    assert_eq!(store.get(&name), Ok(Some(b"replaced".to_vec())));
    assert!(store.set("", b"x").is_err());

    store.delete(&name).unwrap();
    assert_eq!(store.get(&name), Ok(None));
    assert_eq!(store.delete(&name), Ok(()));
}

#[test]
#[ignore = "needs internet access"]
fn https_get_returns_200() {
    let http = WinHttp::new();
    let response = http
        .request(
            "GET",
            "https://www.roblox.com/robots.txt",
            &[("Accept", "text/plain")],
            None,
            15_000,
        )
        .expect("request");
    println!(
        "status {}, {} bytes, retry_after {:?}",
        response.status,
        response.body.len(),
        response.retry_after
    );
    assert_eq!(response.status, 200);
    assert!(!response.body.is_empty());
    assert!(String::from_utf8_lossy(&response.body).to_ascii_lowercase().contains("user-agent"));
}

#[test]
#[ignore = "needs internet access"]
fn unresolvable_host_is_offline() {
    let http = WinHttp::new();
    let result = http.request("GET", "https://no-such-host.invalid/", &[], None, 5_000);
    assert_eq!(result, Err(NetError::Offline));
}
