//! Per-client controls for running Roblox clients, addressed by process id:
//! minimise and restore without taking the focus, shrink the window below the
//! size Roblox holds it to, mute the client's audio session, page its working
//! set out, and a bandwidth limit for all clients together.
//!
//! The mechanisms follow RoALT (MIT licensed), reimplemented here:
//!
//! * windows: `ShowWindowAsync` for minimise/restore; for the small size a
//!   resize done while the window briefly has a borderless `WS_POPUP` style,
//!   because Windows only asks a window for its minimum size
//!   (`WM_GETMINMAXINFO`) while it has a sizing frame or is not a popup;
//! * audio: the client's own Core Audio sessions, found by process id;
//! * memory: `EmptyWorkingSet`, the pages come back as the client uses them;
//! * bandwidth: a Windows QoS policy in the `ActiveStore`, created by a
//!   fixed PowerShell command that runs elevated after the UAC prompt.
//!
//! Everything goes through documented window-manager, Core Audio and process
//! APIs. Nothing here reads or writes a client's memory, injects code or ends
//! a client. Errors are short sentences meant to be shown as they are.

use std::sync::Mutex;
use std::time::Duration;

use windows::core::{w, Error, Interface, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, SetLastError, HWND, RECT, RPC_E_CHANGED_MODE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    WIN32_ERROR,
};
use windows::Win32::Media::Audio::{
    eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, IsHungAppWindow, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    ShowWindowAsync, GWL_STYLE, GW_OWNER, SET_WINDOW_POS_FLAGS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWMINNOACTIVE,
    SW_SHOWNOACTIVATE, WINDOWPLACEMENT, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME,
};

use ram_core::platform::PlatformError;

use crate::util::{code_of, open_process, top_level_windows, wide, Handle, ERROR_ACCESS_DENIED};

/// Name of the QoS policy the bandwidth limit lives in.
const QOS_POLICY: &str = "RAM-Roblox";
/// Image name the policy matches.
const CLIENT_IMAGE: &str = "RobloxPlayerBeta.exe";
/// How long the elevated command may run once the prompt is answered.
const ELEVATED_TIMEOUT_MS: u32 = 60_000;
/// `ShellExecuteExW` reports a declined UAC prompt with this error.
const ERROR_CANCELLED: u32 = 1223;

/// Windows parks a minimised window at (-32000, -32000).
const PARKED: i32 = -30_000;
/// How long a window that is just being restored gets to leave that place.
const SETTLE_TRIES: u32 = 50;
const SETTLE_GAP: Duration = Duration::from_millis(20);

const NOT_RESPONDING: &str = "The client isn't responding.";
const WINDOW_CLOSED: &str = "The client's window has closed.";

/// A Win32 error as one short sentence.
fn describe(what: &str, e: &Error) -> String {
    if code_of(e) == ERROR_ACCESS_DENIED {
        return format!("{what}: access denied (is Roblox running as administrator?)");
    }
    let text = e.message();
    let text = text.trim_end();
    if text.is_empty() {
        format!("{what} (error {:#x})", e.code().0)
    } else {
        format!("{what}: {text}")
    }
}

fn last_error(what: &str) -> String {
    let code = unsafe { GetLastError() };
    describe(what, &Error::from_hresult(code.to_hresult()))
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(rect)
}

fn area(rect: &RECT) -> i64 {
    (rect.right - rect.left).max(0) as i64 * (rect.bottom - rect.top).max(0) as i64
}

/// Size a window counts with when the main window is chosen. A minimised
/// window is only a title-bar stub, so its restored size is used instead.
fn rank(hwnd: HWND) -> Option<i64> {
    if unsafe { IsIconic(hwnd) }.as_bool() {
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
            return Some(area(&placement.rcNormalPosition));
        }
    }
    window_rect(hwnd).map(|r| area(&r))
}

