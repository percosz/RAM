//! Login checks and launching (Place ID / Job ID / Follow).
//!
//! Network work runs on worker threads; results are applied to the state and
//! announced with events so the front-end re-reads what changed.

use crate::dto::LaunchEvent;
use crate::state::{now, Shared};
use ram_core::model::{AccountId, RecentLaunch, Target};
use ram_core::roblox::{self, LaunchError, LaunchTarget};
use ram_core::roblox_profile;
use ram_core::signin::{self, CheckOutcome};
use ram_win::WinHttp;
use tauri::{AppHandle, Emitter, Manager, State};

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

fn emit_account(app: &AppHandle, id: AccountId) {
    let _ = app.emit("account-updated", id);
}

fn emit_launch(app: &AppHandle, account: AccountId, state: &str, message: impl Into<String>) {
    let _ = app.emit("launch-status", LaunchEvent { account, state: state.to_string(), message: message.into() });
}

fn describe(e: &LaunchError) -> String {
    match e {
        LaunchError::Unauthorized => "the stored cookie is no longer valid".into(),
        LaunchError::BoundSessionRequired => "Roblox requires a fresh sign-in for this account (bound session)".into(),
        LaunchError::Challenge => "Roblox asked for a verification step; open the browser for this account".into(),
        LaunchError::Offline => "this PC appears to be offline".into(),
        LaunchError::Timeout => "the request to Roblox timed out".into(),
        LaunchError::RateLimited { retry_after_secs: Some(s) } => format!("Roblox rate limit; retry in {s} s"),
        LaunchError::RateLimited { retry_after_secs: None } => "Roblox rate limit".into(),
        LaunchError::Service { status } => format!("Roblox returned HTTP {status}"),
        LaunchError::Malformed => "Roblox returned an unexpected response".into(),
    }
}

/// Runs the RoALT-style login check for one account on a worker thread:
/// `GET users/authenticated` with the cookie decides validity; Robux and
/// Premium are fetched alongside when it succeeds.
pub fn start_check(app: AppHandle, id: AccountId) {
    let cookie = {
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        if s.checking.contains(&id) {
            return;
        }
        let Some(c) = s.cookie(id) else {
            let configured = s.vault.is_configured();
            if let Some(a) = s.store.account_mut(id) {
                signin::apply(&mut a.sign_in, &CheckOutcome::NotAuthorized, now());
                a.sign_in.detail = if configured {
                    "No cookie stored. Add one, or sign in through Open Browser.".into()
                } else {
                    "Set up the vault and add a cookie to enable checks.".into()
                };
            }
            s.save();
            emit_account(&app, id);
            return;
        };
        s.checking.insert(id);
        c
    };
    emit_account(&app, id);
    std::thread::spawn(move || {
        let http = WinHttp::new();
        let result = roblox::whoami_full(&http, &cookie);
        let extras = result.as_ref().ok().map(|w| {
            (roblox_profile::robux(&http, &cookie, w.user.id).ok(), roblox_profile::premium(&http, &cookie, w.user.id).ok())
        });
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        s.checking.remove(&id);
        let t = now();
        // Roblox may rotate the cookie on a check; keep the vault current.
        if let Ok(w) = &result {
            if let Some(rotated) = &w.rotated_cookie {
                if s.vault.is_unlocked() {
                    let _ = s.vault.set_secret(id, rotated, &crate::svc::WinEntropy, t);
                }
            }
        }
        if let Some(a) = s.store.account_mut(id) {
            match result.map(|w| w.user) {
                Ok(user) => {
                    let outcome = match a.roblox_user_id {
                        Some(expected) if a.user_id_confirmed && expected != user.id => {
                            CheckOutcome::DifferentAccount { username: user.name.clone() }
                        }
                        _ => CheckOutcome::Confirmed,
                    };
                    signin::apply(&mut a.sign_in, &outcome, t);
                    if outcome == CheckOutcome::Confirmed {
                        a.roblox_user_id = Some(user.id);
                        a.user_id_confirmed = true;
                        a.display_name = user.display_name;
                        if a.username.is_empty() || !a.username.eq_ignore_ascii_case(&user.name) {
                            a.username = user.name;
                        }
                        if let Some((robux, premium)) = extras {
                            if robux.is_some() {
                                a.robux = robux;
                            }
                            if premium.is_some() {
                                a.premium = premium;
                            }
                        }
                    }
                }
                Err(e) => signin::apply(&mut a.sign_in, &e.to_sign_in_outcome(), t),
            }
        }
        s.save();
        drop(s);
        emit_account(&app, id);
    });
}

