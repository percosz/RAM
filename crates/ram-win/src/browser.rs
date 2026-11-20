//! Installed browsers, their profile names, and launching a URL in a chosen
//! profile through each browser's documented command line.

use std::path::{Path, PathBuf};

use ram_core::platform::{BrowserApi, BrowserInstall, BrowserProfile, PlatformError, PlatformResult};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::{AssocQueryStringW, ShellExecuteW, ASSOCF_NONE, ASSOCSTR_PROGID};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::paths;
use crate::util::{map_code, map_err, other, wide};

struct Known {
    id: &'static str,
    display_name: &'static str,
    exe: &'static str,
    /// Install locations relative to Program Files, Program Files (x86) and
    /// the local app data folder.
    install_subdir: &'static str,
    /// `User Data` folder relative to local app data; `None` for Firefox.
    user_data: Option<&'static str>,
}

const KNOWN: &[Known] = &[
    Known {
        id: "chrome",
        display_name: "Google Chrome",
        exe: "chrome.exe",
        install_subdir: r"Google\Chrome\Application",
        user_data: Some(r"Google\Chrome\User Data"),
    },
    Known {
        id: "edge",
        display_name: "Microsoft Edge",
        exe: "msedge.exe",
        install_subdir: r"Microsoft\Edge\Application",
        user_data: Some(r"Microsoft\Edge\User Data"),
    },
    Known {
        id: "brave",
        display_name: "Brave",
        exe: "brave.exe",
        install_subdir: r"BraveSoftware\Brave-Browser\Application",
        user_data: Some(r"BraveSoftware\Brave-Browser\User Data"),
    },
    Known {
        id: "vivaldi",
        display_name: "Vivaldi",
        exe: "vivaldi.exe",
        install_subdir: r"Vivaldi\Application",
        user_data: Some(r"Vivaldi\User Data"),
    },
    Known {
        id: "firefox",
        display_name: "Mozilla Firefox",
        exe: "firefox.exe",
        install_subdir: "Mozilla Firefox",
        user_data: None,
    },
];

/// [`BrowserApi`] for Chrome, Edge, Brave, Vivaldi and Firefox.
///
/// Profile discovery reads exactly one metadata file per browser (`Local
/// State` for the Chromium family, `profiles.ini` for Firefox). Cookie,
/// history and credential stores are never opened.
#[derive(Default)]
pub struct WinBrowsers;

impl WinBrowsers {
    pub fn new() -> Self {
        Self
    }
}

fn reg_string(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    let value_ptr = value.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr()));
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS || bytes == 0 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut bytes),
        )
    };
    (status == ERROR_SUCCESS).then(|| crate::util::from_wide(&buf))
}

fn locate(known: &Known) -> Option<PathBuf> {
    let app_paths = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{}",
        known.exe
    );
    let mut candidates: Vec<PathBuf> = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(|root| reg_string(root, &app_paths, None))
        .map(|p| PathBuf::from(p.trim().trim_matches('"')))
        .collect();
    let roots = [
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        paths::local_app_data(),
    ];
    for root in roots.into_iter().flatten() {
        candidates.push(root.join(known.install_subdir).join(known.exe));
    }
    candidates
        .into_iter()
        .find(|p| p.is_absolute() && p.is_file())
}

fn read_capped(path: &Path) -> Option<String> {
    // Metadata files are small; refuse anything implausibly large.
    const LIMIT: u64 = 16 * 1024 * 1024;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn sort_profiles(profiles: &mut [BrowserProfile]) {
    profiles.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.key.cmp(&b.key))
    });
}

/// Parses a Chromium `Local State` file. The key is the profile directory
/// name (`Default`, `Profile 1`) and the display name is the profile's
/// `name`, falling back to the directory name. Sorted by display name.
pub fn parse_local_state(json: &str) -> Vec<BrowserProfile> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(cache) = root
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };
    let mut profiles: Vec<BrowserProfile> = cache
        .iter()
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, info)| {
            let name = info
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(key);
            BrowserProfile {
                key: key.clone(),
                display_name: name.to_string(),
            }
        })
        .collect();
    sort_profiles(&mut profiles);
    profiles
}

