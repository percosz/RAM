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
    /// FPS cap, graphics quality and rendering flags for alt clients.
    pub graphics: crate::client_settings::GraphicsSettings,
    /// How clients ended, newest first.
    pub crashes: Vec<CrashRecord>,
    /// The user's own `GlobalBasicSettings` values while a launch has ours
    /// in place, so a crash mid-launch can still put them back.
    pub xml_undo: Option<crate::client_settings::XmlUndo>,
    /// Window, sound, CPU, memory and bandwidth limits for alt clients.
    pub limits: LimitSettings,
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
            graphics: Default::default(),
            crashes: Vec::new(),
            xml_undo: None,
            limits: LimitSettings::default(),
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
                    creator: String::new(),
                    fetched_at: None,
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

    /// Adds a crash-history record (newest first) and returns its id.
    pub fn push_crash(&mut self, mut record: CrashRecord) -> u64 {
        let id = self.alloc_id();
        record.id = id;
        self.crashes.insert(0, record);
        self.crashes.truncate(CRASH_LIMIT);
        id
    }

    /// Per-account graphics inputs for a launch.
    pub fn account_graphics(&self, id: AccountId) -> crate::client_settings::AccountGraphics {
        let a = self.account(id);
        crate::client_settings::AccountGraphics {
            fps_cap: a.and_then(|a| a.fps_cap),
            full_quality: a.map(|a| a.full_quality).unwrap_or(false),
        }
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
    /// When RAM last saw this account online (in Roblox's presence, or as
    /// a client running here). Roblox no longer reports this itself.
    #[serde(default)]
    pub last_online: Option<i64>,
    /// Where this entry came from, for repeat-safe imports.
    pub import_key: Option<String>,
    /// Display name reported by Roblox at the last cookie check.
    #[serde(default)]
    pub display_name: String,
    /// Robux balance at the last cookie check, when fetched.
    #[serde(default)]
    pub robux: Option<i64>,
    /// Premium membership at the last cookie check, when fetched.
    #[serde(default)]
    pub premium: Option<bool>,
    /// Headshot image URL from the public thumbnails API, and when it was
    /// fetched (the URLs expire, so they are refreshed daily).
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub avatar_at: Option<i64>,
    /// FPS cap for this account's client; `None` uses the global cap.
    #[serde(default)]
    pub fps_cap: Option<u32>,
    /// Keep the user's own graphics quality for this account (the FPS cap
    /// still applies). Main is always exempt from everything.
    #[serde(default)]
    pub full_quality: bool,
    /// How the stored session was obtained.
    #[serde(default)]
    pub method: SignInMethod,
}

/// How an account's session got into the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignInMethod {
    #[default]
    Unknown,
    /// A pasted or imported `.ROBLOSECURITY` cookie.
    Cookie,
    /// A sign-in in the browser (by hand, or filled from username and
    /// password).
    Login,
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
            last_online: None,
            import_key: None,
            display_name: String::new(),
            robux: None,
            premium: None,
            avatar_url: None,
            avatar_at: None,
            fps_cap: None,
            full_quality: false,
            method: SignInMethod::Unknown,
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
    /// Creator name from the games API, when looked up.
    #[serde(default)]
    pub creator: String,
    /// When the name was last confirmed with Roblox.
    #[serde(default)]
    pub fetched_at: Option<i64>,
}

