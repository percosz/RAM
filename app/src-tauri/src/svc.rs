//! Windows-backed services the core vault and the importer/exporter need.

use ram_core::vault::{Entropy, Protector};

pub struct WinEntropy;

impl Entropy for WinEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let bytes = ram_win::crypto::random_bytes(buf.len());
        buf.copy_from_slice(&bytes);
    }
}

const VAULT_ENTROPY: &[u8] = b"RAM.vault.dpapi.v1";

pub struct WinProtector;

impl Protector for WinProtector {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::protect(data, VAULT_ENTROPY).map_err(|e| e.to_string())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::unprotect(data, VAULT_ENTROPY).map_err(|e| e.to_string())
    }
}

/// DPAPI for the legacy AccountData.json format (its own entropy value).
pub struct LegacyDpapi;

impl ram_core::export::LegacyProtector for LegacyDpapi {
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::protect(data, entropy).map_err(|e| e.to_string())
    }
}

impl ram_core::import::LegacyDecryptor for LegacyDpapi {
    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::unprotect(data, entropy).map_err(|e| e.to_string())
    }
}

/// Windows Hello confirmation attached to the main window; `Ok(true)` only
/// on a positive result. Blocks: call from a worker thread, never the UI
/// thread (the prompt needs the window's message loop).
pub fn hello_confirm(hwnd: isize, message: &str) -> Result<bool, String> {
    if !ram_win::hello::is_available() {
        return Err("Windows Hello is not set up on this PC".to_string());
    }
    ram_win::hello::verify_for_window(hwnd, message).map_err(|e| e.to_string())
}
