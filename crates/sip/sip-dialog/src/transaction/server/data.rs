use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportRoute};
/// # Server Transaction Data Structures
///
/// This module provides data structures and traits for implementing the server transaction
/// state machines defined in RFC 3261 Section 17.2.
///
/// Server transactions in SIP are responsible for:
/// - Processing incoming requests from clients
/// - Sending responses reliably
/// - Managing state transitions based on request/response types
/// - Handling retransmissions according to RFC 3261 rules
///
/// The key components in this module are:
/// - `ServerTransactionData`: Core data structure shared by all server transaction types
/// - `CommonServerTransaction`: Trait providing shared behavior across transaction types
/// - Command channels for communication with the transaction's event loop
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::transaction::event_sender::TransactionEventSender;
use crate::transaction::runner::{
    AsRefKey, AsRefState, HasCommandSender, HasLifecycle, HasTransactionEvents, HasTransport,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::timer::TimerSettings;
use crate::transaction::{
    AtomicTransactionState, InternalTransactionCommand, TransactionKey, TransactionState,
};

const FINAL_RESPONSE_IDLE: u64 = 0;
const FINAL_RESPONSE_PENDING: u64 = 1;
const FINAL_RESPONSE_WRITE_IN_FLIGHT: u64 = 2;
const FINAL_RESPONSE_WRITTEN: u64 = 3;
const FINAL_RESPONSE_FAILED_BEFORE_WRITE: u64 = 4;
const FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY: u64 = 5;
const FINAL_RESPONSE_STATE_BITS: u32 = 3;
const FINAL_RESPONSE_STATE_MASK: u64 = (1 << FINAL_RESPONSE_STATE_BITS) - 1;
const FINAL_RESPONSE_GENERATION_MAX: u64 = u64::MAX >> FINAL_RESPONSE_STATE_BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalResponseSupervisionGeneration(u64);

/// Authoritative completion classification for one exact final server response.
///
/// Only a response proven not to have crossed the transport-write boundary is
/// retryable. A successful write and an error after entering that boundary both
/// leave final-response ownership with the transaction layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalResponseCompletionDisposition {
    /// The transport write completed successfully. The response is terminal and
    /// no upper layer may author another final response.
    WrittenSuccessTerminal,
    /// The attempt failed before entering the transport-write boundary. The
    /// exact transaction may be retried under a new supervision generation.
    ZeroWireRetryable,
    /// The attempt failed after entering the transport-write boundary. Whether
    /// the peer observed bytes is unknown, so retrying could duplicate a final
    /// response and is forbidden.
    WireUnknownErrorTerminal,
}

const fn final_response_supervision_state(value: u64) -> u64 {
    value & FINAL_RESPONSE_STATE_MASK
}

const fn final_response_supervision_generation(value: u64) -> FinalResponseSupervisionGeneration {
    FinalResponseSupervisionGeneration(value >> FINAL_RESPONSE_STATE_BITS)
}

const fn pack_final_response_supervision(
    generation: FinalResponseSupervisionGeneration,
    state: u64,
) -> u64 {
    (generation.0 << FINAL_RESPONSE_STATE_BITS) | state
}

/// Command sender for transaction event loops.
///
/// Used to send commands to the transaction's internal event loop, allowing
/// asynchronous control of the transaction's behavior.
pub type CommandSender = mpsc::Sender<InternalTransactionCommand>;

/// Backward-compatible receiver alias. Server transaction data no longer
/// stores a receiver (the runner owns it directly), but downstream code may
/// still use this public name when constructing transaction plumbing.
pub type CommandReceiver = mpsc::Receiver<InternalTransactionCommand>;

/// Common data structure for both INVITE and non-INVITE server transactions.
///
/// This structure contains all the state required for implementing the server transaction
/// state machines defined in RFC 3261 Section 17.2. It includes:
///
/// - Identity information (transaction key)
/// - State tracking (current transaction state)
/// - Message storage (original request, last response)
/// - Communication channels (transport, event channels, command channels)
/// - Timer configuration
///
/// Both `ServerInviteTransaction` and `ServerNonInviteTransaction` use this structure
/// as their core data store, while implementing different behavior around it.
pub struct ServerTransactionData {
    /// Transaction ID based on RFC 3261 transaction matching rules
    pub id: TransactionKey,

