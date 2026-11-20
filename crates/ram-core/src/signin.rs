//! Sign-in status rules.
//!
//! The manager never sees browser cookies, so most of what it knows is
//! second-hand. These rules keep the displayed state honest: only an explicit
//! answer from Roblox moves an account to Verified or Signed out, and a
//! failed check never changes the state at all.

use crate::model::{SignInRecord, SignInState};

/// What one check or user action established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Roblox's authorization response named this account.
    Confirmed,
    /// The authorization is valid but belongs to a different Roblox account.
    DifferentAccount { username: String },
    /// Roblox explicitly rejected the stored authorization.
    AuthorizationRejected,
    /// Roblox or the user reported that verification is needed.
    VerificationRequired { reason: String },
    /// An explicit, supported signal identified a CAPTCHA challenge.
    CaptchaRequired,
    /// A sign-in, recovery or launch page was opened in the browser.
    HandedOff { what: String },
    /// The user removed the manager's authorization for this account.
    AuthorizationRemoved,
    RateLimited { retry_after_secs: Option<u64> },
    Timeout,
    Offline,
    /// Roblox answered with an error that says nothing about the account.
    ServiceError { status: u16 },
    /// Verified checks are not set up (no OAuth client ID).
    NotConfigured,
    /// Verified checks are set up, but this account has not been authorized.
    NotAuthorized,
}

impl CheckOutcome {
    /// True for outcomes that carry no information about the account.
    pub fn is_inconclusive(&self) -> bool {
        matches!(
            self,
            CheckOutcome::RateLimited { .. }
                | CheckOutcome::Timeout
                | CheckOutcome::Offline
                | CheckOutcome::ServiceError { .. }
                | CheckOutcome::NotConfigured
                | CheckOutcome::NotAuthorized
        )
    }
}

/// Applies an outcome to a record. Inconclusive outcomes only update
/// `last_failed_check` and `detail`.
pub fn apply(record: &mut SignInRecord, outcome: &CheckOutcome, now: i64) {
    let (state, detail) = match outcome {
        CheckOutcome::Confirmed => (SignInState::Verified, "Roblox confirmed this account.".to_string()),
        CheckOutcome::DifferentAccount { username } => (
            SignInState::CheckInBrowser,
            format!("The browser profile is signed in as {username}, not this account."),
        ),
        CheckOutcome::AuthorizationRejected => (
            SignInState::SignedOut,
            "Roblox no longer accepts the saved authorization. Sign in again.".to_string(),
        ),
        CheckOutcome::VerificationRequired { reason } => (SignInState::VerificationRequired, reason.clone()),
        CheckOutcome::CaptchaRequired => (
            SignInState::CaptchaRequired,
            "Roblox asked for a CAPTCHA. Complete it in the browser.".to_string(),
        ),
        CheckOutcome::HandedOff { what } => (
            SignInState::CheckInBrowser,
            format!("{what} opened in the browser. The manager cannot see the result."),
        ),
        CheckOutcome::AuthorizationRemoved => (
            SignInState::Unknown,
            "Authorization removed from this PC. The browser session is unchanged.".to_string(),
        ),
        inconclusive => {
            record.last_failed_check = Some(now);
            record.detail = match inconclusive {
                CheckOutcome::RateLimited { retry_after_secs: Some(s) } => {
                    format!("Check not completed: Roblox rate limit, retry in {s} s.")
                }
                CheckOutcome::RateLimited { retry_after_secs: None } => {
                    "Check not completed: Roblox rate limit.".to_string()
                }
                CheckOutcome::Timeout => "Check not completed: the request timed out.".to_string(),
                CheckOutcome::Offline => "Check not completed: this PC appears to be offline.".to_string(),
                CheckOutcome::ServiceError { status } => {
                    format!("Check not completed: Roblox returned HTTP {status}.")
                }
                CheckOutcome::NotAuthorized => {
                    "Not authorized yet. Use Authorize to let Roblox confirm this account.".to_string()
                }
                _ => "Verified checks are not set up. Add an OAuth client ID in Settings.".to_string(),
            };
            return;
        }
    };
    record.state = state;
    record.detail = detail;
    record.last_check = Some(now);
    record.last_failed_check = None;
}

