//! Transaction Integration for Dialog Management
//!
//! This module handles the integration between dialog-core and transaction-core,
//! managing transaction lifecycle events, request/response routing, and event processing.
//! It provides the bridge between SIP transaction reliability and dialog state management.
//!
//! ## Key Responsibilities
//!
//! - Processing transaction events and routing to appropriate dialogs
//! - Managing transaction-to-dialog associations
//! - Handling transaction completion and cleanup
//! - Converting between transaction and dialog abstractions
//! - Coordinating request sending through transaction layer

use super::core::DialogManager;
use crate::api::config::RelUsage;
use crate::dialog::{dialog_utils::extract_uri_from_contact, DialogId, DialogState};
use crate::errors::DialogResult;
use crate::events::session_coordination::tracks_generic_outbound_request_completion;
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::protocol::response_handler::response_has_auth_challenge;
use crate::transaction::builders::dialog_quick;
use crate::transaction::client::builders::ByeBuilder;
use crate::transaction::dialog::{request_builder_from_dialog_template, DialogRequestTemplate};
use crate::transaction::{TransactionEvent, TransactionKey, TransactionState};
use rvoip_infra_common::events::cross_crate::OutboundRequestOutcome;
use rvoip_sip_core::{HeaderName, Host, Method, Request, Response, TypedHeader};
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

fn safe_operation_failure(operation: &'static str, error_class: &'static str) -> String {
    format!("operation={operation}; error_class={error_class}")
}

fn safe_method_operation_failure(
    operation: &'static str,
    error_class: &'static str,
    method: &Method,
) -> String {
    format!(
        "operation={operation}; method={}; error_class={error_class}",
        crate::transaction::safe_diagnostics::SafeMethod::new(method)
    )
}

/// Validated caller-controlled portion of an initial INVITE.
///
/// Contact is planned separately because `InviteBuilder` owns the one stack
/// Contact slot. All remaining fields retain their caller-supplied order and
/// multiplicity and are appended only after stack policy headers exist.
struct InitialInviteHeaderPlan {
    contact_uri: Option<String>,
    appended: Vec<TypedHeader>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CandidateWirePlan {
    /// The request's Contact was synthesized by the stack and may be
    /// regenerated to match each resolver-selected transport candidate.
    pub regenerate_stack_default_contact: bool,
}

/// Failure from initial-INVITE dispatch with a monotonic wire-boundary
/// receipt. `ZeroWire` is safe for exact local rollback; `Unknown` must retain
/// the dialog and transaction route for CANCEL/BYE teardown.
pub enum InitialInviteSendFailure {
    ZeroWire(crate::errors::DialogError),
    Unknown(crate::errors::DialogError),
}

impl InitialInviteSendFailure {
    pub fn wire_was_attempted(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    pub fn into_dialog_error(self) -> crate::errors::DialogError {
        match self {
            Self::ZeroWire(error) | Self::Unknown(error) => error,
        }
    }
}

impl std::fmt::Debug for InitialInviteSendFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialInviteSendFailure")
            .field("wire_was_attempted", &self.wire_was_attempted())
            .finish_non_exhaustive()
    }
}

pub(crate) const INVITE_FAILOVER_PLAN_TTL: Duration = Duration::from_secs(90);
pub(crate) const INVITE_FAILOVER_PRUNE_INTERVAL: usize = 1024;
const INVITE_FAILOVER_EXPIRY_BATCH: usize = 4096;
pub(super) const INVITE_FAILOVER_EXPIRY_BUSY_RETRY: Duration = Duration::from_secs(1);
const INVITE_FAILOVER_DIALOG_CLEANUP_BATCH: usize = INVITE_FAILOVER_EXPIRY_BATCH / 2;
// Kept as an explicit policy seam so deployments can disable Timer-B and
// transport-event failover without changing 503/immediate-send behavior.
// The default is enabled only because exact-route authentication, bounded
// orphan cleanup and serialized CANCEL targeting are covered by conformance
// tests in this crate.
const INVITE_TIMEOUT_FAILOVER_ENABLED: bool = true;
const INVITE_CANDIDATE_COMPENSATION_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InviteFailoverPlanPhase {
    Active,
    /// Wire emission may have occurred and exact CANCEL/BYE teardown has not
    /// completed. This phase is non-expiring and remains capacity-charged.
    WireUnknown,
    Accepted,
    Cancelled,
    Exhausted,
    Closed,
}

impl InviteFailoverPlanPhase {
    fn has_expiry_deadline(self) -> bool {
        self != Self::WireUnknown
    }

    fn is_overflow_evictable(self) -> bool {
        !matches!(self, Self::Active | Self::WireUnknown)
    }
}

/// One versioned wake-up in the retained initial-INVITE expiry scheduler.
///
/// Ordering by deadline keeps maintenance proportional to work that is
/// actually due. The plan generation prevents a deadline captured before a
/// concurrent close/re-arm from removing the newer retained generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct InviteFailoverExpiryKey {
    wake_at: Instant,
    plan_id: u64,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InviteFailoverScheduledExpiry {
    key: InviteFailoverExpiryKey,
    overflow_evictable: bool,
}

/// One exact retry for dialog-owned failover compaction when the plan mutex
/// was busy during synchronous dialog removal.
///
/// This queue is deliberately orthogonal to protocol expiry deadlines: a
/// short cleanup retry must never replace or extend the retained late-2xx
/// horizon. The scheduler generation prevents a popped busy retry from
/// replacing a newer request, while the weak plan identity prevents a
/// wrapped/reused numeric ID from touching another plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct InviteFailoverDialogCleanupKey {
    wake_at: Instant,
    plan_id: u64,
    generation: u64,
}

#[derive(Clone)]
struct InviteFailoverScheduledDialogCleanup {
    key: InviteFailoverDialogCleanupKey,
    dialog_id: DialogId,
    plan: std::sync::Weak<tokio::sync::Mutex<InviteFailoverPlan>>,
}

/// Exact bounded scheduler for retained initial-INVITE plans.
///
/// `current` and the two ordered sets contain at most one entry per plan, so
/// repeated deadline changes cannot accumulate stale heap nodes. The explicit
/// capacity is the same as the retained-plan registry capacity. A popped entry
/// may still race with a re-arm; its generation is checked against the plan
/// before removal and `restore_after_busy` never replaces a newer entry.
pub(crate) struct InviteFailoverExpiryScheduler {
    capacity: usize,
    deadlines: BTreeSet<InviteFailoverExpiryKey>,
    overflow_evictable: BTreeSet<InviteFailoverExpiryKey>,
    current: HashMap<u64, InviteFailoverScheduledExpiry>,
    dialog_cleanup_deadlines: BTreeSet<InviteFailoverDialogCleanupKey>,
    dialog_cleanup_current: HashMap<u64, InviteFailoverScheduledDialogCleanup>,
    next_dialog_cleanup_generation: u64,
}

