//! What handles `roblox-player:` links on this PC, and where its FastFlag
//! file lives.
//!
//! Roblox's own installer registers `RobloxPlayerBeta.exe`; Bloxstrap and
//! its forks (Fishstrap, Froststrap, Voidstrap) register themselves and copy
//! their `Modifications\ClientSettings\ClientAppSettings.json` over the
//! client's file at every launch. RAM writes its per-launch flags into
//! whichever file the client will actually read.

use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};

use crate::util::{from_wide, wide};

const BOOTSTRAPPERS: [&str; 4] = ["Bloxstrap", "Fishstrap", "Froststrap", "Voidstrap"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    /// "Roblox", or the bootstrapper's name.
    pub name: String,
    /// The registered handler executable, when one is registered.
    pub exe: Option<PathBuf>,
    pub bootstrapper: bool,
    /// Where `ClientAppSettings.json` must be written for the next client
    /// to read it, or `None` when no location could be worked out.
    pub flag_file: Option<PathBuf>,
}

/// The Windows build number (22000 and up is Windows 11), or 0 when it
/// cannot be read.
pub fn windows_build() -> u32 {
    reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", Some("CurrentBuildNumber"))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Whether Windows' "Transparency effects" setting (Settings →
/// Personalization → Colors) is on. With it off, Windows draws the Mica and
/// Acrylic materials as a solid colour for every app. Unknown counts as on.
pub fn transparency_effects() -> bool {
    reg_dword(HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "EnableTransparency").map(|v| v != 0).unwrap_or(true)
}

fn reg_dword(root: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut data = 0u32;
    let mut bytes = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(root, PCWSTR(subkey.as_ptr()), PCWSTR(value.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut data as *mut u32 as *mut _), Some(&mut bytes))
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn reg_sz(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    let value_ptr = value.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr()));
    let mut bytes = 0u32;
    let status = unsafe { RegGetValueW(root, PCWSTR(subkey.as_ptr()), value_ptr, RRF_RT_REG_SZ, None, None, Some(&mut bytes)) };
    if status != ERROR_SUCCESS || bytes == 0 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(root, PCWSTR(subkey.as_ptr()), value_ptr, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut bytes))
    };
    (status == ERROR_SUCCESS).then(|| from_wide(&buf))
}

/// The executable named by a `shell\open\command` string.
pub fn parse_command_exe(command: &str) -> Option<PathBuf> {
    let s = command.trim_start();
    let exe = if let Some(rest) = s.strip_prefix('"') { rest.split('"').next()? } else { s.split_whitespace().next()? };
    let exe = exe.trim();
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// Bootstrapper name for a handler path, by executable stem or by a parent
/// folder named after one.
pub fn bootstrapper_of(exe: &Path) -> Option<&'static str> {
    let stem = exe.file_stem()?.to_string_lossy().to_ascii_lowercase();
    if let Some(b) = BOOTSTRAPPERS.iter().find(|b| b.to_ascii_lowercase() == stem) {
        return Some(b);
    }
    exe.components()
        .filter_map(|c| c.as_os_str().to_str())
        .find_map(|part| BOOTSTRAPPERS.iter().find(|b| b.eq_ignore_ascii_case(part)).copied())
}

/// The bootstrapper's `Modifications\ClientSettings\ClientAppSettings.json`:
/// Bloxstrap keeps its data under `%LOCALAPPDATA%`, Froststrap under
/// `%APPDATA%`; whichever base folder exists wins.
pub fn bootstrapper_flag_file(name: &str) -> Option<PathBuf> {
    let candidates = [crate::paths::local_app_data(), crate::paths::roaming_app_data()];
    let bases: Vec<PathBuf> = candidates.into_iter().flatten().map(|b| b.join(name)).collect();
    let base = bases
        .iter()
        .find(|b| b.join("Modifications").is_dir())
        .or_else(|| bases.iter().find(|b| b.is_dir()))?;
    Some(base.join("Modifications").join("ClientSettings").join("ClientAppSettings.json"))
}

