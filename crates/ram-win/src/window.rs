//! Window lookup, focus and placement for identified clients.

use ram_core::platform::{PlatformError, PlatformResult, ProcKey, Rect, WindowApi};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, SetForegroundWindow, SetWindowPos, ShowWindow, GW_OWNER,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SW_RESTORE, SW_SHOWNOACTIVATE,
};

use crate::util::{map_err, open_verified, other, top_level_windows, Handle};

/// [`WindowApi`] for client windows.
///
/// The main window of a process is its largest visible, unowned top-level
/// window. Rectangles are the outer window rectangles from `GetWindowRect`,
/// in the coordinate space of the calling process: physical pixels when the
/// manager is per-monitor DPI aware. They include the invisible resize
/// border Windows 10 and 11 draw around framed windows.
pub struct WinWindows {
    owner: isize,
}

impl WinWindows {
    /// `owner_hwnd` is the manager window used by
    /// [`work_area`](WindowApi::work_area); 0 selects the primary monitor.
    pub fn new(owner_hwnd: isize) -> Self {
        Self { owner: owner_hwnd }
    }
}

/// PID that owns a window, or `None` for an invalid handle.
pub fn pid_of_window(hwnd: isize) -> Option<u32> {
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(HWND(hwnd as _), Some(&mut pid)) };
    (thread != 0 && pid != 0).then_some(pid)
}

/// PID that owns the current foreground window, if there is one.
pub fn foreground_pid() -> Option<u32> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    pid_of_window(hwnd.0 as isize)
}

fn rect_of(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(rect)
}

fn main_window_of(pid: u32) -> Option<HWND> {
    top_level_windows(pid)
        .into_iter()
        .filter(|&hwnd| unsafe { IsWindowVisible(hwnd) }.as_bool())
        .filter(|&hwnd| match unsafe { GetWindow(hwnd, GW_OWNER) } {
            Ok(owner) => owner.0.is_null(),
            Err(_) => true,
        })
        .filter_map(|hwnd| {
            let r = rect_of(hwnd)?;
            let area = (r.right - r.left).max(0) as i64 * (r.bottom - r.top).max(0) as i64;
            Some((area, hwnd))
        })
        // Keep the first of equally sized windows so the choice is stable.
        .fold(None, |best: Option<(i64, HWND)>, cur| match best {
            Some(b) if b.0 >= cur.0 => Some(b),
            _ => Some(cur),
        })
        .map(|(_, hwnd)| hwnd)
}

/// Verifies the key and returns the main window together with a process
/// handle that pins the PID while the window is used.
fn locate(key: ProcKey) -> PlatformResult<(Handle, HWND)> {
    let guard = open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let hwnd = main_window_of(key.pid).ok_or(PlatformError::NotFound)?;
    Ok((guard, hwnd))
}

impl WindowApi for WinWindows {
    fn has_main_window(&self, key: ProcKey) -> bool {
        locate(key).is_ok()
    }

    /// Restores the window if minimised and brings it to the foreground.
    /// Windows only honours the request while the manager is allowed to set
    /// the foreground window (normally: it is handling user input).
    fn focus(&self, key: ProcKey) -> PlatformResult<()> {
        let (_guard, hwnd) = locate(key)?;
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err(other("Windows did not allow the window to take focus"))
            }
        }
    }

    fn window_rect(&self, key: ProcKey) -> PlatformResult<Rect> {
        let (_guard, hwnd) = locate(key)?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| map_err(&e, "GetWindowRect"))?;
        Ok(Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    /// Moves and resizes the window without activating it or changing its
    /// z-order. A minimised or maximised window is restored first. The client
    /// may enforce its own minimum size.
    fn move_window(&self, key: ProcKey, rect: Rect) -> PlatformResult<()> {
        if rect.width() <= 0 || rect.height() <= 0 {
            return Err(other("window rectangle is empty"));
        }
        let (_guard, hwnd) = locate(key)?;
        unsafe {
            if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        }
        .map_err(|e| map_err(&e, "SetWindowPos"))
    }

    /// Returns an empty rectangle if the monitor cannot be queried.
    fn work_area(&self) -> Rect {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let ok = unsafe {
            let monitor = MonitorFromWindow(HWND(self.owner as _), MONITOR_DEFAULTTOPRIMARY);
            GetMonitorInfoW(monitor, &mut info).as_bool()
        };
        if !ok {
            return Rect::default();
        }
        Rect {
            left: info.rcWork.left,
            top: info.rcWork.top,
            right: info.rcWork.right,
            bottom: info.rcWork.bottom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_window_has_no_pid() {
        assert_eq!(pid_of_window(0), None);
    }

    #[test]
    fn stale_key_is_not_found() {
        let windows = WinWindows::new(0);
        let key = ProcKey {
            pid: std::process::id(),
            start_time: 1,
        };
        assert!(!windows.has_main_window(key));
        assert_eq!(windows.focus(key), Err(PlatformError::NotFound));
        assert_eq!(windows.window_rect(key), Err(PlatformError::NotFound));
        let rect = Rect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        assert_eq!(windows.move_window(key, rect), Err(PlatformError::NotFound));
    }
}
