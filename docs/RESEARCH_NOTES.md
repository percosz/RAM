# Research notes — Rust rewrite of Roblox Account Manager (no cookies, browser hand-off)

Research date: 2026-11-20. Web-only research; nothing was tested on a machine.

Labels used on every claim:

- **Documented** — stated on an official page (create.roblox.com docs, en.help.roblox.com, a Roblox staff DevForum post, learn.microsoft.com, vendor docs) that was fetched this session.
- **Documented (not re-fetched)** — I am citing a well-known official page from memory; the URL is given but the page was not re-read today. Re-check before relying on exact wording.
- **Community-observed** — third-party tools, non-staff forum posts, blogs. Works in practice, no contract.
- **Unverified** — could not confirm either way. Treat as unknown.

"Undocumented" below means Roblox/Microsoft/the vendor publish no contract for it, even if it is widely used.

---

## Part A — Reference repositories

Licence verdicts are from the GitHub licence indicator plus the raw LICENSE file where fetched. GitHub's "GPL-3.0" badge does not distinguish `GPL-3.0-only` from `GPL-3.0-or-later`; that depends on per-file headers, which were not inspected.

| Repo | Exists | Stack | Licence | Reuse in a GPL-3.0 project |
|---|---|---|---|---|
| inspectsoftware/RoALT | Yes | Rust + Tauri 2 (WebView2), React 19/TS/Vite, Tailwind, Inno Setup | **MIT** (LICENSE: "MIT License, Copyright (c) 2026 inspectsoftware") | Yes. MIT is GPL-compatible; keep the copyright + permission notice. |
| ericky0/rbx-account-manager | Yes (0 stars, 7 commits) | C# / .NET Framework 4.7.2, fork/continuation of ic3w0lf22 RAM, Lua helper | **GPL-3.0** (LICENSE = GNU GPL v3, 29 June 2007) | Yes (same licence). C# code is not directly usable in Rust anyway. |
| niccsprojects/Roblox-Account-Manager | Yes ("v4 beta") | Rust + TypeScript on Tauri, Vite, Bun, react-i18next | **GPL-3.0** (LICENSE = GNU GPL v3) | Yes (same licence). |
| vysaely/alyae | Yes | Rust workspace (`crates/`), CMake build of patched "Mocktail" layer for Linux | **GPL-3.0** (LICENSE file present per GitHub indicator; raw file not fetched). Patches under `modules/roblox-linux/mocktail/` are **Apache-2.0** per README | Yes. GPL-3.0 same licence; Apache-2.0 is one-way compatible with GPLv3 (keep NOTICE/attribution). |

Sources: https://github.com/inspectsoftware/RoALT , https://raw.githubusercontent.com/inspectsoftware/RoALT/HEAD/LICENSE , https://github.com/ericky0/rbx-account-manager , https://raw.githubusercontent.com/ericky0/rbx-account-manager/HEAD/LICENSE , https://github.com/niccsprojects/Roblox-Account-Manager , https://raw.githubusercontent.com/niccsprojects/Roblox-Account-Manager/HEAD/LICENSE , https://github.com/vysaely/alyae (all **Documented** as of today's README/licence state; README contents are the projects' own claims).

### A.1 RoALT (MIT)

Workflow/UX ideas worth borrowing:
1. Account list with status, groups, notes, favourites.
2. "Play" box that accepts a game link, place ID, job ID, private-server link, or a saved launch profile.
3. Batch launch with a configurable stagger; "same server" modes (pick a server with room / follow the first account) and "different servers" modes.
4. Per-client monitor: CPU, memory, I/O per client PID; clients tracked by PID + start time + log file.
5. Performance profiles: per-client CPU cap, core affinity, efficiency mode, exportable presets ("Max alts").
6. Server browser sorted by players/ping.

Avoid (cookie storage / tampering):
- Stores `.ROBLOSECURITY` cookies (AES-256-GCM, DPAPI, SQLCipher) and accepts pasted cookies.
- Obtains an authentication ticket and builds a `roblox-player:` URI itself (undocumented endpoints).
- Multi-instance by pre-creating Roblox's named singleton object, holding Roblox's shared sign-in files, and an optional "Take over" that closes Roblox's single-instance lock.
- FPS cap/render flags via client settings, memory trimming, Anti-AFK, embedded roblox.com browser.

### A.2 ericky0/rbx-account-manager (GPL-3.0)

Ideas:
1. Real-time search by username, alias, description.
2. Four "server slots", each holding a PlaceID + JobID or VIP link, with a radio button for the active one.
3. Groups with drag-and-drop sorting.
4. Server browser showing player count, ping, region.
5. Auto relaunch on disconnect (idea only; see Part B.6 for what is observable).

Avoid:
- Encrypted cookie/password store, automatic cookie refresh, bulk import of user:pass combos or cookies, bulk password/email changes.
- Multi Roblox: pre-acquires the singleton mutex/event, then runs Sysinternals `handle.exe` to close singleton handles (mutex, event, `.mtx`, `.shm`) inside running Roblox processes — direct manipulation of the client process.
- "FPS Unlocker" by patching ClientAppSettings (now ineffective, see B.5).
- Remote in-game control over WebSocket + Lua (needs a script executor).

### A.3 niccsprojects/Roblox-Account-Manager (GPL-3.0)

Ideas:
1. Single and multi launch from Place / Job / Launch Data fields.
2. Favourites and recent games; "Shuffle Job ID"; server list with region data.
3. Grouping, drag-and-drop ordering, multi-select.
4. A "Watcher" that monitors clients by memory and window checks.
5. Themes and localisation (Crowdin).
6. A README warning never to share generated `rbx-player` links, because they let others act as the account. Good text to echo in a "what we do not do" page.

