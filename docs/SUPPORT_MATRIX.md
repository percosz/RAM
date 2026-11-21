# Support matrix

This describes **RAM v0.3**, the Tauri + React application in `app/`. Each
feature has one status:

- **Verified** — exercised on Windows 11 (build 26300) during development,
  by automated test or by running the app.
- **Conditional** — works only under the stated condition, or depends on
  something outside the app.
- **Unavailable** — not offered, with the reason.

A separate column says whether the feature still needs a test against a live
Roblox client or a signed-in Roblox account. **No Roblox account was signed in
and no Roblox client was joined to a game during development**, so everything
that touches a real session or a real client is marked, even where the code
path is unit-tested against recorded or scripted responses. The developer PC
has Brave as the default browser and Froststrap as the `roblox-player:`
handler, which is what the "on this PC" notes refer to.

## Accounts and organisation

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Account list with search by username, alias, group, notes | Verified | No | Unit tests on the search rules; screenshots of the list. |
| Username first; alias and description on the second line; edited in the details pane | Verified | No | Screenshotted. |
| Tick boxes per row, per group and "all shown" | Verified | No | Screenshotted; Ctrl/Shift selection still works. |
| Drag rows to reorder (one row or the ticked selection; dropping in another group moves it) | Verified | No | Driven with mouse messages posted to the window; the new order was read back from `store.json`. |
| Right-click menu (copy, paste cookie, Main, favorite, groups, sort, remove) | Verified | No | Menu screenshotted. Paste cookie asks Roblox whose cookie it is and refuses another account's (that check needs a live cookie). |
| Hide usernames (first three characters only) | Verified | No | Screenshotted; applies to the list, details, clients and crash history. |
| Start-up time | Verified | No | 0.65 s to the window on a copy of a real data folder with seven stale browser profiles (28.8 s before v0.3.2). |
| Roblox headshots | Verified | No | `thumbnails.roblox.com` batch of up to 100 ids, no cookie; URLs cached a day in `store.json`; initials until the image loads. Real headshots loaded for the demo ids. |
| Groups, aliases, notes, favorites, Main pin | Verified | No | Main is excluded from Select All (Ctrl+A), from batch launches and from every performance setting. |
| Row sizes Compact / Normal / Comfortable / Large | Verified | No | Compact screenshotted at the 900×560 minimum. |
| Themes, accent colour, corner radius | Verified | No | Palettes are CSS variables on `:root[data-theme]`. |
| Versioned store with atomic saves and backup recovery | Verified | No | Unit tests cover interrupted saves, corrupt files, newer versions. A v0.2 store loads (dropped settings are ignored). |
| Keyboard: Ctrl+A select all, Enter join, Delete remove, Esc close dialog | Verified | No | |
| High DPI | Conditional | No | WebView2 scales the page with the monitor; checked at 100% only on this machine. |
| Screen-reader output | Conditional | No | Standard HTML controls with labels; not audited with Narrator or NVDA. |

## Cookie vault

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Vault protected with Windows DPAPI | Verified | No | Round-trip tests; `vault.bin` is per Windows user and per PC. |
| Vault protected with a master password (Argon2id 64 MiB/3 passes, AES-256-GCM) | Verified | No | Unit tests, including wrong password and tampered file. |
| Auto-lock after inactivity; manual lock | Verified | No | Unit tests on the clock rules; the UI polls every 20 s. |
| One-time recovery code | Verified | No | Unit tests. The code is shown once at setup and is not stored in clear. |
| Change master password; remove the vault | Verified | No | Unit tests. Removing the vault deletes every stored cookie. |
| Reveal/copy a cookie behind Windows Hello | Conditional | No | The prompt is attached to RAM's window through `IUserConsentVerifierInterop` and runs on a worker thread (v0.3.0 showed it from the UI thread, which froze the app). A Verified/Canceled round trip was not driven on the dev PC. When Hello is not available, the master password is asked instead; a DPAPI vault without Hello reveals after the Windows sign-in alone. |
| Cookies never in `store.json`, the log, or error messages | Verified | No | Redaction unit tests assert on distinctive values; the browser module maps cookie errors to fixed strings. |

