//! Login checks and launching (Place ID / Job ID / Follow).
//!
//! Network work runs on worker threads; results are applied to the state and
//! announced with events so the front-end re-reads what changed.

use crate::dto::LaunchEvent;
use crate::state::{now, Shared};
use ram_core::client_settings;
use ram_core::model::{AccountId, RecentLaunch, Target};
use ram_core::roblox::{self, LaunchError, LaunchTarget};
use ram_core::roblox_profile;
use ram_core::signin::{self, CheckOutcome};
use ram_win::WinHttp;
use tauri::{AppHandle, Emitter, Manager, State};

fn lock<'a>(state: &'a State<'_, Shared>) -> Result<std::sync::MutexGuard<'a, crate::state::AppState>, String> {
    state.lock().map_err(|_| "application state is unavailable".to_string())
}

fn emit_account(app: &AppHandle, id: AccountId) {
    let _ = app.emit("account-updated", id);
}

fn emit_launch(app: &AppHandle, account: AccountId, state: &str, message: impl Into<String>) {
    let _ = app.emit("launch-status", LaunchEvent { account, state: state.to_string(), message: message.into() });
}

fn describe(e: &LaunchError) -> String {
    match e {
        LaunchError::Unauthorized => "the stored cookie is no longer valid".into(),
        LaunchError::BoundSessionRequired => "Roblox requires a fresh sign-in for this account (bound session)".into(),
        LaunchError::Challenge => "Roblox asked for a verification step; open the browser for this account".into(),
        LaunchError::Offline => "this PC appears to be offline".into(),
        LaunchError::Timeout => "the request to Roblox timed out".into(),
        LaunchError::RateLimited { retry_after_secs: Some(s) } => format!("Roblox rate limit; retry in {s} s"),
        LaunchError::RateLimited { retry_after_secs: None } => "Roblox rate limit".into(),
        LaunchError::Service { status } => format!("Roblox returned HTTP {status}"),
        LaunchError::Malformed => "Roblox returned an unexpected response".into(),
    }
}

/// Runs the RoALT-style login check for one account on a worker thread:
/// `GET users/authenticated` with the cookie decides validity; Robux and
/// Premium are fetched alongside when it succeeds.
pub fn start_check(app: AppHandle, id: AccountId) {
    let cookie = {
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        if s.checking.contains(&id) {
            return;
        }
        let Some(c) = s.cookie(id) else {
            let configured = s.vault.is_configured();
            if let Some(a) = s.store.account_mut(id) {
                signin::apply(&mut a.sign_in, &CheckOutcome::NotAuthorized, now());
                a.sign_in.detail = if configured {
                    "No cookie stored. Add one, or sign in through Open Browser.".into()
                } else {
                    "Set up the vault and add a cookie to enable checks.".into()
                };
            }
            s.save();
            emit_account(&app, id);
            return;
        };
        s.checking.insert(id);
        c
    };
    emit_account(&app, id);
    std::thread::spawn(move || {
        let http = WinHttp::new();
        let result = roblox::whoami_full(&http, &cookie);
        let extras = result.as_ref().ok().map(|w| {
            (roblox_profile::robux(&http, &cookie, w.user.id).ok(), roblox_profile::premium(&http, &cookie, w.user.id).ok())
        });
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        s.checking.remove(&id);
        let t = now();
        // Roblox may rotate the cookie on a check; keep the vault current.
        if let Ok(w) = &result {
            if let Some(rotated) = &w.rotated_cookie {
                if s.vault.is_unlocked() {
                    let _ = s.vault.set_secret(id, rotated, &crate::svc::WinEntropy, t);
                }
            }
        }
        match &result {
            Ok(w) => s.log.info(&format!("check {id}: valid (user {})", w.user.id)),
            Err(e) => s.log.warn(&format!("check {id}: {e:?}")),
        }
        if let Some(a) = s.store.account_mut(id) {
            match result.map(|w| w.user) {
                Ok(user) => {
                    let outcome = match a.roblox_user_id {
                        Some(expected) if a.user_id_confirmed && expected != user.id => {
                            CheckOutcome::DifferentAccount { username: user.name.clone() }
                        }
                        _ => CheckOutcome::Confirmed,
                    };
                    signin::apply(&mut a.sign_in, &outcome, t);
                    if outcome == CheckOutcome::Confirmed {
                        a.roblox_user_id = Some(user.id);
                        a.user_id_confirmed = true;
                        a.display_name = user.display_name;
                        if a.username.is_empty() || !a.username.eq_ignore_ascii_case(&user.name) {
                            a.username = user.name;
                        }
                        if let Some((robux, premium)) = extras {
                            if robux.is_some() {
                                a.robux = robux;
                            }
                            if premium.is_some() {
                                a.premium = premium;
                            }
                        }
                    }
                }
                Err(e) => signin::apply(&mut a.sign_in, &e.to_sign_in_outcome(), t),
            }
        }
        s.save();
        drop(s);
        emit_account(&app, id);
    });
}