impl InviteFailoverExpiryScheduler {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            deadlines: BTreeSet::new(),
            overflow_evictable: BTreeSet::new(),
            // `capacity` is the logical correctness bound. Reserving the
            // complete multi-second churn horizon here made an otherwise
            // empty manager allocate for every possible retained plan.
            current: HashMap::with_capacity(capacity.min(4_096)),
            dialog_cleanup_deadlines: BTreeSet::new(),
            dialog_cleanup_current: HashMap::with_capacity(capacity.min(256)),
            next_dialog_cleanup_generation: 1,
        }
    }

    /// Diagnostic-only storage shape for retained failover scheduling.
    ///
    /// Logical protocol capacity is intentionally distinct from allocated
    /// hash capacity. Reporting both catches accidental eager reservation of
    /// the complete high-CPS retention horizon without scanning SIP payloads.
    pub(crate) fn storage_breakdown(&self) -> serde_json::Value {
        serde_json::json!({
            "logical_capacity": self.capacity,
            "deadlines": self.deadlines.len(),
            "overflow_evictable": self.overflow_evictable.len(),
            "current": self.current.len(),
            "current_hash_capacity": self.current.capacity(),
            "dialog_cleanup_deadlines": self.dialog_cleanup_deadlines.len(),
            "dialog_cleanup_current": self.dialog_cleanup_current.len(),
            "dialog_cleanup_hash_capacity": self.dialog_cleanup_current.capacity(),
            "record_inline_bytes": {
                "expiry_key": std::mem::size_of::<InviteFailoverExpiryKey>(),
                "scheduled_expiry": std::mem::size_of::<InviteFailoverScheduledExpiry>(),
                "dialog_cleanup_key": std::mem::size_of::<InviteFailoverDialogCleanupKey>(),
                "scheduled_dialog_cleanup": std::mem::size_of::<InviteFailoverScheduledDialogCleanup>(),
            },
        })
    }

    fn remove_current(&mut self, plan_id: u64) {
        let Some(previous) = self.current.remove(&plan_id) else {
            return;
        };
        self.deadlines.remove(&previous.key);
        if previous.overflow_evictable {
            self.overflow_evictable.remove(&previous.key);
        }
    }

    /// Install or replace the sole current deadline for a plan. Wire-unknown
    /// plans deliberately remove their deadline and remain capacity-charged.
    fn schedule(
        &mut self,
        plan_id: u64,
        generation: u64,
        wake_at: Instant,
        phase: InviteFailoverPlanPhase,
    ) -> bool {
        let previous = self.current.get(&plan_id).copied();
        self.remove_current(plan_id);
        if !phase.has_expiry_deadline() {
            return true;
        }
        if self.current.len() >= self.capacity {
            if let Some(previous) = previous {
                self.insert(previous);
            }
            return false;
        }
        self.insert(InviteFailoverScheduledExpiry {
            key: InviteFailoverExpiryKey {
                wake_at,
                plan_id,
                generation,
            },
            overflow_evictable: phase.is_overflow_evictable(),
        });
        true
    }

    fn insert(&mut self, expiry: InviteFailoverScheduledExpiry) {
        self.deadlines.insert(expiry.key);
        if expiry.overflow_evictable {
            self.overflow_evictable.insert(expiry.key);
        }
        self.current.insert(expiry.key.plan_id, expiry);
    }

    fn pop_due(&mut self, now: Instant, limit: usize) -> Vec<InviteFailoverScheduledExpiry> {
        let mut due = Vec::with_capacity(limit.min(self.current.len()));
        while due.len() < limit {
            let Some(key) = self.deadlines.first().copied() else {
                break;
            };
            if key.wake_at > now {
                break;
            }
            self.deadlines.remove(&key);
            let Some(current) = self.current.get(&key.plan_id).copied() else {
                continue;
            };
            if current.key != key {
                continue;
            }
            self.current.remove(&key.plan_id);
            if current.overflow_evictable {
                self.overflow_evictable.remove(&key);
            }
            due.push(current);
        }
        due
    }

    fn pop_oldest_overflow_evictable(&mut self) -> Option<InviteFailoverScheduledExpiry> {
        loop {
            let key = self.overflow_evictable.pop_first()?;
            let Some(current) = self.current.get(&key.plan_id).copied() else {
                continue;
            };
            if current.key != key || !current.overflow_evictable {
                continue;
            }
            self.current.remove(&key.plan_id);
            self.deadlines.remove(&key);
            return Some(current);
        }
    }

    /// Requeue a due entry whose plan mutex was busy, without overwriting a
    /// newer deadline installed while maintenance was attempting the lock.
    fn restore_after_busy(&mut self, mut expiry: InviteFailoverScheduledExpiry, retry_at: Instant) {
        if self.current.contains_key(&expiry.key.plan_id) || self.current.len() >= self.capacity {
            return;
        }
        expiry.key.wake_at = retry_at;
        self.insert(expiry);
    }

    fn remove_dialog_cleanup_current(&mut self, plan_id: u64) {
        let Some(previous) = self.dialog_cleanup_current.remove(&plan_id) else {
            return;
        };
        self.dialog_cleanup_deadlines.remove(&previous.key);
    }

    /// Install or replace the sole dialog-cleanup retry for one exact plan.
    pub(crate) fn schedule_dialog_cleanup(
        &mut self,
        plan_id: u64,
        dialog_id: &DialogId,
        plan: std::sync::Weak<tokio::sync::Mutex<InviteFailoverPlan>>,
        wake_at: Instant,
    ) -> bool {
        let replacing = self.dialog_cleanup_current.contains_key(&plan_id);
        if !replacing && self.dialog_cleanup_current.len() >= self.capacity {
            return false;
        }
        self.remove_dialog_cleanup_current(plan_id);
        let generation = self.next_dialog_cleanup_generation;
        self.next_dialog_cleanup_generation =
            self.next_dialog_cleanup_generation.wrapping_add(1).max(1);
        let cleanup = InviteFailoverScheduledDialogCleanup {
            key: InviteFailoverDialogCleanupKey {
                wake_at,
                plan_id,
                generation,
            },
            dialog_id: dialog_id.clone(),
            plan,
        };
        self.dialog_cleanup_deadlines.insert(cleanup.key);
        self.dialog_cleanup_current.insert(plan_id, cleanup);
        true
    }

    fn pop_due_dialog_cleanup(
        &mut self,
        now: Instant,
        limit: usize,
    ) -> Vec<InviteFailoverScheduledDialogCleanup> {
        let mut due = Vec::with_capacity(limit.min(self.dialog_cleanup_current.len()));
        while due.len() < limit {
            let Some(key) = self.dialog_cleanup_deadlines.first().copied() else {
                break;
            };
            if key.wake_at > now {
                break;
            }
            self.dialog_cleanup_deadlines.remove(&key);
            let Some(current) = self.dialog_cleanup_current.get(&key.plan_id).cloned() else {
                continue;
            };
            if current.key != key {
                continue;
            }
            self.dialog_cleanup_current.remove(&key.plan_id);
            due.push(current);
        }
        due
    }

    /// Requeue a busy cleanup without overwriting a newer exact generation.
    fn restore_dialog_cleanup_after_busy(
        &mut self,
        mut cleanup: InviteFailoverScheduledDialogCleanup,
        retry_at: Instant,
    ) -> bool {
        if self
            .dialog_cleanup_current
            .contains_key(&cleanup.key.plan_id)
        {
            return true;
        }
        if self.dialog_cleanup_current.len() >= self.capacity {
            return false;
        }
        cleanup.key.wake_at = retry_at;
        self.dialog_cleanup_deadlines.insert(cleanup.key);
        self.dialog_cleanup_current
            .insert(cleanup.key.plan_id, cleanup);
        true
    }

    fn remove(&mut self, plan_id: u64) {
        self.remove_current(plan_id);
        self.remove_dialog_cleanup_current(plan_id);
    }

    pub(crate) fn clear(&mut self) {
        self.deadlines.clear();
        self.overflow_evictable.clear();
        self.current.clear();
        self.dialog_cleanup_deadlines.clear();
        self.dialog_cleanup_current.clear();
    }

    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (
            self.current.len(),
            self.overflow_evictable.len(),
            self.dialog_cleanup_current.len(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InviteFailoverRemovalOutcome {
    Removed,
    Missing,
    Busy,
    Stale,
    NonExpiring,
    Deferred {
        wake_at: Instant,
        overflow_evictable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InviteFailoverDialogCleanupOutcome {
    Complete,
    Busy,
    Stale,
}

#[cfg(test)]
mod retained_invite_expiry_scheduler_tests {
    use super::{InviteFailoverExpiryScheduler, InviteFailoverPlan, InviteFailoverPlanPhase};
    use crate::dialog::DialogId;
    use std::sync::Weak;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;

    #[test]
    fn exact_deadline_order_only_returns_due_plans() {
        let now = Instant::now();
        let mut scheduler = InviteFailoverExpiryScheduler::new(4);
        assert!(scheduler.schedule(
            1,
            1,
            now - Duration::from_millis(1),
            InviteFailoverPlanPhase::Closed,
        ));
        assert!(scheduler.schedule(
            2,
            1,
            now + Duration::from_secs(30),
            InviteFailoverPlanPhase::Closed,
        ));

        let due = scheduler.pop_due(now, 4);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].key.plan_id, 1);
        assert_eq!(scheduler.counts(), (1, 1, 0));
        assert!(scheduler.pop_due(now, 4).is_empty());
    }

    #[test]
    fn stale_popped_deadline_cannot_replace_new_generation() {
        let now = Instant::now();
        let later = now + Duration::from_secs(30);
        let mut scheduler = InviteFailoverExpiryScheduler::new(2);
        assert!(scheduler.schedule(
            7,
            1,
            now - Duration::from_millis(1),
            InviteFailoverPlanPhase::Closed,
        ));
        let stale = scheduler.pop_due(now, 1).pop().expect("old deadline");

        assert!(scheduler.schedule(7, 2, later, InviteFailoverPlanPhase::Closed));
        scheduler.restore_after_busy(stale, now);

        assert!(scheduler.pop_due(now, 1).is_empty());
        let current = scheduler
            .pop_due(later + Duration::from_millis(1), 1)
            .pop()
            .expect("new deadline");
        assert_eq!(current.key.plan_id, 7);
        assert_eq!(current.key.generation, 2);
    }

    #[test]
    fn wire_unknown_is_unscheduled_and_capacity_stays_bounded() {
        let now = Instant::now();
        let mut scheduler = InviteFailoverExpiryScheduler::new(2);
        assert!(scheduler.schedule(
            1,
            1,
            now + Duration::from_secs(10),
            InviteFailoverPlanPhase::Closed,
        ));
        assert!(scheduler.schedule(1, 2, now, InviteFailoverPlanPhase::WireUnknown,));
        assert_eq!(scheduler.counts(), (0, 0, 0));
        assert!(scheduler
            .pop_due(now + Duration::from_secs(60), 2)
            .is_empty());

        assert!(scheduler.schedule(
            2,
            1,
            now + Duration::from_secs(20),
            InviteFailoverPlanPhase::Closed,
        ));
        assert!(scheduler.schedule(
            3,
            1,
            now + Duration::from_secs(30),
            InviteFailoverPlanPhase::Active,
        ));
        assert!(!scheduler.schedule(
            4,
            1,
            now + Duration::from_secs(40),
            InviteFailoverPlanPhase::Closed,
        ));
        assert_eq!(scheduler.counts(), (2, 1, 0));

        let overflow = scheduler
            .pop_oldest_overflow_evictable()
            .expect("closed plan is overflow-eligible");
        assert_eq!(overflow.key.plan_id, 2);
        assert!(scheduler.schedule(
            4,
            1,
            now + Duration::from_secs(40),
            InviteFailoverPlanPhase::Closed,
        ));
        assert_eq!(scheduler.counts(), (2, 1, 0));
    }

    #[test]
    fn dialog_cleanup_retry_is_exact_deduplicated_and_bounded() {
        let now = Instant::now();
        let later = now + Duration::from_secs(30);
        let dialog_id = DialogId::new();
        let plan = Weak::<Mutex<InviteFailoverPlan>>::new();
        let mut scheduler = InviteFailoverExpiryScheduler::new(1);

        assert!(scheduler.schedule_dialog_cleanup(
            7,
            &dialog_id,
            plan.clone(),
            now - Duration::from_millis(1),
        ));
        let stale = scheduler
            .pop_due_dialog_cleanup(now, 1)
            .pop()
            .expect("first cleanup generation");
        assert!(scheduler.schedule_dialog_cleanup(7, &dialog_id, plan, later));
        assert!(scheduler.restore_dialog_cleanup_after_busy(stale, now));
        assert!(scheduler.pop_due_dialog_cleanup(now, 1).is_empty());
        assert!(!scheduler.schedule_dialog_cleanup(8, &DialogId::new(), Weak::new(), now,));

        let current = scheduler
            .pop_due_dialog_cleanup(later + Duration::from_millis(1), 1)
            .pop()
            .expect("new cleanup generation");
        assert_eq!(current.key.plan_id, 7);
        assert_eq!(current.dialog_id, dialog_id);
        assert_ne!(current.key.generation, 0);
        assert_eq!(scheduler.counts(), (0, 0, 0));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InviteFailoverAttemptOutcome {
    Active,
    ImmediateTransportFailure,
    ServiceUnavailable,
    TransactionTimeout,
    TransportError,
    InternalError,
    Accepted,
    Cancelled,
    FinalResponse,
}

pub(crate) struct InviteFailoverAttempt {
    /// Shared with the exact attempt index. Retained plans therefore keep one
    /// branch allocation rather than cloning the complete transaction key
    /// into the plan, attempt map, and dialog reverse index.
    pub transaction_id: std::sync::Arc<TransactionKey>,
    pub outcome: InviteFailoverAttemptOutcome,
}

/// Heavy state needed only while an initial INVITE can still advance to a
/// resolver candidate.
///
/// Retained plans deliberately outlive the dialog/active call for late-2xx
/// ACK+BYE handling. Keeping the immutable request and resolver targets in
/// those tombstones multiplies call-setup memory by the retention window, even
/// though terminal processing uses the transaction manager's exact request.
/// An `Option` makes that lifetime boundary explicit and lets every terminal
/// transition release the payload immediately.
pub(crate) struct InviteFailoverActivePayload {
    pub request: Request,
    pub candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
    pub wire_plan: CandidateWirePlan,
    pub next_candidate_index: usize,
    /// Monotonic token assigned before candidate work leaves the plan mutex.
    /// Completion may mutate the plan only while this generation is current.
    pub next_send_generation: u64,
    pub pending_candidate: Option<(u64, usize)>,
    pub current_candidate_index: Option<usize>,
    pub current_send_generation: Option<u64>,
    pub provisional_seen: bool,
    pub attempts: Vec<InviteFailoverAttempt>,
    pub setup_deadline: Instant,
}

impl InviteFailoverActivePayload {
    pub(crate) fn new(
        request: Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        wire_plan: CandidateWirePlan,
        setup_deadline: Instant,
    ) -> Self {
        Self {
            request,
            candidates,
            wire_plan,
            next_candidate_index: 0,
            next_send_generation: 0,
            pending_candidate: None,
            current_candidate_index: None,
            current_send_generation: None,
            provisional_seen: false,
            attempts: Vec::new(),
            setup_deadline,
        }
    }
}

#[derive(Default)]
pub(crate) struct InviteFailoverForkCleanup {
    cleaned: std::collections::HashSet<String>,
    in_flight: std::collections::HashSet<String>,
}

/// One logical initial-INVITE operation. The immutable request is finalized
/// separately for every candidate so Via, Contact provenance and signatures
/// remain candidate-correct. The mutex around this record serializes 503,
/// timeout, transport-error and CANCEL races for a single dialog.
pub(crate) struct InviteFailoverPlan {
    pub id: u64,
    pub dialog_id: DialogId,
    /// The active state is boxed so a terminal tombstone does not retain the
    /// inline size of a complete SIP request, resolver plan, counters, and
    /// attempt history for the entire late-response horizon.
    pub active_payload: Option<Box<InviteFailoverActivePayload>>,
    /// Immutable amount charged to the retained-attempt capacity counter.
    /// Terminal compaction drops `active_payload.candidates`, so release must
    /// never infer the reservation from the live vector length.
    pub reserved_attempt_slots: usize,
    pub current_transaction: Option<std::sync::Arc<TransactionKey>>,
    /// Monotonic proof that this logical INVITE crossed the transaction
    /// layer's wire-attempt boundary. Once true, an error can no longer be
    /// treated as a local-only rollback.
    pub wire_attempted: bool,
    pub phase: InviteFailoverPlanPhase,
    /// Compact terminal ownership needed to remove exact attempt indexes at
    /// expiry. The Arc keys are shared with the attempt map.
    pub retained_attempts: Box<[std::sync::Arc<TransactionKey>]>,
    pub accepted_candidate_index: Option<usize>,
    pub accepted_to_tag: Option<String>,
    /// Allocated only for the exceptional late/forked-2xx cleanup path.
    pub(crate) fork_cleanup: Option<Box<InviteFailoverForkCleanup>>,
    pub expires_at: Instant,
    /// Incremented whenever `expires_at` or its expiry eligibility changes.
    /// Scheduled maintenance may remove only the exact matching generation.
    pub expiry_generation: u64,
}

impl InviteFailoverPlan {
    /// Move a plan to a non-active phase and release all active-only payload.
    /// The attempt history, selected fork identity, cleanup tags, and exact
    /// current transaction remain available for late response processing.
    pub(crate) fn transition_to_terminal(&mut self, phase: InviteFailoverPlanPhase) {
        debug_assert_ne!(phase, InviteFailoverPlanPhase::Active);
        self.phase = phase;
        if let Some(active) = self.active_payload.take() {
            self.retained_attempts = active
                .attempts
                .into_iter()
                .map(|attempt| attempt.transaction_id)
                .collect();
        }
        if !matches!(
            phase,
            InviteFailoverPlanPhase::Cancelled | InviteFailoverPlanPhase::WireUnknown
        ) {
            self.current_transaction = None;
        }
    }

    fn candidate_count(&self) -> usize {
        self.active_payload
            .as_ref()
            .map_or(0, |payload| payload.candidates.len())
    }

    fn attempt_transaction_ids(&self) -> Vec<std::sync::Arc<TransactionKey>> {
        if let Some(active) = self.active_payload.as_ref() {
            active
                .attempts
                .iter()
                .map(|attempt| attempt.transaction_id.clone())
                .collect()
        } else {
            self.retained_attempts.to_vec()
        }
    }

    fn is_cleaned_fork(&self, to_tag: &str) -> bool {
        self.fork_cleanup
            .as_ref()
            .is_some_and(|cleanup| cleanup.cleaned.contains(to_tag))
    }

    fn is_fork_cleanup_in_flight(&self, to_tag: &str) -> bool {
        self.fork_cleanup
            .as_ref()
            .is_some_and(|cleanup| cleanup.in_flight.contains(to_tag))
    }

    fn mark_fork_cleanup_in_flight(&mut self, to_tag: String) {
        self.fork_cleanup
            .get_or_insert_with(|| Box::new(InviteFailoverForkCleanup::default()))
            .in_flight
            .insert(to_tag);
    }

    fn clear_fork_cleanup_in_flight(&mut self, to_tag: &str) {
        if let Some(cleanup) = self.fork_cleanup.as_mut() {
            cleanup.in_flight.remove(to_tag);
        }
    }

    fn mark_cleaned_fork(&mut self, to_tag: String) {
        self.fork_cleanup
            .get_or_insert_with(|| Box::new(InviteFailoverForkCleanup::default()))
            .cleaned
            .insert(to_tag);
    }
}

#[cfg(test)]
mod retained_invite_payload_tests {
    use super::{
        CandidateWirePlan, InviteFailoverActivePayload, InviteFailoverAttempt,
        InviteFailoverAttemptOutcome, InviteFailoverForkCleanup, InviteFailoverPlan,
        InviteFailoverPlanPhase,
    };
    use crate::dialog::DialogId;
    use crate::transaction::TransactionKey;
    use rvoip_sip_core::{builder::SimpleRequestBuilder, Method};
    use std::time::{Duration, Instant};

    fn active_plan() -> InviteFailoverPlan {
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .expect("request builder")
            .from("Alice", "sip:alice@example.com", Some("alice-payload"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-payload")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-payload"))
            .max_forwards(70)
            .build();
        let transaction_id =
            TransactionKey::new("z9hG4bK-payload-attempt".into(), Method::Invite, false);
        let transaction_id = std::sync::Arc::new(transaction_id);
        let now = Instant::now();
        let mut active_payload = InviteFailoverActivePayload::new(
            request,
            vec![rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                "127.0.0.1:5060".parse().expect("target address"),
                rvoip_sip_transport::transport::TransportType::Udp,
            )],
            CandidateWirePlan {
                regenerate_stack_default_contact: true,
            },
            now + Duration::from_secs(32),
        );
        active_payload.next_candidate_index = 1;
        active_payload.next_send_generation = 1;
        active_payload.pending_candidate = Some((1, 0));
        active_payload.current_candidate_index = Some(0);
        active_payload.current_send_generation = Some(1);
        active_payload.attempts.push(InviteFailoverAttempt {
            transaction_id: transaction_id.clone(),
            outcome: InviteFailoverAttemptOutcome::Active,
        });
        InviteFailoverPlan {
            id: 1,
            dialog_id: DialogId::new(),
            active_payload: Some(Box::new(active_payload)),
            reserved_attempt_slots: 1,
            current_transaction: Some(transaction_id.clone()),
            wire_attempted: true,
            phase: InviteFailoverPlanPhase::Active,
            retained_attempts: Box::new([]),
            accepted_candidate_index: Some(0),
            accepted_to_tag: Some("selected-tag".into()),
            fork_cleanup: Some(Box::new(InviteFailoverForkCleanup {
                cleaned: ["cleaned-tag".into()].into_iter().collect(),
                in_flight: ["pending-tag".into()].into_iter().collect(),
            })),
            expires_at: now + Duration::from_secs(90),
            expiry_generation: 1,
        }
    }

    #[test]
    fn every_terminal_phase_drops_only_active_payload() {
        for phase in [
            InviteFailoverPlanPhase::Accepted,
            InviteFailoverPlanPhase::Cancelled,
            InviteFailoverPlanPhase::Exhausted,
            InviteFailoverPlanPhase::Closed,
            InviteFailoverPlanPhase::WireUnknown,
        ] {
            let mut plan = active_plan();
            plan.transition_to_terminal(phase);

            assert_eq!(plan.phase, phase);
            assert!(plan.active_payload.is_none());
            assert_eq!(plan.reserved_attempt_slots, 1);
            assert_eq!(plan.retained_attempts.len(), 1);
            if matches!(
                phase,
                InviteFailoverPlanPhase::Cancelled | InviteFailoverPlanPhase::WireUnknown
            ) {
                assert!(plan.current_transaction.is_some());
            } else {
                assert!(plan.current_transaction.is_none());
            }
            assert_eq!(plan.accepted_to_tag.as_deref(), Some("selected-tag"));
            assert!(plan.is_cleaned_fork("cleaned-tag"));
            assert!(plan.is_fork_cleanup_in_flight("pending-tag"));
        }
    }
}

struct InviteCandidateSendLease {
    plan_id: u64,
    dialog_id: DialogId,
    generation: u64,
    candidate_index: usize,
    target: rvoip_sip_transport::resolver::ResolvedTarget,
    request: Request,
    wire_plan: CandidateWirePlan,
    setup_deadline: Instant,
    remaining_candidate_count: usize,
}

struct InviteCandidateCancellationGuard {
    manager: DialogManager,
    plan: std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
    generation: u64,
    transaction_id: Option<TransactionKey>,
    armed: bool,
}

impl InviteCandidateCancellationGuard {
    fn new(
        manager: &DialogManager,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        generation: u64,
    ) -> Self {
        Self {
            manager: manager.clone(),
            plan: plan.clone(),
            generation,
            transaction_id: None,
            armed: true,
        }
    }

    fn set_transaction(&mut self, transaction_id: &TransactionKey) {
        self.transaction_id = Some(transaction_id.clone());
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InviteCandidateCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let transaction_id = self.transaction_id.clone();
        if let Ok(mut plan) = self.plan.try_lock() {
            let terminate = self.manager.abandon_invite_candidate_generation_locked(
                &mut plan,
                self.generation,
                transaction_id.as_ref(),
            );
            drop(plan);
            if let (Some(transaction_id), Ok(handle)) =
                (terminate, tokio::runtime::Handle::try_current())
            {
                let manager = self.manager.clone();
                if let Some(cleanup_operation) = manager.enter_invite_failover_operation() {
                    handle.spawn(async move {
                        let _cleanup_operation = cleanup_operation;
                        manager
                            .compensate_invite_candidate_transaction(&transaction_id, false)
                            .await;
                    });
                }
            }
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let manager = self.manager.clone();
            let plan = self.plan.clone();
            let generation = self.generation;
            if let Some(cleanup_operation) = manager.enter_invite_failover_operation() {
                handle.spawn(async move {
                    let _cleanup_operation = cleanup_operation;
                    manager
                        .abandon_invite_candidate_generation(
                            &plan,
                            generation,
                            transaction_id.as_ref(),
                        )
                        .await;
                });
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct InviteFailoverAttemptIndex {
    pub plan_id: u64,
    pub dialog_id: DialogId,
    pub candidate_index: usize,
    /// Exact transaction-layer admission owner. Failover tombstones may be
    /// rearmed after the transaction manager's ordinary late-2xx horizon, so
    /// the attempt index itself must fence same-wire-key reuse until its exact
    /// plan removal.
    pub(crate) _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

pub(crate) enum InviteFailoverEventDisposition {
    Continue,
    Consumed,
}

enum InviteCandidateSendError {
    Fatal(crate::errors::DialogError),
    WireUnknown(crate::errors::DialogError),
}

impl InviteCandidateSendError {
    fn into_dialog_error(self) -> crate::errors::DialogError {
        match self {
            Self::Fatal(error) | Self::WireUnknown(error) => error,
        }
    }

    fn retains_wire_owner(&self) -> bool {
        matches!(self, Self::WireUnknown(_))
    }
}

fn candidate_transport_token(
    transport: rvoip_sip_transport::transport::TransportType,
) -> &'static str {
    use rvoip_sip_transport::transport::TransportType;
    match transport {
        TransportType::Udp => "UDP",
        TransportType::Tcp => "TCP",
        TransportType::Tls => "TLS",
        TransportType::Ws => "WS",
        TransportType::Wss => "WSS",
    }
}

fn finalize_request_for_candidate(
    manager: &DialogManager,
    request: &Request,
    target: &rvoip_sip_transport::resolver::ResolvedTarget,
    wire_plan: CandidateWirePlan,
) -> DialogResult<Request> {
    use rvoip_sip_core::types::{
        address::Address,
        contact::{Contact, ContactParamInfo},
        uri::{Host, Scheme},
        Param,
    };

    let mut request = request.clone();
    let local_address = manager.local_address_for_transport(target.transport);
    let branch = (request.method() != Method::Cancel)
        .then(crate::transaction::utils::dialog_utils::generate_branch);
    let mut stamped_via = false;
    for header in &mut request.headers {
        if !header.name().wire_eq(&HeaderName::Via) {
            continue;
        }
        let TypedHeader::Via(via) = header else {
            return Err(crate::errors::DialogError::protocol_error(
                "outbound request contains an unstructured Via header",
            ));
        };
        let Some(top_via) = via.0.first_mut() else {
            return Err(crate::errors::DialogError::protocol_error(
                "outbound request has an empty Via header",
            ));
        };
        top_via.sent_protocol.transport = candidate_transport_token(target.transport).to_string();
        top_via.sent_by_host = Host::Address(local_address.ip());
        top_via.sent_by_port = Some(local_address.port());
        if let Some(branch) = branch.as_ref() {
            top_via
                .params
                .retain(|param| !matches!(param, Param::Branch(_)));
            top_via.params.push(Param::branch(branch.clone()));
        }
        stamped_via = true;
        break;
    }
    if !stamped_via {
        return Err(crate::errors::DialogError::protocol_error(
            "candidate-planned request is missing a structured Via header",
        ));
    }

    if !wire_plan.regenerate_stack_default_contact {
        return Ok(request);
    }

    let user = request
        .from()
        .and_then(|from| from.uri().user.as_ref())
        .map(ToString::to_string)
        .filter(|user| !user.is_empty())
        .unwrap_or_else(|| "user".to_string());
    let (scheme, transport_parameter) = match target.transport {
        rvoip_sip_transport::transport::TransportType::Udp => ("sip", None),
        rvoip_sip_transport::transport::TransportType::Tcp => ("sip", Some("tcp")),
        rvoip_sip_transport::transport::TransportType::Tls => ("sips", Some("tls")),
        rvoip_sip_transport::transport::TransportType::Ws => ("sip", Some("ws")),
        rvoip_sip_transport::transport::TransportType::Wss => ("sips", Some("wss")),
    };
    let suffix = transport_parameter
        .map(|transport| format!(";transport={transport}"))
        .unwrap_or_default();
    let contact_uri: rvoip_sip_core::Uri = format!("{scheme}:{user}@{local_address}{suffix}")
        .parse()
        .map_err(|_| {
            crate::errors::DialogError::protocol_error(
                "failed to plan the stack-default Contact for a candidate",
            )
        })?;
    if matches!(request.uri().scheme(), Scheme::Sips)
        && !matches!(contact_uri.scheme(), Scheme::Sips)
    {
        return Err(crate::errors::DialogError::routing_error(
            "secure request candidate produced a non-SIPS Contact",
        ));
    }
    let replacement = TypedHeader::Contact(Contact::new_params(vec![ContactParamInfo {
        address: Address::new(contact_uri),
    }]));
    let Some(contact) = request
        .headers
        .iter_mut()
        .find(|header| header.name().wire_eq(&HeaderName::Contact))
    else {
        return Err(crate::errors::DialogError::protocol_error(
            "stack-default Contact plan has no Contact header",
        ));
    };
    *contact = replacement;
    Ok(request)
}

fn planned_initial_invite_routes(
    outbound_proxy_uri: Option<&rvoip_sip_core::types::uri::Uri>,
    service_routes: &[rvoip_sip_core::types::uri::Uri],
) -> Vec<rvoip_sip_core::types::uri::Uri> {
    outbound_proxy_uri
        .into_iter()
        .cloned()
        .chain(service_routes.iter().cloned())
        .collect()
}

fn is_stack_owned_initial_invite_header(header: &TypedHeader) -> bool {
    [
        HeaderName::Via,
        HeaderName::Route,
        HeaderName::RecordRoute,
        HeaderName::Contact,
        HeaderName::From,
        HeaderName::To,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
        HeaderName::ContentLength,
        HeaderName::ContentType,
        HeaderName::SessionExpires,
        HeaderName::MinSE,
    ]
    .iter()
    .any(|name| header.name().wire_eq(name))
}

fn plan_initial_invite_headers(
    contact_override: Option<String>,
    headers: Vec<TypedHeader>,
) -> DialogResult<InitialInviteHeaderPlan> {
    let mut appended = Vec::with_capacity(headers.len());

    for header in headers {
        if is_stack_owned_initial_invite_header(&header) {
            return Err(crate::errors::DialogError::protocol_error(
                "initial INVITE application headers contain a stack-owned field",
            ));
        }
        appended.push(header);
    }

    Ok(InitialInviteHeaderPlan {
        contact_uri: contact_override,
        appended,
    })
}

fn append_validated_initial_invite_headers(
    request: &mut Request,
    authorization: Vec<TypedHeader>,
    headers: Vec<TypedHeader>,
) -> DialogResult<()> {
    for header in &authorization {
        if !matches!(
            header.name(),
            HeaderName::Authorization | HeaderName::ProxyAuthorization
        ) {
            return Err(crate::errors::DialogError::protocol_error(
                "INVITE auth retry contains a non-authorization credential header",
            ));
        }
    }
    request.headers.extend(authorization);
    request.headers.extend(headers);
    rvoip_sip_core::validation::validate_wire_request(request).map_err(|_| {
        crate::errors::DialogError::protocol_error(
            "initial INVITE failed final wire-safety validation",
        )
    })
}

/// Detect a reliable provisional response per RFC 3262.
///
/// Returns `Some(rseq)` when the response carries both `Require: 100rel`
/// and an `RSeq` header — meaning the UAC must PRACK it. Returns `None`
/// for unreliable provisionals.
pub fn detect_reliable_provisional(response: &Response) -> Option<u32> {
    use rvoip_sip_core::types::TypedHeader;

    let mut requires_100rel = false;
    let mut rseq_value: Option<u32> = None;

    for header in &response.headers {
        match header {
            TypedHeader::Require(req) if req.requires("100rel") => {
                requires_100rel = true;
            }
            TypedHeader::RSeq(rseq) => {
                rseq_value = Some(rseq.value);
            }
            _ => {}
        }
    }

    if requires_100rel {
        rseq_value
    } else {
        None
    }
}

/// Inspect a request's `Supported`/`Require` headers for the `100rel`
/// option tag. Returns `(supports, requires)` — `supports` is true when the
/// tag appears in either header (i.e., the peer has indicated 100rel
/// capability at minimum); `requires` is true only when the peer listed it
/// in `Require` (i.e., insists on it per RFC 3262 §4).
pub fn detect_peer_100rel_support(request: &Request) -> (bool, bool) {
    use rvoip_sip_core::types::TypedHeader;

    let mut supports = false;
    let mut requires = false;
    for header in &request.headers {
        match header {
            TypedHeader::Supported(sup) if sup.option_tags.iter().any(|t| t == "100rel") => {
                supports = true;
            }
            TypedHeader::Require(req) if req.requires("100rel") => {
                supports = true;
                requires = true;
            }
            _ => {}
        }
    }
    (supports, requires)
}

/// Inject the configured `100rel` option tag into an outgoing INVITE
/// (adds to existing `Supported`/`Require` headers if present).
///
/// `NotSupported` is a no-op — no header is added. `Supported` appends
/// `100rel` to any existing `Supported` header or creates one. `Required`
/// does the same for `Require`.
pub fn inject_100rel_policy(request: &mut Request, policy: RelUsage) {
    use rvoip_sip_core::types::{Require, Supported, TypedHeader};

    match policy {
        RelUsage::NotSupported => {}
        RelUsage::Supported => {
            let mut updated = false;
            for header in request.headers.iter_mut() {
                if let TypedHeader::Supported(ref mut sup) = header {
                    if !sup.option_tags.iter().any(|t| t == "100rel") {
                        sup.option_tags.push("100rel".to_string());
                    }
                    updated = true;
                    break;
                }
            }
            if !updated {
                request
                    .headers
                    .push(TypedHeader::Supported(Supported::new(vec![
                        "100rel".to_string()
                    ])));
            }
        }
        RelUsage::Required => {
            let mut updated = false;
            for header in request.headers.iter_mut() {
                if let TypedHeader::Require(ref mut req) = header {
                    if !req.requires("100rel") {
                        req.add_tag("100rel");
                    }
                    updated = true;
                    break;
                }
            }
            if !updated {
                request
                    .headers
                    .push(TypedHeader::Require(Require::with_tag("100rel")));
            }
        }
    }
}

/// Trait for transaction integration operations
pub trait TransactionIntegration {
    /// Send a request within a dialog using transaction-core
    fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> impl std::future::Future<Output = DialogResult<TransactionKey>> + Send;

    /// Send a response using transaction-core
    fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for transaction helper operations
pub trait TransactionHelpers {
    /// Associate a transaction with a dialog
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId);

    /// Create ACK for 2xx response using transaction-core helpers
    fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<Request>> + Send;
}

// Actual implementations for DialogManager
impl TransactionIntegration for DialogManager {
    /// Send a request within a dialog using transaction-core
    ///
    /// Implements proper request creation within dialogs using Phase 3 dialog functions
    /// for significantly simplified and more maintainable code.
    async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        self.send_request_in_dialog_with_extras(dialog_id, method, body, Vec::new())
            .await
    }

    /// Send a response using transaction-core
    async fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        Self::send_transaction_response_impl(self, transaction_id, response).await
    }
}

impl DialogManager {
    /// SIP_API_DESIGN_2 §7.2 — in-dialog request dispatch with
    /// application-staged `extra_headers` appended after the
    /// stack-managed slice. Used by every `send_*_with_options` path on
    /// `UnifiedDialogApi`. The legacy `send_request_in_dialog` (no
    /// extras) forwards to this with an empty Vec.
    pub async fn send_request_in_dialog_with_extras(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<TransactionKey> {
        self.send_request_in_dialog_with_extras_owned(dialog_id, method, body, extra_headers)
            .await
            .map(|(transaction_id, _completion)| transaction_id)
    }

    /// In-dialog request dispatch that returns the exact transaction
    /// completion captured before the request can reach the wire.
    pub(crate) async fn send_request_in_dialog_with_extras_and_completion(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<(
        TransactionKey,
        crate::transaction::ClientTransactionCompletionHandle,
    )> {
        let method_for_error = method.clone();
        let (transaction_id, completion) = self
            .send_request_in_dialog_with_extras_owned(dialog_id, method, body, extra_headers)
            .await?;
        let completion =
            completion.ok_or_else(|| crate::errors::DialogError::TransactionError {
                message: safe_method_operation_failure(
                    "dialog_completion_capture",
                    "missing_exact_completion",
                    &method_for_error,
                ),
            })?;
        Ok((transaction_id, completion))
    }

    async fn send_request_in_dialog_with_extras_owned(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<(
        TransactionKey,
        Option<crate::transaction::ClientTransactionCompletionHandle>,
    )> {
        debug!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method), dialog=%dialog_id, "Sending request using dialog functions");

        // Get dialog context and build the request. Destination is resolved
        // from the final request next hop after Route headers are present.
        let (candidates, request, wire_plan) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            // Convert body to String if provided
            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            // Create dialog template using the proper dialog method
            let template = dialog.create_request_template(method.clone());

            // Capture INVITE CSeq for later use by RAck (RFC 3262 §7.2). Applies
            // to both initial INVITE and re-INVITE — a re-INVITE can also produce
            // reliable provisionals, so the most recent INVITE CSeq is what counts.
            if method == Method::Invite {
                dialog.invite_cseq = Some(template.cseq_number);
            }

            // Read dialog-scoped fields needed by per-method request builders
            // BEFORE entering the match — the DashMap write lock held by
            // `dialog` would otherwise deadlock on any `self.get_dialog()` call
            // inside an arm (hit us on NOTIFY, which reads event_package +
            // subscription_state).
            let notify_event_package = dialog
                .event_package
                .clone()
                .unwrap_or_else(|| "dialog".to_string());
            let notify_subscription_state = dialog
                .subscription_state
                .as_ref()
                .map(|s| s.to_header_value());

            // Generate local tag if missing (for outgoing requests we should always have a local tag)
            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            // Handle remote tag based on dialog state and method
            let remote_tag = match (&template.remote_tag, dialog.state.clone()) {
                // If we have a valid remote tag, use it
                (Some(tag), _) if !tag.is_empty() => Some(tag.clone()),

                // For certain methods in confirmed dialogs, remote tag is required
                (_, crate::dialog::DialogState::Confirmed) => {
                    error!(
                        dialog=%dialog_id,
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method),
                        has_local_tag=dialog.local_tag.is_some(),
                        has_remote_tag=dialog.remote_tag.is_some(),
                        "Confirmed dialog is missing remote tag for request"
                    );
                    return Err(crate::errors::DialogError::protocol_error(
                        &safe_method_operation_failure(
                            "dialog_request",
                            "missing_remote_tag",
                            &method,
                        ),
                    ));
                }

                // For early/initial dialogs, remote tag may be None (will be set to None, not empty string)
                _ => None,
            };

            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set);

            // SIP_API_DESIGN_2 §7.2 — applications stage headers on the
            // builder; the dialog stack stamps Call-ID/CSeq/Via/From-tag
            // and appends application extras after that fixed prefix.
            // Empty Vec means legacy path (no extras to stamp).
            let extras_opt: Option<Vec<rvoip_sip_core::types::TypedHeader>> =
                if extra_headers.is_empty() {
                    None
                } else {
                    Some(extra_headers.clone())
                };

            // Build request using Phase 3 dialog quick functions (MUCH simpler!)
            let request = match method {
                Method::Invite => {
                    // Distinguish between initial INVITE and re-INVITE based on remote tag
                    match remote_tag {
                        Some(remote_tag) => {
                            // re-INVITE: We have a remote tag, so this is for an established dialog
                            // re-INVITE requires SDP content for session modification
                            let sdp_content = body_string.ok_or_else(|| {
                                crate::errors::DialogError::protocol_error("re-INVITE request requires SDP content for session modification")
                            })?;

                            dialog_quick::reinvite_for_dialog_with_extras(
                                &template.call_id,
                                &template.local_uri.to_string(),
                                &local_tag,
                                &template.remote_uri.to_string(),
                                &remote_tag,
                                &sdp_content,
                                template.cseq_number,
                                local_address,
                                if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                                self.local_contact_uri(),
                                extras_opt.clone(),
                            )
                        },
                        None => {
                            // Initial INVITE: No remote tag yet, creating new dialog
                            use crate::transaction::client::builders::InviteBuilder;

                            let mut invite_builder = InviteBuilder::new()
                                .from_detailed(
                                    Some("User"), // Display name
                                    template.local_uri.to_string(),
                                    Some(&local_tag)
                                )
                                .to_detailed(
                                    Some("User"), // Display name
                                    template.remote_uri.to_string(),
                                    None // No remote tag for initial INVITE
                                )
                                .call_id(&template.call_id)
                                .cseq(template.cseq_number)
                                .request_uri(template.target_uri.to_string())
                                .local_address(local_address);

                            // Add route set if present
                            for route in &template.route_set {
                                invite_builder = invite_builder.add_route(route.clone());
                            }

                            if let Some(contact) = self.local_contact_uri() {
                                invite_builder = invite_builder.contact(contact);
                            }

                            // Add SDP content if provided
                            if let Some(sdp_content) = body_string {
                                invite_builder = invite_builder.with_sdp(sdp_content);
                            }

                            // SIP_API_DESIGN_2 §5.2 — extras after stack-managed prefix.
                            invite_builder.build().map(|mut request| {
                                for hdr in extra_headers.iter().cloned() {
                                    request.headers.push(hdr);
                                }
                                request
                            })
                        }
                    }
                },

                Method::Bye => {
                    // BYE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("BYE request requires remote tag in established dialog")
                    })?;

                    dialog_quick::bye_for_dialog_with_request_uri(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &template.target_uri.to_string(),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Refer => {
                    // REFER requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("REFER request requires remote tag in established dialog")
                    })?;

                    // Extract the target URI from the body if it's in the old format ("Refer-To: <uri>")
                    // Otherwise use it directly as the target URI
                    let target_uri = if let Some(body) = body_string.clone() {
                        // Check if it's in the old format with "Refer-To: " prefix
                        if body.starts_with("Refer-To: ") {
                            body.trim_start_matches("Refer-To: ").trim_end_matches("\r\n").to_string()
                        } else {
                            body
                        }
                    } else {
                        "sip:unknown".to_string()
                    };

                    dialog_quick::refer_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &target_uri,
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        self.local_contact_uri(),
                        extras_opt.clone(),
                    )
                },

                Method::Update => {
                    // UPDATE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("UPDATE request requires remote tag in established dialog")
                    })?;

                    dialog_quick::update_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        body_string.clone(),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        self.local_contact_uri(),
                        extras_opt.clone(),
                    )
                },

                Method::Info => {
                    // INFO requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("INFO request requires remote tag in established dialog")
                    })?;

                    let content = body_string.unwrap_or_else(|| "Application info".to_string());
                    dialog_quick::info_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("application/info".to_string()),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Notify => {
                    // NOTIFY requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("NOTIFY request requires remote tag in established dialog")
                    })?;

                    dialog_quick::notify_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &notify_event_package,
                        body_string,
                        notify_subscription_state.clone(),
                        None, // content_type — legacy path infers from event package
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Message => {
                    // MESSAGE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("MESSAGE request requires remote tag in established dialog")
                    })?;

                    let content = body_string.unwrap_or_else(|| "".to_string());
                    dialog_quick::message_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("text/plain".to_string()),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                _ => {
                    // For any other method, require established dialog
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error(
                            &safe_method_operation_failure(
                                "dialog_request",
                                "missing_remote_tag",
                                &method,
                            ),
                        )
                    })?;

                    let contact = if matches!(method, Method::Update | Method::Refer | Method::Subscribe | Method::Notify) {
                        self.local_contact_uri()
                    } else {
                        None
                    };

                    // Use dialog template + utility function
                    let template_struct = DialogRequestTemplate {
                        call_id: template.call_id,
                        from_uri: template.local_uri.to_string(),
                        from_tag: local_tag,
                        to_uri: template.remote_uri.to_string(),
                        to_tag: remote_tag,
                        request_uri: template.target_uri.to_string(),
                        cseq: template.cseq_number,
                        local_address,
                        route_set: template.route_set.clone(),
                        contact,
                    };

                    request_builder_from_dialog_template(
                        &template_struct,
                        method.clone(),
                        body_string,
                        None, // Auto-detect content type
                        extras_opt.clone(),
                    )
                }
            }.map_err(|_error| crate::errors::DialogError::InternalError {
                message: safe_method_operation_failure(
                    "dialog_request_build",
                    "builder_error",
                    &method,
                ),
                context: None,
            })?;

            let mut request = request;
            // RFC 3262: advertise or demand the `100rel` extension on outgoing
            // INVITEs per dialog config. Applies to both initial and re-INVITE.
            if method == Method::Invite {
                inject_100rel_policy(&mut request, self.config_100rel_policy());
                // RFC 4028: advertise session timers. Only emitted when the
                // config has `session_timer_secs = Some(_)`.
                if let Some((secs, min_se)) = self.config_session_timer_settings() {
                    inject_session_timer_headers(&mut request, secs, min_se);
                }
            }

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    crate::errors::DialogError::routing_error(
                        "Outbound request contains an unusable Route header",
                    )
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;

            if candidates.is_empty() {
                return Err(crate::errors::DialogError::routing_error(
                    "No address candidates for the exact request next hop",
                ));
            }
            let wire_plan = CandidateWirePlan {
                regenerate_stack_default_contact: request.header(&HeaderName::Contact).is_some()
                    && self.local_contact_uri().is_none(),
            };
            (candidates, request, wire_plan)
        };

        // RFC 3263 §4.3 multi-candidate failover. STIR/SHAKEN signing
        // (`pre_send_request`) and the RFC 3261 §17.1.1.3 benign-
        // terminate-after-2xx handling both live in the helper — only
        // INVITE gets the benign-terminate suppression so non-INVITE
        // methods (BYE / REFER / UPDATE / etc.) still surface real
        // transport failures.
        let (transaction_id, _addr, completion) = self
            .send_request_with_candidate_wire_plan_owned(
                request,
                candidates,
                Some(dialog_id),
                wire_plan,
            )
            .await?;

        debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method),
            dialog=%dialog_id,
            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id),
            "Sent request via candidate failover path"
        );

        Ok((transaction_id, completion))
    }

    /// Send a response using transaction-core
    ///
    /// Delegates response sending to transaction-core while maintaining dialog state.
    /// Reliable-provisional wrapping (RFC 3262 §3) is applied here: a 1xx
    /// response with a body on a dialog whose peer advertised `100rel` is
    /// rewritten with `Require: 100rel` + `RSeq: <n>` and retransmitted with
    /// T1 backoff until PRACK acknowledges it.
    pub async fn send_transaction_response_impl(
        &self,
        transaction_id: &TransactionKey,
        mut response: Response,
    ) -> DialogResult<()> {
        debug!(status=response.status_code(), transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Sending response");

        // RFC 4028: echo Session-Expires on 2xx to INVITE so the UAC learns
        // the negotiated interval + refresher assignment.
        if response.status_code() == 200 {
            if let Some(dialog_id_ref) = self.transaction_to_dialog.get(transaction_id) {
                let dialog_id = dialog_id_ref.clone();
                drop(dialog_id_ref);
                if let Ok(dialog) = self.get_dialog(&dialog_id) {
                    if let Some(secs) = dialog.session_expires_secs {
                        let refresher = if dialog.is_session_refresher {
                            rvoip_sip_core::types::session_expires::Refresher::Uas
                        } else {
                            rvoip_sip_core::types::session_expires::Refresher::Uac
                        };
                        let already_has = response.headers.iter().any(|h| {
                            matches!(h, rvoip_sip_core::types::TypedHeader::SessionExpires(_))
                        });
                        if !already_has {
                            response.headers.push(
                                rvoip_sip_core::types::TypedHeader::SessionExpires(
                                    rvoip_sip_core::types::session_expires::SessionExpires::new(
                                        secs,
                                        Some(refresher),
                                    ),
                                ),
                            );
                        }
                        let supports_has_timer = response.headers.iter().any(|h| matches!(h, rvoip_sip_core::types::TypedHeader::Require(r) if r.requires("timer")));
                        if !supports_has_timer {
                            response
                                .headers
                                .push(rvoip_sip_core::types::TypedHeader::Require(
                                    rvoip_sip_core::types::Require::with_tag("timer"),
                                ));
                        }
                    }
                }
            }
        }

        let mut reliable_prepared = None;
        if should_send_reliably(&response) {
            if let Some(dialog_id_ref) = self.transaction_to_dialog.get(transaction_id) {
                let dialog_id = dialog_id_ref.clone();
                drop(dialog_id_ref);

                let our_policy = self.config_100rel_policy();
                let rseq_opt = match self.get_dialog_mut(&dialog_id) {
                    Ok(mut dialog) => {
                        if dialog.state != DialogState::Terminated
                            && dialog.peer_supports_100rel
                            && !matches!(our_policy, RelUsage::NotSupported)
                        {
                            Some(dialog.next_local_rseq())
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                };

                if let Some(rseq) = rseq_opt {
                    inject_reliable_provisional_headers(&mut response, rseq);
                    reliable_prepared = Some(
                        crate::transaction::server::reliable_invite::prepare_reliable_provisional_retransmit(
                            dialog_id.clone(),
                            rseq,
                            transaction_id.clone(),
                            response.clone(),
                            self.transaction_manager.clone(),
                            self.reliable_provisional_tasks.clone(),
                            || {
                                self.get_dialog(&dialog_id)
                                    .map(|dialog| dialog.state != DialogState::Terminated)
                                    .unwrap_or(false)
                            },
                        )
                        .map_err(|_error| crate::errors::DialogError::InternalError {
                            message: safe_operation_failure(
                                "reliable_provisional_reserve",
                                "lifecycle_error",
                            ),
                            context: None,
                        })?,
                    );
                    debug!(
                        "Wrapping 18x {} as reliable (policy={:?}, rseq={})",
                        response.status_code(),
                        our_policy,
                        rseq
                    );
                }
            }
        }

        // A final response closes the RFC 3262 provisional phase before its
        // own wire write. No reliable 18x task can therefore retransmit after
        // the final response.
        let final_invite_dialog =
            if !response.status().is_provisional() && transaction_id.method() == &Method::Invite {
                self.transaction_to_dialog
                    .get(transaction_id)
                    .map(|entry| entry.value().clone())
            } else {
                None
            };
        if let Some(dialog_id) = final_invite_dialog.as_ref() {
            self.reliable_provisional_tasks
                .close_transaction(dialog_id, transaction_id)
                .await
                .map_err(|_error| crate::errors::DialogError::InternalError {
                    message: safe_operation_failure(
                        "reliable_provisional_close",
                        "lifecycle_error",
                    ),
                    context: None,
                })?;
        }

        // Use transaction-core to send the response
        if let Err(_error) = self
            .transaction_manager
            .send_response(transaction_id, response)
            .await
        {
            if let Some(prepared) = reliable_prepared {
                prepared.cancel().await.map_err(|_error| {
                    crate::errors::DialogError::InternalError {
                        message: safe_operation_failure(
                            "reliable_provisional_cancel",
                            "lifecycle_error",
                        ),
                        context: None,
                    }
                })?;
            }
            return Err(crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("transaction_response_send", "transaction_error"),
            });
        }

        if let Some(prepared) = reliable_prepared {
            prepared.activate().await.map_err(|_error| {
                crate::errors::DialogError::InternalError {
                    message: safe_operation_failure(
                        "reliable_provisional_activate",
                        "lifecycle_error",
                    ),
                    context: None,
                }
            })?;
        }
        if let Some(dialog_id) = final_invite_dialog.as_ref() {
            self.reliable_provisional_tasks
                .release_transaction(dialog_id, transaction_id);
        }

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Successfully sent response");
        Ok(())
    }
}

#[cfg(test)]
mod outward_error_redaction_tests {
    use super::*;
    use rvoip_sip_core::HeaderValue;
    use std::str::FromStr;

    const LOWER_ERROR_SECRET: &str = "lower-builder-parser-secret";
    const EXTENSION_METHOD_SECRET: &str = "extension-method-secret";

    #[test]
    fn constructed_outward_errors_keep_only_operation_class_and_safe_method() {
        let lower_result: Result<(), &str> = Err(LOWER_ERROR_SECRET);
        let method = Method::Extension(EXTENSION_METHOD_SECRET.to_string());
        let error = lower_result
            .map_err(|_error| crate::errors::DialogError::InternalError {
                message: safe_method_operation_failure("request_build", "builder_error", &method),
                context: None,
            })
            .unwrap_err();
        let rendered = format!("{error:?}");

        let crate::errors::DialogError::InternalError { message, .. } = &error else {
            panic!("internal error expected");
        };
        assert!(message.contains("operation=request_build"));
        assert!(message.contains("method=extension"));
        assert!(message.contains("error_class=builder_error"));
        assert!(rendered.contains("class: \"internal\""));
        assert!(!rendered.contains(LOWER_ERROR_SECRET));
        assert!(!rendered.contains(EXTENSION_METHOD_SECRET));
    }

    #[test]
    fn outward_dialog_error_construction_has_no_raw_lower_error_formatting() {
        let source = include_str!("transaction_integration.rs");
        let test_marker = "#[cfg(test)]\nmod outward_error_redaction_tests";
        let test_start = source.find(test_marker).expect("test module marker");
        let after_test_offset = source[test_start..]
            .find("\nimpl DialogManager {")
            .expect("production resumes after test module");
        let production = format!(
            "{}{}",
            &source[..test_start],
            &source[test_start + after_test_offset..]
        );

        for forbidden in [
            "message: format!",
            "map_err(|e| crate::errors::DialogError",
            "DialogError::protocol_error(&format!",
            "DialogError::routing_error(&format!",
            "DialogError::internal_error(&format!",
        ] {
            assert!(
                !production.contains(forbidden),
                "outward DialogError construction contains unsafe form: {forbidden}"
            );
        }

        for line in production.lines().filter(|line| line.contains("message:")) {
            assert!(
                line.contains("safe_operation_failure")
                    || line.contains("safe_method_operation_failure"),
                "outward DialogError message bypasses safe construction: {line}"
            );
        }

        let raw_lower_error_stringifications = production
            .match_indices("e.to_string()")
            .filter(|(index, _)| {
                production[..*index]
                    .chars()
                    .next_back()
                    .is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
            })
            .count();
        assert_eq!(
            raw_lower_error_stringifications, 0,
            "lower transaction errors must never be classified by string contents"
        );
    }

    fn application_header(name: &str, value: &str) -> TypedHeader {
        TypedHeader::Other(
            HeaderName::Other(name.to_string()),
            HeaderValue::Raw(value.as_bytes().to_vec()),
        )
    }

    fn valid_initial_invite() -> Request {
        crate::transaction::client::builders::InviteBuilder::new()
            .from_detailed(Some("Alice"), "sip:alice@example.test", Some("from-tag"))
            .to_detailed(Some("Bob"), "sip:bob@example.test", None)
            .call_id("call-id@example.test")
            .cseq(1)
            .request_uri("sip:bob@example.test")
            .local_address("127.0.0.1:5060".parse().unwrap())
            .contact("sip:alice@127.0.0.1:5060")
            .build()
            .unwrap()
    }

    #[test]
    fn initial_invite_plan_preserves_application_order_and_duplicates() {
        let headers = vec![
            application_header("X-Order", "first"),
            application_header("X-Other", "middle"),
            application_header("x-order", "second"),
        ];
        let plan = plan_initial_invite_headers(None, headers).unwrap();
        assert_eq!(plan.appended.len(), 3);

        let mut request = valid_initial_invite();
        append_validated_initial_invite_headers(&mut request, Vec::new(), plan.appended).unwrap();
        let values = request
            .headers
            .iter()
            .filter_map(|header| match header {
                TypedHeader::Other(name, HeaderValue::Raw(value))
                    if name.wire_eq(&HeaderName::Other("X-Order".into())) =>
                {
                    Some(String::from_utf8(value.clone()).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(values, ["first", "second"]);
    }

    #[test]
    fn initial_invite_plan_rejects_contact_aliases_and_collisions() {
        let raw_alias = application_header("m", "<sip:raw@example.test>");
        assert!(plan_initial_invite_headers(None, vec![raw_alias]).is_err());

        let contact = rvoip_sip_core::types::Contact::from_str("<sip:typed@example.test>").unwrap();
        assert!(plan_initial_invite_headers(
            Some("sip:override@example.test".into()),
            vec![TypedHeader::Contact(contact.clone())],
        )
        .is_err());
        assert!(plan_initial_invite_headers(
            None,
            vec![
                TypedHeader::Contact(contact.clone()),
                TypedHeader::Contact(contact),
            ],
        )
        .is_err());
    }

    #[test]
    fn outbound_proxy_precedes_registration_service_routes() {
        let proxy: rvoip_sip_core::Uri = "sips:edge.example.test;lr".parse().unwrap();
        let service_one: rvoip_sip_core::Uri = "sip:service-one.example.test;lr".parse().unwrap();
        let service_two: rvoip_sip_core::Uri = "sip:service-two.example.test;lr".parse().unwrap();
        let planned = planned_initial_invite_routes(
            Some(&proxy),
            &[service_one.clone(), service_two.clone()],
        );
        assert_eq!(planned, vec![proxy, service_one, service_two]);
    }

    #[test]
    fn final_initial_invite_append_rejects_singleton_alias_before_send() {
        let mut request = valid_initial_invite();
        let duplicate = application_header("i", "other-call-id@example.test");
        assert!(
            append_validated_initial_invite_headers(&mut request, Vec::new(), vec![duplicate],)
                .is_err()
        );
    }
}

/// RFC 3261 §22.2 — resend an INVITE with an `Authorization` or
/// `Proxy-Authorization` header after the UAS/proxy challenged with 401/407.
///
/// The local UAC request template is reused so the retry keeps the same
/// `Call-ID` and `From` tag, has no remote tag, and bumps CSeq on a new client
/// transaction. On the UAS side, the 401/407 final response terminates the
/// early dialog, so this retry must be routed as a fresh initial INVITE rather
/// than a re-INVITE. The caller supplies the fully-formatted auth header value.
impl DialogManager {
    fn invite_failover_plan_ids_for_dialog(&self, dialog_id: &DialogId) -> Vec<u64> {
        self.invite_failover_plans_by_dialog
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    fn invite_failover_attempt_ids_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Vec<std::sync::Arc<TransactionKey>> {
        self.invite_failover_attempts_by_dialog
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    /// Link a retained plan while `invite_failover_registry_lock` is held.
    fn index_invite_failover_plan_locked(&self, dialog_id: &DialogId, plan_id: u64) {
        let mut plan_ids = self
            .invite_failover_plans_by_dialog
            .entry(dialog_id.clone())
            .or_default();
        if !plan_ids.contains(&plan_id) {
            plan_ids.push(plan_id);
        }
    }

    /// Unlink a retained plan while `invite_failover_registry_lock` is held.
    fn unindex_invite_failover_plan_locked(&self, dialog_id: &DialogId, plan_id: u64) {
        if let Some(mut plan_ids) = self.invite_failover_plans_by_dialog.get_mut(dialog_id) {
            plan_ids.retain(|candidate| *candidate != plan_id);
        }
        self.invite_failover_plans_by_dialog
            .remove_if(dialog_id, |_, plan_ids| plan_ids.is_empty());
    }

    /// Publish an exact retained attempt and reverse owner while
    /// `invite_failover_registry_lock` is held.
    fn index_invite_failover_attempt_locked(
        &self,
        transaction_id: std::sync::Arc<TransactionKey>,
        attempt: InviteFailoverAttemptIndex,
    ) {
        if let Some(previous) = self
            .invite_failover_attempts
            .insert(transaction_id.clone(), attempt.clone())
        {
            self.unindex_invite_failover_attempt_locked(
                &previous.dialog_id,
                transaction_id.as_ref(),
            );
        }
        let mut transaction_ids = self
            .invite_failover_attempts_by_dialog
            .entry(attempt.dialog_id)
            .or_default();
        if !transaction_ids
            .iter()
            .any(|candidate| candidate.as_ref() == transaction_id.as_ref())
        {
            transaction_ids.push(transaction_id);
        }
    }

    fn unindex_invite_failover_attempt_locked(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) {
        if let Some(mut transaction_ids) =
            self.invite_failover_attempts_by_dialog.get_mut(dialog_id)
        {
            transaction_ids.retain(|candidate| candidate.as_ref() != transaction_id);
        }
        self.invite_failover_attempts_by_dialog
            .remove_if(dialog_id, |_, transaction_ids| transaction_ids.is_empty());
    }

    /// Remove only the exact retained attempt expected by the caller while
    /// `invite_failover_registry_lock` is held.
    fn remove_invite_failover_attempt_locked(
        &self,
        transaction_id: &TransactionKey,
        expected_plan_id: Option<u64>,
    ) -> Option<InviteFailoverAttemptIndex> {
        let expected = self
            .invite_failover_attempts
            .get(transaction_id)
            .map(|entry| entry.value().clone())?;
        if expected_plan_id.is_some_and(|plan_id| expected.plan_id != plan_id) {
            return None;
        }
        let removed = self
            .invite_failover_attempts
            .remove_if(transaction_id, |_, current| {
                current.plan_id == expected.plan_id
                    && current.dialog_id == expected.dialog_id
                    && current.candidate_index == expected.candidate_index
            })?;
        self.unindex_invite_failover_attempt_locked(&expected.dialog_id, transaction_id);
        Some(removed.1)
    }

    /// True only when the exact retained wire-unknown INVITE transaction has
    /// received a terminal non-2xx response. A CANCEL transaction's own final
    /// response is not sufficient: RFC 3261 teardown completes when the
    /// original INVITE resolves (normally 487), while a late 2xx requires the
    /// separate ACK+BYE path.
    pub(crate) async fn wire_unknown_invite_has_terminal_failure(
        &self,
        dialog_id: &DialogId,
    ) -> bool {
        for plan_id in self.invite_failover_plan_ids_for_dialog(dialog_id) {
            let Some(plan) = self
                .invite_failover_plans
                .get(&plan_id)
                .map(|entry| entry.value().clone())
            else {
                continue;
            };
            let transaction_id = {
                let plan = plan.lock().await;
                if plan.id != plan_id
                    || &plan.dialog_id != dialog_id
                    || plan.phase != InviteFailoverPlanPhase::WireUnknown
                {
                    continue;
                }
                plan.current_transaction.clone()
            };
            let Some(transaction_id) = transaction_id else {
                continue;
            };
            if self
                .transaction_manager
                .last_response(&transaction_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|response| response.status().as_u16() >= 300)
            {
                return true;
            }
        }
        false
    }

    /// Mark a non-expiring wire-unknown INVITE plan as legally torn down and
    /// immediately release its retained CANCEL route/reservations.
    pub(crate) async fn complete_wire_unknown_invite_for_dialog(&self, dialog_id: &DialogId) {
        let plan_ids = self.invite_failover_plan_ids_for_dialog(dialog_id);
        for plan_id in &plan_ids {
            if let Some(plan) = self
                .invite_failover_plans
                .get(plan_id)
                .map(|entry| entry.value().clone())
            {
                let mut plan = plan.lock().await;
                if plan.id == *plan_id
                    && &plan.dialog_id == dialog_id
                    && plan.phase == InviteFailoverPlanPhase::WireUnknown
                {
                    plan.transition_to_terminal(InviteFailoverPlanPhase::Closed);
                    self.schedule_invite_failover_plan_expiry(&mut plan, Instant::now());
                }
            }
        }
        for plan_id in plan_ids {
            self.try_remove_invite_failover_plan(plan_id);
        }
    }

    /// Change a retained plan's deadline/eligibility and atomically replace
    /// its sole scheduler entry. Callers hold the plan mutex, which makes the
    /// generation update the ordering boundary for concurrent maintenance.
    pub(crate) fn schedule_invite_failover_plan_expiry(
        &self,
        plan: &mut InviteFailoverPlan,
        expires_at: Instant,
    ) -> bool {
        plan.expires_at = expires_at;
        plan.expiry_generation = plan.expiry_generation.wrapping_add(1).max(1);
        let scheduled = self
            .invite_failover_expiry_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .schedule(plan.id, plan.expiry_generation, plan.expires_at, plan.phase);
        if !scheduled {
            error!(
                plan_id = plan.id,
                "Retained initial-INVITE expiry scheduler reached its hard capacity"
            );
        }
        scheduled
    }

    pub(crate) async fn prune_invite_failover_state(&self) {
        let now = Instant::now();
        // Dialog cleanup has a reserved share of the bounded maintenance
        // budget so sustained expiry churn cannot starve heavy-state
        // compaction, while ordinary protocol expiry always retains the
        // remaining share.
        let cleanup_due = self
            .invite_failover_expiry_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_due_dialog_cleanup(now, INVITE_FAILOVER_DIALOG_CLEANUP_BATCH);
        let cleanup_attempts = cleanup_due.len();
        for cleanup in cleanup_due {
            match self.try_apply_invite_failover_dialog_cleanup(&cleanup) {
                InviteFailoverDialogCleanupOutcome::Busy => {
                    let plan_id = cleanup.key.plan_id;
                    let restored = self
                        .invite_failover_expiry_scheduler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .restore_dialog_cleanup_after_busy(
                            cleanup,
                            Instant::now() + INVITE_FAILOVER_EXPIRY_BUSY_RETRY,
                        );
                    if !restored {
                        error!(
                            plan_id,
                            "Retained initial-INVITE dialog cleanup retry reached its hard capacity"
                        );
                    }
                }
                InviteFailoverDialogCleanupOutcome::Complete
                | InviteFailoverDialogCleanupOutcome::Stale => {}
            }
        }

        let expiry_budget = INVITE_FAILOVER_EXPIRY_BATCH.saturating_sub(cleanup_attempts);
        let due = self
            .invite_failover_expiry_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_due(now, expiry_budget);
        for mut expiry in due {
            match self.try_remove_scheduled_invite_failover_plan(
                expiry.key.plan_id,
                expiry.key.generation,
                false,
            ) {
                InviteFailoverRemovalOutcome::Busy => {
                    self.invite_failover_expiry_scheduler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .restore_after_busy(
                            expiry,
                            Instant::now() + INVITE_FAILOVER_EXPIRY_BUSY_RETRY,
                        );
                }
                InviteFailoverRemovalOutcome::Deferred {
                    wake_at,
                    overflow_evictable,
                } => {
                    expiry.overflow_evictable = overflow_evictable;
                    self.invite_failover_expiry_scheduler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .restore_after_busy(expiry, wake_at);
                }
                InviteFailoverRemovalOutcome::Removed
                | InviteFailoverRemovalOutcome::Missing
                | InviteFailoverRemovalOutcome::Stale
                | InviteFailoverRemovalOutcome::NonExpiring => {}
            }
        }

        // The registry normally cannot exceed its hard cap. Preserve the
        // defensive overflow semantics for corrupted/restored state without
        // scanning every plan: the scheduler has an exact ordered subset of
        // non-active, non-wire-unknown tombstones.
        let mut overflow_attempts = 0usize;
        while self.invite_failover_plans.len() > self.invite_failover_plan_capacity
            && overflow_attempts < INVITE_FAILOVER_EXPIRY_BATCH
        {
            let Some(mut expiry) = self
                .invite_failover_expiry_scheduler
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_oldest_overflow_evictable()
            else {
                break;
            };
            overflow_attempts += 1;
            match self.try_remove_scheduled_invite_failover_plan(
                expiry.key.plan_id,
                expiry.key.generation,
                true,
            ) {
                InviteFailoverRemovalOutcome::Busy => {
                    self.invite_failover_expiry_scheduler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .restore_after_busy(expiry, now + INVITE_FAILOVER_EXPIRY_BUSY_RETRY);
                }
                InviteFailoverRemovalOutcome::Deferred {
                    wake_at,
                    overflow_evictable,
                } => {
                    expiry.overflow_evictable = overflow_evictable;
                    self.invite_failover_expiry_scheduler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .restore_after_busy(expiry, wake_at);
                }
                InviteFailoverRemovalOutcome::Removed
                | InviteFailoverRemovalOutcome::Missing
                | InviteFailoverRemovalOutcome::Stale
                | InviteFailoverRemovalOutcome::NonExpiring => {}
            }
        }
    }

    fn try_apply_invite_failover_dialog_cleanup(
        &self,
        cleanup: &InviteFailoverScheduledDialogCleanup,
    ) -> InviteFailoverDialogCleanupOutcome {
        let Some(expected_plan) = cleanup.plan.upgrade() else {
            return InviteFailoverDialogCleanupOutcome::Stale;
        };
        let Some(registered_plan) = self
            .invite_failover_plans
            .get(&cleanup.key.plan_id)
            .map(|entry| entry.value().clone())
        else {
            return InviteFailoverDialogCleanupOutcome::Stale;
        };
        if !std::sync::Arc::ptr_eq(&registered_plan, &expected_plan) {
            return InviteFailoverDialogCleanupOutcome::Stale;
        }
        let Ok(mut plan) = expected_plan.try_lock() else {
            return InviteFailoverDialogCleanupOutcome::Busy;
        };
        if plan.id != cleanup.key.plan_id || plan.dialog_id != cleanup.dialog_id {
            return InviteFailoverDialogCleanupOutcome::Stale;
        }

        // Wire-unknown teardown remains dialog-addressed and non-expiring.
        // Consume only the retry; preserve its phase, current transaction,
        // reverse indexes, and capacity charge for legal CANCEL/BYE recovery.
        if plan.phase == InviteFailoverPlanPhase::WireUnknown {
            self.schedule_invite_failover_plan_expiry(
                &mut plan,
                Instant::now() + INVITE_FAILOVER_PLAN_TTL,
            );
            return InviteFailoverDialogCleanupOutcome::Complete;
        }

        if plan.phase == InviteFailoverPlanPhase::Active {
            plan.transition_to_terminal(InviteFailoverPlanPhase::Cancelled);
        } else {
            // Restored or test-injected terminal state may still carry the
            // active-only payload. Reapplying its phase compacts idempotently.
            let phase = plan.phase;
            plan.transition_to_terminal(phase);
        }
        self.schedule_invite_failover_plan_expiry(
            &mut plan,
            Instant::now() + INVITE_FAILOVER_PLAN_TTL,
        );
        let attempts = plan.attempt_transaction_ids();

        // Removing active ownership during synchronous dialog cleanup
        // linearizes the attempt set. These exact unlinks are idempotent and
        // ensure a captured pre-cleanup route cannot keep the dialog alive.
        for transaction_id in &attempts {
            self.unlink_transaction_from_dialog_indexed(transaction_id.as_ref());
        }

        let _registry = self
            .invite_failover_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let still_registered = self
            .invite_failover_plans
            .get(&cleanup.key.plan_id)
            .is_some_and(|entry| std::sync::Arc::ptr_eq(entry.value(), &expected_plan));
        if !still_registered {
            return InviteFailoverDialogCleanupOutcome::Stale;
        }
        self.unindex_invite_failover_plan_locked(&cleanup.dialog_id, cleanup.key.plan_id);
        for transaction_id in attempts {
            if self
                .invite_failover_attempts
                .get(transaction_id.as_ref())
                .is_some_and(|attempt| {
                    attempt.plan_id == cleanup.key.plan_id && attempt.dialog_id == cleanup.dialog_id
                })
            {
                self.unindex_invite_failover_attempt_locked(
                    &cleanup.dialog_id,
                    transaction_id.as_ref(),
                );
            }
        }
        InviteFailoverDialogCleanupOutcome::Complete
    }

    fn try_remove_invite_failover_plan(&self, plan_id: u64) {
        let _ = self.try_remove_invite_failover_plan_inner(plan_id, None, false);
    }

    fn try_remove_scheduled_invite_failover_plan(
        &self,
        plan_id: u64,
        generation: u64,
        permit_overflow: bool,
    ) -> InviteFailoverRemovalOutcome {
        self.try_remove_invite_failover_plan_inner(plan_id, Some(generation), permit_overflow)
    }

    fn try_remove_invite_failover_plan_inner(
        &self,
        plan_id: u64,
        expected_generation: Option<u64>,
        permit_overflow: bool,
    ) -> InviteFailoverRemovalOutcome {
        let Some(plan) = self
            .invite_failover_plans
            .get(&plan_id)
            .map(|entry| entry.value().clone())
        else {
            return InviteFailoverRemovalOutcome::Missing;
        };
        let Ok(mut plan_guard) = plan.try_lock() else {
            return InviteFailoverRemovalOutcome::Busy;
        };
        let _registry = self
            .invite_failover_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let still_registered = self
            .invite_failover_plans
            .get(&plan_id)
            .is_some_and(|entry| std::sync::Arc::ptr_eq(entry.value(), &plan));
        if !still_registered {
            return InviteFailoverRemovalOutcome::Missing;
        }
        if expected_generation.is_some_and(|generation| generation != plan_guard.expiry_generation)
        {
            return InviteFailoverRemovalOutcome::Stale;
        }
        let expired = plan_guard.expires_at <= Instant::now();
        if plan_guard.phase == InviteFailoverPlanPhase::WireUnknown {
            return InviteFailoverRemovalOutcome::NonExpiring;
        }
        let evictable_overflow = permit_overflow
            && plan_guard.phase.is_overflow_evictable()
            && self.invite_failover_plans.len() > self.invite_failover_plan_capacity;
        if !expired && !evictable_overflow {
            return InviteFailoverRemovalOutcome::Deferred {
                wake_at: plan_guard.expires_at,
                overflow_evictable: plan_guard.phase.is_overflow_evictable(),
            };
        }
        let dialog_id = plan_guard.dialog_id.clone();
        let reserved_attempt_slots = plan_guard.reserved_attempt_slots;
        let attempts = plan_guard.attempt_transaction_ids();

        // The plan remains locked until every owner/index is detached and its
        // reservation is released. A worker that captured this Arc before the
        // prune therefore observes a non-active generation and cannot advance
        // an orphan after capacity has been returned.
        let terminal_phase = if plan_guard.phase == InviteFailoverPlanPhase::Active {
            InviteFailoverPlanPhase::Closed
        } else {
            plan_guard.phase
        };
        plan_guard.transition_to_terminal(terminal_phase);
        self.active_invite_failover_by_dialog
            .remove_if(&dialog_id, |_, active_plan_id| *active_plan_id == plan_id);

        for transaction_id in attempts {
            self.remove_invite_failover_attempt_locked(transaction_id.as_ref(), Some(plan_id));
            self.unlink_transaction_from_dialog_indexed(transaction_id.as_ref());
        }
        if self.invite_failover_plans.remove(&plan_id).is_none() {
            return InviteFailoverRemovalOutcome::Missing;
        }
        self.invite_failover_expiry_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(plan_id);
        self.unindex_invite_failover_plan_locked(&dialog_id, plan_id);
        Self::release_invite_failover_reservation(&self.invite_failover_plan_reservations, 1);
        Self::release_invite_failover_reservation(
            &self.invite_failover_attempt_reservations,
            reserved_attempt_slots,
        );
        InviteFailoverRemovalOutcome::Removed
    }

    pub(crate) fn remove_invite_failover_state_for_dialog(&self, dialog_id: &DialogId) {
        // Detach the normal dialog indexes, but deliberately retain the
        // bounded failover tombstone. A forked 2xx can arrive after the
        // application removes a cancelled/failed dialog and still requires
        // an authenticated ACK followed by one BYE. Removing the active
        // ownership entry prevents any further candidate advancement. The
        // retained reverse indexes make this work proportional only to this
        // dialog, even when its plan mutex is temporarily busy.
        let (plan_ids, attempts) = {
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.active_invite_failover_by_dialog.remove(dialog_id);
            (
                self.invite_failover_plan_ids_for_dialog(dialog_id),
                self.invite_failover_attempt_ids_for_dialog(dialog_id),
            )
        };
        // Ordinary terminal plans remain reachable by their exact transaction
        // keys for late/forked 2xx processing. Once the dialog itself is
        // detached, retaining the two dialog-keyed reverse indexes duplicates
        // every plan/attempt key for the full failover TTL without serving a
        // protocol lookup. Wire-unknown plans are the exception: their
        // cancellation/recovery API is deliberately dialog-addressed, so they
        // keep both reverse indexes until exact teardown completes.
        let mut terminal_reverse_indexes = Vec::new();
        for plan_id in plan_ids {
            if let Some(plan) = self
                .invite_failover_plans
                .get(&plan_id)
                .map(|entry| entry.value().clone())
            {
                if let Ok(mut plan) = plan.try_lock() {
                    if plan.phase == InviteFailoverPlanPhase::Active {
                        plan.transition_to_terminal(InviteFailoverPlanPhase::Cancelled);
                    }
                    self.schedule_invite_failover_plan_expiry(
                        &mut plan,
                        Instant::now() + INVITE_FAILOVER_PLAN_TTL,
                    );
                    if plan.phase != InviteFailoverPlanPhase::WireUnknown {
                        terminal_reverse_indexes.push((plan.id, plan.attempt_transaction_ids()));
                    }
                } else {
                    // Dialog removal is synchronous and must never await a
                    // candidate worker that owns this plan. Publish one exact,
                    // bounded retry while the registry is stable; the manager
                    // maintenance task will compact it after the lock is free.
                    let scheduled = {
                        let _registry = self
                            .invite_failover_registry_lock
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        let still_registered = self
                            .invite_failover_plans
                            .get(&plan_id)
                            .is_some_and(|entry| std::sync::Arc::ptr_eq(entry.value(), &plan));
                        let still_indexed = self
                            .invite_failover_plans_by_dialog
                            .get(dialog_id)
                            .is_some_and(|plan_ids| plan_ids.contains(&plan_id));
                        if self.is_accepting_work() && still_registered && still_indexed {
                            self.invite_failover_expiry_scheduler
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .schedule_dialog_cleanup(
                                    plan_id,
                                    dialog_id,
                                    std::sync::Arc::downgrade(&plan),
                                    Instant::now(),
                                )
                        } else {
                            true
                        }
                    };
                    if !scheduled {
                        error!(
                            plan_id,
                            "Retained initial-INVITE dialog cleanup retry reached its hard capacity"
                        );
                    }
                }
            }
        }
        for transaction_id in &attempts {
            self.transaction_to_dialog
                .remove_if(transaction_id, |_, mapped| mapped == dialog_id);
        }

        if !terminal_reverse_indexes.is_empty() {
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (plan_id, transaction_ids) in terminal_reverse_indexes {
                self.unindex_invite_failover_plan_locked(dialog_id, plan_id);
                for transaction_id in transaction_ids {
                    self.unindex_invite_failover_attempt_locked(dialog_id, transaction_id.as_ref());
                }
            }
        }
    }

    fn invite_failover_event_transaction_id(event: &TransactionEvent) -> Option<&TransactionKey> {
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id, .. }
            | TransactionEvent::SuccessResponse { transaction_id, .. }
            | TransactionEvent::FailureResponse { transaction_id, .. }
            | TransactionEvent::TransactionTimeout { transaction_id }
            | TransactionEvent::TransportError { transaction_id }
            | TransactionEvent::TransactionTerminated { transaction_id }
            | TransactionEvent::StateChanged { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::Error {
                transaction_id: Some(transaction_id),
                ..
            } => Some(transaction_id),
            _ => None,
        }
    }

    async fn emit_invite_internal_failure(
        &self,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
    ) {
        self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
            dialog_id: Some(dialog_id),
            transaction_id,
            status_code: 500,
            reason_phrase: "Local transaction processing failed".to_string(),
            method: Method::Invite.to_string(),
        })
        .await;
    }

    fn close_active_invite_failover_plan(
        &self,
        plan: &mut InviteFailoverPlan,
        phase: InviteFailoverPlanPhase,
    ) {
        plan.transition_to_terminal(phase);
        self.schedule_invite_failover_plan_expiry(plan, Instant::now() + INVITE_FAILOVER_PLAN_TTL);
        let _registry = self
            .invite_failover_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.active_invite_failover_by_dialog
            .remove_if(&plan.dialog_id, |_, active_plan_id| {
                *active_plan_id == plan.id
            });
    }

    async fn advance_invite_failover_plan(
        &self,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        completed_transaction: &TransactionKey,
        outcome: InviteFailoverAttemptOutcome,
    ) -> bool {
        let Some(_operation) = self.enter_invite_failover_operation() else {
            let mut plan = plan.lock().await;
            if plan.phase == InviteFailoverPlanPhase::Active {
                self.close_active_invite_failover_plan(&mut plan, InviteFailoverPlanPhase::Closed);
            }
            return false;
        };
        {
            let mut plan = plan.lock().await;
            let owner_is_current = self
                .active_invite_failover_by_dialog
                .get(&plan.dialog_id)
                .is_some_and(|owner| *owner.value() == plan.id);
            if plan.phase != InviteFailoverPlanPhase::Active
                || !owner_is_current
                || plan.current_transaction.as_deref() != Some(completed_transaction)
            {
                return false;
            }
            Self::set_invite_attempt_outcome(&mut plan, completed_transaction, outcome);
            plan.current_transaction = None;
            if let Some(active) = plan.active_payload.as_mut() {
                active.current_candidate_index = None;
                active.current_send_generation = None;
            }
        }

        let candidate_index = {
            let mut plan = plan.lock().await;
            if plan.phase != InviteFailoverPlanPhase::Active {
                return false;
            }
            let next_candidate_index = plan
                .active_payload
                .as_ref()
                .map_or(0, |active| active.next_candidate_index);
            if next_candidate_index >= plan.candidate_count() {
                self.close_active_invite_failover_plan(
                    &mut plan,
                    InviteFailoverPlanPhase::Exhausted,
                );
                return false;
            }
            next_candidate_index
        };
        match self.send_invite_failover_candidate(plan).await {
            Ok((transaction_id, destination)) => {
                let plan_id = plan.lock().await.id;
                debug!(
                    plan_id,
                    candidate_index,
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id),
                    %destination,
                    "Advanced retained initial-INVITE failover plan"
                );
                true
            }
            Err(InviteCandidateSendError::Fatal(error)) => {
                let plan_id = plan.lock().await.id;
                warn!(
                    plan_id,
                    candidate_index,
                    error_class = error.diagnostic_class(),
                    "Retained initial-INVITE failover stopped on non-transport failure"
                );
                false
            }
            Err(InviteCandidateSendError::WireUnknown(error)) => {
                let plan_id = plan.lock().await.id;
                warn!(
                    plan_id,
                    candidate_index,
                    error_class = error.diagnostic_class(),
                    "Retained initial-INVITE failover stopped with wire outcome unknown"
                );
                false
            }
        }
    }

    fn local_sent_by_for_request(request: &Request) -> Option<SocketAddr> {
        request.first_via().and_then(|via| {
            via.0.first().and_then(|via_header| {
                let Host::Address(address) = via_header.host() else {
                    return None;
                };
                let default_port = if via_header
                    .sent_protocol
                    .transport
                    .eq_ignore_ascii_case("TLS")
                {
                    5061
                } else {
                    5060
                };
                Some(SocketAddr::new(
                    *address,
                    via_header.port().unwrap_or(default_port),
                ))
            })
        })
    }

    async fn start_bye_for_late_invite_success(
        &self,
        invite_transaction: &TransactionKey,
        response: &Response,
    ) -> DialogResult<TransactionKey> {
        let ack = self
            .transaction_manager
            .create_ack_for_2xx(invite_transaction, response)
            .await
            .map_err(|_| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("late_invite_bye_template", "transaction_error"),
            })?;
        let from = ack.from().ok_or_else(|| {
            crate::errors::DialogError::routing_error("late 2xx ACK missing From")
        })?;
        let to = ack
            .to()
            .ok_or_else(|| crate::errors::DialogError::routing_error("late 2xx ACK missing To"))?;
        let call_id = ack.call_id().ok_or_else(|| {
            crate::errors::DialogError::routing_error("late 2xx ACK missing Call-ID")
        })?;
        let cseq = ack
            .cseq()
            .and_then(|cseq| cseq.seq.checked_add(1))
            .ok_or_else(|| crate::errors::DialogError::routing_error("late 2xx CSeq overflow"))?;
        let local_address = Self::local_sent_by_for_request(&ack).ok_or_else(|| {
            crate::errors::DialogError::routing_error("late 2xx ACK has no numeric Via sent-by")
        })?;
        let from_tag = from.tag().ok_or_else(|| {
            crate::errors::DialogError::routing_error("late 2xx ACK missing From tag")
        })?;
        let to_tag = to.tag().ok_or_else(|| {
            crate::errors::DialogError::routing_error("late 2xx ACK missing To tag")
        })?;

        let mut bye = ByeBuilder::from_dialog_enhanced(
            call_id.as_str(),
            from.address().uri.to_string(),
            from_tag,
            to.address().uri.to_string(),
            to_tag,
            ack.uri().to_string(),
            cseq,
            local_address,
            Vec::new(),
        )
        .via_transport(ack.first_via_transport().unwrap_or("UDP"))
        .build()
        .map_err(|_| crate::errors::DialogError::TransactionError {
            message: safe_operation_failure("late_invite_bye_build", "transaction_error"),
        })?;
        bye.headers.extend(
            ack.headers
                .iter()
                .filter(|header| header.name().wire_eq(&HeaderName::Route))
                .cloned(),
        );

        let mut route = self
            .transaction_manager
            .transaction_route(invite_transaction)
            .await
            .ok_or_else(|| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("late_invite_bye_route", "transaction_not_found"),
            })?;
        let next_hop =
            crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(&bye)
                .map_err(|_| crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("late_invite_bye_route", "invalid_route"),
                })?;
        let next_hop_authority = match &next_hop.host {
            Host::Domain(domain) => rvoip_sip_transport::TransportAuthority::dns(domain.clone())
                .map_err(|_| crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("late_invite_bye_route", "invalid_authority"),
                })?,
            Host::Address(address) => rvoip_sip_transport::TransportAuthority::ip(*address),
        };
        if let Some(destination) =
            crate::transaction::manager::utils::socket_addr_from_uri(&next_hop)
        {
            if route.destination != destination {
                route.destination = destination;
                route.flow_id = None;
                route.authority = Some(next_hop_authority);
            }
        } else {
            let same_authority = route.authority.as_ref() == Some(&next_hop_authority);
            let same_explicit_port = next_hop
                .port
                .is_none_or(|port| port == route.destination.port());
            if !same_authority || !same_explicit_port {
                let target = self
                    .resolve_uri_to_candidates(&next_hop)
                    .await
                    .into_iter()
                    .next()
                    .ok_or_else(|| crate::errors::DialogError::TransactionError {
                        message: safe_operation_failure("late_invite_bye_route", "no_candidates"),
                    })?;
                route.destination = target.addr;
                route.transport_type = Some(target.transport);
                route.authority = target.authority.or(Some(next_hop_authority));
                route.flow_id = None;
            }
        }

        let bye_transaction = self
            .transaction_manager
            .create_client_transaction_on_route(bye, route)
            .await
            .map_err(|_| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("late_invite_bye_create", "transaction_error"),
            })?;
        if self
            .transaction_manager
            .send_request(&bye_transaction)
            .await
            .is_err()
        {
            let _ = self
                .transaction_manager
                .terminate_transaction(&bye_transaction)
                .await;
            return Err(crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("late_invite_bye_send", "transport_error"),
            });
        }

        Ok(bye_transaction)
    }

    async fn wait_for_late_invite_bye_success(
        &self,
        bye_transaction: &TransactionKey,
    ) -> DialogResult<()> {
        // `send_request` confirms the exact first transport write; it does not
        // mean that the remote fork accepted the BYE. Wait on the
        // transaction-keyed completion authority rather than allocating a
        // global observational subscription. The completion is retained past
        // runner removal, so both response-before-wait and
        // response-versus-removal races are covered.
        let completion_deadline = self
            .transaction_manager
            .timer_settings()
            .transaction_timeout
            .saturating_add(Duration::from_millis(100));
        let outcome = self
            .transaction_manager
            .wait_for_client_transaction_outcome(bye_transaction, completion_deadline)
            .await
            .map_err(|_| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("late_invite_bye_completion", "transaction_error"),
            })?;

        match outcome {
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(response))
                if (200..300).contains(&response.status().as_u16()) =>
            {
                Ok(())
            }
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(_)) => {
                Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure(
                        "late_invite_bye_completion",
                        "non_success_response",
                    ),
                })
            }
            Some(crate::transaction::ClientTransactionOutcome::Failure(_)) => {
                Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure(
                        "late_invite_bye_completion",
                        "terminal_failure",
                    ),
                })
            }
            None => {
                // The transaction timer should normally publish its typed
                // failure before this guard expires. If it does not, force
                // local cleanup and leave the fork tag uncommitted so a
                // retransmitted late 2xx can retry ACK-then-BYE.
                let _ = self
                    .transaction_manager
                    .terminate_transaction(bye_transaction)
                    .await;
                Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("late_invite_bye_completion", "timeout"),
                })
            }
        }
    }

    async fn acknowledge_and_cleanup_late_invite_success(
        &self,
        plan_arc: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        plan: &mut InviteFailoverPlan,
        transaction_id: &TransactionKey,
        response: &Response,
    ) {
        if let Err(error) = self
            .transaction_manager
            .send_ack_for_2xx(transaction_id, response)
            .await
        {
            warn!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error),
                "Failed to ACK late or duplicate initial-INVITE success"
            );
            // RFC 3261 requires the 2xx ACK before terminating a
            // non-selected fork. Leave the tag unmarked so a retransmitted
            // 2xx serially retries the complete ACK-then-BYE cleanup.
            return;
        }

        let Some(to_tag) = response.to().and_then(|to| to.tag()).map(str::to_owned) else {
            warn!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                "Cannot terminate late initial-INVITE fork without a To tag"
            );
            return;
        };
        if plan.is_cleaned_fork(&to_tag) {
            return;
        }
        if plan.is_fork_cleanup_in_flight(&to_tag) {
            // Every retransmitted 2xx still receives the ACK above. The
            // existing BYE owns cleanup until its exact final outcome is
            // known; do not create a parallel transaction for the same fork.
            return;
        }

        let Some(cleanup_operation) = self.enter_invite_failover_operation() else {
            return;
        };
        plan.mark_fork_cleanup_in_flight(to_tag.clone());

        // Start the compensating BYE before returning from the authoritative
        // late-2xx handler. This preserves the event-path ordering contract:
        // once handling completes, the exact BYE transaction is registered
        // and its first transport write has either succeeded or cleanup has
        // already been left retryable. Only the potentially long final-response
        // wait runs in the drain-tracked background operation.
        let bye_transaction = match self
            .start_bye_for_late_invite_success(transaction_id, response)
            .await
        {
            Ok(transaction_id) => transaction_id,
            Err(error) => {
                plan.clear_fork_cleanup_in_flight(&to_tag);
                warn!(
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    error_class = error.diagnostic_class(),
                    "Failed to start termination of a non-selected initial-INVITE fork"
                );
                return;
            }
        };

        let manager = self.clone();
        let plan_arc = plan_arc.clone();
        let invite_transaction = transaction_id.clone();
        tokio::spawn(async move {
            let _cleanup_operation = cleanup_operation;
            let result = manager
                .wait_for_late_invite_bye_success(&bye_transaction)
                .await;
            let mut plan = plan_arc.lock().await;
            plan.clear_fork_cleanup_in_flight(&to_tag);

            match result {
                Ok(()) => {
                    plan.mark_cleaned_fork(to_tag);
                    info!(
                        invite_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_transaction),
                        bye_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&bye_transaction),
                        "ACKed and terminated a non-selected initial-INVITE fork"
                    );
                }
                Err(error) => {
                    warn!(
                        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_transaction),
                        error_class = error.diagnostic_class(),
                        "Failed to terminate a non-selected initial-INVITE fork"
                    );
                }
            }
        });
    }

    pub(crate) async fn handle_invite_failover_event(
        &self,
        event: &TransactionEvent,
    ) -> InviteFailoverEventDisposition {
        let Some(transaction_id) = Self::invite_failover_event_transaction_id(event) else {
            return InviteFailoverEventDisposition::Continue;
        };
        let Some(attempt_index) = self
            .invite_failover_attempts
            .get(transaction_id)
            .map(|entry| entry.value().clone())
        else {
            return InviteFailoverEventDisposition::Continue;
        };
        let Some(plan_arc) = self
            .invite_failover_plans
            .get(&attempt_index.plan_id)
            .map(|entry| entry.value().clone())
        else {
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.unindex_invite_failover_plan_locked(
                &attempt_index.dialog_id,
                attempt_index.plan_id,
            );
            self.remove_invite_failover_attempt_locked(transaction_id, Some(attempt_index.plan_id));
            return InviteFailoverEventDisposition::Continue;
        };
        let mut plan = plan_arc.lock().await;
        // The exact transaction-keyed attempt map is authoritative. Keeping
        // and scanning a second candidate/outcome history in every terminal
        // plan added duplicate ownership without strengthening this check.
        if plan.id != attempt_index.plan_id || plan.dialog_id != attempt_index.dialog_id {
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.remove_invite_failover_attempt_locked(transaction_id, Some(attempt_index.plan_id));
            return InviteFailoverEventDisposition::Continue;
        }

        let is_current = plan.current_transaction.as_deref() == Some(transaction_id);
        let plan_is_active = plan.phase == InviteFailoverPlanPhase::Active
            && self
                .active_invite_failover_by_dialog
                .get(&plan.dialog_id)
                .is_some_and(|active_plan_id| *active_plan_id.value() == plan.id);
        match event {
            TransactionEvent::ProvisionalResponse { .. } => {
                if plan_is_active && is_current {
                    if let Some(active) = plan.active_payload.as_mut() {
                        active.provisional_seen = true;
                    }
                    InviteFailoverEventDisposition::Continue
                } else {
                    InviteFailoverEventDisposition::Consumed
                }
            }
            TransactionEvent::SuccessResponse { response, .. } => {
                let response_tag = response.to().and_then(|to| to.tag()).map(str::to_owned);
                if plan_is_active && is_current {
                    Self::set_invite_attempt_outcome(
                        &mut plan,
                        transaction_id,
                        InviteFailoverAttemptOutcome::Accepted,
                    );
                    plan.accepted_candidate_index = Some(attempt_index.candidate_index);
                    plan.accepted_to_tag = response_tag;
                    self.close_active_invite_failover_plan(
                        &mut plan,
                        InviteFailoverPlanPhase::Accepted,
                    );
                    InviteFailoverEventDisposition::Continue
                } else if plan.accepted_to_tag == response_tag
                    && (response_tag.is_some()
                        || plan.accepted_candidate_index == Some(attempt_index.candidate_index))
                {
                    if let Err(error) = self
                        .transaction_manager
                        .send_ack_for_2xx(transaction_id, response)
                        .await
                    {
                        warn!(
                            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                            error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error),
                            "Failed to re-ACK duplicate selected initial-INVITE success"
                        );
                    }
                    InviteFailoverEventDisposition::Consumed
                } else {
                    self.acknowledge_and_cleanup_late_invite_success(
                        &plan_arc,
                        &mut plan,
                        transaction_id,
                        response,
                    )
                    .await;
                    InviteFailoverEventDisposition::Consumed
                }
            }
            TransactionEvent::FailureResponse { response, .. } => {
                // A locally initiated CANCEL closes the plan before the wire
                // command is sent so it wins races with candidate failover.
                // The matching 487 still belongs to the selected INVITE and
                // must reach normal dialog processing, which publishes the
                // terminal CallCancelled event. Retire the current slot here
                // so a duplicate terminal event cannot be handed off twice.
                if plan.phase == InviteFailoverPlanPhase::Cancelled
                    && is_current
                    && response.status_code() == 487
                {
                    plan.current_transaction = None;
                    return InviteFailoverEventDisposition::Continue;
                }

                if !plan_is_active || !is_current {
                    return InviteFailoverEventDisposition::Consumed;
                }

                let (provisional_seen, next_candidate_index) =
                    plan.active_payload.as_ref().map_or((true, 0), |active| {
                        (active.provisional_seen, active.next_candidate_index)
                    });
                if response.status_code() == 503 && !provisional_seen {
                    if next_candidate_index < plan.candidate_count() {
                        drop(plan);
                        if self
                            .advance_invite_failover_plan(
                                &plan_arc,
                                transaction_id,
                                InviteFailoverAttemptOutcome::ServiceUnavailable,
                            )
                            .await
                        {
                            return InviteFailoverEventDisposition::Consumed;
                        }
                        return InviteFailoverEventDisposition::Continue;
                    }
                    Self::set_invite_attempt_outcome(
                        &mut plan,
                        transaction_id,
                        InviteFailoverAttemptOutcome::ServiceUnavailable,
                    );
                    self.close_active_invite_failover_plan(
                        &mut plan,
                        InviteFailoverPlanPhase::Exhausted,
                    );
                } else {
                    Self::set_invite_attempt_outcome(
                        &mut plan,
                        transaction_id,
                        InviteFailoverAttemptOutcome::FinalResponse,
                    );
                    self.close_active_invite_failover_plan(
                        &mut plan,
                        InviteFailoverPlanPhase::Closed,
                    );
                }
                InviteFailoverEventDisposition::Continue
            }
            TransactionEvent::TransactionTimeout { .. }
            | TransactionEvent::TransportError { .. } => {
                if !plan_is_active || !is_current {
                    return InviteFailoverEventDisposition::Consumed;
                }
                let outcome = if matches!(event, TransactionEvent::TransactionTimeout { .. }) {
                    InviteFailoverAttemptOutcome::TransactionTimeout
                } else {
                    InviteFailoverAttemptOutcome::TransportError
                };
                let (provisional_seen, next_candidate_index) =
                    plan.active_payload.as_ref().map_or((true, 0), |active| {
                        (active.provisional_seen, active.next_candidate_index)
                    });
                if INVITE_TIMEOUT_FAILOVER_ENABLED
                    && !provisional_seen
                    && next_candidate_index < plan.candidate_count()
                {
                    drop(plan);
                    if self
                        .advance_invite_failover_plan(&plan_arc, transaction_id, outcome)
                        .await
                    {
                        InviteFailoverEventDisposition::Consumed
                    } else {
                        InviteFailoverEventDisposition::Continue
                    }
                } else {
                    Self::set_invite_attempt_outcome(&mut plan, transaction_id, outcome);
                    self.close_active_invite_failover_plan(
                        &mut plan,
                        InviteFailoverPlanPhase::Closed,
                    );
                    InviteFailoverEventDisposition::Continue
                }
            }
            TransactionEvent::Error {
                transaction_id: Some(error_transaction_id),
                ..
            } if error_transaction_id == transaction_id => {
                if !plan_is_active || !is_current {
                    return InviteFailoverEventDisposition::Consumed;
                }

                let (provisional_seen, next_candidate_index) =
                    plan.active_payload.as_ref().map_or((true, 0), |active| {
                        (active.provisional_seen, active.next_candidate_index)
                    });
                if !provisional_seen && next_candidate_index < plan.candidate_count() {
                    drop(plan);
                    if self
                        .advance_invite_failover_plan(
                            &plan_arc,
                            transaction_id,
                            InviteFailoverAttemptOutcome::InternalError,
                        )
                        .await
                    {
                        return InviteFailoverEventDisposition::Consumed;
                    }

                    // A zero-wire failure while preparing the replacement may
                    // have closed the plan. Surface one deterministic call
                    // failure in that case. WireUnknown keeps its exact route
                    // alive for a later final response and must not fail early.
                    let terminal_dialog = {
                        let plan = plan_arc.lock().await;
                        matches!(
                            plan.phase,
                            InviteFailoverPlanPhase::Closed | InviteFailoverPlanPhase::Exhausted
                        )
                        .then(|| plan.dialog_id.clone())
                    };
                    if let Some(dialog_id) = terminal_dialog {
                        self.emit_invite_internal_failure(dialog_id, transaction_id.clone())
                            .await;
                    }
                    return InviteFailoverEventDisposition::Consumed;
                }

                Self::set_invite_attempt_outcome(
                    &mut plan,
                    transaction_id,
                    InviteFailoverAttemptOutcome::InternalError,
                );
                let dialog_id = plan.dialog_id.clone();
                let phase = if next_candidate_index >= plan.candidate_count() {
                    InviteFailoverPlanPhase::Exhausted
                } else {
                    // A provisional response makes another resolver candidate
                    // unsafe: the old branch may still establish a dialog.
                    InviteFailoverPlanPhase::Closed
                };
                self.close_active_invite_failover_plan(&mut plan, phase);
                drop(plan);
                self.emit_invite_internal_failure(dialog_id, transaction_id.clone())
                    .await;
                InviteFailoverEventDisposition::Consumed
            }
            TransactionEvent::TransactionTerminated { .. }
            | TransactionEvent::StateChanged {
                new_state: TransactionState::Terminated,
                ..
            } => InviteFailoverEventDisposition::Consumed,
            _ => InviteFailoverEventDisposition::Continue,
        }
    }

    /// Find the newest outbound INVITE transaction for a dialog.
    ///
    /// A challenged initial INVITE (401/407) and its authenticated retry share
    /// the same dialog record but are different transactions. RFC 3261 CANCEL
    /// must target the currently pending INVITE transaction, so prefer the
    /// mapped INVITE with the highest CSeq instead of returning an arbitrary
    /// DashMap entry.
    pub async fn find_latest_invite_transaction_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<TransactionKey> {
        use rvoip_sip_core::types::cseq::CSeq;

        if let Some(plan_id) = self
            .active_invite_failover_by_dialog
            .get(dialog_id)
            .map(|entry| *entry.value())
        {
            if let Some(plan) = self
                .invite_failover_plans
                .get(&plan_id)
                .map(|entry| entry.value().clone())
            {
                let plan = plan.lock().await;
                if plan.phase == InviteFailoverPlanPhase::Active {
                    if let Some(transaction_id) = plan.current_transaction.clone() {
                        debug!(
                            plan_id,
                            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id),
                            dialog=%dialog_id,
                            "Selected current retained initial-INVITE attempt for CANCEL"
                        );
                        return Some(transaction_id.as_ref().clone());
                    }
                }
            }
        }

        let candidates: Vec<TransactionKey> = self
            .dialog_invite_transactions
            .get(dialog_id)
            .map(|entry| {
                entry
                    .iter()
                    .filter(|tx_key| {
                        !tx_key.is_server()
                            && self
                                .transaction_to_dialog
                                .get(tx_key)
                                .is_some_and(|mapped| mapped.value() == dialog_id)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut best: Option<(u32, TransactionKey)> = None;
        for tx_key in candidates {
            let cseq = self
                .transaction_manager
                .original_request(&tx_key)
                .await
                .ok()
                .flatten()
                .and_then(|request| request.typed_header::<CSeq>().map(|cseq| cseq.seq))
                .unwrap_or_default();

            match &best {
                Some((best_cseq, _)) if cseq < *best_cseq => {}
                _ => best = Some((cseq, tx_key)),
            }
        }

        if let Some((cseq, tx_key)) = best {
            debug!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_key),
                dialog=%dialog_id,
                cseq,
                "Selected latest INVITE transaction for dialog"
            );
            Some(tx_key)
        } else {
            debug!(
                "No outbound INVITE transaction found for dialog {}",
                dialog_id
            );
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_invite_with_auth(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        auth_header_name: &str,
        auth_header_value: String,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
    ) -> DialogResult<TransactionKey> {
        let header_name = rvoip_sip_core::validation::authorization_header_name(auth_header_name)
            .map_err(|_| {
            crate::errors::DialogError::protocol_error(
                "unsupported INVITE authorization header name",
            )
        })?;
        let authorization = rvoip_sip_core::validation::validated_authorization_header(
            header_name,
            auth_header_value,
        )
        .map_err(|_| {
            crate::errors::DialogError::protocol_error(
                "INVITE authorization failed wire-safety validation",
            )
        })?;
        self.send_invite_with_auth_options(
            dialog_id,
            body,
            vec![authorization],
            extras,
            from_display,
            contact_override,
            None,
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_invite_with_auth_options(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        authorization_headers: Vec<TypedHeader>,
        extras: Vec<TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
        outbound_proxy_uri: Option<rvoip_sip_core::types::uri::Uri>,
        supported_100rel: bool,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;

        // Reject structural/header errors before CSeq mutation or DNS.
        for header in &authorization_headers {
            if !matches!(
                header.name(),
                HeaderName::Authorization | HeaderName::ProxyAuthorization
            ) {
                return Err(crate::errors::DialogError::protocol_error(
                    "INVITE auth retry contains a non-authorization credential header",
                ));
            }
        }
        let preview_dialog = self.get_dialog(dialog_id)?;
        let mut preview_headers = authorization_headers.clone();
        preview_headers.extend(extras.iter().cloned());
        crate::api::unified::validate_initial_invite_options(
            &crate::api::unified::InviteRequestOptions {
                from_uri: preview_dialog.local_uri.to_string(),
                to_uri: preview_dialog.remote_uri.to_string(),
                sdp: body
                    .as_ref()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
                call_id: Some(preview_dialog.call_id.clone()),
                from_display: from_display.clone(),
                contact_uri: contact_override.clone(),
                precomputed_authorization: None,
                outbound_proxy_uri: outbound_proxy_uri.clone(),
                supported_100rel,
                extra_headers: preview_headers,
            },
        )
        .map_err(|_| {
            crate::errors::DialogError::protocol_error(
                "authenticated INVITE failed preflight validation",
            )
        })?;
        let InitialInviteHeaderPlan {
            contact_uri,
            appended,
        } = plan_initial_invite_headers(contact_override, extras)?;
        let wire_plan = CandidateWirePlan {
            regenerate_stack_default_contact: contact_uri.is_none()
                && self.local_contact_uri().is_none(),
        };

        debug!("Resending INVITE with auth for dialog {}", dialog_id);

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            let template = dialog.create_request_template(Method::Invite);

            // Preserve the new INVITE's CSeq for later use by RAck (RFC 3262 §7.2).
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            // The challenge was a final response on the original INVITE, so no
            // remote tag was established. Rebuild as an initial INVITE with
            // the same Call-ID (dialog.create_request_template carries it).
            let planned_routes =
                planned_initial_invite_routes(outbound_proxy_uri.as_ref(), &template.route_set);
            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &planned_routes);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    from_display.as_deref().or(Some("User")),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            // The explicit outbound proxy is always the top Route. REGISTER
            // Service-Route entries follow it in their learned order.
            for route in &planned_routes {
                invite_builder = invite_builder.add_route(route.clone());
            }

            if let Some(uri) = contact_uri {
                invite_builder = invite_builder.contact(uri);
            } else if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = body_string {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            let mut request = invite_builder.build().map_err(|_error| {
                crate::errors::DialogError::InternalError {
                    message: safe_operation_failure("auth_retry_invite_build", "builder_error"),
                    context: None,
                }
            })?;

            // Re-inject the negotiated policy headers (100rel, session-timer)
            // just like the initial send does.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            if supported_100rel {
                inject_100rel_policy(&mut request, RelUsage::Supported);
            }
            if let Some((secs, min_se)) = self.config_session_timer_settings() {
                inject_session_timer_headers(&mut request, secs, min_se);
            }

            // SIP_API_DESIGN_2 §7.3 — preserve application-staged extras
            // across the 401/407 → retry hop. The original INVITE's
            // extras live in `pending_invite_options` on session-core's
            // SessionState; the caller forwards them here so the retry
            // wire form matches the initial send.
            append_validated_initial_invite_headers(&mut request, authorization_headers, appended)?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    crate::errors::DialogError::routing_error(
                        "Authenticated INVITE contains an unusable Route header",
                    )
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;

            if candidates.is_empty() {
                return Err(crate::errors::DialogError::routing_error(
                    "No address candidates for the exact INVITE next hop",
                ));
            }

            (candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. The auth-retry path
        // re-signs the PASSporT per attempt (fresh Via/branch) — the
        // helper fires `pre_send_request` inside the retry loop.
        let (transaction_id, _addr) = self
            .send_request_with_candidate_wire_plan(request, candidates, Some(dialog_id), wire_plan)
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Auth-retry INVITE sent via candidate failover path");
        Ok(transaction_id)
    }

    /// RFC 4028 §6 — resend an INVITE with a per-call `Session-Expires` /
    /// `Min-SE` override after the peer replied 422 Session Interval Too
    /// Small. The peer's `Min-SE` header dictates the required floor; callers
    /// pass it here together with the desired `Session-Expires` (typically
    /// set to `min_se` so the retry passes the first check).
    ///
    /// Mirrors `send_invite_with_auth` — reuses the original dialog's
    /// `Call-ID` + `From` tag, rebuilds as an initial INVITE (422 was a final
    /// response that did *not* establish a dialog), bumps CSeq via
    /// `Dialog::create_request_template`. The timer headers use the supplied
    /// overrides instead of the global `DialogManagerConfig` values.
    pub async fn send_invite_with_session_timer_override(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        session_secs: u32,
        min_se: u32,
    ) -> DialogResult<TransactionKey> {
        self.send_invite_with_session_timer_options(
            dialog_id,
            crate::api::unified::InviteAuthRetryOptions {
                sdp: body.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                ..Default::default()
            },
            session_secs,
            min_se,
        )
        .await
    }

    /// Structural 422 retry that retains the initial INVITE's routing,
    /// application headers, accumulated credentials and exact body.
    pub async fn send_invite_with_session_timer_options(
        &self,
        dialog_id: &DialogId,
        opts: crate::api::unified::InviteAuthRetryOptions,
        session_secs: u32,
        min_se: u32,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;

        for header in &opts.authorization_headers {
            if !matches!(
                header.name(),
                HeaderName::Authorization | HeaderName::ProxyAuthorization
            ) {
                return Err(crate::errors::DialogError::protocol_error(
                    "422 INVITE retry contains a non-authorization credential header",
                ));
            }
        }
        let preview_dialog = self.get_dialog(dialog_id)?;
        let mut preview_headers = opts.authorization_headers.clone();
        preview_headers.extend(opts.extra_headers.iter().cloned());
        crate::api::unified::validate_initial_invite_options(
            &crate::api::unified::InviteRequestOptions {
                from_uri: preview_dialog.local_uri.to_string(),
                to_uri: preview_dialog.remote_uri.to_string(),
                sdp: opts.sdp.clone(),
                call_id: Some(preview_dialog.call_id.clone()),
                from_display: opts.from_display.clone(),
                contact_uri: opts.contact_uri.clone(),
                precomputed_authorization: None,
                outbound_proxy_uri: opts.outbound_proxy_uri.clone(),
                supported_100rel: opts.supported_100rel,
                extra_headers: preview_headers,
            },
        )
        .map_err(|_| {
            crate::errors::DialogError::protocol_error(
                "422 INVITE retry failed preflight validation",
            )
        })?;
        let InitialInviteHeaderPlan {
            contact_uri,
            appended,
        } = plan_initial_invite_headers(opts.contact_uri, opts.extra_headers)?;
        let wire_plan = CandidateWirePlan {
            regenerate_stack_default_contact: contact_uri.is_none()
                && self.local_contact_uri().is_none(),
        };

        debug!(
            "Resending INVITE with session-timer override (SE={}, Min-SE={}) for dialog {}",
            session_secs, min_se, dialog_id
        );

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let template = dialog.create_request_template(Method::Invite);
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let planned_routes = planned_initial_invite_routes(
                opts.outbound_proxy_uri.as_ref(),
                &template.route_set,
            );
            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &planned_routes);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    opts.from_display.as_deref().or(Some("User")),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            for route in &planned_routes {
                invite_builder = invite_builder.add_route(route.clone());
            }

            if let Some(contact) = contact_uri {
                invite_builder = invite_builder.contact(contact);
            } else if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = opts.sdp {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            let mut request = invite_builder.build().map_err(|_error| {
                crate::errors::DialogError::InternalError {
                    message: safe_operation_failure(
                        "session_timer_retry_invite_build",
                        "builder_error",
                    ),
                    context: None,
                }
            })?;

            // Re-inject policy headers. 100rel follows the global config (the
            // peer's 100rel preference didn't change); session-timer headers
            // use the per-call overrides so the retry carries the peer's
            // required Min-SE floor.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            if opts.supported_100rel {
                inject_100rel_policy(&mut request, RelUsage::Supported);
            }
            inject_session_timer_headers(&mut request, session_secs, min_se);
            append_validated_initial_invite_headers(
                &mut request,
                opts.authorization_headers,
                appended,
            )?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    crate::errors::DialogError::routing_error(
                        "422 retry contains an unusable Route header",
                    )
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;

            if candidates.is_empty() {
                return Err(crate::errors::DialogError::routing_error(
                    "No address candidates for the exact 422-retry next hop",
                ));
            }
            (candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. STIR/SHAKEN re-signs
        // per attempt inside the helper since the 422-retry carries a
        // new CSeq + adjusted Session-Expires (the original PASSporT
        // no longer covers the canonical form).
        let (transaction_id, _addr) = self
            .send_request_with_candidate_wire_plan(request, candidates, Some(dialog_id), wire_plan)
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), session_secs, min_se, "422-retry INVITE sent via candidate failover path");
        Ok(transaction_id)
    }

    /// Send an *initial* INVITE on a freshly-created outgoing dialog, with
    /// caller-supplied extra headers appended to the wire request.
    ///
    /// Mirrors `send_invite_with_auth` / `send_invite_with_session_timer_override`
    /// in construction shape (rebuild the INVITE via `InviteBuilder`, inject
    /// global policy headers, send via `create_invite_client_transaction`)
    /// but is intended for the *first* transmission rather than a retry.
    /// Callers go through `crate::manager::unified::UnifiedManager::make_call_with_extra_headers`
    /// rather than calling this directly; this method is the layer that
    /// actually puts the bytes on the wire.
    ///
    /// `extra_headers` is validated and appended in exact caller order after
    /// stack-managed fields. Repeatable application fields retain duplicates;
    /// singleton collisions and unstructured Contact aliases are rejected.
    /// Typical contents:
    /// - `TypedHeader::PAssertedIdentity(...)` (RFC 3325) for trunk identity
    /// - `TypedHeader::PPreferredIdentity(...)` (RFC 3325) for asserted-identity preference
    /// - any other carrier-specific headers (`P-Charging-Vector`, etc.) the
    ///   application has already constructed.
    pub async fn send_initial_invite_with_extra_headers(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
        outbound_proxy_uri: Option<rvoip_sip_core::types::uri::Uri>,
        supported_100rel: bool,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;

        // Plan all caller-controlled headers before dialog mutation or route
        // resolution. A Contact alias cannot bypass the builder's one Contact
        // slot, and an explicit override cannot silently discard another
        // caller-supplied Contact.
        let InitialInviteHeaderPlan {
            contact_uri,
            appended,
        } = plan_initial_invite_headers(contact_override, extra_headers)?;
        let wire_plan = CandidateWirePlan {
            regenerate_stack_default_contact: contact_uri.is_none()
                && self.local_contact_uri().is_none(),
        };

        debug!(
            "Sending initial INVITE with {} extra header(s) for dialog {}",
            appended.len(),
            dialog_id
        );

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            let template = dialog.create_request_template(Method::Invite);
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let planned_routes =
                planned_initial_invite_routes(outbound_proxy_uri.as_ref(), &template.route_set);
            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &planned_routes);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    from_display.as_deref().or(Some("User")),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            for route in &planned_routes {
                invite_builder = invite_builder.add_route(route.clone());
            }

            if let Some(uri) = contact_uri {
                invite_builder = invite_builder.contact(uri);
            } else if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = body_string {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            let mut request = invite_builder.build().map_err(|_error| {
                crate::errors::DialogError::InternalError {
                    message: safe_operation_failure(
                        "initial_invite_with_extras_build",
                        "builder_error",
                    ),
                    context: None,
                }
            })?;

            // Re-inject the negotiated policy headers (100rel, session-timer),
            // mirroring `send_request_in_dialog`'s initial-INVITE arm.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            if supported_100rel {
                inject_100rel_policy(&mut request, RelUsage::Supported);
            }
            if let Some((secs, min_se)) = self.config_session_timer_settings() {
                inject_session_timer_headers(&mut request, secs, min_se);
            }

            append_validated_initial_invite_headers(&mut request, Vec::new(), appended)?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    crate::errors::DialogError::routing_error(
                        "Initial INVITE contains an unusable Route header",
                    )
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;

            if candidates.is_empty() {
                return Err(crate::errors::DialogError::routing_error(
                    "No address candidates for the exact INVITE next hop",
                ));
            }

            (candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. Walks the resolved
        // candidates in order on transport-level failure. STIR/SHAKEN
        // signing (`pre_send_request`) fires once per attempt inside
        // the helper since Via/branch change between attempts. The
        // helper also registers tx→dialog BEFORE send so 401-driven
        // auth retry and other fast-response paths can locate the
        // dialog without racing.
        let (transaction_id, _addr) = self
            .send_request_with_candidate_wire_plan(request, candidates, Some(dialog_id), wire_plan)
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Initial INVITE-with-extras sent");
        Ok(transaction_id)
    }

    /// Initial-INVITE send with an explicit lower-layer wire-boundary receipt.
    ///
    /// This preserves the legacy method above while giving lifecycle-aware
    /// callers enough information to distinguish exact local rollback from a
    /// required signaling teardown.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_initial_invite_with_wire_receipt(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
        outbound_proxy_uri: Option<rvoip_sip_core::types::uri::Uri>,
        supported_100rel: bool,
    ) -> Result<TransactionKey, InitialInviteSendFailure> {
        match self
            .send_initial_invite_with_extra_headers(
                dialog_id,
                body,
                extra_headers,
                from_display,
                contact_override,
                outbound_proxy_uri,
                supported_100rel,
            )
            .await
        {
            Ok(transaction_id) => Ok(transaction_id),
            Err(error) => {
                for plan_id in self.invite_failover_plan_ids_for_dialog(dialog_id) {
                    let Some(plan) = self
                        .invite_failover_plans
                        .get(&plan_id)
                        .map(|entry| entry.value().clone())
                    else {
                        continue;
                    };
                    let plan = plan.lock().await;
                    if plan.id == plan_id && &plan.dialog_id == dialog_id && plan.wire_attempted {
                        return Err(InitialInviteSendFailure::Unknown(error));
                    }
                }
                Err(InitialInviteSendFailure::ZeroWire(error))
            }
        }
    }

    async fn register_invite_failover_plan(
        &self,
        dialog_id: &DialogId,
        request: Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        wire_plan: CandidateWirePlan,
    ) -> DialogResult<std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>> {
        // Retained plans are already reaped by the manager's one-second
        // maintenance task and by the bounded insert cadence below. Scanning
        // and cloning the complete 90-second retention set for every INVITE
        // makes admission O(retained calls) and turns sustained CPS into
        // quadratic work. Only force an admission-side reap when the retained
        // capacity is actually full, so expired entries get one final chance
        // to free a slot before overload is reported.
        if self.invite_failover_plans.len() >= self.invite_failover_plan_capacity {
            self.prune_invite_failover_state().await;
        }
        let (plan, inserts) = {
            // Registration is one linearized ownership transition. Two callers
            // cannot both replace the active owner for one dialog, and drain
            // cannot begin between admission and index publication.
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.is_accepting_work() {
                return Err(crate::errors::DialogError::InvalidState {
                    expected: "dialog manager running".to_string(),
                    actual: "dialog manager draining or stopped".to_string(),
                });
            }
            if self
                .active_invite_failover_by_dialog
                .contains_key(dialog_id)
            {
                return Err(crate::errors::DialogError::InvalidState {
                    expected: "no active initial INVITE operation".to_string(),
                    actual: "active initial INVITE operation".to_string(),
                });
            }
            if self.active_invite_failover_by_dialog.len()
                >= self.invite_failover_active_plan_capacity
                || self.invite_failover_plans.len() >= self.invite_failover_plan_capacity
            {
                return Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("invite_failover_admission", "capacity"),
                });
            }

            if !Self::try_reserve_invite_failover_capacity(
                &self.invite_failover_plan_reservations,
                1,
                self.invite_failover_plan_capacity,
            ) {
                return Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("invite_failover_admission", "capacity"),
                });
            }
            if !Self::try_reserve_invite_failover_capacity(
                &self.invite_failover_attempt_reservations,
                candidates.len(),
                self.invite_failover_attempt_capacity,
            ) {
                Self::release_invite_failover_reservation(
                    &self.invite_failover_plan_reservations,
                    1,
                );
                return Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("invite_failover_admission", "capacity"),
                });
            }

            let plan_id = self
                .next_invite_failover_plan_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let now = Instant::now();
            let setup_deadline = now
                + self
                    .transaction_manager
                    .timer_settings()
                    .transaction_timeout;
            let expires_at = setup_deadline + INVITE_FAILOVER_PLAN_TTL;
            let expiry_generation = 1;
            let reserved_candidate_count = candidates.len();
            let plan = std::sync::Arc::new(tokio::sync::Mutex::new(InviteFailoverPlan {
                id: plan_id,
                dialog_id: dialog_id.clone(),
                active_payload: Some(Box::new(InviteFailoverActivePayload::new(
                    request,
                    candidates,
                    wire_plan,
                    setup_deadline,
                ))),
                reserved_attempt_slots: reserved_candidate_count,
                current_transaction: None,
                wire_attempted: false,
                phase: InviteFailoverPlanPhase::Active,
                retained_attempts: Box::new([]),
                accepted_candidate_index: None,
                accepted_to_tag: None,
                fork_cleanup: None,
                expires_at,
                expiry_generation,
            }));
            self.invite_failover_plans.insert(plan_id, plan.clone());
            if !self
                .invite_failover_expiry_scheduler
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .schedule(
                    plan_id,
                    expiry_generation,
                    expires_at,
                    InviteFailoverPlanPhase::Active,
                )
            {
                self.invite_failover_plans.remove(&plan_id);
                Self::release_invite_failover_reservation(
                    &self.invite_failover_plan_reservations,
                    1,
                );
                Self::release_invite_failover_reservation(
                    &self.invite_failover_attempt_reservations,
                    reserved_candidate_count,
                );
                return Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("invite_failover_admission", "expiry_capacity"),
                });
            }
            self.index_invite_failover_plan_locked(dialog_id, plan_id);
            self.active_invite_failover_by_dialog
                .insert(dialog_id.clone(), plan_id);
            let inserts = self
                .invite_failover_insert_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            (plan, inserts)
        };
        if inserts % INVITE_FAILOVER_PRUNE_INTERVAL == 0 {
            self.prune_invite_failover_state().await;
        }
        Ok(plan)
    }

    fn try_reserve_invite_failover_capacity(
        reservations: &std::sync::atomic::AtomicUsize,
        amount: usize,
        capacity: usize,
    ) -> bool {
        reservations
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(amount).filter(|next| *next <= capacity),
            )
            .is_ok()
    }

    fn release_invite_failover_reservation(
        reservations: &std::sync::atomic::AtomicUsize,
        amount: usize,
    ) {
        let _ = reservations.fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |current| Some(current.saturating_sub(amount)),
        );
    }

    fn set_invite_attempt_outcome(
        plan: &mut InviteFailoverPlan,
        transaction_id: &TransactionKey,
        outcome: InviteFailoverAttemptOutcome,
    ) {
        if let Some(active) = plan.active_payload.as_mut() {
            if let Some(attempt) = active
                .attempts
                .iter_mut()
                .find(|attempt| attempt.transaction_id.as_ref() == transaction_id)
            {
                attempt.outcome = outcome;
            }
        }
    }

    fn invite_setup_timeout(operation: &'static str) -> crate::errors::DialogError {
        crate::errors::DialogError::TimeoutError {
            operation: safe_operation_failure(operation, "setup_deadline"),
        }
    }

    fn abandon_invite_candidate_generation_locked(
        &self,
        plan: &mut InviteFailoverPlan,
        generation: u64,
        transaction_hint: Option<&TransactionKey>,
    ) -> Option<TransactionKey> {
        let owns_pending = plan
            .active_payload
            .as_ref()
            .and_then(|active| active.pending_candidate)
            .is_some_and(|(pending_generation, _)| pending_generation == generation);
        let owns_current = plan
            .active_payload
            .as_ref()
            .is_some_and(|active| active.current_send_generation == Some(generation));
        if !owns_pending && !owns_current {
            // A transaction created just before another operation cancelled
            // the pending generation was never published in the plan. It
            // still needs compensation, whereas an indexed attempt now owned
            // by a response/failover path must be left alone.
            return transaction_hint
                .filter(|transaction_id| {
                    !plan
                        .active_payload
                        .as_ref()
                        .into_iter()
                        .flat_map(|active| active.attempts.iter())
                        .any(|attempt| attempt.transaction_id.as_ref() == *transaction_id)
                })
                .cloned();
        }

        let _registry = self
            .invite_failover_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let transaction_id = if owns_current {
            plan.current_transaction
                .as_deref()
                .cloned()
                .or_else(|| transaction_hint.cloned())
        } else {
            transaction_hint.cloned()
        };
        if let Some(transaction_id) = transaction_id.as_ref() {
            Self::set_invite_attempt_outcome(
                plan,
                transaction_id,
                InviteFailoverAttemptOutcome::TransportError,
            );
        }
        if owns_pending {
            if let Some(active) = plan.active_payload.as_mut() {
                active.pending_candidate = None;
            }
        }
        if owns_current {
            plan.current_transaction = None;
            if let Some(active) = plan.active_payload.as_mut() {
                active.current_candidate_index = None;
                active.current_send_generation = None;
            }
        }
        if plan.phase == InviteFailoverPlanPhase::Active {
            plan.transition_to_terminal(InviteFailoverPlanPhase::Closed);
            self.schedule_invite_failover_plan_expiry(
                plan,
                Instant::now() + INVITE_FAILOVER_PLAN_TTL,
            );
        }
        self.active_invite_failover_by_dialog
            .remove_if(&plan.dialog_id, |_, active_plan_id| {
                *active_plan_id == plan.id
            });
        if let Some(transaction_id) = transaction_id.as_ref() {
            self.remove_invite_failover_attempt_locked(transaction_id, Some(plan.id));
            self.unlink_transaction_from_dialog_indexed(transaction_id);
        }
        transaction_id
    }

    async fn abandon_invite_candidate_generation(
        &self,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        generation: u64,
        transaction_hint: Option<&TransactionKey>,
    ) {
        let terminate = {
            let mut plan = plan.lock().await;
            self.abandon_invite_candidate_generation_locked(&mut plan, generation, transaction_hint)
        };
        if let Some(transaction_id) = terminate {
            self.compensate_invite_candidate_transaction(&transaction_id, false)
                .await;
        }
    }

    async fn close_pending_invite_candidate(
        &self,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        generation: u64,
        phase: InviteFailoverPlanPhase,
    ) {
        let mut plan = plan.lock().await;
        if plan
            .active_payload
            .as_ref()
            .and_then(|active| active.pending_candidate)
            .is_some_and(|(pending_generation, _)| pending_generation == generation)
        {
            if let Some(active) = plan.active_payload.as_mut() {
                active.pending_candidate = None;
            }
            if plan.phase == InviteFailoverPlanPhase::Active {
                self.close_active_invite_failover_plan(&mut plan, phase);
            }
        }
    }

    async fn compensate_invite_candidate_transaction(
        &self,
        transaction_id: &TransactionKey,
        unlink_attempt: bool,
    ) {
        if unlink_attempt {
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.remove_invite_failover_attempt_locked(transaction_id, None);
            self.unlink_transaction_from_dialog_indexed(transaction_id);
        }
        if tokio::time::timeout(
            INVITE_CANDIDATE_COMPENSATION_TIMEOUT,
            self.transaction_manager
                .terminate_transaction(transaction_id),
        )
        .await
        .is_err()
        {
            warn!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                "Timed out compensating an incomplete initial-INVITE candidate"
            );
        }
    }

    async fn retain_wire_unknown_invite_candidate(
        &self,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
        generation: u64,
        transaction_id: &TransactionKey,
        outcome: InviteFailoverAttemptOutcome,
    ) -> bool {
        let mut plan = plan.lock().await;
        if !plan
            .active_payload
            .as_ref()
            .is_some_and(|active| active.current_send_generation == Some(generation))
            || plan.current_transaction.as_deref() != Some(transaction_id)
        {
            return false;
        }
        Self::set_invite_attempt_outcome(&mut plan, transaction_id, outcome);
        // Preserve current_transaction plus tx→dialog/request route indexes so
        // a compatibility cleanup supervisor can construct the exact CANCEL.
        self.close_active_invite_failover_plan(&mut plan, InviteFailoverPlanPhase::WireUnknown);
        true
    }

    async fn send_invite_failover_candidate(
        &self,
        plan: &std::sync::Arc<tokio::sync::Mutex<InviteFailoverPlan>>,
    ) -> Result<(TransactionKey, SocketAddr), InviteCandidateSendError> {
        use crate::manager::RequestLifecycle;

        let lease = {
            let mut plan = plan.lock().await;
            let owner_is_current = self
                .active_invite_failover_by_dialog
                .get(&plan.dialog_id)
                .is_some_and(|owner| *owner.value() == plan.id);
            if !self.is_accepting_work()
                || plan.phase != InviteFailoverPlanPhase::Active
                || !owner_is_current
            {
                return Err(InviteCandidateSendError::Fatal(
                    crate::errors::DialogError::InvalidState {
                        expected: "active initial INVITE plan on a running manager".to_string(),
                        actual: "inactive plan or draining manager".to_string(),
                    },
                ));
            }
            if plan
                .active_payload
                .as_ref()
                .is_some_and(|active| active.pending_candidate.is_some())
                || plan.current_transaction.is_some()
            {
                return Err(InviteCandidateSendError::Fatal(
                    crate::errors::DialogError::InvalidState {
                        expected: "idle initial INVITE candidate slot".to_string(),
                        actual: "candidate operation already active".to_string(),
                    },
                ));
            }
            let plan_id = plan.id;
            let dialog_id = plan.dialog_id.clone();
            let Some(active_payload) = plan.active_payload.as_mut() else {
                self.close_active_invite_failover_plan(&mut plan, InviteFailoverPlanPhase::Closed);
                return Err(InviteCandidateSendError::Fatal(
                    crate::errors::DialogError::InvalidState {
                        expected: "active initial INVITE payload".to_string(),
                        actual: "compacted initial INVITE plan".to_string(),
                    },
                ));
            };
            if active_payload.setup_deadline <= Instant::now() {
                self.close_active_invite_failover_plan(
                    &mut plan,
                    InviteFailoverPlanPhase::Exhausted,
                );
                return Err(InviteCandidateSendError::Fatal(Self::invite_setup_timeout(
                    "invite_candidate_reserve",
                )));
            }
            let candidate_index = active_payload.next_candidate_index;
            let Some(target) = active_payload.candidates.get(candidate_index).cloned() else {
                self.close_active_invite_failover_plan(
                    &mut plan,
                    InviteFailoverPlanPhase::Exhausted,
                );
                return Err(InviteCandidateSendError::Fatal(
                    crate::errors::DialogError::routing_error("INVITE candidate missing"),
                ));
            };
            let request = active_payload.request.clone();
            let wire_plan = active_payload.wire_plan;
            let remaining_candidate_count = active_payload
                .candidates
                .len()
                .saturating_sub(candidate_index)
                .max(1);
            let generation = active_payload.next_send_generation.wrapping_add(1).max(1);
            active_payload.next_send_generation = generation;
            active_payload.next_candidate_index = candidate_index + 1;
            active_payload.pending_candidate = Some((generation, candidate_index));
            InviteCandidateSendLease {
                plan_id,
                dialog_id,
                generation,
                candidate_index,
                target,
                request,
                wire_plan,
                setup_deadline: active_payload.setup_deadline,
                remaining_candidate_count,
            }
        };
        let mut cancellation_guard =
            InviteCandidateCancellationGuard::new(self, plan, lease.generation);

        let mut request = match finalize_request_for_candidate(
            self,
            &lease.request,
            &lease.target,
            lease.wire_plan,
        ) {
            Ok(request) => request,
            Err(error) => {
                self.close_pending_invite_candidate(
                    plan,
                    lease.generation,
                    InviteFailoverPlanPhase::Closed,
                )
                .await;
                return Err(InviteCandidateSendError::Fatal(error));
            }
        };
        match tokio::time::timeout_at(
            tokio::time::Instant::from_std(lease.setup_deadline),
            self.pre_send_request(&mut request, lease.target.addr),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.close_pending_invite_candidate(
                    plan,
                    lease.generation,
                    InviteFailoverPlanPhase::Closed,
                )
                .await;
                return Err(InviteCandidateSendError::Fatal(error));
            }
            Err(_) => {
                self.close_pending_invite_candidate(
                    plan,
                    lease.generation,
                    InviteFailoverPlanPhase::Exhausted,
                )
                .await;
                return Err(InviteCandidateSendError::Fatal(Self::invite_setup_timeout(
                    "invite_candidate_pre_send",
                )));
            }
        }

        let sent_request = request.clone();
        let request_key = crate::manager::core::outbound_request_key(&sent_request);
        let mut request_route = rvoip_sip_transport::TransportRoute::new(lease.target.addr)
            .with_transport_type(lease.target.transport);
        if let Some(authority) = lease.target.authority.clone() {
            request_route.authority = Some(authority);
        }
        let Some(remaining_setup_budget) = lease
            .setup_deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
        else {
            self.close_pending_invite_candidate(
                plan,
                lease.generation,
                InviteFailoverPlanPhase::Exhausted,
            )
            .await;
            return Err(InviteCandidateSendError::Fatal(Self::invite_setup_timeout(
                "invite_candidate_transaction_create",
            )));
        };
        let attempt_timeout = remaining_setup_budget / (lease.remaining_candidate_count as u32);
        let transaction_id = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(lease.setup_deadline),
            self.transaction_manager
                .create_client_transaction_on_route_with_timeout_and_owner(
                    request,
                    request_route,
                    attempt_timeout,
                ),
        )
        .await
        {
            Ok(Ok(allocation)) => allocation,
            Ok(Err(_)) => {
                self.close_pending_invite_candidate(
                    plan,
                    lease.generation,
                    InviteFailoverPlanPhase::Closed,
                )
                .await;
                return Err(InviteCandidateSendError::Fatal(
                    crate::errors::DialogError::TransactionError {
                        message: safe_operation_failure(
                            "invite_candidate_transaction_create",
                            "transaction_error",
                        ),
                    },
                ));
            }
            Err(_) => {
                self.close_pending_invite_candidate(
                    plan,
                    lease.generation,
                    InviteFailoverPlanPhase::Exhausted,
                )
                .await;
                return Err(InviteCandidateSendError::Fatal(Self::invite_setup_timeout(
                    "invite_candidate_transaction_create",
                )));
            }
        };
        let (transaction_id, failover_admission_owner) = transaction_id;
        cancellation_guard.set_transaction(&transaction_id);

        let committed = {
            let mut plan = plan.lock().await;
            let _registry = self
                .invite_failover_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let owner_is_current = self
                .active_invite_failover_by_dialog
                .get(&lease.dialog_id)
                .is_some_and(|owner| *owner.value() == lease.plan_id);
            if self.is_accepting_work()
                && owner_is_current
                && plan.id == lease.plan_id
                && plan.phase == InviteFailoverPlanPhase::Active
                && plan.active_payload.as_ref().is_some_and(|active| {
                    active.pending_candidate == Some((lease.generation, lease.candidate_index))
                        && active.setup_deadline > Instant::now()
                })
            {
                self.link_outbound_transaction_to_dialog_indexed(
                    &transaction_id,
                    &lease.dialog_id,
                    &sent_request,
                );
                let retained_transaction = std::sync::Arc::new(transaction_id.clone());
                self.index_invite_failover_attempt_locked(
                    retained_transaction.clone(),
                    InviteFailoverAttemptIndex {
                        plan_id: lease.plan_id,
                        dialog_id: lease.dialog_id.clone(),
                        candidate_index: lease.candidate_index,
                        _admission_owner: Some(failover_admission_owner),
                    },
                );
                plan.current_transaction = Some(retained_transaction.clone());
                let active = plan
                    .active_payload
                    .as_mut()
                    .expect("active payload validated above");
                active.pending_candidate = None;
                active.current_candidate_index = Some(lease.candidate_index);
                active.current_send_generation = Some(lease.generation);
                active.provisional_seen = false;
                active.attempts.push(InviteFailoverAttempt {
                    transaction_id: retained_transaction,
                    outcome: InviteFailoverAttemptOutcome::Active,
                });
                true
            } else {
                false
            }
        };
        if !committed {
            self.close_pending_invite_candidate(
                plan,
                lease.generation,
                InviteFailoverPlanPhase::Closed,
            )
            .await;
            self.compensate_invite_candidate_transaction(&transaction_id, false)
                .await;
            return Err(InviteCandidateSendError::Fatal(
                crate::errors::DialogError::InvalidState {
                    expected: "current admitted initial INVITE generation".to_string(),
                    actual: "stale generation or draining manager".to_string(),
                },
            ));
        }

        let stale_before_wire = {
            let mut plan = plan.lock().await;
            if !plan
                .active_payload
                .as_ref()
                .is_some_and(|active| active.current_send_generation == Some(lease.generation))
                || plan.current_transaction.as_deref() != Some(&transaction_id)
            {
                // This transaction was committed to the retained indexes but
                // this lease has not crossed the wire boundary yet. A
                // concurrent cancel/drain may have compacted the plan and
                // invalidated the generation; detach the zero-wire attempt so
                // it cannot occupy a retained route until TTL. The guard stays
                // armed across async compensation and retries termination if
                // this future is itself cancelled.
                if plan.current_transaction.as_deref() == Some(&transaction_id) {
                    plan.current_transaction = None;
                    if let Some(active) = plan.active_payload.as_mut() {
                        active.current_candidate_index = None;
                    }
                }
                if let Some(active) = plan.active_payload.as_mut() {
                    active
                        .attempts
                        .retain(|attempt| attempt.transaction_id.as_ref() != &transaction_id);
                }
                true
            } else {
                // Monotonic boundary: after this store, even an immediate
                // transport error is conservatively wire-unknown.
                plan.wire_attempted = true;
                false
            }
        };
        if stale_before_wire {
            self.compensate_invite_candidate_transaction(&transaction_id, true)
                .await;
            cancellation_guard.disarm();
            return Err(InviteCandidateSendError::Fatal(
                crate::errors::DialogError::InvalidState {
                    expected: "current committed INVITE wire lease".to_string(),
                    actual: "stale INVITE wire lease".to_string(),
                },
            ));
        }

        let result = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(lease.setup_deadline),
            self.transaction_manager.send_request(&transaction_id),
        )
        .await
        {
            Ok(Ok(())) => {
                self.record_outbound_transport_context(
                    &transaction_id,
                    request_key,
                    lease.target.transport,
                    lease.target.addr,
                );
                match tokio::time::timeout_at(
                    tokio::time::Instant::from_std(lease.setup_deadline),
                    self.post_send_request(&sent_request, lease.target.addr),
                )
                .await
                {
                    Ok(Ok(())) => Ok((transaction_id, lease.target.addr)),
                    Ok(Err(error)) => {
                        let still_current = self
                            .retain_wire_unknown_invite_candidate(
                                plan,
                                lease.generation,
                                &transaction_id,
                                InviteFailoverAttemptOutcome::TransportError,
                            )
                            .await;
                        if still_current {
                            Err(InviteCandidateSendError::WireUnknown(error))
                        } else {
                            Ok((transaction_id, lease.target.addr))
                        }
                    }
                    Err(_) => {
                        let still_current = self
                            .retain_wire_unknown_invite_candidate(
                                plan,
                                lease.generation,
                                &transaction_id,
                                InviteFailoverAttemptOutcome::TransactionTimeout,
                            )
                            .await;
                        if still_current {
                            Err(InviteCandidateSendError::WireUnknown(
                                Self::invite_setup_timeout("invite_candidate_post_send"),
                            ))
                        } else {
                            Ok((transaction_id, lease.target.addr))
                        }
                    }
                }
            }
            Ok(Err(_error)) => {
                self.retain_wire_unknown_invite_candidate(
                    plan,
                    lease.generation,
                    &transaction_id,
                    InviteFailoverAttemptOutcome::ImmediateTransportFailure,
                )
                .await;
                Err(InviteCandidateSendError::WireUnknown(
                    crate::errors::DialogError::TransactionError {
                        message: safe_operation_failure("invite_candidate_send", "wire_unknown"),
                    },
                ))
            }
            Err(_) => {
                self.retain_wire_unknown_invite_candidate(
                    plan,
                    lease.generation,
                    &transaction_id,
                    InviteFailoverAttemptOutcome::TransactionTimeout,
                )
                .await;
                Err(InviteCandidateSendError::WireUnknown(
                    Self::invite_setup_timeout("invite_candidate_send"),
                ))
            }
        };
        if result.is_ok() || matches!(&result, Err(error) if error.retains_wire_owner()) {
            cancellation_guard.disarm();
        }
        result
    }

    async fn send_initial_invite_plan(
        &self,
        dialog_id: &DialogId,
        request: Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        wire_plan: CandidateWirePlan,
    ) -> DialogResult<(TransactionKey, SocketAddr)> {
        let plan = self
            .register_invite_failover_plan(dialog_id, request, candidates, wire_plan)
            .await?;
        match self.send_invite_failover_candidate(&plan).await {
            Ok(sent) => Ok(sent),
            Err(error) => Err(error.into_dialog_error()),
        }
    }

    /// Send a freshly-built request via a new client transaction,
    /// retrying with the next `ResolvedTarget` on transport-level
    /// failure (RFC 3263 §4.3).
    ///
    /// On a recoverable transport error from `send_request` (the
    /// transaction terminated immediately, transport error event, or
    /// general transport failure), the helper destroys the failed
    /// transaction by leaving it for the normal cleanup path and
    /// creates a fresh client transaction targeted at the next
    /// candidate. Non-transport errors (parse failures, state-machine
    /// errors, signer errors) fail fast — retrying on a different
    /// candidate would not help.
    ///
    /// For INVITE specifically, fires
    /// `RequestLifecycle::pre_send_request` once per attempt so the
    /// installed signer sees the per-attempt request (Via / branch
    /// differ across attempts).
    ///
    /// Returns the transaction key of the first attempt that
    /// successfully reached `send_request`-Ok, along with the
    /// [`SocketAddr`] that succeeded. Caller is responsible for
    /// registering it into `transaction_to_dialog`.
    ///
    /// An empty candidate set fails closed. The caller must resolve the exact
    /// next-hop URI (top Route when present, otherwise Request-URI); falling
    /// back to an independently resolved remote target could bypass a proxy.
    /// RFC 3261 §17.1.1.3 normal-termination after 2xx on a fast loopback is
    /// treated as success.
    ///
    /// `tx_to_dialog`, when supplied, is the dialog id to register
    /// against the freshly-created transaction *before* `send_request`
    /// fires. Critical for paths whose response handling (e.g.,
    /// 401-driven auth retry, dialog state transitions) looks the
    /// dialog up via `transaction_to_dialog`: registering AFTER send
    /// would race with a fast response and the dialog would be
    /// unreachable. Pass `None` for stateless sends (e.g. the
    /// proxy's per-leg failover).
    pub async fn send_request_with_candidate_failover(
        &self,
        request: rvoip_sip_core::Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        tx_to_dialog: Option<&DialogId>,
    ) -> DialogResult<(TransactionKey, std::net::SocketAddr)> {
        self.send_request_with_candidate_wire_plan(
            request,
            candidates,
            tx_to_dialog,
            CandidateWirePlan::default(),
        )
        .await
    }

    /// Candidate failover with explicit ownership of stack-generated wire
    /// fields. Application-authored Contact values must use the default plan.
    pub async fn send_request_with_candidate_wire_plan(
        &self,
        request: rvoip_sip_core::Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        tx_to_dialog: Option<&DialogId>,
        wire_plan: CandidateWirePlan,
    ) -> DialogResult<(TransactionKey, std::net::SocketAddr)> {
        self.send_request_with_candidate_wire_plan_owned(
            request,
            candidates,
            tx_to_dialog,
            wire_plan,
        )
        .await
        .map(|(transaction_id, destination, _completion)| (transaction_id, destination))
    }

    async fn send_request_with_candidate_wire_plan_owned(
        &self,
        request: rvoip_sip_core::Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        tx_to_dialog: Option<&DialogId>,
        wire_plan: CandidateWirePlan,
    ) -> DialogResult<(
        TransactionKey,
        std::net::SocketAddr,
        Option<crate::transaction::ClientTransactionCompletionHandle>,
    )> {
        use crate::manager::RequestLifecycle;

        let _operation = self.enter_invite_failover_operation().ok_or_else(|| {
            crate::errors::DialogError::InvalidState {
                expected: "dialog manager running".to_string(),
                actual: "dialog manager draining or stopped".to_string(),
            }
        })?;
        let method = request.method();
        if candidates.is_empty() {
            return Err(crate::errors::DialogError::routing_error(
                "No address candidates for the exact request next hop",
            ));
        }

        let is_initial_invite = method == Method::Invite
            && request
                .to()
                .is_some_and(|to_header| to_header.tag().is_none());
        if let (true, Some(dialog_id)) = (is_initial_invite, tx_to_dialog) {
            // Keep the retained-plan state machine off callers' concrete
            // async frames. Several high-level state machines await this
            // helper from large match expressions; embedding the complete
            // failover future there can exhaust Tokio's default worker stack
            // even when the runtime takes a different match arm.
            return Box::pin(
                self.send_initial_invite_plan(dialog_id, request, candidates, wire_plan),
            )
            .await
            .map(|(transaction_id, destination)| (transaction_id, destination, None));
        }

        let total = candidates.len();
        let mut last_err: Option<crate::errors::DialogError> = None;

        for (idx, target) in candidates.iter().enumerate() {
            let attempt = idx + 1;
            let mut req = finalize_request_for_candidate(self, &request, target, wire_plan)?;

            if method == Method::Invite {
                if let Err(e) = self.pre_send_request(&mut req, target.addr).await {
                    return Err(e);
                }
            }

            let sent_request = req.clone();
            let request_key = crate::manager::core::outbound_request_key(&sent_request);

            let mut request_route = rvoip_sip_transport::TransportRoute::new(target.addr)
                .with_transport_type(target.transport);
            if let Some(authority) = target.authority.clone() {
                request_route.authority = Some(authority);
            }
            let tx_result = self
                .transaction_manager
                .create_client_transaction_on_route_with_completion(req, request_route)
                .await;
            let (tx_id, completion) = match tx_result {
                Ok(created) => created,
                Err(_error) => {
                    last_err = Some(crate::errors::DialogError::TransactionError {
                        message: safe_method_operation_failure(
                            "candidate_transaction_create",
                            "transaction_error",
                            &method,
                        ),
                    });
                    continue;
                }
            };

            // Register tx→dialog mapping BEFORE send so a fast
            // response (e.g. 401 hitting loopback before send_request
            // returns) can locate the dialog. Removed on failed
            // attempts so the next candidate's tx replaces it.
            if let Some(dialog_id) = tx_to_dialog {
                self.link_outbound_transaction_to_dialog_indexed(&tx_id, dialog_id, &sent_request);
            }

            match self.transaction_manager.send_request(&tx_id).await {
                Ok(()) => {
                    self.record_outbound_transport_context(
                        &tx_id,
                        request_key,
                        target.transport,
                        target.addr,
                    );
                    self.post_send_request(&sent_request, target.addr).await?;
                    if attempt > 1 {
                        debug!(
                            "RFC 3263 §4.3: candidate {}/{} ({}) succeeded after {} prior failure(s)",
                            attempt,
                            total,
                            target.addr,
                            attempt - 1
                        );
                    }
                    return Ok((tx_id, target.addr, Some(completion)));
                }
                Err(e) => {
                    let is_transport_failure =
                        matches!(&e, crate::transaction::error::Error::TransportError { .. });
                    if is_transport_failure && idx + 1 < total {
                        debug!(
                            attempt,
                            total,
                            destination=%target.addr,
                            error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                            "Candidate failed with transport error; trying next"
                        );
                        // Drop the failed-leg mapping so the next
                        // attempt's tx is the canonical one for this
                        // dialog.
                        if tx_to_dialog.is_some() {
                            self.unlink_transaction_from_dialog_indexed(&tx_id);
                        }
                        if let Err(_cleanup_error) =
                            self.transaction_manager.terminate_transaction(&tx_id).await
                        {
                            debug!(
                                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id),
                                "Failed candidate transaction was already retired"
                            );
                        }
                        last_err = Some(crate::errors::DialogError::TransactionError {
                            message: safe_method_operation_failure(
                                "candidate_request_send",
                                "transport_error",
                                &method,
                            ),
                        });
                        continue;
                    }

                    if tx_to_dialog.is_some() {
                        self.unlink_transaction_from_dialog_indexed(&tx_id);
                    }
                    if let Err(_cleanup_error) =
                        self.transaction_manager.terminate_transaction(&tx_id).await
                    {
                        debug!(
                            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id),
                            "Failed client transaction was already retired"
                        );
                    }

                    return Err(crate::errors::DialogError::TransactionError {
                        message: safe_method_operation_failure(
                            "candidate_request_send",
                            "transaction_error",
                            &method,
                        ),
                    });
                }
            }
        }

        Err(
            last_err.unwrap_or_else(|| crate::errors::DialogError::TransactionError {
                message: safe_method_operation_failure("candidate_failover", "exhausted", &method),
            }),
        )
    }
}

