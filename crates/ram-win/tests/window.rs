//! `WinWindows` and `request_close` against a real window in a child process.

mod common;

use std::time::{Duration, Instant};

use common::{key_of, powershell_processes, spawn_window};
use ram_core::platform::{ProcessApi, Rect, WindowApi};
use ram_win::WinWindows;

#[test]
fn work_area_of_primary_monitor_is_not_empty() {
    let area = WinWindows::new(0).work_area();
    assert!(area.width() > 0 && area.height() > 0, "{area:?}");
}

/// Needs an interactive desktop: a child shows a real window.
#[test]
#[ignore = "opens a window on the interactive desktop"]
fn window_can_be_found_moved_restored_and_closed() {
    let procs = powershell_processes();
    let windows = WinWindows::new(0);
    let mut child = spawn_window();
    let key = key_of(&procs, &child);

    let deadline = Instant::now() + Duration::from_secs(30);
    while !windows.has_main_window(key) {
        assert!(Instant::now() < deadline, "child never showed a window");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));

    let original = windows.window_rect(key).unwrap();
    assert!(original.width() > 0 && original.height() > 0, "{original:?}");

    let area = windows.work_area();
    let target = Rect {
        left: area.left + 60,
        top: area.top + 40,
        right: area.left + 60 + 520,
        bottom: area.top + 40 + 380,
    };
    windows.move_window(key, target).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(windows.window_rect(key).unwrap(), target);

    // Undo, as tiling does.
    windows.move_window(key, original).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(windows.window_rect(key).unwrap(), original);

    // Focus depends on the foreground lock; it must either work or say why.
    let focus = windows.focus(key);
    println!("focus: {focus:?}; foreground pid: {:?}", ram_win::window::foreground_pid());
    if focus.is_ok() {
        assert_eq!(ram_win::window::foreground_pid(), Some(key.pid));
    }

    // A close request ends the process by itself; the test never kills it.
    procs.request_close(key).unwrap();
    assert!(child.wait_exit(Duration::from_secs(15)), "child ignored the close request");
    assert!(!procs.is_alive(key));
    assert!(!windows.has_main_window(key));
}
