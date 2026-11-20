# Waypoint

A Windows account manager for Roblox, built for everyday Rogue Lineage use:
fast account switching, saved server destinations, honest sign-in status,
and optional controls that keep background clients from slowing down the
account you are playing on.

Waypoint is a Rust rewrite of
[Roblox Account Manager](https://github.com/ic3w0lf22/Roblox-Account-Manager)
by ic3w0lf22 (see [NOTICE.md](NOTICE.md)). It is not affiliated with Roblox.

## How it differs from the original

- **No passwords or cookies.** You sign in on roblox.com in your own
  browser. Roblox handles the password, two-step verification and CAPTCHA.
  Waypoint stores account names, notes and destinations.
- **The browser launches the game.** Each account can be tied to a named
  browser profile (Chrome, Edge, Brave, Vivaldi, Firefox). Launching opens
  the join link in that profile, and the browser starts Roblox.
- **Status you can trust.** Sign-in status and client status are separate,
  and each says only what was actually observed. A network problem never
  shows up as "Signed out".
- **Small and quiet.** One executable, no embedded browser, no background
  service, no administrator rights, no telemetry, no updater.

## Getting started

1. Run `waypoint.exe`.
2. Add an account with **Ctrl+N**, or import from Roblox Account Manager
   with **Ctrl+I** ([migration notes](docs/MIGRATION.md)).
3. In the details pane choose the account's **Browser** and **Profile**,
   then **Sign in in browser**.
4. Pick a destination in **Launch to** (Rogue Lineage is pinned) and press
   **Enter**.

## Accounts

The main screen is a compact table with a details pane.

- **Search** matches username, alias, tags, notes and group. Narrow it with
  `tag:farm`, `group:alts`, `user:name`, `note:text`, `is:fav`, `is:main`,
  `is:running`.
- **Main.** Pin one account as Main (**Ctrl+M**). Main sorts first, is left
  out of Select All and batch launches, and is never touched by automatic
  performance changes. Both exemptions can be changed in settings.
- **Sign-in status**: *Verified*, *Signed out*, *Verification required*,
  *CAPTCHA required*, *Check in browser*, *Unknown*, with the time of the
  last check and one recovery action.
- **Client status**: *Queued*, *Launching*, *Running*, *Unconfirmed*,
  *Paused*, *Exited*, *Failed*.

### What the statuses mean

| Status | Meaning |
|---|---|
| Verified | Roblox confirmed this account through its OAuth sign-in at the time shown. Needs the optional setup in Settings. |
| Signed out | Roblox explicitly rejected the saved authorization. |
| Verification required | You (or Roblox) reported that a verification step is pending. Launches for the account pause until you choose **Resume**. |
| CAPTCHA required | Shown only when an explicit signal names a CAPTCHA. |
| Check in browser | Waypoint opened a page in the browser and cannot see the result. |
| Unknown | Nothing is known yet, for example right after an import. |
| Running | A client process is linked to the account. The details pane says how: matched by launch timing, or assigned by you. |
| Unconfirmed | A client started, but more than one launch could explain it. Assign it in Performance. |

## Destinations

- **Rogue Lineage** is built in and pinned. Other games work the same way.
- Paste a Roblox link or a place ID to save a named shortcut. Waypoint
  validates it and tells you what kind it is:
  - **Saved** — the experience itself; always joinable.
  - **Saved link** — a private-server or share link; it stops working if
    its owner regenerates it.
  - **Live server** — one specific running server; it may have closed.
- Recent launches are kept per account for a one-click repeat.

Private-server links and share codes are never shown in lists, logs or
diagnostics.

## Performance tools (optional)

Everything is off until you turn it on, and **Reset overrides**
(**Ctrl+Shift+R**) puts every client back the way it was.

- Presets for background clients: **Off**, **Balanced** (below-normal
  priority), **Low Resource** (low priority and Windows efficiency mode).
  Per-account overrides are in the details pane.
- Optional hard CPU limit through a Windows Job Object.
- Live CPU, working set, private memory and GPU use per client, where
  Windows provides them.
- Advisory memory thresholds, with a one-click action to close a selected
  background client. Waypoint never trims memory or force-closes a client.
- Idle reminders, focus-client shortcut and tiling for identified clients.

Limits that are stated in the app rather than worked around:

- Roblox keeps one frame-rate cap and one graphics setting per Windows
  user, so there is no per-client FPS or graphics control. The global cap
  is shown and left alone.
- Windows has no per-process GPU cap.
- Roblox does not officially support running several clients at once, and
  Waypoint does not bypass that. It manages however many clients Roblox
  allows.

The full list, with what has and has not been tested against a live client,
is in the [support matrix](docs/SUPPORT_MATRIX.md).

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+1 … Ctrl+4 | Accounts, Destinations, Performance, Settings |
| Ctrl+F | Search accounts |
| Enter | Launch the selected accounts to the chosen destination |
| Ctrl+L | Launch |
| Esc | Clear the search; with an empty search, cancel queued launches |
| Ctrl+N | Add account |
| Ctrl+I | Import |
| Delete | Remove the selected accounts or destination |
| Ctrl+A | Select all (Main is left out) |
| Ctrl+M | Pin or unpin Main |
| Ctrl+D | Favorite |
| F5 | Check sign-in |
| F6 | Focus the selected account's client |
| Ctrl+R | Rejoin |
| Ctrl+Shift+R | Reset performance overrides |
| Tab / Shift+Tab | Move between controls |

## Verified sign-in (optional)

Without setup, sign-in status stays at *Check in browser* because Waypoint
does not read browser sessions. To let Roblox confirm an account:

1. In the Roblox Creator Dashboard create an OAuth 2.0 app with the scopes
   `openid` and `profile` and the redirect URL shown in Settings
   (`http://localhost:53682/callback` by default).
2. Paste the app's client ID in Settings.
3. Select an account and choose **Authorize**. Roblox asks for consent in
   the account's browser profile.

The authorization only identifies the account. It cannot launch games or
act as the user. It is kept in Windows Credential Manager.

## Building

See [docs/BUILD.md](docs/BUILD.md). In short:

```powershell
cargo build --release -p ram-app
```

```powershell
.\scripts\package.ps1
```

## Documentation

- [Design decisions](docs/DECISIONS.md) — what was kept, rewritten, removed
- [Migration notes](docs/MIGRATION.md)
- [Support matrix](docs/SUPPORT_MATRIX.md)
- [Benchmarks](docs/BENCHMARKS.md)
- [Legacy source audit](docs/LEGACY_AUDIT.md)
- [Research notes](docs/RESEARCH_NOTES.md)

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
