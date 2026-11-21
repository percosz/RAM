# Reference code study: RoALT, alyae, niccs RAM v4

Read-only study of three Roblox account managers, done 2026-11-20 for the RAM Tauri 2 + React front-end
rewrite. Every path below is relative to the clone it names; excerpts are marked with their source
file and licence. Nothing in the clones was run, built or installed.

| Repo | Licence | Stack | Clone root (local) |
|---|---|---|---|
| RoALT (inspectsoftware) | MIT (code only; name, logo and "the look of the app" are excluded by `NOTICE.md`) | Rust + Tauri 2.12 + React 19 / TS, Tailwind 4 | `refs/RoALT` |
| alyae (vysaely) | GPL-3.0 (Mocktail patches Apache-2.0) | Rust workspace, egui/eframe GUI, helper binaries | `refs/alyae` |
| Roblox-Account-Manager v4 (niccdevs, "niccs") | GPL-3.0 | Rust + Tauri 2 + React 19 / TS, Tailwind 4 | `refs/Roblox-Account-Manager` |

Licence note on reuse: RoALT's MIT lets us copy code into a GPL-3.0 project with its copyright notice
and licence text kept; `NOTICE.md` explicitly forbids reusing the RoALT name, logo and look-and-feel
"as RoALT" (a re-branded app that borrows CSS tokens is fine; shipping it as "RoALT" is not). niccs is
GPL-3.0, same licence as RAM, so code can be copied with attribution. alyae is GPL-3.0 as well.

---

## A. Cookie / login validity check

### A1. RoALT (the one to copy)

**The request.** One typed client, `src-tauri/src/roblox_api/mod.rs`, owns every Roblox call. The
validity check is a single GET:

```
GET https://users.roblox.com/v1/users/authenticated
Cookie: .ROBLOSECURITY=<value>           (per request; no cookie jar)
Accept: application/json, text/plain, */*
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Safari/537.36
```

No CSRF header on a GET (only POST/PUT, or a GET that is being retried after a 403 that carried
`x-csrf-token`). No `Referer`/`Origin` for users.roblox.com (those two are added only for
auth.roblox.com, apis.roblox.com and gamejoin.roblox.com, see `target()` at mod.rs:935-946).

```rust
// refs/RoALT/src-tauri/src/roblox_api/endpoints.rs:3  (MIT)
pub const USERS_AUTHENTICATED: &str = "https://users.roblox.com/v1/users/authenticated";
```

```rust
// refs/RoALT/src-tauri/src/roblox_api/mod.rs:103-110  (MIT)
/// `GET users.roblox.com/v1/users/authenticated`, the only reliable cookie check.
/// `account_id` 0 = cookie not stored yet (no sink call). Returns the user and, if Roblox
/// rotated the cookie during the call, the new value.
pub async fn authenticated_user(&self, account_id: i64, cookie: &Secret) -> Result<(AuthenticatedUser, Option<Secret>)> {
    let url = endpoints::USERS_AUTHENTICATED;
    let (_, body, rotated) = self.request(Method::GET, url, url, None, account_id, Some(cookie)).await?;
    Ok((json(&body)?, rotated))
}
```

The answer is `{id, name, displayName}` (`roblox_api/types.rs:7-11`, `AuthenticatedUser`).

**How the cookie header is built** (`mod.rs:1040-1048`): `.ROBLOSECURITY={value}` as a
`HeaderValue` marked `set_sensitive(true)`, wrapped in `Zeroizing`; a value with non-header bytes
is `AppError::CookieInvalid` before any call. Cookie normalisation on input (`commands.rs:191-198`)
strips quotes, the `.ROBLOSECURITY=` prefix and a trailing `;`, and refuses empty, >8192 bytes or
non-ASCII-graphic values.

**The generic request loop** (`mod.rs:828-932`) is what gives the state mapping:

```rust
// refs/RoALT/src-tauri/src/roblox_api/mod.rs:888-910, 917, 930  (MIT, abridged)
if status.is_success() { return Ok((headers, bytes, rotated)); }
if status == StatusCode::TOO_MANY_REQUESTS {
    attempts += 1;
    let wait = limits.reset.or(limits.retry_after).unwrap_or(1 << (attempts - 1)).max(1);
    self.set_window(route, 0, wait);
    if attempts >= MAX_ATTEMPTS || wait > MAX_WAIT_SECS {   // 3 tries, 60 s
        return Err(AppError::RateLimited(wait));
    }
    continue;
}
if status == StatusCode::FORBIDDEN && !csrf_retried && !headers.contains_key(CHALLENGE)
    && let Some(token) = headers.get(CSRF).and_then(|v| v.to_str().ok())
{
    lock(&self.csrf).insert(account_id, token.to_owned());
    csrf_retried = true;
    continue;
}
let e = classify(status, &headers, &bytes);
return Err(e);
```

```rust
// refs/RoALT/src-tauri/src/roblox_api/mod.rs:1050-1058  (MIT)
/// Maps a final non-2xx answer. Never quotes the body: it may echo what was sent.
fn classify(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> AppError {
    match status {
        StatusCode::UNAUTHORIZED if error_code(body) == Some((9002, 2)) => AppError::BoundSessionRequired,
        StatusCode::UNAUTHORIZED => AppError::CookieInvalid,
        StatusCode::FORBIDDEN if headers.contains_key(CHALLENGE) => AppError::Challenge,   // "rblx-challenge-id"
        s => AppError::Roblox(format!("HTTP {}", s.as_u16())),
    }
}
```

Transport errors become `AppError::Network("timed out" | "could not connect" | "connection failed" (host))`
(`mod.rs:1069-1091`). Rate-limit headers `x-ratelimit-limit: N, N;w=60`, `x-ratelimit-remaining`,
`x-ratelimit-reset` and `retry-after` feed a per-route window (`RateLimit::parse`, `mod.rs:996-1014`;
`wait_turn`, `mod.rs:950-969`): a call waits up to 60 s for its window, else fails `RateLimited`.

**Cookie rotation**: any `Set-Cookie: .ROBLOSECURITY=` on any response (deletions excluded:
empty value, `Max-Age=0`, 1970 expiry, wrong prefix) is handed to a `RotationSink` and stored
(`rotated_cookie`, `mod.rs:1021-1038`). The validation call itself can therefore update the
stored cookie.

**Mapping to account status** (`src-tauri/src/model.rs:33-39`: `Unknown | Valid | Invalid | Challenged`):

```rust
// refs/RoALT/src-tauri/src/commands.rs:521-536  (MIT)
let stored = match s.api.authenticated_user(id, &cookie).await {
    Ok((u, _)) if u.id == user_id => {
        s.db.set_status(id, AccountStatus::Valid, now, Some(&u.name), Some(&u.display_name))
    }
    // The cookie signs in as someone else (e.g. an account switch in a browser).
    Ok(_) => s.db.set_status(id, AccountStatus::Invalid, now, None, None),
    Err(AppError::CookieInvalid | AppError::BoundSessionRequired) => {
        s.db.set_status(id, AccountStatus::Invalid, now, None, None)
    }
    Err(AppError::Challenge) => s.db.set_status(id, AccountStatus::Challenged, now, None, None),
    Err(e) => {               // network, rate limit, 5xx: status and status_checked_at unchanged
        log::warn!("could not validate account {id}: {e}");
        last_err = Some(e);
        continue;
    }
};
```

So the states are:

| Outcome | RoALT state | Note |
|---|---|---|
| 200 and `id == stored user_id` | `valid`, `status_checked_at = now`, name/displayName refreshed | |
| 200 but a different user id | `invalid` | cookie belongs to another account |
| 401 (any body) | `invalid` | "This cookie is not valid or has expired. Sign in again." |
| 401 with `{errors:[{code:9002, subcode:2}]}` | `invalid` (error kind `boundSessionRequired`) | Roblox's bound-session requirement |
| 403 with `rblx-challenge-id` header | `challenged` | captcha / 2SV; UI says sign in again via the login window |
| 403 with `x-csrf-token` (no challenge) | retried once with the token, then classified | |
| 429 | retry up to 3 times honouring `x-ratelimit-reset`/`retry-after`; then `RateLimited(secs)` and **the stored status is left alone** | UI toast counts it as "missed" |
| transport error / 5xx | `Network` / `Roblox("HTTP 5xx")`, status left alone | `AppError::temporary()` is true for these |

There is no "aged" state in RoALT; freshness is shown via `status_checked_at` ("checked 3 h ago").
A dead cookie found by any other call is recorded the same way by `record_cookie_error`
(`commands.rs:578-593`), and Discord is told only when the status *changes*.

