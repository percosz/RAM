//! Narrow interfaces to the operating system.
//!
//! Everything the core needs from Windows goes through these traits so the
//! policy code can be tested with fakes. `ram-win` provides the real
//! implementations; `crate::fakes` provides in-memory ones for tests.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Why a platform call failed. Callers must not collapse these: "access
/// denied" on a protected client is a normal, displayable limitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// The OS refused the operation (e.g. the client is protected).
    AccessDenied,
    /// The process, window or item no longer exists.
    NotFound,
    /// The operation is not available on this Windows version or setup.
    Unsupported(String),
    /// Anything else, with the OS error code where there is one.
    Other { code: u32, message: String },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::AccessDenied => write!(f, "access denied"),
            PlatformError::NotFound => write!(f, "no longer exists"),
            PlatformError::Unsupported(why) => write!(f, "unsupported: {why}"),
            PlatformError::Other { code, message } => write!(f, "{message} (error {code})"),
        }
    }
}

impl std::error::Error for PlatformError {}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// Identifies one process instance. PIDs are reused by Windows, so the
/// creation time is part of the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProcKey {
    pub pid: u32,
    /// Process creation time as a Windows FILETIME (100 ns ticks since 1601).
    pub start_time: u64,
}

/// Which Roblox client distribution a process belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    /// `RobloxPlayerBeta.exe` from roblox.com.
    Desktop,
    /// The Microsoft Store package.
    Store,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub key: ProcKey,
    pub kind: ClientKind,
    /// Full image path, when the OS lets us read it.
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl PriorityClass {
    pub fn label(self) -> &'static str {
        match self {
            PriorityClass::Idle => "Low",
            PriorityClass::BelowNormal => "Below normal",
            PriorityClass::Normal => "Normal",
            PriorityClass::AboveNormal => "Above normal",
            PriorityClass::High => "High",
            PriorityClass::Realtime => "Realtime",
        }
    }
}

/// One low-rate sample of a process. Fields are `None` when Windows does not
/// expose a reliable value; the UI shows a dash rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProcSample {
    /// Kernel + user CPU time consumed so far, in 100 ns ticks.
    pub cpu_time: u64,
    pub working_set: u64,
    pub private_bytes: u64,
    pub priority: Option<PriorityClass>,
    /// Sum of this process's GPU engine utilisation, 0..=100.
    pub gpu_percent: Option<f32>,
    /// Dedicated GPU memory attributed to this process, in bytes.
    pub gpu_dedicated_bytes: Option<u64>,
}

/// System-wide memory state used for advisory thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemMemory {
    pub total_physical: u64,
    pub available_physical: u64,
    /// Percentage of physical memory in use, 0..=100.
    pub load_percent: u32,
}

/// A screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Process inspection and reversible control.
pub trait ProcessApi {
    /// Snapshot of running Roblox client processes owned by this user session.
    fn list_clients(&self) -> PlatformResult<Vec<ProcInfo>>;
    /// True while this exact process instance is still running.
    fn is_alive(&self, key: ProcKey) -> bool;
    fn sample(&self, key: ProcKey) -> PlatformResult<ProcSample>;
    fn priority(&self, key: ProcKey) -> PlatformResult<PriorityClass>;
    fn set_priority(&self, key: ProcKey, class: PriorityClass) -> PlatformResult<()>;
    /// Enables or disables Windows efficiency mode (EcoQoS) for the process.
    fn set_efficiency_mode(&self, key: ProcKey, enabled: bool) -> PlatformResult<()>;
    /// Asks the client to close by sending its main window a close request.
    /// Never terminates the process.
    fn request_close(&self, key: ProcKey) -> PlatformResult<()>;
    fn system_memory(&self) -> PlatformResult<SystemMemory>;
}

/// Optional hard CPU limit through a Windows Job Object.
pub trait JobApi {
    /// Places the process in a job owned by the manager and limits the job to
    /// `percent` (1..=100) of total CPU. Fails without side effects when the
    /// process cannot be assigned safely.
    fn apply_cpu_cap(&self, key: ProcKey, percent: u32) -> PlatformResult<()>;
    /// Removes the rate limit. The process stays in the (now unlimited) job
    /// because Windows cannot remove a process from a job.
    fn clear_cpu_cap(&self, key: ProcKey) -> PlatformResult<()>;
    fn has_cpu_cap(&self, key: ProcKey) -> bool;
}

/// Window management for positively identified clients.
pub trait WindowApi {
    /// True when the process currently owns a visible top-level window.
    fn has_main_window(&self, key: ProcKey) -> bool;
    fn focus(&self, key: ProcKey) -> PlatformResult<()>;
    /// Current placement, so tiling can be undone.
    fn window_rect(&self, key: ProcKey) -> PlatformResult<Rect>;
    fn move_window(&self, key: ProcKey, rect: Rect) -> PlatformResult<()>;
    /// Work area of the monitor the manager window is on.
    fn work_area(&self) -> Rect;
}

/// An installed browser that has documented profile-launch support, or the
/// system default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserInstall {
    /// Stable identifier: "chrome", "edge", "brave", "vivaldi", "firefox".
    pub id: String,
    pub display_name: String,
    pub exe_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProfile {
    /// Value passed on the command line (directory name for Chromium
    /// browsers, profile name for Firefox).
    pub key: String,
    /// Name the user sees in the browser's profile menu.
    pub display_name: String,
}

pub trait BrowserApi {
    fn installed(&self) -> Vec<BrowserInstall>;
    /// Named profiles for a browser. Reads profile names only; never cookies,
    /// history or saved passwords.
    fn profiles(&self, browser_id: &str) -> Vec<BrowserProfile>;
    /// Opens `url` in the given browser profile, or in the default browser
    /// when `target` is `None`.
    fn open(&self, target: Option<(&str, &str)>, url: &str) -> PlatformResult<()>;
}

/// Windows-backed protected storage for authorization secrets.
pub trait SecretStore {
    fn get(&self, name: &str) -> PlatformResult<Option<Vec<u8>>>;
    fn set(&self, name: &str, value: &[u8]) -> PlatformResult<()>;
    fn delete(&self, name: &str) -> PlatformResult<()>;
}

/// Transport-level failure. None of these say anything about whether the
/// account is signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    Offline,
    Timeout,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Value of `Retry-After` in seconds when present.
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

pub trait HttpApi {
    /// Performs one HTTPS request. `headers` are (name, value) pairs.
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        timeout_ms: u32,
    ) -> Result<HttpResponse, NetError>;
}

pub trait Notifier {
    fn notify(&self, title: &str, body: &str);
}

pub trait Clock {
    /// Seconds since the Unix epoch.
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}