fn find_main_window(pid: u32) -> Option<HWND> {
    if pid == 0 {
        return None;
    }
    top_level_windows(pid)
        .into_iter()
        .filter(|&hwnd| unsafe { IsWindowVisible(hwnd) }.as_bool())
        .filter(|&hwnd| match unsafe { GetWindow(hwnd, GW_OWNER) } {
            Ok(owner) => owner.0.is_null(),
            Err(_) => true,
        })
        .filter_map(|hwnd| Some((rank(hwnd)?, hwnd)))
        // The first of equally sized windows wins, so the choice is stable.
        .fold(None, |best: Option<(i64, HWND)>, cur| match best {
            Some(b) if b.0 >= cur.0 => Some(b),
            _ => Some(cur),
        })
        .map(|(_, hwnd)| hwnd)
}

/// Top-level visible main window of the process (largest visible, unowned
/// top-level window), as an isize HWND. Window queries only.
pub fn main_window(pid: u32) -> Option<isize> {
    find_main_window(pid).map(|hwnd| hwnd.0 as isize)
}

/// PID that owns the foreground window, if any.
pub fn foreground_pid() -> Option<u32> {
    crate::window::foreground_pid()
}

/// Minimize without stealing focus (`ShowWindowAsync` with
/// `SW_SHOWMINNOACTIVE`). `Ok(false)` when the process has no window yet.
///
/// The request is posted to the client, so this returns at once even for a
/// client that is busy; [`is_minimized`] turns true a moment later.
pub fn minimize(pid: u32) -> Result<bool, String> {
    let Some(hwnd) = find_main_window(pid) else {
        return Ok(false);
    };
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return Ok(true);
    }
    if unsafe { ShowWindowAsync(hwnd, SW_SHOWMINNOACTIVE) }.as_bool() {
        Ok(true)
    } else {
        Err(last_error("Could not minimize the client"))
    }
}

/// Undo [`minimize`] without activating the window (`ShowWindowAsync` with
/// `SW_SHOWNOACTIVATE`). `Ok(false)` when the process has no window.
///
/// Posted like `minimize`. The window comes back at its normal size and
/// position; one that was maximised before it was minimised is not maximised
/// again, because Windows has no way to do that without activating it.
pub fn restore(pid: u32) -> Result<bool, String> {
    let Some(hwnd) = find_main_window(pid) else {
        return Ok(false);
    };
    if !unsafe { IsIconic(hwnd) }.as_bool() {
        return Ok(true);
    }
    if unsafe { ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE) }.as_bool() {
        Ok(true)
    } else {
        Err(last_error("Could not restore the client"))
    }
}

/// Whether the main window of the process is minimised; false without one.
pub fn is_minimized(pid: u32) -> bool {
    find_main_window(pid).is_some_and(|hwnd| unsafe { IsIconic(hwnd) }.as_bool())
}

/// The style a window has for the length of one resize: no caption, no
/// sizing frame, popup. Windows does not apply the program's minimum track
/// size to such a window.
fn borderless(style: u32) -> u32 {
    (style & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0)
        | WS_POPUP.0
}

fn set_style(hwnd: HWND, style: u32) -> Result<(), String> {
    // The previous style is a valid return value and can be 0, so failure
    // shows only in the last error.
    unsafe {
        SetLastError(WIN32_ERROR(0));
        if SetWindowLongPtrW(hwnd, GWL_STYLE, style as i32 as _) == 0 {
            let code = GetLastError();
            if code.0 != 0 {
                return Err(describe(
                    "Could not change the client window",
                    &Error::from_hresult(code.to_hresult()),
                ));
            }
        }
    }
    Ok(())
}

/// Serialises the style swap: a second resize of the same window would read
/// the borderless style as the one to put back.
static RESIZE: Mutex<()> = Mutex::new(());

