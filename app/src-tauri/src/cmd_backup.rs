//! Backup and restore: the account list, settings, and every stored cookie
//! and password in one file sealed with a password of the user's choosing
//! (`ram_core::backup`).
//!
//! Making a backup hands out every secret at once, so it passes the same
//! gate as copying a single cookie (Windows Hello, or the master password).
//! Restoring replaces what RAM holds; the data file it replaces is kept
//! next to it as `store.before-restore.json`.

use crate::state::{now, Shared};
use crate::svc;
use ram_core::backup::{self, Backup, BackupError};
use ram_core::model::AccountId;
use ram_core::vault::VaultMode;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use tauri::{AppHandle, Manager, State};

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

#[derive(Serialize)]
pub struct BackupInfo {
    pub created: i64,
    pub app_version: String,
    pub accounts: usize,
    pub cookies: usize,
    pub passwords: usize,
}

fn info(b: &Backup) -> BackupInfo {
    BackupInfo { created: b.created, app_version: b.app_version.clone(), accounts: b.store.accounts.len(), cookies: b.cookies.len(), passwords: b.passwords.len() }
}

/// Writes a backup to `path`. `master_password` is only needed when the
/// vault uses one and Windows Hello did not confirm; the error
/// `PASSWORD_REQUIRED` asks the front-end for it.
#[tauri::command(async)]
pub fn backup_create(app: AppHandle, state: State<'_, Shared>, path: String, password: String, master_password: Option<String>) -> Result<String, String> {
    if password.chars().count() < backup::MIN_PASSWORD_LEN {
        return Err(BackupError::WeakPassword.to_string());
    }
    // Gate first, holding no lock while the Hello prompt is up.
    let (configured, require_hello, master) = {
        let s = lock(&state)?;
        if s.vault.is_configured() && !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        (s.vault.is_configured(), s.vault.require_hello_reveal(), s.vault.mode() == Some(VaultMode::MasterPassword))
    };
    let mut gated = !configured;
    if !gated && require_hello && ram_win::hello::is_available() {
        let hwnd = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
        match svc::hello_confirm(hwnd, "Confirm to back up every stored cookie and password") {
            Ok(true) => gated = true,
            Ok(false) => return Err("Cancelled.".into()),
            Err(e) => {
                if let Ok(s) = lock(&state) {
                    s.log.warn(&format!("Windows Hello prompt failed, falling back: {e}"));
                }
            }
        }
    }
    let bundle = {
        let mut s = lock(&state)?;
        if !gated && master {
            match &master_password {
                Some(p) if s.vault.verify_password(p) => {}
                Some(_) => return Err("Incorrect master password.".into()),
                None => return Err("PASSWORD_REQUIRED".into()),
            }
        }
        let mut cookies = BTreeMap::new();
        let mut passwords = BTreeMap::new();
        if configured {
            s.vault.touch(now());
            for id in s.vault.accounts_with_secret() {
                if let Ok(Some(c)) = s.vault.secret(id) {
                    cookies.insert(id, c.to_string());
                }
            }
            let ids: Vec<AccountId> = s.store.accounts.iter().map(|a| a.id).collect();
            for id in ids {
                if s.vault.has_password(id) {
                    if let Ok(Some(p)) = s.vault.password(id) {
                        passwords.insert(id, p.to_string());
                    }
                }
            }
        }
        Backup::new(&s.store, cookies, passwords, env!("CARGO_PKG_VERSION"), now())
    };
    // Sealing runs the password through Argon2; not under the lock.
    let bytes = backup::seal(&bundle, &password, &svc::WinEntropy).map_err(|e| e.to_string())?;
    let target = Path::new(&path);
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Could not write the backup: {e}"))?;
    std::fs::rename(&tmp, target).map_err(|e| format!("Could not write the backup: {e}"))?;
    let summary = info(&bundle);
    if let Ok(s) = lock(&state) {
        s.log.info(&format!("backup: wrote {} accounts, {} cookies, {} passwords", summary.accounts, summary.cookies, summary.passwords));
    }
    Ok(format!(
        "Backed up {} account{}, {} cookie{} and {} password{}.",
        summary.accounts,
        if summary.accounts == 1 { "" } else { "s" },
        summary.cookies,
        if summary.cookies == 1 { "" } else { "s" },
        summary.passwords,
        if summary.passwords == 1 { "" } else { "s" }
    ))
}

fn read_backup(path: &str, password: &str) -> Result<Backup, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read that file: {e}"))?;
    backup::open(&bytes, password).map_err(|e| e.to_string())
}

/// Opens a backup and says what is in it, without changing anything.
#[tauri::command(async)]
pub fn backup_inspect(path: String, password: String) -> Result<BackupInfo, String> {
    read_backup(&path, &password).map(|b| info(&b))
}

/// Replaces the account list, settings and stored secrets with a backup.
#[tauri::command(async)]
pub fn backup_restore(app: AppHandle, state: State<'_, Shared>, path: String, password: String) -> Result<String, String> {
    let b = read_backup(&path, &password)?;
    let summary = info(&b);
    let mut s = lock(&state)?;
    if s.read_only {
        return Err("RAM is in read-only mode (the data file came from a newer version).".into());
    }
    if !s.diag.watched.is_empty() {
        return Err("Close the running Roblox clients first: restoring changes which account each client belongs to.".into());
    }
    let has_secrets = !b.cookies.is_empty() || !b.passwords.is_empty();
    let vault_ready = s.vault.is_configured() && s.vault.is_unlocked();
    if has_secrets && !vault_ready {
        return Err("Set up or unlock the vault first (Settings → Security): the cookies in the backup are stored in it.".into());
    }
    // Keep what is being replaced.
    let _ = std::fs::copy(s.data_dir.join("store.json"), s.data_dir.join("store.before-restore.json"));
    let old_ids: Vec<AccountId> = s.store.accounts.iter().map(|a| a.id).collect();
    let applied = std::mem::take(&mut s.store.applied);
    let xml_undo = s.store.xml_undo.take();
    let mut store = b.store;
    store.applied = applied;
    store.xml_undo = xml_undo;
    store.links.clear();
    store.ensure_builtin_games();
    s.store = store;
    if vault_ready {
        let t = now();
        for id in s.vault.accounts_with_secret() {
            let _ = s.vault.remove_secret(id);
        }
        for id in old_ids {
            if s.vault.has_password(id) {
                let _ = s.vault.remove_password(id);
            }
        }
        for (id, cookie) in &b.cookies {
            s.vault.set_secret(*id, cookie, &svc::WinEntropy, t).map_err(|e| e.to_string())?;
        }
        for (id, pw) in &b.passwords {
            s.vault.set_password(*id, pw, &svc::WinEntropy, t).map_err(|e| e.to_string())?;
        }
    }
    s.diag.last_launch.clear();
    s.diag.rejoin.clear();
    s.apply_multi_instance();
    crate::window::set_always_on_top(&app, s.store.settings.always_on_top);
    crate::window::set_streamer_mode(&app, s.store.settings.streamer_mode);
    s.save();
    s.log.info(&format!("backup: restored {} accounts, {} cookies, {} passwords", summary.accounts, summary.cookies, summary.passwords));
    Ok(format!(
        "Restored {} account{}, {} cookie{} and {} password{}.",
        summary.accounts,
        if summary.accounts == 1 { "" } else { "s" },
        summary.cookies,
        if summary.cookies == 1 { "" } else { "s" },
        summary.passwords,
        if summary.passwords == 1 { "" } else { "s" }
    ))
}
