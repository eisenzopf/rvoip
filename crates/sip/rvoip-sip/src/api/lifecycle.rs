//! Race-resistant per-session lifecycle observations.
//!
//! This module backs the public handle-first wait APIs. App-visible events are
//! still delivered through the global session event bus; the lifecycle index is
//! an internal companion cache updated immediately before event publication so
//! late waiters can observe recently published lifecycle facts without polling.

use crate::adapters::SessionApiCrossCrateEvent;
use crate::api::events::{Event, MediaSecurityState};
use crate::api::handle::TransferOutcome;
use crate::cleanup_diag::{self, CleanupStage};
use crate::errors::{Result, SessionError};
use crate::session_lifecycle::SessionGeneration;
use crate::session_registry::{RegistrySlotRevision, SessionRegistryHandle};
use crate::state_table::types::SessionId;
use crate::types::CallState;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch, Mutex as TokioMutex, Notify, RwLock as TokioRwLock};

const TERMINAL_EVENT_TTL: Duration = Duration::from_secs(60);
const MAX_PROGRESS_EVENTS: usize = 8;
const MAX_EAGER_LIFECYCLE_ENTRY_CAPACITY: usize = 4_096;
const MAX_EAGER_LIFECYCLE_WAITER_CAPACITY: usize = 256;
/// Maximum number of same-deadline terminal records removed in one scheduler
/// turn. A qualified high-CPS burst can make more than one hundred thousand
/// records expire together; bounded waves prevent that horizon from becoming
/// one large temporary vector and one monopolized Tokio worker.
const TERMINAL_DEADLINE_PRUNE_BATCH_MAX: usize = 4_096;

const EXACT_TERMINAL_PENDING: u8 = 0;
const EXACT_TERMINAL_PUBLISHED_AND_RELEASED: u8 = 1;
const EXACT_TERMINAL_PUBLICATION_FAILED: u8 = 2;
const EXACT_TERMINAL_RELEASE_FAILED: u8 = 3;
const EXACT_TERMINAL_PUBLICATION_AND_RELEASE_FAILED: u8 = 4;
const EXACT_TERMINAL_OWNER_DROPPED: u8 = 5;

/// Result retained by an exact terminal-publication claim.
///
/// Publication and local resource release are one ordered terminal operation:
/// the winner publishes first, releases second, and only then wakes observers.
/// This lets a public hangup path safely join a dialog-event winner without
/// returning before exact cleanup has completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExactTerminalCompletion {
    PublishedAndReleased,
    /// The authoritative lifecycle record was committed and exact resources
    /// were released, but the observational cross-crate bus rejected the
    /// delivery attempt. Protocol callers still completed successfully; the
    /// dispatcher diagnostics retain the publication failure.
    PublicationFailed,
    ReleaseFailed,
    PublicationAndReleaseFailed,
    OwnerDropped,
}

impl ExactTerminalCompletion {
    fn encode(self) -> u8 {
        match self {
            Self::PublishedAndReleased => EXACT_TERMINAL_PUBLISHED_AND_RELEASED,
            Self::PublicationFailed => EXACT_TERMINAL_PUBLICATION_FAILED,
            Self::ReleaseFailed => EXACT_TERMINAL_RELEASE_FAILED,
            Self::PublicationAndReleaseFailed => EXACT_TERMINAL_PUBLICATION_AND_RELEASE_FAILED,
            Self::OwnerDropped => EXACT_TERMINAL_OWNER_DROPPED,
        }
    }

    fn decode(value: u8) -> Option<Self> {
        match value {
            EXACT_TERMINAL_PUBLISHED_AND_RELEASED => Some(Self::PublishedAndReleased),
            EXACT_TERMINAL_PUBLICATION_FAILED => Some(Self::PublicationFailed),
            EXACT_TERMINAL_RELEASE_FAILED => Some(Self::ReleaseFailed),
            EXACT_TERMINAL_PUBLICATION_AND_RELEASE_FAILED => {
                Some(Self::PublicationAndReleaseFailed)
            }
            EXACT_TERMINAL_OWNER_DROPPED => Some(Self::OwnerDropped),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct ExactTerminalClaimSlot {
    completion: AtomicU8,
    completed: Notify,
}

impl ExactTerminalClaimSlot {
    fn pending() -> Self {
        Self {
            completion: AtomicU8::new(EXACT_TERMINAL_PENDING),
            completed: Notify::new(),
        }
    }

    fn finish(&self, completion: ExactTerminalCompletion) -> bool {
        let finished = self
            .completion
            .compare_exchange(
                EXACT_TERMINAL_PENDING,
                completion.encode(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if finished {
            self.completed.notify_waiters();
        }
        finished
    }

    async fn wait(&self) -> ExactTerminalCompletion {
        loop {
            let notified = self.completed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(completion) =
                ExactTerminalCompletion::decode(self.completion.load(Ordering::Acquire))
            {
                return completion;
            }
            notified.await;
        }
    }
}

/// Winning ownership of terminal publication for one exact SIP lifetime.
/// Dropping an unfinished owner wakes observers with a deterministic failure
/// instead of leaving a public hangup blocked forever.
pub(crate) struct ExactTerminalClaimOwner {
    claims: ExactTerminalClaims,
    key: ExactTerminalClaimKey,
    slot: Arc<ExactTerminalClaimSlot>,
    finished: bool,
}

impl ExactTerminalClaimOwner {
    pub(crate) fn finish(mut self, completion: ExactTerminalCompletion) {
        if self.slot.finish(completion) {
            self.claims
                .compact_completed(self.key, &self.slot, completion, Instant::now());
        }
        self.finished = true;
    }
}

impl Drop for ExactTerminalClaimOwner {
    fn drop(&mut self) {
        if !self.finished && self.slot.finish(ExactTerminalCompletion::OwnerDropped) {
            self.claims.compact_completed(
                self.key,
                &self.slot,
                ExactTerminalCompletion::OwnerDropped,
                Instant::now(),
            );
        }
    }
}

/// Observation of the exact terminal operation already owned by another path.
pub(crate) struct ExactTerminalClaimObserver {
    observation: ExactTerminalClaimObservation,
}

enum ExactTerminalClaimObservation {
    Pending(Arc<ExactTerminalClaimSlot>),
    Completed(ExactTerminalCompletion),
}

impl ExactTerminalClaimObserver {
    pub(crate) async fn wait(self) -> ExactTerminalCompletion {
        match self.observation {
            ExactTerminalClaimObservation::Pending(slot) => slot.wait().await,
            ExactTerminalClaimObservation::Completed(completion) => completion,
        }
    }
}

pub(crate) enum ExactTerminalClaim {
    Owner(ExactTerminalClaimOwner),
    Observer(ExactTerminalClaimObserver),
}

/// Compact, process-unique identity for one exact registry slot.
///
/// The authority generation is process unique and the slot revision prevents
/// delayed cleanup for a removed mapping from targeting a later registry slot.
/// Terminal claims do not need to retain the application-facing `SessionId`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ExactTerminalClaimKey {
    generation: SessionGeneration,
    slot_revision: RegistrySlotRevision,
}

impl From<&SessionRegistryHandle> for ExactTerminalClaimKey {
    fn from(handle: &SessionRegistryHandle) -> Self {
        Self {
            generation: handle.key().generation,
            slot_revision: handle.slot_revision(),
        }
    }
}

#[derive(Debug)]
struct ExactTerminalClaimDeadline {
    key: ExactTerminalClaimKey,
    generation: u64,
}

#[derive(Default, Debug)]
struct ExactTerminalClaimDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), ExactTerminalClaimDeadline>,
    next_sequence: u64,
}

impl ExactTerminalClaimDeadlineQueue {
    fn schedule(&mut self, key: ExactTerminalClaimKey, completed_at: Instant) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.by_deadline.insert(
            (completed_at + TERMINAL_EVENT_TTL, sequence),
            ExactTerminalClaimDeadline {
                key,
                generation: sequence,
            },
        );
        sequence
    }

    fn take_due(&mut self, now: Instant, max_work: usize) -> Vec<ExactTerminalClaimDeadline> {
        let mut due = Vec::with_capacity(max_work.min(self.by_deadline.len()));
        while due.len() < max_work
            && self
                .by_deadline
                .first_key_value()
                .is_some_and(|((deadline, _), _)| *deadline <= now)
        {
            if let Some((_, deadline)) = self.by_deadline.pop_first() {
                due.push(deadline);
            }
        }
        due
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.by_deadline
            .first_key_value()
            .map(|((deadline, _), _)| *deadline)
    }
}

#[derive(Debug)]
enum ExactTerminalClaimState {
    /// The owner is still publishing and releasing resources. Existing
    /// observers retain this allocation until the exact completion arrives.
    Pending(Arc<ExactTerminalClaimSlot>),
    /// Completed claims retain only their immutable result. The comparatively
    /// heavy `Notify` allocation is released as soon as pending observers have
    /// consumed it rather than for the full terminal retention horizon.
    Completed {
        completion: ExactTerminalCompletion,
        deadline_generation: u64,
    },
}

#[derive(Clone, Default, Debug)]
struct ExactTerminalClaims {
    slots: Arc<DashMap<ExactTerminalClaimKey, ExactTerminalClaimState>>,
    deadlines: Arc<Mutex<ExactTerminalClaimDeadlineQueue>>,
    pruner_started: Arc<AtomicBool>,
    pruner_changed: Arc<Notify>,
}

impl ExactTerminalClaims {
    fn claim(&self, handle: &SessionRegistryHandle) -> ExactTerminalClaim {
        self.start_background_pruner();
        self.prune_due(Instant::now());
        let key = ExactTerminalClaimKey::from(handle);
        match self.slots.entry(key) {
            Entry::Vacant(entry) => {
                let slot = Arc::new(ExactTerminalClaimSlot::pending());
                entry.insert(ExactTerminalClaimState::Pending(Arc::clone(&slot)));
                ExactTerminalClaim::Owner(ExactTerminalClaimOwner {
                    claims: self.clone(),
                    key,
                    slot,
                    finished: false,
                })
            }
            Entry::Occupied(entry) => {
                let observation = match entry.get() {
                    ExactTerminalClaimState::Pending(slot) => {
                        ExactTerminalClaimObservation::Pending(Arc::clone(slot))
                    }
                    ExactTerminalClaimState::Completed { completion, .. } => {
                        ExactTerminalClaimObservation::Completed(*completion)
                    }
                };
                ExactTerminalClaim::Observer(ExactTerminalClaimObserver { observation })
            }
        }
    }

    fn compact_completed(
        &self,
        key: ExactTerminalClaimKey,
        slot: &Arc<ExactTerminalClaimSlot>,
        completion: ExactTerminalCompletion,
        completed_at: Instant,
    ) {
        // Schedule first so a successfully compacted entry always has a
        // due-driven expiry. A failed pointer check can only mean the exact
        // pending generation is no longer authoritative; the eventual
        // deadline is harmless because pruning removes completed entries only.
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline_generation = deadlines.schedule(key, completed_at);
        if let Some(mut current) = self.slots.get_mut(&key) {
            if matches!(
                current.value(),
                ExactTerminalClaimState::Pending(current_slot)
                    if Arc::ptr_eq(current_slot, slot)
            ) {
                *current = ExactTerminalClaimState::Completed {
                    completion,
                    deadline_generation,
                };
            }
        }
        drop(deadlines);
        self.pruner_changed.notify_one();
    }

    fn start_background_pruner(&self) {
        if self
            .pruner_started
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.pruner_started.store(false, Ordering::Relaxed);
            return;
        };

        let slots = Arc::downgrade(&self.slots);
        let deadlines = Arc::downgrade(&self.deadlines);
        let changed = Arc::clone(&self.pruner_changed);
        handle.spawn(async move {
            loop {
                // Register the wake before reading the ordered queue. A
                // concurrent earlier deadline then either appears in this
                // read or leaves a stored notification permit.
                let notified = changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                let Some(slots) = slots.upgrade() else {
                    break;
                };
                let Some(deadlines) = deadlines.upgrade() else {
                    break;
                };
                let now = Instant::now();
                prune_due_exact_terminal_claims(
                    &slots,
                    &deadlines,
                    now,
                    TERMINAL_DEADLINE_PRUNE_BATCH_MAX,
                );
                let next_deadline = deadlines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .next_deadline();
                drop(deadlines);
                drop(slots);

                match next_deadline {
                    Some(deadline) if deadline <= now => tokio::task::yield_now().await,
                    Some(deadline) => {
                        let sleep =
                            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                        tokio::pin!(sleep);
                        tokio::select! {
                            () = &mut sleep => {}
                            () = &mut notified => {}
                        }
                    }
                    None => notified.await,
                }
            }
        });
    }