/// Sets the outer size (and with `place` the position) of a window, past the
/// minimum size its program asks for. The style is changed for this one call
/// only, so the window keeps its title bar and edges afterwards.
fn resize_unclamped(
    hwnd: HWND,
    place: Option<(i32, i32)>,
    width: i32,
    height: i32,
) -> Result<(), String> {
    let _one_at_a_time = RESIZE.lock().unwrap_or_else(|e| e.into_inner());
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let bare = borderless(style);
    let swapped = bare != style;
    if swapped {
        set_style(hwnd, bare)?;
    }
    let (x, y) = place.unwrap_or((0, 0));
    let keep_place = if place.is_some() {
        SET_WINDOW_POS_FLAGS(0)
    } else {
        SWP_NOMOVE
    };
    let sized = unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | keep_place,
        )
    }
    .map_err(|e| describe("Could not resize the client window", &e));
    let put_back = if swapped {
        let put_back = set_style(hwnd, style);
        // Only makes the frame follow the style again; the size stays.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE
                    | SWP_NOSIZE
                    | SWP_NOZORDER
                    | SWP_NOACTIVATE
                    | SWP_NOOWNERZORDER
                    | SWP_FRAMECHANGED,
            );
        }
        put_back
    } else {
        Ok(())
    };
    sized.and(put_back)
}

fn rect_or_closed(hwnd: HWND) -> Result<RECT, String> {
    window_rect(hwnd).ok_or_else(|| {
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            last_error("Could not read the client window")
        } else {
            WINDOW_CLOSED.to_string()
        }
    })
}

/// The main window in its normal state and where it is, ready to be moved:
/// `Ok(None)` without one, an error while it does not answer (sizing a hung
/// window would wait for it). A minimised or maximised window is brought
/// back first, without activating it.
///
/// A restore asked for a moment ago ([`restore`] only posts it) may still be
/// under way: the window is no longer minimised then, but still at the place
/// minimised windows are parked. That is waited out, so the rect is never
/// the parked one.
fn window_to_size(pid: u32) -> Result<Option<(HWND, RECT)>, String> {
    let Some(hwnd) = find_main_window(pid) else {
        return Ok(None);
    };
    unsafe {
        if IsHungAppWindow(hwnd).as_bool() {
            return Err(NOT_RESPONDING.into());
        }
        if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        if IsIconic(hwnd).as_bool() {
            return Err("The client window could not be restored.".into());
        }
    }
    for _ in 0..SETTLE_TRIES {
        let rect = rect_or_closed(hwnd)?;
        if rect.left > PARKED || rect.top > PARKED {
            return Ok(Some((hwnd, rect)));
        }
        std::thread::sleep(SETTLE_GAP);
    }
    Err("The client window is still being restored. Try again in a moment.".into())
}

/// Shrink the client window to `width` x `height` pixels (outer size),
/// beating Roblox's minimum window size (816x638, enforced through
/// `WM_GETMINMAXINFO`): the window is switched to a borderless `WS_POPUP`
/// style for the one `SetWindowPos`, then gets its style back with
/// `SWP_FRAMECHANGED`. The top-left corner stays where it is and the window
/// is not activated.
///
/// Returns the window rect (left, top, right, bottom) before the change so
/// the caller can put it back with [`restore_rect`]; `Ok(None)` when there
/// is no window. A minimised or maximised window is restored first, and the
/// rect returned is the one it has then.
///
/// The client may still resize itself later (Roblox does when it comes back
/// from a minimise); callers that want it kept small apply this again.
pub fn set_tiny(pid: u32, width: i32, height: i32) -> Result<Option<(i32, i32, i32, i32)>, String> {
    if width <= 0 || height <= 0 {
        return Err("The window size must be at least 1 x 1.".into());
    }
    let Some((hwnd, before)) = window_to_size(pid)? else {
        return Ok(None);
    };
    resize_unclamped(hwnd, None, width, height)?;
    Ok(Some((before.left, before.top, before.right, before.bottom)))
}

