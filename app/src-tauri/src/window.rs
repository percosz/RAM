//! The main window, and its optional see-through backdrop.
//!
//! A window can only show the desktop behind it if it was created
//! transparent, so the backdrop is decided once, at start, from the saved
//! setting: Clear, Mica (Windows 11) or Acrylic. With the setting off the window is
//! created exactly as before: opaque, with the dark start colour, so there
//! is no cost and no flash for anyone who does not turn it on.

use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Manager, Theme, WebviewWindowBuilder};

/// First Windows 11 build; Mica does not exist before it.
const WINDOWS_11: u32 = 22000;
/// Acrylic needs Windows 10 1809.
const WINDOWS_10_1809: u32 = 17763;

/// The backdrop that `wanted` gives on this Windows build, or "" when it is
/// not available.
///
/// - "mica" and "acrylic" are Windows' own materials. Windows draws them as
///   a solid colour when its "Transparency effects" setting is off, and for
///   any window that is not the active one.
/// - "clear" has no effect at all: the window is simply see-through and the
///   page's own tint is all that is in front of the desktop. It works on
///   every PC and for an inactive window. (The older blur-behind effect was
///   tried and renders black on Windows 11 build 26300, so it is not offered.)
pub fn backdrop_for(wanted: &str, build: u32) -> &'static str {
    match wanted {
        "mica" if build >= WINDOWS_11 => "mica",
        "acrylic" if build >= WINDOWS_10_1809 => "acrylic",
        "clear" => "clear",
        _ => "",
    }
}

/// Command-line switches for the window's browser engine (WebView2).
///
/// The first is what Tauri passes by default. `--disable-gpu` makes the
/// engine draw the page with the processor: measured on the release build,
/// its GPU process drops from about 40 MB to 9-17 MB, and the graphics card
/// is left to the game clients. The page is mostly still, so the cost is
/// small; `gpu` turns the graphics card back on.
pub fn browser_args(gpu: bool) -> String {
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if !gpu {
        args.push_str(" --disable-gpu");
    }
    args
}

/// Creates the main window from the configuration and returns the backdrop
/// it was given ("" for the normal opaque window).
pub fn create_main(app: &AppHandle, wanted: &str, build: u32, always_on_top: bool, gpu: bool, streamer: bool) -> tauri::Result<&'static str> {
    let mut config = app.config().app.windows.first().cloned().unwrap_or_default();
    config.always_on_top = always_on_top;
    config.content_protected = streamer;
    // Until the window says it is in front, it counts as in the background.
    BACKGROUND_SINCE.store(unix_now(), std::sync::atomic::Ordering::SeqCst);
    let backdrop = backdrop_for(wanted, build);
    if !backdrop.is_empty() {
        // No background colour at all: Windows ignores the alpha of a window
        // background, so even "fully transparent" is painted as solid black
        // and hides the backdrop behind it.
        config.background_color = None;
        config.transparent = true;
    }
    // WebView2 refuses to start with switches that differ from those of an
    // engine still running on the same profile ("not in the correct state").
    // That engine is normally RAM's own from a moment ago (a restart after
    // the graphics-card setting changed, or an update), and is gone within a
    // second or two: wait for it. If it stays (another RAM is open with the
    // other setting), RAM starts itself again with that engine's switches:
    // a window that draws the other way is better than no window. (Trying
    // the other switches in this same process does not work: WebView2 keeps
    // refusing once a process has asked for different ones.)
    let mut last = None;
    config.additional_browser_args = Some(browser_args(gpu));
    for wait_ms in RETRY_WAITS_MS {
        if wait_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(wait_ms));
        }
        let mut builder = WebviewWindowBuilder::from_config(app, &config)?;
        let effect = match backdrop {
            "mica" => Some(Effect::Mica),
            "acrylic" => Some(Effect::Acrylic),
            _ => None,
        };
        if let Some(effect) = effect {
            builder = builder.effects(EffectsBuilder::new().effect(effect).build());
        }
        match builder.build() {
            Ok(_) => {
                if let Some(e) = last {
                    eprintln!("RAM: the window opened on a later attempt (first error: {e})");
                }
                return Ok(backdrop);
            }
            Err(e) => {
                // A window whose page could not be created must not stay behind.
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.destroy();
                }
                last = Some(e);
            }
        }
    }
    // Once only: a copy started this way does not start another.
    if std::env::var_os(GPU_OVERRIDE).is_none() {
        if let Ok(exe) = std::env::current_exe() {
            if std::process::Command::new(exe).env(GPU_OVERRIDE, if gpu { "off" } else { "on" }).spawn().is_ok() {
                std::process::exit(0);
            }
        }
    }
    Err(last.unwrap_or(tauri::Error::WindowNotFound))
}

