//! `client_ctl` against things the test owns: a window of the test process
//! that holds itself to Roblox's minimum size, an idle child process, and an
//! audio session the test opens itself. No other process is touched and no
//! input is synthesised. `apply_bandwidth_limit` is never called: it changes
//! system network policy behind a UAC prompt.

mod common;

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::{spawn_idle, TestChild};
use ram_win::client_ctl;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsZoomed, PostMessageW, PostQuitMessage, RegisterClassW,
    SetWindowPos, ShowWindow, TranslateMessage, GWL_STYLE, MINMAXINFO, MSG, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_SHOWMAXIMIZED, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WM_CLOSE,
    WM_DESTROY, WM_GETMINMAXINFO, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

/// The minimum size the test window holds itself to, as Roblox does.
const MIN: (i32, i32) = (816, 638);
const TINY: (i32, i32) = (240, 180);

/// `main_window` picks the largest window of the process, so only one test
/// window may exist at a time.
static ONE_WINDOW: Mutex<()> = Mutex::new(());

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT { x: MIN.0, y: MIN.1 };
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// An ordinary framed top-level window with its own thread and message
/// loop, shown without being activated.
struct TestWindow {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
    _only: MutexGuard<'static, ()>,
}

impl TestWindow {
    fn open() -> Self {
        let only = ONE_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("ram-win client_ctl test"),
                ..Default::default()
            };
            // Registered again by the next test window: that failure is fine.
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class.lpszClassName,
                w!("ram-win client_ctl test"),
                WS_OVERLAPPEDWINDOW,
                80,
                60,
                900,
                700,
                None,
                None,
                None,
                None,
            )
            .expect("test window");
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            tx.send(hwnd.0 as isize).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        let hwnd = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("test window never appeared");
        Self {
            hwnd,
            thread: Some(thread),
            _only: only,
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as _)
    }

    fn rect(&self) -> (i32, i32, i32, i32) {
        let mut r = RECT::default();
        unsafe { GetWindowRect(self.hwnd(), &mut r) }.expect("GetWindowRect");
        (r.left, r.top, r.right, r.bottom)
    }

    fn size(&self) -> (i32, i32) {
        let (left, top, right, bottom) = self.rect();
        (right - left, bottom - top)
    }

    fn style(&self) -> isize {
        unsafe { GetWindowLongPtrW(self.hwnd(), GWL_STYLE) }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn own_pid() -> u32 {
    std::process::id()
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn main_window_is_found_by_pid() {
    let window = TestWindow::open();
    assert_eq!(client_ctl::main_window(own_pid()), Some(window.hwnd));
    assert!(!client_ctl::is_minimized(own_pid()));
    drop(window);
    assert_eq!(client_ctl::main_window(own_pid()), None);
}

#[test]
fn foreground_pid_does_not_panic() {
    let pid = client_ctl::foreground_pid();
    println!("foreground pid: {pid:?}");
    assert_ne!(pid, Some(0));
}

#[test]
fn minimize_and_restore_round_trip_without_taking_focus() {
    let window = TestWindow::open();
    let pid = own_pid();
    let before = window.rect();

    assert_eq!(client_ctl::minimize(pid), Ok(true));
    wait_until("window minimised", || client_ctl::is_minimized(pid));
    // Still found while it is only a stub on the taskbar.
    assert_eq!(client_ctl::main_window(pid), Some(window.hwnd));
    // Asking again is a no-op.
    assert_eq!(client_ctl::minimize(pid), Ok(true));

    assert_eq!(client_ctl::restore(pid), Ok(true));
    wait_until("window restored", || !client_ctl::is_minimized(pid));
    // The request is posted: the window is back in place a moment later.
    wait_until("window back in place", || window.rect() == before);
    assert_eq!(client_ctl::restore(pid), Ok(true));

    // A resize asked for while a restore is still under way waits it out
    // and never reports the place minimised windows are parked at.
    assert_eq!(client_ctl::minimize(pid), Ok(true));
    wait_until("window minimised", || client_ctl::is_minimized(pid));
    assert_eq!(client_ctl::restore(pid), Ok(true));
    assert_eq!(client_ctl::set_tiny(pid, TINY.0, TINY.1), Ok(Some(before)));
    assert_eq!(window.size(), TINY);
    assert_eq!(client_ctl::restore_rect(pid, before), Ok(true));
    assert_eq!(window.rect(), before);

    // The window was never activated on the way.
    assert_ne!(client_ctl::foreground_pid(), Some(pid));
}

#[test]
fn set_tiny_goes_below_the_minimum_and_restore_rect_puts_it_back() {
    let window = TestWindow::open();
    let pid = own_pid();
    let before = window.rect();
    let style = window.style();
    assert_eq!((before.2 - before.0, before.3 - before.1), (900, 700));

    // The window really holds itself to the minimum against a plain resize.
    unsafe {
        SetWindowPos(
            window.hwnd(),
            None,
            0,
            0,
            TINY.0,
            TINY.1,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .unwrap();
    assert_eq!(window.size(), MIN, "the test window does not enforce its minimum");
    let clamped = window.rect();

    let saved = client_ctl::set_tiny(pid, TINY.0, TINY.1).unwrap();
    assert_eq!(saved, Some(clamped), "the rect before the change");
    assert_eq!(window.size(), TINY);
    assert_eq!((window.rect().0, window.rect().1), (before.0, before.1), "top-left stays");
    assert_eq!(window.style(), style, "the frame style is back");
    assert_eq!(client_ctl::main_window(pid), Some(window.hwnd));

    // Applying it again changes nothing and reports the tiny rect.
    let tiny = window.rect();
    assert_eq!(client_ctl::set_tiny(pid, TINY.0, TINY.1).unwrap(), Some(tiny));
    assert_eq!(window.rect(), tiny);

    assert_eq!(client_ctl::restore_rect(pid, before), Ok(true));
    assert_eq!(window.rect(), before);
    assert_eq!(window.style(), style);

    // A saved rect under the minimum is put back too.
    let small = (before.0 + 30, before.1 + 20, before.0 + 30 + 320, before.1 + 20 + 200);
    assert_eq!(client_ctl::restore_rect(pid, small), Ok(true));
    assert_eq!(window.rect(), small);
    assert_eq!(window.style(), style);

    assert_ne!(client_ctl::foreground_pid(), Some(pid));
}

#[test]
fn set_tiny_brings_a_minimized_window_back_first() {
    let window = TestWindow::open();
    let pid = own_pid();
    let before = window.rect();

    assert_eq!(client_ctl::minimize(pid), Ok(true));
    wait_until("window minimised", || client_ctl::is_minimized(pid));
    let saved = client_ctl::set_tiny(pid, TINY.0, TINY.1).unwrap();
    assert_eq!(saved, Some(before), "the restored rect, not the minimised stub");
    assert!(!client_ctl::is_minimized(pid));
    assert_eq!(window.size(), TINY);

    // And restore_rect does the same on the way back.
    assert_eq!(client_ctl::minimize(pid), Ok(true));
    wait_until("window minimised", || client_ctl::is_minimized(pid));
    assert_eq!(client_ctl::restore_rect(pid, before), Ok(true));
    assert!(!client_ctl::is_minimized(pid));
    assert_eq!(window.rect(), before);

    assert_ne!(client_ctl::foreground_pid(), Some(pid));
}

/// Maximising activates the window and covers the screen for a moment.
#[test]
#[ignore = "maximises a window on the interactive desktop"]
fn set_tiny_brings_a_maximized_window_back_first() {
    let window = TestWindow::open();
    let pid = own_pid();
    let before = window.rect();

    unsafe {
        let _ = ShowWindow(window.hwnd(), SW_SHOWMAXIMIZED);
    }
    assert!(unsafe { IsZoomed(window.hwnd()) }.as_bool());
    let saved = client_ctl::set_tiny(pid, TINY.0, TINY.1).unwrap();
    assert_eq!(saved, Some(before));
    assert!(!unsafe { IsZoomed(window.hwnd()) }.as_bool());
    assert_eq!(window.size(), TINY);
}

/// The same against a window of another process, as with a real client: a
/// child the test starts shows a form that holds itself to the minimum
/// size. It is asked to close through its own window at the end.
#[test]
#[ignore = "opens a window on the interactive desktop"]
fn set_tiny_goes_below_the_minimum_of_a_window_in_another_process() {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             $f = New-Object System.Windows.Forms.Form; \
             $f.Text = 'ram-win client_ctl test'; \
             $f.StartPosition = 'Manual'; $f.Left = 80; $f.Top = 60; \
             $f.Width = 900; $f.Height = 700; \
             $f.MinimumSize = New-Object System.Drawing.Size(816, 638); \
             [System.Windows.Forms.Application]::Run($f)",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("powershell.exe");
    let mut child = TestChild(child);
    let pid = child.pid();

    let deadline = Instant::now() + Duration::from_secs(30);
    while client_ctl::main_window(pid).is_none() {
        assert!(Instant::now() < deadline, "child never showed a window");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));
    let hwnd = HWND(client_ctl::main_window(pid).unwrap() as _);
    let rect = || {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut r) }.expect("GetWindowRect");
        (r.left, r.top, r.right, r.bottom)
    };
    let size = || {
        let r = rect();
        (r.2 - r.0, r.3 - r.1)
    };
    let before = rect();
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };

    // The form holds itself to its minimum against a plain resize.
    unsafe {
        SetWindowPos(hwnd, None, 0, 0, TINY.0, TINY.1, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE)
    }
    .unwrap();
    assert_eq!(size(), MIN, "the form does not enforce its minimum");
    assert_eq!(client_ctl::restore_rect(pid, before), Ok(true));
    assert_eq!(rect(), before);

    assert_eq!(client_ctl::set_tiny(pid, TINY.0, TINY.1), Ok(Some(before)));
    assert_eq!(size(), TINY);
    assert_eq!(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) }, style);

    assert_eq!(client_ctl::minimize(pid), Ok(true));
    wait_until("form minimised", || client_ctl::is_minimized(pid));
    assert_eq!(client_ctl::restore(pid), Ok(true));
    wait_until("form restored", || !client_ctl::is_minimized(pid));
    println!("size after minimise and restore: {:?}", size());

    assert_eq!(client_ctl::restore_rect(pid, before), Ok(true));
    assert_eq!(rect(), before);
    assert_eq!(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) }, style);

    assert_eq!(client_ctl::trim_memory(pid), Ok(()));
    assert_eq!(client_ctl::set_mute(pid, true), Ok(0));

    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    assert!(child.wait_exit(Duration::from_secs(15)), "child ignored the close request");
}