/// Put a window back to a saved rect (left, top, right, bottom), without
/// activating it. `Ok(false)` when the process has no window.
///
/// A rect smaller than the client's own minimum size is honoured as well,
/// through the same style swap as [`set_tiny`].
pub fn restore_rect(pid: u32, rect: (i32, i32, i32, i32)) -> Result<bool, String> {
    let (left, top, right, bottom) = rect;
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    if width <= 0 || height <= 0 {
        return Err("The saved window rectangle is empty.".into());
    }
    let Some((hwnd, _)) = window_to_size(pid)? else {
        return Ok(false);
    };
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            left,
            top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
    }
    .map_err(|e| describe("Could not move the client window", &e))?;
    // The client pushed back to its minimum size: go past it.
    let now = rect_or_closed(hwnd)?;
    if (now.right - now.left, now.bottom - now.top) != (width, height) {
        resize_unclamped(hwnd, Some((left, top)), width, height)?;
    }
    Ok(true)
}

/// COM on the calling thread for the length of one call.
struct ComApartment {
    entered: bool,
}

impl ComApartment {
    fn enter() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            // S_OK and S_FALSE (already initialised) both count and are
            // both paired with CoUninitialize.
            Ok(Self { entered: true })
        } else if hr == RPC_E_CHANGED_MODE {
            // The thread already lives in a single-threaded apartment,
            // which serves just as well; that one is not ours to leave.
            Ok(Self { entered: false })
        } else {
            Err(describe("Could not start COM", &Error::from_hresult(hr)))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.entered {
            unsafe { CoUninitialize() };
        }
    }
}

/// Mute/unmute every Core Audio session of the process on all active render
/// endpoints. Returns how many sessions were set: 0 when the process has not
/// made a session yet (a client opens one a moment after it reaches a game;
/// the caller retries).
///
/// Only the client's own session in the Windows volume mixer changes, the
/// same switch the user has there; Roblox's settings are untouched. Windows
/// may remember the state for the next session of the same program.
///
/// Initialises COM on the calling thread for the length of the call
/// (multithreaded; a thread that already has an apartment is used as it is).
pub fn set_mute(pid: u32, mute: bool) -> Result<u32, String> {
    // Declared first, so every interface is released before COM is left.
    let _com = ComApartment::enter()?;
    let what = if mute {
        "Could not mute the client"
    } else {
        "Could not unmute the client"
    };
    let failed = |e: Error| describe(what, &e);
    let mut changed = 0u32;
    let mut problem: Option<String> = None;
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(failed)?;
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(failed)?;
        for d in 0..devices.GetCount().map_err(failed)? {
            // A device can go away between the count and here: the others
            // still get their turn.
            let sessions = devices
                .Item(d)
                .and_then(|device| device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
                .and_then(|manager| manager.GetSessionEnumerator())
                .and_then(|sessions| Ok((sessions.GetCount()?, sessions)));
            let (count, sessions) = match sessions {
                Ok(found) => found,
                Err(e) => {
                    problem.get_or_insert(failed(e));
                    continue;
                }
            };
            for s in 0..count {
                let Ok(session) = sessions.GetSession(s) else {
                    continue;
                };
                let owner = session
                    .cast::<IAudioSessionControl2>()
                    .and_then(|control| control.GetProcessId());
                if owner.ok() != Some(pid) {
                    continue;
                }
                let set = session
                    .cast::<ISimpleAudioVolume>()
                    .and_then(|volume| volume.SetMute(mute, std::ptr::null()));
                match set {
                    Ok(()) => changed += 1,
                    Err(e) => {
                        problem.get_or_insert(failed(e));
                    }
                }
            }
        }
    }
    match problem {
        Some(problem) if changed == 0 => Err(problem),
        _ => Ok(changed),
    }
}

/// Ask Windows to page the process's working set out (`EmptyWorkingSet`).
/// The pages come back as the client touches them, so this lowers its RAM
/// use for a while, not its commit. Opens the process with
/// `PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION` and nothing more.
pub fn trim_memory(pid: u32) -> Result<(), String> {
    let handle = open_process(pid, PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION).map_err(
        |e| match e {
            PlatformError::NotFound => "The client is no longer running.".to_string(),
            PlatformError::AccessDenied => {
                "Windows refused access to the client (is Roblox running as administrator?)."
                    .to_string()
            }
            other => format!("Could not open the client: {other}"),
        },
    )?;
    if unsafe { K32EmptyWorkingSet(handle.raw()) }.as_bool() {
        Ok(())
    } else {
        Err(last_error("Could not trim the client's memory"))
    }
}

