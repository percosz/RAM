//! Persistent data model. Everything here is local metadata; no cookies,
//! passwords or tokens are ever part of the store.

use crate::platform::{PriorityClass, ProcKey, Rect};
use serde::{Deserialize, Serialize};

/// Current on-disk schema version. Bump when a migration is added in
/// `store::migrate`.
pub const STORE_VERSION: u32 = 1;

pub type AccountId = u64;
pub type DestinationId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub version: u32,
    pub next_id: u64,
    pub accounts: Vec<Account>,
    /// The account pinned as Main, excluded from Select All and batch
    /// actions and exempt from automatic performance changes by default.
    pub main_account: Option<AccountId>,
    pub games: Vec<Game>,
    pub destinations: Vec<Destination>,
    pub recent: Vec<RecentLaunch>,
    pub settings: Settings,
    pub perf: PerfSettings,
    /// Overrides currently applied to client processes, kept so they can be
    /// restored after a crash or restart.
    pub applied: Vec<AppliedOverride>,
    /// Account-to-client links, kept so a restart does not lose them.
    pub links: Vec<crate::clients::ClientLink>,
    pub imports: Vec<ImportRecord>,
}

impl Default for Store {
    fn default() -> Self {
        let mut s = Store {
            version: STORE_VERSION,
            next_id: 1,
            accounts: Vec::new(),
            main_account: None,
            games: Vec::new(),
            destinations: Vec::new(),
            recent: Vec::new(),
            settings: Settings::default(),
            perf: PerfSettings::default(),
            applied: Vec::new(),
            links: Vec::new(),
            imports: Vec::new(),
        };
        s.ensure_builtin_games();
        s
    }
}

impl Store {
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    /// Makes sure the pinned Rogue Lineage entry exists. Called on every load
    /// so a hand-edited store cannot lose it.
    pub fn ensure_builtin_games(&mut self) {
        if !self.games.iter().any(|g| g.place_id == ROGUE_LINEAGE_PLACE_ID) {
            self.games.insert(
                0,
                Game {
                    place_id: ROGUE_LINEAGE_PLACE_ID,
                    universe_id: None,
                    name: "Rogue Lineage".to_string(),
                    pinned: true,
                    builtin: true,
                },
            );
        }
        if let Some(g) = self.games.iter_mut().find(|g| g.place_id == ROGUE_LINEAGE_PLACE_ID) {
            g.builtin = true;
            g.pinned = true;
        }
    }

    pub fn account(&self, id: AccountId) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn account_mut(&mut self, id: AccountId) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    pub fn is_main(&self, id: AccountId) -> bool {
        self.main_account == Some(id)
    }

    pub fn game_name(&self, place_id: u64) -> String {
        self.games
            .iter()
            .find(|g| g.place_id == place_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("Place {place_id}"))
    }

    /// Distinct group names in display order.
    pub fn groups(&self) -> Vec<String> {
        let mut v: Vec<String> = self.accounts.iter().map(|a| a.group.clone()).filter(|g| !g.is_empty()).collect();
        v.sort_by_key(|g| g.to_lowercase());
        v.dedup();
        v
    }

    /// Records a launch in the recent history (newest first, bounded,
    /// one entry per account + destination).
    pub fn push_recent(&mut self, entry: RecentLaunch) {
        self.recent.retain(|r| !(r.account == entry.account && r.target == entry.target));
        self.recent.insert(0, entry);
        self.recent.truncate(RECENT_LIMIT);
    }
}

pub const RECENT_LIMIT: usize = 30;

/// Root place of Rogue Lineage. See docs/RESEARCH_NOTES.md for the source.
pub const ROGUE_LINEAGE_PLACE_ID: u64 = 3016661674;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub id: AccountId,
    pub username: String,
    pub alias: String,
    /// Roblox user ID when known.
    pub roblox_user_id: Option<u64>,
    /// True only when the ID came from Roblox itself (authorization or the
    /// public user lookup), not from an imported file.
    pub user_id_confirmed: bool,
    pub group: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub favorite: bool,
    /// Browser profile this account signs in with. `None` uses the default
    /// browser and the user picks the profile there.
    pub browser: Option<BrowserBinding>,
    pub perf_override: Option<PresetKind>,
    pub sign_in: SignInRecord,
    pub created: i64,
    pub last_used: Option<i64>,
    /// Where this entry came from, for repeat-safe imports.
    pub import_key: Option<String>,
}

impl Default for Account {
    fn default() -> Self {
        Account {
            id: 0,
            username: String::new(),
            alias: String::new(),
            roblox_user_id: None,
            user_id_confirmed: false,
            group: String::new(),
            tags: Vec::new(),
            notes: String::new(),
            favorite: false,
            browser: None,
            perf_override: None,
            sign_in: SignInRecord::default(),
            created: 0,
            last_used: None,
            import_key: None,
        }
    }
}

