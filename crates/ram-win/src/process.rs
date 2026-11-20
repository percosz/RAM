//! Process discovery, sampling and reversible control.

use std::sync::Mutex;

use ram_core::platform::{
    ClientKind, PlatformError, PlatformResult, PriorityClass, ProcInfo, ProcKey, ProcSample,
    ProcessApi, SystemMemory,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetPriorityClass, ProcessPowerThrottling, SetPriorityClass,
    SetProcessInformation, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    REALTIME_PRIORITY_CLASS,
};
use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, PostMessageW, WM_CLOSE};

use crate::gpu::GpuSampler;
use crate::util::{
    code_of, file_name, from_wide, image_path, last_error, map_err, open_process, open_verified,
    process_times, top_level_windows, Handle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
};

/// Image name of the roblox.com desktop client.
pub const DESKTOP_IMAGE: &str = "RobloxPlayerBeta.exe";
/// Image name of the Microsoft Store client. It is shared with other UWP
/// apps, so the image path must also contain [`STORE_PATH_MARKER`].
pub const STORE_IMAGE: &str = "Windows10Universal.exe";
/// Package-family fragment that identifies the Store client's install path.
pub const STORE_PATH_MARKER: &str = "ROBLOXCORPORATION";

const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_INVALID_FUNCTION: u32 = 1;

struct GpuState {
    enabled: bool,
    sampler: Option<GpuSampler>,
}

/// [`ProcessApi`] backed by Toolhelp, the process status API and PDH.
///
/// Every call that takes a [`ProcKey`] re-checks the process creation time
/// first, so a recycled PID is reported as `NotFound` and never touched.
pub struct WinProcesses {
    desktop_names: Vec<String>,
    gpu: Mutex<GpuState>,
}

impl Default for WinProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl WinProcesses {
    /// Matches `RobloxPlayerBeta.exe` and the Store client.
    pub fn new() -> Self {
        Self::with_image_names(vec![DESKTOP_IMAGE.to_string()])
    }

    /// Replaces the list of image names reported as [`ClientKind::Desktop`].
    /// Names compare case-insensitively. Store detection is unchanged.
    pub fn with_image_names(names: Vec<String>) -> Self {
        Self {
            desktop_names: names,
            gpu: Mutex::new(GpuState {
                enabled: true,
                sampler: None,
            }),
        }
    }

    /// Performs one GPU counter collection. Call once per sampling tick,
    /// before [`ProcessApi::sample`]. The PDH query is opened on the first
    /// call; utilisation becomes available from the second call onwards.
    /// Does nothing while GPU sampling is disabled or unavailable.
    pub fn collect_gpu(&self) {
        let Ok(mut gpu) = self.gpu.lock() else {
            return;
        };
        if !gpu.enabled {
            return;
        }
        if gpu.sampler.is_none() {
            gpu.sampler = GpuSampler::new();
            if gpu.sampler.is_none() {
                // Counters are missing on this system; stop retrying.
                gpu.enabled = false;
                return;
            }
        }
        if let Some(sampler) = gpu.sampler.as_mut() {
            sampler.collect();
        }
    }

    /// Enables or disables GPU sampling. Disabling closes the PDH query, so
    /// nothing is collected while monitoring is hidden or off.
    pub fn set_gpu_enabled(&self, enabled: bool) {
        if let Ok(mut gpu) = self.gpu.lock() {
            gpu.enabled = enabled;
            if !enabled {
                gpu.sampler = None;
            }
        }
    }

    fn classify(&self, image: &str) -> Option<Candidate> {
        if self
            .desktop_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(image))
        {
            Some(Candidate::Desktop)
        } else if image.eq_ignore_ascii_case(STORE_IMAGE) {
            Some(Candidate::StoreIfPathMatches)
        } else {
            None
        }
    }
}

enum Candidate {
    Desktop,
    StoreIfPathMatches,
}

fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(pid, &mut session) }.ok()?;
    Some(session)
}