/// The PowerShell command line that applies (`Some(kbit_per_sec)`) or
/// removes (`None`) a bandwidth limit for `RobloxPlayerBeta.exe`: it removes
/// the `RAM-Roblox` QoS policy and, when limiting, creates it again with the
/// new rate. The policy throttles the outbound traffic of all clients
/// together and lives in the `ActiveStore`, so it is gone after a reboot.
///
/// Pure. The only variable part is the rate, an integer; the text holds no
/// double quote, so it can be passed inside `-Command "..."`. A rate of 0
/// would block the clients, so `Some(0)` removes the limit like `None`.
pub fn bandwidth_script(kbit_per_sec: Option<u32>) -> String {
    let remove = format!(
        "Remove-NetQosPolicy -Name '{QOS_POLICY}' -PolicyStore ActiveStore -Confirm:$false \
         -ErrorAction SilentlyContinue"
    );
    match kbit_per_sec {
        Some(kbit) if kbit > 0 => {
            let bits = u64::from(kbit) * 1000;
            format!(
                "{remove}; New-NetQosPolicy -Name '{QOS_POLICY}' -AppPathNameMatchCondition \
                 '{CLIENT_IMAGE}' -ThrottleRateActionBitsPerSecond {bits} -PolicyStore ActiveStore \
                 | Out-Null"
            )
        }
        _ => remove,
    }
}

/// The arguments PowerShell is started with for `script`.
///
/// `powershell -Command` exits with 1 when the last statement reported an
/// error, which `Remove-NetQosPolicy` does when there is no policy to
/// remove. So: errors of any statement that does not silence them end the
/// run (exit code 1), and a run that reaches the end exits with 0.
fn powershell_arguments(script: &str) -> String {
    format!(
        "-NoProfile -NonInteractive -WindowStyle Hidden -Command \
         \"$ErrorActionPreference = 'Stop'; {script}; exit 0\""
    )
}

