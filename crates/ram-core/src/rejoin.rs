//! Auto-rejoin policy: whether a client that just ended should be launched
//! again, after how long, and whether to fall back to a simpler target.
//!
//! The rule is deliberately small. Only endings RAM did not cause and the
//! user did not ask for are retried (a server drop or a crash). Never a
//! client the user closed, never one replaced by another sign-in (rejoining
//! would kick that other session, and the two would fight forever), and
//! never a refused sign-in. Attempts are capped and spaced out so a kick or
//! a ban cannot turn into a launch loop; a client that then stays up for
//! [`RESET_AFTER_SECS`] counts as a success and resets the counter.

use crate::clientlog::EndKind;

/// A client that ran at least this long proves the previous attempts worked.
pub const RESET_AFTER_SECS: i64 = 600;
/// A rejoined client that is not in a game after this long is given up on.
pub const JOIN_TIMEOUT_SECS: i64 = 120;
/// Seconds to wait before attempt 1, 2, 3, … (the last value repeats).
pub const DELAYS: [u64; 5] = [10, 30, 90, 180, 300];

/// What to do about one ended client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// 1-based number of this attempt.
    pub attempt: u32,
    pub delay_secs: u64,
    /// Use the fallback target (the place instead of the exact server or the
    /// followed user): the exact way was already tried once and did not hold.
    pub fallback: bool,
}

/// Decides whether to rejoin. `attempts_so_far` counts the attempts made
/// since the last success; `uptime_secs` is how long the ended client ran.
pub fn plan(kind: EndKind, uptime_secs: i64, attempts_so_far: u32, max_attempts: u32) -> Option<Plan> {
    if !matches!(kind, EndKind::Disconnected | EndKind::Crashed) {
        return None;
    }
    let before = if uptime_secs >= RESET_AFTER_SECS { 0 } else { attempts_so_far };
    if before >= max_attempts {
        return None;
    }
    let delay_secs = DELAYS[(before as usize).min(DELAYS.len() - 1)];
    Some(Plan { attempt: before + 1, delay_secs, fallback: before >= 1 })
}

/// Whether attempts ran out (as opposed to the ending not being one that
/// is retried at all).
pub fn gave_up(kind: EndKind, uptime_secs: i64, attempts_so_far: u32, max_attempts: u32) -> bool {
    matches!(kind, EndKind::Disconnected | EndKind::Crashed) && max_attempts > 0 && plan(kind, uptime_secs, attempts_so_far, max_attempts).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_drops_and_crashes_are_retried() {
        for kind in [EndKind::Disconnected, EndKind::Crashed] {
            assert_eq!(plan(kind, 1200, 0, 3), Some(Plan { attempt: 1, delay_secs: 10, fallback: false }));
        }
        for kind in [EndKind::Closed, EndKind::Replaced, EndKind::SignedOut, EndKind::Teleported] {
            assert_eq!(plan(kind, 1200, 0, 3), None, "{kind:?}");
            assert!(!gave_up(kind, 1200, 9, 3));
        }
    }

    #[test]
    fn attempts_are_spaced_capped_and_fall_back() {
        // The exact way first, then the fallback, with growing waits.
        assert_eq!(plan(EndKind::Crashed, 20, 0, 3), Some(Plan { attempt: 1, delay_secs: 10, fallback: false }));
        assert_eq!(plan(EndKind::Crashed, 20, 1, 3), Some(Plan { attempt: 2, delay_secs: 30, fallback: true }));
        assert_eq!(plan(EndKind::Crashed, 20, 2, 3), Some(Plan { attempt: 3, delay_secs: 90, fallback: true }));
        assert_eq!(plan(EndKind::Crashed, 20, 3, 3), None);
        assert!(gave_up(EndKind::Crashed, 20, 3, 3));
        // The longest wait repeats.
        assert_eq!(plan(EndKind::Disconnected, 5, 8, 10).map(|p| p.delay_secs), Some(300));
        // Off.
        assert_eq!(plan(EndKind::Crashed, 20, 0, 0), None);
        assert!(!gave_up(EndKind::Crashed, 20, 0, 0));
    }

    #[test]
    fn a_long_run_resets_the_count() {
        // Three attempts were used, then the client stayed up for an hour:
        // the next drop starts over with the exact way.
        assert_eq!(plan(EndKind::Disconnected, 3600, 3, 3), Some(Plan { attempt: 1, delay_secs: 10, fallback: false }));
        assert_eq!(plan(EndKind::Disconnected, RESET_AFTER_SECS, 2, 3).map(|p| p.attempt), Some(1));
        assert_eq!(plan(EndKind::Disconnected, RESET_AFTER_SECS - 1, 2, 3).map(|p| p.attempt), Some(3));
    }
}
