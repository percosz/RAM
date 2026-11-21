//! "Open Browser": the user's own default browser (Brave, Chrome, Edge…)
//! opened on roblox.com with a separate browser profile per account and
//! the account's session cookie injected through the Chrome DevTools
//! protocol on a loopback port the browser opens for us. Nothing is
//! embedded: the window is the real browser, extensions and all.
//!
//! The vault stays the only place a cookie lives between sessions: when the
//! browser window closes, the profile's cookie database is deleted (a
//! watcher thread waits for the browser process), and a leftover one is
//! deleted at the next start too. "Capture session" reads the live cookie
//! back out of the open window after a manual sign-in.
//!
//! Firefox cannot be driven this way; when it is the default and no
//! Chromium-based browser is installed, the command explains that.

use crate::state::{now, Shared};
use ram_core::model::AccountId;
use ram_win::sysbrowser::{self, Browser};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

const ROBLOX_HOME: &str = "https://www.roblox.com/home";
const ROBLOX_LOGIN: &str = "https://www.roblox.com/login";

/// The browser Open Browser will use: the default one when it is
/// Chromium-based, else the first Chromium-based browser installed.
pub fn pick_browser() -> Option<Browser> {
    match sysbrowser::default_browser() {
        Some(b) if b.chromium => Some(b),
        _ => sysbrowser::known_chromium_browsers().into_iter().next(),
    }
}

/// The browser to use given the Settings choice: a full path picks that
/// browser when it still exists and is Chromium-based; empty (or a stale
/// path) follows the Windows default, read fresh every time.
pub fn pick_browser_for(choice: &str) -> Option<Browser> {
    let choice = choice.trim();
    if !choice.is_empty() {
        let path = PathBuf::from(choice);
        if path.is_file() {
            if let Some(b) = installed_browsers().into_iter().find(|b| b.exe.to_string_lossy().eq_ignore_ascii_case(choice)) {
                return Some(b);
            }
        }
    }
    pick_browser()
}

/// Chromium-based browsers found on this PC, the Windows default first.
pub fn installed_browsers() -> Vec<Browser> {
    let mut out: Vec<Browser> = Vec::new();
    if let Some(b) = sysbrowser::default_browser() {
        if b.chromium {
            out.push(b);
        }
    }
    for b in sysbrowser::known_chromium_browsers() {
        if !out.iter().any(|x| x.exe.to_string_lossy().eq_ignore_ascii_case(&b.exe.to_string_lossy())) {
            out.push(b);
        }
    }
    out
}

/// A profile folder belongs to the browser that created it. When the
/// browser changes (new default, or a new choice in Settings) the old
/// profile is wiped so the new browser starts from a clean one instead of
/// tripping over another browser's files.
fn claim_profile(dir: &std::path::Path, browser: &Browser) {
    let marker = dir.join(".ram-browser");
    let want = browser.exe.to_string_lossy().to_ascii_lowercase();
    match std::fs::read_to_string(&marker) {
        Ok(have) if have.trim() == want => {}
        Ok(_) => {
            if !sysbrowser::is_running(dir) {
                let _ = sysbrowser::wipe_profile(dir);
            }
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&marker, &want);
        }
        Err(_) => {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&marker, &want);
        }
    }
}

#[derive(serde::Serialize, Clone)]
pub struct BrowserChoice {
    pub name: String,
    pub path: String,
    pub is_default: bool,
}

/// Browsers the user can pick in Settings.
#[tauri::command]
pub fn list_browsers() -> Vec<BrowserChoice> {
    let default = sysbrowser::default_browser().map(|b| b.exe.to_string_lossy().to_ascii_lowercase());
    installed_browsers()
        .into_iter()
        .map(|b| {
            let path = b.exe.to_string_lossy().to_string();
            BrowserChoice { is_default: default.as_deref() == Some(path.to_ascii_lowercase().as_str()), name: b.name, path }
        })
        .collect()
}

pub fn profile_dir(data_dir: &std::path::Path, id: AccountId) -> PathBuf {
    data_dir.join("browser-profiles").join(id.to_string())
}

/// Deletes cookie databases left in profiles whose browser is not running
/// (a crash, or RAM closed while a window was open).
pub fn sweep_profiles(data_dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(data_dir.join("browser-profiles")) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && !sysbrowser::is_running(&p) {
            let _ = sysbrowser::clear_cookies(&p);
        }
    }
}

