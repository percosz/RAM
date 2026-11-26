//! Diagnostics: the client watcher (links Roblox clients to accounts through
//! their logs, records how each one ended) and the Debug log commands.
//!
//! Every two seconds the watcher lists Roblox client processes, binds each
//! new one to the `_Player_` log whose name carries the same start time,
//! tails the log, and when the process is gone classifies the ending
//! (`ram_core::clientlog::classify`) into the crash history. A log that
//! names the browser tracker id of a launch RAM made links that client to
//! its account; otherwise the `userid` Roblox prints at join time does.

use crate::state::{now, Shared};
use ram_core::clientlog::{self, EndKind, LogFacts, Tail};
use ram_core::model::{AccountId, CrashRecord, Target};
use ram_core::rejoin;
use ram_core::roblox::LaunchTarget;
use ram_core::platform::{JobApi, ProcKey, ProcessApi};
use ram_core::redact::{redact, Level};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// A launch RAM made, waiting for its client to show up.
pub struct PendingLaunch {
    pub account: AccountId,
    pub browser_tracker_id: String,
    pub at: i64,
}

pub struct Watch {
    pub account: Option<AccountId>,
    pub started: i64,
    pub tail: Option<Tail>,
    pub facts: LogFacts,
    pub browser_tracker_id: Option<String>,
    /// Crash dumps present when the client started, to spot a new one.
    pub dumps_at_start: usize,
    pub closed_by_ram: bool,
    /// Kept open so the exit code can be read once the client is gone: it
    /// is what tells "ended from outside" from a crash.
    pub exit: Option<ram_win::process::ExitWatch>,
    /// What the limits have done to this client so far.
    pub limits: Applied,
}

/// Limits applied to one client, so each is done once and undone when the
/// setting goes off.
#[derive(Default)]
pub struct Applied {
    /// Tiny / minimize / mute were issued after the client joined a game.
    pub on_join: bool,
    pub cpu_capped: bool,
    pub efficiency: bool,
    pub last_trim: i64,
}

/// How an account last joined, as the user asked for it.
#[derive(Clone)]
pub struct LastLaunch {
    pub target: LaunchTarget,
    pub model_target: Target,
    /// For a follow: the place this account went to before, used when the
    /// followed user can no longer be joined.
    pub fallback_place: Option<u64>,
}

impl LastLaunch {
    /// The place this launch leads to, when it names one.
    pub fn place(&self) -> Option<u64> {
        match &self.target {
            LaunchTarget::Place { place_id } | LaunchTarget::Server { place_id, .. } => Some(*place_id),
            LaunchTarget::Follow { .. } => self.fallback_place,
        }
    }

    /// The target for a rejoin attempt. The first attempt repeats the launch
    /// exactly; later ones fall back to the place (a specific server may be
    /// gone, a followed user may have left).
    pub fn for_attempt(&self, fallback: bool) -> (LaunchTarget, Target) {
        match (&self.target, fallback) {
            (LaunchTarget::Server { place_id, .. }, true) => (LaunchTarget::Place { place_id: *place_id }, Target::Place { place_id: *place_id }),
            (LaunchTarget::Follow { .. }, true) => match self.fallback_place {
                Some(place_id) => (LaunchTarget::Place { place_id }, Target::Place { place_id }),
                None => (self.target.clone(), self.model_target.clone()),
            },
            _ => (self.target.clone(), self.model_target.clone()),
        }
    }
}

/// Auto-rejoin progress for one account.
#[derive(Clone, Default)]
pub struct RejoinState {
    /// Attempts since the last client that stayed up.
    pub attempts: u32,
    /// When the next attempt launches.
    pub due: Option<i64>,
    pub fallback: bool,
    /// Set when an attempt launched, until its client is in a game.
    pub awaiting_since: Option<i64>,
}

pub struct DiagState {
    pub watched: HashMap<ProcKey, Watch>,
    pub launches: Vec<PendingLaunch>,
    /// Browser tracker ids whose client has read its settings files.
    pub reads: HashMap<String, bool>,
    /// Processes RAM asked to close, so they are recorded as Closed.
    pub closing: Vec<u32>,
    /// Job objects behind the CPU limits.
    pub jobs: ram_win::WinJobs,
    /// How each account last joined (this session), for auto-rejoin.
    pub last_launch: HashMap<AccountId, LastLaunch>,
    pub rejoin: HashMap<AccountId, RejoinState>,
}

impl Default for DiagState {
    fn default() -> Self {
        DiagState {
            watched: HashMap::new(),
            launches: Vec::new(),
            reads: HashMap::new(),
            closing: Vec::new(),
            jobs: ram_win::WinJobs::new(),
            last_launch: HashMap::new(),
            rejoin: HashMap::new(),
        }
    }
}

/// Something to do to a client window or process outside the state lock
/// (these calls can wait on the client for up to a second).
enum Act {
    OnJoin { pid: u32, tiny: Option<(i32, i32)>, minimize: bool, mute: bool },
    Trim { pid: u32 },
}

