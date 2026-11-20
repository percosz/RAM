//! Launch queue and client tracking.
//!
//! A launch is a browser hand-off: the manager opens a Roblox URL in the
//! account's browser profile and Roblox starts the client. The manager can
//! only observe that a client process appeared afterwards, so the link
//! between an account and a process is evidence-based and stays unconfirmed
//! whenever more than one explanation fits.

use crate::model::{AccountId, SignInState, Target};
use crate::platform::{ProcInfo, ProcKey};
use crate::signin;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// What the manager can say about an account's client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClientState {
    /// No launch has been requested and no client is linked.
    #[default]
    Idle,
    Queued,
    /// Waiting for the user, e.g. to finish verification in the browser.
    Paused { reason: String },
    /// The hand-off was sent; no matching client has appeared yet.
    Launching { since: i64 },
    /// A client process is linked to this account.
    Running,
    /// A client started, but it cannot be tied to this account.
    Unconfirmed,
    Exited { at: i64 },
    Failed { reason: String },
}

impl ClientState {
    pub fn label(&self) -> &'static str {
        match self {
            ClientState::Idle => "",
            ClientState::Queued => "Queued",
            ClientState::Paused { .. } => "Paused",
            ClientState::Launching { .. } => "Launching",
            ClientState::Running => "Running",
            ClientState::Unconfirmed => "Unconfirmed",
            ClientState::Exited { .. } => "Exited",
            ClientState::Failed { .. } => "Failed",
        }
    }
}

/// Why a process is linked to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Evidence {
    /// Exactly one hand-off was in flight when exactly one client appeared.
    LaunchTiming,
    /// The user assigned the client by hand.
    Manual,
}