#[tauri::command(async)]
pub fn check_accounts(app: AppHandle, ids: Vec<AccountId>) -> Result<(), String> {
    for id in ids {
        start_check(app.clone(), id);
    }
    Ok(())
}

/// Seconds a headshot URL is kept before it is looked up again.
const AVATAR_TTL_SECS: i64 = 24 * 3600;

/// Fetches headshots for every account with a known user id whose image is
/// missing or older than a day (`force` refreshes all). Public API, no
/// cookie; runs on a worker thread and announces each updated account.
#[tauri::command(async)]
pub fn refresh_avatars(app: AppHandle, force: bool) -> Result<(), String> {
    let wanted: Vec<(AccountId, u64)> = {
        let st = app.state::<Shared>();
        let s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        let t = now();
        s.store
            .accounts
            .iter()
            .filter_map(|a| {
                let uid = a.roblox_user_id?;
                let stale = a.avatar_url.is_none() || a.avatar_at.map(|at| t - at > AVATAR_TTL_SECS).unwrap_or(true);
                (force || stale).then_some((a.id, uid))
            })
            .collect()
    };
    if wanted.is_empty() {
        return Ok(());
    }
    std::thread::spawn(move || {
        let http = WinHttp::new();
        let ids: Vec<u64> = wanted.iter().map(|(_, u)| *u).collect();
        let got = roblox_profile::headshots(&http, &ids);
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return };
        let t = now();
        let mut changed = Vec::new();
        for (uid, url) in got {
            for (id, u) in &wanted {
                if *u == uid {
                    if let Some(a) = s.store.account_mut(*id) {
                        a.avatar_url = Some(url.clone());
                        a.avatar_at = Some(t);
                        changed.push(*id);
                    }
                }
            }
        }
        if !changed.is_empty() {
            s.save();
        }
        drop(s);
        for id in changed {
            emit_account(&app, id);
        }
    });
    Ok(())
}

/// Name and creator of a place, from the store's cache or the public games
/// API. `Ok(None)` means Roblox does not know that place id.
#[tauri::command]
pub async fn place_info(app: AppHandle, place_id: u64) -> Result<Option<ram_core::model::Game>, String> {
    if place_id == 0 {
        return Ok(None);
    }
    {
        let st = app.state::<Shared>();
        let s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
        if let Some(g) = s.store.games.iter().find(|g| g.place_id == place_id) {
            if g.builtin || g.fetched_at.map(|at| now() - at < 7 * 24 * 3600).unwrap_or(false) {
                return Ok(Some(g.clone()));
            }
        }
    }
    let result = tauri::async_runtime::spawn_blocking(move || roblox_profile::place_info(&WinHttp::new(), place_id))
        .await
        .map_err(|e| e.to_string())?;
    match result {
        Ok(info) => {
            let st = app.state::<Shared>();
            let mut s = st.lock().map_err(|_| "application state is unavailable".to_string())?;
            let t = now();
            let game = match s.store.games.iter_mut().find(|g| g.place_id == place_id) {
                Some(g) => {
                    g.name = info.name;
                    g.creator = info.creator;
                    g.universe_id = Some(info.universe_id);
                    g.fetched_at = Some(t);
                    g.clone()
                }
                None => {
                    let g = ram_core::model::Game {
                        place_id,
                        universe_id: Some(info.universe_id),
                        name: info.name,
                        creator: info.creator,
                        fetched_at: Some(t),
                        ..Default::default()
                    };
                    s.store.games.push(g.clone());
                    // Keep the cache bounded; built-ins and pinned entries stay.
                    let keep: Vec<ram_core::model::Game> = s.store.games.iter().filter(|g| g.builtin || g.pinned).cloned().collect();
                    let mut rest: Vec<ram_core::model::Game> = s.store.games.iter().filter(|g| !(g.builtin || g.pinned)).cloned().collect();
                    if rest.len() > 50 {
                        rest.drain(0..rest.len() - 50);
                    }
                    s.store.games = keep.into_iter().chain(rest).collect();
                    g
                }
            };
            s.save();
            Ok(Some(game))
        }
        Err(LaunchError::Malformed) => Ok(None),
        Err(e) => Err(describe(&e)),
    }
}

