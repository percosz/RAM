# Changelog

RAM follows [Keep a Changelog](https://keepachangelog.com/) and semantic
versions. The section for the running version is what "What's new" shows
after an update; the release body on GitHub is generated from it.

## 0.3.0 (2026-11-21)

### New
- **Add an account** the RoALT way: *Sign in with Roblox* opens your browser on Roblox's sign-in page and RAM captures the session by itself; *Paste a cookie*; *Add many at once* (cookies one per line, `user:pass:cookie` lines keep the cookie). The username comes from Roblox. The vault can be set up with one click from the dialog.
- Roblox headshots in the account list and details (public thumbnails API, refreshed daily).
- **Open Browser** opens your default browser (Brave, Chrome, Edge, Vivaldi) signed in as the account, each account in its own browser profile. The profile's cookies are wiped when the window closes; the vault stays the only store.
- Tick boxes on every row and group header: pick any set of accounts without holding Shift or Ctrl.
- The launch bar shows the name of the place you typed, or says when the ID is not a Roblox place.
- Row sizes: Compact (one line), Normal, Comfortable, Large.
- **Performance** (Settings): FPS cap, lowest graphics, lighter rendering (MSAA, texture quality, rendering mode, gray sky, no grass, lowest render quality, paused lighting) and custom FastFlags for alt clients, applied just before each launch and put back once the client has read them. Per-account FPS cap and Full quality; the Main account is never touched. Presets: Max alts, Balanced, Roblox defaults.
- **Diagnostics**: crash history (how each client ended, with the reason from its log, CSV export, open the log) and RAM's own log with level filters and a redacted "Copy diagnostics".
- Auto-update from GitHub Releases: offered in the sidebar, installed on your click, "What's new" shown once after.

### Changed
- Rows show the Roblox username first; the alias and description sit underneath and are edited in the details pane.
- Import can move the cookies of accounts that are already in RAM (re-run the import with the cookie switch on).
- The Classic layout is gone.
- RAM.log now carries dates and levels.

## 0.2.0 (2026-11-21)

- Tauri 2 + React shell; Place ID / Job ID / Follow launching; RoALT-style login check; browser profiles; import/export; cookie vault; themes.