impl Evidence {
    pub fn label(self) -> &'static str {
        match self {
            Evidence::LaunchTiming => "matched by launch timing",
            Evidence::Manual => "assigned by you",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientLink {
    pub process: ProcKey,
    pub account: AccountId,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    Started { process: ProcKey, account: Option<AccountId> },
    Exited { process: ProcKey, account: Option<AccountId> },
    /// Clients appeared that could belong to any of these accounts.
    Ambiguous { accounts: Vec<AccountId> },
    TimedOut { account: AccountId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueItem {
    account: AccountId,
    target: Target,
    paused: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub account: AccountId,
    pub target: Target,
}

#[derive(Debug, Default)]
pub struct Clients {
    primed: bool,
    known: BTreeMap<ProcKey, ProcInfo>,
    links: Vec<ClientLink>,
    states: HashMap<AccountId, ClientState>,
    last_target: HashMap<AccountId, Target>,
    queue: VecDeque<QueueItem>,
    last_handoff: Option<i64>,
}

impl Clients {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self, account: AccountId) -> ClientState {
        self.states.get(&account).cloned().unwrap_or_default()
    }

    pub fn links(&self) -> &[ClientLink] {
        &self.links
    }

    pub fn link_for_account(&self, account: AccountId) -> Option<ClientLink> {
        self.links.iter().copied().find(|l| l.account == account)
    }

    pub fn link_for_process(&self, process: ProcKey) -> Option<ClientLink> {
        self.links.iter().copied().find(|l| l.process == process)
    }

    /// Live client processes in a stable order.
    pub fn processes(&self) -> Vec<ProcInfo> {
        self.known.values().cloned().collect()
    }

    pub fn running_accounts(&self) -> Vec<AccountId> {
        self.links.iter().map(|l| l.account).collect()
    }

    /// Target of the account's most recent launch, for one-click rejoin.
    pub fn last_target(&self, account: AccountId) -> Option<&Target> {
        self.last_target.get(&account)
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    pub fn in_flight(&self) -> Vec<AccountId> {
        let mut v: Vec<AccountId> = self
            .states
            .iter()
            .filter(|(_, s)| matches!(s, ClientState::Launching { .. }))
            .map(|(id, _)| *id)
            .collect();
        v.sort_unstable();
        v
    }

    /// Adds a launch to the queue. An account whose sign-in needs attention
    /// is queued paused, so the rest of a batch continues without it.
    pub fn enqueue(&mut self, account: AccountId, target: Target, sign_in: SignInState) {
        if matches!(self.state(account), ClientState::Queued | ClientState::Launching { .. }) {
            return;
        }
        self.queue.retain(|q| q.account != account);
        let paused = signin::blocks_launch(sign_in)
            .then(|| "Complete verification in the browser, then resume.".to_string());
        self.states.insert(
            account,
            match &paused {
                Some(reason) => ClientState::Paused { reason: reason.clone() },
                None => ClientState::Queued,
            },
        );
        self.queue.push_back(QueueItem { account, target, paused });
    }

    /// Stops a queued launch because authentication needs attention.
    pub fn pause(&mut self, account: AccountId, reason: &str) {
        if let Some(item) = self.queue.iter_mut().find(|q| q.account == account) {
            item.paused = Some(reason.to_string());
            self.states.insert(account, ClientState::Paused { reason: reason.to_string() });
        }
    }

    pub fn resume(&mut self, account: AccountId) {
        if let Some(item) = self.queue.iter_mut().find(|q| q.account == account) {
            item.paused = None;
            self.states.insert(account, ClientState::Queued);
        }
    }

    /// Removes one launch that has not been handed off yet.
    pub fn cancel(&mut self, account: AccountId) -> bool {
        let before = self.queue.len();
        self.queue.retain(|q| q.account != account);
        if self.queue.len() != before {
            self.states.insert(account, ClientState::Idle);
            true
        } else {
            false
        }
    }

    /// Cancels every launch that has not been handed off yet and returns how
    /// many were cancelled. Hand-offs already sent to the browser cannot be
    /// recalled and keep their Launching state.
    pub fn cancel_all(&mut self) -> usize {
        let n = self.queue.len();
        for item in self.queue.drain(..) {
            self.states.insert(item.account, ClientState::Idle);
        }
        n
    }

    /// Launches to hand off now, respecting the concurrency limit and the
    /// spacing between hand-offs.
    pub fn next_handoffs(&mut self, now: i64, concurrency: u32, spacing_secs: u32) -> Vec<Handoff> {
        let mut out = Vec::new();
        loop {
            let in_flight = self.in_flight().len();
            if in_flight >= concurrency.max(1) as usize {
                break;
            }
            if let Some(last) = self.last_handoff {
                if now - last < spacing_secs as i64 {
                    break;
                }
            }
            let Some(pos) = self.queue.iter().position(|q| q.paused.is_none()) else { break };
            let item = self.queue.remove(pos).expect("position is in range");
            self.states.insert(item.account, ClientState::Launching { since: now });
            self.last_target.insert(item.account, item.target.clone());
            self.last_handoff = Some(now);
            out.push(Handoff { account: item.account, target: item.target });
        }
        out
    }

    /// The browser could not be started for this hand-off.
    pub fn handoff_failed(&mut self, account: AccountId, reason: &str) {
        self.states.insert(account, ClientState::Failed { reason: reason.to_string() });
    }

    /// Reconciles with a snapshot of client processes.
    ///
    /// The first snapshot only records what is already running; nothing is
    /// matched because no launch can explain those clients.
    pub fn observe(&mut self, now: i64, snapshot: &[ProcInfo], launch_timeout_secs: u32) -> Vec<ClientEvent> {
        let mut events = Vec::new();

        let gone: Vec<ProcKey> =
            self.known.keys().copied().filter(|k| !snapshot.iter().any(|p| p.key == *k)).collect();
        for key in gone {
            self.known.remove(&key);
            let account = self.link_for_process(key).map(|l| l.account);
            self.links.retain(|l| l.process != key);
            if let Some(acc) = account {
                self.states.insert(acc, ClientState::Exited { at: now });
            }
            events.push(ClientEvent::Exited { process: key, account });
        }

        let new: Vec<ProcInfo> = snapshot.iter().filter(|p| !self.known.contains_key(&p.key)).cloned().collect();
        for p in &new {
            self.known.insert(p.key, p.clone());
        }
        if !self.primed {
            self.primed = true;
            for p in &new {
                events.push(ClientEvent::Started { process: p.key, account: None });
            }
            return events;
        }

        let in_flight = self.in_flight();
        if new.len() == 1 && in_flight.len() == 1 {
            let (process, account) = (new[0].key, in_flight[0]);
            self.links.retain(|l| l.account != account);
            self.links.push(ClientLink { process, account, evidence: Evidence::LaunchTiming });
            self.states.insert(account, ClientState::Running);
            events.push(ClientEvent::Started { process, account: Some(account) });
        } else {
            for p in &new {
                events.push(ClientEvent::Started { process: p.key, account: None });
            }
            if !new.is_empty() && !in_flight.is_empty() {
                for acc in &in_flight {
                    self.states.insert(*acc, ClientState::Unconfirmed);
                }
                events.push(ClientEvent::Ambiguous { accounts: in_flight });
            }
        }

        // An unconfirmed account can only have a client while some client
        // is still unassigned.
        if self.unassigned().is_empty() {
            let stale: Vec<AccountId> = self
                .states
                .iter()
                .filter(|(_, s)| **s == ClientState::Unconfirmed)
                .map(|(id, _)| *id)
                .collect();
            for acc in stale {
                self.states.insert(acc, ClientState::Exited { at: now });
            }
        }

        let timed_out: Vec<AccountId> = self
            .states
            .iter()
            .filter(|(_, s)| matches!(s, ClientState::Launching { since } if now - since >= launch_timeout_secs as i64))
            .map(|(id, _)| *id)
            .collect();
        for acc in timed_out {
            self.states.insert(
                acc,
                ClientState::Failed {
                    reason: format!(
                        "No new client appeared within {launch_timeout_secs} s. Check the browser window."
                    ),
                },
            );
            events.push(ClientEvent::TimedOut { account: acc });
        }
        events
    }

    /// Client processes not linked to any account.
    pub fn unassigned(&self) -> Vec<ProcKey> {
        self.known.keys().copied().filter(|k| self.link_for_process(*k).is_none()).collect()
    }

    /// Links a client to an account on the user's say-so.
    pub fn assign(&mut self, process: ProcKey, account: AccountId) -> bool {
        if !self.known.contains_key(&process) {
            return false;
        }
        self.links.retain(|l| l.process != process && l.account != account);
        self.links.push(ClientLink { process, account, evidence: Evidence::Manual });
        self.states.insert(account, ClientState::Running);
        true
    }

    pub fn unassign(&mut self, process: ProcKey) {
        if let Some(link) = self.link_for_process(process) {
            self.links.retain(|l| l.process != process);
            self.states.insert(link.account, ClientState::Idle);
        }
    }

    /// Re-applies links saved by a previous run. Only links whose exact
    /// process instance is still alive are kept. Call after the first
    /// `observe`.
    pub fn restore_links(&mut self, saved: &[ClientLink]) {
        for link in saved {
            if self.known.contains_key(&link.process)
                && self.link_for_process(link.process).is_none()
                && self.link_for_account(link.account).is_none()
            {
                self.links.push(*link);
                self.states.insert(link.account, ClientState::Running);
            }
        }
    }

    /// Forgets everything about a removed account.
    pub fn forget_account(&mut self, account: AccountId) {
        self.queue.retain(|q| q.account != account);
        self.links.retain(|l| l.account != account);
        self.states.remove(&account);
        self.last_target.remove(&account);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ClientKind;

    fn proc(pid: u32) -> ProcInfo {
        ProcInfo { key: ProcKey { pid, start_time: pid as u64 * 10 }, kind: ClientKind::Desktop, exe_path: None }
    }
    fn place() -> Target {
        Target::Place { place_id: 1 }
    }
    fn primed() -> Clients {
        let mut c = Clients::new();
        c.observe(0, &[], 90);
        c
    }

    #[test]
    fn single_launch_single_new_process_is_matched() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::Unknown);
        assert_eq!(c.state(1), ClientState::Queued);
        let h = c.next_handoffs(10, 1, 8);
        assert_eq!(h.len(), 1);
        assert_eq!(c.state(1), ClientState::Launching { since: 10 });
        let ev = c.observe(15, &[proc(100)], 90);
        assert_eq!(ev, vec![ClientEvent::Started { process: proc(100).key, account: Some(1) }]);
        assert_eq!(c.state(1), ClientState::Running);
        assert_eq!(c.link_for_account(1).unwrap().evidence, Evidence::LaunchTiming);
    }

    #[test]
    fn clients_running_before_start_are_never_matched() {
        let mut c = Clients::new();
        c.observe(0, &[proc(5)], 90);
        c.enqueue(1, place(), SignInState::Unknown);
        c.next_handoffs(1, 1, 0);
        c.observe(2, &[proc(5)], 90);
        assert_eq!(c.state(1), ClientState::Launching { since: 1 });
        assert_eq!(c.unassigned(), vec![proc(5).key]);
    }

    #[test]
    fn two_in_flight_one_new_process_stays_unconfirmed() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::Unknown);
        c.enqueue(2, place(), SignInState::Unknown);
        assert_eq!(c.next_handoffs(10, 2, 0).len(), 2);
        let ev = c.observe(12, &[proc(100)], 90);
        assert!(ev.contains(&ClientEvent::Ambiguous { accounts: vec![1, 2] }));
        assert_eq!(c.state(1), ClientState::Unconfirmed);
        assert_eq!(c.state(2), ClientState::Unconfirmed);
        assert!(c.links().is_empty());
        assert_eq!(c.unassigned().len(), 1);
    }

    #[test]
    fn one_in_flight_two_new_processes_stays_unconfirmed() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::Unknown);
        c.next_handoffs(10, 1, 0);
        c.observe(12, &[proc(100), proc(101)], 90);
        assert_eq!(c.state(1), ClientState::Unconfirmed);
        assert!(c.link_for_account(1).is_none());
    }

    #[test]
    fn manual_assignment_confirms_and_exit_is_observed() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::Unknown);
        c.enqueue(2, place(), SignInState::Unknown);
        c.next_handoffs(10, 2, 0);
        c.observe(12, &[proc(100), proc(101)], 90);
        assert!(c.assign(proc(100).key, 1));
        assert_eq!(c.state(1), ClientState::Running);
        assert_eq!(c.link_for_account(1).unwrap().evidence, Evidence::Manual);
        let ev = c.observe(20, &[proc(101)], 90);
        assert_eq!(ev, vec![ClientEvent::Exited { process: proc(100).key, account: Some(1) }]);
        assert_eq!(c.state(1), ClientState::Exited { at: 20 });
        // The last unassigned client exits: the unconfirmed account has none.
        c.observe(30, &[], 90);
        assert_eq!(c.state(2), ClientState::Exited { at: 30 });
        assert!(!c.assign(proc(100).key, 2));
    }

    #[test]
    fn launch_times_out_as_failed_not_running() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::Unknown);
        c.next_handoffs(10, 1, 0);
        assert!(c.observe(99, &[], 90).is_empty());
        let ev = c.observe(100, &[], 90);
        assert_eq!(ev, vec![ClientEvent::TimedOut { account: 1 }]);
        assert!(matches!(c.state(1), ClientState::Failed { .. }));
    }

    #[test]
    fn concurrency_and_spacing_are_bounded() {
        let mut c = primed();
        for id in 1..=4 {
            c.enqueue(id, place(), SignInState::Unknown);
        }
        assert_eq!(c.next_handoffs(100, 1, 8).len(), 1);
        assert!(c.next_handoffs(101, 1, 8).is_empty(), "one in flight");
        c.observe(102, &[proc(1)], 90);
        assert!(c.next_handoffs(103, 1, 8).is_empty(), "spacing not elapsed");
        assert_eq!(c.next_handoffs(108, 1, 8).len(), 1);
        assert_eq!(c.queued_len(), 2);
    }

    #[test]
    fn cancelled_batch_leaves_no_queued_launches() {
        let mut c = primed();
        for id in 1..=5 {
            c.enqueue(id, place(), SignInState::Unknown);
        }
        c.next_handoffs(0, 1, 8);
        assert_eq!(c.cancel_all(), 4);
        assert_eq!(c.queued_len(), 0);
        assert!(c.next_handoffs(100, 4, 0).is_empty());
        for id in 2..=5 {
            assert_eq!(c.state(id), ClientState::Idle);
        }
        // The hand-off that was already sent is still tracked honestly.
        assert_eq!(c.state(1), ClientState::Launching { since: 0 });
    }

    #[test]
    fn verification_pauses_only_the_affected_launch() {
        let mut c = primed();
        c.enqueue(1, place(), SignInState::VerificationRequired);
        c.enqueue(2, place(), SignInState::Unknown);
        assert!(matches!(c.state(1), ClientState::Paused { .. }));
        let h = c.next_handoffs(0, 2, 0);
        assert_eq!(h.iter().map(|h| h.account).collect::<Vec<_>>(), vec![2]);
        c.resume(1);
        assert_eq!(c.next_handoffs(1, 2, 0).len(), 1);

        c.enqueue(3, place(), SignInState::Unknown);
        c.pause(3, "Roblox asked for verification.");
        assert!(c.next_handoffs(50, 4, 0).is_empty());
        assert!(c.cancel(3));
        assert_eq!(c.state(3), ClientState::Idle);
    }

    #[test]
    fn restored_links_require_the_same_process_instance() {
        let mut c = Clients::new();
        c.observe(0, &[proc(7)], 90);
        let stale = ClientLink { process: ProcKey { pid: 7, start_time: 1 }, account: 1, evidence: Evidence::Manual };
        let good = ClientLink { process: proc(7).key, account: 2, evidence: Evidence::LaunchTiming };
        c.restore_links(&[stale, good]);
        assert_eq!(c.state(1), ClientState::Idle);
        assert_eq!(c.state(2), ClientState::Running);
    }

    #[test]
    fn rejoin_target_is_remembered() {
        let mut c = primed();
        let t = Target::LiveServer { place_id: 9, job_id: "x".into() };
        c.enqueue(1, t.clone(), SignInState::Unknown);
        c.next_handoffs(0, 1, 0);
        assert_eq!(c.last_target(1), Some(&t));
    }
}