/// Queues launches for several accounts to a place (optionally a specific
/// server), spaced by the configured delay. Accounts without a stored cookie
/// are reported and skipped.
#[tauri::command(async)]
pub fn launch(app: AppHandle, state: State<'_, Shared>, ids: Vec<AccountId>, place_id: u64, job_id: Option<String>) -> Result<(), String> {
    if place_id == 0 {
        return Err("Enter a place ID.".into());
    }
    let job = job_id.map(|j| j.trim().to_string()).filter(|j| !j.is_empty());
    if let Some(j) = &job {
        if !ram_core::links::is_job_id(j) {
            return Err("The job ID is not in Roblox's format (a GUID).".into());
        }
    }
    let target = match &job {
        Some(j) => LaunchTarget::Server { place_id, job_id: j.to_ascii_lowercase() },
        None => LaunchTarget::Place { place_id },
    };
    let model_target = match &job {
        Some(j) => Target::LiveServer { place_id, job_id: j.to_ascii_lowercase() },
        None => Target::Place { place_id },
    };
    let spacing = {
        let s = lock(&state)?;
        s.store.settings.launch_spacing_secs
    };
    run_launches(app, ids, target, model_target, spacing, true);
    Ok(())
}

/// Follows a user by username: resolves the id, then launches into their
/// server for each selected account.
#[tauri::command(async)]
pub fn follow(app: AppHandle, state: State<'_, Shared>, ids: Vec<AccountId>, username: String) -> Result<(), String> {
    let username = username.trim().to_string();
    if username.is_empty() {
        return Err("Enter a username to follow.".into());
    }
    let spacing = {
        let s = lock(&state)?;
        s.store.settings.launch_spacing_secs
    };
    std::thread::spawn(move || {
        let http = WinHttp::new();
        match roblox_profile::resolve_username(&http, &username) {
            Ok(user_id) => {
                let target = LaunchTarget::Follow { user_id };
                run_launches(app, ids, target, Target::Place { place_id: 0 }, spacing, true);
            }
            Err(e) => {
                for id in ids {
                    emit_launch(&app, id, "Failed", format!("Could not find {username}: {}", describe(&e)));
                }
            }
        }
    });
    Ok(())
}

/// Serializes the short part of a launch that writes Roblox's files and
/// starts the client. It is no longer held while a client loads: launches
/// that need the same settings go back to back.
static LAUNCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long after the last launch the alt settings stay in place when a
/// client never shows that it has read them (a very busy PC).
const READ_WAIT_SECS: u64 = 120;

/// The alt graphics settings currently written to Roblox's files.
///
/// Roblox's settings file and the FastFlag file are shared by every client,
/// and a client reads them once while it starts. They used to be written,
/// waited on and put back for every single launch, so each account waited
/// for the one before it to finish loading. Now they are written once for a
/// run of launches that want the same settings, and put back when the last
/// of those clients has read them. Only a launch that needs *different*
/// settings (another FPS cap, Full quality, or Main) waits for the earlier
/// clients first.
struct Session {
    eff: client_settings::Effective,
    xml_undo: Option<client_settings::XmlUndo>,
    flag_undo: Option<client_settings::FlagUndo>,
    /// Tracker ids of the clients started under these settings.
    pending: Vec<String>,
    last_launch: std::time::Instant,
}

static SESSION: std::sync::Mutex<Option<Session>> = std::sync::Mutex::new(None);
/// Whether the thread that puts the settings back is running.
static SETTLER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The settings a launch of `id` needs written, or `None` for the user's
/// own (performance off, or Main).
fn effective_for(app: &AppHandle, id: AccountId) -> Option<client_settings::Effective> {
    let st = app.state::<Shared>();
    let s = st.lock().ok()?;
    client_settings::effective(&s.store.graphics, &s.store.account_graphics(id), s.store.is_main(id))
}

