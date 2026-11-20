//! Event-driven tracking that posts messages to a GUI window instead of
//! polling.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use ram_core::platform::{PlatformResult, ProcKey};
use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    RegisterWaitForSingleObject, UnregisterWaitEx, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, WT_EXECUTEONLYONCE,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowThreadProcessId, PostMessageW, EVENT_OBJECT_SHOW, GA_ROOT, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::util::{file_name, image_path, last_error, map_err, open_process, open_verified, other, Handle};

struct ExitContext {
    hwnd: isize,
    msg: u32,
    pid: u32,
    start_low: u32,
}

struct Watch {
    wait: HANDLE,
    context: *mut ExitContext,
    // Declared last: the process handle must outlive the registered wait.
    _process: Handle,
}

unsafe extern "system" fn on_exit(context: *mut c_void, _timed_out: bool) {
    let ctx = &*(context as *const ExitContext);
    let _ = PostMessageW(
        Some(HWND(ctx.hwnd as _)),
        ctx.msg,
        WPARAM(ctx.pid as usize),
        LPARAM(ctx.start_low as isize),
    );
}

/// Posts a window message when a watched process exits.
///
/// The message arrives with `wParam` = PID and `lParam` = low 32 bits of the
/// process start time. The wait runs on the system thread pool and only
/// needs `SYNCHRONIZE` access to the process. A message already posted may
/// still be delivered after [`unwatch`](Self::unwatch).
#[derive(Default)]
pub struct ExitWatcher {
    watches: HashMap<ProcKey, Watch>,
}

impl ExitWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts watching `key`. Watching an already watched key does nothing.
    /// Returns `NotFound` when the process has already exited, in which case
    /// no message is posted.
    pub fn watch(&mut self, key: ProcKey, notify_hwnd: isize, msg: u32) -> PlatformResult<()> {
        if self.watches.contains_key(&key) {
            return Ok(());
        }
        let process = open_verified(key, PROCESS_SYNCHRONIZE)?;
        let context = Box::into_raw(Box::new(ExitContext {
            hwnd: notify_hwnd,
            msg,
            pid: key.pid,
            start_low: key.start_time as u32,
        }));
        let mut wait = HANDLE::default();
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                process.raw(),
                Some(on_exit),
                Some(context as *const c_void),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if let Err(e) = registered {
            drop(unsafe { Box::from_raw(context) });
            return Err(map_err(&e, "RegisterWaitForSingleObject"));
        }
        self.watches.insert(
            key,
            Watch {
                wait,
                context,
                _process: process,
            },
        );
        Ok(())
    }

    /// Stops watching `key`. Waits for a callback in progress to finish.
    pub fn unwatch(&mut self, key: ProcKey) {
        if let Some(watch) = self.watches.remove(&key) {
            release(watch);
        }
    }

    /// True while `key` is registered, including after its exit message has
    /// been posted and before `unwatch`.
    pub fn is_watching(&self, key: ProcKey) -> bool {
        self.watches.contains_key(&key)
    }
}

fn release(watch: Watch) {
    unsafe {
        // INVALID_HANDLE_VALUE blocks until any running callback returns, so
        // the context can be freed safely afterwards.
        let _ = UnregisterWaitEx(watch.wait, Some(INVALID_HANDLE_VALUE));
        drop(Box::from_raw(watch.context));
    }
}

impl Drop for ExitWatcher {
    fn drop(&mut self) {
        for (_, watch) in self.watches.drain() {
            release(watch);
        }
    }
}

struct HookConfig {
    hwnd: isize,
    msg: u32,
    names: Vec<String>,
}

thread_local! {
    static HOOK_CONFIG: RefCell<Option<HookConfig>> = const { RefCell::new(None) };
}

unsafe extern "system" fn on_show(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_OBJECT_SHOW || id_object != OBJID_WINDOW.0 || id_child != 0 || hwnd.0.is_null()
    {
        return;
    }
    if GetAncestor(hwnd, GA_ROOT) != hwnd {
        return;
    }
    let mut pid = 0u32;
    if GetWindowThreadProcessId(hwnd, Some(&mut pid)) == 0 || pid == 0 {
        return;
    }
    let Ok(process) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return;
    };
    let Some(path) = image_path(&process) else {
        return;
    };
    let image = file_name(&path);
    let _ = HOOK_CONFIG.try_with(|config| {
        if let Ok(config) = config.try_borrow() {
            if let Some(config) = config.as_ref() {
                if config.names.iter().any(|n| n.eq_ignore_ascii_case(image)) {
                    let _ = PostMessageW(
                        Some(HWND(config.hwnd as _)),
                        config.msg,
                        WPARAM(pid as usize),
                        LPARAM(0),
                    );
                }
            }
        }
    });
}

/// Posts a window message (`wParam` = PID, `lParam` = 0) whenever a process
/// with one of the given image names shows a top-level window.
///
/// This is a hint that a client probably just started, not an identity
/// check: confirm with `ProcessApi::list_clients`. The hook is out-of-context
/// (nothing is loaded into other processes) and its callback runs on the
/// installing thread, which must pump messages. One hook per thread; it must
/// be dropped on the thread that installed it. Windows of processes running
/// at a higher integrity level are not reported.
pub struct WindowShowHook {
    hook: HWINEVENTHOOK,
}

impl WindowShowHook {
    pub fn install(notify_hwnd: isize, msg: u32, image_names: &[&str]) -> PlatformResult<Self> {
        let already = HOOK_CONFIG.with(|config| config.borrow().is_some());
        if already {
            return Err(other("a window-show hook is already installed on this thread"));
        }
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_SHOW,
                None,
                Some(on_show),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.0.is_null() {
            return Err(last_error("SetWinEventHook"));
        }
        HOOK_CONFIG.with(|config| {
            *config.borrow_mut() = Some(HookConfig {
                hwnd: notify_hwnd,
                msg,
                names: image_names.iter().map(|n| n.to_string()).collect(),
            });
        });
        Ok(Self { hook })
    }
}

impl Drop for WindowShowHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.hook);
        }
        let _ = HOOK_CONFIG.try_with(|config| {
            if let Ok(mut config) = config.try_borrow_mut() {
                *config = None;
            }
        });
    }
}
