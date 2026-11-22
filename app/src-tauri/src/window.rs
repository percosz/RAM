//! The main window, and its optional see-through backdrop.
//!
//! A window can only show the desktop behind it if it was created
//! transparent, so the backdrop is decided once, at start, from the saved
//! setting: Mica (Windows 11) or Acrylic. With the setting off the window is
//! created exactly as before: opaque, with the dark start colour, so there
//! is no cost and no flash for anyone who does not turn it on.

use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Manager, Theme, WebviewWindowBuilder};

/// First Windows 11 build; Mica does not exist before it.
const WINDOWS_11: u32 = 22000;
/// Acrylic needs Windows 10 1809.
const WINDOWS_10_1809: u32 = 17763;

/// The backdrop that `wanted` ("mica", "acrylic" or "") gives on this
/// Windows build, or "" when it is not available.
pub fn backdrop_for(wanted: &str, build: u32) -> &'static str {
    match wanted {
        "mica" if build >= WINDOWS_11 => "mica",
        "acrylic" if build >= WINDOWS_10_1809 => "acrylic",
        _ => "",
    }
}

/// Creates the main window from the configuration and returns the backdrop
/// it was given ("" for the normal opaque window).
pub fn create_main(app: &AppHandle, wanted: &str, build: u32) -> tauri::Result<&'static str> {
    let config = app.config().app.windows.first().cloned().unwrap_or_default();
    let mut builder = WebviewWindowBuilder::from_config(app, &config)?;
    let backdrop = backdrop_for(wanted, build);
    if !backdrop.is_empty() {
        let effect = if backdrop == "mica" { Effect::Mica } else { Effect::Acrylic };
        builder = builder.transparent(true).background_color(Color(0, 0, 0, 0)).effects(EffectsBuilder::new().effect(effect).build());
    }
    builder.build()?;
    Ok(backdrop)
}

/// Makes the title bar (and a Mica backdrop) dark or light to match the
/// app's theme.
#[tauri::command]
pub fn set_window_theme(app: AppHandle, dark: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
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
    fn backdrop_depends_on_the_windows_build() {
        assert_eq!(backdrop_for("mica", 26300), "mica");
        assert_eq!(backdrop_for("mica", 19045), "", "Windows 10 has no Mica");
        assert_eq!(backdrop_for("acrylic", 19045), "acrylic");
        assert_eq!(backdrop_for("acrylic", 14393), "");
        assert_eq!(backdrop_for("", 26300), "");
        assert_eq!(backdrop_for("blur", 26300), "");
        assert_eq!(backdrop_for("mica", 0), "", "unknown build: stay opaque");
    }
}
