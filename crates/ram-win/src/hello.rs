//! Windows Hello consent gate.
//!
//! Wraps the WinRT `UserConsentVerifier`, which shows the system Windows Hello
//! or device-PIN prompt and verifies the signed-in user. The OS owns the modal
//! UI; [`verify_for_window`] blocks the calling thread until the user
//! responds, so it must never run on the UI thread (the prompt needs that
//! thread's message loop; blocking it hangs the app).
//!
//! A desktop (Win32) app has to go through `IUserConsentVerifierInterop` and
//! hand over the window the prompt belongs to; the plain
//! `RequestVerificationAsync` is for apps with a CoreWindow and fails or
//! fail-fasts outside one.

use ram_core::platform::{PlatformError, PlatformResult};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows_future::IAsyncOperation;

use crate::util::map_err;

/// Cached availability: the WinRT query takes tens of milliseconds and the
/// answer does not change from one snapshot to the next.
static AVAILABLE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);
const AVAILABLE_TTL: Duration = Duration::from_secs(120);

/// True when Windows Hello (or a device PIN) can verify the user right now.
/// Any error querying the capability is reported as "not available".
pub fn is_available() -> bool {
    if let Ok(cache) = AVAILABLE.lock() {
        if let Some((v, at)) = *cache {
            if at.elapsed() < AVAILABLE_TTL {
                return v;
            }
        }
    }
    // `.join()` blocks this thread until the async query completes; in 0.62
    // it is the inherent blocking wait on a WinRT `IAsyncOperation`.
    let v = matches!(
        UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.join()),
        Ok(UserConsentVerifierAvailability::Available)
    );
    if let Ok(mut cache) = AVAILABLE.lock() {
        *cache = Some((v, Instant::now()));
    }
    v
}

fn map_result(result: UserConsentVerificationResult) -> PlatformResult<bool> {
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

/// Shows the Windows Hello / device-PIN prompt attached to `hwnd` with
/// `message` and blocks until the user responds. Call from a worker thread.
///
/// Returns `Ok(true)` when the user verified, `Ok(false)` when they cancelled
/// or ran out of retries, and `Unsupported` when the device cannot verify (not
/// present, not configured for this user, disabled by policy or busy).
pub fn verify_for_window(hwnd: isize, message: &str) -> PlatformResult<bool> {
    let interop: IUserConsentVerifierInterop =
        windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()
            .map_err(|e| map_err(&e, "UserConsentVerifier factory"))?;
    let op: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        interop.RequestVerificationForWindowAsync(HWND(hwnd as *mut _), &HSTRING::from(message))
    }
    .map_err(|e| map_err(&e, "RequestVerificationForWindow"))?;
    let result = op.join().map_err(|e| map_err(&e, "RequestVerificationForWindow"))?;
    map_result(result)
}

/// The windowless variant, for callers without a window (tests). Prefer
/// [`verify_for_window`] in the app.
pub fn verify(message: &str) -> PlatformResult<bool> {
    let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(message))
        .map_err(|e| map_err(&e, "RequestVerification"))?
        .join()
        .map_err(|e| map_err(&e, "RequestVerification"))?;
    map_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_available_returns_a_bool_without_panicking() {
        // The result depends on the machine; we only require a clean bool,
        // and the second call must come from the cache.
        let a: bool = is_available();
        let t = Instant::now();
        let b: bool = is_available();
        assert_eq!(a, b);
        assert!(t.elapsed() < Duration::from_millis(5));
    }

    // `verify` is #[ignore]d: it pops the modal Windows Hello UI and blocks on
    // a human, so it cannot run unattended in CI.
    #[test]
    #[ignore = "shows interactive Windows Hello UI"]
    fn verify_prompts_and_returns() {
        let _ = verify("RAM self-test");
    }
}