#[test]
fn a_child_without_a_window_is_left_alone() {
    let child = spawn_idle();
    let pid = child.pid();
    assert_eq!(client_ctl::main_window(pid), None);
    assert_eq!(client_ctl::minimize(pid), Ok(false));
    assert_eq!(client_ctl::restore(pid), Ok(false));
    assert!(!client_ctl::is_minimized(pid));
    assert_eq!(client_ctl::set_tiny(pid, TINY.0, TINY.1), Ok(None));
    assert_eq!(client_ctl::restore_rect(pid, (0, 0, 300, 200)), Ok(false));
}

#[test]
fn set_mute_without_an_audio_session_changes_nothing() {
    let child = spawn_idle();
    assert_eq!(client_ctl::set_mute(child.pid(), true), Ok(0));
    assert_eq!(client_ctl::set_mute(child.pid(), false), Ok(0));
}

#[test]
fn set_mute_works_on_a_thread_that_already_has_an_apartment() {
    let child = spawn_idle();
    let pid = child.pid();
    let sta = std::thread::spawn(move || unsafe {
        assert!(CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok());
        let result = client_ctl::set_mute(pid, true);
        CoUninitialize();
        result
    });
    assert_eq!(sta.join().unwrap(), Ok(0));
    let mta = std::thread::spawn(move || unsafe {
        assert!(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let result = client_ctl::set_mute(pid, true);
        // The thread's own initialisation is still in place afterwards.
        let still = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL);
        assert!(still.is_ok());
        drop(still);
        CoUninitialize();
        result
    });
    assert_eq!(mta.join().unwrap(), Ok(0));
}