/// Opens (or focuses) the account's browser window signed in with the
/// stored cookie; without one it shows the sign-in page in that account's
/// own profile, ready for `browser_capture`. Returns a short note for the
/// UI when there is something to say.
#[tauri::command]
pub fn open_browser(app: AppHandle, state: State<'_, Shared>, id: AccountId) -> Result<String, String> {
    let (cookie, dir, name, choice) = {
        let mut s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        let cookie = s.cookie(id);
        let dir = profile_dir(&s.data_dir, id);
        let name = s.account_name(id);
        (cookie, dir, name, s.store.settings.browser.clone())
    };
    let browser = pick_browser_for(&choice).ok_or_else(|| {
        let d = sysbrowser::default_browser().map(|b| b.name).unwrap_or_else(|| "your default browser".into());
        format!("RAM can only drive Chromium-based browsers (Brave, Chrome, Edge, Vivaldi). {d} is the default and none of those is installed.")
    })?;
    claim_profile(&dir, &browser);
    let was_running = sysbrowser::is_running(&dir);
    let url = if cookie.is_some() { ROBLOX_HOME } else { ROBLOX_LOGIN };
    let session = sysbrowser::open_signed_in(&browser, &dir, cookie.as_deref(), url, &name)?;
    {
        let s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        s.log.info(&format!("browser: opened {} for account {id} ({}){}", browser.name, if cookie.is_some() { "signed in" } else { "sign-in page" }, if was_running { ", reused the open window" } else { "" }));
    }
    if !was_running {
        // When the window closes, drop the session from the profile.
        let app2 = app.clone();
        let dir2 = dir.clone();
        let pid = session.pid;
        std::thread::spawn(move || {
            // While the window is open, keep the vault in step with it: a
            // sign-in done there, or a cookie Roblox rotated, is captured
            // without the user pressing anything (only when it signs in as
            // this same account).
            let mut last_seen: Option<String> = None;
            while !sysbrowser::wait_exit(pid, Duration::from_secs(4)) {
                if let Ok(Some(c)) = sysbrowser::read_cookie(&dir2) {
                    if last_seen.as_deref() != Some(c.as_str()) {
                        last_seen = Some(c.clone());
                        capture_if_same_account(&app2, id, &c);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(800));
            let _ = sysbrowser::clear_cookies(&dir2);
            if let Ok(s) = app2.state::<Shared>().lock() {
                s.log.info(&format!("browser: window for account {id} closed; profile cookies cleared"));
            }
        });
    }
    Ok(if cookie.is_some() {
        String::new()
    } else {
        format!("No cookie stored: sign in inside {}; RAM saves the session by itself.", browser.name)
    })
}

/// Reads the `.ROBLOSECURITY` cookie from the account's open browser
/// window and stores it in the vault, then starts a login check.
#[tauri::command]
pub fn browser_capture(app: AppHandle, state: State<'_, Shared>, id: AccountId) -> Result<(), String> {
    let dir = {
        let s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        profile_dir(&s.data_dir, id)
    };
    let value = sysbrowser::read_cookie(&dir)?.ok_or("No Roblox session in that window yet. Sign in there, then try again.")?;
    {
        let mut s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        s.vault.set_secret(id, &value, &crate::svc::WinEntropy, now()).map_err(|e| e.to_string())?;
        s.log.info(&format!("browser: captured a session for account {id}"));
    }
    crate::cmd_launch::start_check(app, id);
    Ok(())
}

/// Closes the account's browser window and clears the profile's cookies.
#[tauri::command]
pub fn browser_clear(state: State<'_, Shared>, id: AccountId) -> Result<(), String> {
    let dir = {
        let s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        profile_dir(&s.data_dir, id)
    };
    if sysbrowser::is_running(&dir) {
        let _ = sysbrowser::clear_cookies(&dir);
        let _ = sysbrowser::close(&dir);
    }
    let _ = sysbrowser::clear_cookies(&dir);
    Ok(())
}

/// Stores `cookie` for account `id` when it differs from the stored one and
/// Roblox says it belongs to that account (or the account has no confirmed
/// identity yet). Used by the open-window watcher.
fn capture_if_same_account(app: &AppHandle, id: AccountId, cookie: &str) {
    let (stored, expected_id, expected_name) = {
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        if !s.vault.is_unlocked() {
            return;
        }
        let stored = s.cookie(id);
        let Some(a) = s.store.account(id) else { return };
        (stored, if a.user_id_confirmed { a.roblox_user_id } else { None }, a.username.clone())
    };
    if stored.as_deref() == Some(cookie) {
        return;
    }
    let Ok(who) = ram_core::roblox::whoami_full(&ram_win::WinHttp::new(), cookie) else { return };
    let same = match expected_id {
        Some(uid) => uid == who.user.id,
        None => who.user.name.eq_ignore_ascii_case(&expected_name),
    };
    let st = app.state::<Shared>();
    let Ok(mut s) = st.lock() else { return };
    if !same {
        s.log.warn(&format!("browser: the window for account {id} is signed in as a different user; not captured"));
        return;
    }
    let value = who.rotated_cookie.as_deref().unwrap_or(cookie);
    if s.vault.set_secret(id, value, &crate::svc::WinEntropy, now()).is_ok() {
        s.log.info(&format!("browser: captured the session for account {id} from its window"));
    }
    drop(s);
    crate::cmd_launch::start_check(app.clone(), id);
}
