//! RAM Tauri shell: wires the Rust core to the React front-end.

mod cmd_accounts;
mod cmd_browser;
mod cmd_io;
mod cmd_launch;
mod cmd_vault;
mod dto;
mod state;
mod svc;

use state::{AppState, Shared};
use std::sync::Mutex;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage::<Shared>(Mutex::new(AppState::init()))
        .invoke_handler(tauri::generate_handler![
            cmd_accounts::app_version,
            cmd_accounts::start_hint,
            cmd_accounts::snapshot,
            cmd_accounts::add_account,
            cmd_accounts::update_account,
            cmd_accounts::remove_accounts,
            cmd_accounts::set_main,
            cmd_accounts::reorder_accounts,
            cmd_accounts::set_settings,
            cmd_accounts::open_data_folder,
            cmd_vault::vault_status,
            cmd_vault::vault_setup_dpapi,
            cmd_vault::vault_setup_password,
            cmd_vault::vault_unlock,
            cmd_vault::vault_unlock_recovery,
            cmd_vault::vault_lock,
            cmd_vault::vault_change_password,
            cmd_vault::vault_remove,
            cmd_vault::vault_options,
            cmd_vault::vault_tick,
            cmd_vault::cookie_set,
            cmd_vault::cookie_remove,
            cmd_vault::cookie_reveal,
            cmd_launch::check_accounts,
            cmd_launch::launch,
            cmd_launch::follow,
            cmd_browser::open_browser,
            cmd_browser::browser_capture,
            cmd_browser::browser_clear,
            cmd_io::import_scan,
            cmd_io::import_apply,
            cmd_io::export_accounts,
            cmd_io::clients_snapshot,
            cmd_io::close_client,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if cmd_browser::is_browser_label(window.label()) {
                    cmd_browser::on_browser_closing(window.app_handle(), window.label());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building RAM")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Ok(mut s) = app.state::<Shared>().lock() {
                    s.save();
                }
            }
        });
}
