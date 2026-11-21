//! Adding accounts: sign in with Roblox in your own browser (RAM captures
//! the session by itself), paste a cookie, paste many lines at once, or a
//! queue of `username:password` lines that RAM fills into Roblox's sign-in
//! page one account at a time. Every path ends in `adopt_cookie`, which asks
//! Roblox who the session belongs to and creates or refreshes that account,
//! so the user never types a username.
//!
//! Passwords are only ever typed into Roblox's own sign-in page in the
//! user's browser. They are kept (encrypted in the vault, like cookies) only
//! when the user leaves "Save passwords" on.

use crate::dto::{self, AccountView};
use crate::state::{now, Shared};
use ram_core::cookies::{self, PastedLine};
use ram_core::model::{Account, AccountId, SignInMethod};
use ram_core::roblox::{self, LaunchError};
use ram_core::signin::{self, CheckOutcome};
use ram_win::sysbrowser::{self, LoginFill};
use ram_win::WinHttp;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroizing;

const ROBLOX_LOGIN: &str = "https://www.roblox.com/login";
/// How long one queued account may sit on the sign-in page.
const QUEUE_ITEM_TIMEOUT: Duration = Duration::from_secs(240);

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
/// (or refreshes the existing one) and stores the cookie in the vault,
/// along with the password when one is given.
pub fn adopt_cookie(app: &AppHandle, cookie: &str, alias: &str, method: SignInMethod, password: Option<&str>) -> Result<Adopted, String> {
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
            a.method = method;
            if !alias.trim().is_empty() && a.alias.is_empty() {
                a.alias = alias.trim().to_string();
            }
            signin::apply(&mut a.sign_in, &CheckOutcome::Confirmed, t);
        }
        s.vault.set_secret(id, stored, &crate::svc::WinEntropy, t).map_err(|e| e.to_string())?;
        if let Some(p) = password {
            s.vault.set_password(id, p, &crate::svc::WinEntropy, t).map_err(|e| e.to_string())?;
        }
        s.log.info(&format!(
            "account {id}: {} with a confirmed session (user {}, by {:?}{})",
            if added { "added" } else { "refreshed" },
            user.id,
            method,
            if password.is_some() { ", password saved" } else { "" }
        ));
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
        PastedLine::Password { .. } => return Err("That is a username and password. Put it in Add many at once: RAM signs it in through your browser.".into()),
        _ => return Err("That does not look like a .ROBLOSECURITY cookie.".into()),
    };
    tauri::async_runtime::spawn_blocking(move || adopt_cookie(&app, &value, &alias, SignInMethod::Cookie, None)).await.map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone, Default)]
