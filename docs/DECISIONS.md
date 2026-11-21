# Design decisions

This records what was kept, rewritten or removed from Roblox Account Manager
3.7.2, and the choices that shape the rewrite. The line-by-line audit of the
legacy source is in [LEGACY_AUDIT.md](LEGACY_AUDIT.md); the evidence for
claims about Roblox, browsers and Windows is in
[RESEARCH_NOTES.md](RESEARCH_NOTES.md).

## Source revision

| | |
|---|---|
| Fork point | `c2b13d146d5db609d499870145e8d2dff2506581` (master, 2024-07-18) |
| Latest stable release | `3.7.2` = `79f61f3351df61fb3774dfa854ab868954da5389` |
| Difference | Two commits; one changed line in `Classes/Account.cs` (a content-type fix for friend requests) |
| Upstream status | Archived, GPL-3.0 |

Because the two revisions differ by one line in a removed feature, 3.7.2 is
the migration target: the importer reads what 3.7.2 writes.

## Update 4: passwords, the sign-in queue, client limits (v0.4)

- **RAM now handles Roblox passwords, on request.** Users of the legacy app
  rely on bulk `username:password` import, so v0.4 adds it, but as a queue:
  one account at a time in the user's own browser, on Roblox's own sign-in
  page, typed in and submitted over DevTools (`Input.insertText` after the
  page has finished loading, a read-back check, Enter, then a real mouse
  click on Sign in as the fallback; v0.4.0 assigned the values from script,
  which Roblox's form often ignored), with the user solving any check. RAM
  never posts credentials to Roblox itself. The legacy app opened every
  account at once; a queue keeps each sign-in looking like a person and lets
  a wrong password be skipped. Passwords are kept only when "Save passwords"
  is on, sealed in the vault exactly like cookies and released only through
  the same Windows Hello / master-password gate.
- **Password-protected AccountData.json is opened** the way the legacy app
  does: SHA-512 of the password, Argon2i v1.3 (ops 6, 128 MiB), XSalsa20-
  Poly1305 secretbox. Verified against a file encrypted by the legacy app's
  own libsodium.dll. The password is used once and not stored.
- **Client limits follow RoALT**: minimize without focus, shrink below
  Roblox's minimum window size by briefly switching the window to a
  borderless style, mute through the Core Audio session of the process, cap
  CPU through a job object only while the client is in the background, trim
  memory on a schedule, and a QoS policy for bandwidth (the only part that
  needs elevation, and the only part that also affects Main). Clients RAM
  cannot tie to an account are never limited, because one of them might be
  Main.
- **Default browser** is read from `UserChoiceLatest` first: newer Windows 11
  builds stopped updating `UserChoice`, which made RAM keep the old browser.

## Update 3: your own browser, light alts, crash history, updates (v0.3)

Driven by use with a real account list (27 accounts imported from the legacy
app) and by the parts of RoALT its users rely on. Read with
`REFERENCE_CODE_STUDY.md` section F.

- **Open Browser uses the user's default browser**, not an embedded webview.
  Brave/Chrome/Edge/Vivaldi are started with a per-account `--user-data-dir`
  and `--remote-debugging-port=0`; RAM connects to the DevTools port the
  browser writes into the profile, sets `.ROBLOSECURITY` for `.roblox.com`
  with `Storage.setCookies`, opens roblox.com and closes the launcher tab. The
  idea is niccs v4's (which downloads Chrome for Testing); RAM drives the
  browser already installed. The embedded window of v0.2 is gone: the user
  asked for their real browser, and fresh WebView2 profiles draw sign-in
  challenges. The cost is a loopback DevTools port while the window is open,
  stated in the README. Firefox as default falls back to any installed
  Chromium browser. The profile's cookie database is deleted when the
  browser exits and swept at start, so the vault stays the only persistent
  store.
- **Performance follows RoALT's two files.** Roblox refuses frame-rate
  FastFlags in `ClientAppSettings.json`, so the FPS cap and the quality level
  go into `GlobalBasicSettings_13.xml` (`FramerateCap`, `GraphicsQualityLevel`,
  `SavedQualityLevel`); rendering goes into FastFlags. Both are written right
  before a launch, under one lock, and restored after the client's log shows
  `LoadClientSettingsFromLocal` / `Replicator created` (plus 1 s), or after
  60 s. The user's values and ours are recorded in `store.json` so a crash
  mid-launch is undone at the next start. Where RoALT skips FastFlags under a
  bootstrapper, RAM writes them into the bootstrapper's own
  `Modifications\ClientSettings\ClientAppSettings.json` (Froststrap, Bloxstrap,
  Fishstrap, Voidstrap), which the bootstrapper copies to the client at every
  launch, so flags work through Froststrap too. **Main is exempt from
  everything**, including the FPS cap (RoALT exempts Full-quality accounts
  from quality only); any other account can be marked Full quality or given
  its own cap. Window tiling, minimize, mute and CPU limits were left out.
- **Crash history from the client's log.** A watcher binds each
  `RobloxPlayerBeta.exe` to the `_Player_` log whose file-name time is within
  six seconds of the process start, preferring a log whose head names the
  browser tracker id of a launch RAM made; the `userid` in the join line
  links clients RAM did not launch. Endings are classified from RoALT's
  marker list (disconnect lines, reason codes 285/273, teleports, shutdown)
  plus a new crash dump under `logs\crashes`; user closes are not recorded.
- **Updates through `tauri-plugin-updater`** reading `latest.json` from the
  GitHub release, signed with a minisign key whose public half is in the
  config. The offer is a sidebar pill, never a modal; install happens on
  click; What's new comes from the embedded CHANGELOG. Portable copies are
  pointed at the release page. RoALT's custom ECDSA updater was read, but the
  Tauri plugin does the same job with less code to trust.
- **Rows show the username**; alias and description are the second line.
  Tick boxes replace the Shift/Ctrl-only selection. The Classic layout was
  removed rather than kept in step with every change.

## Update 2: Tauri shell, simpler launching, browser sessions (v0.2)

The native Win32 shell (`crates/ram-app`, kept for reference, no longer built)
is replaced by a **Tauri 2 + React/TypeScript** application in `app/`. The
Rust core is unchanged in role: `ram-core` holds the vault, importer/exporter,
ticket flow and status rules; `ram-win` the Windows layer; the Tauri crate only
adds state and commands. This was a deliberate trade: a webview front-end gives
the look of RoALT and niccs' v4 that native controls could not, at the cost of
the WebView2 dependency (present on Windows 10/11) and a larger binary.

Alongside the shell change, three behaviours changed after reading RoALT,
alyae and niccs closely (`REFERENCE_CODE_STUDY.md`):

- **Launching is Place ID / Job ID / Follow again.** The named-destination
  system and its view are gone. The launch bar is the classic one: a place id
  (Rogue Lineage pre-filled), an optional job id, and a username to follow
  (resolved with `POST users.roblox.com/v1/usernames/users`, then
  `RequestFollowUser`). Saved destinations in old data files are ignored.
- **The login check is RoALT's**, replacing the OAuth 2.0 option. `GET
  users.roblox.com/v1/users/authenticated` with the cookie and a browser user
  agent: 200 is valid; 401 invalid; 401 with Roblox error 9002/2 "bound
  session required"; 403 with `rblx-challenge-id` a verification challenge;
  429 and transport errors leave the stored state untouched. A `.ROBLOSECURITY`
  in `Set-Cookie` replaces the stored cookie. Robux and Premium are fetched
  alongside. The OAuth module remains in `ram-core` but is unused.
- **"Open Browser" is an embedded, signed-in browser window** (alyae's
  model): a Tauri webview window per account with its own WebView2 profile
  folder, the cookie injected before navigation (domain written as
  `..roblox.com` because the cookie crate strips one leading dot), navigation
  limited to Roblox and CAPTCHA hosts, and the profile's browsing data cleared
  on close so the vault remains the only cookie store (RoALT's rule). The same
  window captures the cookie after a manual sign-in, which is how an account
  is added without pasting anything. The earlier "bind an account to a Chrome
  profile" option is gone.

## Update: the cookie vault and direct launch

An earlier revision of this document described a design that stored **no**
credentials and launched only through the browser. That is no longer the
default. RAM now keeps the original program's model — storing each account's
session cookie and launching the client directly with an authentication
ticket — but does so securely:

- Cookies live in an **encrypted vault** (`vault.bin`), never in the plain
  data file or the log. The vault is protected with Windows data protection
  (DPAPI) or a master password (Argon2id + AES-256-GCM) with auto-lock and a
  one-time recovery code. Revealing or copying a raw cookie is gated behind
  Windows Hello or the master password. See `crates/ram-core/src/vault.rs`.
- **Direct launch** (`ram-core/src/roblox.rs`) exchanges a stored cookie for
  a one-time Roblox authentication ticket and starts the client, falling back
  to the browser when no cookie is stored or the cookie has expired.
- **Multi-instance is on by default** (`ram-win/src/multi_instance.rs`): RAM
  holds Roblox's singleton object so several clients can run. It owns a named
  kernel object only — it does not modify or inject into the client. Roblox
  does not officially support this; the support matrix states the caveat.
- Sign-in status can now reach **Verified** from a cookie check
  (`users.roblox.com/v1/users/authenticated`) as well as from OAuth.

The security trade-off is explicit: a stored session cookie is as powerful as
a password, so the vault, the Hello/password gate and the master-password mode
exist to protect it at rest. The rest of this document — the browser hand-off,
the honest-status rules, the removed remote-control and self-update features,
the performance tools — still holds. The browser hand-off remains available
and is the path for accounts without a stored cookie.

## The original no-credentials design (superseded)

The sections below describe the stricter no-credentials approach the rewrite
started from. It remains an accurate description of the browser hand-off path,
which is still present; where it says RAM stores no cookies, read "unless you
set up the vault and store one".

The legacy app stores each account's `.ROBLOSECURITY` cookie and password,
requests authentication tickets with them, and starts the client itself.
Everything else follows from that: the embedded browser, cookie refresh, the
local web API that can return cookies, the encryption modes.

RAM stores account names and notes, and — when you opt in — encrypted
cookies. For accounts without a stored cookie, Roblox handles the password,
two-step verification and CAPTCHA in the user's own browser, and the browser
starts the client. In that case the manager cannot see a browser session, so
it says what it knows and what it does not:

- **Sign-in status** is `Verified` only when Roblox confirms the identity
  through its documented OAuth 2.0 flow, `Signed out` only when Roblox
  explicitly rejects a saved authorization, and `Check in browser` after any
  hand-off. A timeout, rate limit, server error or offline PC never changes
  the status.
- **Client status** is based on observed processes. A launch is linked to a
  client only when exactly one hand-off was in flight and exactly one new
  client appeared; otherwise the account shows `Unconfirmed` until the user
  assigns the client by hand.

## Feature decisions

| Feature (legacy) | Decision | Why |
|---|---|---|
| Account list, alias, description, groups | **Keep**, rewritten | Core organisation. Adds tags, favorites, Main pin, multi-field search. |
| Group sort prefix (`001 Name`) | **Rewrite** | Legacy strips an unanchored match and mangles names like `Team 12`; the prefix is now stripped only at the start. |
| Cookie and password storage, three encryption modes | **Remove** | The manager no longer holds credentials. The importer can still read the unencrypted and Windows-protected files once, to take the metadata. |
| Embedded browsers (CefSharp, Puppeteer) and credential capture | **Remove** | Sign-in happens in the user's installed browser. No embedded browser ships. |
| Bulk `user:pass` and cookie import | **Remove** | Credential handling. |
| Authentication-ticket launch (`roblox-player:` URI) | **Remove** | Requires the cookie. Replaced by opening `https://www.roblox.com/...` links in the browser profile bound to the account. |
| Saved place, job ID, private server per account | **Rewrite** | Now named destinations with a durability label: saved, saved link, or live server that may have expired. |
| Recent and favorite games | **Keep**, rewritten | Rogue Lineage is a built-in pinned game; other games stay usable. |
| Follow user, server browser, shuffle to random server, region lookup, player finder | **Remove** (first release) | Need the cookie, or send data to third parties (`ip-api.com`). A public-server picker could return later through the unauthenticated endpoint. |
| Multi-Roblox (holding `ROBLOX_singletonMutex`) | **Remove** | Not a supported interface; Roblox states that running several clients is not officially supported. RAM does not defeat the single-instance check. Tracking, presets and tiling work with however many clients Roblox itself allows. |
| Process tracking through `-b <browserTrackerId>` on the command line (WMI) | **Rewrite** | The value was injected by the legacy launcher and no longer exists. Tracking now uses process snapshots, exit waits and a window-show hook, with evidence-based account links. |
| Bundled Sysinternals `handle.exe`, log-file tailing with remotely downloaded patterns | **Remove** | Redistribution and trust concerns; closes handles inside the client in some forks. |
| Roblox cookie-file lock (`RobloxCookies.dat` held open) | **Remove** | Deliberately breaks the client's own file. |
| FPS unlock by writing `ClientAppSettings.json` | **Remove** | The flag is no longer honored (Roblox restricted fast flags to an allow-list in September 2025) and it changes client files. The frame-rate cap is shown read-only as a global Roblox setting. |
| Auto-relaunch, Account Control, Nexus websocket, `Nexus.lua`, auto-execute | **Remove** | Remote control and script execution inside clients. |
| Local HTTP API (`WebServer`), `RAMAccount.lua` | **Remove** | Exposes account actions and, in one code path, cookies to local callers. |
| Developer mode switches, credential-bearing "quick login" and share links | **Remove** | Credential exposure. |
| Account utilities (password, email, PIN, sessions, block, avatar, purchases) | **Remove** | Act as the user with the cookie; done on roblox.com instead. |
| Auto-updater, silent VC++ install, config self-repair | **Remove** | Unverified downloads run with elevation. RAM has no updater and makes no network request except optional OAuth checks. |
| Captcha solver, stealth plugin, proxies, script injection | **Remove** | Circumvent Roblox's verification. |
| Window position save/restore | **Rewrite** | Optional tiling of identified clients, with a one-click reset. |
| Themes (`RAMTheme.ini`) | **Rewrite** | System, Light, Dark and Lineage palettes, custom accent, compact density; high-contrast mode respected. Legacy theme files are not imported. |
| Settings (`RAMSettings.ini`) | **Rewrite** | One versioned JSON store with atomic saves. Only the saved place ID is imported. |
| Performance controls | **New** | Priority class, efficiency mode, optional Job Object CPU limit; off by default, Main exempt, fully reversible. |

## Stack

- **Rust (stable), three crates.** `ram-core` holds every rule and has no
  Windows dependency, so it is tested with in-memory fakes. `ram-win`
  implements the narrow platform traits (`ProcessApi`, `JobApi`, `WindowApi`,
  `BrowserApi`, `SecretStore`, `HttpApi`) with the `windows` crate and is
  tested against real child processes. `ram-app` is the Win32 shell.
- **Native Win32 controls.** The prototype was the Accounts view itself: a
  virtual list view, native edits and combo boxes, owner-drawn buttons.

  | Requirement | Result of the prototype |
  |---|---|
  | Table performance | Search + sort + repaint: 5–19 ms at 100 rows and at 10 000 rows. Selecting a row and repainting: 2.2 ms. The list is virtual, so it holds no per-row memory. |
  | Keyboard navigation | Standard dialog navigation (Tab, arrows, type-to-select in the list) plus an accelerator table. |
  | Accessibility | Native controls expose names, roles, values and selection to UI Automation without extra code. Focus is always visible, including on owner-drawn buttons. High-contrast mode switches to the system colors. |
  | High DPI | Per-monitor v2 manifest; fonts, metrics and row heights are recomputed on `WM_DPICHANGED`. |
  | Themes | Four palettes through documented messages (`WM_CTLCOLOR*`, list-view colors, custom draw) plus `SetWindowTheme` for dark scroll bars. |

  Native controls met every requirement, so Slint was not benchmarked. The
  one compromise is that dark scroll bars and combo boxes rely on the
  `DarkMode_Explorer` / `DarkMode_CFD` visual styles, which exist on
  Windows 10 1809 and later; on older builds those parts stay light.
- **No embedded browser, no service, no elevation.** The manifest requests
  `asInvoker`. Nothing runs when the window is closed.
- **Event-driven tracking.** Exits come from `RegisterWaitForSingleObject`;
  starts from a `SetWinEventHook` window-show hook filtered to the client's
  image name. A 3-second safety poll runs only while a launch is in flight.
  With no clients and nothing queued, no timer runs at all.
- **Secrets.** The only secret is an optional OAuth refresh token per
  account, kept in Windows Credential Manager. It is never written to the
  store or the log.

## Things deliberately not built

- No per-client FPS cap or graphics preset: Roblox persists these once per
  Windows user, so any change would also hit Main.
- No hard GPU cap: Windows has no per-process GPU limit.
- No memory trimming and no forced termination, in presets or elsewhere.
- No anti-AFK input. Idle reminders tell the user; the user plays.
- No telemetry, integrations or update checks.
