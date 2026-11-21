//! Self-update helpers around `tauri-plugin-updater`: the front-end does the
//! check and install through the plugin; these commands supply what the
//! plugin does not know — whether this copy is installed or portable, the
//! bundled release notes for "What's new", and the seen-version marker.

use crate::state::Shared;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The GitHub repository releases are published from. The updater reads
/// `https://github.com/<repo>/releases/latest/download/latest.json`, which
/// `tauri-action` writes with every release.
pub const GITHUB_REPO: &str = "claudeoffdaperc30/RAM";

/// The repository's CHANGELOG, embedded so What's new works offline.
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// The update found by the last check, kept for `update_install`.
#[derive(Default)]
pub struct UpdateSlot(pub Mutex<Option<Update>>);

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub date: String,
}

#[derive(Serialize, Clone)]
pub struct UpdateProgress {
    pub got: u64,
    pub total: u64,
    pub done: bool,
}

/// Asks GitHub for a newer signed release. `Ok(None)` means up to date.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://github.com/{GITHUB_REPO}/releases/latest/download/latest.json");
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{e}"))?])
        .map_err(|e| e.to_string())?
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(u)) => {
            let info = UpdateInfo {
                version: u.version.clone(),
                notes: u.body.clone().unwrap_or_default(),
                date: u.date.map(|d| d.to_string()).unwrap_or_default(),
            };
            if let Ok(s) = app.state::<Shared>().lock() {
                s.log.info(&format!("update: {} is available (running {})", info.version, env!("CARGO_PKG_VERSION")));
            }
            *app.state::<UpdateSlot>().0.lock().map_err(|_| "update state is unavailable")? = Some(u);
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(e) => {
            if let Ok(s) = app.state::<Shared>().lock() {
                s.log.warn(&format!("update check: {e}"));
            }
            Err(e.to_string())
        }
    }
}

/// Downloads the installer found by `update_check`, verifies its signature,
/// runs it and exits; the installer starts RAM again. Progress goes out as
/// `update-progress` events.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let update = app.state::<UpdateSlot>().0.lock().map_err(|_| "update state is unavailable")?.take().ok_or("Check for updates first.")?;
    if let Ok(mut s) = app.state::<Shared>().lock() {
        s.log.info(&format!("update: installing {}", update.version));
        s.restore_client_settings();
        s.save();
    }
    let a1 = app.clone();
    let a2 = app.clone();
    let mut got = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let _ = a1.emit("update-progress", UpdateProgress { got, total: total.unwrap_or(0), done: false });
            },
            move || {
                let _ = a2.emit("update-progress", UpdateProgress { got: 0, total: 0, done: true });
            },
        )
        .await
        .map_err(|e| {
            if let Ok(s) = app.state::<Shared>().lock() {
                s.log.error(&format!("update install: {e}"));
            }
            e.to_string()
        })
}

/// "installed" when RAM runs from an NSIS install (its uninstaller sits
/// beside the exe), else "portable" (a zip, which cannot update itself).
#[tauri::command]
pub fn install_kind() -> String {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("uninstall.exe").is_file()))
        .unwrap_or(false);
    if installed { "installed" } else { "portable" }.to_string()
}

/// The CHANGELOG section for `version` (`## <version>` up to the next
/// `## `), or an empty string.
pub fn notes_for(changelog: &str, version: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in changelog.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if inside {
                break;
            }
            let v = rest.trim().split_whitespace().next().unwrap_or("").trim_start_matches('v');
            inside = v == version;
            continue;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[tauri::command]
pub fn release_notes() -> String {
    notes_for(CHANGELOG, env!("CARGO_PKG_VERSION"))
}

#[tauri::command]
pub fn mark_seen_version(state: State<'_, Shared>) -> Result<(), String> {
    let mut s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
    s.store.settings.seen_version = env!("CARGO_PKG_VERSION").to_string();
    s.save();
    Ok(())
}

/// Opens an https URL in the default browser. Only GitHub and Roblox
/// addresses pass, so a page cannot talk RAM into opening anything else.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let ok = url.starts_with("https://github.com/") || url.starts_with("https://www.roblox.com/");
    if !ok {
        return Err("refusing to open that address".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_pick_one_section() {
        let log = "# Changelog\n\n## 0.3.0 (2026-11-22)\n\n### New\n- A thing\n\n## 0.2.0\n- Old\n";
        assert_eq!(notes_for(log, "0.3.0"), "### New\n- A thing");
        assert_eq!(notes_for(log, "0.2.0"), "- Old");
        assert_eq!(notes_for(log, "9.9.9"), "");
    }
}
