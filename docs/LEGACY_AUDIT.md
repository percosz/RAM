# Legacy audit: Roblox Account Manager (ic3w0lf22)

Audit of the C# WinForms original, done by reading source only (nothing was built or run). All paths are relative to the repository root. `AM.cs` below abbreviates `RBX Alt Manager/AccountManager.cs`; `Classes/`, `Forms/`, `Nexus/` are under `RBX Alt Manager/`.

Statements marked **[external]** come from general knowledge of Windows / libsodium / Roblox and are not verifiable from this repository; verify them before relying on them.

## 1. Revision

| Item | Value |
| --- | --- |
| Upstream | `github.com/ic3w0lf22/Roblox-Account-Manager` |
| Audited revision | `c2b13d146d5db609d499870145e8d2dff2506581` (master HEAD, 2024-07-18) |
| Latest stable tag | `3.7.2` = `79f61f3351df61fb3774dfa854ab868954da5389` |
| HEAD vs 3.7.2 | HEAD is 2 commits ahead. Only file diff: one line in `RBX Alt Manager/Classes/Account.cs` (friend-request content-type fix; at HEAD the request adds `Content-Type: application/json`, `Classes/Account.cs:746`). |
| Target | .NET Framework 4.7.2, x86 preferred (`RBX Alt Manager/RBX Alt Manager.csproj:16,71-75`) |
| Assembly name | `Roblox Account Manager` (`RBX Alt Manager.csproj:15`); `AssemblyFileVersion` in source is `3.6.1.0` (`Properties/AssemblyInfo.cs:35`), which does not match the 3.7.2 tag. |

On-disk formats described here are therefore identical for 3.7.2 and HEAD.

## 2. License and attributions

- `LICENSE`: GNU General Public License v3 (29 June 2007). The About box states "created by ic3w0lf under the GNU GPLv3 license" (`AM.cs:2045-2046`). Copyright string: "Copyright (c) ic3 2023" (`Properties/AssemblyInfo.cs:12`).
- A Rust rewrite that reuses no code is not a derivative work merely by reading the same file formats. If any legacy code, the default theme text, icons, `Nexus.lua` or `RAMAccount.lua` is copied or translated, the result must be GPL-3.0 and must keep the upstream copyright notice. Recommended either way: credit the original project and author, and state clearly that the new app is unaffiliated.
- The repository contains no third-party NOTICE file. Attributions that exist only as code comments:
  - `Classes/IniFile.cs:9` — derived from `github.com/janerist/IniFile`.
  - `Classes/Utilities.cs:51` and `:387` — Stack Overflow snippets (`TryParseJson`, `DownloadAsync`).
  - `Forms/Updater.cs:223` — Stack Overflow snippet (`RunAsDesktopUser`).
  - `.github/workflows/download-button.yml:1` — modified `DenverCoder1/readme-download-button-action`.
  - Icon resource named `icons8-history-32.png` (Icons8 asset; Icons8 normally requires attribution **[external]**).
- Binaries committed to the repo and embedded as resources:
  - `RBX Alt Manager/Resources/handle.exe` — Microsoft Sysinternals Handle. Extracted at runtime as `handle.bin` (`Forms/ServerList.cs:857-858`, `Classes/RobloxWatcher.cs:30`). Sysinternals tools are under their own Microsoft licence and are not normally redistributable **[external]**; do not carry this into the rewrite.
  - `RBX Alt Manager/Resources/libsodium.dll` — libsodium (ISC licence **[external]**), extracted at startup (`Classes/Program.cs:158-159`).
  - `Launcher` (repo root) — a small .NET executable (`RAM-Launcher.exe`) that starts `Roblox Account Manager.exe`.
- NuGet dependencies (`RBX Alt Manager/packages.config`), relevant ones: CefSharp.Common 117.2.20 / CefSharp.WinForms 109.1.110 / cef.redist 109.1.11; PuppeteerSharp 10.0.0; PuppeteerExtraSharp 2.0.0; libsodium-net 0.10.0; Newtonsoft.Json 13.0.3 (+ Schema 3.0.15); RestSharp 110.2.0; ObjectListView.Official 2.9.1 (GPL-3.0 **[external]**); FCTB 2.16.24 (FastColoredTextBox); WebSocketSharp-netstandard 1.0.1; log4net 2.0.15; SharpZipLib 1.4.2; Yove.Proxy 1.1.1; Esendex.TokenBucket 1.0.7; WindowsAPICodePack-Core 1.1.2; Costura.Fody 5.7.0 / Fody 6.7.0 (assemblies merged into the exe, `FodyWeavers.xml`); assorted `Microsoft.Extensions.*` and `System.*` packages. COM reference: `IWshRuntimeLibrary` (`RBX Alt Manager.csproj:624`).
- None of these libraries need to be mentioned by a clean Rust rewrite unless it actually ships them.

## 3. Files the legacy app reads and writes

All data files are addressed through `Environment.CurrentDirectory` (process working directory), not the executable's directory. They coincide when the app is started normally. `Classes/Program.cs:89-92` uses the executable's parent directory for one existence check of `AccountData.json`, and the updater assumes the working directory is the install directory.