fn priority_from_raw(raw: u32) -> Option<PriorityClass> {
    Some(match raw {
        x if x == IDLE_PRIORITY_CLASS.0 => PriorityClass::Idle,
        x if x == BELOW_NORMAL_PRIORITY_CLASS.0 => PriorityClass::BelowNormal,
        x if x == NORMAL_PRIORITY_CLASS.0 => PriorityClass::Normal,
        x if x == ABOVE_NORMAL_PRIORITY_CLASS.0 => PriorityClass::AboveNormal,
        x if x == HIGH_PRIORITY_CLASS.0 => PriorityClass::High,
        x if x == REALTIME_PRIORITY_CLASS.0 => PriorityClass::Realtime,
        _ => return None,
    })
}

fn read_priority(h: &Handle) -> PlatformResult<PriorityClass> {
    let raw = unsafe { GetPriorityClass(h.raw()) };
    if raw == 0 {
        return Err(last_error("GetPriorityClass"));
    }
    priority_from_raw(raw).ok_or(PlatformError::Other {
        code: raw,
        message: "unknown priority class".into(),
    })
}

impl ProcessApi for WinProcesses {
    fn list_clients(&self) -> PlatformResult<Vec<ProcInfo>> {
        let my_session = session_of(unsafe { GetCurrentProcessId() });
        let snapshot = Handle(
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
                .map_err(|e| map_err(&e, "CreateToolhelp32Snapshot"))?,
        );
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut clients = Vec::new();
        let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) }.is_ok();
        while more {
            let pid = entry.th32ProcessID;
            if let Some(candidate) = self.classify(&from_wide(&entry.szExeFile)) {
                if my_session.is_some() && session_of(pid) == my_session {
                    // Processes whose times cannot be read have no usable
                    // identity and are skipped.
                    if let Ok(handle) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
                        if let Ok((start_time, _)) = process_times(&handle) {
                            let exe_path = image_path(&handle);
                            let kind = match candidate {
                                Candidate::Desktop => Some(ClientKind::Desktop),
                                Candidate::StoreIfPathMatches => exe_path
                                    .as_deref()
                                    .filter(|p| p.to_ascii_uppercase().contains(STORE_PATH_MARKER))
                                    .map(|_| ClientKind::Store),
                            };
                            if let Some(kind) = kind {
                                clients.push(ProcInfo {
                                    key: ProcKey { pid, start_time },
                                    kind,
                                    exe_path,
                                });
                            }
                        }
                    }
                }
            }
            more = unsafe { Process32NextW(snapshot.raw(), &mut entry) }.is_ok();
        }
        Ok(clients)
    }

    fn is_alive(&self, key: ProcKey) -> bool {
        open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION).is_ok()
    }

    fn sample(&self, key: ProcKey) -> PlatformResult<ProcSample> {
        let handle = open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let (_, cpu_time) = process_times(&handle)?;

        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        counters.cb = size;
        unsafe {
            GetProcessMemoryInfo(
                handle.raw(),
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                size,
            )
        }
        .map_err(|e| map_err(&e, "GetProcessMemoryInfo"))?;

        let (gpu_percent, gpu_dedicated_bytes) = match self.gpu.lock() {
            Ok(gpu) => gpu
                .sampler
                .as_ref()
                .map(|s| s.get(key.pid))
                .unwrap_or((None, None)),
            Err(_) => (None, None),
        };

        Ok(ProcSample {
            cpu_time,
            working_set: counters.WorkingSetSize as u64,
            private_bytes: counters.PrivateUsage as u64,
            priority: read_priority(&handle).ok(),
            gpu_percent,
            gpu_dedicated_bytes,
        })
    }

    fn priority(&self, key: ProcKey) -> PlatformResult<PriorityClass> {
        let handle = open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION)?;
        read_priority(&handle)
    }

    /// Sets the priority class. Windows silently applies `High` instead of
    /// `Realtime` when the caller lacks the increase-scheduling-priority
    /// privilege.
    fn set_priority(&self, key: ProcKey, class: PriorityClass) -> PlatformResult<()> {
        let raw = match class {
            PriorityClass::Idle => IDLE_PRIORITY_CLASS,
            PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
            PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            PriorityClass::High => HIGH_PRIORITY_CLASS,
            PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
        };
        let handle = open_verified(key, PROCESS_SET_INFORMATION)?;
        unsafe { SetPriorityClass(handle.raw(), raw) }.map_err(|e| map_err(&e, "SetPriorityClass"))
    }

    /// Turns EcoQoS execution-speed throttling on, or hands the decision back
    /// to Windows when `enabled` is false. Only the power-throttling state is
    /// changed; the priority class is left alone (Task Manager's "Efficiency
    /// mode" additionally lowers priority). Requires Windows 10 1709 or
    /// later; EcoQoS scheduling itself takes effect on Windows 11.
    fn set_efficiency_mode(&self, key: ProcKey, enabled: bool) -> PlatformResult<()> {
        let handle = open_verified(key, PROCESS_SET_INFORMATION)?;
        let (control, state) = if enabled {
            (
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            )
        } else {
            (0, 0)
        };
        let throttling = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: control,
            StateMask: state,
        };
        unsafe {
            SetProcessInformation(
                handle.raw(),
                ProcessPowerThrottling,
                &throttling as *const PROCESS_POWER_THROTTLING_STATE as *const _,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        }
        .map_err(|e| match code_of(&e) {
            ERROR_ACCESS_DENIED => PlatformError::AccessDenied,
            ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION => {
                PlatformError::Unsupported(
                    "efficiency mode is not available on this Windows version".into(),
                )
            }
            _ => map_err(&e, "SetProcessInformation"),
        })
    }

    /// Posts `WM_CLOSE` to every visible top-level window of the process,
    /// which is what clicking the close button does. The client decides
    /// whether and when to exit. `NotFound` when it has no visible window.
    fn request_close(&self, key: ProcKey) -> PlatformResult<()> {
        // Holding the handle keeps the PID from being reused while posting.
        let _guard = open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut posted = false;
        let mut failure = None;
        for hwnd in top_level_windows(key.pid) {
            if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
                continue;
            }
            match unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) } {
                Ok(()) => posted = true,
                Err(e) => failure = Some(map_err(&e, "PostMessage")),
            }
        }
        match (posted, failure) {
            (true, _) => Ok(()),
            (false, Some(e)) => Err(e),
            (false, None) => Err(PlatformError::NotFound),
        }
    }

    fn system_memory(&self) -> PlatformResult<SystemMemory> {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut status) }
            .map_err(|e| map_err(&e, "GlobalMemoryStatusEx"))?;
        Ok(SystemMemory {
            total_physical: status.ullTotalPhys,
            available_physical: status.ullAvailPhys,
            load_percent: status.dwMemoryLoad.min(100),
        })
    }
}

