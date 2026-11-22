//! Account list, editing and settings commands.

use crate::dto::{self, AccountPatch, AccountView, Snapshot};
use crate::state::{now, Shared};
use ram_core::model::{Account, AccountId, Settings};
use ram_core::signin;
use tauri::State;

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

#[tauri::command]
pub fn snapshot(state: State<'_, Shared>) -> Result<Snapshot, String> {
    let s = lock(&state)?;
    Ok(dto::snapshot(&s))
}

#[tauri::command]
pub fn add_account(state: State<'_, Shared>, username: String, alias: String) -> Result<AccountView, String> {
    let name = username.trim();
    if name.len() < 3 || name.len() > 20 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Usernames are 3–20 letters, digits or underscores.".into());
    }
    let mut s = lock(&state)?;
    if s.store.accounts.iter().any(|a| a.username.eq_ignore_ascii_case(name)) {
        return Err("That account is already in the list.".into());
    }
    let id = s.store.alloc_id();
    let mut account = Account { id, username: name.to_string(), alias: alias.trim().to_string(), created: now(), ..Default::default() };
    signin::require_fresh_sign_in(&mut account.sign_in);
    account.sign_in.detail = "Add a cookie, or open the browser and sign in.".into();
    s.store.accounts.push(account);
    s.save();
    dto::account_view(&s, id).ok_or_else(|| "account not found".into())
}

#[tauri::command]
pub fn update_account(state: State<'_, Shared>, patch: AccountPatch) -> Result<AccountView, String> {
    let mut s = lock(&state)?;
    {
        let a = s.store.account_mut(patch.id).ok_or("account not found")?;
        if let Some(v) = patch.alias {
            a.alias = v.trim().to_string();
        }
        if let Some(v) = patch.group {
            a.group = v.trim().to_string();
        }
        if let Some(v) = patch.tags {
            a.tags = v.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        }
        if let Some(v) = patch.notes {
            a.notes = v;
        }
        if let Some(v) = patch.favorite {
            a.favorite = v;
        }
        if let Some(v) = patch.full_quality {
            a.full_quality = v;
        }
        if patch.clear_fps_cap {
            a.fps_cap = None;
        } else if let Some(v) = patch.fps_cap {
            a.fps_cap = Some(v.clamp(5, 360));
        }
    }
    s.save();
    dto::account_view(&s, patch.id).ok_or_else(|| "account not found".into())
}

#[tauri::command]
pub fn remove_accounts(state: State<'_, Shared>, ids: Vec<AccountId>) -> Result<(), String> {
    let mut s = lock(&state)?;
    for id in &ids {
        let _ = s.vault.remove_secret(*id);
        let _ = s.vault.remove_password(*id);
        s.clients.forget_account(*id);
        if s.store.is_main(*id) {
            s.store.main_account = None;
        }
    }
    s.store.accounts.retain(|a| !ids.contains(&a.id));
    s.store.recent.retain(|r| !ids.contains(&r.account));
    s.save();
    Ok(())
}

#[tauri::command]
pub fn set_main(state: State<'_, Shared>, id: Option<AccountId>) -> Result<(), String> {
    let mut s = lock(&state)?;
    if let Some(id) = id {
        if s.store.account(id).is_none() {
            return Err("account not found".into());
        }
    }
    s.store.main_account = id;
    s.save();
    Ok(())
}

#[tauri::command]
pub fn reorder_accounts(state: State<'_, Shared>, ids: Vec<AccountId>) -> Result<(), String> {
    let mut s = lock(&state)?;
    let mut ordered: Vec<Account> = Vec::with_capacity(s.store.accounts.len());
    for id in &ids {
        if let Some(pos) = s.store.accounts.iter().position(|a| a.id == *id) {
            ordered.push(s.store.accounts.remove(pos));
        }
    }
    let rest = std::mem::take(&mut s.store.accounts);
    ordered.extend(rest);
    s.store.accounts = ordered;
    s.save();
    Ok(())
}

#[tauri::command]
pub fn set_settings(state: State<'_, Shared>, settings: Settings) -> Result<Settings, String> {
    let mut s = lock(&state)?;
    let multi_changed = s.store.settings.multi_instance != settings.multi_instance;
    s.store.settings = settings;
    if multi_changed {
        s.apply_multi_instance();
    }
    s.save();
    Ok(s.store.settings.clone())
}

/// Saves a place to the launch bar's list, or takes it off. The built-in
/// place always stays.
#[tauri::command]
pub fn place_pin(state: State<'_, Shared>, place_id: u64, pinned: bool) -> Result<(), String> {
    if place_id == 0 {
        return Err("Enter a place ID first.".into());
    }
    let mut s = lock(&state)?;
    match s.store.games.iter_mut().find(|g| g.place_id == place_id) {
        Some(g) if g.builtin && !pinned => return Err("The built-in place stays in the list.".into()),
        Some(g) => g.pinned = pinned,
        None if pinned => s.store.games.push(ram_core::model::Game { place_id, pinned: true, ..Default::default() }),
        None => {}
    }
    s.save();
    Ok(())
}

#[tauri::command]
pub fn set_graphics(state: State<'_, Shared>, graphics: ram_core::client_settings::GraphicsSettings) -> Result<ram_core::client_settings::GraphicsSettings, String> {
    ram_core::client_settings::validate_custom(&graphics.custom_flags)?;
    let mut s = lock(&state)?;
    let mut g = graphics;
    g.fps_cap = ram_core::client_settings::clamp_fps(g.fps_cap);
    s.store.graphics = g;
    s.save();
    Ok(s.store.graphics.clone())
}

/// Saves the client limits. The bandwidth limit is a system QoS policy and
/// is applied separately (`apply_bandwidth`), because it needs an
/// administrator prompt.
#[tauri::command]
pub fn set_limits(state: State<'_, Shared>, limits: ram_core::model::LimitSettings) -> Result<ram_core::model::LimitSettings, String> {
    let mut s = lock(&state)?;
    let mut l = limits;
    l.cpu_limit_percent = l.cpu_limit_percent.min(100);
    l.trim_every_min = l.trim_every_min.min(720);
    l.tiny_width = l.tiny_width.clamp(160, 1920);
    l.tiny_height = l.tiny_height.clamp(120, 1080);
    // The stored bandwidth value changes only through apply_bandwidth.
    l.bandwidth_kbps = s.store.limits.bandwidth_kbps;
    s.store.limits = l;
    s.save();
    Ok(s.store.limits.clone())
}

#[tauri::command]
pub fn open_data_folder(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = {
        let s = lock(&state)?;
        s.data_dir.display().to_string()
    };
    app.opener().open_path(&dir, None::<&str>).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Development hook: `RAM_START_VIEW` picks the initial view and
/// `RAM_SELECT_FIRST` selects the first account, so screenshots need no
/// simulated input. Both are empty in normal use.
#[tauri::command]
pub fn start_hint() -> (String, bool, String) {
    (
        std::env::var("RAM_START_VIEW").unwrap_or_default(),
        std::env::var("RAM_SELECT_FIRST").map(|v| v == "1").unwrap_or(false),
        std::env::var("RAM_START_DIALOG").unwrap_or_default(),
    )
}
