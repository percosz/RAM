# RAM

A fast, clean Windows account manager for Roblox, built for everyday Rogue
Lineage use: switch between many accounts, join a place or follow a friend
with one click, keep every session cookie in an encrypted vault, open your
own browser already signed in as any account, run several clients at once
and keep the alts light on your PC.

RAM is a Rust rewrite of
[Roblox Account Manager](https://github.com/ic3w0lf22/Roblox-Account-Manager)
by ic3w0lf22, with a React front-end in a Tauri 2 shell. Its login check,
performance model and crash history follow RoALT, its browser-profile idea
follows niccs' RAM v4 and alyae, and its layout takes cues from both — see
[NOTICE.md](NOTICE.md). It is not affiliated with Roblox.

![Accounts](docs/screenshot-accounts.png)

## What it does

- **Accounts.** A searchable, groupable list with Roblox headshots, the
  username first and your alias and description underneath, a Valid/Invalid
  badge, Robux and Premium. Tick boxes on rows and group headers pick any set
  of accounts; the details pane edits alias, group and description.
- **Launch.** Type a **Place ID** (Rogue Lineage is pre-filled; the bar shows
  the place's name) and optional **Job ID**, tick accounts, press **Join
  Server**. Or type a **username** and **Follow** them into their game.
  Several accounts launch one by one, spaced by a delay you set. Launching
  uses a one-time Roblox authentication ticket from the stored cookie.
- **Open Browser.** Opens roblox.com in your default browser (Brave, Chrome,
  Edge, Vivaldi) already signed in as that account, each account in its own
  browser profile. With no cookie stored yet it shows the sign-in page: sign
  in there and press **Capture session**. The profile's cookies are wiped when
  the window closes; the vault stays the only store.
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
- **Performance for alts.** An FPS cap, lowest graphics and lighter rendering
  (MSAA, texture quality, rendering mode, gray sky, no grass, lowest render
  quality, paused lighting) plus your own FastFlags, written just before each
  alt client starts and put back once it has read them. Per-account FPS cap
  and *Full quality*; the **Main** account is never touched. Works through
  Bloxstrap-style launchers (Froststrap, Fishstrap, Voidstrap) as well as
  Roblox's own.
- **Diagnostics.** A crash history that says how each client ended
  (disconnected, crashed, replaced, sign-in refused) with the reason from its
  log, and RAM's own log with level filters and a redacted "Copy diagnostics".
- **Import / export.** Preview-first import from Roblox Account Manager's
  `AccountData.json`, moving its cookies into the vault (also for accounts you
  already have); export back out with or without cookies.
- **Themes and rows.** System, Light, Dark, Catppuccin (Mocha, Macchiato,
  Frappé, Latte), Nord, Dracula, Gruvbox, Rosé Pine and a muted Lineage
  palette, an accent colour, corner radius, and four row sizes from Compact
  (one line) to Large.
- **Updates.** RAM checks GitHub Releases quietly and offers a new version in
  the sidebar; it installs only when you click, and the installer must carry
  RAM's release signature. "What's new" shows once after an update.

![Diagnostics](docs/screenshot-diagnostics.png)

## Getting started

1. Run the installer from the latest release (or unzip the portable build).
   Windows 10 1809+ with the WebView2 runtime, which Windows 11 and current
   Windows 10 already have.
2. **Settings → Security → Set up vault** (Windows account or master password).
3. **Add** an account, then **Open Browser**, sign in, and **Capture session**.
   Or paste a cookie with **Set cookie…**. Or **Import** from Roblox Account
   Manager with the cookie switch on.
4. Mark your own account as **Main** (details pane), then turn on
   **Settings → Performance** if you want light alts.
5. Tick accounts, keep the Place ID (Rogue Lineage) or type another, press
   **Join Server**.

## Keyboard

| Keys | Action |
|---|---|
| Tick box | Add or remove one account from the selection |
| Click / Ctrl+click / Shift+click | Select / toggle / range-select accounts |
| Ctrl+A | Select all (Main excluded) |
| Enter | Join Server with the selected accounts |
| Double-click | Join Server with that account |
| Delete | Remove the selected accounts |
| Esc | Close a dialog |

## Security notes

A stored session cookie is as powerful as a password. RAM encrypts cookies at
rest and gates reveal/copy behind Windows Hello or the master password, but it
cannot protect against malware already running as your Windows user. Use the
master-password vault on shared machines, keep the recovery code offline, and
remove cookies you no longer need.

Open Browser drives the browser through Chrome's DevTools protocol on a
loopback port that only exists while that window is open; any program running
as you could talk to that window during that time. Close it when you are done.
Running several clients at once is not officially supported by Roblox; the
performance settings use the same files Roblox and Bloxstrap use (Roblox's own
settings XML and `ClientAppSettings.json`) and are restored after each launch.

## Building

See [docs/BUILD.md](docs/BUILD.md). In short:

```powershell
.\scripts\package.ps1
```

## Documentation

- [Changelog](CHANGELOG.md) · [Design decisions](docs/DECISIONS.md) · [Reference code study](docs/REFERENCE_CODE_STUDY.md)
- [Migration notes](docs/MIGRATION.md) · [Support matrix](docs/SUPPORT_MATRIX.md) · [Benchmarks](docs/BENCHMARKS.md)
- [Legacy source audit](docs/LEGACY_AUDIT.md) · [Research notes](docs/RESEARCH_NOTES.md)

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md). Roblox
and Rogue Lineage are trademarks of their owners; RAM is unaffiliated and does
not modify the Roblox client.