| File / directory | Purpose | Reference |
| --- | --- | --- |
| `AccountData.json` | Account list incl. cookies and passwords (three storage variants, section 4) | `AM.cs:243, 263-383` |
| `AccountData.json.backup` | Copy of the previous file contents, refreshed when older than 8 h; same variant as the file it copied | `AM.cs:367-371, 309-343` |
| `AccountData.json.bak` | Raw copy written when loading fails | `AM.cs:300` |
| `AccountData.json.old` | Previous file when a password-locked backup is restored | `AM.cs:315-322` |
| `RAMSettings.ini` | Settings (section 6) | `AM.cs:112, 785` |
| `RAMTheme.ini` | Theme (section 12) | `Classes/Program.cs:133-134`, `Forms/ThemeEditor.cs:98-163` |
| `RecentGames.json` | Recent places (section 5) | `AM.cs:244, 854-892` |
| `FavoriteGames.json` | Favourites incl. private-server links (section 5) | `Forms/ServerList.cs:141, 166-185, 475-485` |
| `AccountControlData.json` | Nexus/auto-relaunch per-account targets (section 5) | `Forms/AccountControl.cs:29, 59-91` |
| `CustomClientAppSettings.json` | User-supplied FastFlag file | `Forms/SettingsForm.cs:339-342` |
| `BrowserConfig.json`, `extension/`, `proxies.txt` | Puppeteer browser customisation, unpacked extension, proxy list | `Classes/AccountBrowser.cs:66-139, 473-481` |
| `NoEncryption.IUnderstandTheRisks.iautamor` | Marker file: store `AccountData.json` as plaintext | `AM.cs:377-378` |
| `no773fix.txt` | Marker file: do not lock Roblox's cookie file | `Classes/Program.cs:178-186` |
| `handle.bin`, `libsodium.dll`, `log4.config`, `<exe>.config`, `log.txt` | Extracted tools, config, log (5 MB rolling) | `Classes/Program.cs:138-159`, `log4.config` |
| `AccountDumps/<Username>.txt` | "Show details" dump (robux, email status, fields) | `AM.cs:2097-2130` |
| `Output/Output_*.txt`, `Nexus.lua` | Nexus output log, extracted loader script | `Forms/AccountControl.cs:128-142, 470-477` |
| `x86/` (CEF), `Update/`, `Update.zip`, `Auto Update.exe`, `AU.exe` | Downloaded CEF runtime and updater artefacts | `AM.cs:638-647, 1198-1247`, `Forms/Updater.cs` |

Outside the app directory it touches: `%LocalAppData%\Roblox\LocalStorage\RobloxCookies.dat` (held open exclusively), `%LocalAppData%\Roblox\logs\*` (read), `<Roblox version folder>\ClientSettings\ClientAppSettings.json` (written), `HKCU\...\Run` (startup entry), `HKCU\SOFTWARE\Sysinternals\Handle\EulaAccepted` (read; set indirectly by running handle), `%TEMP%\vcredist.tmp` (downloaded and executed).

## 4. Account storage (`AccountData.json`)

### 4.1 Variants and detection

Load order in `LoadAccounts` (`AM.cs:263-305`):

1. **Password mode.** If the first 64 bytes equal `Cryptography.RAMHeader` (`Classes/Cryptography.cs:10`, compared at `AM.cs:270-272`), the UI asks for a password. The header is the ASCII text `Roblox Account Manager created by ic3w0lf22 @ github.com .......` (64 bytes, hex `52 6F 62 6C 6F 78 20 41 63 63 6F 75 6E 74 20 4D 61 6E 61 67 65 72 20 63 72 65 61 74 65 64 20 62 79 20 69 63 33 77 30 6C 66 32 32 20 40 20 67 69 74 68 75 62 2E 63 6F 6D 20 2E 2E 2E 2E 2E 2E 2E`).
   Layout (`Classes/Cryptography.cs:12-47`): header (64) | Argon2 salt (16) | secretbox nonce (24) | secretbox ciphertext (MAC + data) to end of file.
   Key derivation: `key = PasswordHash.ArgonHashBinary(pw, salt, StrengthArgon.Moderate, 32)` where `pw = CryptoHash.Hash(passwordText)` (`AM.cs:422, 492`). In libsodium-net 0.10.0 `CryptoHash.Hash(string)` is SHA-512 of the UTF-8 text (64 bytes), the default Argon algorithm is Argon2i v1.3, and "Moderate" is opslimit 6 / memlimit 128 MiB **[external — confirm against libsodium-net 0.10.0 before implementing]**. Cipher is `crypto_secretbox` (XSalsa20-Poly1305). Minimum password length 4 (`AM.cs:424, 486`).
   The in-memory password hash is kept DPAPI-protected (CurrentUser, no entropy) so the file can be re-encrypted on save (`AM.cs:288, 373-374`).
2. **Default mode (DPAPI).** Otherwise it tries `ProtectedData.Unprotect(data, Entropy, CurrentUser)` (`AM.cs:294`). Saving uses `ProtectedData.Protect(json, Entropy, DataProtectionScope.LocalMachine)` (`AM.cs:380`). `Entropy` is 56 bytes of ASCII: `ROBLOX ACCOUNT MANAGER | :) | BROUGHT TO YOU BUY ic3w0lf` (`AM.cs:93`). A DPAPI blob starts with `01 00 00 00 D0 8C 9D DF 01 15 D1 11 8C 7A 00 C0 4F C2 97 EB` (version + provider GUID) **[external]**. Because the blob is LocalMachine-scoped, any account on the same PC can decrypt it; it cannot be decrypted on another PC.
3. **Plaintext.** If DPAPI throws `CryptographicException`, the bytes are parsed as UTF-8 JSON (`AM.cs:295-297`). Written without BOM when the marker file `NoEncryption.IUnderstandTheRisks.iautamor` exists (`AM.cs:377-378`). First non-whitespace byte is `[`.

The decrypted payload in all three variants is the same: a UTF-8 JSON array of `Account` objects produced by `JsonConvert.SerializeObject(AccountsList)` with default settings (`AM.cs:365`).

Notes:
- A file shorter than 64 bytes makes the header check throw (`AM.cs:270`); the legacy app does not handle that.
- `SaveAccounts` skips writing when the list is empty unless forced, and has a faulty rate-limit (`(DateTime.Now - startTime).Seconds < 2` tests the seconds component, so saves are skipped for two seconds of every minute of uptime) (`AM.cs:360`).

### 4.2 `Account` schema (`Classes/Account.cs:20-88`)

Serialised members (Newtonsoft default: public fields and public properties; names are case-exact):

