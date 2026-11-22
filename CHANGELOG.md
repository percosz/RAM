# Changelog

RAM follows [Keep a Changelog](https://keepachangelog.com/) and semantic
versions. The section for the running version is what "What's new" shows
after an update; the release body on GitHub is generated from it.

## 0.6.2 (2026-11-22)

### Changed
- **In game / online updates every 15 seconds** instead of once a minute, and the interval is a setting (Settings → Launching → Check who is in game every: 10, 15, 30 or 60 seconds). If Roblox asks RAM to slow down, RAM waits longer by itself and speeds back up afterwards.

## 0.6.1 (2026-11-22)

### New
- **Who is on an account right now.** RAM asks Roblox about every account once a minute, each with its own cookie, and shows it in the list: **in game** (accent when it is a client RAM started on this PC, amber when it is somewhere else: another device, or started outside RAM), **online**, or **studio**. A dot on the avatar shows the same thing at any list width, and the details pane names the game when Roblox shares it.
- **A warning before launching an account that is already playing.** Join Server and Follow ask Roblox again at that moment. If an account is in a game, RAM says which and where, and offers **Launch anyway**, **Skip it** (launch the others) or **Cancel**. Launching signs the account in again, which disconnects the session that is already playing. Auto-rejoin does not ask.
- **Last online instead of last launched here.** The time in each row is now when the account was last seen online. Roblox has removed its last-online lookup, so RAM records this itself from the presence checks and from clients running here; it is accurate from now on for as long as RAM is running, and falls back to the last launch from RAM until then.
- **Draggable dividers.** Drag the line between the sidebar and the list, or between the list and the details pane, to resize them. The widths are remembered; double-click a divider for the default.

## 0.6.0 (2026-11-22)

### New
- **Auto-rejoin** (Settings → Launching, off by default). When an alt's client is disconnected by Roblox or crashes, RAM launches it again the way it last joined: the same server, the same place, or the user it followed. The first attempt repeats that exactly; if it does not get the account back into a game within two minutes, later attempts go to the place instead (a server can be gone, a followed user can have left). Attempts are spaced out (10 s, 30 s, 90 s, longer) and capped at the number you pick; a client that stays up for 10 minutes resets the count. It never rejoins a client you closed yourself, one that was replaced by another sign-in, a refused sign-in, or Main.
- **Saved places.** The bookmark button next to the Place ID keeps a list of places: pick one to fill the box, or save the place you have typed.
- **Scheduled sign-in check** (Settings → Launching): re-checks every stored cookie every 1, 3, 6 or 12 hours and tells you which accounts need signing in again.
- **Backup and restore** (Settings → Data). One file with your accounts, settings, cookies and saved passwords, encrypted with a password you choose, restorable on any PC. Making one asks for Windows Hello or your master password, the same as copying a cookie. Restoring replaces what RAM holds and keeps the replaced data file as `store.before-restore.json`.
- **Window backdrop** (Settings → Appearance): **Mica** (Windows 11) or **Acrylic** lets your desktop show through the window, tinted by the theme. It takes effect when RAM starts again; with it off, the window is created exactly as before.
- **Ten more themes**, now in three groups. Palettes: One Dark, Solarized Dark, Solarized Light, Monokai, Everforest, Sakura. Looks: Ocean, Sunset, Frutiger Aero (glossy aqua and green), Terminal (green on black, monospace, scan lines).
- The window's title bar is dark or light to match the theme.

### Changed
- The theme picker is split into Basics, Palettes and Looks.

## 0.5.0 (2026-11-22)

### New
- **Seven more themes.** **Windows XP** (Luna blue side pane, green primary buttons, blue title bars on dialogs), **Windows Vista** (Aero glass over a soft aurora), **Classic (95)** (grey, bevelled, navy title bars), **Aurora** (see-through panels over a colour wash), **Synthwave** (neon on violet), **Tokyo Night** and **Midnight** (pure black). Settings → Appearance.
- **A bit prettier everywhere.** Dialogs blur what is behind them, menus are translucent, cards have a soft shadow, primary buttons a slight gloss, and tabs fade in when you switch. It is all plain styling with no images or running animations, so start-up time and memory use are unchanged.
- **"What's new" after every update.** The first start after an update shows the notes of every version since the one you had, not just the latest.
- **Hide names hides more.** The eye button (or Settings → Appearance → Hide names) now also masks aliases, swaps avatars for initials, and hides Robux and user IDs; the alias box shows dots.

### Fixed
- **Ending a client with End task was still recorded as a crash.** The exit code was read correctly, but the check for a crash dump counted any file Roblox touched in its crash folder, and Roblox rewrites a small settings file there on every run. Only real dump files (`.dmp`) count now.
- **"What's new" never appeared.** RAM only recorded the version you had seen when you closed that dialog, so it had nothing to compare against and never opened it.

### Changed
- The crash history only records clients that belong to an account in RAM. A client started outside RAM, or for an account that is not in your list, is still written to the debug log but no longer shows up as a crash.

## 0.4.2 (2026-11-21)

### New
- **Tutorial.** The first time RAM starts, a one-minute tour points at each part of the app in turn: adding and importing accounts, the list, joining a game, an account's details, Clients, Diagnostics, performance settings and the vault. Skip it at any step. **Settings → Tutorial** (at the very bottom) repeats it. If you already use RAM, it shows once after this update.

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
