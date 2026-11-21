# Changelog

RAM follows [Keep a Changelog](https://keepachangelog.com/) and semantic
versions. The section for the running version is what "What's new" shows
after an update; the release body on GitHub is generated from it.

## 0.4.1 (2026-11-21)

### New
- **Update prompt on launch.** When a newer RAM is out, a dialog says so a moment after start, with the release notes and two choices: **Update now** or **Later**. Later keeps the "available" button in the sidebar for the rest of the session.

### Fixed
- **The sign-in queue types the login for real.** It used to set the fields from page script, which Roblox's page often ignored or wiped while it was still loading, and Sign in was not pressed. RAM now waits until the page and the form are fully loaded, types the username and password through the browser's own input channel, checks that both stuck (and types again if the page cleared them), presses Enter, and clicks Sign in with a real mouse click if the form did not react.
- **The automatic update check often never ran.** Its timer was cancelled by the first refresh of the account list after start, so a new version was only found by pressing "Check now".
- **A client you closed yourself was recorded as a crash** when it was ended from outside the game window (Task Manager's End task, a bootstrapper or another tool closing it). Such a client stops without a closing line in its log, exactly like a crash; RAM now reads the process exit code and only records a crash for a real exception, a hang, or a crash dump. The exit code is written to the debug log.

## 0.4.0 (2026-11-21)

### New
- **Bulk add from `username:password`.** Paste lines into Add → Add many at once. RAM opens Roblox's sign-in page in your browser for one account at a time, fills it in and submits it; you solve any check, RAM saves the session and moves to the next. Skip and Stop buttons; closing the browser window stops the queue. Cookie lines in the same paste are still added straight away.
- **Saved passwords (optional).** Passwords from the queue, and passwords stored in an imported AccountData.json, are kept encrypted in the vault. Right-click → Copy → Password or username:password, behind Windows Hello or the master password. Each account shows how it was signed in: *cookie only*, *browser sign-in*, or *login + password*.
- **Import finds your file.** The Import dialog searches Desktop, Downloads, Documents, your user folder, OneDrive and other drives for `AccountData.json` and its backups (`.backup`, `.bak`, `.old`), and you can pick a file directly.
- **Password-protected AccountData.json.** Enter the password you set in Roblox Account Manager and RAM unlocks the file for the import. The file is not changed and the password is not stored.
- **Running clients** (Settings): minimize on launch, start clients tiny, mute on launch, a CPU limit and efficiency mode for clients in the background, memory trim on a schedule, and a bandwidth limit (a Windows network policy; asks for administrator approval). Never applied to Main or Full quality accounts.
- **Volume 0** for alt clients (Roblox's own volume, restored afterwards).
- **Minimize all / Restore all** in Clients.
- **Browser choice** in Settings → Launching: follow the Windows default, or pin Brave, Chrome, Edge or Vivaldi.
- New app icon.

### Fixed
- A changed default browser was ignored on newer Windows 11 builds, which record the choice in a new registry key. A profile created by another browser is reset when the browser changes.
- Accounts stayed on "Client starting…" after the client was running or closed. The line now clears when the client is seen, when it exits, and after two minutes at most.

### Changed
- Performance starts at **Roblox defaults (off)**; the three presets are a selector that shows which one is active.

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