| JSON name | Type | Class | Notes |
| --- | --- | --- | --- |
| `Valid` | bool | metadata | True when the cookie validated at add time (`:22, 102`) |
| `SecurityToken` | string | **SECRET** | `.ROBLOSECURITY` cookie value (`:23, 110`) |
| `Username` | string | metadata | Roblox username (`:24, 99`) |
| `LastUse` | string (ISO-8601 DateTime, local time with offset, up to 7 fractional digits) | metadata | Updated on CSRF fetch / add (`:25, 104, 153`) |
| `Alias` | string, max 50 chars | metadata | `:52-63` |
| `Description` | string, max 5000 chars | metadata | Free-form notes (`:64-75`) |
| `Password` | string, max 5000 chars | **SECRET** | Captured at login; may be empty (`:76-87`) |
| `Group` | string | metadata | Default `"Default"`; omitted when null (`:29`) |
| `UserID` | integer (int64) | metadata | `:30, 100` |
| `Fields` | object: string -> string | metadata (inspect values) | Arbitrary per-account key/values (`:31, 828-830`) |
| `LastAttemptedRefresh` | string (DateTime; `0001-01-01T00:00:00` when never) | metadata | Cookie auto-refresh bookkeeping (`:32`, `AM.cs:806-810`) |
| `BrowserTrackerID` | string or null | low-sensitivity identifier | Random numeric string used for PID matching (`:50, 507-512`) |

Not serialised (`[JsonIgnore]`): `PinUnlocked`, `TokenSet`, `LastAppLaunch`, `CSRFToken`, `Presence` (`:33-37`).

Known `Fields` keys used by the code: `SavedPlaceId`, `SavedJobId` (`AM.cs:1442-1443, 1862-1898, 2015-2016`), `NoCookieRefresh` (`AM.cs:806`), `Window_Position_X`, `Window_Position_Y`, `Window_Width`, `Window_Height` (`Classes/RobloxWatcher.cs:99-102`, `Classes/Account.cs:685`). Any other key is user- or script-defined (Fields editor in dev mode, WebServer `SetField`).

Array order is display order: drag-and-drop reorders `AccountsList` and saves (`AM.cs:1905-1925`).

### 4.3 Groups

- Group is a plain string on each account; there is no separate group list. Empty input becomes `Default` (`AM.cs:1746-1760`).
- Sort-prefix convention (help text `AM.cs:2048-2049`, README "Group Sorting"): a leading number of up to three digits, optionally followed by one whitespace character, orders groups and is hidden in the title (`001 Apple`, `1Apple`). Groups sort by the raw string.
- Implementation (`AM.cs:196-205`): `Regex.Match(GroupName, @"\d{1,3}\s?")` is not anchored. It finds the first digit run anywhere and then removes that many characters from the start of the name. A group named `Team 12` is displayed as `am 12`. The rewrite should strip only an anchored `^\d{1,3}\s?` prefix.

## 5. Destination storage

### 5.1 Per-account saved destination
`Fields["SavedPlaceId"]` and `Fields["SavedJobId"]` (strings). Written by the Save button (`AM.cs:1862-1898`); loaded into the PlaceId/JobId boxes when the account is selected (`AM.cs:1442-1443`); override the global destination during multi-account launch (`AM.cs:2013-2017`). `SavedJobId` holds whatever was in the Job ID box: empty, a server job GUID, a full private-server URL containing `privateServerLinkCode=`, or `VIP:<accessCode>`.

### 5.2 Global last destination
`RAMSettings.ini` `[General]` `SavedPlaceId` (text of the PlaceId box; default `5315046213`) and `SavedFollowUser` (a username, not a user ID). Saved on close (`AM.cs:1604-1608`), restored on load (`AM.cs:676-677`). The Job ID box is not persisted globally.

### 5.3 `RecentGames.json`
JSON array of `Game` (`Classes/GameClass.cs:9-39`), written at `AM.cs:889-890`, oldest first / newest last, trimmed to `[General] MaxRecentGames` (default 8) (`AM.cs:869-892`).

```
[ { "Details": { GameDetails }, "ImageUrl": "<string or null>" }, ... ]
```

`GameDetails` members (`Classes/GameClass.cs:41-76`): `filteredName` (string), `placeId` (int64), `description`, `sourceName`, `sourceDescription`, `url`, `builder` (strings), `builderId` (int64), `hasVerifiedBadge` (bool), `isPlayable` (bool), `reasonProhibited` (string), `universeId` (int64), `universeRootPlaceId` (int64), `price` (int64), `imageToken` (string), `name` (string). When details could not be fetched the entry is a stub with only `placeId` and `name: "Unknown"` meaningful (`:75`). `filteredName` is `name` with bracketed tags and most punctuation removed and is what the PlaceId box autocompletes on (`:46-58`, `AM.cs:1469-1472`).

### 5.4 `FavoriteGames.json`
JSON array of `FavoriteGame` (`Classes/GameClass.cs:106-125`), written at `Forms/ServerList.cs:475-485`:

```
[ { "Name": "<display name>", "PrivateServer": "<string>", "PlaceID": <int64>,
    "Details": { GameDetails }, "ImageUrl": "<string>" }, ... ]
```

- `Name`: user-renamable; private-server favourites get the suffix ` (VIP)` (`Forms/ServerList.cs:611`).
- `PrivateServer`: empty, or the full Job ID box text containing `privateServerLinkCode=` (normally the full share URL) (`Forms/ServerList.cs:593-617`).
- `PlaceID` is taken from the PlaceId box, not from the link, when the favourite is created (`:611-613`); the legacy UI uses `Details.placeId` when a favourite is selected (`:466-473, 508`). Entries with null `Details` are dropped on save (`:481`).

### 5.5 `AccountControlData.json`
JSON array of `ControlledAccount` (`Nexus/ControlledAccount.cs:14-41`): `Username` (string), `AutoExecute` (string, Lua source), `PlaceId` (int64), `JobId` (string), `RelaunchDelay` (double, default 30), `AutoRelaunch` (bool), `IsChecked` (bool), `ClientCanReceive` (bool). `PlaceId`/`JobId` are the auto-relaunch target. `AutoExecute` is exploit script text and must not be imported.

### 5.6 No other destination stores
Server-list results, universe viewer and games search are in-memory only.

## 6. `RAMSettings.ini`

Parser/writer: `Classes/IniFile.cs`. `[Section]` headers, `key=value`, split on the first `=`, both sides trimmed, lines starting with `;` or `#` are comments, a comment line may precede a key, empty values are deleted rather than written (`:105-111, 191-229, 282-305`). Booleans are written as `true`/`false`; numbers use invariant culture on read but `decimal.ToString()` with the current culture on write (a comma decimal separator is possible on some locales).

`[General]` (defaults at `AM.cs:121-148` unless noted):