/// A response qualifies for RFC 3262 reliable-provisional wrapping when it
/// is a non-100 provisional (101–199) and carries a body (typically SDP
/// early media). 100 Trying is hop-by-hop and never reliable; bodiless
/// 180/183 are still sent unreliably since there's nothing to protect.
pub fn should_send_reliably(response: &Response) -> bool {
    let code = response.status_code();
    (101..200).contains(&code) && !response.body().is_empty()
}

/// Append RFC 4028 session-timer headers to an outgoing INVITE: a
/// `Session-Expires: <secs>;refresher=uac` (caller-side refresh by default —
/// keeps NAT pinholes alive on the UAC), a `Min-SE: <min_se>`, and the
/// `timer` option tag in `Supported`. No-op if `secs` is 0.
pub fn inject_session_timer_headers(request: &mut Request, secs: u32, min_se: u32) {
    use rvoip_sip_core::types::min_se::MinSE;
    use rvoip_sip_core::types::session_expires::{Refresher, SessionExpires};
    use rvoip_sip_core::types::{Supported, TypedHeader};

    if secs == 0 {
        return;
    }

    request
        .headers
        .push(TypedHeader::SessionExpires(SessionExpires::new(
            secs,
            Some(Refresher::Uac),
        )));
    request.headers.push(TypedHeader::MinSE(MinSE::new(min_se)));

    let mut found = false;
    for header in request.headers.iter_mut() {
        if let TypedHeader::Supported(ref mut sup) = header {
            if !sup.option_tags.iter().any(|t| t == "timer") {
                sup.option_tags.push("timer".to_string());
            }
            found = true;
            break;
        }
    }
    if !found {
        request
            .headers
            .push(TypedHeader::Supported(Supported::new(vec![
                "timer".to_string()
            ])));
    }
}