/// Waits before each try at creating the window: at once, then again over
/// about three seconds.
const RETRY_WAITS_MS: [u64; 6] = [0, 300, 500, 700, 800, 800];

/// Environment variable RAM sets for the copy of itself it starts when the
/// window could not be created with the wanted engine switches: "on" or
/// "off" for the graphics card, for that run only.
pub const GPU_OVERRIDE: &str = "RAM_ENGINE_GPU";

/// Whether this run uses the graphics card: the override from a restart
/// (see [`GPU_OVERRIDE`]) if there is one, else the setting.
pub fn gpu_for_this_run(setting: bool, over: Option<&str>) -> bool {
    match over {
        Some("on") => true,
        Some("off") => false,
        _ => setting,
    }
}

/// The last size the window had while it was a usable size.
static LAST_GOOD: std::sync::Mutex<Option<tauri::PhysicalSize<u32>>> = std::sync::Mutex::new(None);
/// Whether the page is currently parked (window minimized).
static PARKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether WebView2 is currently asked to keep its memory use low.
static LOW_MEMORY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When the window stopped being the one in front (Unix seconds), or 0
/// while it is in front.
static BACKGROUND_SINCE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(1).max(1)
}

/// How long the window has not been the one in front (minimized, or behind
/// another window, or simply not the active one); `None` while it is.
pub fn background_secs() -> Option<i64> {
    match BACKGROUND_SINCE.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        since => Some((unix_now() - since).max(0)),
    }
}

/// Smaller than this (logical pixels) is not a size anyone chose: the
/// window's minimum is 900x560.
const UNUSABLE_W: f64 = 420.0;
const UNUSABLE_H: f64 = 300.0;

/// Whether a window of `size` at `scale` is too small to use.
pub fn is_unusable(size: tauri::PhysicalSize<u32>, scale: f64) -> bool {
    (size.width as f64) < UNUSABLE_W * scale || (size.height as f64) < UNUSABLE_H * scale
}

/// Window events for the main window:
/// - If it ever comes back from being minimized (or from a busy spell) at
///   an unusably small size, it is put back to the last good size.
/// - While it is minimized the page is parked: it stops drawing, its timers
///   slow down and it fetches nothing. Undone when the window is shown again.
/// - While it is not the window in front (minimized or not), WebView2 is
///   asked to keep its memory low, and the watcher hands the engine's
///   unused memory back to Windows (see `background_secs`).
pub fn on_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    use std::sync::atomic::Ordering;
    if window.label() != "main" {
        return;
    }
    if let tauri::WindowEvent::Focused(focused) = event {
        let minimized = window.is_minimized().unwrap_or(false);
        BACKGROUND_SINCE.store(if *focused { 0 } else { unix_now() }, Ordering::SeqCst);
        if *focused && !minimized {
            // Also a second chance to wake the page, should a restore ever
            // arrive without a size change.
            park(window.app_handle(), false);
        }
        set_low_memory(window.app_handle(), !*focused || minimized);
        return;
    }
    let tauri::WindowEvent::Resized(size) = event else { return };
    let minimized = window.is_minimized().unwrap_or(false) || size.width == 0 || size.height == 0;
    park(window.app_handle(), minimized);
    if minimized {
        let _ = BACKGROUND_SINCE.compare_exchange(0, unix_now(), Ordering::SeqCst, Ordering::SeqCst);
        set_low_memory(window.app_handle(), true);
    }
    if minimized || window.is_maximized().unwrap_or(false) {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    if is_unusable(*size, scale) {
        let good = LAST_GOOD.lock().ok().and_then(|g| *g);
        let target = good.unwrap_or(tauri::PhysicalSize::new((1180.0 * scale) as u32, (720.0 * scale) as u32));
        let _ = window.set_size(target);
        // Logged from another thread: this one draws the window and must
        // never wait for the state lock.
        let (app, was) = (window.app_handle().clone(), *size);
        std::thread::spawn(move || {
            if let Ok(s) = app.state::<crate::state::Shared>().lock() {
                s.log.warn(&format!("window: came back at {}x{}; restored to {}x{}", was.width, was.height, target.width, target.height));
            }
        });
    } else if let Ok(mut g) = LAST_GOOD.lock() {
        *g = Some(*size);
    }
}