impl Default for Game {
    fn default() -> Self {
        Game { place_id: 0, universe_id: None, name: String::new(), pinned: false, builtin: false, creator: String::new(), fetched_at: None }
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
    TokyoNight,
    /// Pure black, for OLED screens.
    Midnight,
    Synthwave,
    /// Translucent panels over a soft colour wash.
    Aurora,
    WindowsXp,
    WindowsVista,
    /// Windows 95-style grey with bevelled controls.
    Classic,
    OneDark,
    SolarizedDark,
    SolarizedLight,
    Monokai,
    Everforest,
    Sakura,
    Ocean,
    Sunset,
    /// Glossy aqua and green, mid-2000s style.
    FrutigerAero,
    /// Green on black in a monospace font.
    Terminal,
}

impl ThemeKind {
    /// All themes in the order shown in the picker.
    pub fn all() -> &'static [ThemeKind] {
        use ThemeKind::*;
        &[
            System, Light, Dark, Mocha, Macchiato, Frappe, Latte, Nord, Dracula, Gruvbox, RosePine, Lineage, TokyoNight, Midnight,
            Synthwave, Aurora, WindowsXp, WindowsVista, Classic, OneDark, SolarizedDark, SolarizedLight, Monokai, Everforest, Sakura,
            Ocean, Sunset, FrutigerAero, Terminal,
        ]
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
            ThemeKind::TokyoNight => "Tokyo Night",
            ThemeKind::Midnight => "Midnight (black)",
            ThemeKind::Synthwave => "Synthwave",
            ThemeKind::Aurora => "Aurora (glass)",
            ThemeKind::WindowsXp => "Windows XP",
            ThemeKind::WindowsVista => "Windows Vista",
            ThemeKind::Classic => "Classic (95)",
            ThemeKind::OneDark => "One Dark",
            ThemeKind::SolarizedDark => "Solarized Dark",
            ThemeKind::SolarizedLight => "Solarized Light",
            ThemeKind::Monokai => "Monokai",
            ThemeKind::Everforest => "Everforest",
            ThemeKind::Sakura => "Sakura",
            ThemeKind::Ocean => "Ocean",
            ThemeKind::Sunset => "Sunset",
            ThemeKind::FrutigerAero => "Frutiger Aero",
            ThemeKind::Terminal => "Terminal",
        }
    }
}

/// Row height of the account list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    /// One line per account.
    Compact,
    /// Two lines: name, then alias and description.
    #[default]
    Normal,
    /// Two lines with more air and a larger avatar.
    Comfortable,
    /// Big avatar, big type.
    Large,
}

impl Density {
    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Normal => "Normal",
            Density::Comfortable => "Comfortable",
            Density::Large => "Large",
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
    /// Row size of the account list.
    pub density: Density,
    /// Corner radius for panels, buttons and fields, in device pixels.
    #[serde(default = "default_corner")]
    pub corner_radius: i32,
    /// Hold the Roblox singleton object so several clients can run at once.
    #[serde(default = "yes")]
    pub multi_instance: bool,
    /// Minimum seconds between two handoffs in a batch.
    pub launch_spacing_secs: u32,
    /// Seconds a handoff waits for a new client before it is marked Failed.
    pub launch_timeout_secs: u32,
    /// Whether batch actions and Select All include the Main account.
    pub include_main_in_batch: bool,
    /// Look for a new RAM release on GitHub at start-up.
    #[serde(default = "yes")]
    pub check_updates: bool,
    /// Version whose release notes the user has already seen, so "What's
    /// new" shows once per update.
    pub seen_version: String,
    /// Show only the first three characters of each username (for
    /// screenshots and streams).
    pub hide_usernames: bool,
    /// The first-launch tutorial has been shown (finished or skipped).
    pub tour_seen: bool,
    /// Launch an alt again when its client is dropped by Roblox or crashes,
    /// the same way it last joined. Never Main.
    pub auto_rejoin: bool,
    /// Rejoin attempts before giving up (reset once a client stays up).
    #[serde(default = "default_rejoin_max")]
    pub rejoin_max: u32,
    /// Re-check every stored sign-in this often; 0 is off.
    pub check_every_hours: u32,
    /// Seconds between checks of who is online or in a game.
    #[serde(default = "default_presence_secs")]
    pub presence_every_secs: u32,
    /// Window backdrop: "" (solid), "clear", "mica" or "acrylic". Takes
    /// effect at the next start, because the window has to be created
    /// see-through.
    pub backdrop: String,
    /// How much of the theme's colour covers the desktop when a backdrop is
    /// on, in percent (lower is more see-through).
    #[serde(default = "default_backdrop_tint")]
    pub backdrop_tint: u32,
    /// Browser for Open Browser and sign-in: empty follows the Windows
    /// default; otherwise the full path of a Chromium-based browser.
    pub browser: String,
    pub window: Option<Rect>,
    pub last_import_dir: String,
}