    fn prune_due(&self, now: Instant) -> usize {
        prune_due_exact_terminal_claims(
            &self.slots,
            &self.deadlines,
            now,
            TERMINAL_DEADLINE_PRUNE_BATCH_MAX,
        )
    }

    #[cfg(feature = "perf-tests")]
    fn perf_diagnostic_counts(&self) -> serde_json::Value {
        let mut pending = 0_u64;
        let mut completed = 0_u64;
        for entry in self.slots.iter() {
            match entry.value() {
                ExactTerminalClaimState::Pending(_) => pending += 1,
                ExactTerminalClaimState::Completed { .. } => completed += 1,
            }
        }
        let deadline_count = self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .by_deadline
            .len();
        serde_json::json!({
            "slots": self.slots.len(),
            "pending": pending,
            "completed": completed,
            "deadlines": deadline_count,
            "retention_secs": TERMINAL_EVENT_TTL.as_secs(),
            "storage": {
                "slot_table_capacity": self.slots.capacity(),
                "handle_identifier_payload_bytes": 0,
                "identity_payload_bytes": 0,
                "record_inline_bytes": {
                    "exact_claim_key": std::mem::size_of::<ExactTerminalClaimKey>(),
                    "claim_state": std::mem::size_of::<ExactTerminalClaimState>(),
                    "deadline_record": std::mem::size_of::<ExactTerminalClaimDeadline>(),
                    "deadline_key": std::mem::size_of::<(Instant, u64)>(),
                },
                "scope": "payload_and_inline_estimates_exclude_container_node_and_allocator_overhead",
            },
        })
    }
}

fn prune_due_exact_terminal_claims(
    slots: &DashMap<ExactTerminalClaimKey, ExactTerminalClaimState>,
    deadlines: &Mutex<ExactTerminalClaimDeadlineQueue>,
    now: Instant,
    max_work: usize,
) -> usize {
    let (due, drained_to_empty) = {
        let mut deadlines = deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let had_deadlines = !deadlines.by_deadline.is_empty();
        let due = deadlines.take_due(now, max_work);
        (due, had_deadlines && deadlines.by_deadline.is_empty())
    };
    let mut removed = 0;
    for deadline in due {
        if slots
            .remove_if(&deadline.key, |_, current| {
                matches!(
                    current,
                    ExactTerminalClaimState::Completed {
                        deadline_generation,
                        ..
                    } if *deadline_generation == deadline.generation
                )
            })
            .is_some()
        {
            removed += 1;
        }
    }
    if drained_to_empty && slots.is_empty() && slots.capacity() > TERMINAL_DEADLINE_PRUNE_BATCH_MAX
    {
        slots.shrink_to_fit();
    }
    removed
}

/// Provisional call-progress evidence observed for a call.
#[derive(Clone, PartialEq, Eq)]
pub struct CallProgressInfo {
    /// Session identifier for the call.
    pub call_id: SessionId,
    /// SIP provisional status code, usually `180` or `183`.
    pub status_code: u16,
    /// SIP reason phrase.
    pub reason: String,
    /// SDP body carried by the provisional response, if present.
    pub sdp: Option<String>,
}

impl std::fmt::Debug for CallProgressInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallProgressInfo")
            .field("call_id", &self.call_id)
            .field("status_code", &self.status_code)
            .field("reason_bytes", &self.reason.len())
            .field("sdp_present", &self.sdp.is_some())
            .field("sdp_bytes", &self.sdp.as_ref().map_or(0, String::len))
            .finish()
    }
}

impl CallProgressInfo {
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::CallProgress {
                call_id,
                status_code,
                reason,
                sdp,
            } => Some(Self {
                call_id: call_id.clone(),
                status_code: *status_code,
                reason: reason.clone(),
                sdp: sdp.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn to_event(&self) -> Event {
        Event::CallProgress {
            call_id: self.call_id.clone(),
            status_code: self.status_code,
            reason: self.reason.clone(),
            sdp: self.sdp.clone(),
        }
    }
}

/// Answer evidence observed for a call.
#[derive(Clone, PartialEq, Eq)]
pub struct CallAnsweredInfo {
    /// Session identifier for the answered call.
    pub call_id: SessionId,
    /// SDP body from the answer, if present.
    pub sdp: Option<String>,
}

impl std::fmt::Debug for CallAnsweredInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallAnsweredInfo")
            .field("call_id", &self.call_id)
            .field("sdp_present", &self.sdp.is_some())
            .field("sdp_bytes", &self.sdp.as_ref().map_or(0, String::len))
            .finish()
    }
}

impl CallAnsweredInfo {
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::CallAnswered { call_id, sdp } => Some(Self {
                call_id: call_id.clone(),
                sdp: sdp.clone(),
            }),
            _ => None,
        }
    }
}

/// Terminal lifecycle evidence for a call.
#[derive(Clone, PartialEq, Eq)]
pub enum CallTerminalInfo {
    /// Normal call end.
    Ended {
        /// Human-readable teardown reason.
        reason: String,
    },
    /// Call setup or dialog failed.
    Failed {
        /// SIP status code or synthesized failure code.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },
    /// Caller cancelled the call before answer.
    Cancelled,
}

impl std::fmt::Debug for CallTerminalInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended { reason } => formatter
                .debug_struct("Ended")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Failed {
                status_code,
                reason,
            } => formatter
                .debug_struct("Failed")
                .field("status_code", status_code)
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

impl CallTerminalInfo {
    fn from_event(event: &Event) -> Option<(SessionId, Self)> {
        match event {
            Event::CallEnded { call_id, reason } => Some((
                call_id.clone(),
                Self::Ended {
                    reason: reason.clone(),
                },
            )),
            Event::CallFailed {
                call_id,
                status_code,
                reason,
            } => Some((
                call_id.clone(),
                Self::Failed {
                    status_code: *status_code,
                    reason: reason.clone(),
                },
            )),
            Event::CallCancelled { call_id } => Some((call_id.clone(), Self::Cancelled)),
            _ => None,
        }
    }

    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Ended { reason } => reason.clone(),
            Self::Failed {
                status_code,
                reason,
            } => format!("{status_code}: {reason}"),
            Self::Cancelled => "Cancelled".to_string(),
        }
    }
}

/// Current typed lifecycle view for one call.
#[derive(Clone, PartialEq, Eq)]
pub struct CallLifecycleSnapshot {
    /// Session identifier for this snapshot.
    pub call_id: SessionId,
    /// Current call state from the session store, if still present.
    pub state: Option<CallState>,
    /// Recent provisional progress events, oldest first.
    pub progress: Vec<CallProgressInfo>,
    /// Answer evidence, if the call has answered.
    pub answered: Option<CallAnsweredInfo>,
    /// Negotiated media-security state, if SRTP was negotiated.
    pub media_security: Option<MediaSecurityState>,
    /// Terminal evidence, retained briefly after session cleanup.
    pub terminal: Option<CallTerminalInfo>,
    /// Latest typed transfer outcome observed for this call, if any.
    pub latest_transfer_outcome: Option<TransferOutcome>,
}

impl std::fmt::Debug for CallLifecycleSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallLifecycleSnapshot")
            .field("call_id", &self.call_id)
            .field("state", &self.state)
            .field("progress", &self.progress)
            .field("answered", &self.answered)
            .field("media_security", &self.media_security)
            .field("terminal", &self.terminal)
            .field("latest_transfer_outcome", &self.latest_transfer_outcome)
            .finish()
    }
}

#[derive(Debug, Clone)]
struct ActiveLifecycleEntry {
    progress: VecDeque<CallProgressInfo>,
    answered: Option<CallAnsweredInfo>,
    media_security: Option<MediaSecurityState>,
    latest_transfer_outcome: Option<Box<TransferOutcome>>,
}

impl Default for ActiveLifecycleEntry {
    fn default() -> Self {
        Self {
            // Most calls publish no provisional history to this observer
            // index. Allocate the bounded progress ring only on first use.
            progress: VecDeque::new(),
            answered: None,
            media_security: None,
            latest_transfer_outcome: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TerminalLifecycleEntry {
    answered: bool,
    media_security: Option<MediaSecurityState>,
    terminal: CallTerminalInfo,
    stored_at: Instant,
    deadline_generation: u64,
}

impl TerminalLifecycleEntry {
    fn from_active(
        active: ActiveLifecycleEntry,
        terminal: CallTerminalInfo,
        stored_at: Instant,
        deadline_generation: u64,
    ) -> Self {
        Self {
            answered: active.answered.is_some(),
            media_security: active.media_security,
            terminal,
            stored_at,
            deadline_generation,
        }
    }
}

#[derive(Debug, Clone)]
enum LifecycleEntry {
    Active(ActiveLifecycleEntry),
    Terminal(TerminalLifecycleEntry),
}

impl Default for LifecycleEntry {
    fn default() -> Self {
        Self::Active(ActiveLifecycleEntry::default())
    }
}

impl LifecycleEntry {
    #[cfg_attr(not(feature = "perf-tests"), allow(dead_code))]
    fn terminal_stored_at(&self) -> Option<Instant> {
        match self {
            Self::Active(_) => None,
            Self::Terminal(terminal) => Some(terminal.stored_at),
        }
    }

    #[cfg_attr(not(feature = "perf-tests"), allow(dead_code))]
    fn progress_len(&self) -> usize {
        match self {
            Self::Active(active) => active.progress.len(),
            Self::Terminal(_) => 0,
        }
    }

    #[cfg_attr(not(feature = "perf-tests"), allow(dead_code))]
    fn is_answered(&self) -> bool {
        match self {
            Self::Active(active) => active.answered.is_some(),
            Self::Terminal(terminal) => terminal.answered,
        }
    }

    #[cfg(feature = "perf-tests")]
    fn retained_sdp_bytes(&self) -> usize {
        match self {
            Self::Active(active) => {
                let progress_bytes: usize = active
                    .progress
                    .iter()
                    .filter_map(|progress| progress.sdp.as_ref())
                    .map(String::len)
                    .sum();
                let answered_bytes = active
                    .answered
                    .as_ref()
                    .and_then(|answered| answered.sdp.as_ref())
                    .map(String::len)
                    .unwrap_or(0);
                progress_bytes + answered_bytes
            }
            Self::Terminal(_) => 0,
        }
    }

    #[cfg(feature = "perf-tests")]
    fn retained_terminal_reason_bytes(&self) -> usize {
        match self {
            Self::Active(_) => 0,
            Self::Terminal(terminal) => match &terminal.terminal {
                CallTerminalInfo::Ended { reason } | CallTerminalInfo::Failed { reason, .. } => {
                    reason.capacity()
                }
                CallTerminalInfo::Cancelled => 0,
            },
        }
    }
}

#[derive(Default, Debug)]
struct LifecycleDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), Arc<SessionId>>,
    next_sequence: u64,
}

impl LifecycleDeadlineQueue {
    fn schedule(
        &mut self,
        call_id: Arc<SessionId>,
        stored_at: Instant,
        previous: Option<(Instant, u64)>,
    ) -> u64 {
        if let Some((previous_stored_at, previous_generation)) = previous {
            self.by_deadline
                .remove(&(previous_stored_at + TERMINAL_EVENT_TTL, previous_generation));
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let key = (stored_at + TERMINAL_EVENT_TTL, sequence);
        self.by_deadline.insert(key, call_id);
        sequence
    }

    fn unschedule(&mut self, call_id: &SessionId, stored_at: Instant, generation: u64) {
        let key = (stored_at + TERMINAL_EVENT_TTL, generation);
        if self
            .by_deadline
            .get(&key)
            .is_some_and(|scheduled| scheduled.as_ref() == call_id)
        {
            self.by_deadline.remove(&key);
        }
    }

    fn take_due(&mut self, now: Instant, max_work: usize) -> Vec<(Arc<SessionId>, u64)> {
        let mut due = Vec::with_capacity(max_work.min(self.by_deadline.len()));
        while due.len() < max_work
            && self
                .by_deadline
                .first_key_value()
                .is_some_and(|((deadline, _), _)| *deadline <= now)
        {
            let Some((key, call_id)) = self.by_deadline.pop_first() else {
                break;
            };
            due.push((call_id, key.1));
        }
        due
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.by_deadline
            .first_key_value()
            .map(|((deadline, _), _)| *deadline)
    }
}

/// Internal lifecycle index keyed by session id.
#[derive(Debug, Clone, Default)]
pub(crate) struct LifecycleIndex {
    entries: Arc<DashMap<Arc<SessionId>, LifecycleEntry>>,
    waiters: Arc<DashMap<SessionId, watch::Sender<u64>>>,
    terminal_deadlines: Arc<Mutex<LifecycleDeadlineQueue>>,
    pruner_started: Arc<AtomicBool>,
    pruner_changed: Arc<Notify>,
}

impl LifecycleIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        // `capacity` is a logical retention/admission bound, not a command to
        // eagerly reserve the entire 64-second churn horizon. Grow the maps
        // with observed calls while retaining a modest warm-start reserve.
        let initial_entries = capacity.min(MAX_EAGER_LIFECYCLE_ENTRY_CAPACITY);
        let initial_waiters = capacity.min(MAX_EAGER_LIFECYCLE_WAITER_CAPACITY);
        Self {
            entries: Arc::new(DashMap::with_capacity(initial_entries)),
            waiters: Arc::new(DashMap::with_capacity(initial_waiters)),
            terminal_deadlines: Arc::new(Mutex::new(LifecycleDeadlineQueue::default())),
            pruner_started: Arc::new(AtomicBool::new(false)),
            pruner_changed: Arc::new(Notify::new()),
        }
    }

