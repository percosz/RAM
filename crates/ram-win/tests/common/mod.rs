//! Helpers shared by the integration tests: child processes the tests own,
//! and a message-only window with a bounded message pump.
#![allow(dead_code)]

use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ram_core::platform::{ProcKey, ProcessApi};
use ram_win::WinProcesses;
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, HWND_MESSAGE, MSG, PM_REMOVE,
    WINDOW_EX_STYLE, WINDOW_STYLE,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A child process owned by the test, killed when the guard drops.
pub struct TestChild(pub Child);

impl TestChild {
    pub fn pid(&self) -> u32 {
        self.0.id()
    }

    /// Waits for the child to exit by itself.
    pub fn wait_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn(exe: &str, args: &[&str]) -> TestChild {
    let child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start {exe}: {e}"));
    TestChild(child)
}

/// An idle, windowless child that lives for about a minute.
pub fn spawn_idle() -> TestChild {
    spawn("ping.exe", &["-n", "60", "127.0.0.1"])
}

/// A child that spins one thread.
pub fn spawn_cpu_burner() -> TestChild {
    spawn(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", "while($true){}"],
    )
}

/// A child that shows one ordinary top-level window and exits when the
/// window is closed.
pub fn spawn_window() -> TestChild {
    spawn(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             $f = New-Object System.Windows.Forms.Form; \
             $f.Text = 'ram-win test window'; $f.Width = 420; $f.Height = 320; \
             [System.Windows.Forms.Application]::Run($f)",
        ],
    )
}

pub fn ping_processes() -> WinProcesses {
    WinProcesses::with_image_names(vec!["PING.EXE".into()])
}

pub fn powershell_processes() -> WinProcesses {
    WinProcesses::with_image_names(vec!["powershell.exe".into()])
}

/// Finds the key of a child through `list_clients`, as the app would.
pub fn key_of(procs: &WinProcesses, child: &TestChild) -> ProcKey {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let clients = procs.list_clients().expect("list_clients");
        if let Some(info) = clients.iter().find(|c| c.key.pid == child.pid()) {
            return info.key;
        }
        assert!(
            Instant::now() < deadline,
            "child {} never appeared in list_clients",
            child.pid()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A message-only window on the current thread.
pub struct MessageWindow(pub HWND);

impl MessageWindow {
    pub fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("ram-win test"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .expect("message-only window");
        Self(hwnd)
    }

    pub fn handle(&self) -> isize {
        self.0 .0 as isize
    }

    /// Pumps this thread's queue until a posted `msg` for this window that
    /// satisfies `accept` arrives; returns its (wParam, lParam).
    pub fn wait_for(
        &self,
        msg: u32,
        timeout: Duration,
        accept: impl Fn(usize, isize) -> bool,
    ) -> Option<(usize, isize)> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut m = MSG::default();
            while unsafe { PeekMessageW(&mut m, None, 0, 0, PM_REMOVE) }.as_bool() {
                if m.hwnd == self.0 && m.message == msg && accept(m.wParam.0, m.lParam.0) {
                    return Some((m.wParam.0, m.lParam.0));
                }
                unsafe { DispatchMessageW(&m) };
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