fn perform(acts: Vec<Act>) {
    if acts.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for act in acts {
            match act {
                Act::OnJoin { pid, tiny, minimize, mute } => {
                    if let Some((w, h)) = tiny {
                        let _ = ram_win::client_ctl::set_tiny(pid, w, h);
                    }
                    if minimize {
                        let _ = ram_win::client_ctl::minimize(pid);
                    }
                    if mute {
                        // The audio session appears a little after the join.
                        for _ in 0..6 {
                            match ram_win::client_ctl::set_mute(pid, true) {
                                Ok(n) if n > 0 => break,
                                _ => std::thread::sleep(Duration::from_secs(2)),
                            }
                        }
                    }
                }
                Act::Trim { pid } => {
                    let _ = ram_win::client_ctl::trim_memory(pid);
                }
            }
        }
    });
}

#[derive(Serialize, Clone)]
pub struct CrashView {
    pub id: u64,
    pub account: Option<AccountId>,
    pub username: String,
    pub kind: EndKind,
    pub at: i64,
    pub uptime_secs: i64,
    pub reason: String,
    pub place_id: Option<u64>,
    pub place_name: String,
    pub has_log: bool,
    pub pid: u32,
}

#[derive(Serialize, Clone)]
pub struct LogLineView {
    pub level: &'static str,
    pub text: String,
}

fn filetime_to_unix(ft: u64) -> i64 {
    (ft / 10_000_000) as i64 - 11_644_473_600
}

fn logs_dir() -> Option<PathBuf> {
    ram_win::paths::roblox_logs_dir()
}

/// Crash dumps (`.dmp`) written since `since`. Only dump files count: the
/// crash handler rewrites `settings.dat` and `metadata` in the same folder on
/// every run, which used to make each ending look like a crash.
fn count_dumps(since: i64) -> usize {
    let Some(dir) = logs_dir() else { return 0 };
    let mut n = 0;
    let mut stack = vec![dir.join("crashes")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if !is_dump(&p) {
                continue;
            } else if let Ok(m) = e.metadata() {
                let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                if t >= since {
                    n += 1;
                }
            }
        }
    }
    n
}

fn is_dump(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dmp"))
}

/// The `_Player_` log a client started at `start` wrote: nearest start time
/// in the file name within `skew` seconds, not already bound to another
/// client, preferring one whose head names one of `trackers`.
fn find_log(dir: &Path, start: i64, skew: i64, taken: &[PathBuf], trackers: &[String]) -> Option<(PathBuf, Option<String>)> {
    let mut candidates: Vec<(i64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !clientlog::is_player_log_name(&name) {
                return None;
            }
            let t = clientlog::log_start_time(&name)?;
            let p = e.path();
            ((t - start).abs() <= skew && !taken.contains(&p)).then_some(((t - start).abs(), p))
        })
        .collect();
    candidates.sort();
    if !trackers.is_empty() {
        for (_, p) in &candidates {
            if let Ok(head) = clientlog::read_head(p, 256 * 1024) {
                if let Some(t) = trackers.iter().find(|t| clientlog::tracker_in_head(&head, t)) {
                    return Some((p.clone(), Some(t.clone())));
                }
            }
        }
    }
    candidates.into_iter().next().map(|(_, p)| (p, None))
}

/// Starts the watcher thread.
/// How long RAM must be out of front before its browser engine's memory is
/// handed back (so a quick switch away and back costs nothing), and how
/// often that is repeated while it stays there.
const TRIM_AFTER_SECS: i64 = 6;
const TRIM_EVERY_SECS: u64 = 30;

pub fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        register_notifications(&app);
        let kept = ram_win::process::keep_resident();
        if let Ok(s) = app.state::<Shared>().lock() {
            s.log.debug(&format!("memory: RAM's own pages kept in memory: {kept}"));
        }
        let mut ticks: u32 = 0;
        let mut trimmed: Option<std::time::Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            // While RAM is not the window in front, hand the memory of its
            // browser engine back to Windows, and again every so often:
            // what the page still uses comes straight back, the rest stays
            // free for the game clients.
            match crate::window::background_secs() {
                Some(secs) if secs >= TRIM_AFTER_SECS => {
                    if trimmed.map_or(true, |t| t.elapsed() >= Duration::from_secs(TRIM_EVERY_SECS)) {
                        ram_win::process::trim_own_webviews();
                        trimmed = Some(std::time::Instant::now());
                    }
                }
                Some(_) => {}
                None => trimmed = None,
            }
            // Keep RAM's own window ahead of the game clients for CPU time.
            // Repeated because WebView2 starts new helper processes over time.
            if ticks % 30 == 1 {
                let n = ram_win::process::boost_own_processes();
                if ticks == 1 {
                    if let Ok(s) = app.state::<Shared>().lock() {
                        s.log.debug(&format!("priority: {n} of RAM's own processes set to above normal"));
                    }
                }
            }
            ticks = ticks.wrapping_add(1);
            // Auto-lock for a master-password vault. Timed here and not by
            // the page, so it also happens while the window is minimized.
            let locked = app.state::<Shared>().lock().map(|mut s| s.vault.tick(now())).unwrap_or(false);
            if locked {
                let _ = app.emit("vault-locked", ());
            }
            tick(&app);
            // Test hook, debug builds only: keep the state locked for a while,
            // the way a slow tick on an overloaded PC does, so a test can
            // check that the window still answers meanwhile.
            #[cfg(debug_assertions)]
            if let Some(secs) = std::env::var("RAM_TEST_HOLD_LOCK_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
                if let Ok(_held) = app.state::<Shared>().lock() {
                    std::thread::sleep(Duration::from_secs(secs));
                }
            }
        }
    });
}