/// Roblox's own client folder from its registry environment, or the
/// handler's folder when the handler is the client itself.
fn roblox_client_dir(handler: Option<&Path>) -> Option<PathBuf> {
    const ENV: &str = r"Software\ROBLOX Corporation\Environments\roblox-player";
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if let Some(exe) = reg_sz(root, ENV, Some("clientExe")) {
            let p = PathBuf::from(exe);
            if p.is_file() && bootstrapper_of(&p).is_none() {
                return p.parent().map(|d| d.to_path_buf());
            }
        }
    }
    let h = handler?;
    let stem = h.file_stem()?.to_string_lossy().to_ascii_lowercase();
    (stem == "robloxplayerbeta" && h.is_file()).then(|| h.parent().map(|d| d.to_path_buf()))?
}

/// Detects the `roblox-player:` handler.
pub fn detect() -> Launcher {
    let command = reg_sz(HKEY_CURRENT_USER, r"Software\Classes\roblox-player\shell\open\command", None)
        .or_else(|| reg_sz(HKEY_CLASSES_ROOT, r"roblox-player\shell\open\command", None));
    let exe = command.as_deref().and_then(parse_command_exe);
    if let Some(name) = exe.as_deref().and_then(bootstrapper_of) {
        return Launcher { name: name.to_string(), exe, bootstrapper: true, flag_file: bootstrapper_flag_file(name) };
    }
    let flag_file = roblox_client_dir(exe.as_deref()).map(|d| d.join("ClientSettings").join("ClientAppSettings.json"));
    Launcher { name: "Roblox".to_string(), exe, bootstrapper: false, flag_file }
}

/// Every flag file RAM may have written to on this PC, for the start-up and
/// exit sweep that puts leftovers back.
pub fn all_flag_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(f) = detect().flag_file {
        out.push(f);
    }
    for b in BOOTSTRAPPERS {
        if let Some(f) = bootstrapper_flag_file(b) {
            if f.with_extension("json.ram-before").exists() || f.parent().map(|p| p.join("ClientAppSettings.json.ram-before").exists()).unwrap_or(false) {
                out.push(f);
            }
        }
    }
    if let Some(local) = crate::paths::local_app_data() {
        let versions = local.join("Roblox").join("Versions");
        if let Ok(rd) = std::fs::read_dir(&versions) {
            for e in rd.flatten() {
                let f = e.path().join("ClientSettings").join("ClientAppSettings.json");
                if f.parent().map(|p| p.join("ClientAppSettings.json.ram-before").exists()).unwrap_or(false) {
                    out.push(f);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_commands_parse() {
        assert_eq!(
            parse_command_exe(r#""C:\Users\x\AppData\Local\Froststrap\current\froststrap.exe" --player "%1""#),
            Some(PathBuf::from(r"C:\Users\x\AppData\Local\Froststrap\current\froststrap.exe"))
        );
        assert_eq!(parse_command_exe(r#"C:\Roblox\RobloxPlayerBeta.exe %1"#), Some(PathBuf::from(r"C:\Roblox\RobloxPlayerBeta.exe")));
        assert_eq!(parse_command_exe(""), None);
    }

    #[test]
    fn bootstrappers_are_recognised_by_stem_or_folder() {
        assert_eq!(bootstrapper_of(Path::new(r"C:\x\Froststrap\current\froststrap.exe")), Some("Froststrap"));
        assert_eq!(bootstrapper_of(Path::new(r"C:\x\Bloxstrap.exe")), Some("Bloxstrap"));
        assert_eq!(bootstrapper_of(Path::new(r"C:\Users\a\AppData\Local\Fishstrap\Versions\v1\RobloxPlayerBeta.exe")), Some("Fishstrap"));
        assert_eq!(bootstrapper_of(Path::new(r"C:\Roblox\Versions\v\RobloxPlayerBeta.exe")), None);
    }

    #[test]
    fn detect_does_not_panic() {
        let l = detect();
        assert!(!l.name.is_empty());
    }
}
