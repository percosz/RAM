//! Application state and the actions shared by every view.
//!
//! All of this lives on the GUI thread. Worker threads (network checks, the
//! OAuth redirect wait) never touch `App`; they post a message with a boxed
//! result back to the main window.

use crate::theme::{self, Brushes, ButtonKind, Palette};
use crate::ui::{self, Metrics};
use crate::{accounts, destinations, performance, settings};
use ram_core::clients::{ClientEvent, ClientState, Clients};
use ram_core::links;
use ram_core::model::{AccountId, RecentLaunch, Store, Target};
use ram_core::oauth::{self, AuthRequest, CheckResult};
use ram_core::perf::{self, IdleTracker, NotifyGate};
use ram_core::platform::*;
use ram_core::redact::Log;
use ram_core::search::SearchIndex;
use ram_core::signin::{self, CheckOutcome};
use ram_core::store::StoreFile;
use ram_core::vault::Vault;
use ram_core::{roblox, vault};
use crate::vaultsvc::{WinEntropy, WinProtector};
use ram_win::{ExitWatcher, SingletonHold, TrayNotifier, WinBrowsers, WinHttp, WinJobs, WinProcesses, WinSecrets, WinWindows, WindowShowHook};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::BOOL;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_PROC_EXIT: u32 = WM_APP + 1;
pub const WM_PROC_SHOW: u32 = WM_APP + 2;
pub const WM_CHECK_DONE: u32 = WM_APP + 3;
pub const WM_TRAY: u32 = WM_APP + 4;
pub const WM_LAUNCH_DONE: u32 = WM_APP + 5;

pub const TIMER_SAVE: usize = 1;
pub const TIMER_QUEUE: usize = 2;
pub const TIMER_SAMPLE: usize = 3;
pub const TIMER_RESCAN: usize = 4;
pub const TIMER_SEARCH: usize = 5;
pub const TIMER_DETAILS: usize = 6;
pub const TIMER_VAULT: usize = 7;

/// Whether an account has a stored session cookie in the vault.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CookieState {
    /// No vault is set up at all.
    NoVault,
    /// A cookie is stored for this account.
    Stored,
    /// No cookie stored for this account.
    Missing,
}

pub const ID_NAV_FIRST: u16 = 101;
pub const ID_STATUS: u16 = 110;
pub const ID_TITLE: u16 = 111;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewId {
    Accounts = 0,
    Destinations = 1,
    Performance = 2,
    Settings = 3,
}

pub const VIEWS: [(ViewId, &str); 4] = [
    (ViewId::Accounts, "Accounts"),
    (ViewId::Destinations, "Destinations"),
    (ViewId::Performance, "Performance"),
    (ViewId::Settings, "Settings"),
];

/// Latest sample of one client, with the CPU share since the previous one.
#[derive(Clone, Copy, Default)]
pub struct Sampled {
    pub sample: ProcSample,
    pub cpu_percent: Option<f32>,
    pub at_ticks: u64,
}

/// Result a worker thread posts back with WM_CHECK_DONE.
pub struct CheckDone {
    pub account: AccountId,
    pub result: Result<CheckResult, String>,
}

/// Outcome of a direct launch's ticket preparation.
pub enum Launched {
    /// The stored cookie was rejected by Roblox.
    Unauthorized,
    /// A displayable, inconclusive reason.
    Message(String),
}

/// Result a worker thread posts back with WM_LAUNCH_DONE: the built launch
/// URI, or why it could not be built.
pub struct LaunchDone {
    pub account: AccountId,
    pub target: Target,
    pub result: Result<String, Launched>,
}

fn launch_error_message(e: &roblox::LaunchError) -> Launched {
    use roblox::LaunchError::*;
    match e {
        Unauthorized => Launched::Unauthorized,
        Offline => Launched::Message("this PC appears to be offline".into()),
        Timeout => Launched::Message("the request to Roblox timed out".into()),
        RateLimited { retry_after_secs: Some(s) } => Launched::Message(format!("Roblox rate limit; retry in {s} s")),
        RateLimited { retry_after_secs: None } => Launched::Message("Roblox rate limit".into()),
        Service { status } => Launched::Message(format!("Roblox returned HTTP {status}")),
        Malformed => Launched::Message("Roblox returned an unexpected response".into()),
    }
}

pub struct App {
    pub hwnd: HWND,
    pub data_dir: PathBuf,
    pub store: Store,
    pub store_file: StoreFile,
    pub read_only: bool,
    pub log: Log,
    pub clients: Clients,
    pub index: SearchIndex,
    pub procs: WinProcesses,
    pub jobs: WinJobs,
    pub windows: WinWindows,
    pub browsers: WinBrowsers,
    pub exit_watch: ExitWatcher,
    pub show_hook: Option<WindowShowHook>,
    pub tray: Option<TrayNotifier>,
    pub palette: Palette,
    pub brushes: Brushes,
    pub m: Metrics,
    pub view: ViewId,
    pub nav: [HWND; 4],
    pub status: HWND,
    /// App name shown at the top of the sidebar.
    pub title: HWND,
    pub panels: [HWND; 4],
    pub accounts: accounts::AccountsView,
    pub destinations: destinations::DestinationsView,
    pub performance: performance::PerformanceView,
    pub settings: settings::SettingsView,
    pub samples: HashMap<ProcKey, Sampled>,
    pub perf_notes: HashMap<ProcKey, String>,
    pub idle: IdleTracker,
    pub gate: NotifyGate,
    pub checking: Vec<AccountId>,
    pub auth_cancel: Option<Arc<AtomicBool>>,
    pub cpus: u32,
    /// Saved account links are re-applied once, after the first snapshot.
    /// Views other than Accounts are built the first time they are opened,
    /// which keeps startup short.
    pub created: [bool; 4],
    pub links_restored: bool,
    /// True when some client could not be opened for an exit wait, so a
    /// slow poll has to notice its exit instead.
    pub unwatched: bool,
    /// Encrypted store of session cookies.
    pub vault: Vault,
    pub entropy: WinEntropy,
    pub protector: WinProtector,
    /// Held while multi-instance is on, so several clients can run.
    pub singleton: Option<SingletonHold>,
}

thread_local! {
    static APP: Cell<*mut App> = const { Cell::new(std::ptr::null_mut()) };
}

pub fn install(app: Box<App>) {
    APP.with(|a| a.set(Box::into_raw(app)));
}