Avoid:
- Cookie input/import, auto cookie refresh, encrypted accounts file.
- "Pre-launch Isolation" that wipes Roblox cache and registry data; per-account Roblox version manager.
- Client Settings Overrides (FPS unlock, max FPS, graphics level).
- Multi Roblox (off by default; the README attributes this to a Byfron developer calling multiple clients potentially malicious — I could not find that primary statement, **Unverified**).
- Botting Mode (timed rejoin), script manager, WebSocket "Nexus" control.

### A.4 vysaely/alyae (GPL-3.0, Apache-2.0 patches)

Ideas:
1. Cross-platform Rust architecture (Windows + Linux), split into crates.
2. One password-locked export file for moving the account list between PCs (for this project: metadata only, no secrets).
3. "Low-memory profile for the main account" as a named preset concept.
4. Data folder beside the executable on Windows (portable mode).

Avoid:
- Stores cookies, passwords and 2FA secrets (encrypted).
- Patched Mocktail compatibility layer; Anti-AFK; shift-lock/camera patches (client-side modification).
- Spoofs User-Agent and client-hint headers so web calls and the sign-in window claim to be Chrome on Windows.
- Embedded sign-in window (WebKitGTK/WebView2).

---

## Part B — Roblox interfaces

### B.1 OAuth 2.0