**Add-account path** (`commands.rs:200-215`): same call with `account_id = 0`, the rotated cookie (if
any) replaces the pasted one before encryption, and the row is written `Valid` with `checked_at = now`.

**Cadence.**
- UI: the Accounts view validates every account once per app session on first show unless every
  account was checked less than 10 minutes ago (`src/features/accounts/AccountsView.tsx:195-201`);
  a "Check" button validates selected or all (`accounts_validate(ids?)`).
- Backend sign-in watch (`src-tauri/src/watch.rs:13-18`): 90 s after start, then every 6 h, only with
  the vault unlocked and no bulk launch running; skips accounts already `invalid` or checked within
  10 min; retry in 5 min when it could not run. Setting `signInWatch`.
- Presence: `accounts_presence(ids)` polled once a minute for the rows on screen (first 100 valid
  accounts), each account asking about itself with its own cookie (`src/data.ts:211-221`,
  `commands.rs:862-883`).
- Account info panel: `account_info(id, refresh)` cached `cacheMinutes` in SQLite; re-asked on
  open and once a minute while shown; presence inside it refreshed at most once a minute.

**Extra info fetched alongside** (`commands.rs:732-826`, `account_info`): nine calls in one
`tokio::join!`, each failing independently and named in `unavailable[]`:

| Field | Endpoint (`endpoints.rs`) | Cookie |
|---|---|---|
| profile (description, created, isBanned, hasVerifiedBadge) | `GET users.roblox.com/v1/users/{id}` | optional (higher limit with) |
| avatar headshot URL | `GET thumbnails.roblox.com/v1/users/avatar-headshot?userIds=..&size=150x150&format=Png&isCircular=false` (≤100 ids, Pending retried 5×) | no |
| robux | `GET economy.roblox.com/v1/user/currency` → `robux` | yes (this is the "probe": a dead cookie here is recorded on the account) |
| premium (legacy) | `GET premiumfeatures.roblox.com/v1/users/{id}/validate-membership` (bare bool) | yes |
| Roblox Plus | `POST apis.roblox.com/user-profile-api/v1/user/profiles/get-profiles {userIds, fields:["hasRobloxSubscription"]}` | no |
| friends / followers / following | `GET friends.roblox.com/v1/users/{id}/{friends,followers,followings}/count` | no |
| presence | `POST presence.roblox.com/v1/presence/users {userIds}` (≤50) | yes, as the viewer |
| email verified | `GET accountsettings.roblox.com/v1/email` → `verified` | yes |
| 2-step methods | `GET twostepverification.roblox.com/v1/users/{id}/configuration` | yes |

Headshots for the whole list: `accounts_headshots` (`commands.rs:830-856`), one batched call for the
ids not in the 6-hour cache.

### A2. niccs v4

`src-tauri/src/api/auth.rs:44-117`, `validate_cookie(security_token)`:

- Primary: `GET https://www.roblox.com/my/account/json` with `Cookie: .ROBLOSECURITY=…`
  (reqwest, redirects off, UA "Chrome/120"). Parses `UserId`, `Name`, `DisplayName`, `UserEmail`,
  `IsEmailVerified`, `AgeBracket`, `UserAbove13`.
- 5xx or 429 → sleep 400 ms·2^attempt, up to 3 tries. 401 → "Invalid cookie (status 401)" and stop.
  Other non-2xx → stop.
- Fallback when the primary did not give a definite 401: `GET users.roblox.com/v1/users/authenticated`
  (`auth.rs:119-190`), same retry rule, 401 → invalid.
- Result is a `String` error, not an enum; the Tauri command `validate_cookie(cookie)`
  (`commands/account_api.rs:124-127`) just forwards it. The front-end (`src/store.tsx:798-814`)
  calls it when adding a cookie and then `add_account`.
- Stored state is a single `Valid: bool` on the account (`data/accounts/model.rs:3-5`), set `true`
  by `Account::new` and by `persist_cookie_update` after a session refresh
  (`commands/account_api.rs:57-63`). It is never set `false` by a validation failure in the code I
  read; a 401 during any later call triggers `refresh_account_session`, which calls
  `log_out_other_sessions` and stores the new cookie, else returns "Roblox invalidated this session.
  Re-login required." (`account_api.rs:65-77, 3-19`). Auth-session detection is string matching
  (`is_auth_session_error`: "status 401", "unauthorized", "code 9002", …).
- "Aged" is a UI notion only: `getFreshnessColor(LastUse)` (`src/types.ts:130-139`) turns a red
  dot on after 20 days since `LastUse`, ramping to full red at 30 days; `AccountRow.tsx:77-84` shows
  dots for invalid (`#ef4444`), aged, launched-by-RAM (`#f59e0b`) and presence.
- CSRF for POSTs: `get_csrf_token` POSTs `auth.roblox.com/v1/authentication-ticket/` with
  `Referer: https://www.roblox.com/games/2753915549/Blox-Fruits` and `RBXAuthenticationNegotiation: 1`,
  reads `x-csrf-token` from the (403) answer (`auth.rs:192-224`); `send_with_csrf` retries once
  on a 403 that carries a new token.
- Presence: `get_presence(userIds, viewerUserId?)` (`account_api.rs:463-480`) uses the viewer's
  cookie, else any stored cookie, else anonymous; the list polls every `PresenceUpdateRate` minutes
  (min 30 s) in 100-id chunks (`store.tsx:2466-2499`). Headshots via `batched_get_avatar_headshots`.

### A3. alyae

`crates/alyae-roblox/src/api.rs:52-65`:

```rust
// refs/alyae/crates/alyae-roblox/src/api.rs:52-65  (GPL-3.0)
pub fn authenticated(cookie: &str) -> Result<(u64, String), String> {
    let mut r = agent()
        .get("https://users.roblox.com/v1/users/authenticated")
        .header("Cookie", &format!(".ROBLOSECURITY={cookie}"))
        .header("Accept", "application/json")
        .call()
        .map_err(net)?;
    if r.status() != 200 {
        return Err(answer(r.status().as_u16(), "sign-in check"));
    }
    let v: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
    let id = v["id"].as_u64().ok_or("Roblox sent no user ID")?;
    Ok((id, v["name"].as_str().unwrap_or("").to_string()))
}
```

`answer()` (`api.rs:43-49`) maps 401 → "the cookie has expired: sign in again and paste the new one",
429 → "Roblox is rate limiting: try again in a minute", other → "{what}: Roblox answered {s}".
ureq (blocking), 10 s timeout, Chrome-158-on-Windows UA plus `sec-ch-ua*` client hints and
`Accept-Language` (`browser.rs`). The check runs on demand only: the module's `check` action
("Refresh" button, `modules/roblox-windows/module.toml:115-117`; helper `check()` in
`alyae-roblox/src/main.rs:374-386`) fills `username` and `user_id`, and after a login/browser window
closes. There is no stored validity state; a failure is a message-bar line. Signed POSTs go
through `call()` (`api.rs:258-325`): CSRF from the 403's `x-csrf-token`, one wait on 429 using
`retry-after` (default 15 s, max 60), and it answers a `rblx-challenge-type: twostepverification`
403 itself from the saved TOTP secret. Presence: `[watch]` every 15 s, anonymous batch of 100 ids,
then each account asks about itself with its own cookie once a minute if it looks offline publicly
(`module.toml:143-154`, `main.rs:395-440`).

### Consensus

All three agree `GET users.roblox.com/v1/users/authenticated` with `Cookie: .ROBLOSECURITY=…` is the
check; 200 = valid (RoALT additionally requires the returned `id` to equal the stored user id), 401 =
dead, 429 = back off and keep the old state. Only RoALT distinguishes the challenge (403 +
`rblx-challenge-id`) and the bound-session 401 (code 9002/2), handles cookie rotation on the
check itself, and keeps a per-route throttle from the `x-ratelimit-*` headers. niccs tries
`www.roblox.com/my/account/json` first (richer payload) and falls back to the same endpoint.

---

## B. "Open Browser" / per-account browser session

### B1. alyae

