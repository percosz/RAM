# RAM

A  very very very fast, clean Windows account manager for Roblox, built for everyday Rogue
Lineage use: switch between many accounts, join a place or follow a friend
with one click, keep every session cookie in an encrypted vault, open your
own browser already signed in as any account, run several clients at once,
and keep the alts light on your PC.


this is vibe coded!!!!!!!!! if you dont like it then i recomend using ic3wolfs account manager. this project aims to add a lot of qol while being faithful to the original. this is pretty decent for 23s and just regular use, i didnt like the old account manager because its ugly

[![Latest release](https://img.shields.io/github/v/release/percosz/RAM?label=download&color=cba6f7)](https://github.com/percosz/RAM/releases/latest)
[![Build](https://github.com/percosz/RAM/actions/workflows/release.yml/badge.svg)](https://github.com/percosz/RAM/actions/workflows/release.yml)
[![Licence](https://img.shields.io/badge/licence-GPL--3.0-blue)](LICENSE)

![Accounts](docs/screenshot-accounts.png)

## Download

Go to the [latest release](https://github.com/percosz/RAM/releases/latest):

| File | Use it when |
|---|---|
| `RAM_<version>_x64-setup.exe` | You want it installed and kept up to date automatically. |
| `RAM-<version>-x64-portable.zip` | You want a single `RAM.exe` in a folder of your choice. It does not update itself. |

Windows 10 1809 or later with the WebView2 runtime (already part of Windows
11 and current Windows 10). Windows SmartScreen may warn the first time
because the installer is not code-signed with a paid certificate; the
auto-updater verifies every update against RAM's own release signature.

## What it does

- **Accounts.** A searchable, groupable list with Roblox headshots, the
  username first and your alias and description underneath, a Valid/Invalid
  badge, Robux and Premium. Tick boxes on rows and group headers pick any set
  of accounts; the details pane edits alias, group and description. Drag rows
  to reorder, right-click for actions, and hide usernames with one click for
  screenshots.
- **Launch.** Type a **Place ID** (Rogue Lineage is pre-filled; the bar shows
  the place's name) and optional **Job ID**, tick accounts, press **Join
  Server**. Or type a **username** and **Follow** them into their game.
  Several accounts launch one by one, spaced by a delay you set. Launching
  uses a one-time Roblox authentication ticket from the stored cookie, through
  whatever handles `roblox-player:` links on your PC (Roblox itself, or
  Bloxstrap, Froststrap, Fishstrap, Voidstrap).
- **Open Browser.** Opens roblox.com in your default browser (Brave, Chrome,
  Edge, Vivaldi) already signed in as that account, each account in its own
  browser profile. With no cookie stored yet it shows the sign-in page: sign
  in there and RAM saves the session by itself. The profile's cookies are
  wiped when the window closes; the vault stays the only store.
- **Login check.** The RoALT method: `users.roblox.com/v1/users/authenticated`
  with the cookie. 401 means invalid; a "bound session" or verification
  challenge is reported as such; rate limits and network errors never change
  what you last knew. Rotated cookies are saved automatically.
- **Vault.** Cookies are encrypted at rest with Windows data protection
  (DPAPI) or a master password (Argon2id + AES‑256‑GCM) with auto-lock and a
  one-time recovery code. Revealing a cookie asks for Windows Hello or the
  master password.
- **Multi-instance.** On by default: RAM holds Roblox's single-instance lock
  so more than one client can run. It owns a named Windows object only; it
  does not modify or inject into the client.
- **Add accounts your way.** Sign in with Roblox in your own browser, paste a
  cookie, or paste a list: cookies are added at once and `username:password`
  lines are signed in one at a time through Roblox's own sign-in page, filled
  in for you. Passwords can be kept in the vault (encrypted) and copied later.
- **Performance for alts.** An FPS cap, lowest graphics and lighter rendering
  (MSAA, texture quality, rendering mode, gray sky, no grass, lowest render
  quality, paused lighting) plus your own FastFlags, written just before each
  alt client starts and put back once it has read them. Per-account FPS cap
  and *Full quality*; the **Main** account is never touched. Presets: Roblox
  defaults (the starting point), Balanced, Max alts. Volume 0 is an option.
- **Running clients.** Minimize on launch, tiny windows, mute, a background
  CPU limit, efficiency mode, memory trim and a bandwidth limit, for alts
  only; Minimize all / Restore all with one click.
- **Diagnostics.** A crash history that says how each client ended
  (disconnected, crashed, replaced, sign-in refused) with the reason from its
  log, and RAM's own log with level filters and a redacted "Copy diagnostics".
- **Import / export.** RAM finds Roblox Account Manager's `AccountData.json`
  (or its backup) on your PC, unlocks it if it has a password, shows a
  preview, and moves its cookies and saved passwords into the vault; export
  back out with or without cookies.
- **Themes and rows.** System, Light, Dark, Catppuccin (Mocha, Macchiato,
  Frappé, Latte), Nord, Dracula, Gruvbox, Rosé Pine and a muted Lineage
  palette, an accent colour, corner radius, and four row sizes from Compact
  (one line) to Large.
- **Tutorial.** A one-minute tour on first launch shows where everything is;
  repeat it from the bottom of Settings.
- **Updates.** RAM checks GitHub Releases, announces a new version when it
  starts, and keeps the offer in the sidebar; it installs only when you
  click, and the installer must carry RAM's release signature. "What's new"
  shows once after an update.

<details>
<summary>More screenshots</summary>

![Settings](docs/screenshot-settings.png)

![Diagnostics](docs/screenshot-diagnostics.png)

</details>

## Getting started

1. Install RAM (or unzip the portable build) and run it.
2. **Settings → Security → Set up vault** (Windows account or master password).
3. **Add** an account: **Sign in with Roblox** (your browser opens, RAM saves
   the session), paste a cookie, or paste many at once. Or **Import** from
   Roblox Account Manager with the cookie switch on.
4. Mark your own account as **Main** (details pane), then turn on
   **Settings → Performance** if you want light alts.
5. Tick accounts, keep the Place ID (Rogue Lineage) or type another, press
   **Join Server**.

## Keyboard

| Keys | Action |
|---|---|
| Tick box | Add or remove one account from the selection |
| Right-click | Actions for that account or the selection: copy, paste cookie, groups, Main, remove |
| Drag a row | Reorder accounts; drop into another group to move it there |
| Click / Ctrl+click / Shift+click | Select / toggle / range-select accounts |
| Ctrl+A | Select all (Main excluded) |
| Enter | Join Server with the selected accounts |
| Double-click | Join Server with that account |
| Delete | Remove the selected accounts |
| Esc | Close a dialog |

## Privacy and security

- RAM talks only to Roblox (login check, tickets, headshots, place names)
  and, for update checks, to GitHub. There is no telemetry.
- Everything stays in `%LOCALAPPDATA%\RAM`: `store.json` (accounts, settings,
  crash history), `vault.bin` (encrypted cookies), `browser-profiles\` and
  `RAM.log`. Cookies never reach the log or the plain data file.
- A stored session cookie is as powerful as a password. RAM encrypts cookies
  (and any passwords you choose to save) at rest and gates copying them behind
  Windows Hello or the master password, but it cannot protect against malware
  already running as your Windows user. Passwords are only ever typed into
  Roblox's own sign-in page in your browser; turn "Save passwords" off and
  they are forgotten as soon as the form is filled. Use
  the master-password vault on shared machines and keep the recovery code
  offline.
- Open Browser drives the browser through Chrome's DevTools protocol on a
  loopback port that only exists while that window is open; any program
  running as you could talk to that window during that time. Close it when
  you are done.
- Running several clients at once is not officially supported by Roblox. The
  performance settings use the same files Roblox and Bloxstrap use (Roblox's
  own settings XML and `ClientAppSettings.json`) and are restored after each
  launch.

## Building

See [docs/BUILD.md](docs/BUILD.md). In short:

```powershell
.\scripts\package.ps1
```

Releases are built and signed by [the Release workflow](.github/workflows/release.yml)
when a version tag is pushed.

## Documentation

- [Changelog](CHANGELOG.md) · [Design decisions](docs/DECISIONS.md) · [Reference code study](docs/REFERENCE_CODE_STUDY.md)
- [Migration notes](docs/MIGRATION.md) · [Support matrix](docs/SUPPORT_MATRIX.md) · [Benchmarks](docs/BENCHMARKS.md)
- [Legacy source audit](docs/LEGACY_AUDIT.md) · [Research notes](docs/RESEARCH_NOTES.md)

## Credits and licence

RAM is a Rust rewrite of
[Roblox Account Manager](https://github.com/ic3w0lf22/Roblox-Account-Manager)
by ic3w0lf22, with a React front-end in a Tauri 2 shell. Its login check,
performance model and crash history follow [RoALT](https://github.com/inspectsoftware/RoALT),
its browser-profile idea follows [niccs' RAM v4](https://github.com/niccsprojects/Roblox-Account-Manager)
and [alyae](https://github.com/vysaely/alyae), and its layout takes cues from
both. See [NOTICE.md](NOTICE.md) for what was taken from where.

GPL-3.0-or-later. See [LICENSE](LICENSE). Roblox and Rogue Lineage are
trademarks of their owners; RAM is not affiliated with either and does not
modify the Roblox client.
