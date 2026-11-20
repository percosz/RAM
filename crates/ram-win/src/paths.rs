//! Per-user folders and the Roblox files the manager reads.

use std::path::PathBuf;

use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

fn known_folder(id: &GUID) -> Option<PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    if path.is_null() {
        return None;
    }
    let text = String::from_utf16_lossy(unsafe { path.as_wide() });
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// `%LOCALAPPDATA%`.
pub fn local_app_data() -> Option<PathBuf> {
    known_folder(&FOLDERID_LocalAppData)
}

/// `%APPDATA%`.
pub fn roaming_app_data() -> Option<PathBuf> {
    known_folder(&FOLDERID_RoamingAppData)
}

/// `%LOCALAPPDATA%\Roblox\logs`. The folder is not guaranteed to exist.
pub fn roblox_logs_dir() -> Option<PathBuf> {
    Some(local_app_data()?.join("Roblox").join("logs"))
}

/// The most recently modified `GlobalBasicSettings_*.xml` in
/// `%LOCALAPPDATA%\Roblox`, or `None` when there is none.
pub fn roblox_global_settings_file() -> Option<PathBuf> {
    newest_settings_file(&local_app_data()?.join("Roblox"))
}

fn is_settings_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("globalbasicsettings_") && lower.ends_with(".xml")
}

fn newest_settings_file(dir: &std::path::Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| is_settings_name(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, entry.path()))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_folders_resolve_to_existing_directories() {
        assert!(local_app_data().unwrap().is_dir());
        assert!(roaming_app_data().unwrap().is_dir());
        assert!(roblox_logs_dir().unwrap().ends_with(r"Roblox\logs"));
    }

    #[test]
    fn settings_file_name_filter() {
        assert!(is_settings_name("GlobalBasicSettings_13.xml"));
        assert!(is_settings_name("globalbasicsettings_13_Studio.XML"));
        assert!(!is_settings_name("GlobalBasicSettings_13.xml.bak"));
        assert!(!is_settings_name("GlobalSettings_13.xml"));
    }

    #[test]
    fn picks_newest_settings_file() {
        let dir = std::env::temp_dir().join(format!("ram-win-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(newest_settings_file(&dir), None);

        let old = dir.join("GlobalBasicSettings_12.xml");
        let new = dir.join("GlobalBasicSettings_13.xml");
        std::fs::write(&old, "<a/>").unwrap();
        std::fs::write(&new, "<b/>").unwrap();
        std::fs::write(dir.join("other.xml"), "<c/>").unwrap();
        let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        assert_eq!(newest_settings_file(&dir), Some(new));
        assert_eq!(newest_settings_file(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