| Key | Default | Meaning / reference |
| --- | --- | --- |
| `CheckForUpdates` | true | Update check at start (`AM.cs:693`) |
| `AccountJoinDelay` | 8 | Seconds between multi-launches (`AM.cs:2001`) |
| `AsyncJoin` | false | Wait for launcher exit instead of fixed delay (`AM.cs:2003, 2021-2027`) |
| `DisableAgingAlert` | false | Hide the "unused > 20 days" dot (`AM.cs:746`, `Classes/AccountRenderer.cs`) |
| `SavePasswords` | true | Setting exists but is never consulted; passwords are always stored (`Forms/SettingsForm.cs:117-123`, `AM.cs:569-575`) |
| `ServerRegionFormat` | `<city>, <countryCode>` | `Forms/ServerList.cs:730-742` |
| `MaxRecentGames` | 8 | `AM.cs:875` |
| `ShuffleChoosesLowestServer` | false | Written by settings UI; not passed to `GetRandomJobId` (`Classes/Account.cs:519`) |
| `ShufflePageCount` | 5 | `Classes/Utilities.cs:119` |
| `IPApiLink` | `http://ip-api.com/json/<ip>` | `Forms/ServerList.cs:728` |
| `WindowScale` | computed | `Classes/Program.cs:27-40` |
| `ScaleFonts` | true | `Classes/Program.cs:42-51` |
| `AutoCookieRefresh` | true | `AM.cs:797-818` |
| `AutoCloseLastProcess` | true | `Classes/Account.cs:523` |
| `ShowPresence` | true | `AM.cs:2155` |
| `PresenceUpdateRate` | 5 | Stored only |
| `UnlockFPS` | false | `Classes/ClientSettingsPatcher.cs:31` |
| `MaxFPSValue` | 120 | `Classes/ClientSettingsPatcher.cs:35` |
| `UseCefSharpBrowser` | false | `AM.cs:1158` |
| `SavedPlaceId`, `SavedFollowUser` | — | Section 5.2 |
| `HideUsernames` | — | `AM.cs:690, 1555` |
| `HideRbxAlert` | — | `AM.cs:1151` |
| `EnableMultiRbx` | absent = off | `AM.cs:1256` |
| `ShuffleJobId` | — | `AM.cs:794, 2079` |
| `DisableImages` | — | `Classes/Batch.cs:49` |
| `CustomClientSettings` | — | Absolute path to custom FastFlag JSON (`Forms/SettingsForm.cs:342`) |
| `UseProxies`, `ProxyTimeout`, `ProxyTestLimit` | — | `Classes/AccountBrowser.cs:83-87` |
| `NopechaKey`, `NopechaAutoSolve` | — | Third-party captcha-solver API key (**secret**) (`Classes/AccountBrowser.cs:202-204, 272`) |

`[Developer]`: `DevMode` (false), `EnableWebServer` (false) (`AM.cs:150-151`).

`[WebServer]`: `WebServerPort` (7963), `AllowGetCookie`, `AllowGetAccounts`, `AllowLaunchAccount`, `AllowAccountEditing`, `EveryRequestRequiresPassword`, `AllowExternalConnections` (all false), `Password` (**secret**) (`AM.cs:153-160`).

`[AccountControl]`: `AllowExternalConnections` (false), `RelaunchDelay` (60), `LauncherDelayNumber` (9), `NexusPort` (5242) (`AM.cs:162-165`); `StartOnLaunch`, `SaveOutput`, `AutoMinimizeEnabled`, `AutoCloseEnabled`, `InternetCheck`, `UsePresence`, `AutoMinimizeInterval`, `AutoCloseInterval`, `MaxInstances`, `AutoCloseType` (`Forms/AccountControl.cs:313-326`). The UI writes `LauncherDelay` but reads `LauncherDelayNumber` (`:318, 570`).

`[Watcher]`: `Enabled`, `ExitOnBeta`, `ExitIfNoConnection`, `SaveWindowPositions`, `VerifyDataModel` (true), `IgnoreExistingProcesses` (true), `CloseRbxWindowTitle`, `CloseRbxMemory`, `ExpectedWindowTitle` (`Roblox`), `MemoryLowValue` (200), `NoConnectionTimeout` (60), `ScanInterval` (6 s), `ReadInterval` (250 ms) (`Forms/ServerList.cs:42-61`).

`[Prompts]`: `VCPrompted` and one key per remembered yes/no dialog, named by the uppercase MD5 of `Caption.Instruction.Text` (`Classes/Utilities.cs:215-243`, `AM.cs:209-220`).

## 7. Login and account addition

- **Puppeteer path (default on Windows 8+).** `AccountBrowser.Login` launches a downloaded Chromium (PuppeteerSharp `BrowserFetcher`, with PuppeteerExtra Stealth plugin and `--disable-web-security`) at `https://roblox.com/login` (`Classes/AccountBrowser.cs:25, 55-180`). A `RequestFinished` hook watches POSTs to `auth.roblox.com` `/v2/login`, `/v2/signup` and `/users/<id>/two-step-verification/login`; it reads the password out of the request body and then reads the `.ROBLOSECURITY` cookie from the page (`:303-320`) and calls `AccountManager.AddAccount(cookie, password)` (`:322-335`).
- **CefSharp path** (`UseCefSharpBrowser=true` or Windows 7). Embedded CEF form; polls the DOM for `#login-password` / `#signup-password` every 50 ms, and when the URL contains `/home` reads `.ROBLOSECURITY` from the global cookie manager (`Classes/AccountBrowser.cs:347-471`).
- **Validation.** `new Account(cookie)` calls `https://www.roblox.com/my/account/json` with the cookie and takes `Name` and `UserId` (`Classes/Account.cs:91-108, 833-842`). Existing accounts with the same `UserID` are updated in place (`AM.cs:569-602`).
- **Other import paths.** Paste-a-cookie form (`Forms/ImportForm.cs:73-88`); drag-and-drop of text containing cookies (`AM.cs:224-241`); bulk `user:pass` list driving multiple browsers (`AM.cs:1396-1417`); WebServer `ImportCookie` (`AM.cs:975-982`).
- **"Open Browser" as account.** Launches the browser with the stored cookie injected (`Classes/AccountBrowser.cs:41-53, 397-414`); variants run arbitrary user-supplied JavaScript (`AM.cs:1798-1843`) or click a group-join button (`AM.cs:1811-1821`).
- **Captcha assistance.** With `NopechaKey` set, captcha frames are scraped and images sent to `api.nopecha.com`; optional auto-click (`Classes/AccountBrowser.cs:200-301`).
- **CSRF.** `GetCSRFToken` POSTs to `https://auth.roblox.com/v1/authentication-ticket/` with a fixed game Referer, expects 403 and reads the `x-csrf-token` response header (`Classes/Account.cs:134-164`).
- **Auth ticket.** `GetAuthTicket` repeats the POST with `X-CSRF-TOKEN` and reads the `rbx-authentication-ticket` response header (`Classes/Account.cs:112-132`).
- **PIN.** `CheckPin` GETs `auth.roblox.com/v1/account/pin/`; `UnlockPin` POSTs `v1/account/pin/unlock` with `pin=<4 digits>` and caches the unlock window in memory (`Classes/Account.cs:166-225`).
- **Cookie auto-refresh.** Every 5 min, for accounts unused > 20 days and not attempted in 7 days (unless `Fields.NoCookieRefresh == "true"`), POSTs `www.roblox.com/authentication/signoutfromallsessionsandreauthenticate` and stores the new cookie (`AM.cs:797-818`, `Classes/Account.cs:362-394`).
- **Quick Log In.** Approves a 6-character cross-device login code via `apis.roblox.com/auth-token-service/v1/login/enterCode` and `validateCode` (`Classes/Account.cs:813-826`).