/// Append `Require: 100rel` and `RSeq: <rseq>` to an outgoing 18x. Creates
/// the `Require` header if absent, appends the tag otherwise.
pub fn inject_reliable_provisional_headers(response: &mut Response, rseq: u32) {
    use rvoip_sip_core::types::rseq::RSeq;
    use rvoip_sip_core::types::{Require, TypedHeader};

    let mut updated = false;
    for header in response.headers.iter_mut() {
        if let TypedHeader::Require(ref mut req) = header {
            if !req.requires("100rel") {
                req.add_tag("100rel");
            }
            updated = true;
            break;
        }
    }
    if !updated {
        response
            .headers
            .push(TypedHeader::Require(Require::with_tag("100rel")));
    }
    response.headers.push(TypedHeader::RSeq(RSeq::new(rseq)));
}

impl TransactionHelpers for DialogManager {
    /// Associate a transaction with a dialog
    ///
    /// Creates the mapping between transactions and dialogs for proper message routing.
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId) {
        self.link_transaction_to_dialog_indexed(transaction_id, dialog_id);
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Linked transaction to dialog");
    }

    /// Create ACK for 2xx response using transaction-core helpers
    ///
    /// Uses transaction-core's ACK creation helpers while maintaining dialog-core concerns.
    async fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<Request> {
        debug!("Creating ACK for 2xx response using transaction-core helpers");

        // Use transaction-core's helper method to create ACK for 2xx response
        // This ensures proper ACK construction according to RFC 3261
        let ack_request = self
            .transaction_manager
            .create_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|_error| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("ack_for_success_build", "transaction_error"),
            })?;

        debug!("Successfully created ACK for 2xx response");
        Ok(ack_request)
    }
}

