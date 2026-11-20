# Support matrix

Each feature has one status:

- **Verified** — exercised on Windows 11 (build 26300) during development,
  by automated test or by running the app.
- **Conditional** — works only under the stated condition, or depends on
  something outside the app.
- **Unavailable** — not offered, with the reason.

A separate column says whether the feature still needs a test against a live
Roblox client or a signed-in Roblox account. No Roblox client was run and no
Roblox account was signed in during development, so everything that touches
a real client or a real sign-in is marked.

## Accounts and organisation

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Account list with search by username, alias, tag, notes, group | Verified | No | Unit tests and UI benchmark. |
| Groups, tags, favorites, multiline notes, details pane | Verified | No | |
| Main account pin; excluded from Select All and batch launches by default | Verified | No | Unit tests. |
| Versioned store with atomic saves and backup recovery | Verified | No | Unit tests cover interrupted saves, corrupt files, newer versions. |
| Keyboard shortcuts, visible focus | Verified | No | See the README for the list. |
| Themes: System, Light, Dark, Lineage; custom accent; compact density | Verified | No | Dark scroll bars and combo boxes need Windows 10 1809 or later. |
| High-contrast mode | Conditional | No | Uses the system colors; checked by code path, not with a screen reader session. |
| Screen-reader output | Conditional | No | Native controls expose standard UI Automation data; not audited with Narrator or NVDA. |
| High DPI (per-monitor v2) | Conditional | No | Layout is computed from the window's DPI; checked at 100% only on this machine. |

## Import

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Preview-first import of aliases, groups, notes, saved places, favorites | Verified | No | 34 tests on synthetic 3.7.2-format files. |
| Unencrypted `AccountData.json` | Verified | No | |
| Windows-protected `AccountData.json` | Conditional | Yes (a real legacy file) | Only on the PC that wrote it. The decrypt call is tested by round trip; a file written by the real legacy app has not been read. |
| Password-protected `AccountData.json` | Unavailable | — | Needs the legacy password-derivation scheme; remove the password in the legacy app first. Other files still import. |
| Repeat-safe, partial and deduplicated imports; source files untouched | Verified | No | Unit tests. |
| Credentials never imported; private links redacted from logs | Verified | No | Unit tests assert on distinctive secret values. |

## Sign-in

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Open Roblox sign-in or recovery page in the default browser | Verified | No | |
| Open in a named browser profile (Chrome, Edge, Brave, Vivaldi, Firefox) | Conditional | Yes (per browser) | Command lines are unit-tested; launching each browser with a real profile was not exercised. Chromium's `--profile-directory` is defined in Chromium source but not in end-user documentation; Firefox's `-P` is documented by Mozilla. Opera and Opera GX have no profile switch and use the default-browser path. |
| Status states with last check time and recovery action | Verified | No | Unit tests. |
| Timeout, rate limit, server error or offline never shown as Signed out | Verified | No | Unit tests on both the rules and the OAuth error mapping. |
| *Verified* status through Roblox OAuth 2.0 (PKCE) | Conditional | Yes | Needs your own OAuth app (client ID) with scopes `openid profile` and a `http://localhost:<port>/callback` redirect. Roblox limits unreviewed apps to 10 users. The flow is tested against scripted responses only. |
| *CAPTCHA required* status | Conditional | Yes | Shown only from an explicit signal. No documented interface reports a CAPTCHA to a desktop app today, so in practice this state is not reached; *Verification required* is set when you report it. |
| Confirming the browser's session without OAuth | Unavailable | — | The manager does not read browser cookies. Status stays *Check in browser*. |
| Launch paused while verification is pending; resume afterwards | Verified | No | Unit tests. |

## Launching and destinations

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Link validation (place ID, game page, private-server link, share link, live server) | Verified | No | Unit tests, including rejection of look-alike hosts. |
| Saved destination vs. live server that may have expired | Verified | No | |
| Rogue Lineage pinned (place `3016661674`) | Verified | Yes (join) | The place ID was checked against Roblox's public API. Rogue Lineage does not allow private servers. Joining Khei or Gaia sub-places directly is not verified. |
| Launch by browser hand-off (`https://www.roblox.com/games/start?placeId=…`) | Conditional | Yes | Documented by Roblox, though marked as superseded by share links. Requires the browser profile to be signed in; the browser asks before opening Roblox. Which account the client uses when the Roblox app has its own session is not verified. |
| Join a specific server (`gameInstanceId`) | Conditional | Yes | Documented parameter; community reports say it is sometimes ignored. |
| Private-server and share links | Conditional | Yes | Stop working when the owner regenerates the link. |
| Batch queue with bounded concurrency, spacing and cancel | Verified | No | Unit tests. Hand-offs already sent to the browser cannot be recalled. |
| Several clients at once | Conditional | Yes | Roblox does not officially support concurrent clients and Waypoint does not bypass its single-instance check. If a second launch replaces or is refused by the first client, Waypoint shows what it observes. |
| Launch with an authentication ticket (no browser) | Unavailable | — | Requires the session cookie. |

