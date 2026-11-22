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

#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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

/// One launch at a time: the graphics files are shared by every client, so
/// a launch holds this until its client has read them and they are back
/// to the user's own values.
static LAUNCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long to wait for the client to read its settings before restoring
/// them anyway.
const READ_WAIT_SECS: u64 = 60;

/// Writes the FPS cap / quality (XML) and rendering flags (JSON) for one
/// account's launch. Returns the undo handles, or `None` when nothing is to
/// be written (settings off, Main account, no location known).
fn apply_graphics(app: &AppHandle, id: AccountId) -> Option<(Option<client_settings::XmlUndo>, Option<client_settings::FlagUndo>, Vec<String>)> {
    let st = app.state::<Shared>();
    let mut s = st.lock().ok()?;
    let eff = client_settings::effective(&s.store.graphics, &s.store.account_graphics(id), s.store.is_main(id))?;
    let mut notes = Vec::new();
    let mut xml_undo = None;
    if eff.fps_cap > 0 || eff.low_graphics || eff.mute {
        let path = ram_win::paths::local_app_data().map(|p| p.join("Roblox").join("GlobalBasicSettings_13.xml"));
        if let Some(path) = path {
            let file = client_settings::XmlFile::new(path);
            let (_, current) = file.read();
            let want = client_settings::xml_for_launch(&eff, &current);
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
    let set = client_settings::flag_set(&eff);
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
    Some((xml_undo, flag_undo, notes))
}

/// Waits until the watcher reports that the client with this tracker id
/// has read its settings (or it exits, or the wait times out).
fn wait_for_read(app: &AppHandle, btid: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(READ_WAIT_SECS);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let st = app.state::<Shared>();
        let Ok(s) = st.lock() else { return };
        if s.diag.reads.get(btid).copied().unwrap_or(false) {
            drop(s);
            // Roblox reads the files a moment after the first mark.
            std::thread::sleep(std::time::Duration::from_secs(1));
            return;
        }
    }
    if let Ok(s) = app.state::<Shared>().lock() {
        s.log.warn(&format!("launch: no settings-read mark within {READ_WAIT_SECS} s; restoring anyway"));
    }
}

fn restore_graphics(app: &AppHandle, xml_undo: Option<client_settings::XmlUndo>, flag_undo: Option<client_settings::FlagUndo>) {
    let st = app.state::<Shared>();
    let Ok(mut s) = st.lock() else { return };
    if let Some(u) = flag_undo {
        if let Err(e) = u.put_back() {
            s.log.warn(&format!("flag restore: {e}"));
        }
    }
    if let Some(u) = xml_undo {
        if let Err(e) = u.put_back() {
            s.log.warn(&format!("settings restore: {e}"));
        }
        s.store.xml_undo = None;
        s.save();
    }
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
                    let graphics = apply_graphics(&app, id);
                    if let Some((_, _, notes)) = &graphics {
                        for n in notes {
                            emit_launch(&app, id, "Launching", n.clone());
                        }
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
                            if let Some((xml_undo, flag_undo, _)) = graphics {
                                if xml_undo.is_some() || flag_undo.is_some() {
                                    wait_for_read(&app, &btid);
                                    restore_graphics(&app, xml_undo, flag_undo);
                                }
                            }
                        }
                        Err(e) => {
                            if let Some((xml_undo, flag_undo, _)) = graphics {
                                restore_graphics(&app, xml_undo, flag_undo);
                            }
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
        }
    });
}
