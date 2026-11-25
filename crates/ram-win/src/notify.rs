//! Tray icon with balloon notifications.

use std::cell::Cell;

use ram_core::platform::Notifier;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::util::copy_wide;

const ICON_ID: u32 = 1;

/// [`Notifier`] that owns one notification-area icon and shows balloons
/// (toast-styled on Windows 10 and 11) from it.
///
/// Mouse events on the icon arrive at the window as `callback_msg` in the
/// classic format: `wParam` is the icon id (1) and `lParam` the mouse
/// message. Windows may suppress balloons during quiet hours or when the
/// user has turned notifications off. Create, use and drop it on the thread
/// that owns the window.
pub struct TrayNotifier {
    hwnd: isize,
    hicon: isize,
    callback_msg: u32,
    tip: String,
    added: Cell<bool>,
}

impl TrayNotifier {
    /// Adds the icon. If the shell is not running yet the icon is added on
    /// the next [`notify`](Notifier::notify) or [`restore`](Self::restore).
    pub fn new(hwnd: isize, hicon: isize, callback_msg: u32, tip: &str) -> Self {
        let notifier = Self {
            hwnd,
            hicon,
            callback_msg,
            tip: tip.to_string(),
            added: Cell::new(false),
        };
        notifier.restore();
        notifier
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(self.hwnd as _),
            uID: ICON_ID,
            ..Default::default()
        }
    }

    /// Adds the icon again. Call when the window receives the registered
    /// `TaskbarCreated` message (Explorer restarted). Returns whether the
    /// icon is present.
    pub fn restore(&self) -> bool {
        let mut data = self.base();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = self.callback_msg;
        data.hIcon = HICON(self.hicon as _);
        copy_wide(&mut data.szTip, &self.tip);
        let ok = unsafe {
            Shell_NotifyIconW(NIM_ADD, &data).as_bool()
                // Already present (for example after a spurious restore).
                || Shell_NotifyIconW(NIM_MODIFY, &data).as_bool()
        };
        self.added.set(ok);
        ok
    }
}

impl Notifier for TrayNotifier {
    /// Shows a balloon. Titles are cut to 63 and bodies to 255 UTF-16 units.
    /// Failures are ignored: a notification is never essential.
    fn notify(&self, title: &str, body: &str) {
        if !self.added.get() && !self.restore() {
            return;
        }
        let mut data = self.base();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        // An empty body would remove the balloon instead of showing one.
        copy_wide(&mut data.szInfo, if body.is_empty() { " " } else { body });
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for TrayNotifier {
    fn drop(&mut self) {
        if self.added.get() {
            let data = self.base();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
        }
    }
}

/// Registers `app_id` as a notification sender for the current user
/// (`HKCU\Software\Classes\AppUserModelId\<app_id>`) with the name and
/// icon Windows shows on its notifications. Windows silently drops
/// notifications from a program it cannot identify, which is every copy
/// that has no Start menu shortcut carrying the id (the portable build).
/// Returns whether the name was written.
pub fn register_app_id(app_id: &str, display_name: &str, icon: Option<&std::path::Path>) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let key = wide(&format!("Software\\Classes\\AppUserModelId\\{app_id}"));
    let set = |name: &str, value: &str| -> bool {
        let (name, value) = (wide(name), wide(value));
        let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), REG_SZ.0, Some(value.as_ptr() as *const _), (value.len() * 2) as u32) };
        status == ERROR_SUCCESS
    };
    let named = set("DisplayName", display_name);
    if let Some(icon) = icon {
        set("IconUri", &icon.display().to_string());
    }
    named
}