// Transaction Event Processing Implementation
impl DialogManager {
    /// Resolve the exact Request-URI owned by an outbound client transaction.
    ///
    /// Authentication retries must sign the URI that actually crossed the
    /// wire. Dialog targets and session metadata may already have changed, so
    /// they are deliberately not used as fallbacks. A method mismatch is a
    /// correlation failure and therefore fails closed.
    pub(crate) async fn exact_outbound_request_uri(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<rvoip_sip_core::Uri> {
        if transaction_id.is_server() {
            return None;
        }
        match self
            .transaction_manager
            .original_request(transaction_id)
            .await
        {
            Ok(Some(request)) if &request.method() == transaction_id.method() => {
                Some(request.uri.clone())
            }
            Ok(Some(_)) => {
                warn!(
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    "Original outbound request method did not match transaction key"
                );
                None
            }
            Ok(None) | Err(_) => {
                if let Some(request_uri) = self
                    .transaction_manager
                    .auth_challenge_request_uri(transaction_id)
                {
                    return Some(request_uri);
                }
                warn!(
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    "Exact original outbound request was unavailable"
                );
                None
            }
        }
    }

    async fn emit_outbound_request_completed(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        outcome: OutboundRequestOutcome,
    ) {
        if transaction_id.is_server()
            || !tracks_generic_outbound_request_completion(transaction_id.method())
        {
            return;
        }
        self.emit_session_coordination_event(SessionCoordinationEvent::OutboundRequestCompleted {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            method: transaction_id.method().clone(),
            outcome,
        })
        .await;
    }

    /// Process a transaction event and update dialog state accordingly
    ///
    /// This is the core event-driven state management for dialogs based on
    /// transaction layer events. It implements proper RFC 3261 dialog state transitions.
    pub async fn process_transaction_event(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
        event: TransactionEvent,
    ) -> DialogResult<()> {
        debug!(
            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
            %dialog_id,
            event=?crate::transaction::safe_diagnostics::SafeTransactionEvent::new(&event),
            "Processing transaction event"
        );

        match event {
            TransactionEvent::StateChanged {
                previous_state,
                new_state,
                ..
            } => {
                self.handle_transaction_state_change(
                    dialog_id,
                    transaction_id,
                    previous_state,
                    new_state,
                )
                .await
            }

            TransactionEvent::SuccessResponse { response, .. } => {
                self.handle_transaction_success_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::FailureResponse { response, .. } => {
                self.handle_transaction_failure_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::ProvisionalResponse { response, .. } => {
                self.handle_transaction_provisional_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::TransactionTerminated { .. } => {
                self.handle_transaction_terminated(dialog_id, transaction_id)
                    .await
            }

            TransactionEvent::TransactionTimeout { .. } => {
                self.emit_outbound_request_completed(
                    dialog_id,
                    transaction_id,
                    OutboundRequestOutcome::Timeout,
                )
                .await;
                Ok(())
            }

            TransactionEvent::TransportError { .. } => {
                self.emit_outbound_request_completed(
                    dialog_id,
                    transaction_id,
                    OutboundRequestOutcome::TransportFailure,
                )
                .await;
                Ok(())
            }

            TransactionEvent::Error {
                transaction_id: Some(error_transaction_id),
                ..
            } if error_transaction_id == *transaction_id => {
                // A transaction-scoped generic error is published only after
                // the runner has fenced the exact transaction as Terminated /
                // Destroyed. Treat its exact key as the earliest authoritative
                // release observation for tracked INFO/REFER/NOTIFY/UPDATE.
                // The later TransactionTerminated observation is safe because
                // session-core removes only the exact current tracker owner.
                self.emit_outbound_request_completed(
                    dialog_id,
                    transaction_id,
                    OutboundRequestOutcome::TransportFailure,
                )
                .await;
                Ok(())
            }

            TransactionEvent::Error {
                transaction_id: Some(error_transaction_id),
                ..
            } => {
                warn!(
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    error_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&error_transaction_id),
                    "Ignoring mismatched transaction error correlation"
                );
                Ok(())
            }

            TransactionEvent::Error {
                transaction_id: None,
                ..
            } => Ok(()),

            TransactionEvent::TimerTriggered { timer, .. } => {
                debug!(
                    timer_len=timer.len(),
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    %dialog_id,
                    "Transaction timer triggered"
                );
                Ok(()) // Most timer events don't require dialog-level action
            }

            TransactionEvent::AckReceived { request, .. } => {
                self.handle_ack_received_event(dialog_id, transaction_id, request)
                    .await
            }

            TransactionEvent::CancelReceived { .. } => {
                // RFC 3261 §9.2. The transaction layer has already handled
                // the wire responses for this matched UAS-side CANCEL
                // (200 to CANCEL, 487 to INVITE). Dialog-core still owns the
                // dialog/session lifecycle notification.
                self.terminate_dialog_for_tx_and_emit_cancelled(transaction_id, "CANCEL received")
                    .await;
                Ok(())
            }

            _ => {
                debug!(
                    %dialog_id,
                    event=?crate::transaction::safe_diagnostics::SafeTransactionEvent::new(&event),
                    "Unhandled transaction event type for dialog"
                );
                Ok(())
            }
        }
    }

    /// Handle transaction state changes and update dialog state accordingly
    async fn handle_transaction_state_change(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        previous_state: TransactionState,
        new_state: TransactionState,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), ?previous_state, ?new_state, dialog=%dialog_id, "Transaction state changed");

        // Update dialog state based on transaction state changes
        match new_state {
            TransactionState::Completed => {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction completed");
                // Transaction completed successfully - dialog remains active
            }

            TransactionState::Terminated => {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction terminated");
                // Every transaction emits the explicit terminal observation
                // after this state transition. Retain both the dialog mapping
                // and route hash until then so sharded Timer/State/Terminal
                // delivery cannot reorder or lose BYE terminal association.
            }

            _ => {
                // Other state changes are informational
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), ?new_state, dialog=%dialog_id, "Transaction state observed");
            }
        }

