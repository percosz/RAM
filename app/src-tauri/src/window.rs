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

/// Creates the main window from the configuration and returns the backdrop
/// it was given ("" for the normal opaque window).
pub fn create_main(app: &AppHandle, wanted: &str, build: u32, always_on_top: bool) -> tauri::Result<&'static str> {
    let mut config = app.config().app.windows.first().cloned().unwrap_or_default();
    config.always_on_top = always_on_top;
    let backdrop = backdrop_for(wanted, build);
    if !backdrop.is_empty() {
        // No background colour at all: Windows ignores the alpha of a window
        // background, so even "fully transparent" is painted as solid black
        // and hides the backdrop behind it.
        config.background_color = None;
        config.transparent = true;
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
    builder.build()?;
    Ok(backdrop)
}

/// The last size the window had while it was a usable size.
static LAST_GOOD: std::sync::Mutex<Option<tauri::PhysicalSize<u32>>> = std::sync::Mutex::new(None);
/// Whether the page is currently parked (window minimized).
static PARKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
///   slow down, it fetches nothing, and WebView2 is asked to give memory
///   back. All of that is undone when the window is shown again.
pub fn on_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if let tauri::WindowEvent::Focused(true) = event {
        // A second chance to wake the page, should a restore ever arrive
        // without a size change.
        if !window.is_minimized().unwrap_or(false) {
            park(window.app_handle(), false);
        }
        return;
    }
    let tauri::WindowEvent::Resized(size) = event else { return };
    let minimized = window.is_minimized().unwrap_or(false) || size.width == 0 || size.height == 0;
    park(window.app_handle(), minimized);
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
/// Parked, WebView2 treats the page as hidden (`document.hidden`), stops
/// drawing it, and is asked to keep its memory use low. Measured on the
/// release build: the engine's memory is trimmed once about 25 s after
/// minimizing (to a few MB), then settles at roughly two thirds of what it
/// uses while visible as its own housekeeping touches memory again.
fn park(app: &AppHandle, low: bool) {
    use std::sync::atomic::Ordering;
    if PARKED.swap(low, Ordering::SeqCst) == low {
        return;
    }
    #[cfg(windows)]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.with_webview(move |webview| {
            use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL};
            use windows_core::Interface;
            let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
            unsafe {
                let _ = webview.controller().SetIsVisible(!low);
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
