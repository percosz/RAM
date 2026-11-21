//! Import from and export to the legacy `AccountData.json`, and a snapshot of
//! running Roblox clients.

use crate::dto::ClientView;
use crate::state::{now, Shared};
use crate::svc::LegacyDpapi;
use ram_core::export::{self, ExportEncryption, ExportOptions};
use ram_core::import::{self, PlanAction, PlanKind};
use ram_core::import_cookies;
use ram_core::model::AccountId;
use ram_core::platform::ProcessApi;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, State};

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

#[derive(Serialize)]
pub struct PlanItemView {
    pub index: usize,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub action: String,
    pub selectable: bool,
    pub selected: bool,
}

#[derive(Serialize)]
pub struct PlanView {
    pub folder: String,
    pub items: Vec<PlanItemView>,
    pub notices: Vec<String>,
    pub add: usize,
    pub update: usize,
    pub unchanged: usize,
    pub skip: usize,
    /// Cookies found in the file (count), importable into the vault.
    pub cookies_available: usize,
}

/// Scans a legacy folder and returns the preview plan.
#[tauri::command]
pub fn import_scan(state: State<'_, Shared>, folder: String) -> Result<PlanView, String> {
    let folder = PathBuf::from(&folder);
    let s = lock(&state)?;
    let scan = import::scan(&folder, Some(&LegacyDpapi));
    let plan = import::plan(&s.store, &scan);
    let (add, update, unchanged, skip) = plan.counts();
    let items = plan
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| PlanItemView {
            index: i,
            kind: match it.kind {
                PlanKind::Account => "Account",
                PlanKind::Destination => "Place",
                PlanKind::Game => "Game",
            }
            .into(),
            label: it.label.clone(),
            detail: it.detail.clone(),
            action: match &it.action {
                PlanAction::Add => "Add".into(),
                PlanAction::Update { changes } => format!("Update {}", changes.join(", ")),
                PlanAction::Unchanged => "Already present".into(),
                PlanAction::Skip { reason } => format!("Skip: {reason}"),
            },
            selectable: matches!(it.action, PlanAction::Add | PlanAction::Update { .. }),
            selected: it.selected,
        })
        .collect();
    let cookies_available = import_cookies::read_cookies(&folder, Some(&LegacyDpapi)).map(|v| v.len()).unwrap_or(0);
    Ok(PlanView { folder: folder.display().to_string(), items, notices: plan.notices.clone(), add, update, unchanged, skip, cookies_available })
}

/// Applies the plan with the given selection; optionally moves cookies into
/// the vault for accounts matched by username.
#[tauri::command]
pub fn import_apply(app: AppHandle, state: State<'_, Shared>, folder: String, selected: Vec<usize>, with_cookies: bool) -> Result<String, String> {
    let folder = PathBuf::from(&folder);
    let mut s = lock(&state)?;
    let scan = import::scan(&folder, Some(&LegacyDpapi));
    let mut plan = import::plan(&s.store, &scan);
    for (i, item) in plan.items.iter_mut().enumerate() {
        item.selected = selected.contains(&i);
    }
    let record = import::apply(&mut s.store, &plan, now());
    s.store.settings.last_import_dir = folder.display().to_string();
    let mut cookie_count = 0;
    let mut to_check: Vec<AccountId> = Vec::new();
    if with_cookies {
        if !s.vault.is_unlocked() {
            s.save();
            return Err("Accounts were imported, but the vault is locked so cookies were not. Unlock it and import again.".into());
        }
        match import_cookies::read_cookies(&folder, Some(&LegacyDpapi)) {
            Ok(list) => {
                for lc in list {
                    let id = s.store.accounts.iter().find(|a| a.username.eq_ignore_ascii_case(&lc.username)).map(|a| a.id);
                    if let Some(id) = id {
                        if s.vault.set_secret(id, &lc.cookie, &crate::svc::WinEntropy, now()).is_ok() {
                            cookie_count += 1;
                            to_check.push(id);
                        }
                    }
                }
            }
            Err(e) => {
                s.save();
                return Err(format!("Accounts were imported, but cookies could not be read: {e}"));
            }
        }
    }
    s.save();
    drop(s);
    for id in to_check {
        crate::cmd_launch::start_check(app.clone(), id);
    }
    Ok(format!(
        "Imported {} account(s), updated {}, skipped {}{}.",
        record.accounts_added,
        record.accounts_updated,
        record.skipped,
        if with_cookies { format!(", {cookie_count} cookie(s) moved into the vault") } else { String::new() }
    ))
}

/// Writes `AccountData.json` into the folder, optionally with cookies
/// (always DPAPI-encrypted in that case).
#[tauri::command]
pub fn export_accounts(state: State<'_, Shared>, folder: String, with_cookies: bool) -> Result<String, String> {
    let mut s = lock(&state)?;
    let ids: Vec<AccountId> = s.store.accounts.iter().map(|a| a.id).collect();
    if ids.is_empty() {
        return Err("There are no accounts to export.".into());
    }
    if with_cookies && !s.vault.is_unlocked() {
        return Err("Unlock the vault to export cookies.".into());
    }
    let mut cookies = std::collections::HashMap::new();
    if with_cookies {
        for id in &ids {
            if let Some(c) = s.cookie(*id) {
                cookies.insert(*id, c);
            }
        }
    }
    let opts = ExportOptions {
        encryption: if with_cookies { ExportEncryption::Dpapi } else { ExportEncryption::Plaintext },
        include_cookies: with_cookies,
    };
    let cookie_of = |id: AccountId| cookies.get(&id).cloned();
    let (bytes, summary) = export::export(&s.store, &ids, &cookie_of, &opts, Some(&LegacyDpapi)).map_err(|e| e.to_string())?;
    let path = PathBuf::from(folder).join("AccountData.json");
    std::fs::write(&path, &bytes).map_err(|e| format!("could not write the file: {e}"))?;
    Ok(format!("Exported {} account(s), {} with a cookie, to {}.", summary.accounts, summary.with_cookie, path.display()))
}

/// Running Roblox clients, matched to accounts where RAM observed the launch.
#[tauri::command]
pub fn clients_snapshot(state: State<'_, Shared>) -> Result<Vec<ClientView>, String> {
    let mut s = lock(&state)?;
    let snapshot = s.procs.list_clients().map_err(|e| e.to_string())?;
    let timeout = s.store.settings.launch_timeout_secs;
    let _ = s.clients.observe(now(), &snapshot, timeout);
    let mut out = Vec::new();
    for p in snapshot {
        let account = s.clients.link_for_process(p.key).map(|l| l.account);
        let sample = s.procs.sample(p.key).ok();
        out.push(ClientView {
            pid: p.key.pid,
            account,
            account_name: account.map(|a| s.account_name(a)),
            working_set_mb: sample.map(|x| x.working_set / (1024 * 1024)).unwrap_or(0),
            private_mb: sample.map(|x| x.private_bytes / (1024 * 1024)).unwrap_or(0),
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn close_client(state: State<'_, Shared>, pid: u32) -> Result<(), String> {
    let s = lock(&state)?;
    let key = s.procs.list_clients().map_err(|e| e.to_string())?.into_iter().find(|p| p.key.pid == pid).map(|p| p.key);
    match key {
        Some(k) => s.procs.request_close(k).map_err(|e| e.to_string()),
        None => Err("that client is no longer running".into()),
    }
}
