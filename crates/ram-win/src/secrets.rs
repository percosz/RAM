//! Protected storage in Windows Credential Manager.

use ram_core::platform::{PlatformResult, SecretStore};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use crate::util::{code_of, map_err, other, wide};

const ERROR_NOT_FOUND: u32 = 1168;
/// `CRED_MAX_CREDENTIAL_BLOB_SIZE`.
const MAX_BLOB: usize = 5 * 512;

/// [`SecretStore`] on generic credentials named `<prefix><name>`.
///
/// Entries are encrypted by Windows for the current user, stay on this
/// machine (`CRED_PERSIST_LOCAL_MACHINE`, no roaming) and are visible in
/// Credential Manager. A value is limited to 2560 bytes.
pub struct WinSecrets {
    prefix: String,
}

impl Default for WinSecrets {
    fn default() -> Self {
        Self::new()
    }
}

impl WinSecrets {
    /// Uses target names of the form `RAM/<name>`.
    pub fn new() -> Self {
        Self::with_prefix("RAM/")
    }

    /// Uses target names of the form `<prefix><name>`.
    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
        }
    }

    fn target(&self, name: &str) -> PlatformResult<Vec<u16>> {
        if name.is_empty() || name.chars().any(|c| c.is_control()) {
            return Err(other("secret name is empty or contains control characters"));
        }
        Ok(wide(&format!("{}{}", self.prefix, name)))
    }
}

impl SecretStore for WinSecrets {
    fn get(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        let target = self.target(name)?;
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut credential) }
        {
            Ok(()) => {
                let value = unsafe {
                    let cred = &*credential;
                    let value = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(
                            cred.CredentialBlob,
                            cred.CredentialBlobSize as usize,
                        )
                        .to_vec()
                    };
                    CredFree(credential as *const _);
                    value
                };
                Ok(Some(value))
            }
            Err(e) if code_of(&e) == ERROR_NOT_FOUND => Ok(None),
            Err(e) => Err(map_err(&e, "CredRead")),
        }
    }

    fn set(&self, name: &str, value: &[u8]) -> PlatformResult<()> {
        if value.len() > MAX_BLOB {
            return Err(other(format!(
                "secret is {} bytes; Credential Manager stores at most {MAX_BLOB}",
                value.len()
            )));
        }
        let mut target = self.target(name)?;
        let mut user = wide("RAM");
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: value.len() as u32,
            // CredWrite only reads the blob.
            CredentialBlob: value.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        unsafe { CredWriteW(&credential, 0) }.map_err(|e| map_err(&e, "CredWrite"))
    }

    fn delete(&self, name: &str) -> PlatformResult<()> {
        let target = self.target(name)?;
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(e) if code_of(&e) == ERROR_NOT_FOUND => Ok(()),
            Err(e) => Err(map_err(&e, "CredDelete")),
        }
    }
}
