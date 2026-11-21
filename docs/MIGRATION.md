# Migrating from Roblox Account Manager

> **Note.** RAM can now store session cookies in an encrypted vault, so the
> import can bring cookies over (into the vault) as well as names and notes,
> and imported accounts can launch directly without a fresh browser sign-in.
> Set up the vault first (Settings -> Security). If you prefer the no-cookie
> flow, skip the vault and import metadata only; those accounts sign in
> through the browser. The sections below describe the metadata-only import;
> the cookie option appears in the import preview when a vault is set up.

RAM imports the organisation you built in Roblox Account Manager
(tested against the 3.7.2 file formats) and nothing that signs you in.

## What you do

1. Close Roblox Account Manager.
2. In RAM press **Ctrl+I** (or **Import…**) and choose the folder that
   contains `AccountData.json`.
3. Review the preview. Every row says what will happen: *Add*, *Update*
   (and which fields), *Already present*, or *Skip* with the reason. Untick
   anything you do not want.
4. Choose **Import**.
5. For each imported account, pick its browser profile in the details pane
   and use **Sign in in browser**. Imported accounts start as *Unknown*.

The legacy folder is opened read-only. Nothing in it is changed, moved or
deleted, so you can keep using the old app or import again later.

## What is imported

| Legacy data | Becomes | Notes |
|---|---|---|
| `Username`, `Alias`, `Description` | Username, alias, notes | |
| `Group` | Group | The default group `Default` becomes "no group". A leading sort prefix such as `001 ` is kept in the stored name and hidden in the list. |
| `UserID` | Roblox user ID, marked *unconfirmed* | Confirmed the first time Roblox verifies the account. |
| `LastUse` | Last used | |
| Per-account saved place / server (`Fields.SavedPlaceId`, `SavedJobId`) | A named destination | A saved job ID becomes a *live server* entry, which may already have closed. |
| `FavoriteGames.json` | Destinations and games | Private-server links are kept and treated as private: they are never shown in lists, logs or diagnostics. |
| `RecentGames.json`, `[General] SavedPlaceId` | Games | |
| `AccountControlData.json` place and job | Destinations | The auto-execute script in that file is never read. |

## What is never imported

- `.ROBLOSECURITY` cookies (`SecurityToken`) and passwords. The importer's
  parser has no field for them, so they are discarded while reading; the
  decrypted buffer is overwritten afterwards.
- The local web API password, the captcha service key, developer settings,
  remembered prompts, proxies, browser configuration, account dumps.
- Themes and window layout.
- Saved servers stored as `VIP:<access code>`: the browser hand-off cannot
  use an access code. Re-add the server from its link.

## The account file's three formats

| Format | How RAM handles it |
|---|---|
| Not encrypted (legacy "no encryption" marker file present) | Read directly. |
| Windows-protected (the legacy default) | Decrypted with the Windows data-protection API. This only works on the PC that wrote the file. |
| Password-protected | Not readable by this version. Favorites, recent games and saved places still import. To import accounts, remove the password in the legacy app first, then import again. |

A file that cannot be read produces a notice in the preview; the rest of the
folder still imports.

## Importing more than once

Repeated imports are safe.

- Accounts are matched by Roblox user ID when the store has one, otherwise
  by username (case-insensitive). A renamed account with the same ID is the
  same account.
- An existing account is only *filled in*: empty alias, notes, group or user
  ID are taken from the legacy data. Anything you have set in RAM is
  kept. Sign-in status, favorites, tags and the browser profile are never
  touched.
- Destinations are matched by where they lead, games by place ID.
- Importing the same folder twice changes nothing the second time.

## What changes in daily use

| In Roblox Account Manager | In RAM |
|---|---|
| Add an account with a password or cookie | Add the username; sign in on roblox.com in your browser. |
| The app joins for you with a ticket | The app opens the join link in the account's browser profile; the browser starts Roblox. |
| One browser inside the app | One browser profile per account (Chrome, Edge, Brave, Vivaldi or Firefox), or the default browser where you pick the profile yourself. |
| "Valid" flag from a cookie check | Sign-in status with the time of the last check; *Verified* requires the optional OAuth setup. |
| Multi Roblox option | Not provided. RAM tracks and manages whatever clients Roblox allows to run. |
| Local web API, Nexus, Account Control | Removed. |
| FPS unlock | Removed. The frame-rate cap is a Roblox setting shared by all clients. |

## Where RAM keeps its data

`%LOCALAPPDATA%\RAM\store.json` (accounts, destinations, settings),
`store.json.bak` (the previous save) and `RAM.log` (redacted).
Authorizations for verified checks are in Windows Credential Manager under
`RAM/oauth-refresh-<n>`. Removing an account removes its authorization.
