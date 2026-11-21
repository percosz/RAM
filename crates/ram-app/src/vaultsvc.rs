//! Windows-backed services the core vault and exporter depend on: OS
//! randomness, DPAPI protection, and Windows Hello consent.

use ram_core::vault::{Entropy, Protector};

/// Operating-system cryptographic randomness (BCryptGenRandom).
pub struct WinEntropy;

impl Entropy for WinEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let bytes = ram_win::crypto::random_bytes(buf.len());
        buf.copy_from_slice(&bytes);
    }
}

/// DPAPI entropy that binds the vault key to this application.
const VAULT_ENTROPY: &[u8] = b"RAM.vault.dpapi.v1";

/// Protects the vault's data key with the signed-in Windows account.
pub struct WinProtector;

impl Protector for WinProtector {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::protect(data, VAULT_ENTROPY).map_err(|e| e.to_string())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::unprotect(data, VAULT_ENTROPY).map_err(|e| e.to_string())
    }
}

/// DPAPI protector for the legacy `AccountData.json` exporter, which supplies
/// its own entropy value to match the legacy format.
pub struct LegacyDpapi;

impl ram_core::export::LegacyProtector for LegacyDpapi {
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::protect(data, entropy).map_err(|e| e.to_string())
    }
}

/// Asks Windows Hello (or the device PIN) to confirm the user before a
/// secret is revealed or copied. Returns true only on a positive result.
pub fn hello_confirm(message: &str) -> Result<bool, String> {
    if !ram_win::hello::is_available() {
        return Err("Windows Hello is not set up on this PC".to_string());
    }
    ram_win::hello::verify(message).map_err(|e| e.to_string())
}