        Ok(())
    }

    /// Handle successful responses from transactions
    async fn handle_transaction_success_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received success response");

        // Update dialog state based on successful response
        let dialog_state_changed = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();

            // Update dialog with response information (remote tag, etc.)
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    info!(dialog=%dialog_id, remote_tag_len=to_tag.len(), "Updating remote tag for dialog");
                    dialog.set_remote_tag(to_tag.to_string());
                } else {
                    warn!("200 OK response has no To tag for dialog {}", dialog_id);
                }
            } else {
                warn!("200 OK response has no To header for dialog {}", dialog_id);
            }

            // RFC 3261 §12.2.1.2: a successful target-refresh INVITE
            // replaces the dialog's remote target with the response Contact.
            // The authenticated initial-INVITE retry reuses the prepared
            // dialog, so merely updating its tag/state leaves BYE and other
            // later in-dialog requests aimed at the original Request-URI.
            // Apply the Contact before publishing confirmation. The dialog
            // enforces the SIPS downgrade invariant in `update_remote_target`.
            if response.status_code() == 200
                && transaction_id.method() == &rvoip_sip_core::Method::Invite
            {
                match response.header(&HeaderName::Contact) {
                    Some(TypedHeader::Contact(contacts)) => match contacts
                        .0
                        .first()
                        .and_then(|contact| extract_uri_from_contact(contact).ok())
                    {
                        Some(remote_target) => {
                            if !dialog.update_remote_target(remote_target) {
                                warn!(
                                    dialog=%dialog_id,
                                    "Rejected insecure Contact target refresh for secure dialog"
                                );
                            }
                        }
                        None => warn!(
                            dialog=%dialog_id,
                            "Successful INVITE response contains an unusable Contact"
                        ),
                    },
                    _ => warn!(
                        dialog=%dialog_id,
                        "Successful INVITE response has no Contact target refresh"
                    ),
                }
            }

            // Update dialog state based on response status and current state
            let state_changed = match response.status_code() {
                200 => {
                    if dialog.state == crate::dialog::DialogState::Early {
                        dialog.state = crate::dialog::DialogState::Confirmed;

                        // CRITICAL FIX: Update dialog lookup now that we have both tags
                        if let Some(tuple) = dialog.dialog_id_tuple() {
                            let key = crate::manager::utils::DialogUtils::create_lookup_key(
                                &tuple.0, &tuple.1, &tuple.2,
                            );
                            self.dialog_lookup.insert(key, dialog_id.clone());
                            info!("Updated dialog lookup for confirmed dialog {}", dialog_id);
                        }

                        // RFC 4028 UAC: capture negotiated Session-Expires
                        // from the 2xx. The refresher is whoever the peer
                        // named; if the peer omitted `refresher=`, RFC 4028
                        // §7.1 default for a UAC that originally requested
                        // `refresher=uac` is that the UAC refreshes.
                        if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                            use rvoip_sip_core::types::session_expires::Refresher;
                            use rvoip_sip_core::types::TypedHeader;
                            if let Some(se) = response.headers.iter().find_map(|h| {
                                if let TypedHeader::SessionExpires(se) = h {
                                    Some(se)
                                } else {
                                    None
                                }
                            }) {
                                dialog.session_expires_secs = Some(se.delta_seconds);
                                dialog.is_session_refresher =
                                    matches!(se.refresher, None | Some(Refresher::Uac),);
                                info!(
                                    "UAC session timer negotiated: expires={}s, we_refresh={}",
                                    se.delta_seconds, dialog.is_session_refresher
                                );
                            }
                        }

                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if state_changed {
                Some((old_state, dialog.state.clone()))
            } else {
                None
            }
        };

        // Emit dialog events for session-core
        if let Some((old_state, new_state)) = dialog_state_changed {
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            })
            .await;
        }

        // Emit session coordination events for session-core
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
            request_uri: None,
        })
        .await;

        // Handle specific successful response types
        let session_id_for_diag = self.get_session_id(dialog_id);
        match response.status_code() {
            200 => {
                // Check if this is a 200 OK to INVITE - need to send ACK
                if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                    crate::diagnostics::record_uac_invite_2xx_response();
                    if let Some(session_id) = session_id_for_diag.as_deref() {
                        crate::diagnostics::record_call_timing_uac_invite_2xx_response(session_id);
                    }
                    info!(
                        "✅ Received 200 OK to INVITE, sending automatic ACK for dialog {}",
                        dialog_id
                    );

                    // Send ACK using transaction-core's send_ack_for_2xx method
                    crate::diagnostics::record_uac_invite_2xx_ack_attempt();
                    if let Some(session_id) = session_id_for_diag.as_deref() {
                        crate::diagnostics::record_call_timing_uac_ack_attempt(session_id);
                    }
                    match self
                        .transaction_manager
                        .send_ack_for_2xx(transaction_id, &response)
                        .await
                    {
                        Ok(_) => {
                            crate::diagnostics::record_uac_invite_2xx_ack_success();
                            if let Some(session_id) = session_id_for_diag.as_deref() {
                                crate::diagnostics::record_call_timing_uac_ack_success(session_id);
                            }
                            info!("Successfully sent automatic ACK for 200 OK to INVITE");

                            // Notify session-core that ACK was sent (for state machine transition)
                            let negotiated_sdp = if !response.body().is_empty() {
                                Some(String::from_utf8_lossy(response.body()).to_string())
                            } else {
                                None
                            };

                            self.emit_session_coordination_event(
                                SessionCoordinationEvent::AckSent {
                                    dialog_id: dialog_id.clone(),
                                    transaction_id: transaction_id.clone(),
                                    negotiated_sdp,
                                },
                            )
                            .await;
                        }
                        Err(e) => {
                            crate::diagnostics::record_uac_invite_2xx_ack_failure();
                            if let Some(session_id) = session_id_for_diag.as_deref() {
                                crate::diagnostics::record_call_timing_uac_ack_failure(session_id);
                            }
                            warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send automatic ACK for 200 OK to INVITE");
                        }
                    }
                }

                // Check if this is a 200 OK to BYE - dialog is terminating
                if transaction_id.method() == &rvoip_sip_core::Method::Bye {
                    info!(
                        "✅ Received 200 OK to BYE, dialog {} is terminating",
                        dialog_id
                    );

                    // Emit CallTerminating event to notify session-core
                    self.emit_session_coordination_event(
                        SessionCoordinationEvent::CallTerminating {
                            dialog_id: dialog_id.clone(),
                            reason: "BYE completed successfully".to_string(),
                        },
                    )
                    .await;
                }

                // Successful completion - could be call answered, request completed, etc.
                if transaction_id.method() == &rvoip_sip_core::Method::Invite
                    && !response.body().is_empty()
                {
                    crate::diagnostics::record_uac_invite_2xx_call_answered_emit();
                    if let Some(session_id) = session_id_for_diag.as_deref() {
                        crate::diagnostics::record_call_timing_uac_call_answered_emit(session_id);
                    }
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallAnswered {
                        dialog_id: dialog_id.clone(),
                        session_answer: sdp,
                    })
                    .await;
                }

                // RFC 4028 UAC: spawn the refresh task now that the dialog
                // is confirmed and negotiated interval is on the dialog.
                if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                    if let Ok(dlg) = self.get_dialog(dialog_id) {
                        if let Some(secs) = dlg.session_expires_secs {
                            let is_refresher = dlg.is_session_refresher;
                            drop(dlg);
                            if let Err(_error) = crate::manager::session_timer::spawn_refresh_task(
                                self.clone(),
                                dialog_id.clone(),
                                secs,
                                is_refresher,
                            )
                            .await
                            {
                                warn!(dialog=%dialog_id, "Session refresh task was not started");
                            }
                        }
                    }
                }
            }
            _ => {
                debug!(
                    "Other successful response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );
            }
        }

        Ok(())
    }

    /// Handle failure responses from transactions
    async fn handle_transaction_failure_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        if response.status_code() == 487
            && transaction_id.method() == &rvoip_sip_core::Method::Invite
        {
            info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received CANCEL terminal response");
        } else {
            warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received failure response");
        }

        let request_uri = if response_has_auth_challenge(&response) {
            self.exact_outbound_request_uri(transaction_id).await
        } else {
            None
        };

        // Handle specific failure cases and emit appropriate events
        match response.status_code() {
            487 if transaction_id.method() == &rvoip_sip_core::Method::Invite => {
                // RFC 3261 §15.1.2 — 487 Request Terminated is a
                // CANCEL-specific termination, distinct from a generic
                // dialog teardown. Emit only `CallCancelled`; emitting
                // `DialogEvent::Terminated` here too causes the event
                // hub to publish both `DialogToSessionEvent::CallTerminated`
                // and `DialogToSessionEvent::CallCancelled` for the same
                // 487, which races in the session-core dispatcher and
                // intermittently surfaces `Event::CallEnded` to the app
                // instead of `Event::CallCancelled`.
                info!("Call cancelled for dialog {}", dialog_id);

                self.emit_session_coordination_event(SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                })
                .await;
            }

            status
                if transaction_id.method() == &rvoip_sip_core::Method::Bye
                    && matches!(status, 408 | 481) =>
            {
                // RFC 3261 BYE terminates this endpoint's participation in
                // the dialog. A 481 means the peer no longer has the dialog;
                // 408 means the request timed out. Both are terminal for our
                // local session state.
                self.emit_session_coordination_event(SessionCoordinationEvent::CallTerminating {
                    dialog_id: dialog_id.clone(),
                    reason: format!(
                        "BYE completed locally after {} {}",
                        status,
                        response.reason_phrase()
                    ),
                })
                .await;
            }

            status if status >= 400 && status < 500 && !response_has_auth_challenge(&response) => {
                // Client error - may require dialog termination
                warn!(
                    "Client error {} for dialog {} - considering termination",
                    status, dialog_id
                );

                // Emit session coordination event for failed requests
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: transaction_id.method().to_string(),
                })
                .await;
            }

            status if matches!(status, 401 | 407) => {
                debug!(
                    "Auth challenge {} for dialog {} - deferring terminal failure handling",
                    status, dialog_id
                );
            }

            status if status >= 500 => {
                // Server error - may require retry or termination
                warn!(
                    "Server error {} for dialog {} - considering retry",
                    status, dialog_id
                );

                // Emit session coordination event for server errors
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: transaction_id.method().to_string(),
                })
                .await;
            }

            _ => {
                debug!(
                    "Other failure response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );
            }
        }

        // Always emit the response received event for session-core to handle
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
            request_uri,
        })
        .await;

        Ok(())
    }

    /// Handle provisional responses from transactions
    async fn handle_transaction_provisional_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received provisional response");

        // Update dialog state for early dialogs
        let dialog_created = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();

            // For provisional responses with to-tag, create early dialog
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    if dialog.remote_tag.is_none() {
                        dialog.set_remote_tag(to_tag.to_string());
                        if dialog.state == crate::dialog::DialogState::Initial {
                            dialog.state = crate::dialog::DialogState::Early;
                            Some((old_state, dialog.state.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Emit dialog state change if early dialog was created
        if let Some((old_state, new_state)) = dialog_created {
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            })
            .await;
        }

        // RFC 3262: auto-PRACK reliable provisionals.
        // Only applies to 18x (101..200), and only when the response carries
        // both Require: 100rel and an RSeq header.
        let status = response.status_code();
        if (101..200).contains(&status) {
            if let Some(rseq_value) = detect_reliable_provisional(&response) {
                let should_send = {
                    let mut dialog = self.get_dialog_mut(dialog_id)?;
                    match dialog.last_rseq_acked {
                        Some(prev) if rseq_value <= prev => {
                            debug!(
                                "Ignoring duplicate/out-of-order reliable {}: dialog {} already acked RSeq {} (got {})",
                                status, dialog_id, prev, rseq_value
                            );
                            false
                        }
                        _ => {
                            dialog.last_rseq_acked = Some(rseq_value);
                            true
                        }
                    }
                };

                if should_send {
                    if let Err(e) = self.send_prack(dialog_id, rseq_value).await {
                        warn!(dialog=%dialog_id, rseq=rseq_value, error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Auto-PRACK failed");
                        // Roll back the ack record so a retransmit can re-trigger.
                        if let Ok(mut dialog) = self.get_dialog_mut(dialog_id) {
                            // Only roll back if we're still the most recent acker.
                            if dialog.last_rseq_acked == Some(rseq_value) {
                                dialog.last_rseq_acked = None;
                            }
                        }
                    }
                }
            }
        }

        // Handle specific provisional responses and emit session coordination events
        match response.status_code() {
            180 => {
                info!("Call ringing for dialog {}", dialog_id);

                self.emit_session_coordination_event(SessionCoordinationEvent::CallRinging {
                    dialog_id: dialog_id.clone(),
                })
                .await;
            }

            183 => {
                info!("Session progress for dialog {}", dialog_id);

                // Check for early media (SDP in 183)
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::EarlyMedia {
                        dialog_id: dialog_id.clone(),
                        sdp,
                    })
                    .await;
                } else {
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                        dialog_id: dialog_id.clone(),
                        status_code: response.status_code(),
                        reason_phrase: response.reason_phrase().to_string(),
                    })
                    .await;
                }
            }

            _ => {
                debug!(
                    "Other provisional response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );

                // Emit general call progress event
                self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                    dialog_id: dialog_id.clone(),
                    status_code: response.status_code(),
                    reason_phrase: response.reason_phrase().to_string(),
                })
                .await;
            }
        }

        Ok(())
    }

    /// Handle transaction termination
    async fn handle_transaction_terminated(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction terminated for dialog");

        // Clean up transaction-dialog association
        self.unlink_transaction_from_dialog_indexed_preserving_route(transaction_id);

        if transaction_id.method() == &rvoip_sip_core::Method::Bye {
            self.emit_session_coordination_event(SessionCoordinationEvent::CallTerminating {
                dialog_id: dialog_id.clone(),
                reason: "BYE transaction terminated".to_string(),
            })
            .await;
        }

        // Note: Other methods do not automatically terminate dialogs when
        // transactions terminate because dialogs can have multiple
        // transactions. Dialog termination is handled by higher-level logic
        // (session-core) or explicit BYE requests.

        Ok(())
    }

    /// Handle ACK received event (RFC 3261 compliant media start point for UAS)
    async fn handle_ack_received_event(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        if self
            .get_dialog_state(dialog_id)
            .map(|state| state.is_terminated())
            .unwrap_or(false)
        {
            debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Ignoring ACK for terminated dialog");
            return Ok(());
        }

        info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "ACK received; media may start on UAS side");

        // Extract any SDP from the ACK (though typically ACK doesn't have SDP for 2xx responses)
        let negotiated_sdp = if !request.body().is_empty() {
            let sdp = String::from_utf8_lossy(request.body()).to_string();
            info!(body_len = request.body().len(), "ACK contains SDP body");
            Some(sdp)
        } else {
            info!("ACK has no SDP body (normal for 2xx ACK)");
            None
        };

        info!(
            "🔔 About to emit AckReceived event for dialog {}",
            dialog_id
        );

        // RFC 3261 COMPLIANT: Emit ACK received event for UAS side media creation
        self.emit_session_coordination_event(SessionCoordinationEvent::AckReceived {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            negotiated_sdp,
        })
        .await;

        info!("🚀 RFC 3261: Emitted AckReceived event for UAS side media creation");
        Ok(())
    }
}