/// The application state, once the main window exists.
///
/// The GUI is single-threaded and window procedures re-enter (a control
/// notifies its parent while the parent is updating it), so the state is
/// reached through one thread-local pointer instead of a borrow-checked
/// cell. Handlers set a view's `loading` flag around programmatic updates
/// so re-entrant notifications are ignored.
pub fn try_app() -> Option<&'static mut App> {
    let p = APP.with(|a| a.get());
    if p.is_null() {
        None
    } else {
        // SAFETY: set once on the GUI thread, freed only in `uninstall`
        // after the message loop ends; never shared with other threads.
        Some(unsafe { &mut *p })
    }
}

pub fn uninstall() {
    let p = APP.with(|a| a.replace(std::ptr::null_mut()));
    if !p.is_null() {
        // SAFETY: `p` came from `Box::into_raw` in `install`.
        drop(unsafe { Box::from_raw(p) });
    }
}

pub fn now() -> i64 {
    SystemClock.now()
}

impl App {
    pub fn set_status(&mut self, text: &str) {
        ui::set_text_if_changed(self.status, text);
    }

    pub fn log(&self, message: &str) {
        self.log.line(now(), message);
    }

    /// Schedules a save shortly after the last change.
    pub fn mark_dirty(&mut self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SAVE, 600, None);
        }
    }

    pub fn save_now(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
        }
        if self.read_only {
            return;
        }
        self.store.links = self.clients.links().to_vec();
        if let Err(e) = self.store_file.save(&self.store) {
            self.log(&format!("save failed: {e}"));
            self.set_status(&format!("Could not save: {e}"));
        }
    }

    pub fn rebuild_index(&mut self) {
        self.index = SearchIndex::build(&self.store.accounts);
    }

    pub fn account_name(&self, id: AccountId) -> String {
        self.store.account(id).map(|a| a.display_name().to_string()).unwrap_or_else(|| format!("#{id}"))
    }

    /// Clients paired with the account each is linked to.
    pub fn client_pairs(&self) -> Vec<(ProcKey, Option<AccountId>)> {
        self.clients
            .processes()
            .iter()
            .map(|p| (p.key, self.clients.link_for_process(p.key).map(|l| l.account)))
            .collect()
    }

    // ---- browser hand-off -------------------------------------------------

    /// Opens a Roblox URL in the account's browser profile, or the default
    /// browser when none is bound. Returns the reason on failure.
    pub fn open_for(&mut self, account: AccountId, url: &str) -> Result<(), String> {
        let binding = self.store.account(account).and_then(|a| a.browser.clone());
        let target = binding.as_ref().map(|b| (b.browser.as_str(), b.profile.as_str()));
        match self.browsers.open(target, url) {
            Ok(()) => {
                self.log(&format!("opened {url} for account {account}"));
                Ok(())
            }
            Err(PlatformError::NotFound) if binding.is_some() => {
                let b = binding.unwrap();
                Err(format!(
                    "The browser profile \"{}\" was not found. Pick the profile again in the details pane.",
                    b.profile_name
                ))
            }
            Err(e) => Err(format!("The browser could not be started: {e}.")),
        }
    }

    pub fn open_sign_in(&mut self, account: AccountId, url: &str, what: &str) {
        match self.open_for(account, url) {
            Ok(()) => {
                if let Some(a) = self.store.account_mut(account) {
                    signin::apply(&mut a.sign_in, &CheckOutcome::HandedOff { what: what.to_string() }, now());
                }
                self.mark_dirty();
                self.set_status(&format!("{what} opened in the browser for {}.", self.account_name(account)));
            }
            Err(e) => self.set_status(&e),
        }
        accounts::refresh_rows(self);
    }

    // ---- verified sign-in checks -----------------------------------------

    /// Runs a verified check on a worker thread. When the vault is unlocked
    /// and holds a cookie, the cookie confirms the account directly;
    /// otherwise the optional OAuth check is used.
    pub fn check_sign_in(&mut self, account: AccountId) {
        if self.checking.contains(&account) {
            return;
        }
        let expected = self.store.account(account).and_then(|a| a.roblox_user_id);
        let cookie = if self.vault.is_unlocked() {
            self.vault.secret(account).ok().flatten().map(|z| z.to_string())
        } else {
            None
        };
        self.checking.push(account);
        let hwnd = self.hwnd.0 as isize;
        if let Some(cookie) = cookie {
            std::thread::spawn(move || {
                let result = match roblox::whoami(&WinHttp::new(), &cookie) {
                    Ok(u) => {
                        let outcome = match expected {
                            Some(exp) if exp != u.id => CheckOutcome::DifferentAccount { username: u.name.clone() },
                            _ => CheckOutcome::Confirmed,
                        };
                        oauth::CheckResult {
                            outcome,
                            identity: Some(oauth::UserInfo { user_id: u.id, username: u.name }),
                        }
                    }
                    Err(e) => oauth::CheckResult { outcome: e.to_sign_in_outcome(), identity: None },
                };
                post_done(hwnd, CheckDone { account, result: Ok(result) });
            });
        } else {
            let client_id = self.store.settings.oauth_client_id.trim().to_string();
            std::thread::spawn(move || {
                let result = oauth::check_account(&WinHttp::new(), &WinSecrets::new(), &client_id, account, expected);
                post_done(hwnd, CheckDone { account, result: Ok(result) });
            });
        }
        accounts::refresh_rows(self);
    }

    // ---- vault & cookies -------------------------------------------------

    pub fn vault_configured(&self) -> bool {
        self.vault.is_configured()
    }

    pub fn is_vault_unlocked(&self) -> bool {
        self.vault.is_unlocked()
    }

    pub fn cookie_state(&self, account: AccountId) -> CookieState {
        if !self.vault.is_configured() {
            CookieState::NoVault
        } else if self.vault.has_secret(account) {
            CookieState::Stored
        } else {
            CookieState::Missing
        }
    }

    /// Ensures the vault is unlocked, running the unlock flow if needed.
    pub fn ensure_vault_unlocked(&mut self) -> bool {
        if !self.vault.is_configured() {
            self.set_status("Set up the cookie vault in Settings \u{2192} Security first.");
            return false;
        }
        if self.vault.is_unlocked() {
            self.vault.touch(now());
            return true;
        }
        match self.vault.mode() {
            Some(vault::VaultMode::Dpapi) => match self.vault.unlock_dpapi(&self.protector, now()) {
                Ok(()) => {
                    self.update_timers();
                    true
                }
                Err(e) => {
                    self.set_status(&format!("Could not unlock the vault: {e}"));
                    false
                }
            },
            Some(vault::VaultMode::MasterPassword) => {
                let pw = crate::dialogs::prompt_secret(
                    self.hwnd,
                    "Unlock vault",
                    "Enter your master password to unlock the cookie vault.",
                    "Master password",
                    "Unlock",
                    &|p| if p.is_empty() { Err("Enter your password.".into()) } else { Ok(()) },
                );
                match pw {
                    Some(pw) => match self.vault.unlock_password(&pw, now()) {
                        Ok(()) => {
                            self.set_status("Vault unlocked.");
                            self.update_timers();
                            accounts::refresh_rows(self);
                            true
                        }
                        Err(e) => {
                            self.set_status(&format!("{e}"));
                            false
                        }
                    },
                    None => false,
                }
            }
            None => false,
        }
    }

    /// Prompts for a cookie and stores it in the vault for an account.
    pub fn set_cookie(&mut self, account: AccountId) {
        if !self.ensure_vault_unlocked() {
            return;
        }
        let name = self.account_name(account);
        let cookie = crate::dialogs::prompt_secret(
            self.hwnd,
            &format!("Store cookie for {name}"),
            "Paste this account's .ROBLOSECURITY cookie. It is encrypted in the vault and never written to the plain data file or the log. Get it from your browser's cookies for roblox.com while signed in to this account.",
            ".ROBLOSECURITY",
            "Store",
            &|c| {
                let c = c.trim();
                if c.len() < 32 {
                    Err("That does not look like a .ROBLOSECURITY cookie.".into())
                } else {
                    Ok(())
                }
            },
        );
        let Some(cookie) = cookie else { return };
        match self.vault.set_secret(account, cookie.trim(), &self.entropy, now()) {
            Ok(()) => {
                self.set_status(&format!("Cookie stored for {name}. Checking sign-in\u{2026}"));
                self.check_sign_in(account);
            }
            Err(e) => self.set_status(&format!("Could not store the cookie: {e}")),
        }
        accounts::refresh_rows(self);
    }

    /// Confirms with Windows Hello or the master password before a reveal or
    /// copy. DPAPI vaults with no Hello fall through, since Windows already
    /// gated them at sign-in.
    fn gate_reveal(&mut self) -> bool {
        if self.vault.require_hello_reveal() && ram_win::hello::is_available() {
            return match crate::vaultsvc::hello_confirm("Confirm to reveal a Roblox session cookie") {
                Ok(v) => v,
                Err(_) => self.gate_with_password(),
            };
        }
        if self.vault.mode() == Some(vault::VaultMode::MasterPassword) {
            return self.gate_with_password();
        }
        true
    }

    fn gate_with_password(&mut self) -> bool {
        if self.vault.mode() != Some(vault::VaultMode::MasterPassword) {
            return true;
        }
        let pw = crate::dialogs::prompt_secret(
            self.hwnd,
            "Confirm",
            "Enter your master password to reveal this cookie.",
            "Master password",
            "Confirm",
            &|p| if p.is_empty() { Err("Enter your password.".into()) } else { Ok(()) },
        );
        match pw {
            Some(pw) if self.vault.verify_password(&pw) => true,
            Some(_) => {
                self.set_status("Incorrect master password.");
                false
            }
            None => false,
        }
    }

    pub fn reveal_cookie(&mut self, account: AccountId) {
        if !self.ensure_vault_unlocked() || !self.gate_reveal() {
            return;
        }
        match self.vault.secret(account) {
            Ok(Some(cookie)) => {
                crate::dialogs::show_secret(
                    self.hwnd,
                    &format!("Cookie for {}", self.account_name(account)),
                    "Keep this secret: anyone with it can sign in as this account. Copy closes this window.",
                    &cookie,
                );
                self.set_status("");
            }
            Ok(None) => self.set_status("No cookie is stored for this account."),
            Err(e) => self.set_status(&format!("{e}")),
        }
    }

    pub fn copy_cookie(&mut self, account: AccountId) {
        if !self.ensure_vault_unlocked() || !self.gate_reveal() {
            return;
        }
        match self.vault.secret(account) {
            Ok(Some(cookie)) => {
                if ui::copy_to_clipboard(self.hwnd, &cookie) {
                    self.set_status("Cookie copied to the clipboard. Clear it when you are done.");
                } else {
                    self.set_status("Could not copy the cookie.");
                }
            }
            Ok(None) => self.set_status("No cookie is stored for this account."),
            Err(e) => self.set_status(&format!("{e}")),
        }
    }

    pub fn remove_cookie(&mut self, account: AccountId) {
        let name = self.account_name(account);
        if !ui::confirm(self.hwnd, "Remove cookie", &format!("Remove the stored cookie for {name}? The account stays in the list; only the saved cookie is deleted from this PC.")) {
            return;
        }
        match self.vault.remove_secret(account) {
            Ok(()) => self.set_status(&format!("Cookie removed for {name}.")),
            Err(e) => self.set_status(&format!("{e}")),
        }
        accounts::refresh_rows(self);
    }

    /// A one-line summary of the vault state for the Settings screen.
    pub fn vault_status_text(&self) -> String {
        if !self.vault.is_configured() {
            return "Not set up. Stored cookies let RAM launch and verify accounts without a browser.".to_string();
        }
        let count = self.vault.accounts_with_secret().len();
        let cookies = format!("{count} cookie{}", if count == 1 { "" } else { "s" });
        match self.vault.mode() {
            Some(vault::VaultMode::Dpapi) => format!("Windows account (DPAPI), unlocked \u{00B7} {cookies}."),
            Some(vault::VaultMode::MasterPassword) => {
                let state = if self.vault.is_unlocked() { "unlocked" } else { "locked" };
                format!("Master password, {state} \u{00B7} {cookies}.")
            }
            None => cookies,
        }
    }

    /// Guides the user through setting up the vault for the first time.
    pub fn setup_vault_flow(&mut self) {
        if self.vault.is_configured() {
            self.set_status("A cookie vault is already set up.");
            return;
        }
        let choice = crate::dialogs::choose(
            self.hwnd,
            "Set up cookie vault",
            "Choose how to protect stored session cookies.",
            "Continue",
            &[
                "Windows account (DPAPI) \u{2014} unlocks automatically for you, no password".to_string(),
                "Master password \u{2014} Argon2id + AES-256-GCM, auto-lock, recovery code".to_string(),
            ],
        );
        match choice {
            Some(0) => match self.vault.setup_dpapi(&self.entropy, &self.protector, 0) {
                Ok(()) => self.set_status("Cookie vault set up. Reveal and copy are gated by Windows Hello when it is available."),
                Err(e) => self.set_status(&format!("Could not set up the vault: {e}")),
            },
            Some(1) => self.setup_master_password_flow(),
            _ => {}
        }
        settings::refresh(self);
        accounts::refresh_rows(self);
    }

    fn setup_master_password_flow(&mut self) {
        let pw = crate::dialogs::prompt_secret(
            self.hwnd,
            "Set master password",
            "Choose a master password for the cookie vault. If you forget it, only the recovery code can restore access.",
            "Master password",
            "Continue",
            &|p| if p.len() < 8 { Err("Use at least 8 characters.".into()) } else { Ok(()) },
        );
        let Some(pw) = pw else { return };
        let confirm = crate::dialogs::prompt_secret(
            self.hwnd,
            "Confirm password",
            "Enter the same master password again.",
            "Master password",
            "Set up",
            &|_| Ok(()),
        );
        if confirm.as_deref() != Some(pw.as_str()) {
            self.set_status("The passwords did not match. Nothing was changed.");
            return;
        }
        match self.vault.setup_master_password(&pw, &self.entropy, 15 * 60) {
            Ok(recovery) => {
                self.show_recovery_code(recovery);
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_VAULT, 20_000, None);
                }
                self.set_status("Cookie vault set up. It locks after 15 minutes idle.");
            }
            Err(e) => self.set_status(&format!("Could not set up the vault: {e}")),
        }
    }

    fn show_recovery_code(&mut self, recovery: vault::RecoveryCode) {
        crate::dialogs::show_secret(
            self.hwnd,
            "Recovery code",
            "Save this code somewhere safe. It is the only way to unlock the vault if you forget the master password, and it is shown only once. Copy closes this window.",
            recovery.display(),
        );
    }

    pub fn lock_vault(&mut self) {
        self.vault.lock();
        self.set_status("Vault locked.");
        settings::refresh(self);
        accounts::refresh_rows(self);
    }

    pub fn change_master_password_flow(&mut self) {
        if self.vault.mode() != Some(vault::VaultMode::MasterPassword) {
            return;
        }
        if !self.ensure_vault_unlocked() {
            return;
        }
        let pw = crate::dialogs::prompt_secret(
            self.hwnd,
            "Change master password",
            "Choose a new master password. A fresh recovery code will be shown.",
            "New password",
            "Continue",
            &|p| if p.len() < 8 { Err("Use at least 8 characters.".into()) } else { Ok(()) },
        );
        let Some(pw) = pw else { return };
        match self.vault.change_master_password(&pw, &self.entropy) {
            Ok(recovery) => {
                self.show_recovery_code(recovery);
                self.set_status("Master password changed.");
            }
            Err(e) => self.set_status(&format!("{e}")),
        }
        settings::refresh(self);
    }

    pub fn remove_vault_flow(&mut self) {
        if !self.vault.is_configured() {
            return;
        }
        let n = self.vault.accounts_with_secret().len();
        if !ui::confirm(
            self.hwnd,
            "Remove vault",
            &format!(
                "Delete the cookie vault and all {n} stored cookie(s) from this PC? This cannot be undone. The accounts stay in the list and the Roblox sessions are not affected."
            ),
        ) {
            return;
        }
        match self.vault.destroy() {
            Ok(()) => self.set_status("Cookie vault removed."),
            Err(e) => self.set_status(&format!("{e}")),
        }
        settings::refresh(self);
        accounts::refresh_rows(self);
    }

    pub fn set_require_hello(&mut self, on: bool) {
        if let Err(e) = self.vault.set_require_hello_reveal(on) {
            self.set_status(&format!("{e}"));
        }
    }

    pub fn set_auto_lock_minutes(&mut self, minutes: u32) {
        if let Err(e) = self.vault.set_auto_lock_secs(minutes.saturating_mul(60)) {
            self.set_status(&format!("{e}"));
        }
    }

    /// Exports accounts to a legacy `AccountData.json`, optionally including
    /// the stored cookies (which requires unlocking the vault).
    pub fn export_accounts_flow(&mut self) {
        use ram_core::export::{self, ExportEncryption, ExportOptions};
        let ids: Vec<AccountId> = self.store.accounts.iter().map(|a| a.id).collect();
        if ids.is_empty() {
            self.set_status("There are no accounts to export.");
            return;
        }
        let with_cookies = self.vault.is_configured()
            && ui::message_box(
                self.hwnd,
                "Export",
                "Include stored session cookies in the export?\n\nYes writes an encrypted AccountData.json that another manager can sign in with. No exports names and settings only.",
                MB_YESNO | MB_ICONQUESTION,
            ) == IDYES;
        if with_cookies && !self.ensure_vault_unlocked() {
            return;
        }
        let Some(dir) = crate::dialogs::pick_folder(self.hwnd, "Choose a folder for AccountData.json") else {
            return;
        };
        // Cookies come from the (unlocked) vault; collect them up front.
        let mut cookies: HashMap<AccountId, String> = HashMap::new();
        if with_cookies {
            for id in &ids {
                if let Ok(Some(c)) = self.vault.secret(*id) {
                    cookies.insert(*id, c.to_string());
                }
            }
        }
        let opts = ExportOptions {
            encryption: if with_cookies { ExportEncryption::Dpapi } else { ExportEncryption::Plaintext },
            include_cookies: with_cookies,
        };
        let protector = crate::vaultsvc::LegacyDpapi;
        let cookie_of = |id: AccountId| cookies.get(&id).cloned();
        match export::export(&self.store, &ids, &cookie_of, &opts, Some(&protector)) {
            Ok((bytes, summary)) => {
                let path = dir.join("AccountData.json");
                match std::fs::write(&path, &bytes) {
                    Ok(()) => self.set_status(&format!(
                        "Exported {} account(s), {} with a cookie, to {}.",
                        summary.accounts,
                        summary.with_cookie,
                        path.display()
                    )),
                    Err(e) => self.set_status(&format!("Could not write the file: {e}")),
                }
            }
            Err(e) => self.set_status(&format!("Export failed: {e}")),
        }
    }

    /// Applies or releases the Roblox singleton hold for multi-instance.
    pub fn apply_multi_instance(&mut self) {
        if self.store.settings.multi_instance {
            if self.singleton.is_none() {
                match ram_win::multi_instance::hold() {
                    Ok(h) => {
                        if !h.is_held() {
                            self.log("multi-instance: another process already holds the Roblox singleton");
                        }
                        self.singleton = Some(h);
                    }
                    Err(e) => self.log(&format!("multi-instance unavailable: {e}")),
                }
            }
        } else {
            self.singleton = None;
        }
    }

    /// Starts an OAuth authorization in the account's browser profile and
    /// waits for the redirect on a worker thread.
    pub fn authorize(&mut self, account: AccountId) {
        let client_id = self.store.settings.oauth_client_id.trim().to_string();
        if client_id.is_empty() {
            self.set_status("Add your Roblox OAuth client ID in Settings to enable verified checks.");
            return;
        }
        if let Some(c) = self.auth_cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        let verifier = oauth::base64url(&ram_win::crypto::random_bytes(48));
        let req = AuthRequest {
            client_id,
            port: self.store.settings.oauth_port,
            state: oauth::base64url(&ram_win::crypto::random_bytes(24)),
            challenge: oauth::base64url(&ram_win::crypto::sha256(verifier.as_bytes())),
            verifier,
        };
        if let Err(e) = self.open_for(account, &req.authorize_url()) {
            self.set_status(&e);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.auth_cancel = Some(cancel.clone());
        self.checking.push(account);
        let expected = self.store.account(account).and_then(|a| a.roblox_user_id);
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let waited = oauth::wait_for_redirect(req.port, &req.state, std::time::Duration::from_secs(300), &|| {
                cancel.load(Ordering::SeqCst)
            });
            let result = match waited {
                Ok(code) => Ok(oauth::complete_authorization(&WinHttp::new(), &WinSecrets::new(), &req, &code, account, expected)),
                Err(oauth::RedirectError::Cancelled) => Err("Authorization cancelled.".to_string()),
                Err(oauth::RedirectError::TimedOut) => {
                    Err("No authorization arrived within 5 minutes. Nothing was changed.".to_string())
                }
                Err(oauth::RedirectError::Denied(why)) => Err(format!("Roblox did not grant the authorization ({why}).")),
                Err(oauth::RedirectError::Bind(why)) => {
                    Err(format!("The local redirect port could not be opened ({why}). Change it in Settings."))
                }
            };
            post_done(hwnd, CheckDone { account, result });
        });
        self.set_status("Waiting for Roblox authorization in the browser…");
        accounts::refresh_rows(self);
    }

    pub fn on_check_done(&mut self, done: CheckDone) {
        self.checking.retain(|a| *a != done.account);
        let name = self.account_name(done.account);
        match done.result {
            Ok(result) => {
                let t = now();
                if let Some(a) = self.store.account_mut(done.account) {
                    signin::apply(&mut a.sign_in, &result.outcome, t);
                    if let (CheckOutcome::Confirmed, Some(id)) = (&result.outcome, &result.identity) {
                        a.roblox_user_id = Some(id.user_id);
                        a.user_id_confirmed = true;
                        if a.username.is_empty() {
                            a.username = id.username.clone();
                        }
                    }
                    let detail = a.sign_in.detail.clone();
                    let state = a.sign_in.state;
                    self.set_status(&format!("{name}: {} \u{2014} {detail}", state.label()));
                    if signin::blocks_launch(state) {
                        self.clients.pause(done.account, "Complete verification in the browser, then resume.");
                    }
                }
                self.mark_dirty();
            }
            Err(message) => self.set_status(&format!("{name}: {message}")),
        }
        accounts::refresh_rows(self);
    }

    // ---- launching --------------------------------------------------------

    pub fn launch(&mut self, accounts: &[AccountId], target: &Target) {
        if accounts.is_empty() {
            self.set_status("Select an account to launch.");
            return;
        }
        for id in accounts {
            let state = self.store.account(*id).map(|a| a.sign_in.state).unwrap_or_default();
            self.clients.enqueue(*id, target.clone(), state);
        }
        self.pump_queue();
    }

    pub fn pump_queue(&mut self) {
        let s = &self.store.settings;
        let handoffs = self.clients.next_handoffs(now(), s.launch_concurrency, s.launch_spacing_secs);
        for h in handoffs {
            if self.start_direct_launch(&h) {
                continue;
            }
            // Browser hand-off: open the join link in the account's profile.
            let url = links::launch_url(&h.target);
            match self.open_for(h.account, &url) {
                Ok(()) => {
                    self.record_launch(h.account, &h.target);
                    self.set_status(&format!(
                        "Handed {} to the browser for {}. Waiting for a client\u{2026}",
                        links::describe(&h.target),
                        self.account_name(h.account)
                    ));
                }
                Err(e) => {
                    self.clients.handoff_failed(h.account, &e);
                    self.set_status(&e);
                }
            }
        }
        self.update_timers();
        accounts::refresh_rows(self);
        if self.view == ViewId::Destinations {
            destinations::refresh(self);
        }
    }

    /// Records a launch against the account, destination and recent history.
    fn record_launch(&mut self, account: AccountId, target: &Target) {
        let t = now();
        if let Some(a) = self.store.account_mut(account) {
            a.last_used = Some(t);
        }
        self.store.push_recent(RecentLaunch { account, target: target.clone(), at: t });
        if let Some(d) = self.store.destinations.iter_mut().find(|d| d.target == *target) {
            d.last_used = Some(t);
        }
        self.mark_dirty();
    }

    /// Starts a direct (ticket-based) launch on a worker thread when direct
    /// launch is enabled, the vault is unlocked, a cookie is stored, and the
    /// target can be served by a ticket. Returns true when that path is
    /// taken; otherwise the caller falls back to the browser.
    fn start_direct_launch(&mut self, h: &ram_core::clients::Handoff) -> bool {
        if !self.store.settings.direct_launch {
            return false;
        }
        let Some(target) = roblox::launch_target(&h.target) else {
            return false;
        };
        if !self.vault.is_unlocked() {
            return false;
        }
        let cookie = match self.vault.secret(h.account) {
            Ok(Some(c)) => c.to_string(),
            _ => return false,
        };
        self.vault.touch(now());
        let name = self.account_name(h.account);
        self.set_status(&format!("Launching {} directly for {}\u{2026}", links::describe(&h.target), name));
        let hwnd = self.hwnd.0 as isize;
        let (account, model_target) = (h.account, h.target.clone());
        std::thread::spawn(move || {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let result = roblox::prepare_launch(&WinHttp::new(), &cookie, &target, None, now_ms)
                .map(|(uri, _btid)| uri)
                .map_err(|e| launch_error_message(&e));
            post_launch_done(hwnd, LaunchDone { account, target: model_target, result });
        });
        true
    }

    pub fn on_launch_done(&mut self, done: LaunchDone) {
        let name = self.account_name(done.account);
        match done.result {
            Ok(uri) => match ram_win::launch::launch_protocol(&uri) {
                Ok(()) => {
                    self.record_launch(done.account, &done.target);
                    self.set_status(&format!("Launching {} for {}\u{2026}", links::describe(&done.target), name));
                }
                Err(e) => {
                    let msg = format!("Could not start the Roblox client: {e}.");
                    self.clients.handoff_failed(done.account, &msg);
                    self.set_status(&msg);
                }
            },
            Err(Launched::Unauthorized) => {
                // The cookie is invalid; record it and fall back to the browser.
                if let Some(a) = self.store.account_mut(done.account) {
                    signin::apply(&mut a.sign_in, &CheckOutcome::AuthorizationRejected, now());
                }
                self.set_status(&format!("{name}: the stored cookie is no longer valid. Opening the browser instead."));
                let url = links::launch_url(&done.target);
                match self.open_for(done.account, &url) {
                    Ok(()) => self.record_launch(done.account, &done.target),
                    Err(e) => {
                        self.clients.handoff_failed(done.account, &e);
                        self.set_status(&e);
                    }
                }
            }
            Err(Launched::Message(msg)) => {
                self.clients.handoff_failed(done.account, &msg);
                self.set_status(&format!("{name}: {msg}"));
            }
        }
        self.update_timers();
        accounts::refresh_rows(self);
    }

    /// Forgets the saved authorization for an account and asks Roblox to
    /// revoke it. The browser session is not affected.
    pub fn remove_authorization(&mut self, account: AccountId) {
        let secrets = WinSecrets::new();
        let name = oauth::secret_name(account);
        let client_id = self.store.settings.oauth_client_id.trim().to_string();
        if let Ok(Some(token)) = secrets.get(&name) {
            if !client_id.is_empty() {
                std::thread::spawn(move || {
                    oauth::revoke(&WinHttp::new(), &client_id, &String::from_utf8_lossy(&token));
                });
            }
        }
        let _ = secrets.delete(&name);
        if let Some(a) = self.store.account_mut(account) {
            signin::apply(&mut a.sign_in, &CheckOutcome::AuthorizationRemoved, now());
        }
        self.mark_dirty();
        self.set_status(&format!("Authorization removed for {}.", self.account_name(account)));
        accounts::refresh_rows(self);
    }

    pub fn cancel_queue(&mut self) {
        let n = self.clients.cancel_all();
        let sent = self.clients.in_flight().len();
        let mut text = format!("Cancelled {n} queued launch{}.", if n == 1 { "" } else { "es" });
        if sent > 0 {
            text.push_str(&format!(" {sent} already handed to the browser cannot be recalled."));
        }
        self.set_status(&text);
        self.update_timers();
        accounts::refresh_rows(self);
    }

    // ---- process tracking -------------------------------------------------

    /// Takes a snapshot of client processes and reconciles everything that
    /// depends on it. Triggered by process and window events, and by a slow
    /// timer only while a launch is in flight.
    pub fn rescan(&mut self) {
        let snapshot = match self.procs.list_clients() {
            Ok(v) => v,
            Err(e) => {
                self.log(&format!("process snapshot failed: {e}"));
                return;
            }
        };
        let events = self.clients.observe(now(), &snapshot, self.store.settings.launch_timeout_secs);
        if !self.links_restored {
            self.links_restored = true;
            let saved = self.store.links.clone();
            self.clients.restore_links(&saved);
        }
        let hwnd = self.hwnd.0 as isize;
        self.unwatched = false;
        for p in &snapshot {
            if !self.exit_watch.is_watching(p.key) && self.exit_watch.watch(p.key, hwnd, WM_PROC_EXIT).is_err() {
                self.unwatched = true;
            }
        }
        for ev in &events {
            match ev {
                ClientEvent::Started { account: Some(acc), .. } => {
                    let name = self.account_name(*acc);
                    self.set_status(&format!("Client running for {name} (matched by launch timing)."));
                }
                ClientEvent::Started { account: None, .. } => {}
                ClientEvent::Exited { process, account } => {
                    self.exit_watch.unwatch(*process);
                    self.samples.remove(process);
                    self.perf_notes.remove(process);
                    if let Some(acc) = account {
                        let name = self.account_name(*acc);
                        self.set_status(&format!("{name}'s client exited. Use Rejoin to launch it again."));
                    }
                }
                ClientEvent::Ambiguous { accounts } => {
                    self.set_status(&format!(
                        "A client started, but {} launches were in flight. Assign it in Performance.",
                        accounts.len()
                    ));
                }
                ClientEvent::TimedOut { account } => {
                    let name = self.account_name(*account);
                    self.set_status(&format!("No client appeared for {name}. Check the browser window."));
                }
            }
        }
        if !events.is_empty() {
            self.apply_perf();
            self.mark_dirty();
            if self.store.perf.tile_on_launch && events.iter().any(|e| matches!(e, ClientEvent::Started { account: Some(_), .. })) {
                let pairs = self.client_pairs();
                perf::tile(&mut self.store, &pairs, &self.windows);
            }
        }
        self.pump_queue();
        performance::refresh(self);
    }

    /// Applies or restores performance overrides to match the settings.
    pub fn apply_perf(&mut self) {
        let pairs = self.client_pairs();
        let notes = perf::reconcile(&mut self.store, &pairs, &self.procs, &self.jobs);
        self.perf_notes.clear();
        for n in notes {
            self.log(&format!("perf: pid {} {}", n.process.pid, n.message));
            self.perf_notes.insert(n.process, n.message);
        }
    }

    pub fn reset_perf(&mut self) {
        let notes = perf::reset_all(&mut self.store, &self.procs, &self.jobs, &self.windows);
        self.perf_notes.clear();
        if notes.is_empty() {
            self.set_status("All performance overrides were removed and original settings restored.");
        } else {
            self.set_status(&format!("{} override(s) could not be restored; see Performance.", notes.len()));
            for n in notes {
                self.perf_notes.insert(n.process, n.message);
            }
        }
        self.mark_dirty();
    }

    // ---- sampling ---------------------------------------------------------

    /// One low-rate resource sample of every client.
    pub fn sample(&mut self) {
        let procs = self.clients.processes();
        if procs.is_empty() {
            self.samples.clear();
            self.update_timers();
            return;
        }
        let visible = self.view == ViewId::Performance && unsafe { IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() };
        self.procs.set_gpu_enabled(visible);
        if visible {
            self.procs.collect_gpu();
        }
        let ticks = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() } * 10_000;
        let mut pairs = Vec::new();
        for p in &procs {
            let Ok(s) = self.procs.sample(p.key) else { continue };
            let prev = self.samples.get(&p.key).copied();
            let cpu_percent = prev.map(|pv| perf::cpu_percent(pv.sample.cpu_time, s.cpu_time, ticks.saturating_sub(pv.at_ticks), self.cpus));
            self.samples.insert(p.key, Sampled { sample: s, cpu_percent, at_ticks: ticks });
            pairs.push((p.key, s));
        }
        let t = now();
        let sys = self.procs.system_memory().ok();
        for adv in perf::advisories(&self.store.perf, &pairs, sys) {
            if self.gate.should_notify(&adv.kind, t) {
                self.set_status(&adv.message);
                if self.store.perf.notify_on_threshold {
                    self.notify("Memory advisory", &adv.message);
                }
            }
        }
        let minutes = self.store.settings.idle_reminder_minutes;
        if minutes > 0 {
            let running: Vec<ProcKey> = procs.iter().map(|p| p.key).collect();
            for key in self.idle.tick(t, &running, ram_win::window::foreground_pid(), minutes) {
                let who = self
                    .clients
                    .link_for_process(key)
                    .map(|l| self.account_name(l.account))
                    .unwrap_or_else(|| format!("client {}", key.pid));
                let text = format!("{who} has not been focused for {minutes} minutes. Roblox disconnects idle players.");
                self.set_status(&text);
                self.notify("Idle reminder", &text);
            }
        }
        if visible {
            performance::refresh(self);
        }
    }

    pub fn notify(&mut self, title: &str, body: &str) {
        if self.tray.is_none() {
            let icon = unsafe { LoadIconW(None, IDI_APPLICATION).map(|i| i.0 as isize).unwrap_or(0) };
            self.tray = Some(TrayNotifier::new(self.hwnd.0 as isize, icon, WM_TRAY, "RAM"));
        }
        if let Some(t) = &self.tray {
            t.notify(title, body);
        }
    }

    /// Runs only the timers that have work to do, so an idle manager with no
    /// clients wakes up for nothing.
    pub fn update_timers(&mut self) {
        let launching = !self.clients.in_flight().is_empty();
        let queued = self.clients.queued_len() > 0;
        let have_clients = !self.clients.processes().is_empty();
        let perf_visible = self.view == ViewId::Performance;
        unsafe {
            if launching || queued {
                SetTimer(Some(self.hwnd), TIMER_QUEUE, 1000, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_QUEUE);
            }
            // Safety net behind the window-show hook while a launch is
            // pending; off otherwise.
            if launching {
                SetTimer(Some(self.hwnd), TIMER_RESCAN, 3000, None);
            } else if self.unwatched {
                // A client that cannot be waited on is polled slowly.
                SetTimer(Some(self.hwnd), TIMER_RESCAN, 10_000, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_RESCAN);
            }
            let wants_samples = have_clients
                && (perf_visible
                    || self.store.settings.idle_reminder_minutes > 0
                    || self.store.perf.client_private_warn_mb > 0
                    || self.store.perf.system_load_warn_percent > 0);
            if wants_samples {
                let secs = if perf_visible { self.store.perf.sample_secs_visible } else { self.store.perf.sample_secs_hidden };
                SetTimer(Some(self.hwnd), TIMER_SAMPLE, secs.clamp(1, 600) * 1000, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_SAMPLE);
            }
        }
    }

    // ---- views and theme --------------------------------------------------

    /// Builds a view's controls on first use.
    fn ensure_view(&mut self, view: ViewId) {
        let i = view as usize;
        if self.created[i] {
            return;
        }
        match view {
            ViewId::Accounts => accounts::create(self),
            ViewId::Destinations => destinations::create(self),
            ViewId::Performance => performance::create(self),
            ViewId::Settings => settings::create(self),
        }
        self.created[i] = true;
        set_fonts(self.panels[i], self.m.font);
        theme::apply_to_tree(self.panels[i], &self.palette);
        for list in [self.accounts.list, self.destinations.list, self.performance.list] {
            ui::list_set_row_height(list, self.m.list_row());
        }
        self.layout();
    }

    pub fn switch_view(&mut self, view: ViewId) {
        self.ensure_view(view);
        self.view = view;
        for (i, panel) in self.panels.iter().enumerate() {
            ui::show(*panel, i == view as usize);
        }
        for h in self.nav {
            ui::invalidate(h);
        }
        match view {
            ViewId::Accounts => accounts::refresh(self),
            ViewId::Destinations => destinations::refresh(self),
            ViewId::Performance => {
                self.sample();
                performance::refresh(self)
            }
            ViewId::Settings => settings::refresh(self),
        }
        self.update_timers();
        let first = match view {
            ViewId::Accounts => self.accounts.list,
            ViewId::Destinations => self.destinations.list,
            ViewId::Performance => self.performance.list,
            ViewId::Settings => self.settings.first,
        };
        ui::focus(first);
    }

    /// Width of the left navigation sidebar.
    pub fn sidebar_w(&self) -> i32 {
        self.m.px(if self.m.compact { 150 } else { 174 })
    }

    pub fn layout(&mut self) {
        let rc = ui::client_rect(self.hwnd);
        let m = &self.m;
        let (gap, row) = (m.gap(), m.row());
        let side = self.sidebar_w();
        // Left sidebar: the app title sits above the stacked nav items.
        let item_h = row + m.px(8);
        let pad = m.px(8);
        ui::place(self.title, pad + m.px(4), m.px(14), side - 2 * pad, row);
        let mut y = m.px(16) + row;
        for h in self.nav.iter() {
            ui::place(*h, pad, y, side - 2 * pad, item_h);
            y += item_h + m.px(2);
        }
        // Content area to the right of the sidebar.
        let content_x = side;
        let status_h = row;
        ui::place(self.status, content_x + gap, rc.bottom - status_h, rc.right - content_x - 2 * gap, status_h - m.px(4));
        let top = gap;
        let (w, h) = (rc.right - content_x, rc.bottom - top - status_h);
        for p in self.panels {
            ui::place(p, content_x, top, w, h);
        }
        if self.created[0] {
            accounts::layout(self);
        }
        if self.created[1] {
            destinations::layout(self);
        }
        if self.created[2] {
            performance::layout(self);
        }
        if self.created[3] {
            settings::layout(self);
        }
    }

    /// Re-reads theme and density settings and restyles every control.
    pub fn apply_theme(&mut self) {
        self.palette = theme::palette(&self.store.settings);
        self.brushes.release();
        self.brushes = Brushes::new(&self.palette);
        let dpi = ui::dpi_of(self.hwnd);
        if dpi != self.m.dpi || self.m.compact != self.store.settings.compact {
            self.m.release();
            self.m = Metrics::new(dpi, self.store.settings.compact);
            set_fonts(self.hwnd, self.m.font);
        }
        theme::apply_to_frame(self.hwnd, &self.palette);
        theme::apply_to_tree(self.hwnd, &self.palette);
        // The sidebar title is always bold.
        ui::send(self.title, WM_SETFONT, self.m.font_bold.0 as usize, 1);
        for list in [self.accounts.list, self.destinations.list, self.performance.list] {
            ui::list_set_row_height(list, self.m.list_row());
        }
        self.layout();
        unsafe {
            let _ = RedrawWindow(Some(self.hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME);
        }
    }

    /// Handles the painting messages shared by the main window, the view
    /// panels and dialogs.
    pub fn theme_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => unsafe {
                let hdc = HDC(wparam.0 as *mut _);
                let ctl = HWND(lparam.0 as *mut _);
                let id = GetDlgCtrlID(ctl) as u16;
                SetBkMode(hdc, TRANSPARENT);
                // The title sits on the surface-coloured sidebar.
                if id == ID_TITLE {
                    SetTextColor(hdc, COLORREF(self.palette.accent));
                    return Some(LRESULT(self.brushes.surface.0 as isize));
                }
                let color = match self.static_tone(id) {
                    Tone::Text => self.palette.text,
                    Tone::Muted => self.palette.muted,
                    Tone::Good => self.palette.good,
                    Tone::Warn => self.palette.warn,
                    Tone::Bad => self.palette.bad,
                };
                SetTextColor(hdc, COLORREF(color));
                Some(LRESULT(self.brushes.window.0 as isize))
            },
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => unsafe {
                let hdc = HDC(wparam.0 as *mut _);
                SetTextColor(hdc, COLORREF(self.palette.text));
                SetBkColor(hdc, COLORREF(self.palette.surface));
                Some(LRESULT(self.brushes.surface.0 as isize))
            },
            WM_DRAWITEM => unsafe {
                let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                if dis.CtlType != ODT_BUTTON {
                    return None;
                }
                let id = dis.CtlID as u16;
                let kind = if (ID_NAV_FIRST..ID_NAV_FIRST + 4).contains(&id) {
                    ButtonKind::Nav((id - ID_NAV_FIRST) as usize == self.view as usize)
                } else if accounts::is_accent(id) || id == 1 {
                    ButtonKind::Accent
                } else {
                    ButtonKind::Normal
                };
                let font = if matches!(kind, ButtonKind::Nav(true)) { self.m.font_bold } else { self.m.font };
                let corner = self.m.px(self.store.settings.corner_radius.clamp(0, 16));
                theme::draw_button(dis, &self.palette, kind, font, corner);
                Some(LRESULT(1))
            },
            _ => None,
        }
    }

    fn static_tone(&self, id: u16) -> Tone {
        if id == ID_STATUS {
            return Tone::Muted;
        }
        if id == settings::ID_ACCENT_NOTE {
            return Tone::Warn;
        }
        accounts::static_tone(self, id)
            .or_else(|| performance::static_tone(self, id))
            .unwrap_or(if id == LABEL_MUTED { Tone::Muted } else { Tone::Text })
    }

    pub fn on_command(&mut self, id: u16, code: u16, ctl: HWND) {
        match id {
            101..=104 => self.switch_view(VIEWS[(id - ID_NAV_FIRST) as usize].0),
            1000..=1099 => accounts::command(self, id, code, ctl),
            1100..=1199 => destinations::command(self, id, code, ctl),
            1200..=1299 => performance::command(self, id, code, ctl),
            1300..=1399 => settings::command(self, id, code, ctl),
            _ => {}
        }
    }

    pub fn on_notify(&mut self, hdr: &NMHDR, lparam: LPARAM) -> Option<LRESULT> {
        match hdr.idFrom as u16 {
            1000..=1099 => accounts::notify(self, hdr, lparam),
            1100..=1199 => destinations::notify(self, hdr, lparam),
            1200..=1299 => performance::notify(self, hdr, lparam),
            _ => None,
        }
    }

    /// Restores overrides and saves; called once when the window closes.
    pub fn shutdown(&mut self) {
        if let Some(c) = self.auth_cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        // CPU limits live in job handles owned by this process; they cannot
        // be re-adopted after exit, so they are removed now. Priority and
        // efficiency changes stay in the ledger and are restored or
        // re-applied on the next start.
        for e in self.store.applied.iter_mut() {
            if e.cpu_cap {
                let _ = self.jobs.clear_cpu_cap(e.process);
                e.cpu_cap = false;
            }
        }
        self.save_now();
        self.tray = None;
        self.show_hook = None;
    }
}

