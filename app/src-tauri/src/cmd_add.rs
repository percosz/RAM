//! Adding accounts the way RoALT does: sign in with Roblox in your own
//! browser (RAM captures the session by itself), paste a cookie, or paste
//! many lines at once. Every path ends in `adopt_cookie`, which asks Roblox
//! who the cookie belongs to and creates or refreshes that account, so the
//! user never types a username.

use crate::dto::{self, AccountView};
use crate::state::{now, Shared};
use ram_core::cookies::{self, PastedLine};
use ram_core::model::{Account, AccountId};
use ram_core::roblox::{self, LaunchError};
use ram_core::signin::{self, CheckOutcome};
use ram_win::sysbrowser;
use ram_win::WinHttp;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

const ROBLOX_LOGIN: &str = "https://www.roblox.com/login";

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

/// The browser profile used for "Sign in with Roblox". Its cookies are
/// cleared as soon as the session has been captured.
fn signin_profile(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("browser-profiles").join("sign-in")
}

fn describe(e: &LaunchError) -> String {
    match e {
        LaunchError::Unauthorized => "Roblox does not accept that cookie (signed out or expired).".into(),
        LaunchError::BoundSessionRequired => "Roblox says this session is bound to its browser; sign in with Roblox instead.".into(),
        LaunchError::Challenge => "Roblox wants a verification step for this account; sign in with Roblox instead.".into(),
        LaunchError::Offline => "This PC appears to be offline.".into(),
        LaunchError::Timeout => "The request to Roblox timed out.".into(),
        LaunchError::RateLimited { retry_after_secs: Some(s) } => format!("Roblox rate limit; retry in {s} s."),
        LaunchError::RateLimited { retry_after_secs: None } => "Roblox rate limit; try again in a minute.".into(),
        LaunchError::Service { status } => format!("Roblox returned HTTP {status}."),
        LaunchError::Malformed => "Roblox returned an unexpected response.".into(),
    }
}

/// Result of adopting one cookie.
#[derive(Serialize, Clone)]
pub struct Adopted {
    pub account: AccountView,
    /// True when the account was created, false when an existing one was refreshed.
    pub added: bool,
}

/// Confirms the cookie with Roblox, then creates the account it belongs to
/// (or refreshes the existing one) and stores the cookie in the vault.
pub fn adopt_cookie(app: &AppHandle, cookie: &str, alias: &str) -> Result<Adopted, String> {
    {
        let st = app.state::<Shared>();
        let s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        if !s.vault.is_configured() {
            return Err("Set up the cookie vault first (Settings → Security).".into());
        }
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
    }
    let http = WinHttp::new();
    let who = roblox::whoami_full(&http, cookie).map_err(|e| describe(&e))?;
    let stored = who.rotated_cookie.as_deref().unwrap_or(cookie);
    let (id, added) = {
        let st = app.state::<Shared>();
        let mut s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        let t = now();
        let user = &who.user;
        let existing = s
            .store
            .accounts
            .iter()
            .find(|a| a.roblox_user_id == Some(user.id) || a.username.eq_ignore_ascii_case(&user.name))
            .map(|a| a.id);
        let (id, added) = match existing {
            Some(id) => (id, false),
            None => {
                let id = s.store.alloc_id();
                s.store.accounts.push(Account { id, username: user.name.clone(), alias: alias.trim().to_string(), created: t, ..Default::default() });
                (id, true)
            }
        };
        if let Some(a) = s.store.account_mut(id) {
            a.username = user.name.clone();
            a.display_name = user.display_name.clone();
            a.roblox_user_id = Some(user.id);
            a.user_id_confirmed = true;
            if !alias.trim().is_empty() && a.alias.is_empty() {
                a.alias = alias.trim().to_string();
            }
            signin::apply(&mut a.sign_in, &CheckOutcome::Confirmed, t);
        }
        s.vault.set_secret(id, stored, &crate::svc::WinEntropy, t).map_err(|e| e.to_string())?;
        s.log.info(&format!("account {id}: {} with a confirmed cookie (user {})", if added { "added" } else { "refreshed" }, user.id));
        s.save();
        (id, added)
    };
    let _ = app.emit("account-updated", id);
    // Robux, Premium and the headshot come with the regular check.
    crate::cmd_launch::start_check(app.clone(), id);
    let _ = crate::cmd_launch::refresh_avatars(app.clone(), false);
    let st = app.state::<Shared>();
    let s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
    let account = dto::account_view(&s, id).ok_or("account not found")?;
    Ok(Adopted { account, added })
}

/// Paste a cookie: one account from one `.ROBLOSECURITY` value.
#[tauri::command]
pub async fn add_by_cookie(app: AppHandle, cookie: String, alias: String) -> Result<Adopted, String> {
    let value = match cookies::parse_line(&cookie) {
        PastedLine::Cookie(c) => c,
        PastedLine::Password { .. } => return Err("That is a username and password. RAM never signs in with passwords; use Sign in with Roblox.".into()),
        _ => return Err("That does not look like a .ROBLOSECURITY cookie.".into()),
    };
    tauri::async_runtime::spawn_blocking(move || adopt_cookie(&app, &value, &alias)).await.map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone, Default)]