Endpoints (**Documented**, https://create.roblox.com/docs/cloud/auth/oauth2-reference ):

| Purpose | Method + URL |
|---|---|
| Authorize | `GET https://apis.roblox.com/oauth/v1/authorize` |
| Token (code exchange + refresh) | `POST https://apis.roblox.com/oauth/v1/token` |
| Introspect | `POST https://apis.roblox.com/oauth/v1/token/introspect` |
| Resources | `POST https://apis.roblox.com/oauth/v1/token/resources` |
| Revoke | `POST https://apis.roblox.com/oauth/v1/token/revoke` (takes the refresh token; 200 with empty body) |
| Userinfo | `GET https://apis.roblox.com/oauth/v1/userinfo` with `Authorization: Bearer` |
| OIDC discovery | `https://apis.roblox.com/oauth/.well-known/openid-configuration` (path inferred from base URL by my reader; confirm at runtime — **Documented (not re-fetched)**) |

- Authorize parameters: `client_id`, `redirect_uri`, `scope` (space-delimited), `response_type` (`code` or `none`), optional `prompt` (`none`, `login`, `consent`, `select_account`), `nonce`, `state`, `code_challenge`, `code_challenge_method=S256`. **Documented** (same page). `prompt=select_account` / `login` is useful for multi-account linking.
- Token bodies are `application/x-www-form-urlencoded`; client auth via HTTP Basic or body fields. **Documented**.
- Lifetimes: authorization code 1 minute, single use; access token 15 minutes (`expires_in` 899); refresh token 90 days, single use (rotates). **Documented**.
- PKCE / public clients: authorization-code flow with PKCE is recommended for all clients and required for public clients; a public client sends client ID + code challenge and omits the secret at token exchange. **Documented**, https://create.roblox.com/docs/cloud/auth/oauth2-develop and https://create.roblox.com/docs/cloud/auth/oauth2-overview .
  - Whether refresh, introspect, resources and revoke work for a public client with no secret is **not stated** (the reference lists `client_secret` on those calls). **Unverified** — test. For a "stores no secrets" design the simplest path is a one-shot `openid profile` sign-in used only to confirm identity, discarding all tokens afterwards.
  - The docs describe public clients as mobile and browser apps; desktop apps are not mentioned. **Unverified** that Roblox classes a native desktop app the same way (it normally would).
- Userinfo claims: with `openid` only, `sub` (the Roblox user ID as a string). With `profile` added: `name`, `nickname`, `preferred_username`, `created_at`, `profile` (profile URL), `picture` (may be null). Use `sub` as the stable key. **Documented** (reference page).
- Scopes: identity scopes `openid` and `profile` (`profile` requires `openid`); all others are Open Cloud resource scopes of the form `asset:read` etc., grouped into app categories; an app's scopes must sit in one category. **Documented**, https://create.roblox.com/docs/cloud/auth/oauth2-registration . The full scope list page I tried (`/docs/cloud/reference/oauth2-scopes`) returned 404, so I did not read a complete list.
- **No launch/session scope.** Nothing in the overview, develop, reference or registration pages describes a scope that lets a third-party app launch the client as a user, mint an authentication ticket, or confirm a browser cookie session. **Confirmed by absence in the pages read** (not by a complete scope enumeration). OAuth can only tell you "the person at the keyboard signed in as user X in that browser"; it does not prove which account a later game launch will use.
- Redirect URIs (**Documented**, registration page): plain HTTPS URLs; localhost paths over HTTP or HTTPS "for local debugging purposes"; custom schemes; max 256 characters; up to 10 redirect URLs.
  - Arbitrary/ephemeral loopback port (RFC 8252 style): **not stated**. Assume exact-match and register a small set of fixed ports (e.g. `http://localhost:<port>/callback` ×N within the 10-URL limit). **Unverified**.
  - `http://127.0.0.1` literal vs `localhost`: **Unverified** (docs only say "localhost").
  - Whether a publicly reviewed app may keep an HTTP localhost redirect (docs frame it as debugging): **Unverified**.
- Registration requirements (**Documented**, overview + registration pages): developer must be ID-verified to register and publish; authorising users need a 13+ account; new apps are in private mode with a limit of 10 unique users; going public needs a review (demo video up to one minute starting signed-out, justification, privacy policy and ToS URLs over HTTPS); approval cannot be reverted to private; scope changes need user re-authorisation. The client secret is shown once.
- Rate limits (**Documented**, https://create.roblox.com/docs/cloud/reference/rate-limits ): per-endpoint limits are listed on each endpoint page; OAuth limits apply per access token (per user + app pair); responses carry `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset`; honour `retry-after` on 429, otherwise exponential backoff; undocumented limits may also apply. No numeric limit for the OAuth endpoints themselves was found — **Unverified**.

### B.2 Deep links / launch URLs

Officially documented (https://create.roblox.com/docs/production/promotion/deeplinks — **Documented**; note the page itself marks this deep-link process as **deprecated** in favour of share links):

- Web → app: `https://www.roblox.com/games/start?placeId=<id>&launchData=<string>`
- Direct to app: `roblox://placeId=<id>&launchData=<string>`
- Deferred (mobile install): `https://ro.blox.com/Ebh5?af_dp=<direct link>&af_web_dp=<web link>` (a May 2024 staff post adds a required `deep_link_value` copy of `af_dp`: https://devforum.roblox.com/t/developer-deeplinking-update/2960548 — **Documented**).
- Parameters: `placeId` (required unless `userId` given), `userId` (follow a user), `accessCode` (private server access code), `linkCode` (private server link code), `gameInstanceId` (= `DataModel.JobId`), `launchData` (max 200 bytes decoded, URL-encoded, visible and user-editable; read with `Player:GetJoinData().LaunchData`).

Not in the official docs:

- `roblox://experiences/start?placeId=<id>&gameInstanceId=<jobId>` — **Community-observed** (widely used; https://devforum.roblox.com/t/deep-links-using-gameinstanceid/3625328 ). Community reports say `gameInstanceId` is sometimes ignored on the web form and that the protocol form caused Windows errors in 2025 (https://devforum.roblox.com/t/roblox-protocol-deeplink-causes-multiple-windows-errors-notifications/3918494 ). Reliability **Unverified**.
- `https://www.roblox.com/games/<placeId>[/<slug>]?privateServerLinkCode=<code>` — old private-server link shape. **Documented** as the old format by a staff post (https://devforum.roblox.com/t/improved-private-server-links/2628225 , 2 Oct 2023).
- `https://www.roblox.com/share?code=<code>&type=Server` — new private-server link shape for links generated from 9 Oct 2023 onward. **Documented** (same staff post). Staff said pre-existing links keep working and that the link code + game ID are still used under the hood.
- `https://www.roblox.com/share?code=<code>&type=ExperienceInvite` and other `type` values — **Unverified** (the share-links doc page https://create.roblox.com/docs/production/promotion/share-links describes creating experience links in the Creator Dashboard but shows no URL shape or type list).
- `roblox://navigation/...` (e.g. `roblox://navigation/share_links?code=..&type=..`) — **Unverified**, not found in any official page read.
- `roblox-player:` URIs with an authentication ticket (what legacy managers build) — undocumented and out of scope by design.

Suggested validation shapes (derive from the above; codes are opaque — do not assume length or alphabet, **Unverified**):

```
https://(www.)roblox.com/games/start?placeId=<digits>[&gameInstanceId=<uuid>][&linkCode=<code>][&accessCode=<code>][&launchData=<urlenc ≤200 bytes>]
https://(www.)roblox.com/games/<digits>[/<slug>][?privateServerLinkCode=<code>]
https://(www.)roblox.com/share?code=<code>&type=Server
roblox://placeId=<digits>[&...same params]
roblox://experiences/start?placeId=<digits>[&gameInstanceId=<uuid>]      (community)
```

`gameInstanceId`/job ID is a 36-character lowercase UUID in client logs (pattern `[0-9a-f\-]{36}` used by Bloxstrap — **Community-observed**).

Private-server link staleness:
- The official FAQ only says the owner can "create a link" on the Configure page and manage who may join; it says nothing about expiry or regeneration. https://en.help.roblox.com/hc/en-us/articles/205345050 — **Documented** (silence noted).
- Regenerating the link invalidates the old one immediately; turning off "Allow Joining" also breaks it. **Community-observed** (blogs + DevForum bug threads, e.g. https://devforum.roblox.com/t/incorrect-terminology-displayed-in-invalid-link-ui-for-private-servers/3537615 ).
- Paid servers expire if the monthly renewal fails. **Documented** (FAQ).
- Time-based expiry of link codes: **Unverified** (none found). Design: treat saved private-server links as possibly stale and surface a "link may have been regenerated" hint.

Important behavioural caveat for the browser hand-off design (**Community-observed / Unverified**): the `https://www.roblox.com/games/start` page launches the client from the browser session, so the account used is the one signed in **in that browser profile**. The `roblox://` forms go straight to the installed app and use whatever account the **app** is signed into, not the browser's. For per-account launching, prefer the https form opened in the chosen browser profile. Verify empirically.

### B.3 Rogue Lineage IDs — verified

| Item | ID | Verified against |
|---|---|---|
| Universe ID | **1087859240** | `https://apis.roblox.com/universes/v1/places/3016661674/universe` → `{"universeId":1087859240}` |
| Root place "[🌍] Rogue Lineage" | **3016661674** | `https://games.roblox.com/v1/games?universeIds=1087859240` (`rootPlaceId` 3016661674); https://www.roblox.com/games/3016661674/Rogue-Lineage (page credits Monad Studios) |
| "Rogue Lineage: Khei" | **3541987450** | `https://develop.roblox.com/v1/universes/1087859240/places` and `https://apis.roblox.com/universes/v1/places/3541987450/universe` → same universe |
| "Rogue Lineage: Gaia" | **5208655184** | same two endpoints |
| "Conquest Lobby" | **12687102410** | `develop.roblox.com/v1/universes/1087859240/places` only |

All **Documented** in the sense of live Roblox API responses fetched today. Creator is the group "Rogue Lineage" (group ID 4556484, verified badge). `maxPlayers` 10. `createVipServersAllowed` is **false**, so private-server links do not apply to this game. Whether the sub-places (Khei, Gaia, Conquest Lobby) can be joined directly by place ID or only via in-game teleport is **Unverified**. Other IDs that search engines associate with "Rogue Lineage" (4598986920, 4598765888 and similar) were not checked and should be treated as copies — **Unverified**.

### B.4 Multiple concurrent clients on one Windows PC

- Official stance (**Documented**, Roblox staff account, 6 May 2025, https://devforum.roblox.com/t/an-update-on-automated-action-against-modified-clients/3640609 ): Roblox does **not officially support** running multiple player clients simultaneously; methods that do not modify the client binary are not affected by the modified-client enforcement; Roblox said it would investigate an officially supported option. I found **no evidence that an official multi-instance feature has shipped** as of today — **Unverified** either way.
- The same post: enforcement targets custom client binaries and official clients modified at runtime by external tools, up to account termination. Bloxstrap was said not to be affected at that time because it does not modify the binary, and is not officially supported. **Documented**.
- Singleton mechanism: a named kernel object (reported as `ROBLOX_singletonEvent`; some sources say `ROBLOX_singletonMutex`) that legacy tools pre-create or close inside the client. **Community-observed**; name inconsistent across sources; nothing official.
- Byfron/Hyperion and multi-instance tools: no staff statement found that Hyperion blocks them. The "Byfron developer said multiple clients may be treated as malicious" claim in RAM's README has no primary source that I could find — **Unverified**. A 2023 bug report notes `TeleportAsync` failing with multiple clients on one machine (https://devforum.roblox.com/t/teleportserviceteleportasync-fails-if-there-are-multiple-clients-running-on-the-same-machine-byfron-issue/2324412 ) — **Community-observed**.
- Account-farming enforcement (**Documented**, Roblox staff account, 8 Jan 2025, https://devforum.roblox.com/t/preventing-account-farming-and-exploits/3360580 ): automated action against abnormal activity and "the responsible accounts and their alts", improved alt detection. Does not mention multi-instance directly, but AFK alt-farming setups are the stated target.
- Terms of Use (effective 1 Nov 2026, https://en.help.roblox.com/hc/en-us/articles/115004647846 — **Documented**): prohibits reverse engineering, circumventing or disabling any part of the Service or gaining unauthorised access, and breaking technical/security measures; users are responsible for actions taken with their credentials. Multiple accounts are mentioned only in the context of abusing Earned Robux. No clause naming multi-instance. Closing singleton handles inside the client could arguably be "circumvent/disable"; the rewrite should not do it.
- Supported-looking paths:
  - Microsoft Store client alongside the desktop client: **Community-observed** (one non-staff user, https://devforum.roblox.com/t/running-multiple-official-roblox-clients-on-single-pc/3730867 ). Since Sept 2025 the Store listing is a new app, "Roblox - Windows", replacing the legacy UWP app, which Roblox says will eventually stop working (https://en.help.roblox.com/hc/en-us/articles/40842585714196 , https://news.xbox.com/en-us/2025/09/12/roblox-updated-app-xbox-pc/ — **Documented**). Whether the new Store app still coexists with the desktop client is **Unverified**.
  - Separate Windows user accounts / RDP sessions (named objects are per session): **Community-observed** suggestion in the same thread; **Unverified**.
  - VMs: community says they are detected. **Unverified**.

Verdict: no documented, supported way to run concurrent clients in one Windows session. The rewrite should not defeat the singleton. It can still manage and tune whatever clients exist.

### B.5 Frame rate and graphics

- In-client "Maximum Frame Rate" (**Documented**, staff post 30 May 2024, https://devforum.roblox.com/t/introducing-the-maximum-framerate-setting/2995965 ): options Default (60), 60, 120, 144, 240; Windows only at launch, Mac later with no ETA; staff said third-party unlockers change an internal dynamic flag that was never a stable public API. Staff in April 2026: above 240 fps is not officially supported (https://devforum.roblox.com/t/roblox-caps-fps-at-240-regardless-of-globalbasicsettings13xml-framerate-cap/4451048 — **Documented**). Current Mac status: **Unverified**.
- Persistence: the setting is stored as `<int name="FramerateCap">` in `%LOCALAPPDATA%\Roblox\GlobalBasicSettings_13.xml`. **Community-observed** (guides, the bug thread above; Roblox has not documented the file). Reports conflict on whether hand-edited values are honoured (one says ignored and locked to 240, the same poster later said editing worked; another says 9999 reverts to -1). The file is rewritten when a client exits, and is reset to defaults if the client fails to start (https://devforum.roblox.com/t/the-reason-why-your-roblox-settings-reset-sometimes-authfailed-403-authentication-failed/3938578 — **Community-observed**).
- Scope: the file sits in the per-Windows-user AppData folder, so it is **per Windows user and shared by every client of that user, not per client**. This is an inference from the path — no source states it outright (**Unverified** but very likely). Consequence: a per-client FPS cap cannot be expressed through this file; last client to exit wins.
- `ClientAppSettings.json` FFlag overrides: restricted to an allowlist (**Documented**, Roblox staff account, 29 Sep 2025, https://devforum.roblox.com/t/allowlist-for-local-client-configuration-via-fast-flags/3966569 ). Flags not on the list are ignored, with no enforcement action; Studio is unaffected; the list can change at any time without notice.
  - Allowlist as posted: `DFIntCSGLevelOfDetailSwitchingDistance`, `...L12`, `...L23`, `...L34`; `FFlagHandleAltEnterFullscreenManually`, `DFFlagTextureQualityOverrideEnabled`, `DFIntTextureQualityOverride`, `FIntDebugForceMSAASamples`, `DFFlagDisableDPIScale`, `FFlagDebugGraphicsPreferD3D11`, `FFlagDebugSkyGray`, `DFFlagDebugPauseVoxelizer`, `DFIntDebugFRMQualityLevelOverride`, `FIntFRMMaxGrassDistance`, `FIntFRMMinGrassDistance`, `FFlagDebugGraphicsPreferVulkan`, `FFlagDebugGraphicsPreferOpenGL`; `FIntGrassMovementReducedMotionFactor`.
  - **`DFIntTaskSchedulerTargetFps` is NOT on the list → blocked (ignored).** Documented by its absence in the staff post; community replies in the thread confirm the omission. Whether it was added later: **Unverified** (re-read the post's current revision before shipping).
  - File location (`<version folder>\ClientSettings\ClientAppSettings.json`): **Community-observed**; it is per install, so also shared by all clients.
- Graphics quality: `UserGameSettings.SavedQualityLevel` is a documented engine property/enum (https://create.roblox.com/docs/reference/engine/enums/SavedQualitySetting — **Documented** as an in-engine API). That it is persisted in `GlobalBasicSettings_13.xml` under `SavedQualityLevel` / `GraphicsQualityLevel` keys is **Community-observed / Unverified** (I found no source tying the key names to the file). The allowlisted `DFIntDebugFRMQualityLevelOverride` is the only staff-acknowledged external knob touching quality level.
- Supported external interface? **No.** Roblox documents neither `GlobalBasicSettings_13.xml` nor a stable contract for `ClientAppSettings.json`; the only supported control is the in-client settings menu. The allowlist is the closest thing to a tolerated interface and carries an explicit "may change without warning" disclaimer.

### B.6 Client observables

- Process name `RobloxPlayerBeta.exe`; install path `%LOCALAPPDATA%\Roblox\Versions\version-<hash>\`: **Community-observed** (consistent across DevForum threads and Bloxstrap). Program Files variant (`C:\Program Files (x86)\Roblox\Versions\...` for all-users installs): **Unverified** today.
- Legacy Store/UWP process `Windows10Universal.exe`: **Unverified** today, and the legacy app is being retired. Process name and packaging of the new "Roblox - Windows" Store app: **Unverified**.
- Logs: `%LOCALAPPDATA%\Roblox\logs`, files containing "Player" in the name; staff troubleshooting posts send users there (**Community-observed**, location effectively acknowledged by Roblox). Log format is undocumented and can change.
- Log lines reveal place / job / user (**Community-observed**, Bloxstrap source https://github.com/bloxstraplabs/bloxstrap/blob/main/Bloxstrap/Integrations/ActivityWatcher.cs ):
  - `[FLog::Output] ! Joining game '<jobId uuid>' place <placeId> at <ip>`
  - `[FLog::GameJoinLoadTime] Report game_join_loadtime: ... universeid:<n> ... userid:<n>`
  - `[FLog::Network] Replicator created:` (joined), `[FLog::Network] Time to disconnect replication data:` (disconnected), `[FLog::SingleSurfaceApp] leaveUGCGameInternal` (left), `[FLog::UgcExperienceController] ... doTeleport: joinScriptUrl` (teleport).
  - Reading a log file is passive and does not touch the client process. It is the only non-invasive way found to learn which user ID a given client actually joined as.
- Idle disconnect: error 278, disconnected after 20 minutes idle. **Community-observed** (error-code guides). I could not load an official page stating the 20-minute figure — **Unverified** officially.
- Hyperion and process handles:
  - Roblox staff (Bitdancer, 2 May 2023, https://devforum.roblox.com/t/64-bit-client-crashing-after-a-couple-minutes-of-play-every-time/2315314/5 ): unsigned software that opens a handle to the client with `PROCESS_ALL_ACCESS` instead of `PROCESS_QUERY_LIMITED_INFORMATION` is what Roblox treats as problem software ("badware") and is a typical cause of Hyperion crashes. **Documented**.
  - Staff (Bitdancer, 18 Sep 2023, https://devforum.roblox.com/t/welcoming-byfron-to-roblox/2018233/693 ): bootstrappers are generally fine if they do not access the client process; digitally signed tools are treated more leniently. **Documented**.
  - Whether Hyperion strips or denies `PROCESS_SET_INFORMATION` (needed for `SetPriorityClass`, EcoQoS) or `PROCESS_SET_QUOTA | PROCESS_TERMINATE` (needed for `AssignProcessToJobObject`) from a non-admin, same-user, unsigned process, or reacts to such handles: **Unverified**. No source found. Task Manager priority changes on Roblox are widely described as working (**Community-observed**, weak); one report of a crash after setting Realtime priority (https://devforum.roblox.com/t/roblox-crashing-after-setting-process-priority-to-realtime/2425378 ).
  - Design consequences: open with the narrowest rights, one purpose per handle (`PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE` for metrics and exit; add `PROCESS_SET_INFORMATION` only when applying priority/EcoQoS; job objects need the heavier `PROCESS_SET_QUOTA | PROCESS_TERMINATE`); never request `PROCESS_ALL_ACCESS`, `PROCESS_VM_*` or `PROCESS_DUP_HANDLE`; treat `ERROR_ACCESS_DENIED` as a normal outcome; code-sign the binary; never use Realtime. All performance controls need an empirical test against a current client before release.

### B.7 Public web APIs without authentication

- These "legacy" APIs are listed in Roblox's docs (https://create.roblox.com/docs/cloud/legacy , https://create.roblox.com/docs/cloud/legacy/users/v1 — **Documented**), with the warning that legacy APIs have minimal stability guarantees, can change without notice, and are not recommended for production.
  - `POST https://users.roblox.com/v1/usernames/users` — listed ("Get users by usernames"), auth "Cookie / None", marked Experimental. **Documented**.
  - `GET https://users.roblox.com/v1/users/{userId}` — listed under "Not Recommended" with a recommended alternative (presumably the Open Cloud users endpoint). **Documented**.
  - `GET https://games.roblox.com/v1/games?universeIds=` — worked unauthenticated today. Listing page not read: **Community-observed**.
  - `GET https://apis.roblox.com/universes/v1/places/{placeId}/universe` — worked unauthenticated today. Documentation status **Unverified**.
  - `GET https://develop.roblox.com/v1/universes/{universeId}/places` — worked unauthenticated today for a public universe. **Community-observed**.
  - Thumbnails (`https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds=&size=&format=`): not fetched. **Unverified** today.
- Rate limits: not published for legacy APIs; a staff reply reportedly said there are no plans to document them (https://devforum.roblox.com/t/add-ratelimiting-documentation-on-official-endpoints/2005502 — seen only via search snippet, **Unverified**). Design: cache aggressively, batch IDs, back off on 429.
- Privacy note: these calls send only a public username or ID, no credentials. With OAuth `profile` scope, `preferred_username` and `picture` already come from userinfo, so the legacy lookups may not be needed at all.

---

## Part C — Browser profile launch

General: opening a plain `https://` URL with `ShellExecuteW` uses the default browser with its last-used profile and needs no detection. Profile targeting requires launching the browser exe directly.

| Browser | Flag | Officially documented? | Profile name source (no cookies/passwords touched) |
|---|---|---|---|
| Chrome | `chrome.exe --profile-directory="Profile 1" <url>` | Switch exists in Chromium source (`switches::kProfileDirectory`), no user-facing Google doc found. `--user-data-dir` and directory layout are documented: https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md . **Community-observed / source-defined** | `%LOCALAPPDATA%\Google\Chrome\User Data\Local State` (JSON) → `profile.info_cache.<dirName>.name`. Format undocumented (**Community-observed**). Directory names (`Default`, `Profile N`) are documented as subdirectories of the user data dir (**Documented**) |
| Edge | `msedge.exe --profile-directory="Profile 1" <url>` | Acknowledged in Microsoft policy docs (the `EdgeDefaultProfileEnabled` policy page says the policy is ignored when `--profile-directory` is passed: https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/edgedefaultprofileenabled ) and in Microsoft Q&A answers; no dedicated reference page. **Documented (indirectly)** | `%LOCALAPPDATA%\Microsoft\Edge\User Data\Local State`, same structure (**Community-observed**); directory name visible at `edge://version` |
| Brave | same flag | Only community forum answers (https://community.brave.app/t/command-line-option-for-starting-in-a-specific-profile/427689 ). **Community-observed** | `%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data\Local State` (**Community-observed**, path not re-verified) |
| Vivaldi | same flag | Vivaldi forum (moderator replies), not help docs (https://forum.vivaldi.net/topic/36663/cli-switch-to-launch-with-different-user-profile ). **Community-observed** | `%LOCALAPPDATA%\Vivaldi\User Data\Local State` (**Community-observed**, path not re-verified) |
| Opera / Opera GX | none documented | No official doc found; a forum report of problems with `--profile-directory` (https://forums.opera.com/post/331334 ). **Unverified — treat as unsupported**; launch with default profile only | n/a (`%APPDATA%\Opera Software\Opera Stable` / `Opera GX Stable` are **Unverified**) |
| Firefox | `firefox.exe -P "name" <url>`, `firefox.exe -profile "<path>" <url>` | **Documented**: https://wiki.mozilla.org/Firefox/CommandLineOptions (`-P` name is case-sensitive; `-no-remote` = do not accept or send remote commands, implies `-new-instance`; `-new-tab URL`; `-url`) | `%APPDATA%\Mozilla\Firefox\profiles.ini` → `[ProfileN]` sections with `Name=`, `IsRelative=`, `Path=` (**Community-observed**, location from Mozilla support answers; also listed at `about:profiles`) |

Caveats:
- Chromium family: the flag takes the **directory** name, not the display name; a non-existent name creates a new profile (**Community-observed**, Microsoft Q&A / Tech Community). Validate against the directories that exist before launching. If the browser is already running, the URL is handed to the running instance and opened in the requested profile (**Community-observed**).
- Chromium `Local State` `info_cache` entries also contain account fields (e.g. `user_name`, `gaia_name`). Read only `name` (and the directory key); never store the others. The file contains no cookies or passwords, but do not open `Cookies`, `Login Data` or `Web Data`.
- Firefox: do not pass `-no-remote` when the profile may already be running (it refuses with "already running"). Without it, a running instance of the same profile receives the URL (**Community-observed**). Microsoft Store Firefox keeps profiles elsewhere (**Unverified**). Firefox's newer in-browser profile switcher (2025) may create profiles not listed as separate named entries in `profiles.ini` — **Unverified**; `-profile <path>` is the safer target.
- Locating executables: `HKLM` or `HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\<exe>` (`chrome.exe`, `msedge.exe`, `firefox.exe`, `brave.exe`, `vivaldi.exe`) — App Paths is a documented registration mechanism: https://learn.microsoft.com/en-us/windows/win32/shell/app-registration (**Documented (not re-fetched)**). Which browsers register there is **Community-observed**; Opera is **Unverified**. Alternative documented registration: `HKLM\SOFTWARE\Clients\StartMenuInternet\<browser>\shell\open\command` and `Capabilities\URLAssociations` (https://learn.microsoft.com/en-us/windows/win32/shell/default-programs — **Documented**).
- Default browser: use `AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_EXECUTABLE, L"https", L"open", ...)` (https://learn.microsoft.com/en-us/windows/win32/api/shlwapi/nf-shlwapi-assocquerystringw ) or `IApplicationAssociationRegistration::QueryCurrentDefault(L"https", AT_URLPROTOCOL, AL_EFFECTIVE, ...)` to get the ProgId (https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationassociationregistration-querycurrentdefault ) — **Documented (not re-fetched)**. The registry value `HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice\ProgId` is where the choice lives (**Community-observed**, described in Mozilla's docs https://firefox-source-docs.mozilla.org/widget/windows/shell/set-default.html ), but Microsoft does not document it as a contract; reading it is harmless, writing is blocked. Map ProgId → browser (`ChromeHTML`, `MSEdgeHTM`, `FirefoxURL-*`, `BraveHTML`, `VivaldiHTM*`, `OperaStable`) is **Community-observed**.

---

## Part D — Windows APIs for the performance tools

Pages fetched today are marked **Documented**; the rest are **Documented (not re-fetched)**.

Process exit (event-driven, no admin):
- `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)` then `RegisterWaitForSingleObject` with `WT_EXECUTEONLYONCE` (thread-pool wait; no 64-handle limit) or `WaitForMultipleObjects` (max `MAXIMUM_WAIT_OBJECTS` = 64 per call). https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-registerwaitforsingleobject , https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitformultipleobjects , access rights: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights . Works for same-user processes without elevation, subject to the Hyperion caveat in B.6. In Rust, `tokio`/`windows` crate wrappers over the same calls.

Process start (no fully event-driven non-admin API):
- WMI `Win32_ProcessStartTrace` (ETW-backed) requires elevation — **Community-observed / Documented (not re-fetched)**: https://learn.microsoft.com/en-us/previous-versions/windows/desktop/krnlprov/win32-processstarttrace .
- WMI `SELECT * FROM __InstanceCreationEvent WITHIN n WHERE TargetInstance ISA 'Win32_Process'` works non-admin but is polling inside WMI (interval `n`), and is heavier than polling yourself. https://learn.microsoft.com/en-us/windows/win32/wmisdk/--instancecreationevent
- `SetWinEventHook(EVENT_OBJECT_CREATE / EVENT_OBJECT_SHOW, ..., WINEVENT_OUTOFCONTEXT)` filtered to top-level windows (`idObject == OBJID_WINDOW`, no parent) is event-driven and needs no admin; it signals "a window appeared", from which `GetWindowThreadProcessId` gives the PID. Needs a message loop on the hooking thread. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook . `RegisterShellHookWindow` (`HSHELL_WINDOWCREATED`) is an alternative: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registershellhookwindow .
- Recommended: window-event hook as the trigger plus a slow fallback poll with `CreateToolhelp32Snapshot` / `EnumProcesses` (https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot ).

Per-process GPU metrics:
- PDH counter sets "GPU Engine" (`Utilization Percentage`) and "GPU Process Memory" (`Dedicated Usage`, `Shared Usage`). Instance names are OS/driver generated and must be enumerated (`PdhEnumObjectItemsW` or a wildcard path with `PdhAddEnglishCounterW`), then filtered on the `pid_<pid>_` prefix and summed. Full instance shape `pid_<pid>_luid_0x..._0x..._phys_<n>_eng_<n>_engtype_<3D|VideoDecode|...>` is **Community-observed**; Microsoft has no reference page for the naming. A Microsoft Q&A moderator answer confirms the enumerate-and-filter-by-`pid_` approach: https://learn.microsoft.com/en-us/answers/questions/5641645/how-to-get-the-special-process-gpu-usage-with-the . Background on engine aggregation: https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/ . PDH API: https://learn.microsoft.com/en-us/windows/win32/perfctrs/using-the-pdh-functions-to-consume-counter-data .
- Reliability: needs WDDM 2.0+ (Windows 10 1709+); rate counters need two samples; instances come and go with processes, so re-expand wildcards each sample; sum 3D engines per PID rather than averaging all engines. **Community-observed**.

CPU cap via job objects:
- `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` (**Documented**, https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_cpu_rate_control_information ): `JOB_OBJECT_CPU_RATE_CONTROL_ENABLE` (0x1) must accompany `..._WEIGHT_BASED` (0x2), `..._HARD_CAP` (0x4) or `..._MIN_MAX_RATE` (0x10); `..._NOTIFY` (0x8). `CpuRate` is percent × 100 of total system CPU (1–10000; 0 is rejected). Hard cap: once the job uses its share for the scheduling interval, no thread in it runs until the next interval (expect stutter, not smooth slowdown). Windows 8+. In a nested hierarchy a child's rate is a portion of its parent's. Not usable under Remote Desktop Services with DFSS.
- Nested jobs (**Documented**, https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs ): Windows 8+; a process already in a job can be added to another and the jobs nest if a valid hierarchy can be formed and neither job sets UI limits; otherwise `AssignProcessToJobObject` fails. Effective limits can only be more restrictive than the parent's.
- `AssignProcessToJobObject` needs `PROCESS_SET_QUOTA | PROCESS_TERMINATE` on the process and `JOB_OBJECT_ASSIGN_PROCESS` on the job: https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject . Use `IsProcessInJob` first to detect an existing job. **Documented (not re-fetched)**.
- Undo: a process **cannot be removed** from a job once assigned (https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects — **Documented (not re-fetched)**). Neutralise instead by calling `SetInformationJobObject(JobObjectCpuRateControlInformation)` again with `ControlFlags = 0` (rate control disabled) — this is the commonly used method but the struct page does not spell it out: **Unverified, test**. Fallback: keep it enabled with a hard cap of 10000 (100%).
- Lifetime hazards: never set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (closing the handle would kill the game). If the manager exits or crashes while a cap is active, the job and its cap persist until the client exits, and an unnamed job cannot be reopened. Mitigation to test: create a **named** job (`CreateJobObjectW` with a per-PID name) and reopen it with `OpenJobObjectW` after restart; whether the name survives the last handle closing is **Unverified**. Always lift caps on clean shutdown. https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-createjobobjectw

Priority and EcoQoS:
- `SetPriorityClass` needs `PROCESS_SET_INFORMATION`; `IDLE_PRIORITY_CLASS` / `BELOW_NORMAL_PRIORITY_CLASS` are valid for other processes; `PROCESS_MODE_BACKGROUND_BEGIN/END` can be used **only on the current process**. Reversible by restoring the class read earlier with `GetPriorityClass`. https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass — **Documented (not re-fetched)**.
- EcoQoS (**Documented**, https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation ): `SetProcessInformation(h, ProcessPowerThrottling, &PROCESS_POWER_THROTTLING_STATE, size)` with handle right `PROCESS_SET_INFORMATION`. On: `ControlMask = StateMask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED`. Force off (HighQoS): `ControlMask = EXECUTION_SPEED, StateMask = 0`. Return to system-managed: `ControlMask = 0, StateMask = 0` — this is the correct "undo". API is Windows 8+; EcoQoS proper is Windows 11 (earlier builds label it LowQoS). Task Manager's "Efficiency mode" = EcoQoS + idle priority (**Community-observed**). Overview: https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service .
- Optional: `SetProcessAffinityMask` (reversible, same access right) https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setprocessaffinitymask .

Memory metrics:
- `GetProcessMemoryInfo` with `PROCESS_MEMORY_COUNTERS_EX`: `WorkingSetSize` = physical RAM currently in the working set (includes shared pages); `PrivateUsage` = private commit charge (what the process has committed, whether or not resident). Show both; `PrivateUsage` is the better "how much does this client cost" figure. Needs `PROCESS_QUERY_LIMITED_INFORMATION` (plus `PROCESS_VM_READ` only on XP/2003). https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex , https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo — **Documented (not re-fetched)**. Working-set trimming (`EmptyWorkingSet`) is not recommended: it needs extra rights and only moves the cost to page faults.

Notifications from an unpackaged Win32 app:
- Classic toast path (**Documented**, https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps and https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl ): an unpackaged app needs an AppUserModelID, declared on a Start-menu shortcut (with a toast activator CLSID if it wants activation callbacks); the installer must create that shortcut before toasts appear.
- Windows App SDK `AppNotificationManager.Register()` sets up the registration for unpackaged apps automatically per its docs (https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/notifications/app-notifications/app-notifications-quickstart ), but it adds a runtime dependency that is awkward from Rust. Whether it fully removes the shortcut need: **Unverified**.
- Simplest: `Shell_NotifyIconW` with `NIF_INFO` (balloon), which Windows 10/11 render as a toast under the app's tray icon identity; no AUMID or shortcut needed; requires a tray icon. https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw — **Documented (not re-fetched)**. Recommended for v1.

---

## Items not verified (consolidated)

1. Full OAuth scope list (reference page 404); "no launch/session scope" is confirmed by absence only.
2. OAuth: arbitrary loopback port, `127.0.0.1` literal, localhost redirect on a publicly reviewed app, secret-less refresh/revoke/introspect for public clients, numeric rate limits, discovery URL wording.
3. `roblox://experiences/start`, `roblox://navigation/...`, `share?code=..&type=ExperienceInvite` (unofficial or not found); reliability of `gameInstanceId`; format/length of link codes; time-based expiry of private-server codes.
4. Which account a `roblox://` launch uses versus an https launch from a browser profile.
5. Whether Rogue Lineage sub-places can be joined directly; other "Rogue Lineage" place IDs are unchecked copies.
6. Whether Roblox shipped any official multi-instance option after May 2025; the "Byfron developer" quote; exact singleton object name; whether the new Store app coexists with the desktop client; separate-Windows-user approach.
7. `GlobalBasicSettings_13.xml` key names for graphics quality; whether hand-edited `FramerateCap` is honoured; explicit confirmation that the file is shared by all clients; current allowlist contents (only the 29 Sep 2025 list was read); Mac status of the frame-rate setting.
8. Program Files install path; `Windows10Universal.exe`; process name of the new Store app; official source for the 20-minute idle / error 278.
9. Whether Hyperion blocks or reacts to `PROCESS_SET_INFORMATION` / `PROCESS_SET_QUOTA | PROCESS_TERMINATE` handles from an unsigned same-user process. Needs a live test; this decides whether the performance tools are feasible at all.
10. Legacy API rate limits; documentation status of `games.roblox.com/v1/games`, `apis.roblox.com/universes/v1/places/{id}/universe`, thumbnails.
11. Brave/Vivaldi/Opera user-data paths and Opera profile launching; App Paths entries per browser; ProgId → browser map; Firefox Store-install and new profile-switcher storage.
12. Job object: disabling the cap with `ControlFlags = 0`; reopening a named job after the manager restarts.
13. Windows App SDK notifications without a Start shortcut.
14. Licence granularity (`-only` vs `-or-later`) for the GPL repos; alyae's raw LICENSE file was not fetched (GitHub indicator only).