/// Parses Firefox's `profiles.ini`. Each `[ProfileN]` section with a `Name=`
/// yields a profile whose key and display name are that name. Sorted by
/// display name.
pub fn parse_profiles_ini(ini: &str) -> Vec<BrowserProfile> {
    let mut profiles: Vec<BrowserProfile> = Vec::new();
    let mut in_profile = false;
    for line in ini.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            in_profile = section
                .get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("profile"))
                && section[7..].chars().all(|c| c.is_ascii_digit())
                && section.len() > 7;
            continue;
        }
        if !in_profile || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            let value = value.trim();
            if name.trim().eq_ignore_ascii_case("name")
                && !value.is_empty()
                && !profiles.iter().any(|p| p.key == value)
            {
                profiles.push(BrowserProfile {
                    key: value.to_string(),
                    display_name: value.to_string(),
                });
            }
        }
    }
    sort_profiles(&mut profiles);
    profiles
}

fn invalid(message: &str) -> PlatformError {
    other(message)
}

/// Accepts only `https://` URLs that cannot break out of a quoted argument.
fn validate_url(url: &str) -> PlatformResult<()> {
    let has_scheme = url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    if !has_scheme || url.len() == 8 {
        return Err(invalid("only https:// URLs can be opened"));
    }
    if url.len() > 2000
        || url
            .chars()
            .any(|c| c == '"' || c.is_control() || c.is_whitespace())
    {
        return Err(invalid("URL contains characters that are not allowed"));
    }
    Ok(())
}

fn validate_profile_key(key: &str) -> PlatformResult<()> {
    if key.is_empty()
        || key.len() > 255
        || key.starts_with('-')
        || key.ends_with('\\')
        || key.chars().any(|c| c == '"' || c.is_control())
    {
        return Err(invalid("profile name contains characters that are not allowed"));
    }
    Ok(())
}

/// Builds the command line that opens `url` in one profile:
/// `"<exe>" --profile-directory="<key>" "<url>"` for Chromium browsers and
/// `"<exe>" -P "<key>" "<url>"` for Firefox.
///
/// Rejects non-`https://` URLs, and URLs, profile keys or executable paths
/// containing quotes or control characters, so no value can add arguments.
pub fn build_command_line(
    install: &BrowserInstall,
    profile_key: &str,
    url: &str,
) -> PlatformResult<String> {
    validate_url(url)?;
    validate_profile_key(profile_key)?;
    let exe = &install.exe_path;
    if exe.is_empty() || exe.chars().any(|c| c == '"' || c.is_control()) {
        return Err(invalid("browser path contains characters that are not allowed"));
    }
    Ok(if install.id == "firefox" {
        format!("\"{exe}\" -P \"{profile_key}\" \"{url}\"")
    } else {
        format!("\"{exe}\" --profile-directory=\"{profile_key}\" \"{url}\"")
    })
}

/// Maps a URL-association ProgId to a browser id.
fn browser_id_from_progid(progid: &str) -> Option<&'static str> {
    const PREFIXES: &[(&str, &str)] = &[
        ("ChromeHTML", "chrome"),
        ("MSEdgeHTM", "edge"),
        ("BraveHTML", "brave"),
        ("VivaldiHTM", "vivaldi"),
        ("FirefoxURL", "firefox"),
    ];
    PREFIXES
        .iter()
        .find(|(prefix, _)| {
            progid
                .get(..prefix.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
        })
        .map(|(_, id)| *id)
}

/// Id of the browser registered as the user's default `https` handler, when
/// it is one of the supported browsers.
pub fn default_browser_id() -> Option<String> {
    let scheme = wide("https");
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_PROGID,
            PCWSTR(scheme.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    }
    .ok()
    .ok()?;
    browser_id_from_progid(&crate::util::from_wide(&buf)).map(str::to_string)
}

fn spawn(exe: &str, command_line: &str) -> PlatformResult<()> {
    let exe = wide(exe);
    let mut command_line = wide(command_line);
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(exe.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        )
    }
    .map_err(|e| map_err(&e, "CreateProcess"))?;
    unsafe {
        let _ = CloseHandle(info.hThread);
        let _ = CloseHandle(info.hProcess);
    }
    Ok(())
}

impl BrowserApi for WinBrowsers {
    fn installed(&self) -> Vec<BrowserInstall> {
        KNOWN
            .iter()
            .filter_map(|known| {
                let exe = locate(known)?;
                Some(BrowserInstall {
                    id: known.id.to_string(),
                    display_name: known.display_name.to_string(),
                    exe_path: exe.to_string_lossy().into_owned(),
                })
            })
            .collect()
    }

    fn profiles(&self, browser_id: &str) -> Vec<BrowserProfile> {
        let Some(known) = KNOWN.iter().find(|k| k.id == browser_id) else {
            return Vec::new();
        };
        match known.user_data {
            Some(user_data) => paths::local_app_data()
                .and_then(|base| read_capped(&base.join(user_data).join("Local State")))
                .map(|json| parse_local_state(&json))
                .unwrap_or_default(),
            None => paths::roaming_app_data()
                .and_then(|base| read_capped(&base.join(r"Mozilla\Firefox\profiles.ini")))
                .map(|ini| parse_profiles_ini(&ini))
                .unwrap_or_default(),
        }
    }