    /// Current transaction state (Trying/Proceeding, Completed, Confirmed, Terminated)
    pub state: Arc<AtomicTransactionState>,

    /// Transaction lifecycle state for robust shutdown coordination
    pub lifecycle: Arc<std::sync::atomic::AtomicU8>, // Using AtomicU8 for TransactionLifecycle

    /// Original request that initiated this transaction.
    /// `Arc<Request>` — see `client/data.rs` for rationale.
    pub request: Arc<Request>,

    /// Last response sent by this transaction
    pub last_response: Arc<Mutex<Option<Response>>>,

    /// Exact first-write fence for a final response. This advances only after
    /// the transport write returns success and the immutable replay response
    /// has been stored, before the fallible/cancellable Completed command is
    /// enqueued. BYE cleanup uses it to distinguish a pre-wire failure from a
    /// wire-written response that still requires its natural Timer J owner.
    pub(crate) final_response_wire_written: std::sync::atomic::AtomicBool,

    /// A final response accepted by the manager is owned by the existing
    /// transaction runner until the transport result and replay fence are
    /// exact. Cancellation of the API waiter cannot cancel that runner work.
    pub(crate) final_response_supervision_state: AtomicU64,
    pub(crate) final_response_supervision_notify: tokio::sync::Notify,

    /// Remote address to which responses are sent
    pub remote_addr: SocketAddr,

    /// Exact route back to the ingress flow. Connection-oriented responses
    /// must use this rather than rediscovering a socket by address.
    pub response_route: TransportRoute,

    /// Transport layer for sending SIP messages
    pub transport: Arc<dyn Transport>,

    /// Channel for sending events to the Transaction User (TU)
    pub events_tx: TransactionEventSender,

    /// Channel for sending commands to the transaction's event loop
    pub cmd_tx: CommandSender,

    /// Handle to the transaction's event loop task
    pub event_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,

    /// Manager-owned, bounded cleanup queue. This is absent for transactions
    /// constructed directly outside a `TransactionManager`.
    pub(crate) termination_cleanup_tx: std::sync::OnceLock<mpsc::WeakSender<TransactionKey>>,

    /// Handle for the one due-driven lifecycle scheduler owned by the
    /// transaction manager.
    pub(crate) lifecycle_scheduler:
        std::sync::OnceLock<crate::transaction::lifecycle_scheduler::WeakLifecycleSchedulerHandle>,

    /// Admission-time lease for the UDP non-INVITE Timer J replay horizon.
    /// It is shared with the compact replay tombstone at retirement.
    pub(crate) compact_retention_reservation:
        std::sync::OnceLock<crate::transaction::lifecycle_scheduler::CompactRetentionReservation>,

    /// Exact wire-key admission owner shared with compact Timer J or the
    /// terminal-event fence until dialog routing cleanup is complete.
    pub(crate) transaction_admission_owner:
        std::sync::OnceLock<crate::transaction::manager::TransactionAdmissionOwner>,

    /// Weak access to the manager admission fence. A runner-owned response
    /// upgrades this and retains an operation guard after its API waiter is
    /// cancelled, preventing shutdown from clearing the transaction mid-write.
    pub(crate) manager_admission_lifecycle: std::sync::OnceLock<
        std::sync::Weak<crate::transaction::manager::TransactionManagerAdmissionLifecycle>,
    >,

    /// Shared one-shot claim for runner versus explicit manager termination.
    pub(crate) terminal_event_publication:
        Arc<crate::transaction::event_sender::TerminalEventPublication>,

    /// Configuration for transaction timers (T1, T2, etc.)
    pub timer_config: TimerSettings,
}

impl std::fmt::Debug for ServerTransactionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerTransactionData")
            .field(
                "id",
                &crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id),
            )
            .field("state", &self.state.get())
            .field("remote_addr", &self.remote_addr)
            .field("response_route", &self.response_route)
            .field("request_header_count", &self.request.all_headers().len())
            .field("request_body_len", &self.request.body().len())
            .field(
                "has_last_response",
                &self
                    .last_response
                    .try_lock()
                    .map(|response| response.is_some())
                    .unwrap_or(false),
            )
            .field(
                "final_response_wire_written",
                &self
                    .final_response_wire_written
                    .load(std::sync::atomic::Ordering::Acquire),
            )
            .field(
                "has_event_loop",
                &self
                    .event_loop_handle
                    .try_lock()
                    .map(|handle| handle.is_some())
                    .unwrap_or(false),
            )
            .finish()
    }
}

