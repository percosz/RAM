//! Optional performance controls for background clients.
//!
//! Everything here is off by default and reversible. The manager records
//! what a client looked like before it changed anything (`Store::applied`)
//! and can put it back at any time. Presets only ever lower priority, turn
//! on efficiency mode, and optionally apply a Job Object CPU limit; they
//! never trim memory, close or terminate a client.

use crate::model::{AccountId, AppliedOverride, PerfSettings, PresetKind, Store};
use crate::platform::{
    JobApi, PlatformError, PriorityClass, ProcKey, ProcSample, ProcessApi, Rect, SystemMemory, WindowApi,
};
use std::collections::HashMap;

/// What a preset asks of a client process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    pub priority: Option<PriorityClass>,
    pub efficiency: bool,
    pub cpu_cap: Option<u32>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.priority.is_none() && !self.efficiency && self.cpu_cap.is_none()
    }
}

pub fn preset_plan(kind: PresetKind, perf: &PerfSettings) -> Plan {
    match kind {
        PresetKind::Off => Plan::default(),
        PresetKind::Balanced => Plan {
            priority: Some(PriorityClass::BelowNormal),
            efficiency: false,
            cpu_cap: perf.cpu_cap_percent,
        },
        PresetKind::LowResource => Plan {
            priority: Some(PriorityClass::Idle),
            efficiency: true,
            cpu_cap: perf.cpu_cap_percent,
        },
    }
}

/// The preset in effect for one client, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    Preset(PresetKind),
    /// The Main account's client; left alone unless the user opts in.
    ExemptMain,
    /// The client is not linked to an account. It might be Main, so it is
    /// never changed automatically.
    ExemptUnidentified,
}

impl Effective {
    pub fn label(self) -> String {
        match self {
            Effective::Preset(p) => p.label().to_string(),
            Effective::ExemptMain => "Exempt (Main)".to_string(),
            Effective::ExemptUnidentified => "Exempt (not identified)".to_string(),
        }
    }

    fn preset(self) -> PresetKind {
        match self {
            Effective::Preset(p) => p,
            _ => PresetKind::Off,
        }
    }
}

/// Resolves global preset, per-account override and the Main exemption.
pub fn effective(store: &Store, account: Option<AccountId>) -> Effective {
    let Some(id) = account else { return Effective::ExemptUnidentified };
    if store.is_main(id) && !store.perf.apply_to_main {
        return Effective::ExemptMain;
    }
    let preset = store.account(id).and_then(|a| a.perf_override).unwrap_or(store.perf.preset);
    Effective::Preset(preset)
}

/// Something the user should know about an attempted change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfNote {
    pub process: ProcKey,
    pub message: String,
}

fn describe(err: &PlatformError, what: &str) -> String {
    match err {
        PlatformError::AccessDenied => format!("Windows did not allow {what} for this client."),
        PlatformError::Unsupported(why) => format!("{what} is unavailable here: {why}."),
        other => format!("Could not change {what}: {other}."),
    }
}