/// Runs [`bandwidth_script`] elevated: Windows shows its administrator
/// prompt, then Windows PowerShell runs the command hidden. Waits up to 60 s
/// for it and maps the exit code. A declined prompt is
/// `Err("The administrator prompt was declined.")`.
///
/// Blocks while the prompt is open; call it off the UI thread.
pub fn apply_bandwidth_limit(kbit_per_sec: Option<u32>) -> Result<(), String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let exe = wide(&format!(
        r"{}\System32\WindowsPowerShell\v1.0\powershell.exe",
        system_root.trim_end_matches('\\')
    ));
    let arguments = wide(&powershell_arguments(&bandwidth_script(kbit_per_sec)));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // `exe` and `arguments` outlive the call.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if code_of(&e) == ERROR_CANCELLED {
            "The administrator prompt was declined.".to_string()
        } else {
            describe("Could not start PowerShell", &e)
        });
    }
    if info.hProcess.is_invalid() {
        return Err("PowerShell did not start.".into());
    }
    // SEE_MASK_NOCLOSEPROCESS hands the process handle over; it is closed on drop.
    let process = Handle(info.hProcess);
    let waited = unsafe { WaitForSingleObject(process.raw(), ELEVATED_TIMEOUT_MS) };
    if waited == WAIT_TIMEOUT {
        return Err(
            "Windows did not finish in 60 seconds; the limit may or may not be set. Apply it again to be sure."
                .into(),
        );
    }
    if waited != WAIT_OBJECT_0 {
        return Err(last_error("Could not wait for PowerShell"));
    }
    let mut code = 1u32;
    unsafe { GetExitCodeProcess(process.raw(), &mut code) }
        .map_err(|e| describe("Could not read the result from PowerShell", &e))?;
    match code {
        0 => Ok(()),
        code if kbit_per_sec.is_some_and(|kbit| kbit > 0) => Err(format!(
            "Windows refused the bandwidth limit (PowerShell exit code {code})."
        )),
        code => Err(format!(
            "Windows could not remove the bandwidth limit (PowerShell exit code {code})."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW;

    const REMOVE: &str = "Remove-NetQosPolicy -Name 'RAM-Roblox' -PolicyStore ActiveStore \
                          -Confirm:$false -ErrorAction SilentlyContinue";

    #[test]
    fn bandwidth_script_limits_in_bits_per_second() {
        let script = bandwidth_script(Some(1000));
        assert_eq!(
            script,
            format!(
                "{REMOVE}; New-NetQosPolicy -Name 'RAM-Roblox' -AppPathNameMatchCondition \
                 'RobloxPlayerBeta.exe' -ThrottleRateActionBitsPerSecond 1000000 -PolicyStore \
                 ActiveStore | Out-Null"
            )
        );
        assert!(
            script.contains("-ThrottleRateActionBitsPerSecond 1000000 -PolicyStore ActiveStore"),
            "{script}"
        );
        // The largest rate does not overflow.
        assert!(bandwidth_script(Some(u32::MAX))
            .contains("-ThrottleRateActionBitsPerSecond 4294967295000 -PolicyStore ActiveStore"));
    }

    #[test]
    fn bandwidth_script_without_a_rate_only_removes() {
        assert_eq!(bandwidth_script(None), REMOVE);
        assert_eq!(bandwidth_script(Some(0)), REMOVE);
        assert!(!bandwidth_script(None).contains("New-NetQosPolicy"));
    }

    #[test]
    fn bandwidth_script_fits_inside_a_quoted_command() {
        for rate in [None, Some(0), Some(64), Some(1000), Some(u32::MAX)] {
            let script = bandwidth_script(rate);
            assert!(!script.contains('"'), "{script}");
            let arguments = powershell_arguments(&script);
            assert_eq!(arguments.matches('"').count(), 2, "{arguments}");
            assert!(
                arguments.starts_with(
                    "-NoProfile -NonInteractive -WindowStyle Hidden -Command \"$ErrorActionPreference = 'Stop'; Remove-NetQosPolicy "
                ),
                "{arguments}"
            );
            assert!(arguments.ends_with("; exit 0\""), "{arguments}");
        }
    }

    #[test]
    fn borderless_drops_the_frame_and_keeps_the_rest() {
        let visible = 0x1000_0000u32;
        let style = WS_OVERLAPPEDWINDOW.0 | visible;
        let bare = borderless(style);
        assert_eq!(bare & (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU).0, 0);
        assert_eq!(bare & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0, 0);
        assert_ne!(bare & WS_POPUP.0, 0);
        assert_ne!(bare & visible, 0);
        assert_eq!(borderless(bare), bare, "already borderless: nothing to swap");
    }

    #[test]
    fn a_process_without_a_window_is_not_an_error() {
        // PID 0 is the idle process: it never owns a window.
        assert_eq!(main_window(0), None);
        assert_eq!(minimize(0), Ok(false));
        assert_eq!(restore(0), Ok(false));
        assert!(!is_minimized(0));
        assert_eq!(set_tiny(0, 240, 180), Ok(None));
        assert_eq!(restore_rect(0, (0, 0, 240, 180)), Ok(false));
    }

    #[test]
    fn sizes_are_checked_before_anything_is_touched() {
        let own = std::process::id();
        assert!(set_tiny(own, 0, 180).is_err());
        assert!(set_tiny(own, 240, -1).is_err());
        assert!(restore_rect(own, (10, 10, 10, 200)).is_err());
        assert!(restore_rect(own, (i32::MAX, 0, i32::MIN, 10)).is_err());
    }

    #[test]
    fn trim_memory_of_a_missing_process_says_so() {
        // PID 0 cannot be opened; Windows answers "invalid parameter".
        assert_eq!(
            trim_memory(0),
            Err("The client is no longer running.".to_string())
        );
    }

    #[test]
    fn foreground_pid_does_not_panic() {
        let _ = foreground_pid();
    }
}