#[tauri::command]
pub fn check_accounts(app: AppHandle, ids: Vec<AccountId>) -> Result<(), String> {
    for id in ids {
        start_check(app.clone(), id);
    }
    Ok(())
}

/// Queues launches for several accounts to a place (optionally a specific
/// server), spaced by the configured delay. Accounts without a stored cookie
/// are reported and skipped.
#[tauri::command]
pub fn launch(app: AppHandle, state: State<'_, Shared>, ids: Vec<AccountId>, place_id: u64, job_id: Option<String>) -> Result<(), String> {
    if place_id == 0 {
        return Err("Enter a place ID.".into());
    }
    let job = job_id.map(|j| j.trim().to_string()).filter(|j| !j.is_empty());
    if let Some(j) = &job {
        if !ram_core::links::is_job_id(j) {
            return Err("The job ID is not in Roblox's format (a GUID).".into());
        }
    }
    let target = match &job {
        Some(j) => LaunchTarget::Server { place_id, job_id: j.to_ascii_lowercase() },
        None => LaunchTarget::Place { place_id },
    };
    let model_target = match &job {
        Some(j) => Target::LiveServer { place_id, job_id: j.to_ascii_lowercase() },
        None => Target::Place { place_id },
    };
    let spacing = {
        let s = lock(&state)?;
        s.store.settings.launch_spacing_secs
    };
    run_launches(app, ids, target, model_target, spacing);
    Ok(())
}

/// Follows a user by username: resolves the id, then launches into their
/// server for each selected account.
#[tauri::command]
pub fn follow(app: AppHandle, state: State<'_, Shared>, ids: Vec<AccountId>, username: String) -> Result<(), String> {
    let username = username.trim().to_string();
    if username.is_empty() {
        return Err("Enter a username to follow.".into());
    }
    let spacing = {
        let s = lock(&state)?;
        s.store.settings.launch_spacing_secs
    };
    std::thread::spawn(move || {
        let http = WinHttp::new();
        match roblox_profile::resolve_username(&http, &username) {
            Ok(user_id) => {
                let target = LaunchTarget::Follow { user_id };
                run_launches(app, ids, target, Target::Place { place_id: 0 }, spacing);
            }
            Err(e) => {
                for id in ids {
                    emit_launch(&app, id, "Failed", format!("Could not find {username}: {}", describe(&e)));
                }
            }
        }
    });
    Ok(())
}

fn run_launches(app: AppHandle, ids: Vec<AccountId>, target: LaunchTarget, model_target: Target, spacing: u32) {
    std::thread::spawn(move || {
        let http = WinHttp::new();
        let mut first = true;
        for id in ids {
            if !first && spacing > 0 {
                std::thread::sleep(std::time::Duration::from_secs(spacing as u64));
            }
            first = false;
            let cookie = {
                let st = app.state::<Shared>();
                let Ok(mut s) = st.lock() else { return };
                s.cookie(id)
            };
            let Some(cookie) = cookie else {
                emit_launch(&app, id, "Failed", "No cookie stored. Add one, or sign in through Open Browser.");
                continue;
            };
            emit_launch(&app, id, "Launching", "Requesting a ticket…");
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            match roblox::prepare_launch(&http, &cookie, &target, None, now_ms) {
                Ok((uri, _btid)) => match ram_win::launch::launch_protocol(&uri) {
                    Ok(()) => {
                        let st = app.state::<Shared>();
                        if let Ok(mut s) = st.lock() {
                            let t = now();
                            if let Some(a) = s.store.account_mut(id) {
                                a.last_used = Some(t);
                            }
                            if model_target.place_id().unwrap_or(0) != 0 {
                                s.store.push_recent(RecentLaunch { account: id, target: model_target.clone(), at: t });
                            }
                            s.save();
                        }
                        emit_launch(&app, id, "Launching", "Client starting…");
                    }
                    Err(e) => emit_launch(&app, id, "Failed", format!("Could not start the Roblox client: {e}")),
                },
                Err(LaunchError::Unauthorized) => {
                    let st = app.state::<Shared>();
                    if let Ok(mut s) = st.lock() {
                        if let Some(a) = s.store.account_mut(id) {
                            signin::apply(&mut a.sign_in, &CheckOutcome::AuthorizationRejected, now());
                        }
                        s.save();
                    }
                    emit_account(&app, id);
                    emit_launch(&app, id, "Failed", "The stored cookie is no longer valid. Sign in again through Open Browser.");
                }
                Err(e) => emit_launch(&app, id, "Failed", describe(&e)),
            }
        }
    });
}
