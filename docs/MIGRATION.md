# Migrating from Roblox Account Manager

RAM imports the accounts you organised in Roblox Account Manager (tested
against the 3.7.2 file formats) and, if you choose, moves the file's cookies
into RAM's encrypted vault.

## What you do

1. Close Roblox Account Manager.
2. In RAM open **Settings → Data → Import from AccountData.json…** and choose
   the folder that contains `AccountData.json`.
3. Review the preview. Every row says what will happen: *Add*, *Update*
   (and which fields), *Already present*, or *Skip* with the reason. Untick
   anything you do not want.
4. Decide whether to **also import cookies**. This needs the vault to be set
   up first (Settings → Security). Without it only the metadata is imported.
5. Choose **Import**. Accounts that received a cookie are checked right away
   and show their real sign-in state; the others start as *Unknown* until you
   sign them in with **Open Browser**.

The legacy folder is opened read-only. Nothing in it is changed, moved or
deleted, so you can keep using the old app or import again later.

## What is imported

| Legacy data | Becomes | Notes |
|---|---|---|
| `Username`, `Alias`, `Description` | Username, alias, notes | |
| `Group` | Group | The default group `Default` becomes "no group". A leading sort prefix such as `001 ` is kept in the stored name and hidden in the list. |
| `UserID` | Roblox user ID, marked *unconfirmed* | Confirmed the first time Roblox verifies the account. |
| `LastUse` | Last used | |
| `SecurityToken` (the `.ROBLOSECURITY` cookie) | A vault entry, **only** when the cookie switch is on | Read from the unencrypted or Windows-protected file; the decrypted buffer is wiped afterwards. Never written to `store.json` or the log. |

## What is not imported

- Passwords. The parser has no field for them.
- Saved places, job IDs, favorites and recent games. RAM's launch bar is just
  Place ID / Job ID / Follow, with Rogue Lineage pre-filled; there is no
  destination list to fill.
- The local web API password, the captcha service key, developer settings,
  remembered prompts, proxies, browser configuration, account dumps.
- Themes and window layout.

## The account file's three formats

| Format | How RAM handles it |
|---|---|
| Not encrypted (legacy "no encryption" marker file present) | Read directly. |
| Windows-protected (the legacy default) | Decrypted with the Windows data-protection API. This only works on the PC that wrote the file. |
| Password-protected | Not readable by this version. Remove the password in the legacy app first, then import again. |

A file that cannot be read produces a notice in the preview.

## Importing more than once

Repeated imports are safe.

- Accounts are matched by Roblox user ID when the store has one, otherwise
  by username (case-insensitive). A renamed account with the same ID is the
  same account.
- An existing account is only *filled in*: empty alias, notes, group or user
  ID are taken from the legacy data. Anything you have set in RAM is
  kept. Sign-in status, favorites and tags are never touched.
- A cookie is imported only for accounts that have none in the vault yet.
- Importing the same folder twice changes nothing the second time.

## Going back

**Settings → Data → Export AccountData.json…** writes a file the legacy app
(and niccs' and RoALT's importers) can read. Without cookies the file is
plain JSON. With cookies (the vault must be unlocked) it is written in the
legacy Windows-protected format, so it only opens on this PC; still, keep it
somewhere safe or delete it after use.

## What changes in daily use

| In Roblox Account Manager | In RAM |
|---|---|
| Add an account with a password or cookie | **Add → Sign in in browser**: a Roblox sign-in page opens in RAM's own browser window and the session is captured into the vault. Pasting a cookie still works. |
| "Valid" flag from a cookie check | Sign-in status from the same check RoALT uses: valid, signed out, *bound session required*, *verification required*, or unchanged when Roblox could not be reached. |
| Join place / job, follow user | The same: Place ID, optional Job ID, Follow by username. A ticket launch, no browser involved. |
| Embedded browser | **Open Browser** opens roblox.com signed in as that account, in a window with its own profile; it is cleared when closed. |
| Multi Roblox option | On by default (Settings → Launching). |
| Local web API, Nexus, Account Control, auto-execute | Removed. |
| FPS unlock | Removed. The frame-rate cap is a Roblox setting shared by all clients. |

## Where RAM keeps its data

`%LOCALAPPDATA%\RAM\store.json` (accounts and settings), `store.json.bak`
(the previous save), `vault.bin` (encrypted cookies), `profiles\<id>\` (per-
account browser profiles) and `RAM.log` (redacted). Removing an account
removes its vault entry and browser profile.
