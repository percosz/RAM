# Changelog

RAM follows [Keep a Changelog](https://keepachangelog.com/) and semantic
versions. The section for the running version is what "What's new" shows
after an update; the release body on GitHub is generated from it.

## 0.3.2 (2026-11-21)

### Fixed
- **Start-up took up to 30 seconds.** RAM probed every browser profile for a leftover DevTools port before showing its window, and Windows takes two seconds to refuse each dead port. The probe is now short, cleans up the leftover file, and runs after the window is up. Measured on a real data folder with seven stale profiles: 28.8 s before, 0.65 s after.

### New
- **Right-click an account** (or a selection): Join Server, Open Browser, Check sign-in, Copy (username, alias, user ID, profile link, cookie), Paste cookie, Set as Main, Favorite, Full quality, Move to group, Sort all by username, Remove cookie, Remove account.
- **Drag accounts to reorder them.** Drag one row or a ticked selection; dropping into another group moves it there.
- **Hide usernames** (Settings → Appearance, or the eye button in the toolbar): shows only the first three characters, for screenshots and streams.
- A sign-in done in an account's browser window, or a cookie Roblox renews there, is saved to the vault by itself.

### Changed
- The details pane keeps Open Browser and Check; Reveal cookie, Capture session and Set cookie are gone (use right-click → Copy → Cookie, and Paste cookie). A pasted cookie is checked with Roblox and refused if it belongs to a different account.
- The webview's own right-click menu (Back, Refresh, Print) no longer appears.

## 0.3.1 (2026-11-21)

### Fixed
- **Reveal cookie froze RAM.** The Windows Hello prompt was shown from the UI thread, which it needs for its own message loop; it now runs on a worker thread and is attached to RAM's window the way desktop apps must.
- Multi-instance no longer gives up when another program holds `ROBLOX_singletonEvent` as a different kind of object (Bloxstrap-style launchers); the mutex, which is what the client checks, is still held.
- Faster start and lighter checks: the Windows Hello availability query is cached, a batch check takes one snapshot instead of one per account, and the window opens dark instead of flashing white.

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
