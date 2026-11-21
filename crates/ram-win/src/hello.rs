//! Windows Hello consent gate.
//!
//! Wraps the WinRT `UserConsentVerifier`, which shows the system Windows Hello
//! or device-PIN prompt and verifies the signed-in user. The OS owns the modal
//! UI; [`verify`] blocks the calling thread until the user responds.

use ram_core::platform::{PlatformError, PlatformResult};
use windows::core::HSTRING;
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};

use crate::util::map_err;

/// True when Windows Hello (or a device PIN) can verify the user right now.
/// Any error querying the capability is reported as "not available".
pub fn is_available() -> bool {
    // `.join()` blocks this thread until the async query completes; in 0.62
    // it is the inherent blocking wait on a WinRT `IAsyncOperation`.
    matches!(
        UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.join()),
        Ok(UserConsentVerifierAvailability::Available)
    )
}

/// Shows the Windows Hello / device-PIN prompt with `message` and blocks until
/// the user responds.
///
/// Returns `Ok(true)` when the user verified, `Ok(false)` when they cancelled
/// or ran out of retries, and `Unsupported` when the device cannot verify (not
/// present, not configured for this user, disabled by policy or busy).
pub fn verify(message: &str) -> PlatformResult<bool> {
    let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(message))
        .map_err(|e| map_err(&e, "RequestVerification"))?
        .join()
        .map_err(|e| map_err(&e, "RequestVerification"))?;
    match result {
        UserConsentVerificationResult::Verified => Ok(true),
        UserConsentVerificationResult::Canceled
        | UserConsentVerificationResult::RetriesExhausted => Ok(false),
        UserConsentVerificationResult::DeviceNotPresent => {
            Err(PlatformError::Unsupported("no Windows Hello device".into()))
        }
        UserConsentVerificationResult::NotConfiguredForUser => Err(PlatformError::Unsupported(
            "Windows Hello is not set up for this user".into(),
        )),
        UserConsentVerificationResult::DisabledByPolicy => Err(PlatformError::Unsupported(
            "Windows Hello is disabled by policy".into(),
        )),
        UserConsentVerificationResult::DeviceBusy => {
            Err(PlatformError::Unsupported("the verifier device is busy".into()))
        }
        other => Err(PlatformError::Other {
            code: other.0 as u32,
            message: "unexpected verification result".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_available_returns_a_bool_without_panicking() {
        // The result depends on the machine; we only require a clean bool.
        let _available: bool = is_available();
    }

    // `verify` is #[ignore]d: it pops the modal Windows Hello UI and blocks on
    // a human, so it cannot run unattended in CI.
    #[test]
    #[ignore = "shows interactive Windows Hello UI"]
    fn verify_prompts_and_returns() {
        let _ = verify("RAM self-test");
    }
}