pub struct ManyResult {
    pub added: u32,
    pub refreshed: u32,
    pub repeated: u32,
    pub failed: u32,
    /// `username:password` lines handed to the sign-in queue.
    pub queued: u32,
    /// One line per failure, without the cookie.
    pub problems: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct ManyProgress {
    pub done: u32,
    pub total: u32,
}

/// Splits `username:password`; the password may itself contain colons.
fn split_userpass(line: &str) -> Option<(String, String)> {
    let s = line.trim();
    let (u, p) = s.split_once(':')?;
    let u = u.trim();
    let ok = u.len() >= 3 && u.len() <= 20 && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (ok && !p.is_empty()).then(|| (u.to_string(), p.to_string()))
}

/// Add many at once: cookies one per line are checked and added right away
/// (`user:pass:cookie` lines keep the cookie); `username:password` lines go
/// to the sign-in queue, which fills Roblox's sign-in page one account at a
/// time.
#[tauri::command]
pub async fn add_many(app: AppHandle, text: String, save_passwords: bool) -> Result<ManyResult, String> {
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let mut r = ManyResult::default();
        let total = lines.iter().filter(|l| !l.trim().is_empty()).count() as u32;
        let mut seen: Vec<AccountId> = Vec::new();
        let mut queue: Vec<QueueItem> = Vec::new();
        let mut done = 0u32;
        for (n, line) in lines.iter().enumerate() {
            match cookies::parse_line(line) {
                PastedLine::Blank => continue,
                PastedLine::Password { .. } | PastedLine::Unrecognised => match split_userpass(line) {
                    Some((username, password)) => queue.push(QueueItem { username, password: Zeroizing::new(password) }),
                    None => {
                        r.failed += 1;
                        r.problems.push(format!("line {}: neither a cookie nor username:password.", n + 1));
                    }
                },
                PastedLine::Cookie(c) => {
                    match adopt_cookie(&app, &c, "", SignInMethod::Cookie, None) {
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
                    }
                    // Be gentle with Roblox: about one check per second.
                    std::thread::sleep(Duration::from_millis(700));
                }
            }
            done += 1;
            let _ = app.emit("add-many-progress", ManyProgress { done, total });
        }
        r.queued = queue.len() as u32;
        if !queue.is_empty() {
            start_queue(&app, queue, save_passwords)?;
        }
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- the username:password sign-in queue -----------------------------------

pub struct QueueItem {
    pub username: String,
    pub password: Zeroizing<String>,
}

/// One queue at a time, worked through on its own thread.
#[derive(Default)]
pub struct SignInQueue {
    items: VecDeque<QueueItem>,
    total: u32,
    index: u32,
    running: bool,
    skip: bool,
    stop: bool,
    save_passwords: bool,
    done: u32,
    failed: u32,
}

/// Progress of the queue, sent as `signin-queue` events.
#[derive(Serialize, Clone)]
pub struct QueueEvent {
    pub index: u32,
    pub total: u32,
    pub username: String,
    /// opening | filling | waiting | done | failed | skipped | finished
    pub state: String,
    pub message: String,
    pub done: u32,
    pub failed: u32,
}

fn emit_queue(app: &AppHandle, username: &str, state: &str, message: impl Into<String>) {
    let (index, total, done, failed) = {
        let st = app.state::<Shared>();
        let Ok(s) = st.lock() else { return };
        (s.signin_queue.index, s.signin_queue.total, s.signin_queue.done, s.signin_queue.failed)
    };
    let _ = app.emit("signin-queue", QueueEvent { index, total, username: username.to_string(), state: state.to_string(), message: message.into(), done, failed });
}

fn start_queue(app: &AppHandle, items: Vec<QueueItem>, save_passwords: bool) -> Result<(), String> {
    {
        let st = app.state::<Shared>();
        let mut s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        let q = &mut s.signin_queue;
        q.total = if q.running { q.total + items.len() as u32 } else { items.len() as u32 };
        if !q.running {
            q.index = 0;
            q.done = 0;
            q.failed = 0;
            q.stop = false;
        }
        q.save_passwords = save_passwords;
        q.items.extend(items);
        if q.running {
            return Ok(());
        }
        q.running = true;
    }
    let app = app.clone();
    std::thread::spawn(move || run_queue(app));
    Ok(())
}

enum Outcome {
    Done(String),
    Failed(String),
    Skipped,
    Stopped,
}

fn run_queue(app: AppHandle) {
    let dir = {
        let st = app.state::<Shared>();
        let Ok(s) = st.lock() else { return };
        signin_profile(&s.data_dir)
    };
    loop {
        let (item, save) = {
            let st = app.state::<Shared>();
            let Ok(mut s) = st.lock() else { return };
            let q = &mut s.signin_queue;
            if q.stop {
                q.items.clear();
            }
            match q.items.pop_front() {
                Some(item) => {
                    q.index += 1;
                    q.skip = false;
                    (item, q.save_passwords)
                }
                None => {
                    q.running = false;
                    break;
                }
            }
        };
        let outcome = sign_in_one(&app, &dir, &item, save);
        let (state, message) = {
            let st = app.state::<Shared>();
            let Ok(mut s) = st.lock() else { return };
            match &outcome {
                Outcome::Done(m) => {
                    s.signin_queue.done += 1;
                    ("done", m.clone())
                }
                Outcome::Failed(m) => {
                    s.signin_queue.failed += 1;
                    s.log.warn(&format!("sign-in queue: {} failed: {m}", item.username));
                    ("failed", m.clone())
                }
                Outcome::Skipped => {
                    s.signin_queue.failed += 1;
                    ("skipped", "Skipped.".to_string())
                }
                Outcome::Stopped => {
                    s.signin_queue.stop = true;
                    ("skipped", "Stopped.".to_string())
                }
            }
        };
        emit_queue(&app, &item.username, state, message);
        // Sign the window out locally so the next account starts clean.
        let _ = sysbrowser::clear_cookies(&dir);
    }
    if sysbrowser::is_running(&dir) {
        let _ = sysbrowser::clear_cookies(&dir);
        let _ = sysbrowser::close(&dir);
    }
    emit_queue(&app, "", "finished", "");
    if let Ok(s) = app.state::<Shared>().lock() {
        s.log.info(&format!("sign-in queue finished: {} signed in, {} not", s.signin_queue.done, s.signin_queue.failed));
    }
}

fn queue_flags(app: &AppHandle) -> (bool, bool) {
    let st = app.state::<Shared>();
    let flags = match st.lock() {
        Ok(s) => (s.signin_queue.skip, s.signin_queue.stop),
        Err(_) => (false, true),
    };
    flags
}

/// Opens the sign-in page, fills it for one account and waits for the
/// session to appear.
fn sign_in_one(app: &AppHandle, dir: &std::path::Path, item: &QueueItem, save_password: bool) -> Outcome {
    emit_queue(app, &item.username, "opening", "Opening Roblox's sign-in page…");
    let browser = {
        let st = app.state::<Shared>();
        let Ok(s) = st.lock() else { return Outcome::Stopped };
        crate::cmd_browser::pick_browser_for(&s.store.settings.browser)
    };
    let Some(browser) = browser else {
        return Outcome::Failed("No Chromium-based browser (Brave, Chrome, Edge, Vivaldi) is installed.".into());
    };
    let opened = if sysbrowser::is_running(dir) {
        let _ = sysbrowser::clear_cookies(dir);
        sysbrowser::open_page(dir, ROBLOX_LOGIN)
    } else {
        let _ = sysbrowser::clear_cookies(dir);
        sysbrowser::open_signed_in(&browser, dir, None, ROBLOX_LOGIN, "Sign in to Roblox").map(|_| ())
    };
    if let Err(e) = opened {
        return Outcome::Failed(e);
    }
    emit_queue(app, &item.username, "filling", "Filling in the sign-in form…");
    let mut filled = LoginFill::NoForm;
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(900));
        match sysbrowser::fill_login(dir, &item.username, &item.password, true) {
            Ok(LoginFill::NoForm) => continue,
            Ok(f) => {
                filled = f;
                break;
            }
            Err(e) => return Outcome::Failed(e),
        }
    }
    let hint = match filled {
        LoginFill::Submitted => format!("Signing in… solve any check in the {} window.", browser.name),
        LoginFill::Filled => format!("Filled in. Press Sign in in the {} window.", browser.name),
        LoginFill::NoForm => format!("The form did not appear. Sign in by hand in the {} window, or skip.", browser.name),
    };
    emit_queue(app, &item.username, "waiting", hint);
    let started = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(1500));
        let (skip, stop) = queue_flags(app);
        if stop {
            return Outcome::Stopped;
        }
        if skip {
            return Outcome::Skipped;
        }
        if !sysbrowser::is_running(dir) {
            // Closing the window ends the whole queue, like RoALT.
            return Outcome::Stopped;
        }
        if let Ok(Some(cookie)) = sysbrowser::read_cookie(dir) {
            let pw = save_password.then(|| item.password.as_str());
            return match adopt_cookie(app, &cookie, "", SignInMethod::Login, pw) {
                Ok(a) if a.account.username.eq_ignore_ascii_case(&item.username) => Outcome::Done(if a.added { "Added.".into() } else { "Refreshed.".into() }),
                Ok(a) => Outcome::Done(format!("Signed in as {} (not {}).", a.account.username, item.username)),
                Err(e) => Outcome::Failed(e),
            };
        }
        if started.elapsed() > QUEUE_ITEM_TIMEOUT {
            return Outcome::Failed("No sign-in within 4 minutes (wrong password, or an unanswered check).".into());
        }
    }
}