/// Writes the FPS cap / quality (XML) and rendering flags (JSON). Returns
/// the undo handles and anything that could not be applied.
fn apply_effective(app: &AppHandle, id: AccountId, eff: &client_settings::Effective) -> (Option<client_settings::XmlUndo>, Option<client_settings::FlagUndo>, Vec<String>) {
    let st = app.state::<Shared>();
    let Ok(mut s) = st.lock() else { return (None, None, Vec::new()) };
    let mut notes = Vec::new();
    let mut xml_undo = None;
    if eff.fps_cap > 0 || eff.low_graphics || eff.mute {
        let path = ram_win::paths::local_app_data().map(|p| p.join("Roblox").join("GlobalBasicSettings_13.xml"));
        if let Some(path) = path {
            let file = client_settings::XmlFile::new(path);
            let (_, current) = file.read();
            let want = client_settings::xml_for_launch(eff, &current);
            match file.apply(&want) {
                Ok(u) => {
                    s.store.xml_undo = Some(u.clone());
                    s.save();
                    xml_undo = Some(u);
                }
                Err(e) => notes.push(format!("FPS cap / quality not applied: {e}")),
            }
        }
    }
    let mut flag_undo = None;
    let set = client_settings::flag_set(eff);
    if !set.is_empty() {
        let launcher = ram_win::launcher_info::detect();
        match launcher.flag_file {
            Some(path) => match client_settings::FlagFile::new(path).apply(&set) {
                Ok(u) => flag_undo = Some(u),
                Err(e) => notes.push(format!("rendering flags not applied: {e}")),
            },
            None => notes.push("rendering flags not applied: no Roblox client folder was found".into()),
        }
    }
    s.log.info(&format!(
        "launch {id}: graphics applied (fps {}, low {}, lighter {}, {} flags){}",
        eff.fps_cap,
        eff.low_graphics,
        eff.lighter.is_some(),
        set.len(),
        if notes.is_empty() { String::new() } else { format!("; {}", notes.join("; ")) }
    ));
    (xml_undo, flag_undo, notes)
}

/// Whether every client started under the session has read its settings
/// (the watcher marks a tracker id once its log shows it, or the client is
/// gone). A session nothing was started under counts as read.
fn session_read(app: &AppHandle, session: &Session) -> bool {
    let st = app.state::<Shared>();
    let Ok(s) = st.lock() else { return true };
    session.pending.iter().all(|b| s.diag.reads.get(b).copied().unwrap_or(false))
}

/// Puts the user's own values back.
fn restore_session(app: &AppHandle, session: Session) {
    let st = app.state::<Shared>();
    let Ok(mut s) = st.lock() else { return };
    if let Some(u) = session.flag_undo {
        if let Err(e) = u.put_back() {
            s.log.warn(&format!("flag restore: {e}"));
        }
    }
    if let Some(u) = session.xml_undo {
        if let Err(e) = u.put_back() {
            s.log.warn(&format!("settings restore: {e}"));
        }
        s.store.xml_undo = None;
        s.save();
    }
    s.log.info(&format!("launch: your own Roblox settings are back in place ({} client{} started with the alt settings)", session.pending.len(), if session.pending.len() == 1 { "" } else { "s" }));
}

