//! Who is online or in a game right now (`ram_core::presence`).
//!
//! Every account is asked about itself with its own cookie, a few at a
//! time, every 15 seconds by default (Settings), and again right before a
//! launch so the "already in
//! a game" warning is never based on an old answer. Accounts without a
//! stored cookie are asked about anonymously in one request. Each time an
//! account is seen online, that moment becomes its last-online time (Roblox
//! no longer offers one).

use crate::state::{now, Shared};
use ram_core::model::AccountId;
use ram_core::presence::{self, Presence, PresenceError, PresenceKind};
use ram_win::WinHttp;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Bounds for the interval between background rounds (the setting).
const MIN_EVERY_SECS: u64 = 5;
const MAX_EVERY_SECS: u64 = 300;
/// Longest pause after Roblox asks RAM to slow down.
const MAX_BACKOFF_SECS: u64 = 600;
/// Accounts asked at the same time.
const PARALLEL: usize = 6;
/// The stored last-online time is rewritten to disk at most this often
/// while an account stays online.
const SAVE_EVERY_SECS: i64 = 600;

/// What RAM knows about one account's presence.
#[derive(Clone)]
pub struct PresenceInfo {
    pub presence: Presence,
    pub checked_at: i64,
}

#[derive(Serialize, Clone)]
pub struct PresenceView {
    pub id: AccountId,
    /// "Offline", "Online", "InGame", "InStudio" or "Invisible".
    pub kind: PresenceKind,
    pub location: String,
    pub place_id: Option<u64>,
    pub checked_at: i64,
}

pub fn view(id: AccountId, info: &PresenceInfo) -> PresenceView {
    PresenceView { id, kind: info.presence.kind, location: info.presence.location.clone(), place_id: info.presence.place_id.or(info.presence.root_place_id), checked_at: info.checked_at }
}

struct Ask {
    id: AccountId,
    user_id: u64,
    cookie: Option<String>,
}

/// Asks Roblox about `ids` (every account when `None`) and records the
/// answers. Returns the answers and whether Roblox asked to slow down.
fn check(app: &AppHandle, ids: Option<&[AccountId]>) -> (Vec<(AccountId, Presence)>, bool) {
    let asks: Vec<Ask> = {
        let st = app.state::<Shared>();
        let Ok(s) = st.lock() else { return (Vec::new(), false) };
        let wanted: Vec<(AccountId, u64)> =
            s.store.accounts.iter().filter(|a| ids.is_none_or(|l| l.contains(&a.id))).filter_map(|a| a.roblox_user_id.map(|u| (a.id, u))).collect();
        let unlocked = s.vault.is_unlocked();
        wanted
            .into_iter()
            .map(|(id, user_id)| Ask { id, user_id, cookie: if unlocked { s.vault.secret(id).ok().flatten().map(|c| c.to_string()) } else { None } })
            .collect()
    };
    if asks.is_empty() {
        return (Vec::new(), false);
    }
    let mut results: Vec<(AccountId, Presence)> = Vec::new();
    let mut limited = false;

    // Signed in: each account about itself, a few at a time.
    let (own, anonymous): (Vec<&Ask>, Vec<&Ask>) = asks.iter().partition(|a| a.cookie.is_some());
    for group in own.chunks(PARALLEL) {
        let answers: Vec<(AccountId, Result<Vec<(u64, Presence)>, PresenceError>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = group
                .iter()
                .map(|a| {
                    scope.spawn(move || {
                        let http = WinHttp::new();
                        (a.id, presence::fetch(&http, a.cookie.as_deref(), &[a.user_id]))
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (id, answer) in answers {
            match answer {
                Ok(list) => {
                    if let Some((_, p)) = list.into_iter().next() {
                        results.push((id, p));
                    }
                }
                Err(PresenceError::RateLimited) => limited = true,
                // A rejected cookie is the sign-in check's business; anything
                // else is tried again next round.
                Err(_) => {}
            }
        }
        if limited {
            break;
        }
    }
    // No cookie: one anonymous request for all of them.
    if !anonymous.is_empty() && !limited {
        let users: Vec<u64> = anonymous.iter().map(|a| a.user_id).collect();
        match presence::fetch(&WinHttp::new(), None, &users) {
            Ok(list) => {
                for (user, p) in list {
                    if let Some(a) = anonymous.iter().find(|a| a.user_id == user) {
                        results.push((a.id, p));
                    }
                }
            }
            Err(PresenceError::RateLimited) => limited = true,
            Err(_) => {}
        }
    }

    // Record.
    let mut changed = false;
    {
        let st = app.state::<Shared>();
        let Ok(mut s) = st.lock() else { return (results, limited) };
        let t = now();
        let mut save = false;
        for (id, p) in &results {
            let before = s.presence.get(id).map(|i| (i.presence.kind, i.presence.location.clone()));
            if before != Some((p.kind, p.location.clone())) {
                changed = true;
            }
            s.presence.insert(*id, PresenceInfo { presence: p.clone(), checked_at: t });
            if p.kind.is_active() {
                if let Some(a) = s.store.account_mut(*id) {
                    if a.last_online.is_none_or(|was| t - was >= SAVE_EVERY_SECS) {
                        save = true;
                    }
                    a.last_online = Some(t);
                }
            }
        }
        if save {
            s.save();
        }
    }
    if changed {
        let _ = app.emit("presence-changed", ());
    }
    (results, limited)
}

/// Starts the background rounds.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        // Let the window and the start-up sign-in checks go first.
        std::thread::sleep(Duration::from_secs(4));
        // Extra seconds added after Roblox asks RAM to slow down; halved
        // again by every round that goes through.
        let mut penalty: u64 = 0;
        loop {
            let started = std::time::Instant::now();
            let (_, limited) = check(&app, None);
            let setting = || app.state::<Shared>().lock().map(|s| u64::from(s.store.settings.presence_every_secs)).unwrap_or(15).clamp(MIN_EVERY_SECS, MAX_EVERY_SECS);
            if limited {
                penalty = (penalty.max(setting()) * 2).min(MAX_BACKOFF_SECS);
                if let Ok(s) = app.state::<Shared>().lock() {
                    s.log.warn(&format!("presence: Roblox asked to slow down; waiting {penalty} s longer"));
                }
            } else {
                penalty /= 2;
            }
            // Sleep in short steps so a changed setting applies at once.
            while started.elapsed().as_secs() < setting() + penalty {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    });
}

/// A fresh answer for `ids`, for the "already in a game" warning before a
/// launch. Accounts Roblox did not answer for are left out.
#[tauri::command(async)]
pub fn presence_check(app: AppHandle, state: State<'_, Shared>, ids: Vec<AccountId>) -> Result<Vec<PresenceView>, String> {
    let (results, _) = check(&app, Some(&ids));
    let s = state.lock().map_err(|_| "application state is unavailable".to_string())?;
    Ok(results.iter().filter_map(|(id, _)| s.presence.get(id).map(|i| view(*id, i))).collect())
}