/// The icon Windows shows on RAM's notifications.
const NOTIFY_ICON: &[u8] = include_bytes!("../icons/icon.png");

/// Tells Windows who RAM's notifications are from (name and icon). Without
/// this a copy with no Start menu shortcut, the portable build, has its
/// notifications dropped without any error.
fn register_notifications(app: &AppHandle) {
    let Some(dir) = app.state::<Shared>().lock().ok().map(|s| s.data_dir.clone()) else { return };
    let icon = dir.join("notification-icon.png");
    if std::fs::metadata(&icon).map(|m| m.len() != NOTIFY_ICON.len() as u64).unwrap_or(true) {
        let _ = std::fs::write(&icon, NOTIFY_ICON);
    }
    let ok = ram_win::notify::register_app_id(&app.config().identifier, "RAM", icon.is_file().then_some(icon.as_path()));
    if let Ok(s) = app.state::<Shared>().lock() {
        s.log.debug(&format!("notifications: registered with Windows: {ok}"));
    }
}

/// Shows a notification on request, so the user can see that Windows lets
/// RAM's notifications through.
#[tauri::command(async)]
pub fn notify_test(app: AppHandle) -> Result<(), String> {
    show_notice(&app, "RAM notifications are on", "You will get one of these when a client is disconnected or crashes.")
}

/// One Windows notification, with the standard notification sound.
fn show_notice(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification().builder().title(title).body(body).sound("Default").show().map_err(|e| e.to_string())
}

/// "Tho********" when names are hidden.
fn shown_name(username: &str, hide: bool) -> String {
    if hide && !username.is_empty() {
        format!("{}********", username.chars().take(3).collect::<String>())
    } else {
        username.to_string()
    }
}

/// Title and text of the Windows notification for the clients that ended
/// in one watcher tick. One client is named with what happened; several are
/// counted, so a PC-wide failure is one notification and not thirty.
pub fn ended_notice(ended: &[(String, EndKind, i64)], rejoining: usize) -> Option<(String, String)> {
    let first = ended.first()?;
    let after = |secs: i64| if secs >= 3600 { format!("{} h {} min", secs / 3600, (secs % 3600) / 60) } else if secs >= 60 { format!("{} min", secs / 60) } else { format!("{secs} s") };
    let rejoin = match rejoining {
        0 => String::new(),
        n if n == ended.len() => " Rejoining.".to_string(),
        n => format!(" Rejoining {n}."),
    };
    if ended.len() == 1 {
        let (name, kind, uptime) = first;
        let what = match kind {
            EndKind::Disconnected => "was disconnected by Roblox",
            EndKind::Crashed => "crashed",
            EndKind::Replaced => "was replaced by another sign-in",
            EndKind::SignedOut => "had its sign-in refused",
            EndKind::Closed | EndKind::Teleported => return None,
        };
        let who = if name.is_empty() { "A client".to_string() } else { name.clone() };
        return Some((format!("{who} {what}"), format!("After {}.{rejoin}", after(*uptime))));
    }
    let count = |k: EndKind| ended.iter().filter(|(_, kind, _)| *kind == k).count();
    let mut parts = Vec::new();
    for (k, label) in [(EndKind::Crashed, "crashed"), (EndKind::Disconnected, "disconnected"), (EndKind::Replaced, "replaced by another sign-in"), (EndKind::SignedOut, "sign-in refused")] {
        let n = count(k);
        if n > 0 {
            parts.push(format!("{n} {label}"));
        }
    }
    let names: Vec<&str> = ended.iter().map(|(n, _, _)| n.as_str()).filter(|n| !n.is_empty()).take(4).collect();
    let more = if ended.len() > names.len() && !names.is_empty() { ", …" } else { "" };
    Some((format!("{} clients ended", ended.len()), format!("{}: {}{more}.{rejoin}", parts.join(", "), names.join(", "))))
}