impl Drop for ServerTransactionData {
    fn drop(&mut self) {
        // Try to terminate the event loop when the transaction is dropped
        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id), "ServerTransactionData dropped, attempting to terminate event loop");

        if let Ok(mut handle_guard) = self.event_loop_handle.try_lock() {
            if let Some(handle) = handle_guard.take() {
                handle.abort();
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id), "Aborted server transaction event loop");
            }
        }
    }
}

impl ServerTransactionData {
    pub(crate) fn begin_final_response_supervision(&self) -> bool {
        let mut current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        loop {
            let state = final_response_supervision_state(current);
            if !matches!(
                state,
                FINAL_RESPONSE_IDLE | FINAL_RESPONSE_FAILED_BEFORE_WRITE
            ) {
                return false;
            }
            let generation = final_response_supervision_generation(current).0;
            if generation == FINAL_RESPONSE_GENERATION_MAX {
                // Never wrap the compact generation tag. Reusing a tag would
                // let a very late guard from an old, proven-pre-wire attempt
                // alter the state of a newer response attempt (an ABA race).
                return false;
            }
            let next = pack_final_response_supervision(
                FinalResponseSupervisionGeneration(generation + 1),
                FINAL_RESPONSE_PENDING,
            );
            match self.final_response_supervision_state.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn mark_final_response_write_in_flight(&self) {
        let Some(generation) = self.pending_final_response_generation() else {
            return;
        };
        let _ = self.transition_final_response_supervision(
            generation,
            FINAL_RESPONSE_PENDING,
            FINAL_RESPONSE_WRITE_IN_FLIGHT,
        );
    }

    pub(crate) fn mark_final_response_wire_written(&self) {
        self.final_response_wire_written
            .store(true, Ordering::Release);
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        let generation = final_response_supervision_generation(current);
        let transitioned = self.transition_final_response_supervision(
            generation,
            FINAL_RESPONSE_WRITE_IN_FLIGHT,
            FINAL_RESPONSE_WRITTEN,
        );
        debug_assert!(
            transitioned,
            "a successful final-response write must own the active supervision generation"
        );
        self.final_response_supervision_notify.notify_waiters();
    }

    pub(crate) fn mark_final_response_failed_before_write(&self) {
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        let generation = final_response_supervision_generation(current);
        self.mark_final_response_failed_before_write_for_generation(generation);
    }

    pub(crate) fn mark_final_response_failed_before_write_for_generation(
        &self,
        generation: FinalResponseSupervisionGeneration,
    ) {
        let _ = self.transition_final_response_supervision(
            generation,
            FINAL_RESPONSE_PENDING,
            FINAL_RESPONSE_FAILED_BEFORE_WRITE,
        );
        self.final_response_supervision_notify.notify_waiters();
    }

    pub(crate) fn mark_final_response_failed_after_write_boundary(&self) {
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        let generation = final_response_supervision_generation(current);
        self.mark_final_response_failed_after_write_boundary_for_generation(generation);
    }

    pub(crate) fn mark_final_response_failed_after_write_boundary_for_generation(
        &self,
        generation: FinalResponseSupervisionGeneration,
    ) {
        let _ = self.transition_final_response_supervision(
            generation,
            FINAL_RESPONSE_WRITE_IN_FLIGHT,
            FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY,
        );
        self.final_response_supervision_notify.notify_waiters();
    }

    fn transition_final_response_supervision(
        &self,
        generation: FinalResponseSupervisionGeneration,
        from: u64,
        to: u64,
    ) -> bool {
        self.final_response_supervision_state
            .compare_exchange(
                pack_final_response_supervision(generation, from),
                pack_final_response_supervision(generation, to),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn pending_final_response_generation(&self) -> Option<FinalResponseSupervisionGeneration> {
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        (final_response_supervision_state(current) == FINAL_RESPONSE_PENDING)
            .then(|| final_response_supervision_generation(current))
    }

    pub(crate) fn current_final_response_supervision_generation(
        &self,
    ) -> Option<FinalResponseSupervisionGeneration> {
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        (final_response_supervision_state(current) != FINAL_RESPONSE_IDLE)
            .then(|| final_response_supervision_generation(current))
    }

    fn final_response_state_for_generation(
        &self,
        generation: FinalResponseSupervisionGeneration,
    ) -> Option<u64> {
        let current = self
            .final_response_supervision_state
            .load(Ordering::Acquire);
        (final_response_supervision_generation(current) == generation)
            .then(|| final_response_supervision_state(current))
    }

    #[cfg(test)]
    pub(crate) fn final_response_supervision_is_pending_for_test(&self) -> bool {
        final_response_supervision_state(
            self.final_response_supervision_state
                .load(Ordering::Acquire),
        ) == FINAL_RESPONSE_PENDING
    }

    /// Wait for one exact supervision generation to reach an authoritative
    /// transport disposition. This deliberately does not follow a newer active
    /// generation: advancing the generation proves that the requested attempt
    /// ended before the transport-write boundary.
    pub(crate) async fn await_final_response_completion_for_generation(
        &self,
        generation: FinalResponseSupervisionGeneration,
    ) -> FinalResponseCompletionDisposition {
        loop {
            let notified = self.final_response_supervision_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let current = self
                .final_response_supervision_state
                .load(Ordering::Acquire);
            if final_response_supervision_generation(current) != generation {
                return FinalResponseCompletionDisposition::ZeroWireRetryable;
            }
            match final_response_supervision_state(current) {
                FINAL_RESPONSE_IDLE | FINAL_RESPONSE_FAILED_BEFORE_WRITE => {
                    return FinalResponseCompletionDisposition::ZeroWireRetryable;
                }
                FINAL_RESPONSE_WRITTEN => {
                    return FinalResponseCompletionDisposition::WrittenSuccessTerminal;
                }
                FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY => {
                    return FinalResponseCompletionDisposition::WireUnknownErrorTerminal;
                }
                FINAL_RESPONSE_PENDING | FINAL_RESPONSE_WRITE_IN_FLIGHT => notified.await,
                _ => unreachable!("invalid final-response supervision state"),
            }
        }
    }

    /// Wait for an accepted runner-owned final response to cross an exact
    /// transport boundary or prove that it ended before a successful write.
    pub(crate) async fn await_final_response_wire_outcome(&self) -> bool {
        loop {
            let notified = self.final_response_supervision_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match final_response_supervision_state(
                self.final_response_supervision_state
                    .load(Ordering::Acquire),
            ) {
                FINAL_RESPONSE_WRITTEN | FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY => return true,
                FINAL_RESPONSE_IDLE | FINAL_RESPONSE_FAILED_BEFORE_WRITE => return false,
                FINAL_RESPONSE_PENDING | FINAL_RESPONSE_WRITE_IN_FLIGHT => {}
                _ => unreachable!("invalid final-response supervision state"),
            }
            notified.await;
        }
    }

    pub(crate) fn final_response_may_have_reached_wire(&self) -> bool {
        self.final_response_wire_written.load(Ordering::Acquire)
            || matches!(
                final_response_supervision_state(
                    self.final_response_supervision_state
                        .load(Ordering::Acquire),
                ),
                FINAL_RESPONSE_WRITE_IN_FLIGHT
                    | FINAL_RESPONSE_WRITTEN
                    | FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY
            )
    }

    pub(crate) fn install_termination_cleanup_sender(&self, sender: mpsc::Sender<TransactionKey>) {
        let _ = self.termination_cleanup_tx.set(sender.downgrade());
    }

    pub(crate) fn install_lifecycle_scheduler(
        &self,
        scheduler: crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle,
    ) {
        let _ = self.lifecycle_scheduler.set(scheduler.downgrade());
    }

    pub(crate) fn install_compact_retention_reservation(
        &self,
        reservation: crate::transaction::lifecycle_scheduler::CompactRetentionReservation,
    ) {
        let _ = self.compact_retention_reservation.set(reservation);
    }

    pub(crate) fn install_transaction_admission_owner(
        &self,
        owner: crate::transaction::manager::TransactionAdmissionOwner,
    ) {
        let _ = self.transaction_admission_owner.set(owner);
    }

    pub(crate) fn install_manager_admission_lifecycle(
        &self,
        lifecycle: &Arc<crate::transaction::manager::TransactionManagerAdmissionLifecycle>,
    ) {
        let _ = self
            .manager_admission_lifecycle
            .set(Arc::downgrade(lifecycle));
    }

    fn acquire_manager_response_operation(
        &self,
    ) -> crate::transaction::error::Result<
        Option<crate::transaction::manager::TransactionManagerAdmissionGuard>,
    > {
        let Some(lifecycle) = self.manager_admission_lifecycle.get() else {
            // A transaction constructed directly outside a manager has no
            // manager shutdown fence to retain.
            return Ok(None);
        };
        let lifecycle = lifecycle.upgrade().ok_or_else(|| {
            crate::transaction::error::Error::Other(
                "transaction manager admission fence is unavailable".to_string(),
            )
        })?;
        lifecycle.try_enter_existing().map(Some).ok_or_else(|| {
            crate::transaction::error::Error::Other(
                "transaction manager is stopping supervised responses".to_string(),
            )
        })
    }

    pub(crate) fn transaction_admission_owner(
        &self,
    ) -> Option<crate::transaction::manager::TransactionAdmissionOwner> {
        self.transaction_admission_owner.get().cloned()
    }

    /// Replace a completed UDP non-INVITE server runner with the compact
    /// manager-owned Timer J replay tombstone. The immutable response bytes
    /// and exact ingress route are the only protocol state retained. Low-level
    /// transactions use the shared runtime deadline worker instead.
    pub(crate) async fn schedule_compact_timer_j(
        self: Arc<Self>,
        delay: std::time::Duration,
    ) -> bool {
        let identity = Arc::as_ptr(&self) as usize;
        let Some(scheduler) = self.lifecycle_scheduler.get().cloned() else {
            return crate::transaction::lifecycle_scheduler::schedule_standalone_non_invite_timer(
                identity,
                self.id.clone(),
                crate::transaction::lifecycle_scheduler::CompactNonInviteTimer::J,
                delay,
                self.cmd_tx.clone(),
            )
            .await;
        };
        let Some(response) = self.last_response.lock().await.clone() else {
            return false;
        };
        scheduler
            .schedule_compact_non_invite_with_reservation(
                identity,
                self.id.clone(),
                crate::transaction::lifecycle_scheduler::CompactNonInviteTimer::J,
                delay,
                Some((
                    bytes::Bytes::from(rvoip_sip_core::Message::Response(response).to_bytes()),
                    self.response_route.clone(),
                )),
                self.state.clone(),
                None,
                self.cmd_tx.clone(),
                self.compact_retention_reservation.get().cloned(),
                self.transaction_admission_owner(),
                Arc::clone(&self.terminal_event_publication),
            )
            .await
    }

    pub(crate) async fn schedule_termination(self: Arc<Self>) -> bool {
        let identity = Arc::as_ptr(&self) as usize;
        let commands =
            std::collections::VecDeque::from([InternalTransactionCommand::TransitionTo(
                crate::transaction::TransactionState::Terminated,
            )]);
        if let Some(scheduler) = self.lifecycle_scheduler.get().cloned() {
            return scheduler
                .schedule_commands(identity, self.id.clone(), self.cmd_tx.clone(), commands)
                .await;
        }
        crate::transaction::lifecycle_scheduler::schedule_standalone_commands(
            identity,
            self.id.clone(),
            self.cmd_tx.clone(),
            commands,
        )
        .await
    }
}

/// Runner-owned response operation. The command queue owns this object after
/// admission, so dropping an API waiter cannot cancel the transport write.
#[doc(hidden)]
pub struct SupervisedServerResponse {
    data: Arc<ServerTransactionData>,
    response: StdMutex<Option<Response>>,
    result: StdMutex<Option<crate::transaction::error::Result<()>>>,
    completed: AtomicBool,
    execution_claimed: AtomicBool,
    notify: tokio::sync::Notify,
    final_response: bool,
    supervision_generation: Option<FinalResponseSupervisionGeneration>,
    wire_unknown_transition: Option<TransactionState>,
    _manager_operation: Option<crate::transaction::manager::TransactionManagerAdmissionGuard>,
}

pub(crate) struct SupervisedServerResponseExecution {
    operation: Arc<SupervisedServerResponse>,
}

impl Drop for SupervisedServerResponseExecution {
    fn drop(&mut self) {
        if self.operation.completed.load(Ordering::Acquire) {
            return;
        }
        let Some(generation) = self.operation.supervision_generation else {
            self.operation
                .complete(Err(crate::transaction::error::Error::Other(
                    "supervised server response runner was cancelled".to_string(),
                )));
            return;
        };
        match self
            .operation
            .data
            .final_response_state_for_generation(generation)
        {
            Some(FINAL_RESPONSE_PENDING) => self
                .operation
                .data
                .mark_final_response_failed_before_write_for_generation(generation),
            Some(FINAL_RESPONSE_WRITE_IN_FLIGHT) => self
                .operation
                .data
                .mark_final_response_failed_after_write_boundary_for_generation(generation),
            _ => {}
        }
        self.operation
            .complete(Err(crate::transaction::error::Error::Other(
                "supervised server response runner was cancelled".to_string(),
            )));
    }
}

impl SupervisedServerResponse {
    pub(crate) fn new(
        data: Arc<ServerTransactionData>,
        response: Response,
    ) -> crate::transaction::error::Result<Arc<Self>> {
        let final_response = !response.status().is_provisional();
        let supervision_generation = if final_response {
            Some(data.pending_final_response_generation().ok_or_else(|| {
                crate::transaction::error::Error::Other(
                    "final response has no active supervision generation".to_string(),
                )
            })?)
        } else {
            None
        };
        let wire_unknown_transition = final_response.then(|| {
            if data.request.method() == Method::Invite && response.status().is_success() {
                TransactionState::Terminated
            } else {
                TransactionState::Completed
            }
        });
        // Manager-owned transactions must transfer a second existing-work
        // admission into the runner command before enqueue. If shutdown has
        // already closed that gate, reject before the command can be accepted;
        // an unguarded response is never queued.
        let manager_operation = data.acquire_manager_response_operation()?;
        Ok(Arc::new(Self {
            data,
            response: StdMutex::new(Some(response)),
            result: StdMutex::new(None),
            completed: AtomicBool::new(false),
            execution_claimed: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
            final_response,
            supervision_generation,
            wire_unknown_transition,
            _manager_operation: manager_operation,
        }))
    }

    pub(crate) fn take_response(&self) -> Option<Response> {
        if self.execution_claimed.swap(true, Ordering::AcqRel) {
            return None;
        }
        self.response
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub(crate) fn execution_guard(self: &Arc<Self>) -> SupervisedServerResponseExecution {
        SupervisedServerResponseExecution {
            operation: Arc::clone(self),
        }
    }

    pub(crate) fn complete(&self, result: crate::transaction::error::Result<()>) {
        let mut result_guard = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.completed.load(Ordering::Acquire) {
            return;
        }
        *result_guard = Some(result);
        self.completed.store(true, Ordering::Release);
        drop(result_guard);
        self.notify.notify_waiters();
    }

    pub(crate) fn wire_unknown_transition(&self) -> Option<TransactionState> {
        self.supervision_generation.and_then(|generation| {
            matches!(
                self.data.final_response_state_for_generation(generation),
                Some(FINAL_RESPONSE_WRITE_IN_FLIGHT | FINAL_RESPONSE_FAILED_AFTER_WRITE_BOUNDARY)
            )
            .then_some(self.wire_unknown_transition)
            .flatten()
        })
    }

    pub(crate) async fn wait(&self) -> crate::transaction::error::Result<()> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.completed.load(Ordering::Acquire) {
                return self
                    .result
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .unwrap_or_else(|| {
                        Err(crate::transaction::error::Error::Other(
                            "supervised response completion was already consumed".to_string(),
                        ))
                    });
            }
            notified.await;
        }
    }
}

impl Drop for SupervisedServerResponse {
    fn drop(&mut self) {
        let Some(generation) = self.supervision_generation else {
            return;
        };
        if self.final_response {
            match self.data.final_response_state_for_generation(generation) {
                Some(FINAL_RESPONSE_PENDING) => self
                    .data
                    .mark_final_response_failed_before_write_for_generation(generation),
                Some(FINAL_RESPONSE_WRITE_IN_FLIGHT) => self
                    .data
                    .mark_final_response_failed_after_write_boundary_for_generation(generation),
                _ => {}
            }
        }
    }
}

/// Common behavior trait for all server transactions.
///
/// This trait provides shared functionality that all server transactions need,
/// regardless of whether they are INVITE or non-INVITE transactions. It serves
/// as a base for the more specific transaction type implementations.
pub trait CommonServerTransaction {
    /// Returns the shared transaction data.
    ///
    /// # Returns
    ///
    /// A reference to the `ServerTransactionData` structure containing the transaction's state.
    fn data(&self) -> &Arc<ServerTransactionData>;
}

// Implementation of transaction runner traits for ServerTransactionData

/// Allows access to the transaction state.
/// Required by the transaction runner to manage state transitions.
impl AsRefState for ServerTransactionData {
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState> {
        &self.state
    }
}

/// Allows access to the transaction key.
/// Required by the transaction runner for identification and logging.
impl AsRefKey for ServerTransactionData {
    fn as_ref_key(&self) -> &TransactionKey {
        &self.id
    }
}

/// Provides access to the event channel.
/// Required by the transaction runner to send events to the Transaction User.
impl HasTransactionEvents for ServerTransactionData {
    fn get_tu_event_sender(&self) -> TransactionEventSender {
        self.events_tx.clone()
    }
}

/// Provides access to the transport layer.
/// Required by the transaction runner to send messages.
impl HasTransport for ServerTransactionData {
    fn get_transport_layer(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }
}

/// Provides access to the command channel.
/// Required by the transaction runner to send commands to itself.
impl HasCommandSender for ServerTransactionData {
    fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand> {
        self.cmd_tx.clone()
    }
}

/// Implementation of HasLifecycle trait for ServerTransactionData
impl HasLifecycle for ServerTransactionData {
    /// Get the current lifecycle state
    fn get_lifecycle(&self) -> TransactionLifecycle {
        let val = self.lifecycle.load(std::sync::atomic::Ordering::Acquire);
        match val {
            0 => TransactionLifecycle::Active,
            1 => TransactionLifecycle::Terminating,
            2 => TransactionLifecycle::Draining,
            3 => TransactionLifecycle::Destroyed,
            _ => TransactionLifecycle::Active, // Default fallback
        }
    }

    /// Set the lifecycle state
    fn set_lifecycle(&self, new_lifecycle: TransactionLifecycle) {
        let val = match new_lifecycle {
            TransactionLifecycle::Active => 0,
            TransactionLifecycle::Terminating => 1,
            TransactionLifecycle::Draining => 2,
            TransactionLifecycle::Destroyed => 3,
        };
        self.lifecycle
            .store(val, std::sync::atomic::Ordering::Release);
    }

    /// Check if transaction should emit events to TU (not in Terminating/Draining states)
    fn should_emit_events(&self) -> bool {
        matches!(self.get_lifecycle(), TransactionLifecycle::Active)
    }

    fn lifecycle_scheduler_installed(&self) -> bool {
        self.lifecycle_scheduler.get().is_some()
    }

    async fn schedule_lifecycle(self: Arc<Self>) -> bool {
        let Some(scheduler) = self.lifecycle_scheduler.get().cloned() else {
            return false;
        };
        scheduler.schedule(self).await
    }

    fn termination_cleanup_sender(&self) -> Option<mpsc::Sender<TransactionKey>> {
        self.termination_cleanup_tx.get()?.upgrade()
    }

    fn transaction_admission_owner(
        &self,
    ) -> Option<crate::transaction::manager::TransactionAdmissionOwner> {
        self.transaction_admission_owner()
    }

    fn terminal_event_publication(
        &self,
    ) -> Option<Arc<crate::transaction::event_sender::TerminalEventPublication>> {
        Some(Arc::clone(&self.terminal_event_publication))
    }

    async fn await_protocol_writes(&self) {
        drop(self.last_response.lock().await);
    }
}