## 8. Launching

Entry point `Account.JoinServer(PlaceID, JobID, FollowUser, JoinVIP)` (`Classes/Account.cs:505-678`):

1. Create `BrowserTrackerID` if missing: decimal string `Next(100000,175000)` concatenated with `Next(100000,900000)` (12 digits) (`:507-512`).
2. Patch `ClientAppSettings.json` (section 10) (`:514`).
3. Get CSRF token; optionally pick a random job ID (`ShuffleJobId`) from the public server list (`:516-519`, `Classes/Utilities.cs:115-152`).
4. Get an auth ticket (`:521`).
5. If `AutoCloseLastProcess`, close any `RobloxPlayerBeta` whose command line carries this account's tracker ID (`:523-547`).
6. Private server by link: `LinkCode = Regex.Match(JobID, "privateServerLinkCode=(.+)")` (greedy to end of string). If present, GET `https://www.roblox.com/games/<PlaceID>?privateServerLinkCode=<LinkCode>` with the cookie and scrape the access code from the HTML with `Roblox.GameLauncher.joinPrivateGame\(\d+\,\s*'(\w+\-\w+\-\w+\-\w+\-\w+)'` (`:489-503, 549-581`). On success `JoinVIP = true`, `AccessCode = <scraped GUID>`.
7. If `JoinVIP`, check and optionally change the account's private-server-invite privacy setting (`:583-609`).
8. Launch. `LaunchTime` = Unix epoch milliseconds (`:611`).

Default path (`:645-674`): `Process.Start` on a `roblox-player:` URI (shell protocol handler; legacy waits for the launcher process to exit):

```
roblox-player:1+launchmode:play+gameinfo:<AuthTicket>+launchtime:<ms>+placelauncherurl:<UrlEncode(PLU)>+browsertrackerid:<BrowserTrackerID>+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp
```

`PLU` (before URL-encoding):

| Case | PlaceLauncher URL |
| --- | --- |
| Place | `https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame&browserTrackerId=<BTID>&placeId=<PlaceID>&isPlayTogetherGame=false` |
| Specific server | `...?request=RequestGameJob&browserTrackerId=<BTID>&placeId=<PlaceID>&gameId=<JobID>&isPlayTogetherGame=false` |
| Private server | `...?request=RequestPrivateGame&placeId=<PlaceID>&accessCode=<AccessCode>&linkCode=<LinkCode>` |
| Follow user | `...?request=RequestFollowUser&userId=<UserID>` (the user ID is passed in the `PlaceID` parameter) |

`&isTeleport=true` is appended to the first two when the dev-mode "IsTeleport" switch is on (`:656`).

"Old join" path (dev-mode switch, `:613-642`): builds arguments for `RobloxPlayerBeta.exe` directly — `--app -t <ticket> -j "<PlaceLauncher URL>"` — looking for the executable in `C:\Program Files (x86)\Roblox\Versions\<CurrentVersion>` then `%LocalAppData%\Roblox\Versions\<CurrentVersion>`, where `CurrentVersion` is `clientVersionUpload` from `https://clientsettings.roblox.com/v1/client-version/WindowsPlayer` (`AM.cs:776-783`). The `ProcessStartInfo` is constructed but never started (no `Process.Start` in `:627-637`), so this path is dead code at HEAD.

UI pre-processing (`AM.cs:1462-1503`): the PlaceId box accepts a number, a recent-game `filteredName`, or a URL containing `/games/<id>`; if it also contains `privateServerLinkCode` the whole text is copied to the Job ID box. A Job ID starting with `VIP:` means "the rest is an access code" (set by double-clicking a private server in the server list, `Forms/ServerList.cs:280-284`) and is passed with `JoinVIP = true` and an empty link code.

Follow (`AM.cs:1505-1531`): resolves a username via `users.roblox.com/v1/usernames/users`, checks presence via `presence.roblox.com/v1/presence/users`, then `JoinServer(UserId, "", true)`.

Multi-account launch (`AM.cs:1999-2036`): sequential, with `AccountJoinDelay` seconds between accounts or, in `AsyncJoin`, until the previous launcher exits.

Dev-mode clipboard helpers build credential-bearing links: `<roblox-player://1/1+launchmode:play+gameinfo:<ticket>+...>` and a `launchmode:app` variant (`AM.cs:1648-1660, 1764-1775`), raw auth tickets (`AM.cs:1624-1646`) and raw cookies (`AM.cs:1671-1679`).

Separate "set server" call: POST `https://gamejoin.roblox.com/v1/join-game-instance` with `{gameId, placeId}` and User-Agent `Roblox/WinInet` to reserve a slot (`Classes/Account.cs:719-739`, `AM.cs:662`).

