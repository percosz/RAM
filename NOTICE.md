# Notices

RAM is free software, licensed under the GNU General Public License,
version 3 or (at your option) any later version. The full text is in
[LICENSE](LICENSE). It comes with no warranty.

## Origin

RAM is a fork and rewrite of **Roblox Account Manager** by ic3w0lf22
and contributors:

- Upstream: <https://github.com/ic3w0lf22/Roblox-Account-Manager> (GPL-3.0)
- Forked at commit `c2b13d146d5db609d499870145e8d2dff2506581` (master, 2024-07-18),
  two commits after the last stable release, tag `3.7.2`
  (`79f61f3351df61fb3774dfa854ab868954da5389`).

The upstream history is kept in this repository. The `rewrite` branch
replaces the C# sources with a new Rust implementation; no upstream code,
icons, theme files or Lua scripts are included in the shipped program. The
importer reads the upstream file formats and contains two byte strings that
identify them (the encrypted-file header and the data-protection entropy
value), taken from the upstream source.

The upstream README is preserved at
[docs/UPSTREAM_README.md](docs/UPSTREAM_README.md).

## Projects whose designs RAM follows

These projects were read closely and several of their approaches are
reimplemented here (no files were copied verbatim; the specific techniques
are credited below). Their licences are compatible with GPL-3.0.

| Project | Licence | What RAM takes from it |
|---|---|---|
| [inspectsoftware/RoALT](https://github.com/inspectsoftware/RoALT) | MIT | The login check: `GET users.roblox.com/v1/users/authenticated` with the cookie, treating 401 as invalid, 401 + Roblox error 9002/2 as "bound session required", 403 + `rblx-challenge-id` as a challenge, 429 and transport errors as inconclusive; cookie rotation from `Set-Cookie`; a browser user agent; a per-account browser profile whose session is cleared so the vault stays the only store; the token-based theme system with named presets and accent; headshot batching (`Completed` only, `rbxcdn` only); the performance model (FPS cap and quality through `GlobalBasicSettings_13.xml`, lighter rendering through FastFlags, written before each launch and restored once the client's log shows it read them, Full quality exemption, the three presets); client-log parsing for the crash history (join, disconnect and teleport markers, player-initiated reason codes); the updater's quiet offer and "Check now". |
| [vysaely/alyae](https://github.com/vysaely/alyae) | GPL-3.0 | The simple Place ID / Job ID / Follow launch deck; reading a session back after a manual sign-in; the one-list look that shaped RAM's rows. (Its embedded sign-in window, used in RAM v0.2, was replaced in v0.3.) |
| [niccsprojects/Roblox-Account-Manager](https://github.com/niccsprojects/Roblox-Account-Manager) (v4) | GPL-3.0 | Layout cues for the account list, details pane and status legend; username-to-id lookup for Follow; the "Open Browser" model: a real Chromium browser started with a per-account `--user-data-dir` and the cookie set over the DevTools protocol (RAM uses the user's installed browser instead of a downloaded Chrome). |
| [ericky0/rbx-account-manager](https://github.com/ericky0/rbx-account-manager) | GPL-3.0 | Workflow reference only. |

RoALT's name, logo and distinctive look are not reused; RAM has its own.

## Third-party components in the binary

| Crate | Licence | Use |
|---|---|---|
| [windows](https://crates.io/crates/windows) (Microsoft) | MIT OR Apache-2.0 | Win32 API bindings |
| [serde](https://crates.io/crates/serde), [serde_json](https://crates.io/crates/serde_json) and their dependencies (itoa, memchr, zmij or ryu) | MIT OR Apache-2.0 | Data file format |
| [argon2](https://crates.io/crates/argon2) | MIT OR Apache-2.0 | Master-password key derivation (vault) |
| [aes-gcm](https://crates.io/crates/aes-gcm) | Apache-2.0 OR MIT | Cookie encryption (vault) |
| [zeroize](https://crates.io/crates/zeroize), [rand](https://crates.io/crates/rand) | MIT OR Apache-2.0 | Key hygiene and randomness |
| [tauri](https://crates.io/crates/tauri) and its plugins (opener, dialog, clipboard-manager, updater) | MIT OR Apache-2.0 | Application shell, WebView2 host, signed updates |
| [React](https://react.dev) | MIT | Front-end |
| [Vite](https://vitejs.dev), [TypeScript](https://www.typescriptlang.org) | MIT, Apache-2.0 | Build-time only |
| [lucide-react](https://lucide.dev) | ISC | Icons |
| Colour palettes: [Catppuccin](https://github.com/catppuccin/catppuccin), [Nord](https://www.nordtheme.com), [Dracula](https://draculatheme.com), [Gruvbox](https://github.com/morhetz/gruvbox), [Rosé Pine](https://rosepinetheme.com) | MIT | Theme presets |

The exact versions are pinned in `Cargo.lock` and `app/package-lock.json`.

## Trademarks and affiliation

Roblox is a trademark of Roblox Corporation. Rogue Lineage is the work of
its developers. RAM is not affiliated with, endorsed by or sponsored by
either. It does not modify the Roblox client and never handles Roblox
passwords. It stores Roblox session cookies only when you choose to, encrypted
at rest with Windows data protection or a master password.
