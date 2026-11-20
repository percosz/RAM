//! Optional hard CPU cap through a Job Object.

use std::collections::HashMap;
use std::sync::Mutex;

use ram_core::platform::{JobApi, PlatformError, PlatformResult, ProcKey};
use windows::core::PCWSTR;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectCpuRateControlInformation,
    SetInformationJobObject, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
    JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0, JOB_OBJECT_CPU_RATE_CONTROL,
    JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
};
use windows::Win32::System::Threading::{
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

use crate::util::{map_err, open_verified, Handle};

struct Job {
    handle: Handle,
    capped: bool,
}

/// [`JobApi`] that places each capped process in its own unnamed job.
///
/// The job carries a CPU rate limit and nothing else: no kill-on-close, no
/// memory, UI or breakaway limits. Windows cannot remove a process from a
/// job, so clearing a cap disables rate control and keeps the job. Dropping
/// `WinJobs` disables every cap before closing the job handles; if the
/// manager is killed without running destructors, a cap stays in force until
/// that client exits. Assignment fails (and nothing is changed) when the
/// client already belongs to a job that does not allow nesting or already
/// has its own CPU rate control.
pub struct WinJobs {
    jobs: Mutex<HashMap<ProcKey, Job>>,
}

impl Default for WinJobs {
    fn default() -> Self {
        Self::new()
    }
}

impl WinJobs {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
        }
    }
}

/// `percent` is of total CPU across all processors; `None` disables control.
fn set_rate(job: &Handle, percent: Option<u32>) -> PlatformResult<()> {
    let info = match percent {
        Some(percent) => JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
            ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
            Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                CpuRate: percent * 100,
            },
        },
        None => JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
            ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL(0),
            Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 { CpuRate: 0 },
        },
    };
    unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectCpuRateControlInformation,
            &info as *const JOBOBJECT_CPU_RATE_CONTROL_INFORMATION as *const _,
            std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
        )
    }
    .map_err(|e| map_err(&e, "SetInformationJobObject"))
}

impl JobApi for WinJobs {
    /// `percent` is a share of total CPU time across all logical processors:
    /// 10 on a 16-thread machine allows about 1.6 cores.
    fn apply_cpu_cap(&self, key: ProcKey, percent: u32) -> PlatformResult<()> {
        if !(1..=100).contains(&percent) {
            return Err(PlatformError::Unsupported(
                "CPU cap must be between 1 and 100 percent".into(),
            ));
        }
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| crate::util::other("job table is poisoned"))?;

        // Forget jobs whose process has gone; an empty job needs no cleanup.
        jobs.retain(|k, _| open_verified(*k, PROCESS_QUERY_LIMITED_INFORMATION).is_ok());

        if let Some(job) = jobs.get_mut(&key) {
            set_rate(&job.handle, Some(percent))?;
            job.capped = true;
            return Ok(());
        }

        // AssignProcessToJobObject requires exactly these two rights.
        let process = open_verified(key, PROCESS_SET_QUOTA | PROCESS_TERMINATE)?;
        let job = Handle(
            unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .map_err(|e| map_err(&e, "CreateJobObject"))?,
        );
        // Configure before assigning so a failure here leaves the client untouched.
        set_rate(&job, Some(percent))?;
        unsafe { AssignProcessToJobObject(job.raw(), process.raw()) }
            .map_err(|e| map_err(&e, "AssignProcessToJobObject"))?;
        jobs.insert(
            key,
            Job {
                handle: job,
                capped: true,
            },
        );
        Ok(())
    }

    fn clear_cpu_cap(&self, key: ProcKey) -> PlatformResult<()> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| crate::util::other("job table is poisoned"))?;
        match jobs.get_mut(&key) {
            Some(job) => {
                if job.capped {
                    set_rate(&job.handle, None)?;
                    job.capped = false;
                }
                Ok(())
            }
            // Nothing was applied by this manager, so nothing is in force.
            None => Ok(()),
        }
    }

    fn has_cpu_cap(&self, key: ProcKey) -> bool {
        self.jobs
            .lock()
            .map(|jobs| jobs.get(&key).is_some_and(|job| job.capped))
            .unwrap_or(false)
    }
}

impl Drop for WinJobs {
    fn drop(&mut self) {
        let jobs = match self.jobs.get_mut() {
            Ok(jobs) => jobs,
            Err(poisoned) => poisoned.into_inner(),
        };
        for job in jobs.values_mut() {
            if job.capped {
                let _ = set_rate(&job.handle, None);
            }
        }
        jobs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_percent() {
        let jobs = WinJobs::new();
        let key = ProcKey {
            pid: std::process::id(),
            start_time: 1,
        };
        assert!(matches!(
            jobs.apply_cpu_cap(key, 0),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            jobs.apply_cpu_cap(key, 101),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(!jobs.has_cpu_cap(key));
    }

    #[test]
    fn stale_key_is_not_found_and_clear_is_a_no_op() {
        let jobs = WinJobs::new();
        let key = ProcKey {
            pid: std::process::id(),
            start_time: 1,
        };
        assert_eq!(jobs.apply_cpu_cap(key, 50), Err(PlatformError::NotFound));
        assert_eq!(jobs.clear_cpu_cap(key), Ok(()));
        assert!(!jobs.has_cpu_cap(key));
    }
}