    fn start_background_pruner(&self) {
        if self
            .pruner_started
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.pruner_started.store(false, Ordering::Relaxed);
            return;
        };

        let entries = Arc::downgrade(&self.entries);
        let waiters = Arc::downgrade(&self.waiters);
        let terminal_deadlines = Arc::downgrade(&self.terminal_deadlines);
        let changed = Arc::clone(&self.pruner_changed);
        handle.spawn(async move {
            loop {
                let notified = changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                let Some(entries) = entries.upgrade() else {
                    break;
                };
                let Some(waiters) = waiters.upgrade() else {
                    break;
                };
                let Some(terminal_deadlines) = terminal_deadlines.upgrade() else {
                    break;
                };
                let now = Instant::now();
                prune_due_terminal_entries_from(
                    &entries,
                    &waiters,
                    &terminal_deadlines,
                    now,
                    TERMINAL_DEADLINE_PRUNE_BATCH_MAX,
                );
                let next_deadline = terminal_deadlines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .next_deadline();
                drop(terminal_deadlines);
                drop(waiters);
                drop(entries);

                match next_deadline {
                    Some(deadline) if deadline <= now => tokio::task::yield_now().await,
                    Some(deadline) => {
                        let sleep =
                            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                        tokio::pin!(sleep);
                        tokio::select! {
                            () = &mut sleep => {}
                            () = &mut notified => {}
                        }
                    }
                    None => notified.await,
                }
            }
        });
    }

    pub(crate) fn record_event(&self, event: &Event) {
        let Some(call_id) = event.call_id().cloned() else {
            return;
        };

        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let lifecycle_entry_was_new = !self.entries.contains_key(&call_id);
        let pending_entry = self.entries.entry(Arc::new(call_id.clone()));
        let retained_call_id = Arc::clone(pending_entry.key());
        let mut entry = pending_entry.or_default();
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        if lifecycle_entry_was_new {
            rvoip_infra_common::memory_diagnostics::record_created(
                "sip.lifecycle.entry",
                std::mem::size_of::<LifecycleEntry>(),
            );
        }

        let terminal = CallTerminalInfo::from_event(event).map(|(_, terminal)| terminal);
        let mut terminal_stored_at = None;
        let is_terminal = terminal.is_some();
        if let LifecycleEntry::Active(active) = entry.value_mut() {
            if let Some(progress) = CallProgressInfo::from_event(event) {
                if active.progress.len() == MAX_PROGRESS_EVENTS {
                    active.progress.pop_front();
                }
                active.progress.push_back(progress);
            }

            if let Some(answered) = CallAnsweredInfo::from_event(event) {
                rvoip_sip_dialog::diagnostics::record_call_timing_lifecycle_call_answered(
                    call_id.as_str(),
                );
                active.answered = Some(answered);
            }

            if let Event::MediaSecurityNegotiated {
                keying,
                suite,
                profile,
                contexts_installed,
                ..
            } = event
            {
                active.media_security = Some(MediaSecurityState {
                    keying: *keying,
                    suite: *suite,
                    profile: *profile,
                    contexts_installed: *contexts_installed,
                });
            }

            if let Ok(outcome) = TransferOutcome::try_from(event.clone()) {
                active.latest_transfer_outcome = Some(Box::new(outcome));
            }

            if let Some(terminal) = terminal {
                let stored_at = Instant::now();
                let active = std::mem::take(active);
                *entry = LifecycleEntry::Terminal(TerminalLifecycleEntry::from_active(
                    active, terminal, stored_at, 0,
                ));
                terminal_stored_at = Some(stored_at);
            }
        }
        drop(entry);

        if let Some(stored_at) = terminal_stored_at {
            let mut deadlines = self
                .terminal_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let deadline_generation = deadlines.schedule(retained_call_id, stored_at, None);
            if let Some(mut entry) = self.entries.get_mut(&call_id) {
                if let LifecycleEntry::Terminal(terminal) = entry.value_mut() {
                    if terminal.stored_at == stored_at && terminal.deadline_generation == 0 {
                        terminal.deadline_generation = deadline_generation;
                    }
                }
            }
            drop(deadlines);
            self.pruner_changed.notify_one();
        }

        self.notify_waiters(&call_id, is_terminal);
    }

    #[cfg_attr(not(feature = "perf-tests"), allow(dead_code))]
    fn prune_expired_terminal_entries(&self) -> usize {
        prune_due_terminal_entries_from(
            &self.entries,
            &self.waiters,
            &self.terminal_deadlines,
            Instant::now(),
            TERMINAL_DEADLINE_PRUNE_BATCH_MAX,
        )
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_diagnostic_counts(&self) -> serde_json::Value {
        let pruned_expired_terminal_entries = self.prune_expired_terminal_entries();
        let mut terminal_entries = 0_u64;
        let mut expired_terminal_entries = 0_u64;
        let mut progress_events = 0_u64;
        let mut answered_entries = 0_u64;
        let mut retained_sdp_bytes = 0_u64;
        let mut retained_terminal_reason_bytes = 0_u64;
        let mut entry_identifier_payload_bytes = 0_u64;
        for entry in self.entries.iter() {
            entry_identifier_payload_bytes += entry.key().0.capacity() as u64;
            progress_events += entry.value().progress_len() as u64;
            if entry.value().is_answered() {
                answered_entries += 1;
            }
            retained_sdp_bytes += entry.value().retained_sdp_bytes() as u64;
            retained_terminal_reason_bytes += entry.value().retained_terminal_reason_bytes() as u64;
            if let Some(stored_at) = entry.value().terminal_stored_at() {
                terminal_entries += 1;
                if stored_at.elapsed() > TERMINAL_EVENT_TTL {
                    expired_terminal_entries += 1;
                }
            }
        }
        let terminal_deadline_records = {
            let deadlines = self
                .terminal_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            deadlines.by_deadline.len()
        };

        serde_json::json!({
            "entries": self.entries.len(),
            "waiters": self.waiters.len(),
            "terminal_entries": terminal_entries,
            "expired_terminal_entries": expired_terminal_entries,
            "progress_events": progress_events,
            "answered_entries": answered_entries,
            "retained_sdp_bytes": retained_sdp_bytes,
            "retained_terminal_reason_bytes": retained_terminal_reason_bytes,
            "terminal_ttl_secs": TERMINAL_EVENT_TTL.as_secs(),
            "pruned_expired_terminal_entries": pruned_expired_terminal_entries,
            "storage": {
                "entry_table_capacity": self.entries.capacity(),
                "waiter_table_capacity": self.waiters.capacity(),
                "entry_identifier_payload_bytes": entry_identifier_payload_bytes,
                "terminal_deadline_records": terminal_deadline_records,
                "terminal_deadline_identifier_payload_bytes": 0,
                "terminal_deadline_identifiers_share_entry_keys": true,
                "record_inline_bytes": {
                    "session_id": std::mem::size_of::<SessionId>(),
                    "entry_key": std::mem::size_of::<Arc<SessionId>>(),
                    "lifecycle_entry": std::mem::size_of::<LifecycleEntry>(),
                    "terminal_lifecycle_entry": std::mem::size_of::<TerminalLifecycleEntry>(),
                    "deadline_key": std::mem::size_of::<(Instant, u64)>(),
                },
                "scope": "payload_and_inline_estimates_exclude_container_node_and_allocator_overhead",
            },
        })
    }

    pub(crate) fn watcher(&self, call_id: &SessionId) -> watch::Receiver<u64> {
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        let waiter_was_new = !self.waiters.contains_key(call_id);
        let receiver = self
            .waiters
            .entry(call_id.clone())
            .or_insert_with(|| {
                let (tx, _) = watch::channel(0);
                tx
            })
            .subscribe();
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        if waiter_was_new {
            rvoip_infra_common::memory_diagnostics::record_created(
                "sip.lifecycle.waiter",
                std::mem::size_of::<watch::Sender<u64>>(),
            );
        }
        receiver
    }

    fn notify_waiters(&self, call_id: &SessionId, terminal: bool) {
        if let Some(sender) = self.waiters.get(call_id) {
            let current = *sender.borrow();
            let _ = sender.send(current.wrapping_add(1));
        }

        if terminal && self.waiters.remove(call_id).is_some() {
            #[cfg(feature = "perf-infra-memory-diagnostics")]
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.lifecycle.waiter",
                std::mem::size_of::<watch::Sender<u64>>(),
            );
        }
    }

    pub(crate) fn snapshot(
        &self,
        call_id: &SessionId,
        state: Option<CallState>,
    ) -> CallLifecycleSnapshot {
        let mut terminal_expired = false;
        let mut expired_deadline = None;
        let snapshot = if let Some(entry) = self.entries.get(call_id) {
            match entry.value() {
                LifecycleEntry::Active(active) => CallLifecycleSnapshot {
                    call_id: call_id.clone(),
                    state,
                    progress: active.progress.iter().cloned().collect(),
                    answered: active.answered.clone(),
                    media_security: active.media_security.clone(),
                    terminal: None,
                    latest_transfer_outcome: active.latest_transfer_outcome.as_deref().cloned(),
                },
                LifecycleEntry::Terminal(terminal) => {
                    if terminal.stored_at.elapsed() > TERMINAL_EVENT_TTL {
                        terminal_expired = true;
                        expired_deadline = Some((terminal.stored_at, terminal.deadline_generation));
                    }
                    CallLifecycleSnapshot {
                        call_id: call_id.clone(),
                        state,
                        // Provisional history and transfer payloads are
                        // active-only. The terminal fact is authoritative;
                        // retaining the setup transcript for every call over
                        // the 60-second late-observer window multiplied RSS.
                        progress: Vec::new(),
                        answered: terminal.answered.then(|| CallAnsweredInfo {
                            call_id: call_id.clone(),
                            sdp: None,
                        }),
                        media_security: terminal.media_security.clone(),
                        terminal: (!terminal_expired).then(|| terminal.terminal.clone()),
                        latest_transfer_outcome: None,
                    }
                }
            }
        } else {
            CallLifecycleSnapshot {
                call_id: call_id.clone(),
                state,
                progress: Vec::new(),
                answered: None,
                media_security: None,
                terminal: None,
                latest_transfer_outcome: None,
            }
        };

        if let Some((stored_at, deadline_generation)) = expired_deadline {
            let removed = self
                .entries
                .remove_if(call_id, |_, entry| {
                    matches!(
                        entry,
                        LifecycleEntry::Terminal(terminal)
                            if terminal.stored_at == stored_at
                                && terminal.deadline_generation == deadline_generation
                    )
                })
                .is_some();
            if removed {
                #[cfg(feature = "perf-infra-memory-diagnostics")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.lifecycle.entry",
                    std::mem::size_of::<LifecycleEntry>(),
                );
                if self.waiters.remove(call_id).is_some() {
                    #[cfg(feature = "perf-infra-memory-diagnostics")]
                    rvoip_infra_common::memory_diagnostics::record_dropped(
                        "sip.lifecycle.waiter",
                        std::mem::size_of::<watch::Sender<u64>>(),
                    );
                }
            }
            self.terminal_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .unschedule(call_id, stored_at, deadline_generation);
        }

        snapshot
    }
}