/// Skips the account the queue is waiting on.
#[tauri::command]
pub fn signin_queue_skip(state: State<'_, Shared>) -> Result<(), String> {
    lock(&state)?.signin_queue.skip = true;
    Ok(())
}

/// Stops the queue; accounts already signed in stay.
#[tauri::command]
pub fn signin_queue_stop(state: State<'_, Shared>) -> Result<(), String> {
    lock(&state)?.signin_queue.stop = true;
    Ok(())
}

/// Sign in with Roblox: opens the sign-in page in the user's browser on a
/// profile of its own. The front-end then polls `signin_poll`.
#[tauri::command]
pub fn signin_start(state: State<'_, Shared>) -> Result<String, String> {
    let (dir, choice) = {
        let s = lock(&state)?;
        if !s.vault.is_configured() {
            return Err("Set up the cookie vault first.".into());
        }
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        if s.signin_queue.running {
            return Err("The sign-in queue is using the sign-in window. Wait for it, or stop it.".into());
        }
        (signin_profile(&s.data_dir), s.store.settings.browser.clone())
    };
    let browser = crate::cmd_browser::pick_browser_for(&choice).ok_or("RAM can only drive Chromium-based browsers (Brave, Chrome, Edge, Vivaldi) and none is installed.")?;
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
    let adopted = tauri::async_runtime::spawn_blocking(move || adopt_cookie(&app2, &cookie, "", SignInMethod::Login, None)).await.map_err(|e| e.to_string())?;
    // The vault has it now: sign the window out and close it.
    let _ = sysbrowser::clear_cookies(&dir);
    let _ = sysbrowser::close(&dir);
    adopted.map(SignInPoll::Done)
}

/// Closes the sign-in window without capturing anything.
#[tauri::command]
pub fn signin_cancel(state: State<'_, Shared>) -> Result<(), String> {
    let dir = {
        let s = lock(&state)?;
        if s.signin_queue.running {
            return Ok(());
        }
        signin_profile(&s.data_dir)
    };
    if sysbrowser::is_running(&dir) {
        let _ = sysbrowser::clear_cookies(&dir);
        let _ = sysbrowser::close(&dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userpass_lines_split_on_the_first_colon() {
        assert_eq!(split_userpass("bob_1:hunter2"), Some(("bob_1".into(), "hunter2".into())));
        assert_eq!(split_userpass("  bob_1:pa:ss:word  "), Some(("bob_1".into(), "pa:ss:word".into())));
        assert_eq!(split_userpass("ab:x"), None); // username too short
        assert_eq!(split_userpass("bob:"), None);
        assert_eq!(split_userpass("no colon here"), None);
        assert_eq!(split_userpass("bad name:x"), None);
    }
}
