//! Shapes sent to the front-end.

use crate::state::AppState;
use ram_core::model::{AccountId, Game, Settings, SignInRecord};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
pub struct AccountView {
    pub id: AccountId,
    pub username: String,
    pub alias: String,
    pub display_name: String,
    pub group: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub favorite: bool,
    pub is_main: bool,
    pub roblox_user_id: Option<u64>,
    pub user_id_confirmed: bool,
    pub sign_in: SignInRecord,
    pub has_cookie: bool,
    pub checking: bool,
    pub last_used: Option<i64>,
    pub robux: Option<i64>,
    pub premium: Option<bool>,
    pub avatar_url: Option<String>,
    pub fps_cap: Option<u32>,
    pub full_quality: bool,
    /// Client state label: "", Queued, Launching, Running, Unconfirmed, Exited, Failed, Paused.
    pub client: String,
    pub client_pid: Option<u32>,
}

#[derive(Serialize, Clone)]
pub struct VaultStatus {
    pub configured: bool,
    pub mode: Option<String>,
    pub unlocked: bool,
    pub cookies: usize,
    pub auto_lock_secs: u32,
    pub require_hello: bool,
    pub hello_available: bool,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub version: String,
    pub accounts: Vec<AccountView>,
    pub settings: Settings,
    pub graphics: ram_core::client_settings::GraphicsSettings,
    pub games: Vec<Game>,
    pub vault: VaultStatus,
    pub main_account: Option<AccountId>,
    pub multi_instance_held: bool,
    pub startup_note: String,
    pub data_dir: String,
    pub browser_name: String,
    pub launcher_name: String,
    pub github_repo: String,
}

#[derive(Deserialize)]
pub struct AccountPatch {
    pub id: AccountId,
    pub alias: Option<String>,
    pub group: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub favorite: Option<bool>,
    pub full_quality: Option<bool>,
    pub fps_cap: Option<u32>,
    /// Set together with `fps_cap: None` to go back to the global cap.
    #[serde(default)]
    pub clear_fps_cap: bool,
}

#[derive(Serialize, Clone)]
pub struct LaunchEvent {
    pub account: AccountId,
    pub state: String,
    pub message: String,
}

#[derive(Serialize, Clone)]
pub struct ClientView {
    pub pid: u32,
    pub account: Option<AccountId>,
    pub account_name: Option<String>,
    pub working_set_mb: u64,
    pub private_mb: u64,
}

pub fn vault_status(s: &AppState) -> VaultStatus {
    VaultStatus {
        configured: s.vault.is_configured(),
        mode: s.vault.mode().map(|m| match m {
            ram_core::vault::VaultMode::Dpapi => "dpapi".to_string(),
            ram_core::vault::VaultMode::MasterPassword => "password".to_string(),
        }),
        unlocked: s.vault.is_unlocked(),
        cookies: s.vault.accounts_with_secret().len(),
        auto_lock_secs: s.vault.auto_lock_secs(),
        require_hello: s.vault.require_hello_reveal(),
        hello_available: ram_win::hello::is_available(),
    }
}

pub fn account_view(s: &AppState, id: AccountId) -> Option<AccountView> {
    let a = s.store.account(id)?;
    let state = s.clients.state(id);
    Some(AccountView {
        id: a.id,
        username: a.username.clone(),
        alias: a.alias.clone(),
        display_name: a.display_name.clone(),
        group: a.group.clone(),
        tags: a.tags.clone(),
        notes: a.notes.clone(),
        favorite: a.favorite,
        is_main: s.store.is_main(a.id),
        roblox_user_id: a.roblox_user_id,
        user_id_confirmed: a.user_id_confirmed,
        sign_in: a.sign_in.clone(),
        has_cookie: s.vault.has_secret(a.id),
        checking: s.checking.contains(&a.id),
        last_used: a.last_used,
        robux: a.robux,
        premium: a.premium,
        avatar_url: a.avatar_url.clone(),
        fps_cap: a.fps_cap,
        full_quality: a.full_quality,
        client: state.label().to_string(),
        client_pid: s.clients.link_for_account(a.id).map(|l| l.process.pid),
    })
}

pub fn snapshot(s: &AppState) -> Snapshot {
    Snapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        accounts: s.store.accounts.iter().filter_map(|a| account_view(s, a.id)).collect(),
        settings: s.store.settings.clone(),
        graphics: s.store.graphics.clone(),
        games: s.store.games.clone(),
        vault: vault_status(s),
        main_account: s.store.main_account,
        multi_instance_held: s.singleton.as_ref().map(|h| h.is_held()).unwrap_or(false),
        startup_note: s.startup_note.clone(),
        data_dir: s.data_dir.display().to_string(),
        browser_name: crate::cmd_browser::pick_browser().map(|b| b.name).unwrap_or_default(),
        launcher_name: ram_win::launcher_info::detect().name,
        github_repo: crate::cmd_update::GITHUB_REPO.to_string(),
    }
}
