//! `WinProcesses` against a real child process owned by the test.

mod common;

use std::os::windows::io::AsRawHandle;
use std::time::Duration;

use common::{key_of, ping_processes, spawn_idle};
use ram_core::platform::{
    ClientKind, JobApi, PlatformError, PriorityClass, ProcKey, ProcessApi, Rect, WindowApi,
};
use ram_win::{ExitWatcher, WinJobs, WinWindows};
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    GetProcessInformation, GetProcessTimes, ProcessPowerThrottling,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE,
};

fn creation_time(child: &common::TestChild) -> u64 {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(HANDLE(child.0.as_raw_handle()), &mut c, &mut e, &mut k, &mut u) }
        .expect("GetProcessTimes on own child");
    ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64
}

/// Reads the EcoQoS state back through the test's own handle to its child.
fn throttling_state(child: &common::TestChild) -> PROCESS_POWER_THROTTLING_STATE {
    let mut state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..Default::default()
    };
    unsafe {
        GetProcessInformation(
            HANDLE(child.0.as_raw_handle()),
            ProcessPowerThrottling,
            &mut state as *mut PROCESS_POWER_THROTTLING_STATE as *mut _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .expect("GetProcessInformation(ProcessPowerThrottling)");
    state
}

#[test]
fn list_clients_finds_child_with_matching_start_time() {
    let procs = ping_processes();
    let child = spawn_idle();
    let clients = {
        let _ = key_of(&procs, &child);
        procs.list_clients().unwrap()
    };
    let info = clients
        .iter()
        .find(|c| c.key.pid == child.pid())
        .expect("child listed");
    assert_eq!(info.key.start_time, creation_time(&child));
    assert_eq!(info.kind, ClientKind::Desktop);
    let path = info.exe_path.as_deref().expect("own child's path is readable");
    assert!(path.to_ascii_lowercase().ends_with(r"\ping.exe"), "{path}");
    assert!(procs.is_alive(info.key));
}

#[test]
fn default_matcher_does_not_report_unrelated_processes() {
    let child = spawn_idle();
    let clients = ram_win::WinProcesses::new().list_clients().unwrap();
    assert!(clients.iter().all(|c| c.key.pid != child.pid()));
    assert!(clients.iter().all(|c| c.key.pid != std::process::id()));
}

#[test]
fn sample_reports_memory_cpu_and_priority() {
    let procs = ping_processes();
    let child = spawn_idle();
    let key = key_of(&procs, &child);
    let sample = procs.sample(key).unwrap();
    assert!(sample.working_set > 0);
    assert!(sample.private_bytes > 0);
    // The child inherits the test runner's class (BelowNormal on GitHub's
    // runners), so compare with what the process API reports, not Normal.
    assert_eq!(sample.priority, Some(procs.priority(key).unwrap()));
    // GPU fields stay empty until collect_gpu has run.
    assert_eq!(sample.gpu_percent, None);
    assert_eq!(sample.gpu_dedicated_bytes, None);

    // Collection must work (or quietly do nothing) and never break sampling.
    procs.collect_gpu();
    std::thread::sleep(Duration::from_millis(200));
    procs.collect_gpu();
    let sample = procs.sample(key).unwrap();
    if let Some(percent) = sample.gpu_percent {
        assert!((0.0..=100.0).contains(&percent));
    }
    procs.set_gpu_enabled(false);
    procs.collect_gpu();
    let sample = procs.sample(key).unwrap();
    assert_eq!((sample.gpu_percent, sample.gpu_dedicated_bytes), (None, None));
    procs.set_gpu_enabled(true);
    procs.collect_gpu();
}

#[test]
fn priority_can_be_lowered_and_restored() {
    let procs = ping_processes();
    let child = spawn_idle();
    let key = key_of(&procs, &child);
    procs.set_priority(key, PriorityClass::Normal).unwrap();
    assert_eq!(procs.priority(key), Ok(PriorityClass::Normal));
    procs.set_priority(key, PriorityClass::BelowNormal).unwrap();
    assert_eq!(procs.priority(key), Ok(PriorityClass::BelowNormal));
    assert_eq!(procs.sample(key).unwrap().priority, Some(PriorityClass::BelowNormal));
    procs.set_priority(key, PriorityClass::Idle).unwrap();
    assert_eq!(procs.priority(key), Ok(PriorityClass::Idle));
    procs.set_priority(key, PriorityClass::Normal).unwrap();
    assert_eq!(procs.priority(key), Ok(PriorityClass::Normal));
}

#[test]
fn efficiency_mode_turns_on_and_back_to_system_managed() {
    let procs = ping_processes();
    let child = spawn_idle();
    let key = key_of(&procs, &child);
    let before = procs.priority(key).unwrap();

    procs.set_efficiency_mode(key, true).unwrap();
    let on = throttling_state(&child);
    assert_ne!(on.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED, 0, "{on:?}");

    procs.set_efficiency_mode(key, false).unwrap();
    let off = throttling_state(&child);
    assert_eq!(off.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED, 0, "{off:?}");
    assert_eq!(off.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED, 0, "{off:?}");

    // Efficiency mode leaves the priority class alone.
    assert_eq!(procs.priority(key), Ok(before));
}

#[test]
fn stale_key_is_not_found_for_every_mutating_call() {
    let procs = ping_processes();
    let child = spawn_idle();
    let key = key_of(&procs, &child);
    let before = procs.priority(key).unwrap();
    let stale = ProcKey {
        pid: key.pid,
        start_time: key.start_time + 1,
    };

    assert!(!procs.is_alive(stale));
    assert_eq!(procs.sample(stale), Err(PlatformError::NotFound));
    assert_eq!(procs.priority(stale), Err(PlatformError::NotFound));
    assert_eq!(
        procs.set_priority(stale, PriorityClass::Idle),
        Err(PlatformError::NotFound)
    );
    assert_eq!(procs.set_efficiency_mode(stale, true), Err(PlatformError::NotFound));
    assert_eq!(procs.set_efficiency_mode(stale, false), Err(PlatformError::NotFound));
    assert_eq!(procs.request_close(stale), Err(PlatformError::NotFound));

    let jobs = WinJobs::new();
    assert_eq!(jobs.apply_cpu_cap(stale, 50), Err(PlatformError::NotFound));
    assert!(!jobs.has_cpu_cap(stale));

    let windows = WinWindows::new(0);
    assert_eq!(windows.focus(stale), Err(PlatformError::NotFound));
    let rect = Rect {
        left: 0,
        top: 0,
        right: 200,
        bottom: 200,
    };
    assert_eq!(windows.move_window(stale, rect), Err(PlatformError::NotFound));

    let mut watcher = ExitWatcher::new();
    assert_eq!(watcher.watch(stale, 0, 0x8001), Err(PlatformError::NotFound));
    assert!(!watcher.is_watching(stale));

    // The real process was left exactly as it was.
    assert_eq!(procs.priority(key), Ok(before));
    assert!(procs.is_alive(key));
}

#[test]
fn request_close_without_a_window_is_not_found_and_harmless() {
    let procs = ping_processes();
    let mut child = spawn_idle();
    let key = key_of(&procs, &child);
    assert_eq!(procs.request_close(key), Err(PlatformError::NotFound));
    assert!(!child.wait_exit(Duration::from_millis(300)));
    assert!(procs.is_alive(key));
}

#[test]
fn exit_watch_reads_the_code_after_the_process_ended() {
    let procs = ping_processes();
    let mut child = spawn_idle();
    let key = key_of(&procs, &child);
    let watch = ram_win::process::ExitWatch::open(key).expect("running process opens");
    assert_eq!(watch.exit_code(), None, "still running");
    // Ended from outside, the way Task Manager does it: code 1, not an
    // exception code.
    child.kill();
    let code = watch.exit_code().expect("code is kept while the handle is open");
    assert_eq!(code, 1);
    assert!(!ram_core::clientlog::exit_is_crash(code));
    // A process that is already gone cannot be opened for watching.
    assert!(ram_win::process::ExitWatch::open(key).is_none());
}

#[test]
fn exited_process_is_not_found() {
    let procs = ping_processes();
    let mut child = spawn_idle();
    let key = key_of(&procs, &child);
    assert!(procs.is_alive(key));
    child.kill();
    // The test still holds a handle, so the PID still resolves; the exited
    // process must be reported as gone anyway.
    assert!(!procs.is_alive(key));
    assert_eq!(procs.sample(key), Err(PlatformError::NotFound));
    assert_eq!(
        procs.set_priority(key, PriorityClass::Idle),
        Err(PlatformError::NotFound)
    );
    assert!(procs.list_clients().unwrap().iter().all(|c| c.key != key));
}
