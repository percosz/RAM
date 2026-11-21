//! "Open Browser": a per-account browser window (its own WebView2 profile
//! folder) with the account's session cookie injected, so roblox.com opens
//! signed in as that account. The same window can capture the cookie after a
//! manual sign-in, which is how an account is added without pasting.
//!
//! Model follows alyae (embedded webview, cookie injected, navigation kept to
//! Roblox hosts) and RoALT (one profile per account; the vault stays the only
//! store, so the profile's browsing data is cleared when the window closes).

use crate::state::{now, Shared};
use ram_core::model::AccountId;
use tauri::webview::Cookie;
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

const ROBLOX_HOME: &str = "https://www.roblox.com/home";
const ROBLOX_LOGIN: &str = "https://www.roblox.com/login";

/// Hosts the browser window may navigate to: Roblox itself, its CDN, and the
/// CAPTCHA providers it embeds.
const ALLOWED_SUFFIXES: &[&str] = &[
    "roblox.com", "rbxcdn.com", "roblox.net", "rbxtrk.com", "arkoselabs.com", "funcaptcha.com", "hcaptcha.com",
];

pub fn label(id: AccountId) -> String {
    format!("browser-{id}")
}

pub fn is_browser_label(label: &str) -> bool {
    label.starts_with("browser-")
}

fn roblox_cookie(value: &str) -> Cookie<'static> {
    // The cookie crate strips one leading dot from the domain; ".." keeps the
    // domain-wide ".roblox.com" that www.roblox.com needs (alyae's trick).
    Cookie::build((".ROBLOSECURITY", value.to_string()))
        .domain("..roblox.com")
        .path("/")
        .secure(true)
        .http_only(true)
        .build()
}

fn host_allowed(url: &Url) -> bool {
    match url.scheme() {
        "https" | "http" => {}
        "about" | "data" | "blob" => return true,
        // The Play button hands roblox-player: to the OS; let it through.
        "roblox-player" | "roblox" => return true,
        _ => return false,
    }
    let Some(host) = url.host_str() else { return false };
    ALLOWED_SUFFIXES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Opens (or focuses) the account's browser window. With a stored cookie it
/// lands on the signed-in home page; without one it shows the sign-in page
/// in that account's own profile, ready for `browser_capture`.
#[tauri::command]
pub fn open_browser(app: AppHandle, state: State<'_, Shared>, id: AccountId) -> Result<(), String> {
    let (title, cookie, profile_dir) = {
        let mut s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        let name = s.account_name(id);
        let cookie = s.cookie(id);
        let dir = s.data_dir.join("profiles").join(id.to_string());
        (format!("{name} — RAM browser"), cookie, dir)
    };
    if let Some(w) = app.get_webview_window(&label(id)) {
        let _ = w.set_focus();
        return Ok(());
    }
    let _ = std::fs::create_dir_all(&profile_dir);
    let blank = Url::parse("about:blank").map_err(|e| e.to_string())?;
    let window = WebviewWindowBuilder::new(&app, label(id), WebviewUrl::External(blank))
        .title(title)
        .inner_size(1120.0, 780.0)
        .data_directory(profile_dir)
        .on_navigation(host_allowed)
        .build()
        .map_err(|e| format!("could not open a browser window: {e}"))?;
    // Start from a clean session so only the vault's cookie applies.
    let _ = window.clear_all_browsing_data();
    let target = if let Some(c) = cookie {
        window.set_cookie(roblox_cookie(&c)).map_err(|e| format!("could not set the session cookie: {e}"))?;
        // Confirm the cookie applies to www.roblox.com (domain-wide), else
        // the sign-in page will show and the user can sign in by hand.
        let check = Url::parse("https://www.roblox.com/").map_err(|e| e.to_string())?;
        let applied = window
            .cookies_for_url(check)
            .map(|cs| cs.iter().any(|c| c.name() == ".ROBLOSECURITY"))
            .unwrap_or(false);
        if applied { ROBLOX_HOME } else { ROBLOX_LOGIN }
    } else {
        ROBLOX_LOGIN
    };
    let url = Url::parse(target).map_err(|e| e.to_string())?;
    window.navigate(url).map_err(|e| format!("could not navigate: {e}"))?;
    Ok(())
}

/// Reads the `.ROBLOSECURITY` cookie from the account's browser window and
/// stores it in the vault, then starts a login check. Lets a user sign in
/// inside RAM's browser and save the session with one click.
#[tauri::command]
pub fn browser_capture(app: AppHandle, state: State<'_, Shared>, id: AccountId) -> Result<(), String> {
    let window = app.get_webview_window(&label(id)).ok_or("Open the browser for this account first.")?;
    let url = Url::parse("https://www.roblox.com/").map_err(|e| e.to_string())?;
    let cookies = window.cookies_for_url(url).map_err(|e| e.to_string())?;
    let value = cookies
        .iter()
        .find(|c| c.name() == ".ROBLOSECURITY")
        .map(|c| c.value().to_string())
        .ok_or("No Roblox session found in that window yet. Sign in there, then try again.")?;
    {
        let mut s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
        if !s.vault.is_unlocked() {
            return Err("Unlock the vault first.".into());
        }
        s.vault.set_secret(id, &value, &crate::svc::WinEntropy, now()).map_err(|e| e.to_string())?;
    }
    crate::cmd_launch::start_check(app, id);
    Ok(())
}

/// Signs the browser window out and closes it.
#[tauri::command]
pub fn browser_clear(app: AppHandle, id: AccountId) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label(id)) {
        let _ = w.clear_all_browsing_data();
        let _ = w.close();
    }
    Ok(())
}

/// Called when a browser window is closing: drops the session from the
/// profile so the vault remains the only place the cookie lives.
pub fn on_browser_closing(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.clear_all_browsing_data();
    }
}