/// Makes Roblox's files right for a launch of `id`. Returns whether the
/// launch runs under the shared session, and notes for the row.
///
/// Same settings as the session in place: nothing to do, the launch goes
/// straight ahead. Different ones: the clients started so far must read
/// theirs first, then the files are put back and rewritten.
fn prepare_graphics(app: &AppHandle, id: AccountId) -> (bool, Vec<String>) {
    let need = effective_for(app, id);
    let mut slot = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    if let (Some(s), Some(n)) = (slot.as_ref(), need.as_ref()) {
        if s.eff == *n {
            return (true, Vec::new());
        }
    }
    if let Some(old) = slot.take() {
        if !session_read(app, &old) {
            emit_launch(app, id, "Launching", "Waiting for the clients before it to load: this account uses different settings…");
        }
        let deadline = old.last_launch + std::time::Duration::from_secs(READ_WAIT_SECS);
        loop {
            if session_read(app, &old) {
                // Roblox reads the files a moment after the first mark.
                std::thread::sleep(std::time::Duration::from_secs(1));
                break;
            }
            if std::time::Instant::now() >= deadline {
                if let Ok(s) = app.state::<Shared>().lock() {
                    s.log.warn(&format!("launch: no settings-read mark within {READ_WAIT_SECS} s; restoring anyway"));
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        restore_session(app, old);
    }
    match need {
        Some(eff) => {
            let (xml_undo, flag_undo, notes) = apply_effective(app, id, &eff);
            *slot = Some(Session { eff, xml_undo, flag_undo, pending: Vec::new(), last_launch: std::time::Instant::now() });
            (true, notes)
        }
        None => (false, Vec::new()),
    }
}

/// Starts (once) the thread that puts the user's settings back when the
/// last client of the session has read them.
fn ensure_settler(app: AppHandle) {
    use std::sync::atomic::Ordering;
    if SETTLER.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || loop {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // Never between a launch writing the files and starting its client.
            let Ok(_serial) = LAUNCH_LOCK.try_lock() else { continue };
            let mut slot = SESSION.lock().unwrap_or_else(|p| p.into_inner());
            let Some(s) = slot.as_ref() else { break };
            let idle = s.last_launch.elapsed();
            let read = session_read(&app, s) && idle >= std::time::Duration::from_secs(2);
            if read || idle >= std::time::Duration::from_secs(READ_WAIT_SECS) {
                if !read {
                    if let Ok(st) = app.state::<Shared>().lock() {
                        st.log.warn(&format!("launch: no settings-read mark within {READ_WAIT_SECS} s; restoring anyway"));
                    }
                }
                if let Some(s) = slot.take() {
                    restore_session(&app, s);
                }
                break;
            }
        }
        SETTLER.store(false, Ordering::SeqCst);
        // A launch may have opened a new session while this one was closing.
        let open = SESSION.lock().map(|s| s.is_some()).unwrap_or(false);
        if !open || SETTLER.swap(true, Ordering::SeqCst) {
            break;
        }
    });
}

/// Orders a batch so accounts that want the same settings launch together:
/// first those matching the settings already in place, then those that need
/// the user's own (Main), then the rest grouped by what they need. Within a
/// group the user's order is kept.
fn batch_order(app: &AppHandle, ids: Vec<AccountId>) -> Vec<AccountId> {
    let needs: Vec<(AccountId, Option<client_settings::Effective>)> = ids.iter().map(|id| (*id, effective_for(app, *id))).collect();
    let active = SESSION.lock().ok().and_then(|s| s.as_ref().map(|s| s.eff.clone()));
    order_by_settings(needs, active.as_ref())
}

/// The pure part of [`batch_order`].
fn order_by_settings<E: PartialEq + Clone>(needs: Vec<(AccountId, Option<E>)>, active: Option<&E>) -> Vec<AccountId> {
    let mut groups: Vec<(Option<E>, Vec<AccountId>)> = Vec::new();
    for (id, need) in needs {
        match groups.iter_mut().find(|(g, _)| *g == need) {
            Some((_, list)) => list.push(id),
            None => groups.push((need, vec![id])),
        }
    }
    // Stable: 0 = what is in place now, 1 = the user's own settings, 2 = the rest.
    groups.sort_by_key(|(g, _)| match (g, active) {
        (Some(g), Some(a)) if g == a => 0,
        (None, _) => 1,
        _ => 2,
    });
    groups.into_iter().flat_map(|(_, list)| list).collect()
}

/// Launches one account again for auto-rejoin. The way it joined is not
/// re-recorded (a fallback target must not replace what the user chose) and
/// its attempt counter is kept.
pub fn rejoin_launch(app: AppHandle, id: AccountId, target: LaunchTarget, model_target: Target) {
    run_launches(app, vec![id], target, model_target, 0, false);
}

/// `remember`: this is a launch the user asked for, so it becomes the way
/// the account is rejoined and clears any rejoin in progress.
fn run_launches(app: AppHandle, ids: Vec<AccountId>, target: LaunchTarget, model_target: Target, spacing: u32, remember: bool) {
    std::thread::spawn(move || {
        let http = WinHttp::new();
        let mut first = true;
        let ids = batch_order(&app, ids);
        for id in ids {
            if !first && spacing > 0 {
                std::thread::sleep(std::time::Duration::from_secs(spacing as u64));
            }
            first = false;
            let cookie = {
                let st = app.state::<Shared>();
                let Ok(mut s) = st.lock() else { return };
                s.cookie(id)
            };
            let Some(cookie) = cookie else {
                emit_launch(&app, id, "Failed", "No cookie stored. Add one, or sign in through Open Browser.");
                continue;
            };
            let _serial = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            emit_launch(&app, id, "Launching", "Requesting a ticket…");
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            match roblox::prepare_launch(&http, &cookie, &target, None, now_ms) {
                Ok((uri, btid)) => {
                    let (in_session, notes) = prepare_graphics(&app, id);
                    for n in notes {
                        emit_launch(&app, id, "Launching", n);
                    }
                    {
                        let st = app.state::<Shared>();
                        let guard = st.lock();
                        if let Ok(mut s) = guard {
                            s.diag.launches.push(crate::cmd_diag::PendingLaunch { account: id, browser_tracker_id: btid.clone(), at: now() });
                            s.diag.reads.remove(&btid);
                        }
                    }
                    match ram_win::launch::launch_protocol(&uri) {
                        Ok(()) => {
                            let st = app.state::<Shared>();
                            if let Ok(mut s) = st.lock() {
                                let t = now();
                                if let Some(a) = s.store.account_mut(id) {
                                    a.last_used = Some(t);
                                    a.last_online = Some(t);
                                }
                                if model_target.place_id().unwrap_or(0) != 0 {
                                    s.store.push_recent(RecentLaunch { account: id, target: model_target.clone(), at: t });
                                }
                                if remember {
                                    // Following someone can stop working (they leave); keep the
                                    // place this account last went to as the way back in.
                                    let fallback_place = match &target {
                                        LaunchTarget::Follow { .. } => s
                                            .diag
                                            .last_launch
                                            .get(&id)
                                            .and_then(|l| l.place())
                                            .or_else(|| s.store.recent.iter().filter(|r| r.account == id).max_by_key(|r| r.at).and_then(|r| r.target.place_id()))
                                            .filter(|p| *p != 0),
                                        _ => None,
                                    };
                                    s.diag.last_launch.insert(id, crate::cmd_diag::LastLaunch { target: target.clone(), model_target: model_target.clone(), fallback_place });
                                    s.diag.rejoin.remove(&id);
                                }
                                s.log.info(&format!("launch {id}: client starting (tracker {btid})"));
                                s.save();
                            }
                            emit_launch(&app, id, "Launching", "Client starting…");
                            if in_session {
                                // The settings stay in place for the next launch; the
                                // settler puts the user's back once this client has read them.
                                if let Some(s) = SESSION.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                                    s.pending.push(btid.clone());
                                    s.last_launch = std::time::Instant::now();
                                }
                            }
                        }
                        Err(e) => {
                            if let Ok(s) = app.state::<Shared>().lock() {
                                s.log.error(&format!("launch {id}: could not start the client: {e}"));
                            }
                            emit_launch(&app, id, "Failed", format!("Could not start the Roblox client: {e}"));
                        }
                    }
                }
                Err(LaunchError::Unauthorized) => {
                    let st = app.state::<Shared>();
                    if let Ok(mut s) = st.lock() {
                        if let Some(a) = s.store.account_mut(id) {
                            signin::apply(&mut a.sign_in, &CheckOutcome::AuthorizationRejected, now());
                        }
                        s.log.warn(&format!("launch {id}: cookie rejected"));
                        s.save();
                    }
                    emit_account(&app, id);
                    emit_launch(&app, id, "Failed", "The stored cookie is no longer valid. Sign in again through Open Browser.");
                }
                Err(e) => {
                    if let Ok(s) = app.state::<Shared>().lock() {
                        s.log.warn(&format!("launch {id}: {}", describe(&e)));
                    }
                    emit_launch(&app, id, "Failed", describe(&e));
                }
            }
            drop(_serial);
            ensure_settler(app.clone());
        }
    });
}

#[cfg(test)]
mod order_tests {
    use super::order_by_settings;

    #[test]
    fn accounts_with_the_same_settings_launch_together() {
        // 1 and 3 want "a", 2 is Main (the user's own), 4 wants "b".
        let needs = vec![(1, Some("a")), (2, None), (3, Some("a")), (4, Some("b"))];
        // Nothing in place: Main first (no waiting), then each group in the order it first appears.
        assert_eq!(order_by_settings(needs.clone(), None), vec![2, 1, 3, 4]);
        // "b" is already in place: its accounts go first, then Main, then the rest.
        assert_eq!(order_by_settings(needs.clone(), Some(&"b")), vec![4, 2, 1, 3]);
        assert_eq!(order_by_settings(needs, Some(&"a")), vec![1, 3, 2, 4]);
        // One kind only: the user's order is untouched.
        assert_eq!(order_by_settings(vec![(5, Some("a")), (3, Some("a")), (9, Some("a"))], None), vec![5, 3, 9]);
        assert_eq!(order_by_settings::<&str>(vec![], None), Vec::<u64>::new());
    }
}
