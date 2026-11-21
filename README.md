# RAM

A fast, good-looking Windows account manager for Roblox, built for everyday
Rogue Lineage use: quick switching between many accounts, saved server
destinations, honest sign-in status, an encrypted cookie vault, optional
multi-client launching, and reversible controls that keep background clients
from slowing down the account you are playing on.

RAM is a Rust rewrite of
[Roblox Account Manager](https://github.com/ic3w0lf22/Roblox-Account-Manager)
by ic3w0lf22 (see [NOTICE.md](NOTICE.md)). It is not affiliated with Roblox.

![RAM](crates/ram-app/assets/RAM-256.png)

![Accounts view](docs/screenshot-accounts.png)

## Highlights

- **Encrypted cookie vault.** Session cookies are encrypted at rest with
  Windows data protection (DPAPI) or a master password (Argon2id +
  AES-256-GCM) with auto-lock and a one-time recovery code. Revealing or
  copying a cookie is gated behind Windows Hello or the master password.
- **One-click launching.** With a stored cookie, RAM starts the client
  directly through a Roblox authentication ticket — no browser step. Without
  one, it hands off to your browser. Several clients can run at once.
- **Clean, themeable UI.** A compact accounts table with avatars, a details
  pane and a launch bar, in a **Modern** or a dense **Classic** layout.
  Themes include System, Light, Dark, the Catppuccin family (Mocha,
  Macchiato, Frappé, Latte), Nord, Dracula, Gruvbox, Rosé Pine and a muted
  Lineage palette, plus a custom accent and a compact density.
- **Honest status.** Sign-in and client status are separate and say only
  what was actually observed; a network problem never reads as "Signed out".
- **Import and export.** Preview-first import from Roblox Account Manager's
  `AccountData.json` (optionally into the vault), and export back out.
- **Small and quiet.** One executable, no background service, no admin
  rights, no telemetry, no updater.

## Getting started

1. Run `RAM.exe`.
2. Add an account with **Ctrl+N**, or import from Roblox Account Manager with
   **Ctrl+I** ([migration notes](docs/MIGRATION.md)).
3. (Recommended) In **Settings → Security**, set up the cookie vault.
4. Select an account and choose **Cookie → Set…** in the details pane to
   store its `.ROBLOSECURITY` cookie, or use **Sign in in browser**.
5. Pick a destination in **Launch to** (Rogue Lineage is pinned) and press
   **Enter**.

## The cookie vault

RAM can store each account's Roblox session cookie so it can launch and
verify without a browser. Cookies are never written to the plain data file
or the log.

- **Windows account (DPAPI).** Unlocks automatically for your Windows user;
  unreadable on any other machine or account. No password.
- **Master password.** The vault key is wrapped with a key derived from your
  password through Argon2id (64 MiB, 3 passes) and AES-256-GCM. The vault
  locks itself after idle time and must be unlocked with the password. A
  one-time **recovery code** can also unlock it if you forget the password.

Revealing or copying a raw cookie always asks for **Windows Hello** (or the
master password). Removing an account, or the vault, deletes its stored
cookies from the PC; your Roblox sessions are unaffected.

## Launching

- **Direct (default when a cookie is stored and the vault is unlocked).** RAM
  exchanges the cookie for a one-time Roblox authentication ticket and starts
  the client. If the cookie has expired, it falls back to the browser and
  marks the account signed out.
- **Browser hand-off.** Opens the join link in the account's browser profile
  (Chrome, Edge, Brave, Vivaldi or Firefox), and the browser starts Roblox.
- **Multi-instance (on by default).** RAM holds Roblox's single-instance
  lock so more than one client can run. This owns a named Windows object
  only; it does not modify or inject into the client. Roblox does not
  officially support running several clients — see the
  [support matrix](docs/SUPPORT_MATRIX.md).

## Accounts

The main screen is a table with a details pane, in two layouts (Settings →
Appearance → Accounts layout):

- **Modern** — avatars, accent selection, a counts-and-legend bar.
- **Classic** — a dense ONLINE / USERNAME / ALIAS / GROUP / MODE /
  DESCRIPTION table with inline Place/Job quick-launch, in the style of the
  original and of alyae.

Search matches username, alias, tags, notes and group; narrow it with
`tag:farm`, `group:alts`, `is:fav`, `is:main`, `is:running`. Pin one account
as **Main** (Ctrl+M): it sorts first, is excluded from Select All and batch
launches, and is never touched by automatic performance changes.

### Sign-in status

| Status | Meaning |
|---|---|
| Verified | Roblox confirmed this account (via the stored cookie, or OAuth) at the time shown. |
| Signed out | Roblox explicitly rejected the cookie or authorization. |
| Verification required | A verification step is pending; launches pause until you resume. |
| Check in browser | RAM opened a page in the browser and cannot see the result. |
| Unknown | Nothing known yet, e.g. right after an import. |

## Performance tools (optional, off by default)

Presets for background clients (**Balanced**, **Low Resource**), an optional
Job Object CPU cap, live CPU/RAM/GPU per client, advisory memory thresholds,
idle reminders, focus and tiling, and a one-click **Reset overrides**
(Ctrl+Shift+R). Main is exempt by default. RAM never trims memory or
force-closes a client. Per-client FPS/graphics and a hard GPU cap are not
offered, with the reasons shown in the app. The full list, with what has and
has not been tested against a live client, is in the
[support matrix](docs/SUPPORT_MATRIX.md).

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+1…4 | Accounts, Destinations, Performance, Settings |
| Ctrl+F | Search · Enter | Launch · Ctrl+L | Launch |
| Ctrl+N | Add · Ctrl+I | Import · Delete | Remove |
| Ctrl+A | Select all (Main excluded) · Ctrl+M | Main · Ctrl+D | Favorite |
| F5 | Check sign-in · F6 | Focus client · Ctrl+R | Rejoin |
| Ctrl+Shift+R | Reset performance overrides |

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
- [Legacy source audit](docs/LEGACY_AUDIT.md) · [Research notes](docs/RESEARCH_NOTES.md)

## Security notes

Storing session cookies is powerful and risky: anyone who can read a
decrypted cookie can sign in as that account. RAM encrypts them at rest and
gates reveal/copy behind Windows Hello or the master password, but it cannot
protect against malware already running as your Windows user. Use the master
password mode on shared machines, keep your recovery code offline, and
remove cookies you no longer need.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md). Roblox
and Rogue Lineage are trademarks of their owners; RAM is unaffiliated and
does not modify the Roblox client.