/// Opens a shared-mode stream on the default output device (silent: it is
/// never started), which gives this process an audio session of its own.
/// `None` on a machine without an output device.
unsafe fn open_own_session() -> Option<(IAudioClient, ISimpleAudioVolume)> {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
    let format = client.GetMixFormat().ok()?;
    let opened = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, format, None);
    CoTaskMemFree(Some(format as *const _));
    opened.ok()?;
    let volume: ISimpleAudioVolume = client.GetService().ok()?;
    Some((client, volume))
}

#[test]
fn set_mute_mutes_and_unmutes_the_sessions_of_one_process() {
    unsafe {
        assert!(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let Some((_client, volume)) = open_own_session() else {
            println!("no usable audio output device: skipped");
            CoUninitialize();
            return;
        };
        let pid = own_pid();
        let was = volume.GetMute().unwrap().as_bool();

        let muted = client_ctl::set_mute(pid, true).unwrap();
        assert!(muted >= 1, "the session of this process was not found");
        assert!(volume.GetMute().unwrap().as_bool());

        // Another process is not affected by it, and this one not by another's.
        let child = spawn_idle();
        assert_eq!(client_ctl::set_mute(child.pid(), false), Ok(0));
        assert!(volume.GetMute().unwrap().as_bool());

        assert_eq!(client_ctl::set_mute(pid, false), Ok(muted));
        assert!(!volume.GetMute().unwrap().as_bool());

        // Leave the mixer entry of the test binary as it was found.
        if was {
            let _ = client_ctl::set_mute(pid, true);
        }
        drop(volume);
        drop(_client);
        CoUninitialize();
    }
}

#[test]
fn trim_memory_pages_a_child_out() {
    let child = spawn_idle();
    // Let it get as far as mapping its image.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(client_ctl::trim_memory(child.pid()), Ok(()));
    // Trimming twice is fine.
    assert_eq!(client_ctl::trim_memory(child.pid()), Ok(()));
    // The child is still running: nothing was ended.
    let mut child = child;
    assert!(matches!(child.0.try_wait(), Ok(None)));
}

#[test]
fn trim_memory_of_this_process_works_too() {
    assert_eq!(client_ctl::trim_memory(own_pid()), Ok(()));
}

#[test]
fn bandwidth_script_is_the_documented_command() {
    let limit = client_ctl::bandwidth_script(Some(1000));
    assert!(limit.starts_with(
        "Remove-NetQosPolicy -Name 'RAM-Roblox' -PolicyStore ActiveStore -Confirm:$false -ErrorAction SilentlyContinue; New-NetQosPolicy -Name 'RAM-Roblox' -AppPathNameMatchCondition 'RobloxPlayerBeta.exe' "
    ), "{limit}");
    assert!(
        limit.ends_with("-ThrottleRateActionBitsPerSecond 1000000 -PolicyStore ActiveStore | Out-Null"),
        "{limit}"
    );
    assert_eq!(
        client_ctl::bandwidth_script(None),
        "Remove-NetQosPolicy -Name 'RAM-Roblox' -PolicyStore ActiveStore -Confirm:$false -ErrorAction SilentlyContinue"
    );
}
