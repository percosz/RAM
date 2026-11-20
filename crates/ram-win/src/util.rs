//! Shared helpers: handle guard, string conversion, error mapping and
//! identity-checked process opening.

use ram_core::platform::{PlatformError, PlatformResult, ProcKey};
use windows::core::{Error, BOOL, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, FILETIME, HANDLE, HWND, LPARAM, WIN32_ERROR,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId};

pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
pub(crate) const ERROR_INVALID_PARAMETER: u32 = 87;
const STILL_ACTIVE: u32 = 259;

/// Owns a kernel handle and closes it on drop.
pub(crate) struct Handle(pub(crate) HANDLE);

// Kernel handles are process-wide and usable from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0 .0.is_null() && self.0 .0 as isize != -1 {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// UTF-16 with a terminating NUL.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Decodes UTF-16 up to the first NUL.
pub(crate) fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Copies `s` into a fixed UTF-16 buffer, truncating and NUL-terminating.
pub(crate) fn copy_wide(dst: &mut [u16], s: &str) {
    if dst.is_empty() {
        return;
    }
    let max = dst.len() - 1;
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n >= max {
            break;
        }
        dst[n] = unit;
        n += 1;
    }
    // Do not leave half of a surrogate pair at the cut.
    if n > 0 && (0xD800..0xDC00).contains(&dst[n - 1]) {
        n -= 1;
    }
    dst[n] = 0;
}

/// Win32 error code carried by a `windows` crate error.
pub(crate) fn code_of(e: &Error) -> u32 {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

pub(crate) fn map_code(code: u32, what: &str) -> PlatformError {
    if code == ERROR_ACCESS_DENIED {
        return PlatformError::AccessDenied;
    }
    let text = Error::from_hresult(WIN32_ERROR(code).to_hresult()).message();
    let message = if text.is_empty() {
        what.to_string()
    } else {
        format!("{what}: {}", text.trim_end())
    };
    PlatformError::Other { code, message }
}

pub(crate) fn map_err(e: &Error, what: &str) -> PlatformError {
    map_code(code_of(e), what)
}

pub(crate) fn last_error(what: &str) -> PlatformError {
    map_code(unsafe { GetLastError() }.0, what)
}

pub(crate) fn other(message: impl Into<String>) -> PlatformError {
    PlatformError::Other {
        code: 0,
        message: message.into(),
    }
}

pub(crate) fn filetime_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// (creation time, kernel + user time) of a process, in 100 ns ticks.
pub(crate) fn process_times(h: &Handle) -> PlatformResult<(u64, u64)> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(h.raw(), &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|e| map_err(&e, "GetProcessTimes"))?;
    Ok((
        filetime_u64(&created),
        filetime_u64(&kernel) + filetime_u64(&user),
    ))
}

/// Opens a process by PID. A PID that no longer exists is `NotFound`.
pub(crate) fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> PlatformResult<Handle> {
    match unsafe { OpenProcess(access, false, pid) } {
        Ok(h) => Ok(Handle(h)),
        Err(e) => Err(match code_of(&e) {
            ERROR_INVALID_PARAMETER => PlatformError::NotFound,
            code => map_code(code, "OpenProcess"),
        }),
    }
}

/// Opens the process identified by `key` with exactly `access`, after
/// confirming through a query-only handle that the PID still belongs to the
/// same, still running, process instance. The query handle stays open until
/// the second handle exists, so the PID cannot be reused in between.
pub(crate) fn open_verified(key: ProcKey, access: PROCESS_ACCESS_RIGHTS) -> PlatformResult<Handle> {
    let query = open_process(key.pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let (created, _) = process_times(&query)?;
    if created != key.start_time {
        return Err(PlatformError::NotFound);
    }
    let mut exit_code = 0u32;
    if unsafe { GetExitCodeProcess(query.raw(), &mut exit_code) }.is_ok()
        && exit_code != STILL_ACTIVE
    {
        return Err(PlatformError::NotFound);
    }
    if access == PROCESS_QUERY_LIMITED_INFORMATION {
        return Ok(query);
    }
    open_process(key.pid, access)
}

/// Full Win32 image path of a process opened for limited query.
pub(crate) fn image_path(h: &Handle) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(h.raw(), PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// File name component of a Windows path.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

struct EnumState {
    pid: u32,
    found: Vec<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut EnumState);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == state.pid {
        state.found.push(hwnd);
    }
    BOOL(1)
}

/// All top-level windows owned by `pid`, visible or not.
pub(crate) fn top_level_windows(pid: u32) -> Vec<HWND> {
    let mut state = EnumState {
        pid,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut state as *mut EnumState as isize));
    }
    state.found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_wide_truncates_and_terminates() {
        let mut buf = [0xFFFFu16; 4];
        copy_wide(&mut buf, "abcdef");
        assert_eq!(buf, [b'a' as u16, b'b' as u16, b'c' as u16, 0]);
        let mut buf = [0xFFFFu16; 3];
        copy_wide(&mut buf, "a\u{1F600}");
        assert_eq!(from_wide(&buf), "a");
    }

    #[test]
    fn file_name_takes_last_component() {
        assert_eq!(file_name(r"C:\a\b\c.exe"), "c.exe");
        assert_eq!(file_name("c.exe"), "c.exe");
    }

    #[test]
    fn access_denied_maps_to_its_own_variant() {
        assert_eq!(map_code(5, "x"), PlatformError::AccessDenied);
        assert!(matches!(map_code(6, "x"), PlatformError::Other { code: 6, .. }));
    }
}