/// Brings every live client to the state its effective preset asks for and
/// restores clients that should no longer be changed. Idempotent: calling it
/// again with nothing changed performs no OS calls that alter a process.
pub fn reconcile(
    store: &mut Store,
    clients: &[(ProcKey, Option<AccountId>)],
    procs: &dyn ProcessApi,
    jobs: &dyn JobApi,
) -> Vec<PerfNote> {
    let mut notes = Vec::new();
    // Entries for processes that are gone need no restoring.
    store.applied.retain(|e| procs.is_alive(e.process));

    for (key, account) in clients {
        if !procs.is_alive(*key) {
            continue;
        }
        let want = preset_plan(effective(store, *account).preset(), &store.perf);
        let pos = store.applied.iter().position(|e| e.process == *key);
        if want.is_empty() && pos.is_none() {
            continue;
        }
        let mut entry = match pos {
            Some(i) => store.applied.remove(i),
            None => AppliedOverride {
                process: *key,
                account: *account,
                original_priority: None,
                efficiency_mode: false,
                cpu_cap: false,
                original_rect: None,
            },
        };
        entry.account = *account;

        match (want.priority, entry.original_priority) {
            (Some(target), original) => match procs.priority(*key) {
                Ok(current) if current != target => {
                    // Only ever lower a client; one the user already set
                    // lower than the preset is left where it is.
                    if original.is_some() || current > target {
                        match procs.set_priority(*key, target) {
                            Ok(()) => {
                                entry.original_priority.get_or_insert(current);
                            }
                            Err(e) => notes.push(PerfNote { process: *key, message: describe(&e, "a priority change") }),
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => notes.push(PerfNote { process: *key, message: describe(&e, "a priority change") }),
            },
            (None, Some(original)) => match procs.set_priority(*key, original) {
                Ok(()) | Err(PlatformError::NotFound) => entry.original_priority = None,
                Err(e) => notes.push(PerfNote { process: *key, message: describe(&e, "restoring priority") }),
            },
            (None, None) => {}
        }

        if want.efficiency != entry.efficiency_mode {
            match procs.set_efficiency_mode(*key, want.efficiency) {
                Ok(()) | Err(PlatformError::NotFound) => entry.efficiency_mode = want.efficiency,
                Err(e) => notes.push(PerfNote { process: *key, message: describe(&e, "efficiency mode") }),
            }
        }

        match (want.cpu_cap, entry.cpu_cap) {
            (Some(percent), _) => match jobs.apply_cpu_cap(*key, percent) {
                Ok(()) => entry.cpu_cap = true,
                Err(e) => {
                    if !entry.cpu_cap {
                        notes.push(PerfNote { process: *key, message: describe(&e, "a CPU limit") });
                    }
                }
            },
            (None, true) => match jobs.clear_cpu_cap(*key) {
                Ok(()) | Err(PlatformError::NotFound) => entry.cpu_cap = false,
                Err(e) => notes.push(PerfNote { process: *key, message: describe(&e, "removing the CPU limit") }),
            },
            (None, false) => {}
        }

        if entry.original_priority.is_some() || entry.efficiency_mode || entry.cpu_cap || entry.original_rect.is_some() {
            store.applied.push(entry);
        }
    }
    notes
}

/// Restores every change the manager has made: priority, efficiency mode,
/// CPU limits and tiled window positions. Entries that could not be restored
/// stay in the ledger so the reset can be retried.
pub fn reset_all(
    store: &mut Store,
    procs: &dyn ProcessApi,
    jobs: &dyn JobApi,
    windows: &dyn WindowApi,
) -> Vec<PerfNote> {
    let mut notes = Vec::new();
    let mut remaining = Vec::new();
    for mut e in std::mem::take(&mut store.applied) {
        if !procs.is_alive(e.process) {
            continue;
        }
        if let Some(original) = e.original_priority {
            match procs.set_priority(e.process, original) {
                Ok(()) | Err(PlatformError::NotFound) => e.original_priority = None,
                Err(err) => notes.push(PerfNote { process: e.process, message: describe(&err, "restoring priority") }),
            }
        }
        if e.efficiency_mode {
            match procs.set_efficiency_mode(e.process, false) {
                Ok(()) | Err(PlatformError::NotFound) => e.efficiency_mode = false,
                Err(err) => notes.push(PerfNote { process: e.process, message: describe(&err, "efficiency mode") }),
            }
        }
        if e.cpu_cap {
            match jobs.clear_cpu_cap(e.process) {
                Ok(()) | Err(PlatformError::NotFound) => e.cpu_cap = false,
                Err(err) => notes.push(PerfNote { process: e.process, message: describe(&err, "removing the CPU limit") }),
            }
        }
        if let Some(rect) = e.original_rect {
            match windows.move_window(e.process, rect) {
                Ok(()) | Err(PlatformError::NotFound) => e.original_rect = None,
                Err(err) => notes.push(PerfNote { process: e.process, message: describe(&err, "restoring the window") }),
            }
        }
        if e.original_priority.is_some() || e.efficiency_mode || e.cpu_cap || e.original_rect.is_some() {
            remaining.push(e);
        }
    }
    store.applied = remaining;
    notes
}

/// True when any override is currently recorded as applied.
pub fn has_active_overrides(store: &Store) -> bool {
    !store.applied.is_empty()
}

/// Splits the work area into a grid of `count` cells, row by row.
pub fn tile_rects(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 || area.width() <= 0 || area.height() <= 0 {
        return Vec::new();
    }
    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    let (w, h) = (area.width() / cols as i32, area.height() / rows as i32);
    (0..count)
        .map(|i| {
            let (c, r) = ((i % cols) as i32, (i / cols) as i32);
            Rect { left: area.left + c * w, top: area.top + r * h, right: area.left + (c + 1) * w, bottom: area.top + (r + 1) * h }
        })
        .collect()
}

/// Tiles the given positively identified clients across the work area and
/// records where each window was so the reset can put it back.
pub fn tile(store: &mut Store, clients: &[(ProcKey, Option<AccountId>)], windows: &dyn WindowApi) -> Vec<PerfNote> {
    let mut notes = Vec::new();
    let targets: Vec<&(ProcKey, Option<AccountId>)> =
        clients.iter().filter(|(k, acc)| acc.is_some() && windows.has_main_window(*k)).collect();
    let rects = tile_rects(windows.work_area(), targets.len());
    for ((key, account), rect) in targets.into_iter().zip(rects) {
        let before = match windows.window_rect(*key) {
            Ok(r) => r,
            Err(e) => {
                notes.push(PerfNote { process: *key, message: describe(&e, "the window position") });
                continue;
            }
        };
        if let Err(e) = windows.move_window(*key, rect) {
            notes.push(PerfNote { process: *key, message: describe(&e, "the window position") });
            continue;
        }
        match store.applied.iter_mut().find(|e| e.process == *key) {
            Some(entry) => {
                entry.original_rect.get_or_insert(before);
            }
            None => store.applied.push(AppliedOverride {
                process: *key,
                account: *account,
                original_priority: None,
                efficiency_mode: false,
                cpu_cap: false,
                original_rect: Some(before),
            }),
        }
    }
    notes
}

/// CPU use between two samples as a percentage of the whole machine.
pub fn cpu_percent(prev_cpu_time: u64, cur_cpu_time: u64, elapsed_100ns: u64, logical_cpus: u32) -> f32 {
    if elapsed_100ns == 0 || logical_cpus == 0 || cur_cpu_time < prev_cpu_time {
        return 0.0;
    }
    let used = (cur_cpu_time - prev_cpu_time) as f64;
    ((used / (elapsed_100ns as f64 * logical_cpus as f64)) * 100.0).clamp(0.0, 100.0) as f32
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AdvisoryKind {
    ClientMemory(ProcKey),
    SystemMemory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    pub kind: AdvisoryKind,
    pub message: String,
}

/// Advisory memory warnings. These never act on a client; they only tell
/// the user, who can then close a background client with one click.
pub fn advisories(perf: &PerfSettings, samples: &[(ProcKey, ProcSample)], system: Option<SystemMemory>) -> Vec<Advisory> {
    let mut out = Vec::new();
    if perf.client_private_warn_mb > 0 {
        for (key, s) in samples {
            let mb = s.private_bytes / (1024 * 1024);
            if mb >= perf.client_private_warn_mb as u64 {
                out.push(Advisory {
                    kind: AdvisoryKind::ClientMemory(*key),
                    message: format!(
                        "Client {} is using {mb} MB of private memory (threshold {} MB).",
                        key.pid, perf.client_private_warn_mb
                    ),
                });
            }
        }
    }
    if let (true, Some(sys)) = (perf.system_load_warn_percent > 0, system) {
        if sys.load_percent >= perf.system_load_warn_percent {
            out.push(Advisory {
                kind: AdvisoryKind::SystemMemory,
                message: format!(
                    "System memory is {}% in use (threshold {}%).",
                    sys.load_percent, perf.system_load_warn_percent
                ),
            });
        }
    }
    out
}

/// Limits how often the same advisory raises a notification.
#[derive(Debug, Default)]
pub struct NotifyGate {
    last: HashMap<AdvisoryKind, i64>,
}

impl NotifyGate {
    pub const COOLDOWN_SECS: i64 = 600;

    pub fn should_notify(&mut self, kind: &AdvisoryKind, now: i64) -> bool {
        match self.last.get(kind) {
            Some(at) if now - at < Self::COOLDOWN_SECS => false,
            _ => {
                self.last.insert(kind.clone(), now);
                true
            }
        }
    }
}

/// Idle reminders: tells the user when a running client has not been the
/// foreground window for a while. The manager never sends input to a client.
#[derive(Debug, Default)]
pub struct IdleTracker {
    last_focus: HashMap<ProcKey, i64>,
    reminded: HashMap<ProcKey, i64>,
}

impl IdleTracker {
    /// Returns clients that just crossed the reminder threshold. Each client
    /// is reported once per idle stretch.
    pub fn tick(&mut self, now: i64, running: &[ProcKey], foreground_pid: Option<u32>, minutes: u32) -> Vec<ProcKey> {
        self.last_focus.retain(|k, _| running.contains(k));
        self.reminded.retain(|k, _| running.contains(k));
        let mut due = Vec::new();
        for key in running {
            if foreground_pid == Some(key.pid) {
                self.last_focus.insert(*key, now);
                self.reminded.remove(key);
                continue;
            }
            let since = *self.last_focus.entry(*key).or_insert(now);
            if minutes > 0 && now - since >= minutes as i64 * 60 && !self.reminded.contains_key(key) {
                self.reminded.insert(*key, now);
                due.push(*key);
            }
        }
        due
    }

    /// Seconds since the client was last in the foreground, as far as the
    /// manager has observed.
    pub fn idle_secs(&self, key: ProcKey, now: i64) -> Option<i64> {
        self.last_focus.get(&key).map(|t| now - t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeSystem;
    use crate::model::Account;

    fn store_with(n: u64) -> Store {
        let mut s = Store::default();
        for i in 1..=n {
            s.accounts.push(Account { id: i, username: format!("a{i}"), ..Default::default() });
        }
        s.next_id = n + 1;
        s.main_account = Some(1);
        s
    }

    #[test]
    fn disabled_by_default_changes_nothing() {
        let sys = FakeSystem::new();
        let (a, b) = (sys.spawn(1), sys.spawn(2));
        let mut s = store_with(2);
        assert_eq!(s.perf.preset, PresetKind::Off);
        let notes = reconcile(&mut s, &[(a, Some(1)), (b, Some(2))], &sys, &sys);
        assert!(notes.is_empty());
        assert!(s.applied.is_empty());
        assert_eq!(sys.get(b).priority, PriorityClass::Normal);
        assert!(sys.ever_capped.borrow().is_empty());
    }

    #[test]
    fn main_and_unidentified_clients_are_exempt() {
        let sys = FakeSystem::new();
        let (main, alt, unknown) = (sys.spawn(1), sys.spawn(2), sys.spawn(3));
        let mut s = store_with(2);
        s.perf.preset = PresetKind::LowResource;
        s.perf.cpu_cap_percent = Some(20);
        reconcile(&mut s, &[(main, Some(1)), (alt, Some(2)), (unknown, None)], &sys, &sys);

        assert_eq!(effective(&s, Some(1)), Effective::ExemptMain);
        assert_eq!(effective(&s, None), Effective::ExemptUnidentified);
        for exempt in [main, unknown] {
            let p = sys.get(exempt);
            assert_eq!(p.priority, PriorityClass::Normal);
            assert!(!p.efficiency);
            assert_eq!(p.cpu_cap, None);
        }
        let p = sys.get(alt);
        assert_eq!(p.priority, PriorityClass::Idle);
        assert!(p.efficiency);
        assert_eq!(p.cpu_cap, Some(20));
        assert_eq!(s.applied.len(), 1);
    }

    #[test]
    fn main_exemption_holds_even_with_a_per_account_override() {
        let sys = FakeSystem::new();
        let main = sys.spawn(1);
        let mut s = store_with(1);
        s.accounts[0].perf_override = Some(PresetKind::LowResource);
        reconcile(&mut s, &[(main, Some(1))], &sys, &sys);
        assert_eq!(sys.get(main).priority, PriorityClass::Normal);
        s.perf.apply_to_main = true;
        reconcile(&mut s, &[(main, Some(1))], &sys, &sys);
        assert_eq!(sys.get(main).priority, PriorityClass::Idle);
    }

    #[test]
    fn switching_preset_off_restores_original_settings() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        sys.procs.borrow_mut().get_mut(&alt).unwrap().priority = PriorityClass::AboveNormal;
        let mut s = store_with(2);
        s.perf.preset = PresetKind::LowResource;
        s.perf.cpu_cap_percent = Some(25);
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(s.applied[0].original_priority, Some(PriorityClass::AboveNormal));

        // Balanced keeps the original that was recorded first.
        s.perf.preset = PresetKind::Balanced;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(sys.get(alt).priority, PriorityClass::BelowNormal);
        assert!(!sys.get(alt).efficiency);
        assert_eq!(s.applied[0].original_priority, Some(PriorityClass::AboveNormal));

        s.perf.preset = PresetKind::Off;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        let p = sys.get(alt);
        assert_eq!(p.priority, PriorityClass::AboveNormal);
        assert!(!p.efficiency);
        assert_eq!(p.cpu_cap, None);
        assert!(s.applied.is_empty());
    }

    #[test]
    fn becoming_main_restores_that_client() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        let mut s = store_with(2);
        s.perf.preset = PresetKind::Balanced;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(sys.get(alt).priority, PriorityClass::BelowNormal);
        s.main_account = Some(2);
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(sys.get(alt).priority, PriorityClass::Normal);
        assert!(s.applied.is_empty());
    }

    #[test]
    fn per_account_override_beats_global_preset() {
        let sys = FakeSystem::new();
        let (b, c) = (sys.spawn(2), sys.spawn(3));
        let mut s = store_with(3);
        s.perf.preset = PresetKind::Balanced;
        s.accounts[2].perf_override = Some(PresetKind::Off);
        reconcile(&mut s, &[(b, Some(2)), (c, Some(3))], &sys, &sys);
        assert_eq!(sys.get(b).priority, PriorityClass::BelowNormal);
        assert_eq!(sys.get(c).priority, PriorityClass::Normal);
    }

    #[test]
    fn reset_all_restores_everything_including_tiling() {
        let sys = FakeSystem::new();
        let (b, c) = (sys.spawn(2), sys.spawn(3));
        let mut s = store_with(3);
        s.perf.preset = PresetKind::LowResource;
        s.perf.cpu_cap_percent = Some(30);
        let clients = [(b, Some(2)), (c, Some(3))];
        reconcile(&mut s, &clients, &sys, &sys);
        let before = sys.get(b).rect;
        tile(&mut s, &clients, &sys);
        assert_ne!(sys.get(b).rect, before);

        let notes = reset_all(&mut s, &sys, &sys, &sys);
        assert!(notes.is_empty());
        for k in [b, c] {
            let p = sys.get(k);
            assert_eq!(p.priority, PriorityClass::Normal);
            assert!(!p.efficiency);
            assert_eq!(p.cpu_cap, None);
            assert_eq!(p.rect, before);
        }
        assert!(!has_active_overrides(&s));
    }

    #[test]
    fn restoration_survives_a_restart_through_the_ledger() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        let mut s = store_with(2);
        s.perf.preset = PresetKind::LowResource;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        // The store is saved, the manager restarts and loads it again.
        let json = serde_json::to_string(&s).unwrap();
        let mut reloaded: Store = serde_json::from_str(&json).unwrap();
        reset_all(&mut reloaded, &sys, &sys, &sys);
        assert_eq!(sys.get(alt).priority, PriorityClass::Normal);
        assert!(!sys.get(alt).efficiency);
    }

    #[test]
    fn protected_client_is_reported_and_not_recorded() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        sys.protect(alt);
        let mut s = store_with(2);
        s.perf.preset = PresetKind::Balanced;
        let notes = reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].message.contains("did not allow"));
        assert!(s.applied.is_empty());
    }

    #[test]
    fn exited_clients_drop_out_of_the_ledger() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        let mut s = store_with(2);
        s.perf.preset = PresetKind::Balanced;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        sys.kill(alt);
        reconcile(&mut s, &[], &sys, &sys);
        assert!(s.applied.is_empty());
    }

    #[test]
    fn a_client_already_lower_than_the_preset_is_left_alone() {
        let sys = FakeSystem::new();
        let alt = sys.spawn(2);
        sys.procs.borrow_mut().get_mut(&alt).unwrap().priority = PriorityClass::Idle;
        let mut s = store_with(2);
        s.perf.preset = PresetKind::Balanced;
        reconcile(&mut s, &[(alt, Some(2))], &sys, &sys);
        assert_eq!(sys.get(alt).priority, PriorityClass::Idle);
        assert!(s.applied.is_empty());
    }

    #[test]
    fn tiling_only_moves_identified_clients() {
        let sys = FakeSystem::new();
        let (b, unknown) = (sys.spawn(2), sys.spawn(9));
        let mut s = store_with(2);
        let before = sys.get(unknown).rect;
        tile(&mut s, &[(b, Some(2)), (unknown, None)], &sys);
        assert_eq!(sys.get(unknown).rect, before);
        assert_eq!(sys.get(b).rect, sys.work_area());
    }

    #[test]
    fn tile_rects_cover_grid() {
        let area = Rect { left: 0, top: 0, right: 1000, bottom: 800 };
        assert_eq!(tile_rects(area, 1), vec![area]);
        let four = tile_rects(area, 4);
        assert_eq!(four[3], Rect { left: 500, top: 400, right: 1000, bottom: 800 });
        assert_eq!(tile_rects(area, 3).len(), 3);
        assert!(tile_rects(area, 0).is_empty());
    }

    #[test]
    fn advisories_respect_thresholds_and_cooldown() {
        let key = ProcKey { pid: 4, start_time: 1 };
        let sample = ProcSample { private_bytes: 3000 << 20, ..Default::default() };
        let sys = SystemMemory { total_physical: 1, available_physical: 0, load_percent: 93 };
        let mut perf = PerfSettings::default();
        assert!(advisories(&perf, &[(key, sample)], Some(sys)).is_empty());
        perf.client_private_warn_mb = 2048;
        perf.system_load_warn_percent = 90;
        let adv = advisories(&perf, &[(key, sample)], Some(sys));
        assert_eq!(adv.len(), 2);
        let mut gate = NotifyGate::default();
        assert!(gate.should_notify(&adv[0].kind, 0));
        assert!(!gate.should_notify(&adv[0].kind, 100));
        assert!(gate.should_notify(&adv[0].kind, 700));
    }

    #[test]
    fn cpu_percent_is_relative_to_all_cores() {
        // One full core for one second on an 8-core machine.
        assert!((cpu_percent(0, 10_000_000, 10_000_000, 8) - 12.5).abs() < 0.01);
        assert_eq!(cpu_percent(5, 1, 10, 8), 0.0);
        assert_eq!(cpu_percent(0, 10, 0, 8), 0.0);
    }

    #[test]
    fn idle_reminder_fires_once_and_resets_on_focus() {
        let k = ProcKey { pid: 4, start_time: 1 };
        let mut t = IdleTracker::default();
        assert!(t.tick(0, &[k], None, 15).is_empty());
        assert!(t.tick(899, &[k], None, 15).is_empty());
        assert_eq!(t.tick(900, &[k], None, 15), vec![k]);
        assert!(t.tick(1000, &[k], None, 15).is_empty());
        assert!(t.tick(1100, &[k], Some(4), 15).is_empty());
        assert_eq!(t.idle_secs(k, 1160), Some(60));
        assert_eq!(t.tick(2000, &[k], None, 15), vec![k]);
        assert!(t.tick(9000, &[k], None, 0).is_empty());
    }
}