fn prune_due_terminal_entries_from(
    entries: &DashMap<Arc<SessionId>, LifecycleEntry>,
    waiters: &DashMap<SessionId, watch::Sender<u64>>,
    terminal_deadlines: &Mutex<LifecycleDeadlineQueue>,
    now: Instant,
    max_work: usize,
) -> usize {
    let (expired, drained_to_empty) = {
        let mut terminal_deadlines = terminal_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let had_deadlines = !terminal_deadlines.by_deadline.is_empty();
        let expired = terminal_deadlines.take_due(now, max_work);
        (
            expired,
            had_deadlines && terminal_deadlines.by_deadline.is_empty(),
        )
    };
    let mut removed = 0;
    for (call_id, deadline_generation) in expired {
        if entries
            .remove_if(call_id.as_ref(), |_, entry| {
                matches!(
                    entry,
                    LifecycleEntry::Terminal(terminal)
                        if terminal.deadline_generation == deadline_generation
                )
            })
            .is_some()
        {
            removed += 1;
            #[cfg(feature = "perf-infra-memory-diagnostics")]
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.lifecycle.entry",
                std::mem::size_of::<LifecycleEntry>(),
            );
            if waiters.remove(call_id.as_ref()).is_some() {
                #[cfg(feature = "perf-infra-memory-diagnostics")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.lifecycle.waiter",
                    std::mem::size_of::<watch::Sender<u64>>(),
                );
            }
        }
    }
    if drained_to_empty {
        if entries.is_empty() && entries.capacity() > MAX_EAGER_LIFECYCLE_ENTRY_CAPACITY {
            entries.shrink_to_fit();
        }
        if waiters.is_empty() && waiters.capacity() > MAX_EAGER_LIFECYCLE_WAITER_CAPACITY {
            waiters.shrink_to_fit();
        }
    }
    removed
}

const SESSION_EVENT_DISPATCHER_OPEN: u8 = 0;
const SESSION_EVENT_DISPATCHER_DRAINING: u8 = 1;
const SESSION_EVENT_DISPATCHER_CLOSED: u8 = 2;
#[cfg(not(test))]
const SESSION_EVENT_DISPATCHER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(test)]
const SESSION_EVENT_DISPATCHER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(100);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionEventDispatchError {
    Closed,
    PublicationFailed,
}

enum SessionEventDispatchCommand {
    Publish {
        event: Arc<SessionApiCrossCrateEvent>,
        completion: Option<oneshot::Sender<std::result::Result<(), SessionEventDispatchError>>>,
        accounting: SessionEventQueueAccounting,
    },
    Shutdown {
        completion: oneshot::Sender<()>,
    },
}

#[derive(Default)]
struct SessionEventDispatcherMetrics {
    enqueued_total: AtomicU64,
    queued_current: AtomicU64,
    queued_max: AtomicU64,
    terminal_queued_current: AtomicU64,
    terminal_queued_max: AtomicU64,
    in_flight_current: AtomicU64,
    in_flight_max: AtomicU64,
    delivered_total: AtomicU64,
    terminal_delivered_total: AtomicU64,
    saturated_admissions: AtomicU64,
    best_effort_dropped: AtomicU64,
    closed_admissions: AtomicU64,
    publication_failures: AtomicU64,
    shutdown_timeouts: AtomicU64,
    shutdown_aborted_workers: AtomicU64,
}

#[cfg(any(test, feature = "perf-tests"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SessionEventDispatcherMetricsSnapshot {
    enqueued_total: u64,
    queued_current: u64,
    queued_max: u64,
    terminal_queued_current: u64,
    terminal_queued_max: u64,
    in_flight_current: u64,
    in_flight_max: u64,
    delivered_total: u64,
    terminal_delivered_total: u64,
    saturated_admissions: u64,
    best_effort_dropped: u64,
    closed_admissions: u64,
    publication_failures: u64,
    publication_timeouts: u64,
    shutdown_timeouts: u64,
    shutdown_aborted_workers: u64,
}

/// Exact live accounting for one command retained by the dispatcher queue.
///
/// Accounting begins before channel admission so a receiver can never race a
/// sender and underflow `queued_current`. A rejected command is returned to
/// the sender and its `Drop` rolls the provisional admission back. The
/// cumulative `enqueued_total` therefore counts admission attempts, while the
/// live and maximum values describe retained commands precisely.
struct SessionEventQueueAccounting {
    metrics: Arc<SessionEventDispatcherMetrics>,
    terminal: bool,
    queued: bool,
}

impl SessionEventQueueAccounting {
    fn new(metrics: Arc<SessionEventDispatcherMetrics>, terminal: bool) -> Self {
        metrics.enqueued_total.fetch_add(1, Ordering::Relaxed);
        let queued = metrics.queued_current.fetch_add(1, Ordering::Relaxed) + 1;
        record_atomic_max(&metrics.queued_max, queued);
        if terminal {
            let terminal_queued = metrics
                .terminal_queued_current
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            record_atomic_max(&metrics.terminal_queued_max, terminal_queued);
        }
        Self {
            metrics,
            terminal,
            queued: true,
        }
    }

    fn begin_delivery(mut self) -> SessionEventInFlightAccounting {
        self.remove_from_queue();
        let in_flight = self
            .metrics
            .in_flight_current
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        record_atomic_max(&self.metrics.in_flight_max, in_flight);
        SessionEventInFlightAccounting {
            metrics: Arc::clone(&self.metrics),
            terminal: self.terminal,
        }
    }

    fn remove_from_queue(&mut self) {
        if self.queued {
            self.metrics.queued_current.fetch_sub(1, Ordering::Relaxed);
            if self.terminal {
                self.metrics
                    .terminal_queued_current
                    .fetch_sub(1, Ordering::Relaxed);
            }
            self.queued = false;
        }
    }
}

impl Drop for SessionEventQueueAccounting {
    fn drop(&mut self) {
        self.remove_from_queue();
    }
}

struct SessionEventInFlightAccounting {
    metrics: Arc<SessionEventDispatcherMetrics>,
    terminal: bool,
}

