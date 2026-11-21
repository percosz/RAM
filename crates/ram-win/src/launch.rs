//! Launches the Roblox client through its registered protocol handler.

use ram_core::platform::{PlatformError, PlatformResult};
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::util::{map_code, other, wide};

/// True when `uri` is a Roblox launch URI we are willing to hand to the shell.
/// Only the `roblox-player:` and `roblox:` schemes pass, so an arbitrary
/// command can never reach `ShellExecute`.
fn is_launch_uri(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    lower.starts_with("roblox-player:") || lower.starts_with("roblox:")
}

/// Starts the client by handing a validated Roblox URI to its registered
/// protocol handler. Any other scheme is rejected without touching the shell.
pub fn launch_protocol(uri: &str) -> PlatformResult<()> {
    if !is_launch_uri(uri) {
        return Err(other(
            "refusing to launch: not a roblox-player: or roblox: URI",
        ));
    }
    let file = wide(uri);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as an HINSTANCE value greater than 32.
    let code = result.0 as usize;
    if code > 32 {
        return Ok(());
    }
    Err(match code as u32 {
        // SE_ERR_FNF / SE_ERR_PNF: the scheme has no registered handler.
        2 | 3 => PlatformError::Unsupported("Roblox client not installed".into()),
        5 => PlatformError::AccessDenied,
        code => map_code(code, "ShellExecute"),
    })
}

/// True when the `roblox-player:` protocol is registered on this machine,
/// checked by opening its class key under `HKEY_CLASSES_ROOT`.
pub fn is_protocol_registered() -> bool {
    let mut key = HKEY::default();
    let opened =
        unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, w!("roblox-player"), Some(0), KEY_READ, &mut key) };
    if opened.is_ok() {
        unsafe {
            let _ = RegCloseKey(key);
        }
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_accepts_roblox_schemes_and_rejects_others() {
        assert!(is_launch_uri("roblox-player:1+launchmode:play+gameinfo:abc"));
        assert!(is_launch_uri("roblox:navigation"));
        assert!(is_launch_uri("ROBLOX-PLAYER:1")); // case-insensitive
        assert!(!is_launch_uri("https://evil"));
        assert!(!is_launch_uri("cmd.exe"));
        assert!(!is_launch_uri(""));
        assert!(!is_launch_uri("notroblox:1"));
    }

    #[test]
    fn launch_protocol_rejects_non_roblox_uri() {
        // Validation fails before any shell call, so this is side-effect free.
        assert!(matches!(
            launch_protocol("https://evil"),
            Err(PlatformError::Other { .. })
        ));
        assert!(matches!(
            launch_protocol(""),
            Err(PlatformError::Other { .. })
        ));
    }

    #[test]
    fn is_protocol_registered_returns_a_bool_without_panicking() {
        let _registered: bool = is_protocol_registered();
    }
}
