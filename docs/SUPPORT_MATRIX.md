# Support matrix

This describes **RAM v0.2**, the Tauri + React application in `app/`. Each
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
path is unit-tested against recorded or scripted responses.

## Accounts and organisation

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Account list with search by username, alias, group, notes | Verified | No | Unit tests on the search rules; screenshots of the list. |
| Groups, aliases, notes, favorites, Main pin | Verified | No | Main is excluded from Select All (Ctrl+A) and from batch launches. |
| Modern and Classic (alyae-style) account layouts | Verified | No | Settings → Appearance → Layout; both screenshotted at 1240×760 and at the 900×560 minimum. |
| Themes: System, Light, Dark, Catppuccin (Latte, Frappé, Macchiato, Mocha), Nord, Dracula, Gruvbox, Rosé Pine, Lineage; accent colour; corner radius | Verified | No | Palettes are CSS variables on `:root[data-theme]`. |
| Versioned store with atomic saves and backup recovery | Verified | No | Unit tests cover interrupted saves, corrupt files, newer versions. |
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
| Reveal/copy a cookie behind Windows Hello | Conditional | No | Hello availability is detected; a Verified/Canceled round trip was not driven on the dev PC. When Hello is not available, the master password is asked instead (DPAPI vaults without Hello: reveal is refused). |
| Cookies never in `store.json`, the log, or error messages | Verified | No | Redaction unit tests assert on distinctive values. |

## Sign-in and the login check

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Add an account by signing in inside RAM's browser window (cookie captured into the vault) | Conditional | **Yes** | The window, profile folder and cookie read-back are implemented against the Tauri 2.12 cookie API; a real sign-in, including Roblox's CAPTCHA and 2-step pages, was not performed. |
| Add an account by pasting `.ROBLOSECURITY` | Verified | No | The cookie is checked right after it is stored. |
| Login check (RoALT's method): `GET users.roblox.com/v1/users/authenticated` | Conditional | **Yes** | Classification unit-tested on scripted responses: 200 valid; 401 signed out; 401 + error 9002/2 *bound session required*; 403 + `rblx-challenge-id` *verification required*; 429 and transport errors leave the state untouched. Not run against live Roblox. |
| Rotated cookie from `Set-Cookie` replaces the stored one | Conditional | Yes | Unit-tested on a scripted response. |
| Robux and Premium shown next to the account | Conditional | Yes | Economy and premium-features endpoints; unit-tested on scripted responses. |
| Check one, selected, or all accounts; status with time of last check | Verified | No | Checks run on a worker thread and stream back as events. |
| Timeout, rate limit, server error or offline never shown as *Signed out* | Verified | No | Unit tests. |

## Open Browser

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Per-account WebView2 window with its own profile folder | Verified | No | `profiles\<id>\` under the data folder; the window opens and navigates. |
| Signed in as the account (cookie injected before navigation) | Conditional | **Yes** | The cookie is written with domain `..roblox.com` (alyae's workaround for the cookie crate stripping one dot) and read back through `cookies_for_url`; if the read-back fails RAM opens the sign-in page instead. Whether roblox.com accepts the injected session was not verified with a real cookie. |
| Navigation limited to Roblox and CAPTCHA hosts | Verified | No | `on_navigation` allow-list; `roblox-player:` links are allowed through so "Play" on the site works. |
| Profile browsing data cleared when the window closes | Verified | No | The vault stays the only cookie store (RoALT's rule). |
| Capture the current session from the window | Conditional | Yes | Reads `.ROBLOSECURITY` back into the vault; same caveat as sign-in. |

## Launching

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Launch by Place ID (Rogue Lineage `3016661674` pre-filled) | Conditional | **Yes** | Ticket request (`auth.roblox.com/v1/authentication-ticket/`, CSRF retry), `roblox-player:` URI and `RequestGame` launcher URL are unit-tested against scripted responses. Not run against live Roblox. |
| Join a specific server (Job ID) | Conditional | Yes | `RequestGameJob`; same caveat. |
| Follow a user by username | Conditional | Yes | Username → id through `POST users.roblox.com/v1/usernames/users`, then `RequestFollowUser`. Presence is not checked first; Roblox shows its own error if the user is not in a game. |
| Launch several selected accounts in sequence | Verified | No | One worker, spaced launches, per-account progress events. |
| Multi-instance (Roblox singleton objects held), on by default | Conditional | **Yes** | Mutex ownership is unit-tested; letting two real clients run at once was not observed. Roblox does not officially support this. |
| Roblox client not installed | Conditional | Yes | The `roblox-player:` launch fails and the error is shown on the account; the ticket is still requested first. |
| Launch by browser hand-off (no cookie) | Unavailable | — | v0.2 launches with a ticket only; sign the account in first. |

## Clients

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| List running Roblox clients (PID, start time, linked account) | Conditional | Yes | Process snapshot; tested with stand-in processes. |
| Link a launch to the one new client that appeared | Verified | Yes (timing) | Rules unit-tested; how long the real launcher takes was not measured. |
| Close a client (normal window close, never terminate) | Conditional | Yes | Tested with a stand-in window. |
| CPU/memory per client, priority and efficiency presets, CPU limit, tiling | Unavailable | — | Present in `ram-core`/`ram-win` from v0.1 but not wired into the v0.2 UI. |

## Import and export

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Preview-first import of usernames, aliases, groups, notes, user IDs | Verified | No | 34 tests on synthetic 3.7.2-format files. |
| Import cookies into the vault (switch) | Verified | No | Unit tests on unencrypted and DPAPI-protected files; a file written by the real legacy app was not read. |
| Password-protected `AccountData.json` | Unavailable | — | Remove the password in the legacy app first. |
| Export to `AccountData.json`, plain or with DPAPI-protected cookies | Verified | No | Round-trips through the importer in tests. |
| Repeat-safe, deduplicated imports; source files untouched | Verified | No | Unit tests. |

## Removed from the legacy app

Local HTTP API, Nexus remote control, Account Control and auto-execute,
developer switches, bulk password import, credential-bearing links, the
auto-updater, FPS unlock, proxies, captcha solving, account utilities, the
server browser. See [DECISIONS.md](DECISIONS.md).