/// Static control ID for secondary text.
pub const LABEL_MUTED: u16 = 0xFFFE;
/// Static control ID for ordinary labels.
pub const LABEL: u16 = 0xFFFF;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Text,
    Muted,
    Good,
    Warn,
    Bad,
}

pub fn tone_for_sign_in(state: ram_core::model::SignInState) -> Tone {
    use ram_core::model::SignInState::*;
    match state {
        Verified => Tone::Good,
        SignedOut => Tone::Bad,
        VerificationRequired | CaptchaRequired => Tone::Warn,
        CheckInBrowser | Unknown => Tone::Muted,
    }
}

pub fn tone_for_client(state: &ClientState) -> Tone {
    match state {
        ClientState::Running => Tone::Good,
        ClientState::Failed { .. } => Tone::Bad,
        ClientState::Paused { .. } | ClientState::Unconfirmed => Tone::Warn,
        ClientState::Idle | ClientState::Exited { .. } => Tone::Muted,
        ClientState::Queued | ClientState::Launching { .. } => Tone::Text,
    }
}

pub fn tone_color(p: &Palette, tone: Tone) -> u32 {
    match tone {
        Tone::Text => p.text,
        Tone::Muted => p.muted,
        Tone::Good => p.good,
        Tone::Warn => p.warn,
        Tone::Bad => p.bad,
    }
}