pub struct ManyResult {
    pub added: u32,
    pub refreshed: u32,
    pub repeated: u32,
    pub failed: u32,
    /// One line per failure, without the cookie.
    pub problems: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct ManyProgress {
    pub done: u32,
    pub total: u32,
}

/// Add many at once: cookies one per line (`user:pass:cookie` lines keep the
/// cookie). `username:password` lines are reported, never used.
#[tauri::command]
pub async fn add_many(app: AppHandle, text: String) -> Result<ManyResult, String> {
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let mut r = ManyResult::default();
        let total = lines.iter().filter(|l| !l.trim().is_empty()).count() as u32;
        let mut seen: Vec<AccountId> = Vec::new();
        let mut done = 0u32;
        for (n, line) in lines.iter().enumerate() {
            match cookies::parse_line(line) {
                PastedLine::Blank => continue,
                PastedLine::Password { username } => {
                    r.failed += 1;
                    r.problems.push(format!("line {}: {username} is a username and password; RAM never uses passwords. Sign in with Roblox instead.", n + 1));
                }
                PastedLine::Unrecognised => {
                    r.failed += 1;
                    r.problems.push(format!("line {}: not a cookie.", n + 1));
                }
                PastedLine::Cookie(c) => match adopt_cookie(&app, &c, "") {
                    Ok(a) => {
                        if seen.contains(&a.account.id) {
                            r.repeated += 1;
                        } else if a.added {
                            r.added += 1;
                        } else {
                            r.refreshed += 1;
                        }
                        seen.push(a.account.id);
                    }
                    Err(e) => {
                        r.failed += 1;
                        r.problems.push(format!("line {}: {e}", n + 1));
                    }
                },
            }
            done += 1;
            let _ = app.emit("add-many-progress", ManyProgress { done, total });
            // Be gentle with Roblox: one check per second.
            std::thread::sleep(std::time::Duration::from_millis(700));
        }
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sign in with Roblox: opens the sign-in page in the user's browser on a
/// profile of its own. The front-end then polls `signin_poll`.
#[tauri::command]
pub fn signin_start(state: State<'_, Shared>) -> Result<String, String> {
    let browser = crate::cmd_browser::pick_browser().ok_or("RAM can only drive Chromium-based browsers (Brave, Chrome, Edge, Vivaldi) and none is installed.")?;
    let dir = {
        let s = lock(&state)?;
        if !s.vault.is_configured() {
            return Err("Set up the cookie vault first.".into());
        }
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        signin_profile(&s.data_dir)
    };
    // Always start signed out, so the window asks for the account you want.
    let _ = sysbrowser::clear_cookies(&dir);
    sysbrowser::open_signed_in(&browser, &dir, None, ROBLOX_LOGIN, "Sign in to Roblox")?;
    if let Ok(s) = state.lock() {
        s.log.info(&format!("sign-in: opened {} on the sign-in page", browser.name));
    }
    Ok(browser.name)
}

/// What the sign-in window is doing.
#[derive(Serialize, Clone)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SignInPoll {
    /// The window is open and nobody has signed in yet.
    Waiting,
    /// The window was closed before a sign-in happened.
    Closed,
    /// A session appeared and was captured into the vault.
    Done(Adopted),
}

/// Looks for a Roblox session in the sign-in window; when one is there it is
/// captured, the window is signed out and closed, and the account appears.
#[tauri::command]
pub async fn signin_poll(app: AppHandle) -> Result<SignInPoll, String> {
    let dir = {
        let st = app.state::<Shared>();
        let s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        signin_profile(&s.data_dir)
    };
    if !sysbrowser::is_running(&dir) {
        return Ok(SignInPoll::Closed);
    }
    let cookie = match sysbrowser::read_cookie(&dir) {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(SignInPoll::Waiting),
        Err(_) => return Ok(SignInPoll::Closed),
    };
    let app2 = app.clone();
    let adopted = tauri::async_runtime::spawn_blocking(move || adopt_cookie(&app2, &cookie, "")).await.map_err(|e| e.to_string())?;
    // The vault has it now: sign the window out and close it.
    let _ = sysbrowser::clear_cookies(&dir);
    let _ = sysbrowser::close(&dir);
    match adopted {
        Ok(a) => Ok(SignInPoll::Done(a)),
        Err(e) => Err(e),
    }
}

/// Closes the sign-in window without capturing anything.
#[tauri::command]
pub fn signin_cancel(state: State<'_, Shared>) -> Result<(), String> {
    let dir = {
        let s = lock(&state)?;
        signin_profile(&s.data_dir)
    };
    if sysbrowser::is_running(&dir) {
        let _ = sysbrowser::clear_cookies(&dir);
        let _ = sysbrowser::close(&dir);
    }
    Ok(())
}