/// File name of an image path, for matching against client image names.
pub fn image_file_name(path: &str) -> &str {
    file_name(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_image_names_case_insensitively() {
        let procs = WinProcesses::new();
        assert!(matches!(
            procs.classify("robloxplayerbeta.EXE"),
            Some(Candidate::Desktop)
        ));
        assert!(matches!(
            procs.classify("Windows10Universal.exe"),
            Some(Candidate::StoreIfPathMatches)
        ));
        assert!(procs.classify("RobloxStudioBeta.exe").is_none());
        assert!(procs.classify("notepad.exe").is_none());
    }

    #[test]
    fn priority_classes_round_trip() {
        assert_eq!(priority_from_raw(0x20), Some(PriorityClass::Normal));
        assert_eq!(priority_from_raw(0x4000), Some(PriorityClass::BelowNormal));
        assert_eq!(priority_from_raw(0x40), Some(PriorityClass::Idle));
        assert_eq!(priority_from_raw(0x12345), None);
    }

    #[test]
    fn system_memory_is_plausible() {
        let mem = WinProcesses::new().system_memory().unwrap();
        assert!(mem.total_physical > 0);
        assert!(mem.available_physical <= mem.total_physical);
        assert!(mem.load_percent <= 100);
    }

    #[test]
    fn pid_zero_and_stale_keys_are_not_found() {
        let procs = WinProcesses::new();
        let stale = ProcKey {
            pid: std::process::id(),
            start_time: 1,
        };
        assert!(!procs.is_alive(stale));
        assert_eq!(procs.sample(stale), Err(PlatformError::NotFound));
        assert_eq!(procs.priority(stale), Err(PlatformError::NotFound));
    }
}