### Multi-Roblox
`UpdateMultiRoblox` (`AM.cs:1254-1274`): when `[General] EnableMultiRbx` is true, the manager creates and takes ownership of the named mutex `ROBLOX_singletonMutex` (`new Mutex(true, "ROBLOX_singletonMutex")` then `WaitOne(TimeSpan.Zero, true)`) and keeps it for the lifetime of the process. If acquisition fails (a Roblox client already owns it) the user is told to close Roblox and restart. Disabling the setting closes the handle. No event object is used, no handles are closed in other processes, and nothing else is patched. The feature is off by default; the README cites a statement from a Roblox anti-cheat developer that multi-client use "may be considered as malicious behavior" (`README.md` FAQ "Why was multi-roblox disabled by default?").

### Roblox cookie-file lock ("773 fix")
For the lifetime of the manager it opens `%LocalAppData%\Roblox\LocalStorage\RobloxCookies.dat` with `FileShare.None`, preventing the Roblox client from reading or writing its own cookie file, unless `no773fix.txt` exists (`Classes/Program.cs:178-190`).

## 9. Process tracking and watcher

- **PID to account.** The only link is `BrowserTrackerID`. The value is placed in the launch URI; the Roblox launcher forwards it to `RobloxPlayerBeta.exe` as `-b <id>`. The manager enumerates `Process.GetProcessesByName("RobloxPlayerBeta")`, reads each command line through WMI (`SELECT CommandLine FROM Win32_Process WHERE ProcessId = ...`, `Classes/Utilities.cs:108-113`) and matches `\-b (\d+)` against `Account.BrowserTrackerID` (`Classes/Account.cs:527-532, 699-704`, `Classes/RobloxWatcher.cs:91-94`). No window-title or handle trick is used for account matching.
- **Window placement.** With `SaveWindowPositions`, the watcher stores the window rectangle (`GetWindowRect` on `MainWindowHandle`) into the account's `Window_*` fields; after launch the window is moved back with `MoveWindow` (`Classes/RobloxWatcher.cs:89-105`, `Classes/Account.cs:680-717`).
- **Watcher scan** (`Classes/RobloxWatcher.cs:59-118`, timer `ScanInterval`): ignores the foreground window; ignores processes whose command line is empty, starts with `\??\`, or lacks both `-t ` and `-j `; after 30 s of process age optionally kills on working set below `MemoryLowValue` MB or on main-window title different from `ExpectedWindowTitle`.
- **Log discovery and reading** (`Classes/RobloxProcess.cs`): runs Sysinternals Handle (`handle.bin -p <pid>`), regex-matches an open file `...\logs\<...>_Player_<...>_last.log`, opens it shared and tails it every `ReadInterval` ms (`:222-275, 104-207`). Detects "Joining game '<guid>' place <id> at <ip>" (connected), "Sending disconnect with reason" (disconnected) and DataModel/return-to-app lines to detect the home screen; kills the process on `ExitOnBeta` or after `NoConnectionTimeout`. The regexes are replaced at every start by a file downloaded from the upstream GitHub repository (`:39-64`, `AM.cs:792`). The no-connection check reads the key `" ExitIfNoConnection"` with a leading space, so it never triggers (`:81`).
- **Auto-relaunch** (`Forms/AccountControl.cs:481-514`): per controlled account, relaunch when presence is not InGame (`UsePresence`) or when no Nexus ping arrived within the relaunch delay.
- **Bulk process actions**: minimise all Roblox windows (`WM_SYSCOMMAND/SC_MINIMIZE`), kill all, periodic auto-minimise, auto-close by age / max instance count (`Forms/AccountControl.cs:582-618`), and kill of any client whose window-title MD5 equals `6B19DD3A0E36191A937AB6FD96869A9D` (`:767-774`; plaintext unknown).
- No priority-class, affinity, Job Object, CPU limiter, anti-AFK or input-injection code exists in the C# application. In-game CPU reduction exists only as the Nexus Lua "performance" command.

## 10. Client settings / FPS

`ClientSettingsPatcher.PatchSettings` (`Classes/ClientSettingsPatcher.cs:9-41`), run on every launch:

- Version folder = parent directory of the path in the default value of `HKEY_CLASSES_ROOT\roblox\DefaultIcon`; must be named `version-*` and contain `RobloxPlayerLauncher.exe`.
- Target file: `<version folder>\ClientSettings\ClientAppSettings.json` (directory created if missing).
- If `[General] CustomClientSettings` points to an existing file: `File.Copy(custom, target)` without overwrite (throws if the target exists; the exception is logged and ignored at `Classes/Account.cs:514`).
- Else if `[General] UnlockFPS`: set JSON key `DFIntTaskSchedulerTargetFps` to `MaxFPSValue` (fallback 240) in the existing file, or write `{"DFIntTaskSchedulerTargetFps":240}` when the file is missing/invalid.
- `GlobalBasicSettings_*.xml` is never read or written.

## 11. Local HTTP API ("WebServer")

`HttpListener` on `http://localhost:<WebServerPort>/` (plus `http://*:<port>/` when external connections are allowed and the process is elevated) (`AM.cs:749-774`, `Classes/WebServer.cs`). Handler `SendResponse` (`AM.cs:898-1147`); a `/v2/` prefix switches to JSON `{Success, Message}` replies. Query parameters `Account` (username or user ID) and `Password`.

Endpoints: `Running`, `GetAccounts`, `GetAccountsJson` (optionally with cookies), `ImportCookie`, `GetCookie`, `LaunchAccount` (`PlaceId`, `JobId`, `FollowUser`, `JoinVIP`), `FollowUser`, `GetCSRFToken`, `GetAlias`, `GetDescription`, `BlockUser`, `UnblockUser`, `GetBlockedList`, `UnblockEveryone`, `SetServer`, `SetRecommendedServer`, `GetField`, `SetField`, `RemoveField`, `SetAvatar`, `SetAlias`, `SetDescription`, `AppendDescription`.