// Additional transaction integration methods for DialogManager
impl DialogManager {
    /// Create server transaction for incoming request
    ///
    /// Helper to create server transactions with proper error handling.
    pub async fn create_server_transaction_for_request(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<TransactionKey> {
        debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
            %source,
            "Creating server transaction for request"
        );

        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request, source)
            .await
            .map_err(|_error| crate::errors::DialogError::TransactionError {
                message: safe_operation_failure("server_transaction_create", "transaction_error"),
            })?;

        let transaction_id = server_transaction.id().clone();

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Created server transaction for request");
        Ok(transaction_id)
    }

    /// Create client transaction for outgoing request
    ///
    /// Helper to create client transactions with method-specific handling.
    pub async fn create_client_transaction_for_request(
        &self,
        request: Request,
        destination: SocketAddr,
        method: &Method,
    ) -> DialogResult<TransactionKey> {
        debug!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(method), %destination, "Creating client transaction for request");

        // STIR/SHAKEN (RFC 8224) — fire the request lifecycle for
        // INVITE so the installed PASSporTSigner attaches an
        // `Identity:` header. Generic helper paths land here when
        // dialog-core's bespoke per-method send paths can't be used
        // (e.g. raw out-of-dialog INVITE injection from upper layers).
        let mut request = request;
        if *method == Method::Invite {
            use crate::manager::RequestLifecycle;
            self.pre_send_request(&mut request, destination).await?;
        }

        let transaction_id = if *method == Method::Invite {
            self.transaction_manager
                .create_invite_client_transaction(request, destination)
                .await
        } else {
            self.transaction_manager
                .create_non_invite_client_transaction(request, destination)
                .await
        }
        .map_err(|_error| crate::errors::DialogError::TransactionError {
            message: safe_method_operation_failure(
                "client_transaction_create",
                "transaction_error",
                method,
            ),
        })?;

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), method=%crate::transaction::safe_diagnostics::SafeMethod::new(method), "Created client transaction for request");
        Ok(transaction_id)
    }

    /// Terminate the dialog associated with an INVITE transaction and
    /// optionally emit a `CallCancelled` session-coordination event.
    ///
    /// UAC and UAS CANCEL differ:
    /// - UAC-side user cancel sends CANCEL and waits for the INVITE's final
    ///   outcome before session-core publishes `CallCancelled`.
    /// - UAS-side inbound CANCEL is already terminal for the pending INVITE
    ///   once 200(CANCEL)/487(INVITE) has been sent, so dialog-core must
    ///   publish `CallCancelled` to session-core.
    async fn dialog_id_for_invite_tx(&self, invite_tx_id: &TransactionKey) -> Option<DialogId> {
        if let Some(dialog_id) = self
            .transaction_to_dialog
            .get(invite_tx_id)
            .map(|d| d.clone())
        {
            return Some(dialog_id);
        } else {
            match self
                .transaction_manager
                .get_server_transaction_request(invite_tx_id)
                .await
            {
                Ok(request) => match self.find_dialog_for_request(&request).await {
                    Some(dialog_id) => return Some(dialog_id),
                    None => {
                        warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), "Cannot emit CallCancelled: no dialog mapping or request match");
                        return None;
                    }
                },
                Err(e) => {
                    warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Cannot emit CallCancelled: failed to fetch INVITE request");
                    return None;
                }
            }
        }
    }

    pub async fn terminate_dialog_for_tx(&self, invite_tx_id: &TransactionKey, _reason: &str) {
        let Some(dialog_id) = self.dialog_id_for_invite_tx(invite_tx_id).await else {
            return;
        };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.terminate();
            debug!("Terminated dialog {} due to INVITE cancellation", dialog_id);
        }
    }

    pub async fn terminate_dialog_for_tx_and_emit_cancelled(
        &self,
        invite_tx_id: &TransactionKey,
        reason: &str,
    ) {
        let Some(dialog_id) = self.dialog_id_for_invite_tx(invite_tx_id).await else {
            return;
        };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.terminate();
            debug!("Terminated dialog {} due to INVITE cancellation", dialog_id);
        }

        self.emit_session_coordination_event(SessionCoordinationEvent::CallCancelled {
            dialog_id,
            reason: reason.to_string(),
        })
        .await;
    }

    /// Cancel an INVITE transaction using transaction-core
    ///
    /// Properly cancels INVITE transactions while updating associated dialogs.
    pub async fn cancel_invite_transaction_with_dialog(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> DialogResult<TransactionKey> {
        self.cancel_invite_transaction_with_dialog_and_extras(invite_tx_id, Vec::new())
            .await
    }

    /// CANCEL with application extras. The transaction-manager helper
    /// builds the wire CANCEL from the targeted INVITE (RFC 3261 §9.1
    /// — same Call-ID/From/To/CSeq-num/Via-branch/Route). When extras
    /// are supplied, they are appended to that wire form after the
    /// stack-managed slice; the resulting CANCEL is sent on its own
    /// new non-INVITE client transaction.
    pub async fn cancel_invite_transaction_with_dialog_and_extras(
        &self,
        invite_tx_id: &TransactionKey,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<TransactionKey> {
        let _operation = self.enter_invite_failover_operation().ok_or_else(|| {
            crate::errors::DialogError::InvalidState {
                expected: "dialog manager running".to_string(),
                actual: "dialog manager draining or stopped".to_string(),
            }
        })?;
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), extra_header_count=extra_headers.len(), "Cancelling INVITE transaction with dialog cleanup");
        let pre_cancel_dialog_id = self
            .transaction_to_dialog
            .get(invite_tx_id)
            .map(|entry| entry.value().clone());

        if let Some(dialog_id) = pre_cancel_dialog_id.as_ref() {
            if let Some(plan_id) = self
                .active_invite_failover_by_dialog
                .get(dialog_id)
                .map(|entry| *entry.value())
            {
                if let Some(plan) = self
                    .invite_failover_plans
                    .get(&plan_id)
                    .map(|entry| entry.value().clone())
                {
                    let selected = {
                        let mut plan = plan.lock().await;
                        if plan.phase == InviteFailoverPlanPhase::Active {
                            let selected = plan.current_transaction.clone();
                            if let Some(current_transaction) = selected.as_ref() {
                                Self::set_invite_attempt_outcome(
                                    &mut plan,
                                    current_transaction,
                                    InviteFailoverAttemptOutcome::Cancelled,
                                );
                            }
                            // Commit logical cancellation before the external
                            // transaction command. This wins atomically against
                            // failover, while the bounded command itself runs
                            // without monopolizing the plan mutex.
                            self.close_active_invite_failover_plan(
                                &mut plan,
                                InviteFailoverPlanPhase::Cancelled,
                            );
                            selected
                        } else {
                            None
                        }
                    };
                    if let Some(current_transaction) = selected {
                        // CANCEL is teardown, not part of the original INVITE
                        // setup attempt. Give it an independent bounded budget:
                        // the setup deadline may already have elapsed, and a
                        // very small configured transaction timeout must not
                        // race transaction-core's bounded send confirmation.
                        let cancel_deadline = tokio::time::Instant::now()
                            + self
                                .transaction_manager
                                .timer_settings()
                                .transaction_timeout
                                .max(INVITE_CANDIDATE_COMPENSATION_TIMEOUT);
                        let cancel_transaction = match tokio::time::timeout_at(
                            cancel_deadline,
                            self.transaction_manager
                                .cancel_invite_transaction_with_extras(
                                    &current_transaction,
                                    extra_headers,
                                ),
                        )
                        .await
                        {
                            Ok(Ok(transaction_id)) => transaction_id,
                            Ok(Err(_)) => {
                                return Err(crate::errors::DialogError::TransactionError {
                                    message: safe_operation_failure(
                                        "invite_cancel",
                                        "transaction_error",
                                    ),
                                });
                            }
                            Err(_) => {
                                return Err(Self::invite_setup_timeout("invite_cancel"));
                            }
                        };

                        if let Ok(mut dialog) = self.get_dialog_mut(dialog_id) {
                            dialog.terminate();
                            debug!(
                                %dialog_id,
                                "Terminated dialog after atomically cancelling current INVITE attempt"
                            );
                        }
                        debug!(
                            invite_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&current_transaction),
                            cancel_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&cancel_transaction),
                            plan_id,
                            "Cancelled exact current retained initial-INVITE attempt"
                        );
                        return Ok(cancel_transaction);
                    }
                }
            }
        }

        // Cancel the transaction using transaction-core
        let cancel_deadline = tokio::time::Instant::now()
            + self
                .transaction_manager
                .timer_settings()
                .transaction_timeout
                .max(INVITE_CANDIDATE_COMPENSATION_TIMEOUT);
        let cancel_tx_id = match tokio::time::timeout_at(
            cancel_deadline,
            self.transaction_manager
                .cancel_invite_transaction_with_extras(invite_tx_id, extra_headers),
        )
        .await
        {
            Ok(Ok(transaction_id)) => transaction_id,
            Ok(Err(_)) => {
                return Err(crate::errors::DialogError::TransactionError {
                    message: safe_operation_failure("invite_cancel", "transaction_error"),
                });
            }
            Err(_) => return Err(Self::invite_setup_timeout("invite_cancel")),
        };

        if let Some(dialog_id) = pre_cancel_dialog_id {
            if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
                dialog.terminate();
                debug!("Terminated dialog {} after sending CANCEL", dialog_id);
            }
        } else {
            self.terminate_dialog_for_tx(invite_tx_id, "INVITE transaction cancelled")
                .await;
        }

        debug!(invite_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), cancel_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&cancel_tx_id), "Successfully cancelled INVITE transaction and created CANCEL transaction");
        Ok(cancel_tx_id)
    }

    /// Get transaction statistics
    ///
    /// Provides insight into transaction-dialog associations.
    pub fn get_transaction_statistics(&self) -> (usize, usize) {
        let dialog_count = self.dialogs.len();
        let transaction_mapping_count = self.transaction_to_dialog.len();

        debug!(
            "Transaction statistics: {} dialogs, {} transaction mappings",
            dialog_count, transaction_mapping_count
        );
        (dialog_count, transaction_mapping_count)
    }

    /// Resolve the configured 100rel policy for outgoing INVITEs.
    ///
    /// Reads `DialogConfig.use_100rel` from the unified config when present,
    /// otherwise defaults to `RelUsage::Supported` (advertise capability).
    pub fn config_100rel_policy(&self) -> RelUsage {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.dialog_config().use_100rel))
            .unwrap_or_default()
    }

    /// Resolve session-timer settings for outgoing INVITEs.
    ///
    /// Returns `Some((session_expires_secs, min_se_secs))` when session
    /// timers are enabled in the config, otherwise `None`.
    pub fn config_session_timer_settings(&self) -> Option<(u32, u32)> {
        self.config.read().ok().and_then(|g| {
            g.as_ref().and_then(|c| {
                let dc = c.dialog_config();
                dc.session_timer_secs
                    .map(|secs| (secs, dc.session_timer_min_se))
            })
        })
    }

    /// Send a PRACK request acknowledging a reliable provisional (RFC 3262 §7.2).
    ///
    /// Builds a PRACK within the given dialog whose `RAck` header references the
    /// supplied `rseq` and the original INVITE's CSeq. A new non-INVITE client
    /// transaction is created and sent. This is the low-level send — callers that
    /// want auto-PRACK on receipt of a reliable 18x should go through
    /// `handle_transaction_provisional_response`.
    pub async fn send_prack(
        &self,
        dialog_id: &DialogId,
        rseq: u32,
    ) -> DialogResult<TransactionKey> {
        debug!(
            "Building PRACK for dialog {} acknowledging RSeq={}",
            dialog_id, rseq
        );

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let invite_cseq = dialog.invite_cseq.ok_or_else(|| {
                crate::errors::DialogError::protocol_error(
                    "Cannot send PRACK: dialog has no INVITE CSeq recorded",
                )
            })?;

            // Need both tags: PRACK is in-dialog and reliable 18x establishes an early dialog.
            let local_tag = dialog.local_tag.clone().ok_or_else(|| {
                crate::errors::DialogError::protocol_error("PRACK requires local tag")
            })?;
            let remote_tag = dialog.remote_tag.clone().ok_or_else(|| {
                crate::errors::DialogError::protocol_error(
                    "PRACK requires remote tag from the reliable 18x response",
                )
            })?;

            // Increment local CSeq for the PRACK (it's a new transaction).
            dialog.local_cseq += 1;
            let prack_cseq = dialog.local_cseq;
            let route_set = dialog.route_set.clone();
            let call_id = dialog.call_id.clone();
            let local_uri = dialog.local_uri.to_string();
            let target_uri = dialog.remote_uri.clone();
            let remote_uri = dialog.remote_uri.to_string();
            let local_address = self.local_address_for_target_and_routes(&target_uri, &route_set);

            let request = crate::transaction::dialog::prack_for_dialog(
                call_id,
                local_uri,
                local_tag,
                remote_uri,
                remote_tag,
                rseq,
                invite_cseq,
                prack_cseq,
                local_address,
                if route_set.is_empty() {
                    None
                } else {
                    Some(route_set)
                },
            )
            .map_err(|_error| crate::errors::DialogError::InternalError {
                message: safe_operation_failure("prack_build", "builder_error"),
                context: None,
            })?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    crate::errors::DialogError::routing_error(
                        "PRACK contains an unusable Route header",
                    )
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;
            if candidates.is_empty() {
                return Err(crate::errors::DialogError::routing_error(
                    "No address candidates for the exact PRACK next hop",
                ));
            }

            (candidates, request)
        };

        let (transaction_id, _) = self
            .send_request_with_candidate_failover(request, candidates, Some(dialog_id))
            .await?;

        info!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), rseq, "Sent PRACK");
        Ok(transaction_id)
    }

    /// Cleanup orphaned transaction mappings
    ///
    /// Removes transaction-dialog mappings for terminated dialogs.
    pub async fn cleanup_orphaned_transaction_mappings(&self) -> usize {
        let mut orphaned_count = 0;
        let active_dialog_ids: std::collections::HashSet<crate::dialog::DialogId> = self
            .dialogs
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // Collect orphaned transaction IDs
        let orphaned_transactions: Vec<TransactionKey> = self
            .transaction_to_dialog
            .iter()
            .filter_map(|entry| {
                let dialog_id = entry.value();
                if !active_dialog_ids.contains(dialog_id) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        // Remove orphaned mappings
        for tx_id in orphaned_transactions {
            self.unlink_transaction_from_dialog_indexed(&tx_id);
            orphaned_count += 1;
        }

        if orphaned_count > 0 {
            debug!(
                "Cleaned up {} orphaned transaction mappings",
                orphaned_count
            );
        }

        orphaned_count
    }
}
