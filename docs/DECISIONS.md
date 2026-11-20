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

## The central change: no credentials in the manager

The legacy app stores each account's `.ROBLOSECURITY` cookie and password,
requests authentication tickets with them, and starts the client itself.
Everything else follows from that: the embedded browser, cookie refresh, the
local web API that can return cookies, the encryption modes.

Waypoint stores account names and notes only. Roblox handles the password,
two-step verification and CAPTCHA in the user's own browser, and the browser
starts the client. The cost is that the manager cannot see a browser session,
so it says what it knows and what it does not:

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
| Multi-Roblox (holding `ROBLOX_singletonMutex`) | **Remove** | Not a supported interface; Roblox states that running several clients is not officially supported. Waypoint does not defeat the single-instance check. Tracking, presets and tiling work with however many clients Roblox itself allows. |
| Process tracking through `-b <browserTrackerId>` on the command line (WMI) | **Rewrite** | The value was injected by the legacy launcher and no longer exists. Tracking now uses process snapshots, exit waits and a window-show hook, with evidence-based account links. |
| Bundled Sysinternals `handle.exe`, log-file tailing with remotely downloaded patterns | **Remove** | Redistribution and trust concerns; closes handles inside the client in some forks. |
| Roblox cookie-file lock (`RobloxCookies.dat` held open) | **Remove** | Deliberately breaks the client's own file. |
| FPS unlock by writing `ClientAppSettings.json` | **Remove** | The flag is no longer honored (Roblox restricted fast flags to an allow-list in September 2025) and it changes client files. The frame-rate cap is shown read-only as a global Roblox setting. |
| Auto-relaunch, Account Control, Nexus websocket, `Nexus.lua`, auto-execute | **Remove** | Remote control and script execution inside clients. |
| Local HTTP API (`WebServer`), `RAMAccount.lua` | **Remove** | Exposes account actions and, in one code path, cookies to local callers. |
| Developer mode switches, credential-bearing "quick login" and share links | **Remove** | Credential exposure. |
| Account utilities (password, email, PIN, sessions, block, avatar, purchases) | **Remove** | Act as the user with the cookie; done on roblox.com instead. |
| Auto-updater, silent VC++ install, config self-repair | **Remove** | Unverified downloads run with elevation. Waypoint has no updater and makes no network request except optional OAuth checks. |
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