    /// Opens an `https://` URL. With a target, the browser must be installed
    /// and the profile key must be one that [`profiles`](Self::profiles)
    /// reports, otherwise `NotFound` (Chromium would silently create a new
    /// profile for an unknown directory name). Without a target the URL goes
    /// to the default browser through the shell.
    fn open(&self, target: Option<(&str, &str)>, url: &str) -> PlatformResult<()> {
        validate_url(url)?;
        match target {
            Some((browser_id, profile_key)) => {
                let install = self
                    .installed()
                    .into_iter()
                    .find(|i| i.id == browser_id)
                    .ok_or(PlatformError::NotFound)?;
                let command_line = build_command_line(&install, profile_key, url)?;
                if !self.profiles(browser_id).iter().any(|p| p.key == profile_key) {
                    return Err(PlatformError::NotFound);
                }
                spawn(&install.exe_path, &command_line)
            }
            None => {
                let verb = wide("open");
                let file = wide(url);
                let result = unsafe {
                    ShellExecuteW(
                        None,
                        PCWSTR(verb.as_ptr()),
                        PCWSTR(file.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    )
                };
                // ShellExecute reports success as a value greater than 32.
                let code = result.0 as usize;
                if code > 32 {
                    Ok(())
                } else {
                    Err(map_code(code as u32, "ShellExecute"))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(id: &str, exe: &str) -> BrowserInstall {
        BrowserInstall {
            id: id.into(),
            display_name: id.into(),
            exe_path: exe.into(),
        }
    }

    fn pairs(profiles: &[BrowserProfile]) -> Vec<(&str, &str)> {
        profiles
            .iter()
            .map(|p| (p.key.as_str(), p.display_name.as_str()))
            .collect()
    }

    #[test]
    fn local_state_profiles_are_sorted_by_display_name() {
        let json = r#"{
            "browser": {"x": 1},
            "profile": {
                "info_cache": {
                    "Profile 2": {"name": "work", "gaia_name": "Someone"},
                    "Default": {"name": "Zed"},
                    "Profile 1": {"name": "Alt account"},
                    "Profile 3": {"shortcut_name": "no name"},
                    "Profile 4": {"name": "   "}
                },
                "last_used": "Default"
            }
        }"#;
        assert_eq!(
            pairs(&parse_local_state(json)),
            vec![
                ("Profile 1", "Alt account"),
                ("Profile 3", "Profile 3"),
                ("Profile 4", "Profile 4"),
                ("Profile 2", "work"),
                ("Default", "Zed"),
            ]
        );
    }

    #[test]
    fn local_state_ties_are_ordered_by_key() {
        let json = r#"{"profile":{"info_cache":{
            "Profile 9":{"name":"Same"},"Profile 10":{"name":"Same"},"Default":{"name":"same"}}}}"#;
        assert_eq!(
            pairs(&parse_local_state(json)),
            vec![("Profile 10", "Same"), ("Profile 9", "Same"), ("Default", "same")]
        );
    }

    #[test]
    fn local_state_tolerates_bad_input() {
        assert!(parse_local_state("").is_empty());
        assert!(parse_local_state("not json").is_empty());
        assert!(parse_local_state("{}").is_empty());
        assert!(parse_local_state(r#"{"profile":{"info_cache":[1,2]}}"#).is_empty());
        assert!(parse_local_state(r#"{"profile":"x"}"#).is_empty());
    }

    #[test]
    fn profiles_ini_reads_profile_sections_only() {
        let ini = "\u{feff}[Install308046B0AF4A39CB]\r\nDefault=Profiles/abcd.default-release\r\nLocked=1\r\n\r\n\
                   [Profile1]\r\nName=default\r\nIsRelative=1\r\nPath=Profiles/x.default\r\nDefault=1\r\n\r\n\
                   [Profile0]\r\nName=Roblox Alt\r\nIsRelative=1\r\nPath=Profiles/y.alt\r\n\r\n\
                   [General]\r\nStartWithLastProfile=1\r\nName=ignored\r\nVersion=2\r\n\
                   [ProfileX]\r\nName=also ignored\r\n\
                   [Profile2]\r\n; Name=commented\r\nname = Third \r\n";
        assert_eq!(
            pairs(&parse_profiles_ini(ini)),
            vec![("default", "default"), ("Roblox Alt", "Roblox Alt"), ("Third", "Third")]
        );
    }

    #[test]
    fn profiles_ini_tolerates_bad_input() {
        assert!(parse_profiles_ini("").is_empty());
        assert!(parse_profiles_ini("[Profile0]\nPath=x\n").is_empty());
        assert!(parse_profiles_ini("Name=orphan\n").is_empty());
        assert_eq!(parse_profiles_ini("[Profile0]\nName=a\n[Profile1]\nName=a\n").len(), 1);
    }

    #[test]
    fn chromium_command_line() {
        let chrome = install("chrome", r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        assert_eq!(
            build_command_line(&chrome, "Profile 1", "https://www.roblox.com/home").unwrap(),
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --profile-directory="Profile 1" "https://www.roblox.com/home""#
        );
    }

    #[test]
    fn firefox_command_line() {
        let firefox = install("firefox", r"C:\Program Files\Mozilla Firefox\firefox.exe");
        assert_eq!(
            build_command_line(&firefox, "Roblox Alt", "https://www.roblox.com/games/1?a=b&c=d")
                .unwrap(),
            r#""C:\Program Files\Mozilla Firefox\firefox.exe" -P "Roblox Alt" "https://www.roblox.com/games/1?a=b&c=d""#
        );
    }

    #[test]
    fn command_line_rejects_injection() {
        let chrome = install("chrome", r"C:\chrome.exe");
        let ok_url = "https://www.roblox.com/";
        for url in [
            "http://www.roblox.com/",
            "file:///C:/Windows/System32/calc.exe",
            "roblox-player:1+launchmode:play",
            "--load-extension=C:\\evil",
            "https://",
            "",
            "https://www.roblox.com/\" --load-extension=\"C:\\evil",
            "https://www.roblox.com/ --incognito",
            "https://www.roblox.com/\r\n",
            " https://www.roblox.com/",
        ] {
            assert!(build_command_line(&chrome, "Default", url).is_err(), "{url:?}");
        }
        for key in [
            "",
            "Default\" --load-extension=\"C:\\evil",
            "Default\n",
            "Default\\",
            "--no-sandbox",
            "a\u{0}b",
        ] {
            assert!(build_command_line(&chrome, key, ok_url).is_err(), "{key:?}");
        }
        let bad_exe = install("chrome", "C:\\x\" --flag \"");
        assert!(build_command_line(&bad_exe, "Default", ok_url).is_err());
        assert!(build_command_line(&chrome, "Default", "HTTPS://www.roblox.com/").is_ok());
    }

    #[test]
    fn open_rejects_non_https_before_doing_anything() {
        let browsers = WinBrowsers::new();
        assert!(matches!(
            browsers.open(None, "http://www.roblox.com/"),
            Err(PlatformError::Other { .. })
        ));
        assert!(matches!(
            browsers.open(None, "calc.exe"),
            Err(PlatformError::Other { .. })
        ));
        assert!(matches!(
            browsers.open(Some(("chrome", "Default")), "javascript:alert(1)"),
            Err(PlatformError::Other { .. })
        ));
    }

    #[test]
    fn open_with_unknown_browser_or_profile_is_not_found() {
        let browsers = WinBrowsers::new();
        assert_eq!(
            browsers.open(Some(("netscape", "Default")), "https://www.roblox.com/"),
            Err(PlatformError::NotFound)
        );
        for install in browsers.installed() {
            assert_eq!(
                browsers.open(
                    Some((&install.id, "ram-win test profile that does not exist")),
                    "https://www.roblox.com/"
                ),
                Err(PlatformError::NotFound)
            );
        }
    }

    #[test]
    fn progid_mapping() {
        assert_eq!(browser_id_from_progid("ChromeHTML"), Some("chrome"));
        assert_eq!(browser_id_from_progid("MSEdgeHTM"), Some("edge"));
        assert_eq!(browser_id_from_progid("BraveHTML"), Some("brave"));
        assert_eq!(browser_id_from_progid("VivaldiHTM.ABCDEF"), Some("vivaldi"));
        assert_eq!(browser_id_from_progid("FirefoxURL-308046B0AF4A39CB"), Some("firefox"));
        assert_eq!(browser_id_from_progid("OperaStable"), None);
        assert_eq!(browser_id_from_progid(""), None);
    }

    #[test]
    fn installed_browsers_exist_on_disk() {
        for install in WinBrowsers::new().installed() {
            assert!(Path::new(&install.exe_path).is_file(), "{install:?}");
        }
        // Must not panic whatever is registered.
        let _ = default_browser_id();
    }
}