Defects observed: `ImportCookie` has no permission check and takes the cookie in the URL (`:975-982`); `GetAccountsJson` includes cookies when the supplied password is *not* equal to the configured one (`Password != WSPassword`, `:948`); `SetDescription` lacks a `return` on the deny path (`:1129`); `GetCSRFToken`, `GetAlias`, `GetDescription`, `GetField`, block/unblock, `SetServer`, `SetAvatar` need no permission flag; `WSPassword` is null on first run and dereferenced (`:158, 922, 948`). `RAMAccount.lua` (repo root) is the in-game client for this API and is meant to run inside a script executor.

## 12. Nexus / Account Control

WebSocket server (`websocket-sharp`) on `NexusPort` (5242), loopback or all interfaces, path `/Nexus`, no authentication (`Forms/AccountControl.cs:97-116`). Clients are in-game scripts (`Nexus/Nexus.lua`, requires a third-party script executor) connecting with `?name=<username>&id=<userId>&jobId=<jobId>` (`Nexus/WebsocketServer.cs:26-46`, `Nexus/Nexus.lua:232`). Client-to-server JSON commands: `ping`, `Log`, `GetText`, `SetRelaunch`, `SetAutoRelaunch`, `SetPlaceId`, `SetJobId`, `Echo`, `CreateButton/TextBox/Numeric/Label`, `NewLine` (`Nexus/ControlledAccount.cs:85-171`). Server-to-client: free-text commands, notably `execute <lua>` which the client runs with `loadstring` (`Forms/AccountControl.cs:376-378`, `Nexus/Nexus.lua:309`), plus per-account auto-execute scripts. Documented in `Nexus/NexusDocs.md`.

## 13. Account utilities and other web-API features (all cookie-authenticated)

Change password / email, follow privacy, sign out of other sessions, block/unblock, display name, friend request, set avatar / wear outfit, purchase missing avatar assets (`Classes/Account.cs:262-487, 741-811`, `Forms/AccountUtils.cs`, `Classes/MissingAssetControl.cs:100-122`), account detail dump to disk (`AM.cs:2097-2130`), presence dots (`Classes/Presence.cs`, `Classes/AccountRenderer.cs`).

Server list window (`Forms/ServerList.cs`): public servers via `games.roblox.com/v1/games/<id>/servers/public` (`:229-256`); private servers via `/servers/VIP` with cookie (`:257-276`); region lookup by requesting a join script from `gamejoin.roblox.com` and sending the server IP to `ip-api.com` over HTTP (`:625-762`); "player finder" by matching avatar-headshot thumbnails against server player tokens (`:308-386`); games search (`:394-452`); favourites (`:466-617`); universe places (`:794-830`); outfits viewer (`:919-940`).

## 14. Updater, self-modification and downloads

- Update check: GitHub `releases/latest` tag vs. file version (`AM.cs:693-744`). On accept the exe copies itself to `Auto Update.exe`, re-launches with `-update`, tries to elevate (`runas`), downloads the first asset of release tag `0.0`, kills other instances and extracts over the install directory; no signature or hash verification (`Classes/Program.cs:99-129`, `Forms/Updater.cs:72-221`).
- Rewrites `<exe>.config` and `log4.config` when their SHA-256 differs from embedded hashes, then restarts (`Classes/Program.cs:136-156`).
- Silently downloads and runs the VC++ redistributable (`/q /norestart`) on first start if not detected (`AM.cs:207-221`).
- Downloads Chromium (PuppeteerSharp) or a CEF zip from an upstream release (`AM.cs:1168-1247`).
- Downloads watcher regexes from upstream master on every start (`Classes/RobloxProcess.cs:45`).
- "Start with Windows" writes `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run` (`Forms/SettingsForm.cs:60-68, 179-187`).
- Single-instance mutex for the manager itself: `{93b3858f-3dac-4dc0-99cb-0476efc5adce}` (`Classes/Program.cs:54, 173`).
- No analytics/telemetry endpoint was found. Outbound hosts: `*.roblox.com`, `api.github.com` / `github.com`, `aka.ms`, `ip-api.com`, `api.nopecha.com`, user-configured proxies.

## 15. Theming

`RAMTheme.ini`, single section named after the assembly, `[Roblox Account Manager]` (old name `[RBX Alt Manager]` is mapped on read, `Classes/IniFile.cs:211`). Keys (`Forms/ThemeEditor.cs:98-163`): `AccountsBG`, `AccountsFG`, `ButtonsBG`, `ButtonsFG`, `ButtonsBC`, `ButtonStyle` (WinForms `FlatStyle` name: `Flat`, `Popup`, `Standard`, `System`), `FormsBG`, `FormsFG`, `DarkTopBar`, `ShowHeaders`, `TextBoxesBG`, `TextBoxesFG`, `TextBoxesBC`, `LabelsBC`, `LabelsFC`, `LabelsTransparent`, `LightImages`. Colours are `#RRGGBB`; booleans are `True`/`False`. Default theme text is an embedded resource (`Properties/Resources.Designer.cs:102-119`).

## 16. Developer mode

`[Developer] DevMode=true` exposes context-menu items "view fields", "get authentication ticket", "copy rbx-player link", "copy security token", "copy app link" and the Arguments window (IsTeleport, old-join, version override) (`AM.cs:679-688`, `Forms/ArgumentsForm.cs`). The README warns that the generated links let a third party act as the account (`README.md` "WARNING").

## 17. Feature decision table

Decisions follow the rewrite constraints: no stored cookies/passwords, no embedded browser, sign-in and launch through the user's own browser, documented OAuth 2.0 only, documented Windows APIs for process tracking, optional reversible performance controls.

