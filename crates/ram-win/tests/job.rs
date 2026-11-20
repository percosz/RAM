//! `WinJobs` CPU caps against real child processes owned by the test.

mod common;

use std::time::{Duration, Instant};

use common::{key_of, ping_processes, powershell_processes, spawn_cpu_burner, spawn_idle};
use ram_core::platform::{JobApi, PlatformError, ProcKey, ProcessApi};
use ram_win::{WinJobs, WinProcesses};

/// Applies a cap, turning the one environment-specific failure into a clear
/// test failure instead of a silent pass.
fn apply_or_explain(jobs: &WinJobs, key: ProcKey, percent: u32) {
    match jobs.apply_cpu_cap(key, percent) {
        Ok(()) => {}
        Err(PlatformError::AccessDenied) => panic!(
            "AssignProcessToJobObject was denied: this test process is itself inside a job \
             that forbids nested jobs or CPU rate control (common in CI sandboxes). \
             The CPU cap cannot be verified in this environment."
        ),
        Err(e) => panic!("apply_cpu_cap failed: {e:?}"),
    }
}

#[test]
fn cap_can_be_applied_updated_and_cleared() {
    let procs = ping_processes();
    let child = spawn_idle();
    let key = key_of(&procs, &child);
    let jobs = WinJobs::new();

    assert!(!jobs.has_cpu_cap(key));
    assert_eq!(jobs.clear_cpu_cap(key), Ok(()));

    apply_or_explain(&jobs, key, 10);
    assert!(jobs.has_cpu_cap(key));

    // A second apply updates the rate on the job already held.
    apply_or_explain(&jobs, key, 25);
    assert!(jobs.has_cpu_cap(key));

    jobs.clear_cpu_cap(key).unwrap();
    assert!(!jobs.has_cpu_cap(key));
    jobs.clear_cpu_cap(key).unwrap();

    // Re-applying after a clear reuses the job the process is still in.
    apply_or_explain(&jobs, key, 50);
    assert!(jobs.has_cpu_cap(key));

    // The capped process is still running: no kill-on-close, no termination.
    drop(jobs);
    assert!(procs.is_alive(key));
}

/// CPU seconds the process consumes over `window` of wall time.
fn cpu_seconds(procs: &WinProcesses, key: ProcKey, window: Duration) -> f64 {
    let before = procs.sample(key).unwrap().cpu_time;
    let started = Instant::now();
    std::thread::sleep(window);
    let after = procs.sample(key).unwrap().cpu_time;
    let elapsed = started.elapsed().as_secs_f64();
    (after - before) as f64 / 1e7 / elapsed * window.as_secs_f64()
}

#[test]
fn cap_reduces_cpu_use_and_clearing_restores_it() {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let procs = powershell_processes();
    let child = spawn_cpu_burner();
    let key = key_of(&procs, &child);
    let window = Duration::from_secs(2);

    // Let PowerShell finish starting and reach its busy loop.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if cpu_seconds(&procs, key, Duration::from_millis(500)) > 0.3 {
            break;
        }
        assert!(Instant::now() < deadline, "burner never became busy");
    }

    let uncapped = cpu_seconds(&procs, key, window);

    // The rate is a share of all processors, so 1% is the only setting that
    // is below one busy thread on any machine with fewer than 100 threads.
    let jobs = WinJobs::new();
    apply_or_explain(&jobs, key, 1);
    std::thread::sleep(Duration::from_millis(300));
    let capped = cpu_seconds(&procs, key, window);

    jobs.clear_cpu_cap(key).unwrap();
    assert!(!jobs.has_cpu_cap(key));
    std::thread::sleep(Duration::from_millis(300));
    let cleared = cpu_seconds(&procs, key, window);

    println!(
        "cpu seconds per {:?} on {cores} logical processors: uncapped {uncapped:.2}, \
         capped at 1% {capped:.2} (allowance {:.2}), after clear {cleared:.2}",
        window,
        0.01 * cores as f64 * window.as_secs_f64()
    );

    assert!(uncapped > 1.0, "burner should use most of one core, got {uncapped:.2}");
    if cores <= 32 {
        assert!(
            capped < uncapped * 0.5,
            "cap had no clear effect: uncapped {uncapped:.2}, capped {capped:.2}"
        );
        assert!(
            cleared > capped * 1.5 && cleared > 1.0,
            "clearing did not restore speed: capped {capped:.2}, cleared {cleared:.2}"
        );
    } else {
        println!("more than 32 logical processors: a 1% cap exceeds a third of one core; effect not asserted");
    }
    assert!(procs.is_alive(key));
}

#[test]
fn dropping_the_manager_lifts_the_cap() {
    let procs = powershell_processes();
    let child = spawn_cpu_burner();
    let key = key_of(&procs, &child);
    let deadline = Instant::now() + Duration::from_secs(30);
    while cpu_seconds(&procs, key, Duration::from_millis(500)) <= 0.3 {
        assert!(Instant::now() < deadline, "burner never became busy");
    }

    let jobs = WinJobs::new();
    apply_or_explain(&jobs, key, 1);
    drop(jobs);
    std::thread::sleep(Duration::from_millis(300));
    let after_drop = cpu_seconds(&procs, key, Duration::from_secs(1));
    println!("cpu seconds per 1s after dropping WinJobs: {after_drop:.2}");
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if cores <= 32 {
        assert!(after_drop > 0.5, "cap survived drop: {after_drop:.2}");
    }
    assert!(procs.is_alive(key));
}