The "Open Browser" button (`crates/alyae/src/ui.rs:1575-1583`) runs the module action `browser`
(`modules/roblox-windows/module.toml:127-130`, "roblox.com signed in as the account, in a window of
its own (the saved cookie, a fresh profile)"). The helper `alyae-roblox browser`
(`crates/alyae-roblox/src/main.rs:296-311`) spawns a second helper binary, **`alyae-roblox-login`**,
feeding it one JSON line on stdin `{"username", "cookie", "user_agent"}` (`web_window()`,
`main.rs:313-371`).

`crates/alyae-roblox-login/src/main.rs` is a tao + **wry** window (WebView2 on Windows, WebKitGTK 4.1
on Linux; no external browser is launched and none needs to be installed beyond the WebView2
runtime / webkit2gtk). The cookie goes in through wry's cookie API before the first navigation:

```rust
// refs/alyae/crates/alyae-roblox-login/src/main.rs:221-255  (GPL-3.0, abridged)
/// The `.ROBLOSECURITY` cookie for every roblox.com host (www, apis, users...). wry passes
/// `Cookie::domain()` to WebKitGTK and WebView2, and the cookie crate strips one leading dot from
/// it: `.roblox.com` arrived as `roblox.com`, a cookie for that host alone, so www.roblox.com
/// opened signed out. Given `..roblox.com`, `domain()` is `.roblox.com`, a cookie for all of them.
fn session_cookie_for(value: String) -> wry::cookie::Cookie<'static> {
    wry::cookie::Cookie::build((".ROBLOSECURITY", value))
        .domain("..roblox.com")
        .path("/")
        .secure(true)
        .http_only(true)
        .build()
}

fn browse(username: &str, cookie: String, ua: &str) {
    ...
    let mut profile = Profile::new();          // temp dir "alyae-login-<pid>", removed on close
    let webview = web_view(&window, &mut profile, ua, |b| {
        b.with_url("about:blank").with_navigation_handler(|url| may_open(&url))
    });
    let c = session_cookie_for(cookie.clone());
    if let Err(e) = webview.set_cookie(&c) { ... return; }
    let _ = webview.load_url(&format!("{ORIGIN}/home"));
```

Details:
- Profile: Windows/macOS get `wry::WebContext::new(Some(temp_dir/alyae-login-<pid>))`, the webview is
  built `with_incognito(true)`, and the folder is deleted when the window closes (profiles older than
  a day are swept). On Linux no profile at all (incognito WebKitGTK). So it is a **fresh profile every
  time**, not a kept per-account profile.
- The window is navigation-locked to roblox.com / rbxcdn / arkoselabs / funcaptcha / hcaptcha
  (`may_open`, `main.rs:126-143`).
- On close it reads the webview's `.ROBLOSECURITY` back (`cookies_for_url`, `main.rs:146-154`); a
  value that differs from the one it was given is printed as `{"cookie": …}` and alyae re-checks it
  with `authenticated()` and saves it (`alyae-roblox/src/main.rs:305-310`).
- The User-Agent is the same Chrome-on-Windows string the API calls use (`browser.rs`), so Roblox
  sees one "browser" for both.
- Windows vs Linux: only the profile handling differs (see above) and the GTK build path
  (`build_gtk`). Packaging notes: `packaging/PORTABLE.txt:13` (needs WebView2 runtime),
  `packaging/README-linux.txt:3` (needs webkit2gtk-4.1 "used by the Roblox Log in and Open Browser
  windows").

### B2. RoALT and niccs

**RoALT** has it, embedded: "Browse" is a child webview inside the main window
(`src-tauri/src/browse.rs`), one account at a time, on a **kept per-account WebView2 profile**
`<data_dir>/webview-sessions/<account_id>` (`webview.rs:179-181`). The webview is created blank with
`.data_directory(profile)`, `.incognito(false)`, `.general_autofill_enabled(false)`, navigation
locked to roblox.com (never Create), new windows denied, downloads refused (`browse.rs:368-405`),
then `Window::add_child` (needs Tauri's `unstable` feature, `Cargo.toml:14-15`). The cookie is put in
with **WebView2's own cookie manager via COM**, not Tauri's API, because Tauri's `set_cookie` drops
the leading dot of the domain:

```rust
// refs/RoALT/src-tauri/src/webview.rs:328-339  (MIT)
// A crash can leave the last sign-in in a kept profile: never start with one.
delete_auth_cookie(&wv)?;
if let Some(cookie) = cookie {
    // WebView2's own manager: `.roblox.com` keeps its dot (Tauri's `set_cookie` drops it).
    let value = wide(cookie.expose());
    let mgr = wv.cast::<ICoreWebView2_2>()?.CookieManager()?;
    let c = mgr.CreateCookie(w!(".ROBLOSECURITY"), PCWSTR(value.as_ptr()), w!(".roblox.com"), w!("/"))?;
    c.SetIsHttpOnly(true)?;
    c.SetIsSecure(true)?;
    c.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX)?;
    mgr.AddOrUpdateCookie(&c)?;
}
```

Around it (`webview.rs:303-349`, `harden_webview`): context menus, browser accelerator keys and
password autosave off via `ICoreWebView2Settings4`, a `NavigationStarting` guard, an optional
`AddScriptToExecuteOnDocumentCreated` script, and the browser process id (used to wait for the
profile folder to unlock, `wait_for_exit`). `.ROBLOSECURITY` is deleted from the profile on open and
again on close (`delete_auth_cookie`: both the domain cookie and a host-only one), Google/Apple
sign-in cookies are purged, and a profile whose window never closed normally carries an
`.roalt-open` marker file and is deleted at the next start (`sweep_dirty_profiles`). The cookie
Browse ends with is kept if it still signs in as the same user (`watch_cookie`, `browse.rs` header
comment). The same `harden` serves the sign-in window (`capture_login`, `webview.rs:88-135`), which
polls `window.cookies()` (Tauri API) every 700 ms for a non-empty `.ROBLOSECURITY` from
`*.roblox.com` (`pick_roblosecurity`, `webview.rs:536-541`, noting "Tauri strips the leading dot").
Browse also intercepts `roblox-player:` links (Play buttons) with `LaunchingExternalUriScheme` so
the launch goes through RoALT (`browse.rs:431-473`), and can block purchases (`spend_of`).

**niccs v4** has it, external: `open_account_browser(userId)` (`src-tauri/src/chromium/commands.rs:57-102`)
launches a **Chrome for Testing** build it downloads itself (`chromium/download.rs:10`,
`googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json`; falls
back to installed Chrome / Chromium / Edge, `download.rs:97-103`) with a per-account
`--user-data-dir` (`ChromiumManager::account_profile(app, user_id)`), `--remote-debugging-port=0`,
reads `DevToolsActivePort` from the profile, connects to the first `page` target over WebSocket and
uses **CDP**:

```rust
// refs/Roblox-Account-Manager/src-tauri/src/chromium/cdp.rs:181-196  (GPL-3.0)
pub async fn set_roblosecurity(&mut self, value: &str, domain: &str) -> Result<(), String> {
    self.send("Network.setCookie", json!({
        "name": ".ROBLOSECURITY", "value": value, "domain": domain, "path": "/",
        "secure": true, "httpOnly": true, "sameSite": "None",
    })).await?;
    Ok(())
}
```

called for both `.roblox.com` and `www.roblox.com`, then `Page.navigate` to `/home`
(`commands.rs:89-96`). Login uses the same mechanism in reverse (`Network.deleteCookies` first,
`Network.getAllCookies` polled every 500 ms for up to 4 min, `commands.rs:131-186`), optional
persistent login profile (`[Login] PersistentProfile`), and `import_userpass` fills the form via
`Runtime.evaluate` (`cdp.rs:232-251`). Cost: a ~150 MB browser download on first use, progress
events `chromium-download-progress`, and a `ChromiumManager` to track child processes.

### B3. Recommendation for RAM (Tauri 2, Windows)

Use RoALT's approach: a Tauri `WebviewWindow` (or a child `Webview` if you want it inside the main
window; that needs `tauri = { features = ["unstable"] }`) per account with its own
`.data_directory(<data_dir>/sessions/<account_id>)`, created on `about:blank`, the cookie injected
through the WebView2 `ICoreWebView2CookieManager` (`webview2-com` crate, `with_webview` closure),
then navigated. Reasons:

1. No external browser or download: WebView2 is already a Tauri prerequisite on Windows.
2. The COM cookie manager sets a real domain cookie `.roblox.com` with HttpOnly/Secure, which both
   alyae (dot-stripping bug, worked around with `..roblox.com`) and RoALT (switched to COM) hit when
   using the wrapper APIs. CDP does the same but costs a Chrome download and a WebSocket client.
3. Per-account kept profiles avoid the captcha-on-every-login RoALT reports for fresh profiles
   (`webview.rs:1-6`); clean `.ROBLOSECURITY` out of the profile on open/close so the vault stays
   the only store, as RoALT does.
4. Keep the webview navigation-locked (`on_navigation`) and give it no IPC capability
   (RoALT's `capabilities/main.json` lists only `main`; `main_only` in main.rs refuses commands from
   any other webview).

What Tauri 2 itself exposes, as seen in these projects: `WebviewWindowBuilder::data_directory`,
`.incognito`, `.general_autofill_enabled`, `.on_navigation`, `.on_new_window`, `.on_download`,
`.on_page_load`, `Window::add_child` (unstable), `Webview::with_webview(|PlatformWebview| …)`
giving `controller().CoreWebView2()`, `Webview::cookies()` / `cookies_for_url()` (read; RoALT notes
`cookies_for_url` panics if the window goes away mid-call and that `cookies()` deadlocks on the main
thread, so it is polled from `spawn_blocking`), and `Webview::set_cookie` (write; wry-level, the one
that loses the domain's leading dot). No Tauri plugin is used for cookies by either project. If you
prefer not to depend on `webview2-com`, alyae's `..roblox.com` trick with `set_cookie` is the
documented workaround (`alyae-roblox-login/src/main.rs:221-232`, with a unit test at :410-413).

---

## C. Launch model (Place ID / Job ID / Follow)

### alyae (UX)

The module defines the deck (`modules/roblox-windows/module.toml:86-113`): the primary action
"Join Server" with two boxes, **Place ID** ("place ID or game link (empty: Roblox home)") and
**Job ID** ("job ID or private server link"), plus a "Servers" button beside it; then a one-box
action **Follow** with a **Username** box ("the Roblox user whose server to join"). egui layout in
`crates/alyae/src/ui.rs:3188-3305` (`deck`): the primary action's inputs in one row with a
Bookmarks menu, `[Join Server][Servers]`, then `[box][Follow]` rows for every one-input action.
Several selected accounts run the action 2 s apart (`gap = 2`).

Parsing (`crates/alyae-roblox/src/main.rs:149-237`, `join_from`): Place ID accepts a number or a
`/games/<id>` or `placeId=` link; the Job ID box accepts a 36-char GUID (job), a link with
`privateServerLinkCode=`/`linkCode=` (link code; place taken from the link if the box is empty), a
`roblox.com/share?code=…&type=Server` link resolved with `POST apis.roblox.com/sharelinks/v1/resolve-link
{linkId, linkType:"Server"}` → `privateServerInviteData.{placeId, linkCode}` (`api.rs:444-463`), or a
bare >15-char alphanumeric link code.

**Follow**: the Username box is a number (user id) or a name resolved with

```rust
// refs/alyae/crates/alyae-roblox/src/api.rs:67-79  (GPL-3.0)
pub fn user_id_of(username: &str) -> Result<u64, String> {
    let mut r = agent()
        .post("https://users.roblox.com/v1/usernames/users")
        .send_json(json!({ "usernames": [username], "excludeBannedUsers": false }))
        .map_err(net)?;
    ...
    v["data"][0]["id"].as_u64().ok_or_else(|| format!("no Roblox user is called '{username}'"))
}
```

On Windows no presence is asked (`follow_plan(needs_place = !cfg!(windows))`, `main.rs:239-273`):
the PlaceLauncher `RequestFollowUser` joins by user id alone. On Linux (Mocktail needs a place) it
asks `POST presence.roblox.com/v1/presence/users {userIds:[user]}` with the account's cookie
(`whereabouts`, `api.rs:509-529`) and uses `placeId`, else `rootPlaceId`, else the typed Place ID,
else an error telling the user to type the place.

Launch URI shapes (`crates/alyae-roblox/src/windows.rs:591-621`):

```rust
// refs/alyae/crates/alyae-roblox/src/windows.rs:591-608  (GPL-3.0)
let base = "https://www.roblox.com/Game/PlaceLauncher.ashx";
let follow = j.follow.map(|u| format!("{base}?request=RequestFollowUser&browserTrackerId={browser}&userId={u}"));
let launcher = follow.or_else(|| j.place.map(|p| match (&j.job, &j.link_code) {
    (Some(job), _) => format!("{base}?request=RequestGameJob&browserTrackerId={browser}&placeId={p}&gameId={job}&isPlayTogetherGame=false"),
    (None, Some(code)) => format!("{base}?request=RequestPrivateGame&browserTrackerId={browser}&placeId={p}&linkCode={code}"),
    (None, None) => format!("{base}?request=RequestGame&browserTrackerId={browser}&placeId={p}&isPlayTogetherGame=false"),
}));
// then:
// roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{ms}+placelauncherurl:{urlencoded}
//   +browsertrackerid:{browser}+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp
```

`browserTrackerId` comes from `POST apis.roblox.com/browser-tracker-api/device/initialize`
(`api.rs:533+`); with no place at all the mode is `launchmode:app` (Roblox home). Launched with
`ShellExecuteW("open", url)`; a missing `roblox-player:` handler is "Roblox is not installed". Linux
Mocktail URIs: `roblox://experiences/start?placeId=P&gameInstanceId=J` or `…&userId=U` for follow
(`linux.rs:472-473`).

### niccs v4 (UX)

Single selection (`src/components/accounts/SingleSelectSidebar.tsx`): Place ID and Job ID inputs
live in the store (`store.placeId`, `store.jobId`, `store.launchData`) and are persisted per account
in `Fields.SavedPlaceId/SavedJobId/SavedLaunchData` ("Save place to account") and globally in
`[General] SavedPlaceId/SavedJobId/SavedLaunchData`. Buttons: Join, Shuffle job (random public
server), Recent games / Recent jobs popovers, Launch arguments, a Roblox version per account, and a
**Follow** row with a username box. Multi selection (`MultiSelectSidebar.tsx:69-96`) has Join and
Follow for all selected (with a delay `AccountJoinDelay`, min 12 s when multi-Roblox is on).

Job ID field parsing (`store.tsx:1281-1318`, `joinServer`): `vip:<code>` prefix → join VIP with
`linkCode`; a URL containing `privateServerLinkCode=|linkCode=|code=` → link code; else a job id.
Default place when empty is `5315046213`.

**Follow** (`store.tsx:1334-1353`):

```ts
// refs/Roblox-Account-Manager/src/store.tsx:1334-1353  (GPL-3.0)
async function resolveFollowTarget(username: string, viewerUserId: number): Promise<FollowTarget> {
  const user = await invoke<{ id: number; name: string }>("lookup_user", { username: name });
  const presence = await invoke<...[]>("get_presence", { userIds: [user.id], viewerUserId });
  const entry = presence[0];
  return { userId: user.id, name: user.name || name,
           presenceType: entry?.userPresenceType ?? entry?.user_presence_type ?? 0,
           placeId: entry?.placeId ?? null, jobId: entry?.gameId ?? "" };
}
```

`lookup_user` (`commands/account_api.rs:194-206`) POSTs `users.roblox.com/v1/usernames/users
{usernames:[name]}` with any stored cookie, falling back to anonymous (`api/roblox/users.rs:9-41`).
`needsFollowWarning` (`store.tsx:88-90`) asks "X is not in a game. Try anyway?" when
`presenceType !== 2`. If presence gave both `placeId` and `gameId` (`canDirectJoin`, `store.tsx:85`)
it launches as a normal place+job join; otherwise `launch_roblox(userId, placeId: target.userId,
jobId: "", followUser: true, …)` (`store.tsx:1265-1274`): note the follow target's user id travels
in the `place_id` argument. The Windows launcher then builds

```rust
// refs/Roblox-Account-Manager/src-tauri/src/platform/windows/launch.rs:39-43, 62-68  (GPL-3.0)
} else if follow_user {
    format!("https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestFollowUser&userId={}{}", place_id, ld_param)
} ...
format!("roblox-player:1+launchmode:play+gameinfo:{}+launchtime:{}+placelauncherurl:{}+browsertrackerid:{}+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp",
    ticket, launch_time, urlencoding::encode(&place_launcher_url), browser_tracker_id)
```

with `RequestGame` / `RequestGameJob&gameId=` / `RequestPrivateGame&placeId=&accessCode=&linkCode=`
for the other cases (`launch.rs:1-69`), `&isTeleport=true` optional, `&launchData=` optional.
Launched with `cmd /C start "" <url>` (`launch_url`), or directly `RobloxPlayerBeta.exe --app -t
<ticket> -j <url>` ("old join", `launch_old_join_from`, `launch.rs:106-170`). Multi-launch command:
`launch_multiple(user_ids, place_id, job_id, launch_data, shuffle_job, follow_user_id?)`
(`commands/launch.rs:425-436`). niccs also exposes the legacy HTTP API (`/FollowUser`,
`api/server/route_wrappers.rs:50-54`).

### RoALT (for contrast)

One **target field** (`src/features/launch/TargetField.tsx`): paste a game link, private-server
link, place id, job id or `@username`; the backend parses it (`target_parse`, 150 ms after typing
stops) into a `LaunchTarget` union (`src/bindings/LaunchTarget.ts`): `place | server | privateLink |
privateAccess | shareLink | shareLinkV2 | universe | followUser{userId}`. Username → user via
`user_lookup(name)` (`POST users.roblox.com/v1/usernames/users`, `endpoints.rs:29`). A "Join user"
dialog lists friends online plus the other stored accounts (`join_candidates`,
`friends.roblox.com/v1/users/{id}/friends/online`).

---

## D. Front-end architecture to reuse

### D1. Stack

**RoALT** (`package.json`): React 19.3, react-dom 19.3, TypeScript 7, Vite 8, `@vitejs/plugin-react` 6,
Tailwind 4 via `@tailwindcss/vite`; state/data: `@tanstack/react-query` 5 (every Tauri command is a
query/mutation, `src/data.ts`); animation: `motion` 14 (`motion/react`); fonts: `@fontsource-variable/geist`
and `geist-mono`; Tauri `@tauri-apps/api` 2.12 plus plugins `clipboard-manager`, `dialog`, `opener`;
Rust side also `single-instance`, `window-state`, `log`, `global-shortcut` (`src-tauri/Cargo.toml:16-22`).
No router (a `view` state in `app/Shell.tsx`, `app/navigation.ts`), no component library, no icon
package (own 16 px SVG set in `src/ui/Icon.tsx`). Types come from Rust via `ts-rs` into
`src/bindings/*.ts`. i18n: own `src/i18n` with 18 JSON locales. Styling: plain CSS with CSS custom
properties in `@layer`s plus Tailwind utilities mapped onto the tokens (`src/theme.css`). CSP is
strict (`tauri.conf.json:15`), hence theme CSS is injected through CSSOM, not `<style>`.

**niccs v4** (`package.json`): React 19.2, TypeScript 5, Vite 6, Tailwind 4 (`@tailwindcss/vite`),
`lucide-react` icons, `@dnd-kit/{core,sortable,modifiers,utilities}` for drag-reorder, `i18next` +
`react-i18next`; Tauri `@tauri-apps/api` 2 plus plugins `autostart`, `dialog`, `global-shortcut`,
`process`, `updater` (Rust: also `single-instance`, `window-state`). No router; one giant
React-context store (`src/store.tsx`, ~2 700 lines) holding accounts, selection, dialogs, launch
state, presence, avatars. Styling: Tailwind utilities + a handful of `theme-*` classes bound to CSS
variables in `src/index.css`. CSP is `null`. Build runs on bun (`tauri.conf.json:7-9`).

### D2. Component map

**RoALT** (`src/`):
- `App.tsx`: providers (react-query, MotionRoot), title bar, lock screen vs `Shell`, global events, toaster.
- `app/Shell.tsx`: the view switch (dashboard, accounts, launch, browse, servers, monitor, discord, mods, crashes, debug, settings), lazy chunks, the island (pill nav in the title bar), account rail (side layout), add-account dialog, launch flight.
- `app/navigation.ts`: view ids, section→icon map, Ctrl+digit shortcuts, `requestNavigation`.
- `app/AppMenu.tsx`, `app/TitleTools.tsx`, `app/KillAll.tsx` (hold-to-kill button), `app/LiveActivities.tsx`.
- `ui/TitleBar.tsx`, `ui/island/{Island,Capsule}.tsx` + `island/model.ts`: custom title bar with the navigation "island".
- `features/accounts/AccountsView.tsx`: list page state (filter, sort, columns, selection, validate mutation, presence ids). `AccountsTable.tsx`: windowed rows, FLIP reorder, drag grip. `AccountsToolbar.tsx`, `GroupsNav.tsx`, `columns.ts`, `AccountMenu.tsx` (context menu), `shared.tsx` (Avatar, Status mark, MainBadge, remove-confirm hook).
- `features/accounts/DetailPanel.tsx` with tabs `DetailOverview`, `DetailOverrides`, `DetailHistory`, `DetailCookie` (reveal/copy with countdown). Dialogs: `AddAccountDialog`, `BulkEditDialog`, `ExportCookies`, `GroupDefaultsDialog`, `Limiteds`.
- `features/launch/LaunchView.tsx` (composer: Who / Where / Ready steps), `TargetField.tsx`, `AccountPicker.tsx`, `LaunchButton.tsx`, `LaunchProgress.tsx`, `Favourites.tsx`, `RecentGames.tsx`, `JoinDialog.tsx`, `ServerDrawer.tsx`, `Running.tsx`, `launchQueue.ts`, `target.ts`.
- `features/settings/SettingsView.tsx` + `registry.ts`/`areas.ts`/`search.ts` (searchable settings), `kit/{SettingRow,SettingSection,ActionButton,AutoControl}.tsx`, `rows/{appearance,launching,security,data,shortcuts,about}.tsx`. Theme picker lives in `rows/appearance.tsx` (accent swatches, look editor, corners, background image, share codes).
- `features/vault/{UnlockScreen,PasswordDialog,RecoveryCodeDialog,VaultFlowDialog}.tsx`.
- `features/browse/{BrowseView,RobloxPane,GameDetails,PlayerDetails,Market,Native}.tsx`: the embedded roblox.com pane and its toolbar.
- `features/servers/`, `features/monitor/`, `features/dashboard/` (running clients grid), `features/crashes/`, `features/discord/`, `features/mods/`, `features/news/`, `features/onboarding/`, `features/splash/`, `features/performance/`.
- `ui/`: `Button.tsx`, `Dialog.tsx` (native `<dialog>`, ConfirmDialog), `Drawer.tsx`, `Field.tsx`, `HotkeyField.tsx`, `Icon.tsx`, `Meter.tsx`, `PillToolbar.tsx`, `Popover.tsx`, `Resizer.tsx`, `StatGrid.tsx`, `Status.tsx` (tone+shape status marks), `Tabs.tsx`, `Toast.tsx` (module-level `toast()`), `Tooltip.tsx`, `controls.tsx` (Switch, Segmented, NumberField…), `menu.ts`, `a11y.ts`, `motion/*` (spring tokens, Collapse, Morph, ListPresence), `sounds.ts`, `stored.ts`, `shareCode.ts`.
- Status bar / legend: there is no bottom status bar; status is per-row (`Status` in `accounts/shared.tsx`) and in the island (`LiveActivities`).

**niccs v4** (`src/`):
- `App.tsx`: `StoreProvider` + `PromptProvider`; password / encryption-setup screens; then `TitleBar`, `UpdateBanner`, `Toolbar`, `AccountList` + `DetailSidebar`, `StatusBar`, `ContextMenu`, inline toasts, and every dialog.
- `components/layout/TitleBar.tsx`, `ModalWindowControls.tsx`, `Toolbar.tsx` (search, select all/none, add menu: browser login, user:pass, cookie, legacy import, generator, versions), `StatusBar.tsx` (selected/total, online/in-game counts, launched count, countdowns), `UpdateBanner.tsx`, `PasswordScreen.tsx`, `EncryptionSetupScreen.tsx`, `FirstRunWalkthrough.tsx`, `AppErrorBoundary.tsx`.
- `components/accounts/AccountList.tsx` (dnd-kit sortable, drag-select rectangle), `GroupSection.tsx`, `AccountRow.tsx` (avatar, alias/username, status dots: invalid red, aged, launched, presence), `AccountChip.tsx`, `DetailSidebar.tsx` → `SingleSelectSidebar.tsx` (alias, description, robux, Place/Job/launch data, Join, Follow, version, args) / `MultiSelectSidebar.tsx` (bulk join/follow/refresh/copy/move group/close later), `SidebarSection.tsx`.
- `components/dialogs/`: `AddAccountDialog`, `ImportDialog` (cookie / user:pass / legacy tabs), `AccountFieldsDialog`, `AccountUtilsDialog`, `AfkDialog`, `BottingDialog`, `GeneratorDialog`, `NexusDialog`, `ScriptsDialog`, `ThemeEditorDialog`, `UpdateDialog`, `VersionsDialog`, `MissingAssetsDialog`, `ArgumentsForm`.
- `components/settings/SettingsDialog.tsx` + `TabBar.tsx`, `TabContent.tsx`, tabs `General, Hotkeys, Isolation, Miscellaneous, Optimization, Versions, Watcher, WebServer, Developer, Generator`.
- `components/server-list/`: `ServerListDialog` with `ServersTab`, `GamesTab`, `FavoritesTab`, `RecentTab`, `RecentGamesPopover`, `RecentJobsPopover`, context menus.
- `components/menus/ContextMenu.tsx`, `MenuItemView.tsx`.
- `components/ui/`: `Toggle`, `ToggleRow`, `Select`, `TextField`, `TextAreaField`, `NumberField`, `NumericInput`, `HotkeyField`, `ColorRow`, `SlidingTabBar`, `Tooltip`, `UtilButton`, `UtilInput`, `Divider`, `SectionHeader`, `SectionLabel`, `RestartBadge`, `WarningBadge`.
- `hooks/usePrompt.tsx` (prompt/confirm as promises), `useModalClose.ts`, `useJoinOnlineWarning.ts`, `useSettings.ts`.
- `store.tsx` (everything), `theme.ts`, `themeFonts.ts`, `fontPresets.ts`, `types.ts`, `i18n/`, `utils/cookies.ts` (COOKIE_PATTERN), `scripting/` (Nexus worker).

### D3. Theme systems

**RoALT**: design tokens as CSS custom properties in `src/styles/tokens.css` inside `@layer tokens`:
`:root` (dark) and `:root[data-theme="light"]`. Names: grounds `--ground --surface-1 --surface-2
--surface-3 --island --island-inset --float --titlebar --thumb`; tints `--hover --press --sel --pick
--pick-ring --line --edge`; text `--fg --fg-2 --fg-3 --fg-disabled --on-fg`; accent `--accent
--on-accent --accent-text --accent-ring --focus`; status `--ok --info --attention --warn --err
--tint-pct`; `--backdrop --scrollbar`; shape `--round` (factor) feeding `--r-pill --r-xl --r-lg --r-md
--r-sm --r-xs`; heights `--h-lg --h-md --h-sm --h-xs --titlebar-h --island-h --row-h …`; type
`--font-ui --font-code --fs-* --lh-body`; shadows `--shadow-island --shadow-pop --shadow-dialog`;
motion `--dur-* --ease-* --spring --spring-soft`. A user accent is applied by setting
`data-accent` and `--user-accent --user-on-accent --user-accent-text --user-accent-ring` on `<html>`
(`tokens.css:157-164`, `app/theme.ts:25-33`, contrast-checked by `app/look.ts:accentTokens`).

Custom looks (`app/look.ts`): a `Look { base: "dark"|"light", colors: {id: "#rrggbb"} }` where the
user picks any of `PARTS = ground, titlebar, island, surface-1, surface-2, float, thumb, fg, edge`
(plus `ADVANCED` tokens); `lookTokens()` derives the rest (panels from ground, fields from panels,
text/edges pushed until they read at 4.5:1 / 3:1, tints as `color-mix` of `--fg`), `lookCss()` emits
`@layer tokens{:root[data-look]{--id:value;…}}`, and `applyLook()` adopts it as a constructed
`CSSStyleSheet` (CSP-safe). `BUILTINS` presets: midnight, sakura, terminal, ocean, paper
(`look.ts:154-175`), each with an accent and a corner factor; `CORNERS = [0,50,100,150,200]`;
share codes (`Shared` v1 JSON, `readShared`). `useTheme(settings)` (`app/theme.ts:72-141`) sets
`data-theme` (dark/light/system via `prefers-color-scheme`, custom on its base), `--round`,
background image dimming (`--bg-dim`, see-through surfaces via `color-mix`), persists to
localStorage so `main.tsx:59-78` can apply the last theme before first paint, and cross-fades with
`document.startViewTransition`. Settings fields: `theme: "dark"|"light"|"system"|"custom"`, `accent`,
`look`, `corners`, `backgroundDim`, `backgroundMotion`, `density` (`src/bindings/Settings.ts`).
Tailwind is wired to the tokens in `src/theme.css` (`@theme inline { --color-*: initial; --color-fg:
var(--fg); … }`), so utilities like `text-fg-2` resolve to tokens.

Files to lift wholesale for a theme system (MIT): `src/styles/tokens.css`, `src/app/look.ts`
(pure, no React), `src/app/theme.ts` (depends on `data.ts` for the background image and on
`ui/motion`, `ui/stored`; trim those), the `@theme inline` block of `src/theme.css`, and the
pre-paint snippet in `src/main.tsx:59-78`. `src/styles/controls.css` / `surfaces.css` /
`overlays.css` give the component classes (`.btn`, `.btn-primary`, `.tag`, `.status-pill`, `.menu-item`,
`.dlg-*`) that the tokens style.

**niccs v4**: a flat `ThemeData` of 16 named colours/flags (`src/types.ts:18-41`, mirrored in Rust
`data/settings/theme.rs:57-86`), persisted as an INI section `[Roblox Account Manager]` with keys
`AccountsBG, AccountsFG, ButtonsBG, ButtonsFG, ButtonsBC, ToggleOnBG, ToggleOffBG, ToggleKnobBG,
FormsBG, FormsFG, TextBoxesBG, TextBoxesFG, TextBoxesBC, LabelsBC, LabelsFC, LabelsTransparent,
DarkTopBar, ShowHeaders, LightImages, ButtonStyle, FontSans, FontMono` (compatible with the legacy
C# RAM `RBX Alt Manager` section). `applyThemeCssVariables()` (`src/theme.ts:457-514`) writes
`--accounts-bg --accounts-fg --buttons-bg --buttons-fg --buttons-bc --toggle-on-bg --toggle-off-bg
--toggle-knob-bg --forms-bg --forms-fg --textboxes-bg --textboxes-fg --textboxes-bc --labels-bg
--labels-fg` plus derived `--app-bg --app-fg --panel-bg --panel-fg --panel-muted --panel-soft
--border-color --row-hover --row-selected --input-focus --accent-color --accent-soft --accent-strong
--toggle-on-shadow --titlebar-bg --titlebar-fg --avatar-filter --font-sans --font-mono
--button-radius --button-shadow --button-inset`, and `data-button-style` / `data-show-headers` on
`<html>`. Accent = `buttons_foreground` unless it is grey, then `#38bdf8`. No dark/light switch: every
preset is dark. Presets (`THEME_PRESETS`, `theme.ts:86-283`): Legacy v4, Catppuccin, Studio, Terminal,
Jakarta, Plex, Soft, Bubble, Graphite, Ocean, Sunset (mostly font variations). Fonts: Google Fonts
(`themeFonts.ts` injects a `<link>`; `fontPresets.ts`), local font files under `RAMThemeFonts/`, or
system. Custom presets are saved via `save_theme_preset` to a JSON file and can be exported as
`.ram-theme.json` or a `.ram-theme.zip` bundle with fonts (`data/settings/presets.rs`). The editor is
`components/dialogs/ThemeEditorDialog.tsx` (categories Accounts / Buttons / Forms / Text Boxes /
Labels / Fonts, `ColorRow`, `ToggleRow`).

Liftable (GPL-3.0): `src/theme.ts` + `src/types.ts` ThemeData/ThemeFontSpec + `src/index.css` `:root`
block and `.theme-*` classes, `src/themeFonts.ts` + `src/fontPresets.ts` if you want Google/local
fonts, and `data/settings/theme.rs` if you want INI compatibility with legacy RAM theme files.
niccs' scheme is simpler but its colours are component-named rather than semantic, has no light
mode and no contrast guard.

### D4. Front-end ↔ Rust

**RoALT** (typed wrappers in `src/api.ts`, commands registered in `src-tauri/src/main.rs:826+`):
- accounts: `accounts_list() -> AccountSummary[]`, `account_add_cookie(cookie)`,
  `accounts_import_cookies(text) -> ImportReport`, `account_add_login(username?, password?)`,
  `account_login_cancel()`, `account_update(id, patch: AccountPatch)`, `accounts_reorder(ids)`,
  `accounts_remove(ids)`, `accounts_validate(ids?) -> AccountSummary[]`, `account_info(id, refresh) -> AccountInfo`,
  `accounts_headshots() -> Headshot[]`, `accounts_presence(ids) -> AccountPresence[]`,
  `account_reveal_cookie(id, password?)`, `account_copy_cookie(id, password?)`, `accounts_export_cookies(accountIds?, password?)`.
- launch: `target_parse(input) -> ParsedInput`, `user_lookup(name) -> UserRef`,
  `launch(accountId, target: LaunchTarget, fpsCap?) -> LaunchInfo`, `bulk_launch(request) -> batchId`,
  `bulk_cancel`, `bulk_current`, `instances_list`, `instance_close(id, force)`, `instance_move`, `join_candidates(accountId)`,
  `servers_list(placeId, order, excludeFull, limit, cursor?, accountId?)`, `private_servers_list`.
- browse: `browse_open(accountId, path, rect)`, `browse_bounds(rect, shown)`, `browse_nav(action)`, `browse_close()`.
- vault: `vault_status`, `vault_unlock(password)`, `vault_lock`, `vault_set_mode(mode, currentPassword?, newPassword?)`, `vault_recover`, `app_activity`.
- settings: `settings_get() -> Settings`, `settings_set(patch: SettingsPatch) -> Settings`.
- events: `accounts://changed`, `vault://locked`, `instances://changed`, `app://notice {message, error}`,
  `settings://changed`, `bulk://progress`, `roblox://client`, `roblox://install`, `app://update`,
  `app://network`, `app://close-ask`, `app://open-link`, `log://log`, plus `browse://page`,
  `browse://play`, `browse://blocked`.
- errors: every rejected invoke is `ErrorPayload { kind, message, code? }` with kinds `locked,
  wrongPassword, cookieInvalid, challenge, rateLimited, boundSessionRequired, roblox, network,
  notFound, cancelled, invalid, storage, crypto, internal` (`src-tauri/src/error.rs:46-63`).

**niccs v4** (raw `invoke` calls throughout `src/store.tsx`; registry in `src-tauri/src/lib.rs:286-441`):
- accounts: `get_accounts() -> Account[]`, `add_account(securityToken, username, userId, password?)`,
  `update_account(account)`, `remove_account(userId)`, `reorder_accounts(userIds)`, `save_accounts()`,
  `unlock_accounts(password)`, `needs_password`, `set_encryption_password(password|null)`, `import_old_account_data`.
- auth/info: `validate_cookie(cookie) -> AccountInfo`, `refresh_cookie(userId) -> bool`,
  `get_csrf_token(userId)`, `get_auth_ticket(userId)`, `get_robux(userId)`, `get_user_info(userId)`,
  `lookup_user(username) -> {id,name,displayName}`, `get_presence(userIds, viewerUserId?)`,
  `batched_get_avatar_headshots(userIds)`, `get_avatar_headshots`.
- launch: `launch_roblox(userId, placeId, jobId, launchData, followUser, joinVip, linkCode, shuffleJob)`,
  `launch_multiple(userIds, placeId, jobId, launchData, shuffleJob, followUserId?) -> count`, `cancel_launch`,
  `get_running_instances`, `cmd_enable_multi_roblox`, `cmd_disable_multi_roblox`, `cmd_kill_all_roblox`.
- browser: `open_login_browser`, `extract_browser_cookie`, `close_login_browser`, `open_account_browser(userId)`,
  `import_userpass(username, password)`, `is_browser_ready`, `ensure_browser(force?)`.
- settings/theme: `get_all_settings`, `get_setting(section, key)`, `update_setting(section, key, value)`,
  `get_theme`, `update_theme(theme)`, `get_theme_presets`, `save_theme_preset`, `delete_theme_preset`,
  `import_theme_preset_file`, `export_theme_preset_file`.
- events: `chromium-download-progress`, `browser-login-detected`, `isolation-progress`,
  `isolation-report`, `version-install-progress`, launch progress events used by `launchProgress`.
- errors: plain `String`s.

### D5. What is self-contained, what is tangled, third-party assets

**RoALT (MIT)** — copy with its `LICENSE` text and a credit line:
- Self-contained: `src/styles/tokens.css`; `src/app/look.ts` (only imports a `Look` type and a `Key`
  type); `src/ui/Status.tsx` (tone+shape marks, plus the `.status-mark` rules in `styles/surfaces.css`);
  `src/ui/Icon.tsx` (own SVG path set, no third-party icons); `src/ui/Dialog.tsx`, `Button.tsx`,
  `Field.tsx`, `Tabs.tsx`, `Popover.tsx`, `Tooltip.tsx`, `Toast.tsx` (needs `motion`, `sounds.ts`,
  `signal.ts`, i18n `t`); `src/ui/motion/*` (needs `motion`); `src/features/accounts/shared.tsx`
  Avatar/Status (needs `data.ts` hooks); `src-tauri/src/roblox_api/{mod,endpoints,types}.rs` (Rust,
  needs `error.rs`, `vault::Secret`, the `tr!` macro); `src-tauri/src/webview.rs` cookie/profile helpers.
- Tangled: `app/Shell.tsx`, `AccountsView.tsx`, `LaunchView.tsx`, `SettingsView.tsx` (deep in
  react-query keys, i18n keys, the island, bindings); `app/theme.ts` (background-image and motion
  hooks); anything importing `src/data.ts` or `src/i18n`.
- Third-party assets: Geist / Geist Mono via `@fontsource-variable` (SIL OFL 1.1); `motion` (MIT);
  react-query (MIT); Tailwind (MIT). No icon font. The RoALT name, logo (`src-tauri/icons`) and
  "look" are excluded by `NOTICE.md`.

**niccs v4 (GPL-3.0)** — same licence as RAM, keep the copyright headers / add attribution:
- Self-contained: `src/theme.ts` + `src/index.css` variables; `components/ui/*` (Toggle, Select,
  TextField, NumberField, HotkeyField, ColorRow, Tooltip, SlidingTabBar) — small, only `useTr` from
  `i18n/text`; `hooks/usePrompt.tsx`; `utils/cookies.ts`; `src-tauri/src/chromium/cdp.rs` (CDP
  client, needs tokio-tungstenite, reqwest, serde_json); `src-tauri/src/platform/windows/launch.rs`
  URL builders; `src-tauri/src/data/settings/{theme,presets,ini}.rs`.
- Tangled: `store.tsx` (everything depends on it), `AccountList.tsx` (dnd-kit + store),
  `SingleSelectSidebar.tsx`/`MultiSelectSidebar.tsx`, `SettingsDialog` tabs, `ThemeEditorDialog`.
- Third-party assets: `lucide-react` icons (ISC); `@dnd-kit` (MIT); i18next (MIT); Google Fonts
  loaded at runtime by URL (Outfit, JetBrains Mono, Space Grotesk, Inter, Fira Code, Plus Jakarta Sans,
  IBM Plex, Nunito, Source Code Pro, Rubik, Space Mono — all SIL OFL, fetched from fonts.googleapis.com,
  not bundled); `src/assets/afk-cycle.wav` (origin not stated); `src-tauri/assets/Nexus.lua` (GPL via repo).

**alyae (GPL-3.0)**: no React. Reusable ideas rather than files: `module.toml` action/input
declarations as a launch UX spec; `browser.rs` UA/client-hint consistency; `alyae-roblox-login`'s
`..roblox.com` cookie trick and `may_open` allow-list; `api::pace()` rate-limit pacing.

---

## E. Multi-instance and cookie storage (brief)

**Multi-instance.** RoALT (`src-tauri/src/instances/multi.rs`) holds a mutex named exactly
`ROBLOX_singletonEvent` (unowned) before any client starts, optionally also opens Roblox's shared
login files (`%LOCALAPPDATA%\Roblox\LocalStorage\RobloxCookies.dat`, `appStorage.json`) with share
mode 0 so clients cannot overwrite each other's session, and offers a confirmed "take over" that
closes the event handle inside running clients with `DuplicateHandle(DUPLICATE_CLOSE_SOURCE)`.
niccs (`platform/windows/core.rs:26-47`) creates an *owned* `ROBLOX_singletonMutex`
(`CreateMutexW(…, 1, …)`), checks it with `WaitForSingleObject(0)`, and locks the cookie files too
(`lock_roblox_cookies`); it also has a `diagnose_mutex_holder` command. alyae's helper
(`alyae-roblox/src/windows.rs:551-554`) creates both `ROBLOX_singletonMutex` and
`ROBLOX_singletonEvent` owned, per helper process, for as long as that account's client runs, and
serialises launches with its own `Local\AlyaeRobloxLaunch` mutex plus load slots.

**Cookies at rest.** RoALT: SQLite (`roalt.db`, SQLCipher-encrypted whole with a key file beside
it, `storage/mod.rs`, `storage/cipher.rs`) holding each cookie as a separate AES-256-GCM blob with
AAD `roalt.cookie:<user_id>` under a random data key that is itself wrapped either by DPAPI
(no prompt) or by `DPAPI(seal(Argon2id(master password), key))` with an optional one-use recovery
code (`vault/mod.rs:1-11`); Windows Hello gates reveal/copy in DPAPI mode; auto-lock, lock on
Windows lock/sleep. niccs: the legacy RAM format, one `AccountData.json` with the whole JSON
encrypted by libsodium `secretbox` under a key derived from a password hash (SHA-512) or a
device/master key file, keeping the "Roblox Account Manager created by ic3w0lf22 @ github.com" header
(`data/crypto.rs`, `data/accounts/store.rs`, `data/vault_key.rs`); cookies are plain strings in
memory and are copied to the clipboard by the UI. alyae: SQLite with plain fields as JSON and all
secrets (cookie, password, TOTP) as one XSalsa20-Poly1305 blob per account under a random 32-byte
`vault.key` file (owner-only on Linux), no password by default; `.alyae` export files are Argon2id +
secretbox (`crates/alyae/src/vault.rs`, `store.rs`, `docs/SECURITY.md`). RAM's vault (per-cookie
AEAD, master-password or OS-bound key, cookie never crossing to the front-end except through an
explicit reveal) is closest to RoALT's model and strictly stronger than niccs' and alyae's.

---

## Not determined / caveats

- niccs: I found no code path that flips `Valid` to `false` after a failed validation; the UI's red
  "Invalid session" dot therefore only appears for accounts added with an empty token or imported as
  invalid. Worth re-checking `data/accounts/store.rs` beyond line 90 if this matters.
- niccs `Account.LastUse` is what feeds the "aged" dot; where `LastUse` is bumped (launch? refresh?)
  was not traced.
- RoALT's `watch_cookie` / `close()` body in `browse.rs` (post line 504) was not read in full; the
  header comment states the behaviour (cookie kept if it still signs in as the same user, then deleted
  from the profile).
- Tauri 2's own cookie API: both projects *avoid* `Webview::set_cookie` for a domain cookie; I did
  not find a Tauri plugin for cookies in either project, and did not verify whether newer Tauri
  versions have fixed the leading-dot behaviour.
- alyae's egui UI has no component "files" to reuse; only the module.toml declarations describe the UX.


---

## F. Second study (2026-11-21): performance, crash history, updater, system browser

Done for v0.3, read-only as before. Line numbers refer to the clones as of that date.

### F1. RoALT performance (`src-tauri/src/launcher/{flags,client_settings,mod,client}.rs`, `performance.rs`)

Two mechanisms, both applied right before `client::start` and undone after the client has read them:

1. **XML, not FastFlags, for the frame-rate cap and quality.** `client_settings.rs:1-6` records that
   Roblox's allowlist refuses every frame-rate flag in `ClientAppSettings.json` ("Denied local
   configuration for: DFIntTaskSchedulerTargetFps"). The three keys edited textually in
   `%LOCALAPPDATA%\Roblox\GlobalBasicSettings_13.xml` (`client_settings.rs:31-35`):
   `<int name="FramerateCap">` (5..=360), `<int name="GraphicsQualityLevel">` (1 for lowest) and
   `<token name="SavedQualityLevel">` (1 for lowest, 0 auto). A missing element is inserted before
   `</Properties>`; a file not ending in `</roblox>` is treated as half-written and left alone.
2. **FastFlags** in `<folder of the launched exe>\ClientSettings\ClientAppSettings.json`
   (`flags.rs:187-188`), merged over the user's file, backed up beside it as
   `ClientAppSettings.json.roalt-before` (byte `1` + original, or `0` = no file), written atomically.
   Table (`flags_of`, `flags.rs:31-73`): lighter rendering always sets the four
   `DFIntCSGLevelOfDetailSwitchingDistance*` flags to `"0"`; MSAA `FIntDebugForceMSAASamples`
   1/2/4; texture `DFFlagTextureQualityOverrideEnabled` + `DFIntTextureQualityOverride` 0..3; render
   mode one of `FFlagDebugGraphicsPreferD3D11|Vulkan|OpenGL`; `FFlagDebugSkyGray`;
   `FIntFRMMinGrassDistance` + `FIntFRMMaxGrassDistance` 0; `DFIntDebugFRMQualityLevelOverride` 1;
   `DFFlagDebugPauseVoxelizer`; custom flags last. All values are JSON strings.

Sequencing (`launcher/mod.rs:148-259`): one global `tokio::sync::Mutex` held from before the ticket
until the client has read the files; detection by the client's log (found by the launch's
`browsertrackerid` in its head) showing `SceneManager: resizing main targets` or
`[FLog::Network] Replicator created`, then 1 s, then restore; timeouts 60 s for a log, 15 s for the
mark. A 100 ms guard rewrites the XML if another client's exit overwrote it meanwhile, and a 1 s
watcher after restore undoes values a closing client saved back (the user's values and the values
RoALT wrote are kept in the DB, `settle_values`, `client_settings.rs:144-162`). Under a
bootstrapper (Bloxstrap/Fishstrap/Froststrap/Voidstrap registered for `roblox-player:`) RoALT
launches the bootstrapper with its registered pre-`%1` argument and the URI, keeps the XML step
and **skips FastFlags** ("a bootstrapper manages Roblox's flags", `launcher/mod.rs:195-198`).
`AccountSummary.full_quality` disables low graphics and lighter rendering (not the FPS cap, not
custom flags) and also exempts the client from tiny/minimize/mute/CPU limits. Presets
(`src/features/performance/presets.ts`): Max alts 10 FPS + gray sky + paused voxelizer,
Balanced 30 FPS, Roblox defaults off. What RAM does differently: writes FastFlags into the
bootstrapper's `Modifications\ClientSettings\ClientAppSettings.json` instead of skipping them,
and exempts Main from everything.

### F2. RoALT crash history (`src-tauri/src/instances/{logwatch,tracker}.rs`, `autopsy.rs`)

Logs: `%LOCALAPPDATA%\Roblox\logs\<version>_<yyyymmddThhmmssZ>_Player_<5hex>_last.log`; crash
handler logs carry `_CrashHandler_`; no PID in the name. A launched client's log is the newest
one created since the launch whose first 256 KB names the launch's tracker id (`websiteBTId is
<n>` / `BTID is overriden to <n>`, `logwatch.rs:82-108`); other clients are matched by file
creation time within 3 s of process start. Markers (`logwatch.rs:49-66`): `! Joining game
'<guid>' place <n> at`, `UgcExperienceController: doTeleport`, six disconnect strings
(`Disconnection Notification`, `Lost connection with reason`, `Time to disconnect replication
data`, `Sending disconnect with reason`, `Disconnected from server for reason`, `Connection closed
by server`), `encountered during authentication` / `Error Code: 403`, `Replicator created` /
`serverId:` for in-game, `userid:` on the `GameJoinLoadTime` line, `challengedByGcs`. Player-
initiated = `DisconnectClientInitiated`, `Same account launched`, `DuplicateLogin`, or reason codes
285/273 (`client_initiated`, `logwatch.rs:71-75`). Lines on `FLog::Output/Warning/Error` are ignored
except the join line (game scripts print anything). Kinds: `crash | disconnect | hang | mem |
timed`; a user close is not recorded; 200 rows kept; CSV with BOM and formula defusing
(`commands.rs:3368-3389`). RAM reuses the marker list and the CSV rules, adds a shutdown marker and
a crash-dump check, and records Replaced and Sign-in refused as their own kinds.

### F3. RoALT self-update (`src-tauri/src/self_update.rs`) and the browsers

RoALT: custom updater against `api.github.com/repos/.../releases/latest`, assets
`RoALT-Setup-<v>.exe` + `.exe.sig` (ECDSA P-256 over `"<name>\n" + bytes`, verified with CNG
against an embedded public key), temp file write-locked while verified, Inno installer run with
`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RELAUNCH`, check at start and every
6 h behind a setting, offered in the title-bar "island", never a modal; release notes shown as a
News card from the GitHub release body generated from `CHANGELOG.md`. RAM uses
`tauri-plugin-updater` (minisign-signed `latest.json`) for the same flow.

Browsers: alyae's "Open in browser" is a wry/WebView2 window on a throw-away temp profile with the
cookie set through wry (`alyae-roblox-login/src/main.rs:234-273`); its `browser.rs` only reads the
default browser's family from `UrlAssociations` for user-agent spoofing. RoALT's Browse is a Tauri
child webview on a kept profile with the cookie injected through the WebView2 COM cookie manager.
Neither drives the system browser; niccs v4 does (section B2), which is the model RAM v0.3 follows
with the installed browser instead of a downloaded Chrome.

Headshots (RoALT `roblox_api/mod.rs:118-146`): `thumbnails.roblox.com/v1/users/avatar-headshot?
userIds=<csv ≤100>&size=150x150&format=Png&isCircular=false`, keep only `Completed` entries with
an `https://*.rbxcdn.com` URL, retry `Pending` ids, cache the URL for hours, initials until the
image loads. Place names (both RoALT and alyae): `apis.roblox.com/universes/v1/places/{id}/universe`
then `games.roblox.com/v1/games?universeIds=`.