## Client tracking

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Detect client start and exit without polling | Conditional | Yes | Exit waits and the window-show hook are tested with stand-in processes. Whether the Roblox client's protection lets a same-user process open it with `SYNCHRONIZE` and query rights is not verified; if not, tracking degrades to the snapshot list. |
| Link a client to an account (one hand-off, one new client) | Verified | Yes (timing in practice) | Rules are unit-tested. How long the browser prompt and the Roblox launcher take, and whether the launcher starts extra processes with the client's image name, needs a live check. |
| Unconfirmed state on ambiguous matches; manual assignment | Verified | No | Unit tests. |
| Links survive a manager restart | Verified | No | Matched by process ID and creation time. |
| Microsoft Store client | Conditional | Yes | Detected by image name and package path of the older Store app; the newer "Roblox – Windows" package is untested. |
| Detect an in-game disconnect while the process stays open | Unavailable | — | Would need reading client logs or memory. *Rejoin* is offered after the process exits. |

## Performance tools (all off by default)

| Feature | Status | Needs live test | Notes |
|---|---|---|---|
| Presets Off / Balanced / Low Resource; per-account override; effective preset shown | Verified | No | Unit tests. |
| Main and unidentified clients exempt | Verified | No | Unit tests. |
| Lower priority class, restore on reset | Verified | Yes | Tested on stand-in processes. Whether the client's protection allows `PROCESS_SET_INFORMATION` from an unsigned same-user process is unknown; a refusal is shown per client and nothing is recorded as applied. |
| Windows efficiency mode (EcoQoS) | Verified | Yes | As above. |
| Job Object CPU limit | Conditional | Yes | Separate opt-in. Verified on a stand-in process (2.00 → 0.22 CPU-seconds per 2 s at a 1% limit, back to 2.00 after clearing). Needs `PROCESS_SET_QUOTA` and `PROCESS_TERMINATE` rights on the client; expected to be refused by a protected client, in which case it is reported and skipped. The limit is a share of total CPU. The limit is removed when the manager exits; if the manager crashes instead, the limit stays on that client until the client closes. |
| One-click reset of all overrides | Verified | No | Unit tests cover priority, efficiency mode, CPU limit, window positions and restart recovery. |
| Live CPU, working set, private memory per client | Verified | Yes | Standard process counters; needs query rights on the client. |
| Live GPU use per client | Conditional | Yes | From the Windows "GPU Engine" counters when they exist for the process; shown as a dash otherwise. Collected only while the Performance view is open. |
| Per-client FPS cap | Unavailable | — | Roblox stores one frame-rate cap per Windows user. Shown read-only as a global setting. |
| Per-client graphics preset | Unavailable | — | Same reason. |
| Hard GPU cap | Unavailable | — | Windows has no per-process GPU limit. |
| Memory thresholds and notifications | Verified | No | Advisory only. Notifications use a tray balloon. |
| Close a selected background client | Conditional | Yes | Sends a normal close request to the client's window after confirmation. Never terminates. |
| Memory trimming, forced termination | Unavailable | — | Deliberately not provided. |
| Idle reminders | Conditional | Yes | Based on how long a client has not been the foreground window while Waypoint is running. No input is sent to the client. |
| Focus a client; tile identified clients; undo tiling | Conditional | Yes | Tested with a stand-in window. Windows may refuse to bring a window forward when the request is not user-initiated. |

## Removed from the legacy app

Local HTTP API, Nexus remote control, Account Control and auto-execute,
developer switches, bulk password and cookie import, credential-bearing
links, the auto-updater, FPS unlock, the multi-instance mutex, proxies,
captcha solving, account utilities. See [DECISIONS.md](DECISIONS.md).