| Feature | Legacy location | Decision | Why |
| --- | --- | --- | --- |
| Account list with alias, description, group, ordering | `Classes/Account.cs:20-88`, `AM.cs:1446-1460, 1746-1760, 1900-1925` | Keep | Core non-secret metadata; import directly |
| Group sort prefix (`NNN name`) | `AM.cs:196-205, 2048-2049` | Rewrite | Keep the convention, fix to an anchored prefix |
| Cookie / password storage, AccountData encryption modes | `AM.cs:263-383`, `Classes/Cryptography.cs` | Remove | New app stores no credentials; decrypt once in the importer only, discard secrets |
| Embedded login browsers (Puppeteer Chromium, CefSharp), cookie and password capture | `Classes/AccountBrowser.cs` | Remove | Replaced by system-browser sign-in / OAuth |
| Bulk user:pass import, cookie import form, cookie drag-drop | `AM.cs:224-241, 1396-1417`, `Forms/ImportForm.cs` | Remove | Credential handling |
| Captcha solver integration, stealth plugin, proxies, custom JS/extension injection | `Classes/AccountBrowser.cs:64-139, 200-301`, `AM.cs:1798-1843` | Remove | Bot-detection circumvention and credential risk |
| Auth-ticket / CSRF flow and `roblox-player:` URI with ticket | `Classes/Account.cs:112-164, 645-674` | Remove | Requires the cookie; replaced by browser hand-off |
| Destinations: place, job ID, private-server link, follow user | `Classes/Account.cs:549-656`, `AM.cs:1462-1531` | Rewrite | Same concepts, expressed as browser/deep links the user's signed-in browser launches |
| Per-account saved destination | `AM.cs:1862-1898, 2013-2017` | Keep | Import `Fields.SavedPlaceId/SavedJobId` |
| Recent games | `AM.cs:854-892`, `Forms/RecentGamesForm.cs` | Keep | Import `RecentGames.json` |
| Favourite games incl. private-server links | `Forms/ServerList.cs:466-617` | Keep | Import `FavoriteGames.json` |
| Multi-account sequential launch with delay | `AM.cs:1999-2036` | Rewrite | Possible only as sequenced browser hand-offs per browser profile |
| Shuffle job ID (random public server) | `Classes/Utilities.cs:115-152` | Rewrite | Public endpoint, no credential needed; optional |
| Multi-Roblox (`ROBLOX_singletonMutex`) | `AM.cs:1254-1274` | Remove | Circumvents the client's single-instance guard; upstream itself disabled it by default citing Roblox anti-cheat; outside documented APIs. Owner decision if reconsidered |
| Roblox cookie-file lock ("773 fix") | `Classes/Program.cs:178-190` | Remove | Interferes with the client's own files |
| Auto-close previous instance of same account | `Classes/Account.cs:523-547` | Rewrite | Useful, but needs a new PID-to-account association (tracker ID no longer controlled by the app) |
| PID matching via `-b` tracker ID and WMI command line | `Classes/Utilities.cs:108-113`, `Classes/RobloxWatcher.cs:91-94` | Rewrite | Use documented process APIs; association must come from launch timing / user confirmation |
| Window position save/restore | `Classes/RobloxWatcher.cs:89-105`, `Classes/Account.cs:680-717` | Rewrite | Documented Win32 calls; optional |
| Watcher: log tailing via Sysinternals Handle, remote-updated regexes, kill on home screen / no connection / low memory / title | `Classes/RobloxProcess.cs`, `Classes/RobloxWatcher.cs` | Rewrite | Keep process list + exit detection; drop handle.exe and remote regex download; any log reading should enumerate `%LocalAppData%\Roblox\logs` directly and be opt-in |
| Auto-relaunch | `Forms/AccountControl.cs:481-514` | Remove | Depends on stored credentials and Nexus; unattended relaunch is not possible through browser hand-off |
| Bulk minimise / close / auto-close by age or count | `Forms/AccountControl.cs:582-618` | Rewrite | Simple, user-initiated process controls are compatible; drop the timers that kill silently |
| FPS unlock / custom FastFlags (`ClientAppSettings.json`) | `Classes/ClientSettingsPatcher.cs` | Remove | Writes into the Roblox install; replaced by the reversible priority / Job Object controls |
| Priority class / CPU rate limit | (none in legacy) | New | Not present in legacy; nothing to port |
| Settings (`RAMSettings.ini`) | `AM.cs:112-165`, `Forms/SettingsForm.cs` | Rewrite | New settings store; import only relevant keys (section 6) |
| Theme editor (`RAMTheme.ini`) | `Forms/ThemeEditor.cs` | Rewrite | Optional colour import; WinForms-specific keys otherwise meaningless |
| Developer mode switches (ticket/link/cookie copy, IsTeleport, old join) | `AM.cs:679-688, 1624-1679, 1764-1775`, `Forms/ArgumentsForm.cs` | Remove | Credential-bearing output |
| Local HTTP API (WebServer) and `RAMAccount.lua` | `AM.cs:898-1147`, `Classes/WebServer.cs` | Remove | Unauthenticated-by-default credential and control surface |
| Nexus websocket control, `Nexus.lua`, auto-execute | `Nexus/*`, `Forms/AccountControl.cs` | Remove | Remote script execution through exploit executors |
| Account utilities (password, email, PIN, privacy, sessions, block, display name, friends, avatar, purchases, quick log in) | `Classes/Account.cs:166-487, 741-826`, `Forms/AccountUtils.cs` | Remove | Undocumented cookie-authenticated endpoints; open the Roblox settings page in the browser instead |
| Cookie auto-refresh | `AM.cs:797-818` | Remove | No cookies |
| Presence indicators | `Classes/Presence.cs` | Rewrite | Only if obtainable without credentials or via documented OAuth scopes; otherwise drop |
| "Unused for 20 days" marker | `Classes/AccountRenderer.cs` | Remove | Exists only to warn about cookie expiry; `LastUse` can still be imported as "last launched" |
| Server list (public servers) | `Forms/ServerList.cs:205-256` | Rewrite | Public endpoint; optional feature |
| Private-server listing, region lookup, player finder | `Forms/ServerList.cs:257-276, 308-386, 625-762` | Remove | Need the cookie / join-script API; player finder is a tracking tool; region lookup leaks IPs over HTTP |
| Games search, universe viewer, outfit viewer | `Forms/ServerList.cs:394-452, 794-830, 919-940` | Remove | Peripheral; can be revisited with documented APIs |
| Account detail dump | `AM.cs:2097-2130` | Remove | Writes sensitive account data to disk |
| Auto-updater, config self-repair, silent VC++ install, Chromium/CEF download | `Forms/Updater.cs`, `Classes/Program.cs:99-156`, `AM.cs:207-221, 1168-1247` | Remove | Unverified downloads and elevation |
| Start with Windows | `Forms/SettingsForm.cs:179-187` | Rewrite | Harmless, optional |
| Legacy import (aliases, notes, groups, destinations, settings) | n/a | New | Section 4-6 formats |
