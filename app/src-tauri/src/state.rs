//! Application state shared by every command, behind one mutex.

use crate::svc::WinProtector;
use ram_core::clients::Clients;
use ram_core::model::{AccountId, Store};
use ram_core::redact::Log;
use ram_core::store::{LoadOrigin, StoreError, StoreFile};
use ram_core::vault::{Vault, VaultMode};
use ram_win::{SingletonHold, WinProcesses};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppState {
    pub data_dir: PathBuf,
    pub store: Store,
    pub store_file: StoreFile,
    pub read_only: bool,
    pub startup_note: String,
    pub vault: Vault,
    pub log: Log,
    pub singleton: Option<SingletonHold>,
    pub clients: Clients,
    pub procs: WinProcesses,
    /// Accounts with a check in flight.
    pub checking: HashSet<AccountId>,
    /// Client watcher state (Diagnostics).
    pub diag: crate::cmd_diag::DiagState,
}

pub type Shared = Mutex<AppState>;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl AppState {
    /// Loads the store and vault from the data folder (`RAM_DATA_DIR` or
    /// `%LOCALAPPDATA%\RAM`), unlocking a DPAPI vault and taking the
    /// multi-instance hold when enabled.
    pub fn init() -> AppState {
        let started = std::time::Instant::now();
        let data_dir = std::env::var_os("RAM_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| ram_win::paths::local_app_data().map(|p| p.join("RAM")))
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&data_dir);
        let log = Log::new(data_dir.join("RAM.log"));
        log.info(&format!("RAM {} starting", env!("CARGO_PKG_VERSION")));
        let store_file = StoreFile::new(data_dir.join("store.json"));
        let mut startup_note = String::new();
        let mut read_only = false;
        let store = match store_file.load() {
            Ok(loaded) => {
                if let LoadOrigin::Backup { preserved_as } = &loaded.origin {
                    startup_note = format!(
                        "The data file could not be read; the last backup was loaded. The unreadable file was kept as {}.",
                        preserved_as.display()
                    );
                    log.warn(&startup_note);
                }
                loaded.store
            }
            Err(e) => {
                read_only = matches!(e, StoreError::NewerVersion { .. });
                startup_note = format!("{e}");
                log.error(&format!("store load: {e}"));
                Store::default()
            }
        };
        let vault_path = data_dir.join("vault.bin");
        let mut vault = Vault::open(&vault_path).unwrap_or_else(|e| {
            log.error(&format!("vault load: {e}"));
            Vault::new_unconfigured(&vault_path)
        });
        if vault.is_configured() && vault.mode() == Some(VaultMode::Dpapi) {
            if let Err(e) = vault.unlock_dpapi(&WinProtector, now()) {
                log.warn(&format!("vault auto-unlock: {e}"));
            }
        }
        let mut state = AppState {
            data_dir,
            store,
            store_file,
            read_only,
            startup_note,
            vault,
            log,
            singleton: None,
            clients: Clients::new(),
            procs: WinProcesses::new(),
            checking: HashSet::new(),
            diag: Default::default(),
        };
        state.apply_multi_instance();
        state.restore_client_settings();
        // Leftover browser-profile cookies are swept off the start-up path:
        // probing each profile must never delay the window.
        let dir = state.data_dir.clone();
        std::thread::spawn(move || crate::cmd_browser::sweep_profiles(&dir));
        state.log.debug(&format!("start-up: state ready in {} ms", started.elapsed().as_millis()));
        state
    }

    /// Puts back any client settings a previous run left in place (a crash
    /// mid-launch), and undoes values a client saved back after we restored.
    pub fn restore_client_settings(&mut self) {
        use ram_core::client_settings;
        let paths = ram_win::launcher_info::all_flag_files();
        for msg in client_settings::restore_leftovers(&paths) {
            self.log.warn(&format!("flag restore: {msg}"));
        }
        if let Some(undo) = self.store.xml_undo.clone() {
            match client_settings::settle(&undo.path, &undo) {
                Ok(wrote) => {
                    if wrote {
                        self.log.info("restored your own Roblox graphics settings left over from the last run");
                    }
                    self.store.xml_undo = None;
                    self.save();
                }
                Err(e) => self.log.warn(&format!("settings restore: {e}")),
            }
        }
    }

    pub fn save(&mut self) {
        if self.read_only {
            return;
        }
        self.store.links = self.clients.links().to_vec();
        if let Err(e) = self.store_file.save(&self.store) {
            self.log.error(&format!("save failed: {e}"));
        }
    }

    pub fn apply_multi_instance(&mut self) {
        if self.store.settings.multi_instance {
            if self.singleton.is_none() {
                match ram_win::multi_instance::hold() {
                    Ok(h) => {
                        if !h.is_held() {
                            self.log.warn("multi-instance: another process already holds the Roblox singleton");
                        } else {
                            self.log.info("multi-instance: holding the Roblox singleton");
                        }
                        self.singleton = Some(h);
                    }
                    Err(e) => self.log.warn(&format!("multi-instance unavailable: {e}")),
                }
            }
        } else {
            self.singleton = None;
        }
    }

    pub fn account_name(&self, id: AccountId) -> String {
        self.store.account(id).map(|a| a.display_name().to_string()).unwrap_or_else(|| format!("#{id}"))
    }

    /// The decrypted cookie for an account when the vault is unlocked.
    pub fn cookie(&mut self, id: AccountId) -> Option<String> {
        if !self.vault.is_unlocked() {
            return None;
        }
        let c = self.vault.secret(id).ok().flatten().map(|z| z.to_string());
        if c.is_some() {
            self.vault.touch(now());
        }
        c
    }
}