impl SessionEventInFlightAccounting {
    fn delivered(&self) {
        self.metrics.delivered_total.fetch_add(1, Ordering::Relaxed);
        if self.terminal {
            self.metrics
                .terminal_delivered_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for SessionEventInFlightAccounting {
    fn drop(&mut self) {
        self.metrics
            .in_flight_current
            .fetch_sub(1, Ordering::Relaxed);
    }
}

fn record_atomic_max(maximum: &AtomicU64, observed: u64) {
    let mut current = maximum.load(Ordering::Relaxed);
    while observed > current {
        match maximum.compare_exchange_weak(current, observed, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

#[derive(Clone)]
struct SessionEventDispatcher {
    workers: Arc<Vec<mpsc::Sender<SessionEventDispatchCommand>>>,
    next_worker: Arc<AtomicUsize>,
    state: Arc<AtomicU8>,
    admission_gate: Arc<TokioRwLock<()>>,
    closed: Arc<Notify>,
    metrics: Arc<SessionEventDispatcherMetrics>,
    worker_tasks: Arc<TokioMutex<Option<Vec<tokio::task::JoinHandle<()>>>>>,
}

impl SessionEventDispatcher {
    fn new(
        coordinator: Arc<GlobalEventCoordinator>,
        worker_count: usize,
        channel_capacity: usize,
    ) -> Self {
        let worker_count = worker_count.max(1);
        let channel_capacity = channel_capacity.max(1);
        let mut workers = Vec::with_capacity(worker_count);
        let mut worker_tasks = Vec::with_capacity(worker_count);
        let metrics = Arc::new(SessionEventDispatcherMetrics::default());

        for _ in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<SessionEventDispatchCommand>(channel_capacity);
            let coordinator = coordinator.clone();
            let worker_metrics = Arc::clone(&metrics);
            let task = tokio::spawn(async move {
                while let Some(command) = rx.recv().await {
                    match command {
                        SessionEventDispatchCommand::Publish {
                            event,
                            completion,
                            accounting,
                        } => {
                            let in_flight = accounting.begin_delivery();
                            let stage = cleanup_stage_for_event(&event.event);
                            let label = cleanup_label_for_event(&event.event);
                            let guard = cleanup_diag::stage_guard(stage, label);
                            let result = match coordinator.publish_observational(event).await {
                                Ok(()) => {
                                    in_flight.delivered();
                                    guard.finish_success();
                                    Ok(())
                                }
                                Err(_) => {
                                    worker_metrics
                                        .publication_failures
                                        .fetch_add(1, Ordering::Relaxed);
                                    cleanup_diag::record_session_event_publication_failed();
                                    guard.finish_failure();
                                    tracing::warn!(
                                        error_class = "coordinator",
                                        "Failed to publish app-level event"
                                    );
                                    Err(SessionEventDispatchError::PublicationFailed)
                                }
                            };
                            if let Some(completion) = completion {
                                let _ = completion.send(result);
                            }
                        }
                        SessionEventDispatchCommand::Shutdown { completion } => {
                            let _ = completion.send(());
                            break;
                        }
                    }
                }
            });
            workers.push(tx);
            worker_tasks.push(task);
        }

        Self {
            workers: Arc::new(workers),
            next_worker: Arc::new(AtomicUsize::new(0)),
            state: Arc::new(AtomicU8::new(SESSION_EVENT_DISPATCHER_OPEN)),
            admission_gate: Arc::new(TokioRwLock::new(())),
            closed: Arc::new(Notify::new()),
            metrics,
            worker_tasks: Arc::new(TokioMutex::new(Some(worker_tasks))),
        }
    }

    fn publish_best_effort(&self, event: Arc<SessionApiCrossCrateEvent>) {
        let idx = self.worker_index(&event.event);
        let tx = self.workers[idx].clone();
        cleanup_diag::record_queue_depth(
            cleanup_stage_for_event(&event.event),
            tx.max_capacity().saturating_sub(tx.capacity()),
        );
        let Ok(_admission) = self.admission_gate.try_read() else {
            self.record_closed_admission(&event.event);
            return;
        };
        if self.state.load(Ordering::Acquire) != SESSION_EVENT_DISPATCHER_OPEN {
            self.record_closed_admission(&event.event);
            return;
        }
        let terminal = cleanup_stage_for_event(&event.event) == CleanupStage::TerminalEventPublish;
        match tx.try_send(SessionEventDispatchCommand::Publish {
            event,
            completion: None,
            accounting: SessionEventQueueAccounting::new(Arc::clone(&self.metrics), terminal),
        }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                let event = command.event();
                self.metrics
                    .saturated_admissions
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .best_effort_dropped
                    .fetch_add(1, Ordering::Relaxed);
                cleanup_diag::record_session_event_dispatch_saturated();
                cleanup_diag::record_session_event_dispatch_dropped();
                cleanup_diag::record_queue_depth(cleanup_stage_for_event(event), tx.max_capacity());
                cleanup_diag::stage_guard(
                    cleanup_stage_for_event(event),
                    cleanup_label_for_event(event),
                )
                .finish_failure();
                tracing::warn!(
                    error_class = "bounded-queue-full",
                    "Session event dispatcher rejected a best-effort observational event"
                );
            }
            Err(mpsc::error::TrySendError::Closed(command)) => {
                self.record_closed_admission(command.event());
            }
        }
    }

    async fn publish_confirmed(
        &self,
        event: Arc<SessionApiCrossCrateEvent>,
    ) -> std::result::Result<(), SessionEventDispatchError> {
        let idx = self.worker_index(&event.event);
        let tx = self.workers[idx].clone();
        let stage = cleanup_stage_for_event(&event.event);
        let label = cleanup_label_for_event(&event.event);
        let depth = tx.max_capacity().saturating_sub(tx.capacity());
        cleanup_diag::record_queue_depth(stage, depth);
        if tx.capacity() == 0 {
            self.metrics
                .saturated_admissions
                .fetch_add(1, Ordering::Relaxed);
            cleanup_diag::record_session_event_dispatch_saturated();
        }

        // Await a bounded-channel permit without holding the shutdown gate.
        // Shutdown can therefore transition to draining and abort a hostile
        // in-flight observer even when this worker queue is full.
        let permit = match tx.reserve_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                cleanup_diag::stage_guard(stage, label).finish_failure();
                self.record_closed_admission_without_event();
                return Err(SessionEventDispatchError::Closed);
            }
        };
        let _admission = self.admission_gate.read().await;
        if self.state.load(Ordering::Acquire) != SESSION_EVENT_DISPATCHER_OPEN {
            self.record_closed_admission(&event.event);
            return Err(SessionEventDispatchError::Closed);
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        permit.send(SessionEventDispatchCommand::Publish {
            event,
            completion: Some(completion_tx),
            accounting: SessionEventQueueAccounting::new(
                Arc::clone(&self.metrics),
                stage == CleanupStage::TerminalEventPublish,
            ),
        });
        drop(_admission);

        completion_rx.await.unwrap_or_else(|_| {
            cleanup_diag::stage_guard(stage, label).finish_failure();
            self.record_closed_admission_without_event();
            Err(SessionEventDispatchError::Closed)
        })
    }

    async fn shutdown(&self) {
        let admission = self.admission_gate.write().await;
        let starts_supervisor = match self.state.compare_exchange(
            SESSION_EVENT_DISPATCHER_OPEN,
            SESSION_EVENT_DISPATCHER_DRAINING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(SESSION_EVENT_DISPATCHER_CLOSED) => return,
            Err(_) => false,
        };
        drop(admission);

        // Once DRAINING is visible, an owned supervisor—not this caller's
        // future—must complete the transition to CLOSED. A caller may be
        // cancelled at any later await without stranding every subsequent
        // shutdown waiter forever.
        if starts_supervisor {
            let dispatcher = self.clone();
            tokio::spawn(async move {
                dispatcher.finish_shutdown().await;
            });
        }
        self.wait_until_closed().await;
    }

    async fn finish_shutdown(self) {
        let queued_events_at_shutdown = self
            .workers
            .iter()
            .map(|worker| worker.max_capacity().saturating_sub(worker.capacity()))
            .sum::<usize>();
        let graceful_drain = async {
            let mut completions = Vec::with_capacity(self.workers.len());
            for worker in self.workers.iter() {
                let (completion_tx, completion_rx) = oneshot::channel();
                if worker
                    .send(SessionEventDispatchCommand::Shutdown {
                        completion: completion_tx,
                    })
                    .await
                    .is_ok()
                {
                    completions.push(completion_rx);
                }
            }

            for completion in completions {
                let _ = completion.await;
            }
        };
        let drain_timed_out =
            tokio::time::timeout(SESSION_EVENT_DISPATCHER_SHUTDOWN_TIMEOUT, graceful_drain)
                .await
                .is_err();

        let mut worker_tasks = self.worker_tasks.lock().await.take().unwrap_or_default();
        if drain_timed_out {
            self.metrics
                .shutdown_timeouts
                .fetch_add(1, Ordering::Relaxed);
            self.metrics
                .shutdown_aborted_workers
                .fetch_add(worker_tasks.len() as u64, Ordering::Relaxed);
            cleanup_diag::record_session_event_dispatch_shutdown_timeout();
            cleanup_diag::record_session_event_dispatch_aborted_workers(worker_tasks.len());
            cleanup_diag::record_session_event_dispatch_dropped_by(queued_events_at_shutdown);
            tracing::warn!(
                error_class = "shutdown-drain-timeout",
                queued_events = queued_events_at_shutdown,
                aborted_workers = worker_tasks.len(),
                "Session event dispatcher aborted blocked workers during bounded shutdown"
            );
            for worker in &worker_tasks {
                worker.abort();
            }
        }
        for worker in worker_tasks.drain(..) {
            let _ = worker.await;
        }
        self.state
            .store(SESSION_EVENT_DISPATCHER_CLOSED, Ordering::Release);
        self.closed.notify_waiters();
    }

    async fn wait_until_closed(&self) {
        loop {
            let closed = self.closed.notified();
            tokio::pin!(closed);
            closed.as_mut().enable();
            if self.state.load(Ordering::Acquire) == SESSION_EVENT_DISPATCHER_CLOSED {
                return;
            }
            closed.await;
        }
    }

    fn record_closed_admission(&self, event: &Event) {
        cleanup_diag::stage_guard(
            cleanup_stage_for_event(event),
            cleanup_label_for_event(event),
        )
        .finish_failure();
        self.record_closed_admission_without_event();
    }

    fn record_closed_admission_without_event(&self) {
        self.metrics
            .closed_admissions
            .fetch_add(1, Ordering::Relaxed);
        cleanup_diag::record_session_event_dispatch_closed();
        tracing::warn!(
            error_class = "dispatcher-closed",
            "Session event dispatcher rejected an event"
        );
    }

    #[cfg(any(test, feature = "perf-tests"))]
    fn metrics_snapshot(&self) -> SessionEventDispatcherMetricsSnapshot {
        SessionEventDispatcherMetricsSnapshot {
            enqueued_total: self.metrics.enqueued_total.load(Ordering::Relaxed),
            queued_current: self.metrics.queued_current.load(Ordering::Relaxed),
            queued_max: self.metrics.queued_max.load(Ordering::Relaxed),
            terminal_queued_current: self.metrics.terminal_queued_current.load(Ordering::Relaxed),
            terminal_queued_max: self.metrics.terminal_queued_max.load(Ordering::Relaxed),
            in_flight_current: self.metrics.in_flight_current.load(Ordering::Relaxed),
            in_flight_max: self.metrics.in_flight_max.load(Ordering::Relaxed),
            delivered_total: self.metrics.delivered_total.load(Ordering::Relaxed),
            terminal_delivered_total: self
                .metrics
                .terminal_delivered_total
                .load(Ordering::Relaxed),
            saturated_admissions: self.metrics.saturated_admissions.load(Ordering::Relaxed),
            best_effort_dropped: self.metrics.best_effort_dropped.load(Ordering::Relaxed),
            closed_admissions: self.metrics.closed_admissions.load(Ordering::Relaxed),
            publication_failures: self.metrics.publication_failures.load(Ordering::Relaxed),
            // Aggregate publication cancellation was removed because it can
            // split delivery across handlers and the bus. Keep the diagnostic
            // field as a compatibility zero for existing perf report readers.
            publication_timeouts: 0,
            shutdown_timeouts: self.metrics.shutdown_timeouts.load(Ordering::Relaxed),
            shutdown_aborted_workers: self
                .metrics
                .shutdown_aborted_workers
                .load(Ordering::Relaxed),
        }
    }

    fn worker_index(&self, event: &Event) -> usize {
        if self.workers.len() == 1 {
            return 0;
        }

        if let Some(call_id) = event.call_id() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            call_id.hash(&mut hasher);
            return (hasher.finish() as usize) % self.workers.len();
        }

        self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len()
    }
}

impl SessionEventDispatchCommand {
    fn event(&self) -> &Event {
        match self {
            Self::Publish { event, .. } => &event.event,
            Self::Shutdown { .. } => {
                unreachable!("shutdown commands are never returned by event admission")
            }
        }
    }
}

/// Publishes app-level session events and updates lifecycle first.
#[derive(Clone)]
pub(crate) struct SessionEventPublisher {
    lifecycle: LifecycleIndex,
    dispatcher: SessionEventDispatcher,
    control_sink: Option<SessionControlSink>,
    exact_terminal_claims: ExactTerminalClaims,
}

#[derive(Clone)]
struct SessionControlSink {
    sender: tokio::sync::mpsc::Sender<Event>,
    claimed: Arc<AtomicBool>,
}

pub(crate) struct TerminalEventReleaseOutcome {
    pub(crate) publication: Result<()>,
    pub(crate) release: Result<()>,
}

impl SessionEventPublisher {
    pub(crate) fn new(coordinator: Arc<GlobalEventCoordinator>, lifecycle: LifecycleIndex) -> Self {
        Self::with_dispatcher(coordinator, lifecycle, default_dispatcher_workers(), 10_000)
    }

    pub(crate) fn with_dispatcher(
        coordinator: Arc<GlobalEventCoordinator>,
        lifecycle: LifecycleIndex,
        worker_count: usize,
        channel_capacity: usize,
    ) -> Self {
        let dispatcher =
            SessionEventDispatcher::new(coordinator.clone(), worker_count, channel_capacity);
        lifecycle.start_background_pruner();
        Self {
            lifecycle,
            dispatcher,
            control_sink: None,
            exact_terminal_claims: ExactTerminalClaims::default(),
        }
    }

    pub(crate) fn with_control_sink(
        mut self,
        sender: tokio::sync::mpsc::Sender<Event>,
        claimed: Arc<AtomicBool>,
    ) -> Self {
        self.control_sink = Some(SessionControlSink { sender, claimed });
        self
    }

    pub(crate) fn claim_exact_terminal(
        &self,
        handle: &SessionRegistryHandle,
    ) -> ExactTerminalClaim {
        self.exact_terminal_claims.claim(handle)
    }

    pub(crate) fn publish(&self, event: Event) {
        self.lifecycle.record_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        self.dispatcher.publish_best_effort(wrapped);
    }

    pub(crate) async fn publish_now(&self, event: Event) -> Result<()> {
        self.lifecycle.record_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        match self.dispatcher.publish_confirmed(wrapped).await {
            Ok(()) => Ok(()),
            Err(SessionEventDispatchError::Closed) => Err(SessionError::Other(
                "Failed to publish app-level event (class=dispatcher-closed)".to_string(),
            )),
            Err(SessionEventDispatchError::PublicationFailed) => Err(SessionError::Other(
                "Failed to publish app-level event (class=coordinator)".to_string(),
            )),
        }
    }

    /// Publish a control-bearing event with nonblocking bounded admission.
    ///
    /// The capability-bearing event is delivered only to the single private
    /// owner. A separately stripped public observation is emitted exactly
    /// once whether owner admission succeeds or fails.
    pub(crate) async fn publish_control_now(&self, event: Event) -> Result<()> {
        self.lifecycle.record_event(&event);
        let mut observation = event.clone();
        if let Event::InfoReceived { request, .. } = &mut observation {
            request.clear_response_capability();
        }
        let control_result = match &self.control_sink {
            Some(sink) if sink.claimed.load(Ordering::Acquire) => {
                sink.sender.try_send(event).map_err(|_| ())
            }
            _ => {
                self.dispatcher
                    .publish_best_effort(SessionApiCrossCrateEvent::new(observation));
                return Err(SessionError::Other(
                    "Failed to publish control event (class=no-owner)".to_string(),
                ));
            }
        };
        self.dispatcher
            .publish_best_effort(SessionApiCrossCrateEvent::new(observation));
        match control_result {
            Ok(()) => Ok(()),
            Err(_) => Err(SessionError::Other(
                "Failed to publish control event (class=owner-rejected)".to_string(),
            )),
        }
    }

    /// Attempt ordered terminal publication, then always run exact release.
    ///
    /// The lifecycle index is updated before queue admission. A closed or
    /// overloaded observational bus is therefore reported separately but can
    /// never skip the authoritative release future.
    pub(crate) async fn publish_terminal_then_release<F>(
        &self,
        event: Event,
        release: F,
    ) -> TerminalEventReleaseOutcome
    where
        F: Future<Output = Result<()>>,
    {
        let publication = self.publish_now(event).await;
        let release = release.await;
        TerminalEventReleaseOutcome {
            publication,
            release,
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.dispatcher.shutdown().await;
    }

    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_diagnostic_counts(&self) -> serde_json::Value {
        let dispatcher = self.dispatcher.metrics_snapshot();
        serde_json::json!({
            "dispatcher": {
                "enqueued_total": dispatcher.enqueued_total,
                "queued_current": dispatcher.queued_current,
                "queued_max": dispatcher.queued_max,
                "terminal_queued_current": dispatcher.terminal_queued_current,
                "terminal_queued_max": dispatcher.terminal_queued_max,
                "in_flight_current": dispatcher.in_flight_current,
                "in_flight_max": dispatcher.in_flight_max,
                "delivered_total": dispatcher.delivered_total,
                "terminal_delivered_total": dispatcher.terminal_delivered_total,
                "saturated_admissions": dispatcher.saturated_admissions,
                "best_effort_dropped": dispatcher.best_effort_dropped,
                "closed_admissions": dispatcher.closed_admissions,
                "publication_failures": dispatcher.publication_failures,
                "publication_timeouts": dispatcher.publication_timeouts,
                "shutdown_timeouts": dispatcher.shutdown_timeouts,
                "shutdown_aborted_workers": dispatcher.shutdown_aborted_workers,
            },
            "exact_terminal_claims": self.exact_terminal_claims.perf_diagnostic_counts(),
        })
    }
}

fn default_dispatcher_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
}

fn cleanup_stage_for_event(event: &Event) -> CleanupStage {
    if matches!(
        event,
        Event::CallEnded { .. } | Event::CallFailed { .. } | Event::CallCancelled { .. }
    ) {
        CleanupStage::TerminalEventPublish
    } else {
        CleanupStage::SessionEventDispatch
    }
}

fn cleanup_label_for_event(event: &Event) -> String {
    event
        .call_id()
        .map(|call_id| call_id.to_string())
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_infra_common::events::config::EventCoordinatorConfig;
    use rvoip_infra_common::events::coordinator::CrossCrateEventHandler;
    use rvoip_infra_common::events::cross_crate::CrossCrateEvent;
    use tokio::sync::{mpsc::UnboundedSender, Semaphore};

    #[derive(Clone)]
    struct BlockingSessionEventHandler {
        observed: UnboundedSender<u16>,
        release: Arc<Semaphore>,
    }

    #[async_trait::async_trait]
    impl CrossCrateEventHandler for BlockingSessionEventHandler {
        async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> anyhow::Result<()> {
            let event = event
                .as_any()
                .downcast_ref::<SessionApiCrossCrateEvent>()
                .ok_or_else(|| anyhow::anyhow!("unexpected event type"))?;
            let marker = match &event.event {
                Event::CallProgress { status_code, .. } => *status_code,
                Event::CallEnded { .. } => 999,
                _ => 998,
            };
            self.observed
                .send(marker)
                .map_err(|_| anyhow::anyhow!("test observer closed"))?;
            self.release
                .acquire()
                .await
                .map_err(|_| anyhow::anyhow!("test release semaphore closed"))?
                .forget();
            Ok(())
        }
    }

    async fn blocking_event_publisher(
        queue_capacity: usize,
    ) -> (
        SessionEventPublisher,
        LifecycleIndex,
        tokio::sync::mpsc::UnboundedReceiver<u16>,
        Arc<Semaphore>,
        Arc<GlobalEventCoordinator>,
    ) {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(8),
            )
            .await
            .expect("create test event coordinator"),
        );
        let (observed_tx, observed_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        coordinator
            .register_handler(
                crate::adapters::SESSION_TO_APP_CHANNEL,
                BlockingSessionEventHandler {
                    observed: observed_tx,
                    release: Arc::clone(&release),
                },
            )
            .await
            .expect("register blocking session event handler");
        let lifecycle = LifecycleIndex::new();
        let publisher = SessionEventPublisher::with_dispatcher(
            Arc::clone(&coordinator),
            lifecycle.clone(),
            1,
            queue_capacity,
        );
        (publisher, lifecycle, observed_rx, release, coordinator)
    }

    async fn next_observed(observed: &mut tokio::sync::mpsc::UnboundedReceiver<u16>) -> u16 {
        tokio::time::timeout(Duration::from_secs(1), observed.recv())
            .await
            .expect("event handler was not reached")
            .expect("event handler observation channel closed")
    }

    async fn next_bus_observed(
        observed: &mut tokio::sync::mpsc::Receiver<Arc<dyn CrossCrateEvent>>,
    ) -> u16 {
        let event = tokio::time::timeout(Duration::from_secs(1), observed.recv())
            .await
            .expect("event bus was not reached")
            .expect("event bus observation channel closed");
        let event = event
            .as_any()
            .downcast_ref::<SessionApiCrossCrateEvent>()
            .expect("session API event");
        match &event.event {
            Event::CallProgress { status_code, .. } => *status_code,
            Event::CallEnded { .. } => 999,
            _ => 998,
        }
    }

    fn exact_terminal_test_handle(label: &str) -> SessionRegistryHandle {
        let authority = crate::session_lifecycle::SessionLeaseAuthority::new();
        let lease = authority
            .admit(SessionId::from_string(label))
            .expect("admit exact-terminal test session");
        let registry = crate::session_registry::SessionRegistry::with_authority(authority);
        registry
            .register_handle_exact(lease.key())
            .expect("register exact-terminal test handle")
    }

    #[tokio::test]
    async fn control_event_admission_fails_immediately_when_dispatcher_is_saturated() {
        let (publisher, _lifecycle, mut observed, release, coordinator) =
            blocking_event_publisher(1).await;
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(1);
        control_tx
            .try_send(Event::CallProgress {
                call_id: SessionId::from_string("queue-filler"),
                status_code: 100,
                reason: "queue filler".to_string(),
                sdp: None,
            })
            .expect("fill control owner queue");
        let publisher = publisher.with_control_sink(control_tx, Arc::new(AtomicBool::new(true)));

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            publisher.publish_control_now(Event::CallProgress {
                call_id: SessionId::from_string("saturated-control-event"),
                status_code: 183,
                reason: "control admission probe".to_string(),
                sdp: None,
            }),
        )
        .await
        .expect("control admission waited for queue capacity")
        .expect_err("saturated control event was accepted");
        assert!(
            matches!(
                &result,
                SessionError::Other(detail) if detail.contains("class=owner-rejected")
            ),
            "unexpected saturation error: {result}"
        );
        assert!(matches!(
            control_rx.try_recv().expect("retained queue filler"),
            Event::CallProgress {
                status_code: 100,
                ..
            }
        ));
        assert_eq!(next_observed(&mut observed).await, 183);
        release.add_permits(1);

        publisher.shutdown().await;
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn control_event_admission_fails_after_dispatcher_shutdown() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .expect("create test event coordinator"),
        );
        let (control_tx, control_rx) = tokio::sync::mpsc::channel(1);
        drop(control_rx);
        let mut observations = coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .expect("subscribe to public control observation");
        let publisher = SessionEventPublisher::with_dispatcher(
            Arc::clone(&coordinator),
            LifecycleIndex::new(),
            1,
            1,
        )
        .with_control_sink(control_tx, Arc::new(AtomicBool::new(true)));

