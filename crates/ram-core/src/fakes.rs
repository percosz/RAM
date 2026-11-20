//! In-memory implementations of the platform traits for tests.

use crate::platform::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct FakeProc {
    pub priority: PriorityClass,
    pub efficiency: bool,
    pub cpu_cap: Option<u32>,
    pub rect: Rect,
    pub close_requested: bool,
    /// When set, every change to this process is refused.
    pub protected: bool,
}

impl Default for FakeProc {
    fn default() -> Self {
        FakeProc {
            priority: PriorityClass::Normal,
            efficiency: false,
            cpu_cap: None,
            rect: Rect { left: 0, top: 0, right: 800, bottom: 600 },
            close_requested: false,
            protected: false,
        }
    }
}

/// A fake machine: a set of client processes that tests can add, remove and
/// inspect. Implements every process-facing trait.
#[derive(Default)]
pub struct FakeSystem {
    pub procs: RefCell<HashMap<ProcKey, FakeProc>>,
    pub focused: RefCell<Vec<ProcKey>>,
    pub jobs_unavailable: RefCell<bool>,
    pub ever_capped: RefCell<HashSet<ProcKey>>,
}

impl FakeSystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&self, pid: u32) -> ProcKey {
        let key = ProcKey { pid, start_time: 1000 + pid as u64 };
        self.procs.borrow_mut().insert(key, FakeProc::default());
        key
    }

    pub fn kill(&self, key: ProcKey) {
        self.procs.borrow_mut().remove(&key);
    }

    pub fn get(&self, key: ProcKey) -> FakeProc {
        self.procs.borrow().get(&key).cloned().expect("process exists")
    }

    pub fn protect(&self, key: ProcKey) {
        self.procs.borrow_mut().get_mut(&key).expect("process exists").protected = true;
    }

    fn with<T>(&self, key: ProcKey, write: bool, f: impl FnOnce(&mut FakeProc) -> T) -> PlatformResult<T> {
        let mut procs = self.procs.borrow_mut();
        let p = procs.get_mut(&key).ok_or(PlatformError::NotFound)?;
        if write && p.protected {
            return Err(PlatformError::AccessDenied);
        }
        Ok(f(p))
    }
}

impl ProcessApi for FakeSystem {
    fn list_clients(&self) -> PlatformResult<Vec<ProcInfo>> {
        let mut v: Vec<ProcInfo> = self
            .procs
            .borrow()
            .keys()
            .map(|k| ProcInfo { key: *k, kind: ClientKind::Desktop, exe_path: None })
            .collect();
        v.sort_by_key(|p| p.key);
        Ok(v)
    }
    fn is_alive(&self, key: ProcKey) -> bool {
        self.procs.borrow().contains_key(&key)
    }
    fn sample(&self, key: ProcKey) -> PlatformResult<ProcSample> {
        self.with(key, false, |p| ProcSample { priority: Some(p.priority), ..Default::default() })
    }
    fn priority(&self, key: ProcKey) -> PlatformResult<PriorityClass> {
        self.with(key, false, |p| p.priority)
    }
    fn set_priority(&self, key: ProcKey, class: PriorityClass) -> PlatformResult<()> {
        self.with(key, true, |p| p.priority = class)
    }
    fn set_efficiency_mode(&self, key: ProcKey, enabled: bool) -> PlatformResult<()> {
        self.with(key, true, |p| p.efficiency = enabled)
    }
    fn request_close(&self, key: ProcKey) -> PlatformResult<()> {
        self.with(key, false, |p| p.close_requested = true)
    }
    fn system_memory(&self) -> PlatformResult<SystemMemory> {
        Ok(SystemMemory { total_physical: 16 << 30, available_physical: 8 << 30, load_percent: 50 })
    }
}

impl JobApi for FakeSystem {
    fn apply_cpu_cap(&self, key: ProcKey, percent: u32) -> PlatformResult<()> {
        if *self.jobs_unavailable.borrow() {
            return Err(PlatformError::AccessDenied);
        }
        self.with(key, true, |p| p.cpu_cap = Some(percent))?;
        self.ever_capped.borrow_mut().insert(key);
        Ok(())
    }
    fn clear_cpu_cap(&self, key: ProcKey) -> PlatformResult<()> {
        self.with(key, true, |p| p.cpu_cap = None)
    }
    fn has_cpu_cap(&self, key: ProcKey) -> bool {
        self.procs.borrow().get(&key).map(|p| p.cpu_cap.is_some()).unwrap_or(false)
    }
}

impl WindowApi for FakeSystem {
    fn has_main_window(&self, key: ProcKey) -> bool {
        self.is_alive(key)
    }
    fn focus(&self, key: ProcKey) -> PlatformResult<()> {
        self.with(key, false, |_| ())?;
        self.focused.borrow_mut().push(key);
        Ok(())
    }
    fn window_rect(&self, key: ProcKey) -> PlatformResult<Rect> {
        self.with(key, false, |p| p.rect)
    }
    fn move_window(&self, key: ProcKey, rect: Rect) -> PlatformResult<()> {
        self.with(key, false, |p| p.rect = rect)
    }
    fn work_area(&self) -> Rect {
        Rect { left: 0, top: 0, right: 1920, bottom: 1040 }
    }
}

/// Scripted HTTP responses, consumed in order.
#[derive(Default)]
pub struct FakeHttp {
    pub responses: RefCell<Vec<Result<HttpResponse, NetError>>>,
    pub requests: RefCell<Vec<(String, String, String)>>,
}

impl FakeHttp {
    pub fn push(&self, status: u16, body: &str) {
        self.responses.borrow_mut().push(Ok(HttpResponse { status, retry_after: None, body: body.as_bytes().to_vec() }));
    }
    pub fn push_err(&self, e: NetError) {
        self.responses.borrow_mut().push(Err(e));
    }
}

impl HttpApi for FakeHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        _headers: &[(&str, &str)],
        body: Option<&[u8]>,
        _timeout_ms: u32,
    ) -> Result<HttpResponse, NetError> {
        self.requests.borrow_mut().push((
            method.to_string(),
            url.to_string(),
            String::from_utf8_lossy(body.unwrap_or_default()).into_owned(),
        ));
        let mut r = self.responses.borrow_mut();
        if r.is_empty() {
            Err(NetError::Other("no scripted response".into()))
        } else {
            r.remove(0)
        }
    }
}

#[derive(Default)]
pub struct FakeSecrets {
    pub items: RefCell<HashMap<String, Vec<u8>>>,
}

impl SecretStore for FakeSecrets {
    fn get(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        Ok(self.items.borrow().get(name).cloned())
    }
    fn set(&self, name: &str, value: &[u8]) -> PlatformResult<()> {
        self.items.borrow_mut().insert(name.to_string(), value.to_vec());
        Ok(())
    }
    fn delete(&self, name: &str) -> PlatformResult<()> {
        self.items.borrow_mut().remove(name);
        Ok(())
    }
}