fn tick(app: &AppHandle) {
    let st = app.state::<Shared>();
    let Ok(mut s) = st.lock() else { return };
    let t = now();
    let snapshot = match s.procs.list_clients() {
        Ok(v) => v,
        Err(_) => return,
    };
    let timeout = s.store.settings.launch_timeout_secs;
    let _ = s.clients.observe(t, &snapshot, timeout);
    s.diag.launches.retain(|l| t - l.at < 180);

    // Accounts whose launch line should clear: their client is now known
    // (Running) or gone (Exited).
    let mut seen_running: Vec<AccountId> = Vec::new();
    let mut seen_exited: Vec<AccountId> = Vec::new();

    // New clients.
    let logs = logs_dir();
    let mut new_watch = Vec::new();
    for p in &snapshot {
        if s.diag.watched.contains_key(&p.key) {
            continue;
        }
        let started = filetime_to_unix(p.key.start_time);
        let account = s.clients.link_for_process(p.key).map(|l| l.account);
        new_watch.push((p.key, started, account));
    }
    for (key, started, account) in new_watch {
        let taken: Vec<PathBuf> = s.diag.watched.values().filter_map(|w| w.tail.as_ref().map(|t| t.path.clone())).collect();
        let trackers: Vec<String> = s.diag.launches.iter().map(|l| l.browser_tracker_id.clone()).collect();
        let mut tail = None;
        let mut btid = None;
        let mut account = account;
        if let Some(dir) = &logs {
            if let Some((path, tracker)) = find_log(dir, started, 6, &taken, &trackers) {
                if let Some(tr) = &tracker {
                    if let Some(pos) = s.diag.launches.iter().position(|l| &l.browser_tracker_id == tr) {
                        let l = s.diag.launches.remove(pos);
                        account = Some(l.account);
                        s.clients.assign(key, l.account);
                    }
                }
                btid = tracker;
                tail = Some(Tail::new(path));
            }
        }
        let dumps = count_dumps(started);
        s.log.debug(&format!(
            "client {} started at {started}: log {}, account {:?}",
            key.pid,
            tail.as_ref().map(|t| t.path.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_else(|| "not found yet".into()),
            account
        ));
        s.diag.watched.insert(key, Watch { account, started, tail, facts: LogFacts::default(), browser_tracker_id: btid, dumps_at_start: dumps, closed_by_ram: false, exit: ram_win::process::ExitWatch::open(key), limits: Applied::default() });
        if let Some(id) = account {
            seen_running.push(id);
        }
    }

    // Tail logs; bind late logs; learn accounts from the join line.
    let alive: Vec<ProcKey> = snapshot.iter().map(|p| p.key).collect();
    let keys: Vec<ProcKey> = s.diag.watched.keys().copied().collect();
    let mut assign = Vec::new();
    let mut reads = Vec::new();
    for key in &keys {
        let taken: Vec<PathBuf> = s.diag.watched.values().filter_map(|w| w.tail.as_ref().map(|t| t.path.clone())).collect();
        let trackers: Vec<String> = s.diag.launches.iter().map(|l| l.browser_tracker_id.clone()).collect();
        let by_user: Vec<(u64, AccountId)> = s.store.accounts.iter().filter_map(|a| a.roblox_user_id.map(|u| (u, a.id))).collect();
        let closing = s.diag.closing.clone();
        let Some(w) = s.diag.watched.get_mut(key) else { continue };
        if closing.contains(&key.pid) {
            w.closed_by_ram = true;
        }
        if w.tail.is_none() && alive.contains(key) {
            if let Some(dir) = &logs {
                if let Some((path, tracker)) = find_log(dir, w.started, 10, &taken, &trackers) {
                    w.browser_tracker_id = tracker;
                    w.tail = Some(Tail::new(path));
                }
            }
        }
        if let Some(tail) = &mut w.tail {
            let _ = tail.read_new(&mut w.facts);
            if w.facts.read_settings {
                if let Some(b) = &w.browser_tracker_id {
                    reads.push(b.clone());
                }
            }
            if w.browser_tracker_id.is_none() {
                if let Some(b) = w.facts.browser_tracker_id {
                    let b = b.to_string();
                    if trackers.contains(&b) {
                        w.browser_tracker_id = Some(b);
                    }
                }
            }
            if w.account.is_none() {
                if let Some(uid) = w.facts.user_id {
                    if let Some((_, id)) = by_user.iter().find(|(u, _)| *u == uid) {
                        w.account = Some(*id);
                        assign.push((*key, *id));
                    }
                }
            }
        }
    }
    for (key, id) in assign {
        s.clients.assign(key, id);
        seen_running.push(id);
        s.log.info(&format!("client {} belongs to account {id} (from its log)", key.pid));
    }
    for b in reads {
        if let Some(pos) = s.diag.launches.iter().position(|l| l.browser_tracker_id == b) {
            let l = s.diag.launches.remove(pos);
            s.diag.reads.insert(b.clone(), true);
            let _ = l;
        } else {
            s.diag.reads.insert(b, true);
        }
    }
    // Pending launches whose tracker just showed up in a watched log.
    let pending: Vec<(AccountId, String)> = s.diag.launches.iter().map(|l| (l.account, l.browser_tracker_id.clone())).collect();
    let mut late = Vec::new();
    for key in &keys {
        if let Some(w) = s.diag.watched.get(key) {
            if w.account.is_none() {
                if let Some(b) = &w.browser_tracker_id {
                    if let Some((acc, _)) = pending.iter().find(|(_, t)| t == b) {
                        late.push((*key, *acc, b.clone()));
                    }
                }
            }
        }
    }
    for (key, acc, b) in late {
        if let Some(w) = s.diag.watched.get_mut(&key) {
            w.account = Some(acc);
        }
        s.diag.launches.retain(|l| l.browser_tracker_id != b);
        s.clients.assign(key, acc);
        seen_running.push(acc);
    }

    // ---- limits for alt clients (never Main, Full quality or unknown) ------
    let limits = s.store.limits.clone();
    let foreground = ram_win::client_ctl::foreground_pid();
    let mut acts: Vec<Act> = Vec::new();
    for key in &keys {
        if !alive.contains(key) {
            continue;
        }
        let Some(account) = s.diag.watched.get(key).and_then(|w| w.account) else { continue };
        let exempt = s.store.is_main(account) || s.store.account(account).map(|a| a.full_quality).unwrap_or(true);
        let in_game = s.diag.watched.get(key).map(|w| w.facts.in_game).unwrap_or(false);
        let background = foreground != Some(key.pid);
        let want_cap = !exempt && background && limits.cpu_limit_percent > 0;
        let want_eff = !exempt && background && limits.efficiency_mode;
        let (was_cap, was_eff, on_join, last_trim) = {
            let w = &s.diag.watched[key];
            (w.limits.cpu_capped, w.limits.efficiency, w.limits.on_join, w.limits.last_trim)
        };
        if want_cap != was_cap || (want_cap && !s.diag.jobs.has_cpu_cap(*key)) {
            let ok = if want_cap { s.diag.jobs.apply_cpu_cap(*key, limits.cpu_limit_percent.clamp(1, 100)).is_ok() } else { s.diag.jobs.clear_cpu_cap(*key).is_ok() };
            if ok {
                if let Some(w) = s.diag.watched.get_mut(key) {
                    w.limits.cpu_capped = want_cap;
                }
            }
        }
        if want_eff != was_eff && s.procs.set_efficiency_mode(*key, want_eff).is_ok() {
            if let Some(w) = s.diag.watched.get_mut(key) {
                w.limits.efficiency = want_eff;
            }
        }
        if !exempt && in_game && !on_join && (limits.tiny_on_launch || limits.minimize_on_launch || limits.mute_on_launch) {
            acts.push(Act::OnJoin { pid: key.pid, tiny: limits.tiny_on_launch.then(|| limits.tiny_size()), minimize: limits.minimize_on_launch, mute: limits.mute_on_launch });
            if let Some(w) = s.diag.watched.get_mut(key) {
                w.limits.on_join = true;
            }
            s.log.info(&format!("limits: client {} joined a game; tiny {}, minimize {}, mute {}", key.pid, limits.tiny_on_launch, limits.minimize_on_launch, limits.mute_on_launch));
        }
        if !exempt && limits.trim_every_min > 0 && in_game && t - last_trim >= i64::from(limits.trim_every_min) * 60 {
            if last_trim != 0 {
                acts.push(Act::Trim { pid: key.pid });
            }
            if let Some(w) = s.diag.watched.get_mut(key) {
                w.limits.last_trim = t;
            }
        }
    }
    perform(acts);

    // ---- auto-rejoin: clients that never got back in --------------------------
    // (account, how it ended, how long it ran) for every ending this tick.
    let mut ended: Vec<(AccountId, EndKind, i64)> = Vec::new();
    let mut joined: Vec<AccountId> = Vec::new();
    let mut stuck: Vec<(ProcKey, AccountId)> = Vec::new();
    for key in &keys {
        if !alive.contains(key) {
            continue;
        }
        let Some(w) = s.diag.watched.get(key) else { continue };
        let Some(acc) = w.account else { continue };
        let Some(since) = s.diag.rejoin.get(&acc).and_then(|r| r.awaiting_since) else { continue };
        if w.started + 5 < since {
            continue; // an older client of the same account
        }
        if w.facts.in_game {
            joined.push(acc);
        } else if t - w.started > rejoin::JOIN_TIMEOUT_SECS {
            stuck.push((*key, acc));
        }
    }
    for acc in joined {
        if let Some(r) = s.diag.rejoin.get_mut(&acc) {
            r.awaiting_since = None;
        }
    }
    // A rejoined client still not in a game (the server is gone, the
    // followed user left) is closed, and the next attempt falls back.
    for (key, acc) in stuck {
        if let Some(r) = s.diag.rejoin.get_mut(&acc) {
            r.awaiting_since = None;
        }
        s.diag.closing.push(key.pid);
        let _ = s.procs.request_close(key);
        s.log.info(&format!("rejoin: client {} did not get into a game in {} s; closing it", key.pid, rejoin::JOIN_TIMEOUT_SECS));
        ended.push((acc, EndKind::Disconnected, 0));
    }

    // Gone clients: classify and record.
    let mut ended_named: Vec<(String, EndKind, i64)> = Vec::new();
    let gone: Vec<ProcKey> = keys.iter().copied().filter(|k| !alive.contains(k)).collect();
    let mut recorded = 0;
    for key in gone {
        let Some(mut w) = s.diag.watched.remove(&key) else { continue };
        if let Some(tail) = &mut w.tail {
            let _ = tail.read_new(&mut w.facts);
        }
        if let Some(id) = w.account {
            seen_exited.push(id);
            if let Some(a) = s.store.account_mut(id) {
                a.last_online = Some(t);
            }
            s.robux_checked.remove(&id);
        }
        s.diag.closing.retain(|p| *p != key.pid);
        if let Some(b) = &w.browser_tracker_id {
            s.diag.reads.insert(b.clone(), true);
        }
        let dump = count_dumps(w.started) > w.dumps_at_start;
        let exit_code = w.exit.as_ref().and_then(|e| e.exit_code());
        let kind = clientlog::classify(&w.facts, true, dump, w.closed_by_ram, exit_code);
        let username = w.account.and_then(|id| s.store.account(id)).map(|a| a.username.clone()).unwrap_or_default();
        let uptime = (t - w.started).max(0);
        s.log.info(&format!(
            "client {} ({}) ended after {uptime}s: {:?} (exit code {}){}",
            key.pid,
            if username.is_empty() { "unknown account" } else { &username },
            kind,
            exit_code.map(|c| format!("{c:#x}")).unwrap_or_else(|| "unknown".into()),
            w.facts.reason.as_deref().map(|r| format!(" — {r}")).unwrap_or_default()
        ));
        let Some(kind) = kind else { continue };
        if let Some(id) = w.account {
            ended.push((id, kind, uptime));
        }
        if matches!(kind, EndKind::Closed | EndKind::Teleported) {
            continue;
        }
        // The history is about the accounts in RAM. A client RAM cannot tie
        // to one of them (started outside RAM, or an account that is not in
        // the list) is logged above but not recorded.
        if w.account.is_none() {
            continue;
        }
        // A notification is drawn by Windows, outside RAM's window, so a
        // stream would show it: streamer mode masks the name too.
        let mask = s.store.settings.hide_usernames || s.store.settings.streamer_mode;
        ended_named.push((shown_name(&username, mask), kind, uptime));
        s.store.push_crash(CrashRecord {
            id: 0,
            account: w.account,
            username,
            kind,
            at: t,
            uptime_secs: uptime,
            reason: w.facts.reason.clone().unwrap_or_default(),
            place_id: w.facts.place_id,
            log_path: w.tail.as_ref().map(|t| t.path.display().to_string()),
            pid: key.pid,
        });
        recorded += 1;
    }
    if recorded > 0 {
        s.save();
    }

    // ---- auto-rejoin: schedule, and launch what is due -------------------------
    let mut notes: Vec<(AccountId, &'static str, String)> = Vec::new();
    for (id, kind, uptime) in ended {
        if let Some(note) = schedule_rejoin(&mut s, id, kind, uptime, t) {
            notes.push(note);
        }
    }
    let mut rejoins: Vec<(AccountId, LaunchTarget, Target)> = Vec::new();
    let due: Vec<AccountId> = s.diag.rejoin.iter().filter(|(_, r)| r.due.is_some_and(|d| d <= t)).map(|(id, _)| *id).collect();
    for id in due {
        let Some(r) = s.diag.rejoin.get(&id).cloned() else { continue };
        let enabled = s.store.settings.auto_rejoin && s.store.account(id).is_some() && !s.store.is_main(id);
        let Some(last) = last_for(&s, id).filter(|_| enabled) else {
            s.diag.rejoin.remove(&id);
            continue;
        };
        // The client that ended may still be closing; give it a minute.
        let running = s.diag.watched.values().any(|w| w.account == Some(id));
        if running && t < r.due.unwrap_or(t) + 60 {
            continue;
        }
        let (target, model) = last.for_attempt(r.fallback);
        if let Some(st) = s.diag.rejoin.get_mut(&id) {
            st.due = None;
            st.awaiting_since = Some(t);
        }
        s.log.info(&format!("rejoin: launching account {id}, attempt {}{}", r.attempts, if r.fallback { " (fallback target)" } else { "" }));
        rejoins.push((id, target, model));
    }
    let notify = s.store.settings.notify_ended;
    let rejoining = notes.iter().filter(|(_, state, _)| *state == "Launching").count();
    drop(s);
    if recorded > 0 {
        let _ = app.emit("crashes-changed", recorded);
    }
    if notify {
        if let Some((title, body)) = ended_notice(&ended_named, rejoining) {
            let _ = show_notice(app, &title, &body);
        }
    }
    for id in seen_running {
        let _ = app.emit("launch-status", crate::dto::LaunchEvent { account: id, state: "Running".into(), message: String::new() });
        let _ = app.emit("account-updated", id);
    }
    for id in seen_exited {
        let _ = app.emit("launch-status", crate::dto::LaunchEvent { account: id, state: "Exited".into(), message: String::new() });
        let _ = app.emit("account-updated", id);
    }
    // After "Exited", so the rejoin line is what stays on the row.
    for (id, state, message) in notes {
        let _ = app.emit("launch-status", crate::dto::LaunchEvent { account: id, state: state.into(), message });
    }
    for (id, target, model) in rejoins {
        crate::cmd_launch::rejoin_launch(app.clone(), id, target, model);
    }
}

/// How `id` last joined: what was recorded this session, else (after RAM
/// was restarted while clients kept running) the place of its most recent
/// launch from the saved history.
fn last_for(s: &crate::state::AppState, id: AccountId) -> Option<LastLaunch> {
    if let Some(l) = s.diag.last_launch.get(&id) {
        return Some(l.clone());
    }
    let recent = s.store.recent.iter().filter(|r| r.account == id).max_by_key(|r| r.at)?;
    let target = ram_core::roblox::launch_target(&recent.target)?;
    Some(LastLaunch { target, model_target: recent.target.clone(), fallback_place: None })
}

/// Decides whether the account whose client just ended is launched again
/// and records when. Returns the line to show on its row.
fn schedule_rejoin(s: &mut crate::state::AppState, id: AccountId, kind: EndKind, uptime: i64, t: i64) -> Option<(AccountId, &'static str, String)> {
    if !s.store.settings.auto_rejoin || s.store.is_main(id) || s.store.account(id).is_none() || last_for(s, id).is_none() {
        return None;
    }
    let max = s.store.settings.rejoin_max.clamp(1, 20);
    let so_far = s.diag.rejoin.get(&id).map(|r| r.attempts).unwrap_or(0);
    match rejoin::plan(kind, uptime, so_far, max) {
        Some(p) => {
            s.diag.rejoin.insert(id, RejoinState { attempts: p.attempt, due: Some(t + p.delay_secs as i64), fallback: p.fallback, awaiting_since: None });
            s.log.info(&format!("rejoin: account {id} ended ({kind:?}); attempt {} of {max} in {} s", p.attempt, p.delay_secs));
            Some((id, "Launching", format!("{}. Rejoining in {} s (attempt {} of {max})…", clientlog::kind_label(kind), p.delay_secs, p.attempt)))
        }
        None if rejoin::gave_up(kind, uptime, so_far, max) => {
            s.log.warn(&format!("rejoin: account {id} ended again ({kind:?}); giving up after {max} attempts"));
            Some((id, "Failed", format!("Stopped rejoining after {max} attempts. Launch it again to start over.")))
        }
        None => None,
    }
}

/// Minimizes (or restores) every running Roblox client. Returns how many
/// windows were asked.
#[tauri::command(async)]
pub fn clients_minimize_all(state: State<'_, Shared>, minimize: bool) -> Result<u32, String> {
    let pids: Vec<u32> = {
        let s = lock(&state)?;
        s.procs.list_clients().map_err(|e| e.to_string())?.into_iter().map(|p| p.key.pid).collect()
    };
    let mut n = 0;
    for pid in pids {
        let r = if minimize { ram_win::client_ctl::minimize(pid) } else { ram_win::client_ctl::restore(pid) };
        if matches!(r, Ok(true)) {
            n += 1;
        }
    }
    Ok(n)
}

/// Applies (kbps > 0) or removes (0) the bandwidth limit for Roblox
/// clients. This is a Windows QoS policy, so Windows shows an
/// administrator prompt; the policy lasts until it is removed here or the
/// PC restarts.
#[tauri::command(async)]
pub fn apply_bandwidth(state: State<'_, Shared>, kbps: u32) -> Result<ram_core::model::LimitSettings, String> {
    let kbps = if kbps == 0 { 0 } else { kbps.clamp(64, 1_000_000) };
    ram_win::client_ctl::apply_bandwidth_limit((kbps > 0).then_some(kbps))?;
    let mut s = lock(&state)?;
    s.store.limits.bandwidth_kbps = kbps;
    s.log.info(&format!("limits: bandwidth limit {}", if kbps > 0 { format!("set to {kbps} kbit/s") } else { "removed".into() }));
    s.save();
    Ok(s.store.limits.clone())
}

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

#[tauri::command(async)]
pub fn crashes_list(state: State<'_, Shared>) -> Result<Vec<CrashView>, String> {
    let s = lock(&state)?;
    Ok(s.store
        .crashes
        .iter()
        .map(|c| CrashView {
            id: c.id,
            account: c.account,
            username: c.username.clone(),
            kind: c.kind,
            at: c.at,
            uptime_secs: c.uptime_secs,
            reason: c.reason.clone(),
            place_id: c.place_id,
            place_name: c.place_id.map(|p| s.store.games.iter().find(|g| g.place_id == p).map(|g| g.name.clone()).unwrap_or_default()).unwrap_or_default(),
            has_log: c.log_path.as_deref().map(|p| Path::new(p).is_file()).unwrap_or(false),
            pid: c.pid,
        })
        .collect())
}

#[tauri::command(async)]
pub fn crashes_clear(state: State<'_, Shared>) -> Result<(), String> {
    let mut s = lock(&state)?;
    s.store.crashes.clear();
    s.save();
    Ok(())
}

#[tauri::command(async)]
pub fn crash_open_log(app: AppHandle, state: State<'_, Shared>, id: u64) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = {
        let s = lock(&state)?;
        s.store.crashes.iter().find(|c| c.id == id).and_then(|c| c.log_path.clone()).ok_or("No log is recorded for that entry.")?
    };
    if !Path::new(&path).is_file() {
        return Err("That client log is gone; Roblox deletes old logs.".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn crashes_export(state: State<'_, Shared>, path: String) -> Result<(), String> {
    let rows: Vec<Vec<String>> = {
        let s = lock(&state)?;
        s.store
            .crashes
            .iter()
            .map(|c| {
                vec![
                    ram_core::redact::stamp(c.at).replace("][", " ").trim_matches(['[', ']']).to_string(),
                    c.username.clone(),
                    clientlog::kind_label(c.kind).to_string(),
                    c.uptime_secs.to_string(),
                    c.place_id.map(|p| p.to_string()).unwrap_or_default(),
                    c.reason.clone(),
                ]
            })
            .collect()
    };
    let text = clientlog::csv_document(&["When (UTC)", "Account", "Kind", "Seconds", "Place ID", "Reason"], &rows);
    std::fs::write(&path, text.as_bytes()).map_err(|e| format!("could not write the file: {e}"))
}

#[tauri::command(async)]
pub fn log_tail(state: State<'_, Shared>, lines: usize) -> Result<Vec<LogLineView>, String> {
    let s = lock(&state)?;
    Ok(s.log
        .tail(lines.min(10_000))
        .into_iter()
        .map(|text| LogLineView {
            level: match Level::of_line(&text) {
                Some(Level::Error) => "error",
                Some(Level::Warn) => "warn",
                Some(Level::Debug) => "debug",
                _ => "info",
            },
            text,
        })
        .collect())
}

#[tauri::command(async)]
pub fn open_log_folder(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = {
        let s = lock(&state)?;
        s.data_dir.display().to_string()
    };
    app.opener().open_path(dir, None::<&str>).map_err(|e| e.to_string())
}

/// A shareable report: version, environment, settings, the last log lines
/// and the tail of the newest client logs, with secrets, user names and
/// paths masked.
#[tauri::command(async)]
pub fn diagnostics_text(state: State<'_, Shared>) -> Result<String, String> {
    let s = lock(&state)?;
    let launcher = ram_win::launcher_info::detect();
    let mut out = String::new();
    out.push_str(&format!("RAM {} diagnostics\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("Launcher: {}{}\n", launcher.name, if launcher.bootstrapper { " (bootstrapper)" } else { "" }));
    out.push_str(&format!("Browser: {}\n", crate::cmd_browser::pick_browser().map(|b| b.name).unwrap_or_else(|| "none usable".into())));
    out.push_str(&format!("Multi-instance held: {}\n", s.singleton.as_ref().map(|h| h.is_held()).unwrap_or(false)));
    out.push_str(&format!("Accounts: {}, cookies: {}, vault: {:?}\n", s.store.accounts.len(), s.vault.accounts_with_secret().len(), s.vault.mode()));
    out.push_str(&format!("Graphics: {}\n", serde_json::to_string(&s.store.graphics).unwrap_or_default()));
    let names: Vec<String> = s.store.accounts.iter().flat_map(|a| [a.username.clone(), a.alias.clone()]).filter(|n| n.len() >= 3).collect();
    let mask = |line: &str| -> String {
        let mut l = redact(line);
        for n in &names {
            l = l.replace(n, "…");
        }
        // Paths under the profile and user ids.
        if let Some(i) = l.find(r"C:\Users\") {
            let rest = &l[i + 9..];
            if let Some(j) = rest.find('\\') {
                l = format!("{}C:\\Users\\…{}", &l[..i], &rest[j..]);
            }
        }
        l
    };
    out.push_str("\n--- RAM.log (last 400 lines) ---\n");
    for line in s.log.tail(400) {
        out.push_str(&mask(&line));
        out.push('\n');
    }
    if let Some(dir) = logs_dir() {
        let mut logs: Vec<(i64, PathBuf)> = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                clientlog::is_player_log_name(&name).then(|| (clientlog::log_start_time(&name).unwrap_or(0), e.path()))
            })
            .collect();
        logs.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        for (_, p) in logs.into_iter().take(3) {
            out.push_str(&format!("\n--- {} (last 120 lines) ---\n", p.file_name().unwrap_or_default().to_string_lossy()));
            if let Ok(text) = std::fs::read_to_string(&p) {
                let lines: Vec<&str> = text.lines().collect();
                for l in lines.iter().rev().take(120).rev() {
                    if clientlog::is_sensitive(l) {
                        continue;
                    }
                    out.push_str(&mask(l));
                    out.push('\n');
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod rejoin_tests {
    use super::*;

    #[test]
    fn one_ending_is_named_and_many_are_counted() {
        let one = vec![("Thornwick_Eli".to_string(), EndKind::Disconnected, 2040)];
        assert_eq!(ended_notice(&one, 1), Some(("Thornwick_Eli was disconnected by Roblox".into(), "After 34 min. Rejoining.".into())));
        assert_eq!(ended_notice(&[("".into(), EndKind::Crashed, 45)], 0), Some(("A client crashed".into(), "After 45 s.".into())));
        assert_eq!(ended_notice(&[("a".into(), EndKind::Replaced, 7300)], 0).unwrap().1, "After 2 h 1 min.");
        let many: Vec<(String, EndKind, i64)> = (0..6).map(|i| (format!("alt{i}"), if i < 4 { EndKind::Crashed } else { EndKind::Disconnected }, 100)).collect();
        assert_eq!(ended_notice(&many, 2), Some(("6 clients ended".into(), "4 crashed, 2 disconnected: alt0, alt1, alt2, alt3, …. Rejoining 2.".into())));
        assert_eq!(ended_notice(&[], 0), None);
        assert_eq!(shown_name("Thornwick_Eli", true), "Tho********");
        assert_eq!(shown_name("Thornwick_Eli", false), "Thornwick_Eli");
    }

    fn last(target: LaunchTarget, fallback_place: Option<u64>) -> LastLaunch {
        LastLaunch { model_target: Target::Place { place_id: 0 }, target, fallback_place }
    }

    #[test]
    fn first_attempt_repeats_the_launch_and_later_ones_fall_back() {
        let server = last(LaunchTarget::Server { place_id: 7, job_id: "j".into() }, None);
        assert_eq!(server.for_attempt(false).0, LaunchTarget::Server { place_id: 7, job_id: "j".into() });
        assert_eq!(server.for_attempt(true), (LaunchTarget::Place { place_id: 7 }, Target::Place { place_id: 7 }));
        assert_eq!(server.place(), Some(7));

        let follow = last(LaunchTarget::Follow { user_id: 99 }, Some(7));
        assert_eq!(follow.for_attempt(false).0, LaunchTarget::Follow { user_id: 99 });
        assert_eq!(follow.for_attempt(true).0, LaunchTarget::Place { place_id: 7 });
        assert_eq!(follow.place(), Some(7));

        // Nothing to fall back to: keep following.
        let blind = last(LaunchTarget::Follow { user_id: 99 }, None);
        assert_eq!(blind.for_attempt(true).0, LaunchTarget::Follow { user_id: 99 });
        assert_eq!(blind.place(), None);

        let place = last(LaunchTarget::Place { place_id: 7 }, None);
        assert_eq!(place.for_attempt(true).0, LaunchTarget::Place { place_id: 7 });
    }
}
