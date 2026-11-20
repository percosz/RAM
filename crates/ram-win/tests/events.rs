//! `ExitWatcher` and `WindowShowHook` delivering messages to a real window.

mod common;

use std::time::Duration;

use common::{key_of, ping_processes, powershell_processes, spawn_idle, spawn_window, MessageWindow};
use ram_core::platform::{PlatformError, ProcessApi};
use ram_win::{ExitWatcher, WindowShowHook};

const WM_APP: u32 = 0x8000;
const MSG_EXIT: u32 = WM_APP + 1;
const MSG_SHOW: u32 = WM_APP + 2;

#[test]
fn exit_watcher_posts_when_the_child_exits() {
    let window = MessageWindow::new();
    let procs = ping_processes();
    let mut child = spawn_idle();
    let mut other = spawn_idle();
    let key = key_of(&procs, &child);
    let other_key = key_of(&procs, &other);

    let mut watcher = ExitWatcher::new();
    watcher.watch(key, window.handle(), MSG_EXIT).unwrap();
    // Watching the same key again is a no-op and must not double-post.
    watcher.watch(key, window.handle(), MSG_EXIT).unwrap();
    watcher.watch(other_key, window.handle(), MSG_EXIT).unwrap();
    assert!(watcher.is_watching(key));

    // Nothing is posted while the processes run.
    assert_eq!(window.wait_for(MSG_EXIT, Duration::from_millis(300), |_, _| true), None);

    child.kill();
    let (wparam, lparam) = window
        .wait_for(MSG_EXIT, Duration::from_secs(5), |_, _| true)
        .expect("exit message");
    assert_eq!(wparam as u32, key.pid);
    assert_eq!(lparam as u32, key.start_time as u32);
    assert_eq!(
        window.wait_for(MSG_EXIT, Duration::from_millis(300), |_, _| true),
        None,
        "exactly one message per exit"
    );

    // An unwatched process posts nothing when it exits.
    watcher.unwatch(other_key);
    assert!(!watcher.is_watching(other_key));
    other.kill();
    assert_eq!(window.wait_for(MSG_EXIT, Duration::from_millis(500), |_, _| true), None);

    // A process that has already exited cannot be watched.
    assert_eq!(
        watcher.watch(key, window.handle(), MSG_EXIT),
        Ok(()),
        "still registered until unwatch"
    );
    watcher.unwatch(key);
    assert_eq!(watcher.watch(key, window.handle(), MSG_EXIT), Err(PlatformError::NotFound));
}

#[test]
fn dropping_a_watcher_with_live_processes_is_clean() {
    let window = MessageWindow::new();
    let procs = ping_processes();
    let mut child = spawn_idle();
    let key = key_of(&procs, &child);
    let mut watcher = ExitWatcher::new();
    watcher.watch(key, window.handle(), MSG_EXIT).unwrap();
    drop(watcher);
    child.kill();
    assert_eq!(window.wait_for(MSG_EXIT, Duration::from_millis(500), |_, _| true), None);
    assert!(!procs.is_alive(key));
}

#[test]
fn only_one_show_hook_per_thread() {
    let window = MessageWindow::new();
    let hook = WindowShowHook::install(window.handle(), MSG_SHOW, &["nothing.exe"]).unwrap();
    assert!(WindowShowHook::install(window.handle(), MSG_SHOW, &["nothing.exe"]).is_err());
    drop(hook);
    let again = WindowShowHook::install(window.handle(), MSG_SHOW, &["nothing.exe"]);
    assert!(again.is_ok());
}

/// Needs an interactive desktop: a child shows a real window.
#[test]
#[ignore = "opens a window on the interactive desktop"]
fn show_hook_reports_a_matching_process_window() {
    let window = MessageWindow::new();
    let _hook =
        WindowShowHook::install(window.handle(), MSG_SHOW, &["POWERSHELL.EXE"]).unwrap();
    let child = spawn_window();
    let pid = child.pid() as usize;
    let got = window.wait_for(MSG_SHOW, Duration::from_secs(30), |w, _| w == pid);
    assert_eq!(got, Some((pid, 0)), "no show event for the child's window");
    // The hint is confirmed the way the app does it.
    let procs = powershell_processes();
    assert!(procs.list_clients().unwrap().iter().any(|c| c.key.pid == child.pid()));
}