fn default_rejoin_max() -> u32 {
    3
}

fn default_backdrop_tint() -> u32 {
    70
}

fn default_presence_secs() -> u32 {
    45
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeKind::System,
            accent: None,
            density: Density::Normal,
            corner_radius: default_corner(),
            multi_instance: true,
            launch_spacing_secs: 8,
            launch_timeout_secs: 90,
            include_main_in_batch: false,
            check_updates: true,
            seen_version: String::new(),
            hide_usernames: false,
            tour_seen: false,
            auto_rejoin: false,
            rejoin_max: default_rejoin_max(),
            check_every_hours: 0,
            presence_every_secs: default_presence_secs(),
            backdrop: String::new(),
            backdrop_tint: default_backdrop_tint(),
            browser: String::new(),
            window: None,
            last_import_dir: String::new(),
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

/// Limits RAM puts on running alt clients (never on Main or Full-quality
/// accounts). Everything is off by default and undone when switched off or
/// when the client exits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitSettings {
    /// Minimize each alt client once it has joined a game.
    pub minimize_on_launch: bool,
    /// Shrink each alt client's window once it has joined a game.
    pub tiny_on_launch: bool,
    pub tiny_width: u32,
    pub tiny_height: u32,
    /// Mute each alt client in the Windows volume mixer once it has joined.
    pub mute_on_launch: bool,
    /// CPU limit (percent of the whole machine) for alt clients that are not
    /// the foreground window. 0 = off.
    pub cpu_limit_percent: u32,
    /// Windows efficiency mode for background alt clients.
    pub efficiency_mode: bool,
    /// Page alt clients' memory out every N minutes. 0 = off.
    pub trim_every_min: u32,
    /// Bandwidth limit for all Roblox clients in kbit/s (Windows QoS
    /// policy; needs an administrator prompt). 0 = off.
    pub bandwidth_kbps: u32,
}

impl Default for LimitSettings {
    fn default() -> Self {
        LimitSettings {
            minimize_on_launch: false,
            tiny_on_launch: false,
            tiny_width: 320,
            tiny_height: 240,
            mute_on_launch: false,
            cpu_limit_percent: 0,
            efficiency_mode: false,
            trim_every_min: 0,
            bandwidth_kbps: 0,
        }
    }
}

impl LimitSettings {
    pub fn tiny_size(&self) -> (i32, i32) {
        (self.tiny_width.clamp(160, 1920) as i32, self.tiny_height.clamp(120, 1080) as i32)
    }
    /// True when any per-client limit is switched on.
    pub fn any_client_limit(&self) -> bool {
        self.minimize_on_launch || self.tiny_on_launch || self.mute_on_launch || self.cpu_limit_percent > 0 || self.efficiency_mode || self.trim_every_min > 0
    }
}

/// How a Roblox client ended, as seen in its log (Diagnostics → Crash
/// history). The newest `CRASH_LIMIT` are kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrashRecord {
    pub id: u64,
    pub account: Option<AccountId>,
    /// Username at the time, so the record reads well after the account is removed.
    pub username: String,
    pub kind: crate::clientlog::EndKind,
    pub at: i64,
    pub uptime_secs: i64,
    pub reason: String,
    pub place_id: Option<u64>,
    pub log_path: Option<String>,
    pub pid: u32,
}

impl Default for CrashRecord {
    fn default() -> Self {
        CrashRecord {
            id: 0,
            account: None,
            username: String::new(),
            kind: crate::clientlog::EndKind::Closed,
            at: 0,
            uptime_secs: 0,
            reason: String::new(),
            place_id: None,
            log_path: None,
            pid: 0,
        }
    }
}

pub const CRASH_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub at: i64,
    pub source: String,
    pub accounts_added: u32,
    pub accounts_updated: u32,
    pub destinations_added: u32,
    pub skipped: u32,
}
