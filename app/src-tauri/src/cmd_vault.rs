//! Vault and cookie commands. Reveal is the only path that hands a raw
//! cookie to the front-end, and it is gated by Windows Hello or the master
//! password.

use crate::dto::{self, VaultStatus};
use crate::state::{now, Shared};
use crate::svc;
use ram_core::model::AccountId;
use ram_core::vault::VaultMode;
use tauri::{AppHandle, State};

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

#[tauri::command]
pub fn vault_status(state: State<'_, Shared>) -> Result<VaultStatus, String> {
    let s = lock(&state)?;
    Ok(dto::vault_status(&s))
}

#[tauri::command]
pub fn vault_setup_dpapi(state: State<'_, Shared>) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    s.vault.setup_dpapi(&crate::svc::WinEntropy, &crate::svc::WinProtector, 0).map_err(|e| e.to_string())?;
    Ok(dto::vault_status(&s))
}

/// Sets up a master-password vault and returns the one-time recovery code.
#[tauri::command]
pub fn vault_setup_password(state: State<'_, Shared>, password: String) -> Result<String, String> {
    if password.len() < 8 {
        return Err("Use at least 8 characters.".into());
    }
    let mut s = lock(&state)?;
    let code = s.vault.setup_master_password(&password, &crate::svc::WinEntropy, 15 * 60).map_err(|e| e.to_string())?;
    Ok(code.display().to_string())
}

#[tauri::command]
pub fn vault_unlock(state: State<'_, Shared>, password: String) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    match s.vault.mode() {
        Some(VaultMode::Dpapi) => s.vault.unlock_dpapi(&crate::svc::WinProtector, now()).map_err(|e| e.to_string())?,
        Some(VaultMode::MasterPassword) => s.vault.unlock_password(&password, now()).map_err(|e| e.to_string())?,
        None => return Err("No vault is set up.".into()),
    }
    Ok(dto::vault_status(&s))
}

#[tauri::command]
pub fn vault_unlock_recovery(state: State<'_, Shared>, code: String) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    s.vault.unlock_recovery(&code, now()).map_err(|e| e.to_string())?;
    Ok(dto::vault_status(&s))
}

#[tauri::command]
pub fn vault_lock(state: State<'_, Shared>) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    s.vault.lock();
    Ok(dto::vault_status(&s))
}

#[tauri::command]
pub fn vault_change_password(state: State<'_, Shared>, current: String, password: String) -> Result<String, String> {
    if password.len() < 8 {
        return Err("Use at least 8 characters.".into());
    }
    let mut s = lock(&state)?;
    if !s.vault.is_unlocked() {
        s.vault.unlock_password(&current, now()).map_err(|e| e.to_string())?;
    } else if !s.vault.verify_password(&current) {
        return Err("Incorrect current password.".into());
    }
    let code = s.vault.change_master_password(&password, &crate::svc::WinEntropy).map_err(|e| e.to_string())?;
    Ok(code.display().to_string())
}

#[tauri::command]
pub fn vault_remove(state: State<'_, Shared>) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    s.vault.destroy().map_err(|e| e.to_string())?;
    Ok(dto::vault_status(&s))
}

#[tauri::command]
pub fn vault_options(state: State<'_, Shared>, auto_lock_secs: u32, require_hello: bool) -> Result<VaultStatus, String> {
    let mut s = lock(&state)?;
    s.vault.set_auto_lock_secs(auto_lock_secs.min(24 * 3600)).map_err(|e| e.to_string())?;
    s.vault.set_require_hello_reveal(require_hello).map_err(|e| e.to_string())?;
    Ok(dto::vault_status(&s))
}

/// Ticks the auto-lock timer; returns true when the vault locked just now.
#[tauri::command]
pub fn vault_tick(state: State<'_, Shared>) -> Result<bool, String> {
    let mut s = lock(&state)?;
    Ok(s.vault.tick(now()))
}

/// Stores a cookie for an account, then starts a login check.
#[tauri::command]
pub fn cookie_set(app: AppHandle, state: State<'_, Shared>, id: AccountId, cookie: String) -> Result<(), String> {
    let cookie = cookie.trim().to_string();
    if cookie.len() < 32 {
        return Err("That does not look like a .ROBLOSECURITY cookie.".into());
    }
    {
        let mut s = lock(&state)?;
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        s.vault.set_secret(id, &cookie, &crate::svc::WinEntropy, now()).map_err(|e| e.to_string())?;
    }
    crate::cmd_launch::start_check(app, id);
    Ok(())
}

#[tauri::command]
pub fn cookie_remove(state: State<'_, Shared>, id: AccountId) -> Result<(), String> {
    let mut s = lock(&state)?;
    s.vault.remove_secret(id).map_err(|e| e.to_string())
}

/// Returns the raw cookie after a Windows Hello or password confirmation.
#[tauri::command]
pub fn cookie_reveal(state: State<'_, Shared>, id: AccountId, password: Option<String>) -> Result<String, String> {
    // Gate first, holding no lock while the Hello prompt is up.
    let (require_hello, master) = {
        let s = lock(&state)?;
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        (s.vault.require_hello_reveal(), s.vault.mode() == Some(VaultMode::MasterPassword))
    };
    let mut gated = false;
    if require_hello && ram_win::hello::is_available() {
        match svc::hello_confirm("Confirm to reveal a Roblox session cookie") {
            Ok(true) => gated = true,
            Ok(false) => return Err("Cancelled.".into()),
            Err(_) => {}
        }
    }
    let mut s = lock(&state)?;
    if !gated {
        if master {
            match password {
                Some(p) if s.vault.verify_password(&p) => {}
                Some(_) => return Err("Incorrect master password.".into()),
                None => return Err("PASSWORD_REQUIRED".into()),
            }
        }
        // DPAPI vault without Hello: Windows sign-in already gated it.
    }
    s.vault.touch(now());
    match s.vault.secret(id) {
        Ok(Some(c)) => Ok(c.to_string()),
        Ok(None) => Err("No cookie is stored for this account.".into()),
        Err(e) => Err(e.to_string()),
    }
}