/// Marks an imported or newly added account: nothing is known until the user
/// signs in again in the browser.
pub fn require_fresh_sign_in(record: &mut SignInRecord) {
    record.state = SignInState::Unknown;
    record.last_check = None;
    record.last_failed_check = None;
    record.detail = "Imported without credentials. Sign in again in the browser.".to_string();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryKind {
    /// Open the Roblox sign-in page in the account's browser profile.
    OpenSignIn,
    /// Open Roblox in the browser so the user can finish a challenge.
    OpenRoblox,
    /// Run the verified check again.
    Recheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    pub kind: RecoveryKind,
    pub label: &'static str,
}

/// The one recovery action to offer for a state.
pub fn recovery(state: SignInState) -> Recovery {
    match state {
        SignInState::Verified => Recovery { kind: RecoveryKind::Recheck, label: "Check again" },
        SignInState::SignedOut | SignInState::Unknown => {
            Recovery { kind: RecoveryKind::OpenSignIn, label: "Sign in in browser" }
        }
        SignInState::VerificationRequired | SignInState::CaptchaRequired => {
            Recovery { kind: RecoveryKind::OpenRoblox, label: "Complete in browser" }
        }
        SignInState::CheckInBrowser => Recovery { kind: RecoveryKind::OpenRoblox, label: "Open Roblox in browser" },
    }
}

/// True when launches for this account must wait for the user.
pub fn blocks_launch(state: SignInState) -> bool {
    matches!(state, SignInState::VerificationRequired | SignInState::CaptchaRequired)
}

pub const SIGN_IN_URL: &str = "https://www.roblox.com/login";
pub const RECOVERY_URL: &str = "https://www.roblox.com/login/forgot-password-or-username";
pub const HOME_URL: &str = "https://www.roblox.com/home";

#[cfg(test)]
mod tests {
    use super::*;

    fn verified() -> SignInRecord {
        let mut r = SignInRecord::default();
        apply(&mut r, &CheckOutcome::Confirmed, 100);
        r
    }

    #[test]
    fn timeout_rate_limit_offline_never_become_signed_out() {
        for outcome in [
            CheckOutcome::Timeout,
            CheckOutcome::Offline,
            CheckOutcome::RateLimited { retry_after_secs: Some(30) },
            CheckOutcome::RateLimited { retry_after_secs: None },
            CheckOutcome::ServiceError { status: 503 },
            CheckOutcome::NotConfigured,
        ] {
            for start in [SignInState::Verified, SignInState::Unknown, SignInState::CheckInBrowser, SignInState::SignedOut] {
                let mut r = SignInRecord { state: start, last_check: Some(100), ..Default::default() };
                apply(&mut r, &outcome, 200);
                assert_eq!(r.state, start, "{outcome:?} changed {start:?}");
                assert_eq!(r.last_check, Some(100));
                assert_eq!(r.last_failed_check, Some(200));
                assert!(!r.detail.is_empty());
            }
        }
    }

    #[test]
    fn only_explicit_rejection_signs_out() {
        let mut r = verified();
        apply(&mut r, &CheckOutcome::AuthorizationRejected, 300);
        assert_eq!(r.state, SignInState::SignedOut);
        assert_eq!(r.last_check, Some(300));
    }

    #[test]
    fn handoff_is_check_in_browser_not_verified() {
        let mut r = SignInRecord::default();
        apply(&mut r, &CheckOutcome::HandedOff { what: "Sign-in page".into() }, 10);
        assert_eq!(r.state, SignInState::CheckInBrowser);
        assert!(r.detail.contains("cannot see"));
    }

    #[test]
    fn captcha_only_from_explicit_signal() {
        let mut r = SignInRecord::default();
        apply(&mut r, &CheckOutcome::VerificationRequired { reason: "Roblox asked for 2-step verification.".into() }, 10);
        assert_eq!(r.state, SignInState::VerificationRequired);
        apply(&mut r, &CheckOutcome::CaptchaRequired, 11);
        assert_eq!(r.state, SignInState::CaptchaRequired);
        assert!(blocks_launch(r.state));
    }

    #[test]
    fn successful_check_clears_failed_marker() {
        let mut r = SignInRecord::default();
        apply(&mut r, &CheckOutcome::Offline, 5);
        apply(&mut r, &CheckOutcome::Confirmed, 6);
        assert_eq!(r.state, SignInState::Verified);
        assert_eq!(r.last_failed_check, None);
    }

    #[test]
    fn different_account_is_not_verified() {
        let mut r = verified();
        apply(&mut r, &CheckOutcome::DifferentAccount { username: "other".into() }, 9);
        assert_eq!(r.state, SignInState::CheckInBrowser);
    }

    #[test]
    fn every_state_has_a_recovery_action() {
        for s in [
            SignInState::Verified,
            SignInState::SignedOut,
            SignInState::VerificationRequired,
            SignInState::CaptchaRequired,
            SignInState::CheckInBrowser,
            SignInState::Unknown,
        ] {
            assert!(!recovery(s).label.is_empty());
        }
    }
}
