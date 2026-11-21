//! Holds the Roblox singleton kernel objects so more than one client can run.
//!
//! The Roblox client refuses to start while another process owns a named
//! mutex. The manager creates and owns that mutex itself before launching
//! clients, so each client's own check passes. This owns named kernel objects
//! only; it never modifies, reads or injects into any client.

use ram_core::platform::PlatformResult;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex};

use crate::util::{map_err, wide};

/// Live ownership of the Roblox singleton mutex, plus a secondary event that
/// some client versions also check. Dropping it releases and closes both.
pub struct SingletonHold {
    mutex: HANDLE,
    event: HANDLE,
    /// True when this process created the mutex as its first owner.
    owned: bool,
}

// The handles are process-wide kernel objects, usable from any thread.
unsafe impl Send for SingletonHold {}
unsafe impl Sync for SingletonHold {}

impl SingletonHold {
    /// True when this process is the mutex's first owner, i.e. no other process
    /// already held it when the hold was taken.
    pub fn is_held(&self) -> bool {
        self.owned
    }
}

/// Creates and owns `ROBLOX_singletonMutex` (and `ROBLOX_singletonEvent`) for
/// the manager's lifetime. See [`hold_named`] for the behaviour and return
/// contract.
pub fn hold() -> PlatformResult<SingletonHold> {
    create(w!("ROBLOX_singletonMutex"), w!("ROBLOX_singletonEvent"))
}

/// Like [`hold`] but with caller-supplied object names, for tests.
///
/// Requests initial ownership of the mutex. When another process already holds
/// it, this still keeps the handle (so the object stays alive) but reports
/// `is_held() == false`; only a genuine failure (null handle) returns an error.
pub fn hold_named(mutex: &str, event: &str) -> PlatformResult<SingletonHold> {
    let mutex_name = wide(mutex);
    let event_name = wide(event);
    create(PCWSTR(mutex_name.as_ptr()), PCWSTR(event_name.as_ptr()))
}

fn create(mutex: PCWSTR, event: PCWSTR) -> PlatformResult<SingletonHold> {
    // Request initial ownership, then read the last error before anything else
    // can overwrite it: ERROR_ALREADY_EXISTS means another process was first.
    let created = unsafe { CreateMutexW(None, true, mutex) };
    let already_existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let mutex_handle = created.map_err(|e| map_err(&e, "CreateMutex"))?;

    let event_handle = match unsafe { CreateEventW(None, true, false, event) } {
        Ok(handle) => handle,
        Err(e) => {
            // Don't leak the mutex we just created.
            unsafe {
                let _ = ReleaseMutex(mutex_handle);
                let _ = CloseHandle(mutex_handle);
            }
            return Err(map_err(&e, "CreateEvent"));
        }
    };

    Ok(SingletonHold {
        mutex: mutex_handle,
        event: event_handle,
        owned: !already_existed,
    })
}

impl Drop for SingletonHold {
    fn drop(&mut self) {
        unsafe {
            // Releasing a mutex this process doesn't own fails harmlessly.
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Per-test unique names so parallel runs (and re-runs) never clash.
    fn unique_names() -> (String, String) {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        (
            format!("RAM_test_mutex_{pid}_{nanos}_{n}"),
            format!("RAM_test_event_{pid}_{nanos}_{n}"),
        )
    }

    #[test]
    fn first_owner_is_held_second_is_not() {
        let (m, e) = unique_names();
        let first = hold_named(&m, &e).unwrap();
        assert!(first.is_held());

        let second = hold_named(&m, &e).unwrap();
        assert!(!second.is_held());

        drop(second);
        drop(first);
    }

    #[test]
    fn name_is_reacquirable_after_drop() {
        let (m, e) = unique_names();
        {
            let first = hold_named(&m, &e).unwrap();
            assert!(first.is_held());
        }
        // The first hold is gone, so a fresh hold owns the name again.
        let again = hold_named(&m, &e).unwrap();
        assert!(again.is_held());
    }
}