/// Parks the page while nobody can see the window, or wakes it again.
/// Parked, WebView2 treats the page as hidden (`document.hidden`) and stops
/// drawing it.
fn park(app: &AppHandle, parked: bool) {
    use std::sync::atomic::Ordering;
    if PARKED.swap(parked, Ordering::SeqCst) == parked {
        return;
    }
    #[cfg(windows)]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.with_webview(move |webview| unsafe {
            let _ = webview.controller().SetIsVisible(!parked);
        });
    }
}

/// Asks WebView2 to keep its memory use low (its own caches and a one-time
/// trim), or to go back to normal.
fn set_low_memory(app: &AppHandle, low: bool) {
    use std::sync::atomic::Ordering;
    if LOW_MEMORY.swap(low, Ordering::SeqCst) == low {
        return;
    }
    #[cfg(windows)]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.with_webview(move |webview| {
            use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL};
            use windows_core::Interface;
            let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
            unsafe {
                if let Ok(core) = webview.controller().CoreWebView2() {
                    if let Ok(v19) = core.cast::<ICoreWebView2_19>() {
                        let _ = v19.SetMemoryUsageTargetLevel(level);
                    }
                }
            }
        });
    }
}

/// Makes the title bar (and a Mica backdrop) dark or light to match the
/// app's theme.
#[tauri::command]
pub fn set_window_theme(app: AppHandle, dark: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
    }
}

/// Streamer mode: leaves the window out of every screen capture, or puts it
/// back. Windows does this itself (`WDA_EXCLUDEFROMCAPTURE`, Windows 10
/// version 2004 and newer): recorders, streams and screenshots get whatever
/// is behind the window, while the screen shows it as usual.
pub fn set_streamer_mode(app: &AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_content_protected(on);
    }
}

/// Keeps the main window above every other window, or lets it go back.
pub fn set_always_on_top(app: &AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_top(on);
    }
}

/// Starts RAM again (for a backdrop change, which needs a new window).
/// Does what a normal exit does first: the user's Roblox settings are put
/// back and the data file is saved.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    if let Ok(mut s) = app.state::<crate::state::Shared>().lock() {
        s.restore_client_settings();
        s.log.info("RAM restarting");
        s.save();
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_override_wins_over_the_setting_for_one_run() {
        assert!(gpu_for_this_run(false, Some("on")));
        assert!(!gpu_for_this_run(true, Some("off")));
        assert!(gpu_for_this_run(true, None) && !gpu_for_this_run(false, None));
        assert!(!gpu_for_this_run(false, Some("nonsense")));
        assert_eq!(RETRY_WAITS_MS[0], 0, "the first try is immediate");
        let waited: u64 = RETRY_WAITS_MS.iter().sum();
        assert!((2500..=4000).contains(&waited), "waits about three seconds in all, not {waited} ms");
    }

    #[test]
    fn the_graphics_card_is_off_unless_asked_for() {
        assert!(browser_args(false).ends_with(" --disable-gpu"));
        assert!(!browser_args(true).contains("gpu"));
        // Tauri's own defaults are kept either way.
        for gpu in [false, true] {
            assert!(browser_args(gpu).starts_with("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
        }
    }

    #[test]
    fn only_sizes_nobody_would_choose_count_as_unusable() {
        let px = |w, h| tauri::PhysicalSize::new(w, h);
        assert!(is_unusable(px(160, 28), 1.0), "the stub Windows leaves for a minimized window");
        assert!(is_unusable(px(300, 800), 1.0) && is_unusable(px(1200, 100), 1.0));
        assert!(!is_unusable(px(900, 560), 1.0), "the smallest size the window allows");
        assert!(!is_unusable(px(1180, 720), 1.0));
        // The same pixels are too small on a 200% display.
        assert!(is_unusable(px(700, 500), 2.0));
    }

    #[test]
    fn backdrop_depends_on_the_windows_build() {
        assert_eq!(backdrop_for("mica", 26300), "mica");
        assert_eq!(backdrop_for("mica", 19045), "", "Windows 10 has no Mica");
        assert_eq!(backdrop_for("acrylic", 19045), "acrylic");
        assert_eq!(backdrop_for("acrylic", 14393), "");
        assert_eq!(backdrop_for("", 26300), "");
        assert_eq!(backdrop_for("blur", 26300), "");
        assert_eq!(backdrop_for("clear", 26300), "clear");
        assert_eq!(backdrop_for("clear", 0), "clear", "a see-through window needs no particular build");
        assert_eq!(backdrop_for("mica", 0), "", "unknown build: stay opaque");
    }
}
