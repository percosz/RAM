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
    /// Saved passwords found in the file.
    pub passwords_available: usize,
    /// The file is locked with a legacy-app password that was not given.
    pub needs_password: bool,
    /// The password given does not open the file.
    pub wrong_password: bool,
}

#[derive(Serialize)]
pub struct FoundFileView {
    pub path: String,
    /// "Plaintext", "Dpapi" or "PasswordProtected".
    pub kind: String,
    pub modified: Option<i64>,
    pub size: u64,
    pub is_backup: bool,
}

/// Looks for the legacy app's `AccountData.json` (and its backups) on this
/// PC: Desktop, Downloads, Documents and the rest of the user folder first,
/// then OneDrive and the other drives, within a few seconds. Only file
/// names and the first bytes are read.
#[tauri::command(async)]
pub fn import_find() -> Vec<FoundFileView> {
    use ram_core::discover;
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let budget = Duration::from_secs(8);
    let mut found = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let mut roots = discover::default_roots(&home);
        if let Some(od) = std::env::var_os("OneDrive").map(PathBuf::from) {
            roots.insert(0, od);
        }
        found.extend(discover::find_account_files(&roots, 6, budget));
    }
    // Other fixed drives, with whatever time is left.
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
    for letter in b'C'..=b'Z' {
        let Some(left) = budget.checked_sub(started.elapsed()) else { break };
        let drive = format!("{}:", letter as char);
        let rootp = PathBuf::from(format!("{drive}\\"));
        if drive == system || !rootp.is_dir() {
            continue;
        }
        found.extend(discover::find_account_files(&[rootp], 4, left));
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|f| seen.insert(f.path.to_string_lossy().to_ascii_lowercase()));
    found.sort_by_key(|f| std::cmp::Reverse(f.modified.unwrap_or(0)));
    found
        .into_iter()
        .take(50)
        .map(|f| FoundFileView { path: f.path.display().to_string(), kind: format!("{:?}", f.kind), modified: f.modified, size: f.size, is_backup: f.is_backup })
        .collect()
}

/// Scans a legacy folder or account file and returns the preview plan.
/// `password` opens a file locked with a Roblox Account Manager password.
#[tauri::command(async)]
pub fn import_scan(state: State<'_, Shared>, folder: String, password: Option<String>) -> Result<PlanView, String> {
    let path = PathBuf::from(&folder);
    let pw = password.as_deref().filter(|p| !p.is_empty());
    // The scan may run a slow key derivation; do it before taking the lock.
    let scan = import::scan_path(&path, Some(&LegacyDpapi), pw);
    let (needs_password, wrong_password) = (scan.needs_password, scan.wrong_password);
    let (cookies_available, passwords_available) = if needs_password || wrong_password {
        (0, 0)
    } else {
        match import_cookies::read_secrets(&path, Some(&LegacyDpapi), pw) {
            Ok(v) => (v.iter().filter(|s| !s.cookie.is_empty()).count(), v.iter().filter(|s| !s.password.is_empty()).count()),
            Err(_) => (0, 0),
        }
    };
    let s = lock(&state)?;
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
    Ok(PlanView { folder: path.display().to_string(), items, notices: plan.notices.clone(), add, update, unchanged, skip, cookies_available, passwords_available, needs_password, wrong_password })
}

/// Applies the plan with the given selection; optionally moves cookies (and
/// any saved passwords) into the vault for accounts matched by username or
/// legacy user id.
#[tauri::command(async)]
pub fn import_apply(app: AppHandle, state: State<'_, Shared>, folder: String, selected: Vec<usize>, with_cookies: bool, password: Option<String>) -> Result<String, String> {
    let path = PathBuf::from(&folder);
    let pw = password.as_deref().filter(|p| !p.is_empty());
    let scan = import::scan_path(&path, Some(&LegacyDpapi), pw);
    if scan.needs_password || scan.wrong_password {
        return Err("The file's password is needed to import it.".into());
    }
    let secrets = if with_cookies { Some(import_cookies::read_secrets(&path, Some(&LegacyDpapi), pw)) } else { None };
    let mut s = lock(&state)?;
    let mut plan = import::plan(&s.store, &scan);
    for (i, item) in plan.items.iter_mut().enumerate() {
        item.selected = selected.contains(&i);
    }
    let record = import::apply(&mut s.store, &plan, now());
    s.store.settings.last_import_dir = path.display().to_string();
    let mut cookie_count = 0;
    let mut password_count = 0;
    let mut to_check: Vec<AccountId> = Vec::new();
    if let Some(secrets) = secrets {
        if !s.vault.is_unlocked() {
            s.save();
            return Err("Accounts were imported, but the vault is locked so cookies were not. Unlock it and import again.".into());
        }
        match secrets {
            Ok(list) => {
                for lc in list {
                    // Match by username, or by the legacy user id for an
                    // account the user has since renamed.
                    let key = format!("legacy:user:{}", lc.user_id);
                    let id = s
                        .store
                        .accounts
                        .iter()
                        .find(|a| a.username.eq_ignore_ascii_case(&lc.username) || (lc.user_id != 0 && a.import_key.as_deref() == Some(&key)))
                        .map(|a| a.id);
                    let Some(id) = id else { continue };
                    if !lc.cookie.is_empty() && s.vault.set_secret(id, &lc.cookie, &crate::svc::WinEntropy, now()).is_ok() {
                        cookie_count += 1;
                        to_check.push(id);
                        if let Some(a) = s.store.account_mut(id) {
                            if a.method == ram_core::model::SignInMethod::Unknown {
                                a.method = ram_core::model::SignInMethod::Cookie;
                            }
                            if a.sign_in.detail.starts_with("Imported without credentials") {
                                a.sign_in.detail = "Cookie imported; checking…".into();
                            }
                        }
                    }
                    if !lc.password.is_empty() && s.vault.set_password(id, &lc.password, &crate::svc::WinEntropy, now()).is_ok() {
                        password_count += 1;
                    }
                }
            }
            Err(e) => {
                s.save();
                return Err(format!("Accounts were imported, but cookies could not be read: {e}"));
            }
        }
    }
    s.log.info(&format!("import: {} added, {} updated, {} cookies, {} passwords", record.accounts_added, record.accounts_updated, cookie_count, password_count));
    s.save();
    drop(s);
    for id in to_check {
        crate::cmd_launch::start_check(app.clone(), id);
    }
    let _ = crate::cmd_launch::refresh_avatars(app.clone(), false);
    Ok(format!(
        "Imported {} account(s), updated {}, skipped {}{}{}.",
        record.accounts_added,
        record.accounts_updated,
        record.skipped,
        if with_cookies { format!(", {cookie_count} cookie(s) moved into the vault") } else { String::new() },
        if password_count > 0 { format!(", {password_count} saved password(s)") } else { String::new() }
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
    let mut s = lock(&state)?;
    let key = s.procs.list_clients().map_err(|e| e.to_string())?.into_iter().find(|p| p.key.pid == pid).map(|p| p.key);
    match key {
        Some(k) => {
            // Recorded as "Closed", not a crash, by the watcher.
            s.diag.closing.push(pid);
            s.log.info(&format!("asked client {pid} to close"));
            s.procs.request_close(k).map_err(|e| e.to_string())
        }
        None => Err("that client is no longer running".into()),
    }
}