## Sign-in and the login check

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Add an account by signing in inside your browser (cookie captured into the vault) | Conditional | **Yes** | Open Browser without a cookie shows roblox.com/login in that account's profile; Capture session reads the cookie back over DevTools. The mechanism is tested with a fake cookie (see Open Browser); a real Roblox sign-in, CAPTCHA and 2-step pages included, was not performed. |
| Add an account by pasting `.ROBLOSECURITY` | Conditional | Yes | RAM asks Roblox whose cookie it is and creates or refreshes that account; the line parser (bare value, `.ROBLOSECURITY=`, `user:pass:cookie`, `username:password` refused) is unit-tested. |
| Add many at once (cookies one per line) | Conditional | Yes | Sequential, 0.7 s apart, with a progress count and a per-line problem list. |
| Sign in with Roblox (Add dialog): the browser opens on the sign-in page, RAM polls for the session and captures it | Conditional | **Yes** | Uses the same DevTools mechanism as Open Browser on a `sign-in` profile that is signed out after capture; a real sign-in was not performed. |
| Login check (RoALT's method): `GET users.roblox.com/v1/users/authenticated` | Conditional | **Yes** | Classification unit-tested on scripted responses: 200 valid; 401 signed out; 401 + error 9002/2 *bound session required*; 403 + `rblx-challenge-id` *verification required*; 429 and transport errors leave the state untouched. Not run against live Roblox. |
| Rotated cookie from `Set-Cookie` replaces the stored one | Conditional | Yes | Unit-tested on a scripted response. |
| Robux and Premium shown next to the account | Conditional | Yes | Economy and premium-features endpoints; unit-tested on scripted responses. |
| Check one, selected, or all accounts; status with time of last check | Verified | No | Checks run on a worker thread and stream back as events. |
| Timeout, rate limit, server error or offline never shown as *Signed out* | Verified | No | Unit tests. |

## Open Browser

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Default browser detected (`UrlAssociations\https` → ProgId → command) | Verified | No | Brave found on this PC; Chrome/Edge/Vivaldi/Opera/Chromium recognised by exe name; unit tests on the command parsing. |
| Firefox or another non-Chromium default | Conditional | No | Falls back to the first installed Chromium browser (Brave, Chrome, Edge, Vivaldi at their usual paths); with none, the button explains. Not driven on this PC. |
| Separate profile per account (`browser-profiles\<id>`), cookie injected over DevTools (`Storage.setCookies`) | Verified | No | Integration test against Brave 158 on this PC with a fresh scratch profile and a fake cookie: open, read back, reuse of the open window, close, cookie files deleted. |
| Signed in on roblox.com with a real session | Conditional | **Yes** | The cookie attributes match RoALT's (`.roblox.com`, Secure, HttpOnly, Lax). Whether roblox.com accepts the injected session in Brave was not verified with a real cookie. |
| Cookie database deleted when the window closes | Verified | No | A watcher waits on the browser process; a sweep at start covers crashes. |
| DevTools port exposure | Conditional | — | While the window is open, any local program could connect to that profile's DevTools port (loopback only). Documented in the README. |
| Capture the current session from the window | Conditional | Yes | Same caveat as sign-in. |

## Launching

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Launch by Place ID (Rogue Lineage `3016661674` pre-filled) | Conditional | **Yes** | Ticket request (`auth.roblox.com/v1/authentication-ticket/`, CSRF retry), `roblox-player:` URI and `RequestGame` launcher URL are unit-tested against scripted responses. Not run against live Roblox. |
| Place name under the launch bar | Verified | No | `apis.roblox.com/universes/v1/places/{id}/universe` then `games.roblox.com/v1/games`; unit-tested on scripted responses, and the real lookup ran for Rogue Lineage. Unknown ids disable Join Server. |
| Join a specific server (Job ID) | Conditional | Yes | `RequestGameJob`; same caveat. |
| Follow a user by username | Conditional | Yes | Username → id through `POST users.roblox.com/v1/usernames/users`, then `RequestFollowUser`. Presence is not checked first; Roblox shows its own error if the user is not in a game. |
| Launch several selected accounts in sequence | Verified | No | One launch at a time, spaced launches, per-account progress events. |
| Multi-instance (Roblox singleton objects held), on by default | Conditional | **Yes** | Mutex ownership is unit-tested; letting two real clients run at once was not observed. Roblox does not officially support this. |
| Launching through a bootstrapper (Froststrap/Bloxstrap/Fishstrap/Voidstrap) | Conditional | Yes | The registered `roblox-player:` handler is used as is; Froststrap is detected on this PC. |
| Roblox client not installed | Conditional | Yes | The `roblox-player:` launch fails and the error is shown on the account; the ticket is still requested first. |

## Performance (alt clients)

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| FPS cap and lowest graphics through `GlobalBasicSettings_13.xml` (`FramerateCap`, `GraphicsQualityLevel`, `SavedQualityLevel`) | Conditional | **Yes** | Text editing, insertion into a bare file, restore logic and crash-recovery record are unit-tested (42 tests). Whether the client honours the values was not observed with a real client. |
| Lighter rendering and custom FastFlags through `ClientAppSettings.json` | Conditional | **Yes** | Flag table, merge with the user's own file, backup/restore are unit-tested. On this PC the file is Froststrap's `Modifications\ClientSettings\ClientAppSettings.json`, which Froststrap copies to the client at launch; with Roblox's own launcher it is the client folder's file. |
| Written before each launch, restored after the client reads them | Conditional | Yes | The watcher looks for `LoadClientSettingsFromLocal` / `Replicator created` / `resizing main targets` in the client's log, waits 1 s and restores; a 60 s timeout restores anyway. Not observed with a real client. |
| Per-account FPS cap and Full quality; Main never touched | Verified | No | Unit tests on the effective-settings rules. |
| User's values put back after a crash or when a client re-saves the XML on exit | Conditional | Yes | The pending restore is kept in `store.json` and finished at the next start; unit-tested, not observed live. |
| Window layout, minimize, mute, CPU limits (RoALT's other controls) | Unavailable | — | Not built in v0.3. |

## Diagnostics

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Client ↔ account linking (launch tracker id in the log head, else the `userid` the client logs at join) | Conditional | **Yes** | Log parsing unit-tested on realistic lines and sanity-run on a real log from this PC; not observed with a live client. |
| Crash history (Disconnected / Crashed / Replaced / Sign-in refused, reason, uptime, place) | Conditional | Yes | Classification rules unit-tested (17 tests); the list, filters, CSV export and "Open log" screenshotted with sample records. User closes are not recorded. |
| Debug log with levels and "Copy diagnostics" (redacted) | Verified | No | Log lines carry `[date][time][LEVEL]`; redaction unit-tested. |

## Updates

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Check GitHub Releases (`latest.json`) after start and every 6 h; offer in the sidebar | Conditional | — | `tauri-plugin-updater` with RAM's public key; no release existed yet when this was written, so the check path was exercised only as "could not fetch" (logged at WARN). |
| Install on click (signed NSIS installer, passive mode, relaunch) | Conditional | — | Standard plugin flow; not exercised end to end. Portable copies are told to download instead. |
| "What's new" once after an update | Verified | No | Notes come from the embedded CHANGELOG section for the running version. |

## Clients

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| List running Roblox clients (PID, start time, linked account) | Conditional | Yes | Process snapshot; tested with stand-in processes. |
| Close a client (normal window close, never terminate) | Conditional | Yes | Tested with a stand-in window; recorded as Closed, not a crash. |
| CPU/memory per client, priority and efficiency presets, CPU limit, tiling | Unavailable | — | Present in `ram-core`/`ram-win` from v0.1 but not wired into the UI. |

## Import and export

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Preview-first import of usernames, aliases, groups, descriptions, user IDs | Verified | No | 34 tests on synthetic 3.7.2-format files; the DPAPI-protected file from the real legacy app on this PC decrypts (27 accounts). |
| Import cookies into the vault (switch), also for accounts already present | Verified | No | Unit tests on unencrypted and DPAPI-protected files; matched by username or legacy user id. |
| Password-protected `AccountData.json` | Unavailable | — | Remove the password in the legacy app first. |
| Export to `AccountData.json`, plain or with DPAPI-protected cookies | Verified | No | Round-trips through the importer in tests. |

## Removed from the legacy app

Local HTTP API, Nexus remote control, Account Control and auto-execute,
developer switches, bulk password import, credential-bearing links, FPS
unlock, proxies, captcha solving, account utilities, the server browser, and
(since v0.3) the Classic layout. See [DECISIONS.md](DECISIONS.md).