        let result = publisher
            .publish_control_now(Event::CallProgress {
                call_id: SessionId::from_string("closed-control-event"),
                status_code: 183,
                reason: "control admission probe".to_string(),
                sdp: None,
            })
            .await
            .expect_err("closed dispatcher accepted a control event");
        assert!(
            matches!(
                &result,
                SessionError::Other(detail) if detail.contains("class=owner-rejected")
            ),
            "unexpected closed-dispatcher error: {result}"
        );
        assert_eq!(next_bus_observed(&mut observations).await, 183);

        publisher.shutdown().await;
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn private_control_delivery_does_not_reenter_global_shutdown_gate() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .expect("create coordinator"),
        );
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(1);
        let publisher = SessionEventPublisher::with_dispatcher(
            Arc::clone(&coordinator),
            LifecycleIndex::new(),
            1,
            1,
        )
        .with_control_sink(control_tx, Arc::new(AtomicBool::new(true)));

        coordinator.shutdown().await.expect("global shutdown");
        tokio::time::timeout(
            Duration::from_millis(100),
            publisher.publish_control_now(Event::CallProgress {
                call_id: SessionId::from_string("private-after-global-shutdown"),
                status_code: 183,
                reason: "private control".to_string(),
                sdp: None,
            }),
        )
        .await
        .expect("private control delivery deadlocked on global shutdown")
        .expect("private control owner accepted event");
        assert!(matches!(
            control_rx.recv().await,
            Some(Event::CallProgress {
                status_code: 183,
                ..
            })
        ));

        publisher.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn terminal_dispatch_is_bus_first_and_preserves_call_order() {
        let (publisher, lifecycle, mut observed, release, coordinator) =
            blocking_event_publisher(1).await;
        let mut bus = coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .expect("subscribe to authoritative app bus");
        let call_id = SessionId::from_string("bounded-terminal-dispatch");

        publisher.publish(Event::CallProgress {
            call_id: call_id.clone(),
            status_code: 101,
            reason: "first".to_string(),
            sdp: None,
        });
        assert_eq!(next_observed(&mut observed).await, 101);
        assert_eq!(next_bus_observed(&mut bus).await, 101);

        let terminal_publisher = publisher.clone();
        let terminal_call_id = call_id.clone();
        let terminal = tokio::spawn(async move {
            terminal_publisher
                .publish_now(Event::CallEnded {
                    call_id: terminal_call_id,
                    reason: "confirmed BYE".to_string(),
                })
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if lifecycle.snapshot(&call_id, None).terminal.is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("terminal lifecycle was not recorded before queue admission");
        tokio::time::timeout(Duration::from_secs(1), terminal)
            .await
            .expect("terminal publish did not complete")
            .expect("terminal publish task panicked")
            .expect("terminal publish failed");
        assert_eq!(next_bus_observed(&mut bus).await, 999);

        let isolated = coordinator.event_bus_diagnostic_snapshot();
        assert_eq!(isolated["observational_handlers"]["in_flight_current"], 1);
        assert_eq!(isolated["observational_handlers"]["queued_current"], 1);
        assert_eq!(isolated["observational_handlers"]["dropped_full"], 0);

        release.add_permits(2);
        assert_eq!(next_observed(&mut observed).await, 999);

        assert!(matches!(
            lifecycle.snapshot(&call_id, None).terminal,
            Some(CallTerminalInfo::Ended { .. })
        ));
        publisher.shutdown().await;
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test]
    async fn closed_dispatcher_retains_terminal_lifecycle_and_exact_completion() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .expect("create test event coordinator"),
        );
        let lifecycle = LifecycleIndex::new();
        let publisher =
            SessionEventPublisher::with_dispatcher(coordinator, lifecycle.clone(), 1, 1);
        publisher.shutdown().await;

        let handle = exact_terminal_test_handle("closed-terminal-dispatch");
        let call_id = handle.session_id().clone();
        let owner = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => panic!("first exact claim was not owner"),
        };
        let release_ran = Arc::new(AtomicBool::new(false));
        let release_ran_in_future = Arc::clone(&release_ran);
        let outcome = publisher
            .publish_terminal_then_release(
                Event::CallEnded {
                    call_id: call_id.clone(),
                    reason: "confirmed BYE".to_string(),
                },
                async move {
                    release_ran_in_future.store(true, Ordering::Release);
                    Ok(())
                },
            )
            .await;
        assert!(matches!(
            outcome.publication,
            Err(SessionError::Other(ref detail))
                if detail == "Failed to publish app-level event (class=dispatcher-closed)"
        ));
        assert!(outcome.release.is_ok());
        assert!(
            release_ran.load(Ordering::Acquire),
            "closed dispatcher skipped exact release"
        );
        assert!(matches!(
            lifecycle.snapshot(&call_id, None).terminal,
            Some(CallTerminalInfo::Ended { .. })
        ));

        // Publication failure is observational. The exact-release owner still
        // records successful local reclamation, and late contenders observe
        // that immutable completion rather than becoming a second owner.
        owner.finish(ExactTerminalCompletion::PublicationFailed);
        let observer = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Observer(observer) => observer,
            ExactTerminalClaim::Owner(_) => panic!("closed dispatcher created a second owner"),
        };
        assert_eq!(
            observer.wait().await,
            ExactTerminalCompletion::PublicationFailed
        );
        let diagnostics = publisher.dispatcher.metrics_snapshot();
        assert_eq!(diagnostics.closed_admissions, 1);
        assert_eq!(diagnostics.best_effort_dropped, 0);
    }

    #[tokio::test]
    async fn publish_terminal_then_release_waits_for_bus_not_observer_completion() {
        let (publisher, _lifecycle, mut observed, release_handler, coordinator) =
            blocking_event_publisher(1).await;
        let mut bus = coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .expect("subscribe to authoritative app bus");
        let call_id = SessionId::from_string("terminal-release-order");
        let release_ran = Arc::new(AtomicBool::new(false));
        let release_ran_in_future = Arc::clone(&release_ran);
        let task_publisher = publisher.clone();
        let terminal = tokio::spawn(async move {
            task_publisher
                .publish_terminal_then_release(
                    Event::CallEnded {
                        call_id,
                        reason: "confirmed BYE".to_string(),
                    },
                    async move {
                        release_ran_in_future.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .await
        });
        assert_eq!(next_observed(&mut observed).await, 999);
        let outcome = tokio::time::timeout(Duration::from_secs(1), terminal)
            .await
            .expect("terminal release did not complete")
            .expect("terminal release task panicked");
        assert!(outcome.publication.is_ok());
        assert!(outcome.release.is_ok());
        assert!(release_ran.load(Ordering::Acquire));
        assert_eq!(next_bus_observed(&mut bus).await, 999);
        assert_eq!(
            coordinator.event_bus_diagnostic_snapshot()["observational_handlers"]
                ["in_flight_current"],
            1
        );
        release_handler.add_permits(1);
        publisher.shutdown().await;
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatcher_shutdown_does_not_wait_for_hostile_observer() {
        let (publisher, lifecycle, mut observed, release_handler, coordinator) =
            blocking_event_publisher(1).await;
        let call_id = SessionId::from_string("hostile-terminal-observer");
        let release_ran = Arc::new(AtomicBool::new(false));
        let release_ran_in_future = Arc::clone(&release_ran);
        let task_publisher = publisher.clone();
        let terminal = tokio::spawn(async move {
            task_publisher
                .publish_terminal_then_release(
                    Event::CallEnded {
                        call_id,
                        reason: "confirmed BYE".to_string(),
                    },
                    async move {
                        release_ran_in_future.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .await
        });
        assert_eq!(next_observed(&mut observed).await, 999);

        tokio::time::timeout(Duration::from_secs(1), publisher.shutdown())
            .await
            .expect("hostile observer blocked dispatcher shutdown");
        let outcome = tokio::time::timeout(Duration::from_secs(1), terminal)
            .await
            .expect("bus-first publication did not unblock exact release")
            .expect("terminal release task panicked");
        assert!(outcome.publication.is_ok());
        assert!(outcome.release.is_ok());
        assert!(release_ran.load(Ordering::Acquire));
        assert!(matches!(
            lifecycle
                .snapshot(&SessionId::from_string("hostile-terminal-observer"), None)
                .terminal,
            Some(CallTerminalInfo::Ended { .. })
        ));
        let diagnostics = publisher.dispatcher.metrics_snapshot();
        assert_eq!(diagnostics.shutdown_timeouts, 0);
        assert_eq!(diagnostics.shutdown_aborted_workers, 0);
        assert_eq!(diagnostics.closed_admissions, 0);
        release_handler.add_permits(1);
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_first_shutdown_cannot_strand_later_waiters() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .expect("create test event coordinator"),
        );
        let publisher =
            SessionEventPublisher::with_dispatcher(coordinator, LifecycleIndex::new(), 1, 1);
        // Hold the join-handle registry so the owned supervisor remains in
        // progress after it has transitioned the dispatcher to DRAINING.
        let worker_tasks = publisher.dispatcher.worker_tasks.lock().await;

        let first_publisher = publisher.clone();
        let first_shutdown = tokio::spawn(async move {
            first_publisher.shutdown().await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while publisher.dispatcher.state.load(Ordering::Acquire)
                != SESSION_EVENT_DISPATCHER_DRAINING
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first shutdown entered draining");
        first_shutdown.abort();
        let _ = first_shutdown.await;
        drop(worker_tasks);

        tokio::time::timeout(Duration::from_secs(1), publisher.shutdown())
            .await
            .expect("supervised shutdown survived cancellation of its first caller");
        assert_eq!(
            publisher.dispatcher.state.load(Ordering::Acquire),
            SESSION_EVENT_DISPATCHER_CLOSED
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hostile_observer_does_not_block_exact_release_or_later_bus_event() {
        let (publisher, lifecycle, mut observed, release_handler, coordinator) =
            blocking_event_publisher(1).await;
        let mut bus = coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .expect("subscribe to authoritative app bus");
        let handle = exact_terminal_test_handle("publication-deadline-release");
        let call_id = handle.session_id().clone();
        let owner = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => panic!("first exact claim was not owner"),
        };
        let release_ran = Arc::new(AtomicBool::new(false));
        let release_ran_in_future = Arc::clone(&release_ran);
        let task_publisher = publisher.clone();
        let terminal_call_id = call_id.clone();
        let terminal = tokio::spawn(async move {
            let outcome = task_publisher
                .publish_terminal_then_release(
                    Event::CallEnded {
                        call_id: terminal_call_id,
                        reason: "confirmed BYE".to_string(),
                    },
                    async move {
                        release_ran_in_future.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .await;
            if outcome.publication.is_ok() && outcome.release.is_ok() {
                owner.finish(ExactTerminalCompletion::PublishedAndReleased);
            }
            outcome
        });
        assert_eq!(next_observed(&mut observed).await, 999);

        let outcome = tokio::time::timeout(Duration::from_secs(1), terminal)
            .await
            .expect("bus-first publication did not unblock exact release")
            .expect("terminal release task panicked");
        assert!(outcome.publication.is_ok());
        assert!(outcome.release.is_ok());
        assert!(release_ran.load(Ordering::Acquire));
        assert!(matches!(
            lifecycle.snapshot(&call_id, None).terminal,
            Some(CallTerminalInfo::Ended { .. })
        ));
        let observer = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Observer(observer) => observer,
            ExactTerminalClaim::Owner(_) => panic!("publication created a second owner"),
        };
        assert_eq!(
            observer.wait().await,
            ExactTerminalCompletion::PublishedAndReleased
        );
        assert_eq!(next_bus_observed(&mut bus).await, 999);

        publisher
            .publish_now(Event::CallProgress {
                call_id,
                status_code: 204,
                reason: "worker continued".to_string(),
                sdp: None,
            })
            .await
            .expect("hostile observer blocked the dispatcher worker");
        assert_eq!(next_bus_observed(&mut bus).await, 204);
        release_handler.add_permits(2);
        assert_eq!(next_observed(&mut observed).await, 204);
        let diagnostics = publisher.dispatcher.metrics_snapshot();
        assert_eq!(diagnostics.publication_timeouts, 0);
        assert_eq!(diagnostics.publication_failures, 0);
        publisher.shutdown().await;
        coordinator.shutdown().await.expect("shutdown coordinator");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn exact_terminal_claim_completion_wakes_all_observers() {
        let slot = Arc::new(ExactTerminalClaimSlot::pending());
        let mut waiters = Vec::new();

        for _ in 0..128 {
            let slot = Arc::clone(&slot);
            waiters.push(tokio::spawn(async move { slot.wait().await }));
        }

        tokio::task::yield_now().await;
        assert!(slot.finish(ExactTerminalCompletion::PublishedAndReleased));
        assert!(!slot.finish(ExactTerminalCompletion::PublicationFailed));

        for waiter in waiters {
            let completion = tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("exact terminal claim waiter timed out")
                .expect("exact terminal claim waiter panicked");
            assert_eq!(completion, ExactTerminalCompletion::PublishedAndReleased);
        }
    }

    #[tokio::test]
    async fn dropped_exact_terminal_owner_wakes_observer() {
        let slot = Arc::new(ExactTerminalClaimSlot::pending());
        let observer = ExactTerminalClaimObserver {
            observation: ExactTerminalClaimObservation::Pending(Arc::clone(&slot)),
        };
        assert!(slot.finish(ExactTerminalCompletion::OwnerDropped));

        let completion = tokio::time::timeout(Duration::from_secs(1), observer.wait())
            .await
            .expect("exact terminal claim observer timed out");
        assert_eq!(completion, ExactTerminalCompletion::OwnerDropped);
    }

    #[tokio::test]
    async fn completed_exact_terminal_claim_releases_slot_and_serves_late_observer() {
        let claims = ExactTerminalClaims::default();
        let handle = exact_terminal_test_handle("compact-terminal-claim");
        let owner = match claims.claim(&handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => panic!("first exact claim was not owner"),
        };
        let retained_slot = Arc::downgrade(&owner.slot);

        owner.finish(ExactTerminalCompletion::PublishedAndReleased);

        let key = ExactTerminalClaimKey::from(&handle);
        assert!(matches!(
            claims.slots.get(&key).as_deref(),
            Some(ExactTerminalClaimState::Completed {
                completion: ExactTerminalCompletion::PublishedAndReleased,
                ..
            })
        ));
        assert!(
            retained_slot.upgrade().is_none(),
            "completed map entry retained the pending Notify slot"
        );

        let observer = match claims.claim(&handle) {
            ExactTerminalClaim::Observer(observer) => observer,
            ExactTerminalClaim::Owner(_) => panic!("late exact claim became a second owner"),
        };
        let completion = tokio::time::timeout(Duration::from_secs(1), observer.wait())
            .await
            .expect("compact exact claim did not resolve immediately");
        assert_eq!(completion, ExactTerminalCompletion::PublishedAndReleased);
    }

    #[tokio::test]
    async fn pending_exact_terminal_observer_survives_map_compaction() {
        let claims = ExactTerminalClaims::default();
        let handle = exact_terminal_test_handle("pending-terminal-observer");
        let owner = match claims.claim(&handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => panic!("first exact claim was not owner"),
        };
        let retained_slot = Arc::downgrade(&owner.slot);
        let observer = match claims.claim(&handle) {
            ExactTerminalClaim::Observer(observer) => observer,
            ExactTerminalClaim::Owner(_) => panic!("concurrent exact claim became a second owner"),
        };

        owner.finish(ExactTerminalCompletion::ReleaseFailed);
        assert!(retained_slot.upgrade().is_some());
        let key = ExactTerminalClaimKey::from(&handle);
        assert!(matches!(
            claims.slots.get(&key).as_deref(),
            Some(ExactTerminalClaimState::Completed {
                completion: ExactTerminalCompletion::ReleaseFailed,
                ..
            })
        ));

        let completion = tokio::time::timeout(Duration::from_secs(1), observer.wait())
            .await
            .expect("pending observer did not receive exact completion");
        assert_eq!(completion, ExactTerminalCompletion::ReleaseFailed);
        assert!(
            retained_slot.upgrade().is_none(),
            "consumed observer retained the pending Notify slot"
        );
    }

    #[test]
    fn compact_exact_terminal_claim_expires_from_due_queue() {
        let claims = ExactTerminalClaims::default();
        let handle = exact_terminal_test_handle("expired-terminal-claim");
        let mut owner = match claims.claim(&handle) {
            ExactTerminalClaim::Owner(owner) => owner,
            ExactTerminalClaim::Observer(_) => panic!("first exact claim was not owner"),
        };
        assert!(owner
            .slot
            .finish(ExactTerminalCompletion::PublishedAndReleased));
        claims.compact_completed(
            owner.key,
            &owner.slot,
            ExactTerminalCompletion::PublishedAndReleased,
            Instant::now() - TERMINAL_EVENT_TTL - Duration::from_millis(1),
        );
        owner.finished = true;
        drop(owner);

        assert_eq!(claims.prune_due(Instant::now()), 1);
        assert!(!claims
            .slots
            .contains_key(&ExactTerminalClaimKey::from(&handle)));
        assert!(claims
            .deadlines
            .lock()
            .expect("exact deadline queue")
            .by_deadline
            .is_empty());

        assert!(matches!(
            claims.claim(&handle),
            ExactTerminalClaim::Owner(_)
        ));
    }

    #[test]
    fn exact_terminal_claim_expiry_converges_in_bounded_waves() {
        const RECORDS: usize = 5;
        const BATCH: usize = 2;

        let claims = ExactTerminalClaims::default();
        // Keep records live while constructing them because `claim` performs
        // its normal hot-path due pruning. Advance only the explicit prune
        // timestamp below so this test measures bounded waves, not setup-time
        // expiry.
        let completed_at = Instant::now();
        let expired_at = completed_at + TERMINAL_EVENT_TTL + Duration::from_millis(1);
        for sequence in 0..RECORDS {
            let handle = exact_terminal_test_handle(&format!("bounded-claim-{sequence}"));
            let mut owner = match claims.claim(&handle) {
                ExactTerminalClaim::Owner(owner) => owner,
                ExactTerminalClaim::Observer(_) => panic!("unique exact claim was not owner"),
            };
            assert!(owner
                .slot
                .finish(ExactTerminalCompletion::PublishedAndReleased));
            claims.compact_completed(
                owner.key,
                &owner.slot,
                ExactTerminalCompletion::PublishedAndReleased,
                completed_at,
            );
            owner.finished = true;
        }

        assert_eq!(
            prune_due_exact_terminal_claims(&claims.slots, &claims.deadlines, expired_at, BATCH,),
            BATCH
        );
        assert_eq!(claims.slots.len(), RECORDS - BATCH);
        assert_eq!(
            prune_due_exact_terminal_claims(&claims.slots, &claims.deadlines, expired_at, BATCH,),
            BATCH
        );
        assert_eq!(
            prune_due_exact_terminal_claims(&claims.slots, &claims.deadlines, expired_at, BATCH,),
            1
        );
        assert!(claims.slots.is_empty());
        assert!(claims
            .deadlines
            .lock()
            .expect("exact deadline queue")
            .by_deadline
            .is_empty());
    }

    #[test]
    fn exact_terminal_claim_key_qualifies_generation_and_registry_slot() {
        let first = exact_terminal_test_handle("qualified-terminal-claim");
        let next_generation = exact_terminal_test_handle("qualified-terminal-claim");
        let next_slot = first.with_next_slot_revision_for_test();

        assert_eq!(first.session_id(), next_generation.session_id());
        assert_ne!(
            ExactTerminalClaimKey::from(&first),
            ExactTerminalClaimKey::from(&next_generation),
            "authority generation must qualify a reused application identifier"
        );
        assert_ne!(
            ExactTerminalClaimKey::from(&first),
            ExactTerminalClaimKey::from(&next_slot),
            "registry slot revision must qualify a replaced exact mapping"
        );

        let claims = ExactTerminalClaims::default();
        assert!(matches!(claims.claim(&first), ExactTerminalClaim::Owner(_)));
        assert!(matches!(
            claims.claim(&next_generation),
            ExactTerminalClaim::Owner(_)
        ));
        assert!(matches!(
            claims.claim(&next_slot),
            ExactTerminalClaim::Owner(_)
        ));
    }

    #[test]
    fn stale_exact_terminal_deadline_cannot_remove_newer_completion() {
        let claims = ExactTerminalClaims::default();
        let handle = exact_terminal_test_handle("stale-terminal-deadline");
        let key = ExactTerminalClaimKey::from(&handle);
        let now = Instant::now();
        let mut deadlines = claims.deadlines.lock().expect("exact deadline queue");
        let stale_generation =
            deadlines.schedule(key, now - TERMINAL_EVENT_TTL - Duration::from_millis(1));
        let current_generation = deadlines.schedule(key, now);
        assert_ne!(stale_generation, current_generation);
        drop(deadlines);
        claims.slots.insert(
            key,
            ExactTerminalClaimState::Completed {
                completion: ExactTerminalCompletion::PublishedAndReleased,
                deadline_generation: current_generation,
            },
        );

        assert_eq!(claims.prune_due(now), 0);
        assert!(matches!(
            claims.slots.get(&key).as_deref(),
            Some(ExactTerminalClaimState::Completed {
                deadline_generation,
                ..
            }) if *deadline_generation == current_generation
        ));
    }

    #[test]
    fn lifecycle_deadlines_are_due_driven_and_deduplicated() {
        let mut deadlines = LifecycleDeadlineQueue::default();
        let first = SessionId::from_string("first");
        let second = SessionId::from_string("second");
        let now = Instant::now();

        let first_stored_at = now - TERMINAL_EVENT_TTL - Duration::from_secs(1);
        let first_generation = deadlines.schedule(Arc::new(first.clone()), first_stored_at, None);
        deadlines.schedule(Arc::new(second.clone()), now, None);
        // A newer deadline for the same call must replace the stale one.
        deadlines.schedule(
            Arc::new(first.clone()),
            now,
            Some((first_stored_at, first_generation)),
        );

        assert!(deadlines.take_due(now, usize::MAX).is_empty());
        let mut due = deadlines.take_due(
            now + TERMINAL_EVENT_TTL + Duration::from_millis(1),
            usize::MAX,
        );
        due.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
        assert_eq!(
            due.iter()
                .map(|(call_id, _)| call_id.as_str())
                .collect::<Vec<_>>(),
            vec![first.as_str(), second.as_str()]
        );
        assert!(deadlines.by_deadline.is_empty());
    }

    #[test]
    fn lifecycle_expiry_converges_in_bounded_waves() {
        const RECORDS: usize = 5;
        const BATCH: usize = 2;

        let index = LifecycleIndex::with_capacity(128);
        for sequence in 0..RECORDS {
            index.record_event(&Event::CallEnded {
                call_id: SessionId::from_string(format!("bounded-lifecycle-{sequence}")),
                reason: "normal".to_string(),
            });
        }
        let after_horizon = Instant::now() + TERMINAL_EVENT_TTL + Duration::from_millis(1);

        assert_eq!(
            prune_due_terminal_entries_from(
                &index.entries,
                &index.waiters,
                &index.terminal_deadlines,
                after_horizon,
                BATCH,
            ),
            BATCH
        );
        assert_eq!(index.entries.len(), RECORDS - BATCH);
        assert_eq!(
            prune_due_terminal_entries_from(
                &index.entries,
                &index.waiters,
                &index.terminal_deadlines,
                after_horizon,
                BATCH,
            ),
            BATCH
        );
        assert_eq!(
            prune_due_terminal_entries_from(
                &index.entries,
                &index.waiters,
                &index.terminal_deadlines,
                after_horizon,
                BATCH,
            ),
            1
        );
        assert!(index.entries.is_empty());
        assert!(index
            .terminal_deadlines
            .lock()
            .expect("lifecycle deadline queue")
            .by_deadline
            .is_empty());
    }

    #[test]
    fn lifecycle_deadline_shares_key_and_stale_generation_cannot_prune() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::from_string("shared-lifecycle-deadline-key");
        index.record_event(&Event::CallEnded {
            call_id: call_id.clone(),
            reason: "normal".to_string(),
        });

        let (retained_key, current_generation) = {
            let entry = index.entries.get(&call_id).expect("terminal entry");
            let generation = match entry.value() {
                LifecycleEntry::Terminal(terminal) => terminal.deadline_generation,
                LifecycleEntry::Active(_) => panic!("terminal event retained active entry"),
            };
            (Arc::clone(entry.key()), generation)
        };
        {
            let mut deadlines = index
                .terminal_deadlines
                .lock()
                .expect("lifecycle deadline queue");
            let scheduled_key = deadlines
                .by_deadline
                .values()
                .next()
                .expect("terminal deadline");
            assert!(
                Arc::ptr_eq(&retained_key, scheduled_key),
                "terminal deadline must share the entry identifier allocation"
            );

            let stale_generation = current_generation.wrapping_add(1);
            deadlines.by_deadline.insert(
                (Instant::now() - Duration::from_millis(1), stale_generation),
                Arc::clone(&retained_key),
            );
        }

        assert_eq!(index.prune_expired_terminal_entries(), 0);
        assert!(index.entries.contains_key(&call_id));
    }

    #[test]
    fn qualified_retention_capacity_is_not_eagerly_reserved() {
        const QUALIFIED_RETAINED_CAPACITY: usize = 168_000;
        let index = LifecycleIndex::with_capacity(QUALIFIED_RETAINED_CAPACITY);

        assert!(index.entries.capacity() < QUALIFIED_RETAINED_CAPACITY);
        assert!(index.waiters.capacity() < QUALIFIED_RETAINED_CAPACITY);
        assert!(index.entries.is_empty());
        assert!(index.waiters.is_empty());
    }

    #[test]
    fn lifecycle_debug_is_payload_free() {
        const SECRET: &str = "lifecycle-debug-secret-canary";
        let progress = CallProgressInfo {
            call_id: SessionId::from_string(SECRET),
            status_code: 183,
            reason: SECRET.to_string(),
            sdp: Some(SECRET.to_string()),
        };
        let answered = CallAnsweredInfo {
            call_id: SessionId::from_string(SECRET),
            sdp: Some(SECRET.to_string()),
        };
        let terminal = CallTerminalInfo::Failed {
            status_code: 500,
            reason: SECRET.to_string(),
        };
        for rendered in [
            format!("{progress:?}"),
            format!("{answered:?}"),
            format!("{terminal:?}"),
        ] {
            assert!(!rendered.contains(SECRET), "debug leaked: {rendered}");
        }
    }

    #[test]
    fn lifecycle_records_progress_and_terminal() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();

        index.record_event(&Event::CallProgress {
            call_id: call_id.clone(),
            status_code: 183,
            reason: "Session Progress".to_string(),
            sdp: Some("v=0".to_string()),
        });
        let active = index.snapshot(&call_id, Some(CallState::Ringing));
        assert_eq!(active.progress.len(), 1);
        assert_eq!(active.progress[0].status_code, 183);
        assert_eq!(active.progress[0].sdp.as_deref(), Some("v=0"));

        index.record_event(&Event::CallCancelled {
            call_id: call_id.clone(),
        });

        let snapshot = index.snapshot(&call_id, Some(CallState::Cancelled));
        assert!(snapshot.progress.is_empty());
        assert_eq!(snapshot.terminal, Some(CallTerminalInfo::Cancelled));
        assert_eq!(
            snapshot.terminal.as_ref().map(CallTerminalInfo::reason),
            Some("Cancelled".to_string())
        );
    }

    #[test]
    fn lifecycle_records_media_security() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();

        index.record_event(&Event::MediaSecurityNegotiated {
            call_id: call_id.clone(),
            keying: crate::api::events::MediaSecurityKeying::Sdes,
            suite: rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_80,
            profile: crate::api::events::MediaSecurityProfile::RtpSavp,
            contexts_installed: true,
        });

        let snapshot = index.snapshot(&call_id, None);
        assert!(snapshot.media_security.is_some());
    }

    #[tokio::test]
    async fn lifecycle_watcher_wakes_only_matching_session() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();
        let other_call_id = SessionId::new();
        let mut watcher = index.watcher(&call_id);

        index.record_event(&Event::CallAnswered {
            call_id: other_call_id,
            sdp: None,
        });
        assert!(watcher.has_changed().is_ok_and(|changed| !changed));

        index.record_event(&Event::CallAnswered {
            call_id: call_id.clone(),
            sdp: None,
        });
        watcher.changed().await.unwrap();

        let snapshot = index.snapshot(&call_id, Some(CallState::Active));
        assert!(snapshot.answered.is_some());
    }

    #[tokio::test]
    async fn lifecycle_watcher_resolves_from_late_snapshot() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();

        index.record_event(&Event::CallEnded {
            call_id: call_id.clone(),
            reason: "Normal".to_string(),
        });

        let snapshot = index.snapshot(&call_id, Some(CallState::Terminated));
        assert_eq!(
            snapshot.terminal.as_ref().map(CallTerminalInfo::reason),
            Some("Normal".to_string())
        );
    }

    #[tokio::test]
    async fn lifecycle_watcher_handles_many_concurrent_waiters() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();
        let mut waiters = Vec::new();

        for _ in 0..256 {
            waiters.push(index.watcher(&call_id));
        }

        index.record_event(&Event::CallAnswered {
            call_id: call_id.clone(),
            sdp: None,
        });

        for waiter in &mut waiters {
            waiter.changed().await.unwrap();
        }

        let snapshot = index.snapshot(&call_id, Some(CallState::Active));
        assert!(snapshot.answered.is_some());
    }
}