fn post_done(hwnd: isize, done: CheckDone) {
    let ptr = Box::into_raw(Box::new(done));
    unsafe {
        if PostMessageW(Some(HWND(hwnd as *mut _)), WM_CHECK_DONE, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
}

fn post_launch_done(hwnd: isize, done: LaunchDone) {
    let ptr = Box::into_raw(Box::new(done));
    unsafe {
        if PostMessageW(Some(HWND(hwnd as *mut _)), WM_LAUNCH_DONE, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
}

/// Applies the UI font to a window and all of its descendants.
pub fn set_fonts(root: HWND, font: HFONT) {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        ui::send(hwnd, WM_SETFONT, lparam.0 as usize, 1);
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(Some(root), Some(each), LPARAM(font.0 as isize));
    }
}

/// Human-readable target for lists: the game name plus how durable it is.
pub fn target_summary(store: &Store, target: &Target) -> String {
    match target {
        Target::Place { place_id } => store.game_name(*place_id),
        Target::PrivateServer { place_id, .. } => format!("{} \u{2014} private server", store.game_name(*place_id)),
        Target::ShareLink { link_type, .. } => format!("Share link ({link_type})"),
        Target::LiveServer { place_id, job_id } => {
            format!("{} \u{2014} server {}\u{2026}", store.game_name(*place_id), &job_id[..job_id.len().min(8)])
        }
    }
}
