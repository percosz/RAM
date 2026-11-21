//! Windows implementations of the `ram-core` platform traits.
//!
//! Only documented Win32 APIs are used. Nothing here reads or writes another
//! process's memory, injects code, closes handles in other processes or
//! terminates a client.
#![cfg(windows)]

pub mod browser;
pub mod client_ctl;
pub mod crypto;
pub mod dpapi;
pub mod events;
pub mod gpu;
pub mod hello;
pub mod http;
pub mod job;
pub mod launch;
pub mod launcher_info;
pub mod multi_instance;
pub mod notify;
pub mod paths;
pub mod process;
pub mod secrets;
pub mod sysbrowser;
pub mod window;

mod util;

pub use browser::WinBrowsers;
pub use events::{ExitWatcher, WindowShowHook};
pub use gpu::GpuSampler;
pub use http::WinHttp;
pub use job::WinJobs;
pub use multi_instance::SingletonHold;
pub use notify::TrayNotifier;
pub use process::WinProcesses;
pub use secrets::WinSecrets;
pub use window::WinWindows;