impl Account {
    /// Alias when set, otherwise the username.
    pub fn display_name(&self) -> &str {
        if self.alias.trim().is_empty() {
            &self.username
        } else {
            &self.alias
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserBinding {
    /// `BrowserInstall::id`.
    pub browser: String,
    /// `BrowserProfile::key`.
    pub profile: String,
    /// Profile name shown to the user when the binding was made.
    pub profile_name: String,
}

/// What the manager knows about an account's sign-in. Deliberately separate
/// from client state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignInState {
    /// Roblox confirmed this identity through a documented authorization
    /// response at `last_check`.
    Verified,
    /// Roblox explicitly reported that the authorization is no longer valid.
    SignedOut,
    /// Roblox or the user reported that verification must be completed in
    /// the browser before continuing.
    VerificationRequired,
    /// Only set from an explicit signal that names a CAPTCHA challenge.
    CaptchaRequired,
    /// The manager handed off to the browser and cannot observe the result.
    CheckInBrowser,
    /// Nothing is known yet.
    #[default]
    Unknown,
}

impl SignInState {
    pub fn label(self) -> &'static str {
        match self {
            SignInState::Verified => "Verified",
            SignInState::SignedOut => "Signed out",
            SignInState::VerificationRequired => "Verification required",
            SignInState::CaptchaRequired => "CAPTCHA required",
            SignInState::CheckInBrowser => "Check in browser",
            SignInState::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SignInRecord {
    pub state: SignInState,
    /// When `state` was last established.
    pub last_check: Option<i64>,
    /// One line explaining the state or the last failed check.
    pub detail: String,
    /// When a check last failed for a reason unrelated to the account
    /// (offline, timeout, rate limit). Never changes `state`.
    pub last_failed_check: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub place_id: u64,
    pub universe_id: Option<u64>,
    pub name: String,
    pub pinned: bool,
    /// Built-in entries cannot be removed.
    pub builtin: bool,
}

impl Default for Game {
    fn default() -> Self {
        Game { place_id: 0, universe_id: None, name: String::new(), pinned: false, builtin: false }
    }
}

/// Where a launch goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// The experience itself; Roblox picks the server.
    Place { place_id: u64 },
    /// A private server reached through its link code. Durable until the
    /// owner regenerates the link.
    PrivateServer { place_id: u64, link_code: String },
    /// A Roblox share link. The manager cannot see what it resolves to.
    ShareLink { code: String, link_type: String },
    /// One specific running server. Goes stale when that server closes.
    LiveServer { place_id: u64, job_id: String },
}

impl Target {
    pub fn place_id(&self) -> Option<u64> {
        match self {
            Target::Place { place_id }
            | Target::PrivateServer { place_id, .. }
            | Target::LiveServer { place_id, .. } => Some(*place_id),
            Target::ShareLink { .. } => None,
        }
    }

    /// Whether the target is a saved destination or a live server that may
    /// have expired.
    pub fn durability(&self) -> Durability {
        match self {
            Target::Place { .. } => Durability::Saved,
            Target::PrivateServer { .. } | Target::ShareLink { .. } => Durability::SavedLink,
            Target::LiveServer { .. } => Durability::Live,
        }
    }

    /// True when the target embeds a private link that must be redacted in
    /// logs and diagnostics.
    pub fn is_private(&self) -> bool {
        matches!(self, Target::PrivateServer { .. } | Target::ShareLink { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Always joinable while the experience exists.
    Saved,
    /// Saved, but depends on a link its owner can regenerate.
    SavedLink,
    /// A live server captured at some moment; may have expired.
    Live,
}

impl Durability {
    pub fn label(self) -> &'static str {
        match self {
            Durability::Saved => "Saved",
            Durability::SavedLink => "Saved link",
            Durability::Live => "Live server (may have expired)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub id: DestinationId,
    pub name: String,
    pub target: Target,
    #[serde(default)]
    pub notes: String,
    pub created: i64,
    #[serde(default)]
    pub last_used: Option<i64>,
    #[serde(default)]
    pub import_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentLaunch {
    pub account: AccountId,
    pub target: Target,
    pub at: i64,
}

/// A named colour theme. `System` follows the Windows light/dark setting;
/// the rest are fixed palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeKind {
    #[default]
    System,
    Light,
    Dark,
    Mocha,
    Macchiato,
    Frappe,
    Latte,
    Nord,
    Dracula,
    Gruvbox,
    RosePine,
    /// Muted, Rogue Lineage-inspired palette.
    Lineage,
}

impl ThemeKind {
    /// All themes in the order shown in the picker.
    pub fn all() -> &'static [ThemeKind] {
        use ThemeKind::*;
        &[System, Light, Dark, Mocha, Macchiato, Frappe, Latte, Nord, Dracula, Gruvbox, RosePine, Lineage]
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::System => "System",
            ThemeKind::Light => "Light",
            ThemeKind::Dark => "Dark",
            ThemeKind::Mocha => "Catppuccin Mocha",
            ThemeKind::Macchiato => "Catppuccin Macchiato",
            ThemeKind::Frappe => "Catppuccin Frappé",
            ThemeKind::Latte => "Catppuccin Latte",
            ThemeKind::Nord => "Nord",
            ThemeKind::Dracula => "Dracula",
            ThemeKind::Gruvbox => "Gruvbox",
            ThemeKind::RosePine => "Rosé Pine",
            ThemeKind::Lineage => "Lineage (muted)",
        }
    }
}

/// How the Accounts screen is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayoutMode {
    /// The modern screen: avatars, grouped list, details card, legend bar.
    #[default]
    Modern,
    /// The classic dense table with an inline quick-launch toolbar.
    Classic,
}

impl LayoutMode {
    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Modern => "Modern",
            LayoutMode::Classic => "Classic",
        }
    }
}

fn default_corner() -> i32 {
    6
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeKind,
    /// Custom accent as 0xRRGGBB; `None` uses the theme's accent.
    pub accent: Option<u32>,
    pub compact: bool,
    /// Modern or classic Accounts layout.
    #[serde(default)]
    pub layout: LayoutMode,
    /// Corner radius for panels, buttons and fields, in device pixels.
    #[serde(default = "default_corner")]
    pub corner_radius: i32,
    /// Launch the client directly with a stored cookie when the vault is
    /// unlocked, instead of handing off to the browser.
    #[serde(default = "yes")]
    pub direct_launch: bool,
    /// Hold the Roblox singleton object so several clients can run at once.
    #[serde(default = "yes")]
    pub multi_instance: bool,
    /// Launch handoffs in flight at once.
    pub launch_concurrency: u32,
    /// Minimum seconds between two handoffs in a batch.
    pub launch_spacing_secs: u32,
    /// Seconds a handoff waits for a new client before it is marked Failed.
    pub launch_timeout_secs: u32,
    /// Whether batch actions and Select All include the Main account.
    pub include_main_in_batch: bool,
    /// Client ID of the user's own Roblox OAuth application; enables
    /// verified sign-in checks. Not a secret.
    pub oauth_client_id: String,
    pub oauth_port: u16,
    /// Minutes of no manager-observed attention before an idle reminder.
    /// 0 disables reminders.
    pub idle_reminder_minutes: u32,
    pub window: Option<Rect>,
    pub last_import_dir: String,
    pub default_destination: Option<DestinationId>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeKind::System,
            accent: None,
            compact: false,
            layout: LayoutMode::Modern,
            corner_radius: default_corner(),
            direct_launch: true,
            multi_instance: true,
            launch_concurrency: 1,
            launch_spacing_secs: 8,
            launch_timeout_secs: 90,
            include_main_in_batch: false,
            oauth_client_id: String::new(),
            oauth_port: 53682,
            idle_reminder_minutes: 0,
            window: None,
            last_import_dir: String::new(),
            default_destination: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PresetKind {
    /// No automatic changes.
    #[default]
    Off,
    Balanced,
    LowResource,
}

impl PresetKind {
    pub fn label(self) -> &'static str {
        match self {
            PresetKind::Off => "Off",
            PresetKind::Balanced => "Balanced",
            PresetKind::LowResource => "Low Resource",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerfSettings {
    /// Preset for background clients. Off by default.
    pub preset: PresetKind,
    /// When false (default) the Main account's client is never changed
    /// automatically.
    pub apply_to_main: bool,
    /// Optional Job Object CPU limit for background clients, percent of
    /// total CPU. Separate opt-in; `None` never creates a job.
    pub cpu_cap_percent: Option<u32>,
    /// Advisory per-client private memory threshold in MiB. 0 disables.
    pub client_private_warn_mb: u32,
    /// Advisory system memory load threshold in percent. 0 disables.
    pub system_load_warn_percent: u32,
    pub notify_on_threshold: bool,
    /// Seconds between samples while the Performance view is visible.
    pub sample_secs_visible: u32,
    /// Seconds between samples while it is hidden.
    pub sample_secs_hidden: u32,
    pub tile_on_launch: bool,
}

impl Default for PerfSettings {
    fn default() -> Self {
        PerfSettings {
            preset: PresetKind::Off,
            apply_to_main: false,
            cpu_cap_percent: None,
            client_private_warn_mb: 0,
            system_load_warn_percent: 0,
            notify_on_threshold: false,
            sample_secs_visible: 2,
            sample_secs_hidden: 15,
            tile_on_launch: false,
        }
    }
}

/// A change the manager made to a client process, with what is needed to
/// undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedOverride {
    pub process: ProcKey,
    pub account: Option<AccountId>,
    /// Priority the client had before the manager changed it.
    pub original_priority: Option<PriorityClass>,
    pub efficiency_mode: bool,
    pub cpu_cap: bool,
    /// Window placement before tiling.
    pub original_rect: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub at: i64,
    pub source: String,
    pub accounts_added: u32,
    pub accounts_updated: u32,
    pub destinations_added: u32,
    pub skipped: u32,
}
