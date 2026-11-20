//! DPAPI for the current user. Used once by the legacy importer to read the
//! user's own old data file.

use ram_core::platform::PlatformResult;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

use crate::util::{map_err, other};

fn blob(data: &[u8]) -> PlatformResult<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(data.len()).map_err(|_| other("data is too large for DPAPI"))?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        // DPAPI only reads its input blobs.
        pbData: data.as_ptr() as *mut u8,
    })
}

/// Copies a DPAPI output blob and releases it.
fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if output.pbData.is_null() {
        return Vec::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        bytes
    }
}

/// Decrypts data that was protected for the current Windows user with the
/// same `entropy`. Never shows UI. Fails for data protected by another user
/// or on another machine.
pub fn unprotect(data: &[u8], entropy: &[u8]) -> PlatformResult<Vec<u8>> {
    let input = blob(data)?;
    let entropy_blob = blob(entropy)?;
    let entropy_ptr = (!entropy.is_empty()).then_some(&entropy_blob as *const CRYPT_INTEGER_BLOB);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            entropy_ptr,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|e| map_err(&e, "CryptUnprotectData"))?;
    Ok(take(output))
}

/// Encrypts data for the current Windows user. Never shows UI.
pub fn protect(data: &[u8], entropy: &[u8]) -> PlatformResult<Vec<u8>> {
    let input = blob(data)?;
    let entropy_blob = blob(entropy)?;
    let entropy_ptr = (!entropy.is_empty()).then_some(&entropy_blob as *const CRYPT_INTEGER_BLOB);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            entropy_ptr,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|e| map_err(&e, "CryptProtectData"))?;
    Ok(take(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_and_without_entropy() {
        let secret = b"legacy data \x00\x01\x02";
        let sealed = protect(secret, b"").unwrap();
        assert_ne!(&sealed[..], &secret[..]);
        assert_eq!(unprotect(&sealed, b"").unwrap(), secret);

        let sealed = protect(secret, b"entropy").unwrap();
        assert_eq!(unprotect(&sealed, b"entropy").unwrap(), secret);
        assert!(unprotect(&sealed, b"wrong").is_err());
        assert!(unprotect(&sealed, b"").is_err());
    }

    #[test]
    fn garbage_input_is_an_error() {
        assert!(unprotect(b"not a dpapi blob", b"").is_err());
        assert!(unprotect(b"", b"").is_err());
    }
}
