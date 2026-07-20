mod functions;
/// # Transaction Manager for SIP Protocol
///
/// This module implements the TransactionManager, which is the central component
/// of the RFC 3261 SIP transaction layer. It manages all four transaction types
/// defined in the specification:
///
/// - INVITE client transactions (ICT)
/// - Non-INVITE client transactions (NICT)
/// - INVITE server transactions (IST)
/// - Non-INVITE server transactions (NIST)
///
/// ## Transaction Layer Architecture
///
/// In the SIP protocol stack, the transaction layer sits between the transport layer
/// and the Transaction User (TU) layer, fulfilling RFC 3261 Section 17:
///
/// ```text
/// +---------------------------+
/// |  Transaction User (TU)    |  <- Dialogs, call control, etc.
/// |  (UAC, UAS, Proxy)        |
/// +---------------------------+
///              ↑ ↓
///              | |  Transaction events, requests, responses
///              ↓ ↑
/// +---------------------------+
/// |  Transaction Layer        |  <- This module
/// |  (Manager + Transactions) |
/// +---------------------------+
///              ↑ ↓
///              | |  Messages, transport events
///              ↓ ↑
/// +---------------------------+
/// |  Transport Layer          |  <- UDP, TCP, etc.
/// +---------------------------+
/// ```
///
/// ## Transaction Manager Responsibilities
///
/// The TransactionManager's primary responsibilities include:
///
/// 1. **Transaction Creation**: Creating the appropriate transaction type for client or server operations
/// 2. **Message Matching**: Matching incoming messages to existing transactions
/// 3. **State Machine Management**: Managing transaction state transitions
/// 4. **Timer Coordination**: Managing transaction timers (A, B, C, D, E, F, G, H, I, J, K)
/// 5. **Message Retransmission**: Handling reliable delivery over unreliable transports
/// 6. **Event Distribution**: Notifying the TU of transaction events
/// 7. **Special Method Handling**: Processing special methods like ACK and CANCEL
///
/// ## Transaction Matching Rules (RFC 3261 Section 17.1.3 and 17.2.3)
///
/// The manager implements these matching rules to route incoming messages:
///
/// - For client transactions (responses):
///   - Match the branch parameter in the top Via header
///   - Match the sent-by value in the top Via header
///   - Match the method in the CSeq header
///
/// - For server transactions (requests):
///   - Match the branch parameter in the top Via header
///   - Match the sent-by value in the top Via header
///   - Match the request method (except for ACK)
///
/// ## Transaction State Machines
///
/// The TransactionManager orchestrates four distinct state machines defined in RFC 3261:
///
/// ```text
///       INVITE Client Transaction          Non-INVITE Client Transaction
///             (Section 17.1.1)                  (Section 17.1.2)
///
///               |INVITE                           |Request
///               V                                 V
///    +-------+                         +-------+
///    |Calling|-------------+           |Trying |------------+
///    +-------+             |           +-------+            |
///        |                 |               |                |
///        |1xx              |               |1xx             |
///        |                 |               |                |
///        V                 |               V                |
///    +----------+          |           +----------+         |
///    |Proceeding|          |           |Proceeding|         |
///    +----------+          |           +----------+         |
///        |                 |               |                |
///        |200-699          |               |200-699         |
///        |                 |               |                |
///        V                 |               V                |
///    +---------+           |           +---------+          |
///    |Completed|<----------+           |Completed|<---------+
///    +---------+                       +---------+
///        |                                 |
///        |                                 |
///        V                                 V
///    +-----------+                    +-----------+
///    |Terminated |                    |Terminated |
///    +-----------+                    +-----------+
///
///
///       INVITE Server Transaction          Non-INVITE Server Transaction
///             (Section 17.2.1)                  (Section 17.2.2)
///
///               |INVITE                           |Request
///               V                                 V
///    +----------+                        +----------+
///    |Proceeding|--+                     |  Trying  |
///    +----------+  |                     +----------+
///        |         |                         |
///        |1xx      |1xx                      |
///        |         |                         |
///        |         v                         v
///        |      +----------+              +----------+
///        |      |Proceeding|---+          |Proceeding|---+
///        |      +----------+   |          +----------+   |
///        |         |           |              |          |
///        |         |2xx        |              |2xx       |
///        |         |           |              |          |
///        v         v           |              v          |
///    +----------+  |           |          +----------+   |
///    |Completed |<-+-----------+          |Completed |<--+
///    +----------+                         +----------+
///        |                                    |
///        |                                    |
///        V                                    V
///    +-----------+                       +-----------+
///    |Terminated |                       |Terminated |
///    +-----------+                       +-----------+
/// ```
///
/// ## Special Method Handling
///
/// The TransactionManager implements special handling for:
///
/// - **ACK for non-2xx responses**: Automatically generated by the transaction layer (RFC 3261 17.1.1.3)
/// - **ACK for 2xx responses**: Generated by the TU, not by the transaction layer
/// - **CANCEL**: Requires matching to an existing INVITE transaction (RFC 3261 Section 9.1)
/// - **UPDATE**: Follows RFC 3311 rules for in-dialog requests
///
/// ## Event Flow Between Layers
///
/// The TransactionManager facilitates communication between the Transport layer and the TU:
///
/// ```text
///   TU (Transaction User)
///      ↑        ↓
///      |        | - Requests to send
///      |        | - Responses to send
///      |        | - Commands (e.g., CANCEL)
/// Events|        |
///      |        |
///      ↓        ↑
///   Transaction Manager
///      ↑        ↓
///      |        | - Outgoing messages
///      |        |
/// Events|        |
///      |        |
///      ↓        ↑
///   Transport Layer
/// ```
mod handlers;
#[cfg(test)]
mod tests;
mod types;
pub mod utils;

pub use handlers::*;
pub use types::*;
pub use utils::*;

use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use futures::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, trace, warn};

use rvoip_core_traits::identity::AuthenticatedPrincipal;
use rvoip_infra_common::events::cross_crate::SipTransportContext;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};
use rvoip_sip_transport::diagnostics as udp_diagnostics;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    Error as TransportError, Transport, TransportEvent, TransportReceiveTiming, TransportRoute,
};

use crate::diagnostics;
use crate::transaction::client::{
    ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction,
};
use crate::transaction::completion::{
    ClientTransactionCompletion, ClientTransactionCompletionEntry,
    ClientTransactionCompletionHandle, RetainedClientTransactionCompletion,
};
use crate::transaction::error::{Error, Result};
use crate::transaction::event_sender::{
    EventSubscriber, TransactionEventSender, TransactionObserverFanout,
};
use crate::transaction::method::{cancel, update};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::{
    ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::timer::{TimerFactory, TimerManager, TimerSettings};
use crate::transaction::transport::multiplexed::{
    next_hop_uri_for_request, select_transport_for_request, top_route_uri,
};
use crate::transaction::transport::{
    NetworkInfoForSdp, SipTraceRuntime, TransportCapabilities, TransportCapabilitiesExt,
    TransportInfo, WebSocketStatus,
};
use crate::transaction::utils::transaction_key_from_message;
use crate::transaction::{
    InternalTransactionCommand, SipRequestIngressAuthorizer, SipRequestIngressContext, Transaction,
    TransactionEvent, TransactionKey, TransactionKind, TransactionState,
    DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
};

// Type aliases without Sync requirement. `BoxedTransaction` and
// `BoxedServerTransaction` below are retained for downstream APIs
// that still construct boxed handles directly; the manager itself
// stores `Arc<dyn ClientTransaction>` / `Arc<dyn ServerTransaction>`.
#[allow(dead_code)]
type BoxedTransaction = Box<dyn Transaction + Send>;
/// Type alias for a boxed client transaction
/// Per-transaction handle stored in `client_transactions`.
///
/// `Arc<dyn ClientTransaction>` so high-traffic call sites can clone
/// the handle out of the map and drop the outer shard guard before any
/// `.await` — eliminates the hold-across-await pattern that pinned the
/// whole map while one transaction was doing transport I/O. Cheap to
/// clone (atomic refcount bump).
pub(crate) type ArcClientTransaction = Arc<dyn ClientTransaction>;
/// Type alias for an Arc wrapped server transaction. See
/// `BoxedTransaction` above for retention rationale.
#[allow(dead_code)]
type BoxedServerTransaction = Arc<dyn ServerTransaction>;

/// Retained transaction-manager state counts used by release-gate leak checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransactionManagerRetentionCounts {
    pub client_transactions: usize,
    pub server_transactions: usize,
    pub active_transactions_total: usize,
    pub terminated_transactions: usize,
    pub server_invite_dialog_index: usize,
    pub server_invite_dialog_keys_by_tx: usize,
    pub invite_2xx_response_cache: usize,
    pub invite_2xx_response_due_queue: usize,
    pub transaction_destinations: usize,
    pub compact_non_invite_tombstones: usize,
    pub compact_non_invite_deadlines: usize,
    pub event_subscribers: usize,
    pub subscriber_to_transactions: usize,
    pub transaction_to_subscribers: usize,
    pub pending_inbound_bytes: usize,
    pub pending_inbound_transport: usize,
    pub pending_inbound_timing: usize,
    pub pending_inbound_principals: usize,
}

/// Exact client-completion retention diagnostics. Kept separate from the
/// compatibility retention-count struct so downstream exhaustive literals do
/// not break when this authority gains new internal representations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientCompletionRetentionCounts {
    pub active: usize,
    pub retained: usize,
    pub compact: usize,
    pub parsed_responses: usize,
    pub wire_responses: usize,
    /// Serialized response bytes owned by compact completion records.
    pub wire_response_bytes: usize,
    pub deadlines: usize,
}

/// Aggregate-safe diagnostics for compact retired INVITE client state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetiredClientRetentionCounts {
    /// Unexpired retired INVITE route/request records.
    pub transactions: usize,
    /// Total serialized request bytes retained by those records.
    pub request_wire_bytes: usize,
    /// Eager parsed ACK templates retained in steady state. This is always
    /// zero: late-2xx handling reconstructs the template from `request_wire`.
    pub ack_template_allocations: usize,
    /// Exact ordered expiry records.
    pub deadlines: usize,
}

/// Minimal UAC state retained after an INVITE client transaction leaves the
/// active transaction set. RFC 3261 requires the UAC core to accept and ACK
/// additional 2xx responses after the INVITE transaction itself has
/// terminated.
///
/// A parsed [`Request`] owns a tree of headers, parameters, URI components,
/// and strings. Retaining that active representation for the complete late
/// 2xx horizon made every completed INVITE unnecessarily expensive. Keep one
/// immutable wire image and reconstruct parsed compatibility/ACK state only
/// when an uncommon caller observes a late 2xx or asks for `original_request`.
#[allow(dead_code)]
struct LegacyRetiredClientTransactionLayout {
    request_wire: bytes::Bytes,
    route: TransportRoute,
    expires_at: Instant,
    deadline_version: u64,
}

#[derive(Clone)]
pub(crate) struct RetiredClientTransaction {
    request_wire: bytes::Bytes,
    completion: RetainedClientTransactionCompletion,
    route: TransportRoute,
    expires_at: Instant,
    deadline_version: u64,
}

impl RetiredClientTransaction {
    fn new(
        request: &Request,
        completion: &ClientTransactionCompletion,
        route: TransportRoute,
        expires_at: Instant,
        deadline_version: u64,
        admission_owner: Option<TransactionAdmissionOwner>,
    ) -> Self {
        // Serialize while the active request is still available, then drop
        // the parsed header tree with the transaction. `Request::to_bytes`
        // borrows the request and preserves arbitrary binary bodies, avoiding
        // a transient clone of every parsed header and parameter.
        // `Bytes::from(Vec)` takes ownership of the serializer allocation;
        // unlike `Arc<[u8]>::from(Vec)`, it does not require a second
        // allocation and payload copy before retirement can publish.
        let (request_wire, completion) = completion.retained_with_shared_prefix(
            request.to_bytes(),
            expires_at,
            deadline_version,
        );
        let completion = completion.with_admission_owner(admission_owner);
        Self {
            request_wire,
            completion,
            route,
            expires_at,
            deadline_version,
        }
    }

    fn original_request_from_wire(request_wire: &[u8]) -> Result<Request> {
        match parse_message(request_wire)? {
            Message::Request(request) => Ok(request),
            Message::Response(_) => Err(Error::Other(
                "retired client request wire image parsed as a response".into(),
            )),
        }
    }

    #[cfg(test)]
    fn original_request(&self) -> Result<Request> {
        Self::original_request_from_wire(self.request_wire.as_ref())
    }

    #[cfg(test)]
    fn request_wire_len(&self) -> usize {
        self.request_wire.len()
    }

    fn completion_wire_len(&self) -> usize {
        self.completion.diagnostics().wire_response_bytes
    }

    fn has_completion_wire(&self) -> bool {
        self.completion.wire_response().is_some()
    }

    fn shares_wire_allocation(&self) -> bool {
        let Some(response) = self.completion.wire_response() else {
            return true;
        };
        response.as_ptr()
            == self
                .request_wire
                .as_ptr()
                .wrapping_add(self.request_wire.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetiredClientDeadlineEntry {
    transaction_id: Arc<TransactionKey>,
    expires_at: Instant,
    version: u64,
}

/// Exact, bounded deadline index for retained client INVITE tombstones.
///
/// The ordered map makes the next expiry and oldest capacity victim O(log n).
/// The authoritative retired record already stores `(expires_at, version)`,
/// so retaining a second transaction-keyed map would duplicate every key and
/// deadline. Replacement removes the exact old ordered key before publishing
/// the new generation.
#[derive(Debug, Default)]
struct RetiredClientDeadlineScheduler {
    by_deadline: BTreeMap<(Instant, u64), Arc<TransactionKey>>,
    next_version: u64,
}

/// Lean exact deadline index for immutable completion records.
///
/// Unlike retired INVITE routes, a completion generation already stores its
/// own `(expires_at, version)` in the authoritative DashMap value.  A reverse
/// HashMap would therefore duplicate every key and deadline at high CPS.  The
/// ordered index can remove a replaced generation directly by its exact tuple.
#[derive(Debug, Default)]
struct ClientCompletionDeadlineScheduler {
    by_deadline: BTreeMap<(Instant, u64), Arc<TransactionKey>>,
    next_version: u64,
}

impl ClientCompletionDeadlineScheduler {
    fn next_version(&mut self, expires_at: Instant) -> u64 {
        loop {
            let version = self.next_version;
            self.next_version = self.next_version.wrapping_add(1);
            if !self.by_deadline.contains_key(&(expires_at, version)) {
                return version;
            }
        }
    }

    fn schedule(
        &mut self,
        transaction_id: impl Into<Arc<TransactionKey>>,
        expires_at: Instant,
        version: u64,
    ) {
        self.by_deadline
            .insert((expires_at, version), transaction_id.into());
    }

    fn unschedule(
        &mut self,
        transaction_id: &TransactionKey,
        expires_at: Instant,
        version: u64,
    ) -> bool {
        if !self
            .by_deadline
            .get(&(expires_at, version))
            .is_some_and(|scheduled| scheduled.as_ref() == transaction_id)
        {
            return false;
        }
        self.by_deadline.remove(&(expires_at, version));
        true
    }

    fn take_due_and_overflow(
        &mut self,
        now: Instant,
        capacity: usize,
        max_records: usize,
    ) -> Vec<RetiredClientDeadlineEntry> {
        let mut removals = Vec::with_capacity(max_records.min(self.by_deadline.len()));
        while removals.len() < max_records {
            let Some((&(expires_at, version), transaction_id)) = self.by_deadline.first_key_value()
            else {
                break;
            };
            if expires_at > now && self.by_deadline.len() <= capacity {
                break;
            }
            let transaction_id = Arc::clone(transaction_id);
            self.by_deadline.pop_first();
            removals.push(RetiredClientDeadlineEntry {
                transaction_id,
                expires_at,
                version,
            });
        }
        removals
    }

    fn next_wake_at(&self, now: Instant, capacity: usize) -> Option<Instant> {
        if self.by_deadline.len() > capacity {
            return Some(now);
        }
        self.by_deadline
            .first_key_value()
            .map(|(&(expires_at, _), _)| expires_at)
    }

    fn has_due_or_overflow(&self, now: Instant, capacity: usize) -> bool {
        self.next_wake_at(now, capacity)
            .is_some_and(|deadline| deadline <= now)
    }

    fn len(&self) -> usize {
        self.by_deadline.len()
    }

    fn clear(&mut self) {
        self.by_deadline.clear();
    }
}

impl RetiredClientDeadlineScheduler {
    fn next_version(&mut self, expires_at: Instant) -> u64 {
        loop {
            let version = self.next_version;
            self.next_version = self.next_version.wrapping_add(1);
            if !self.by_deadline.contains_key(&(expires_at, version)) {
                return version;
            }
        }
    }

    fn schedule(
        &mut self,
        transaction_id: impl Into<Arc<TransactionKey>>,
        expires_at: Instant,
        version: u64,
    ) {
        self.by_deadline
            .insert((expires_at, version), transaction_id.into());
    }

    fn unschedule(
        &mut self,
        transaction_id: &TransactionKey,
        expires_at: Instant,
        version: u64,
    ) -> bool {
        if !self
            .by_deadline
            .get(&(expires_at, version))
            .is_some_and(|scheduled| scheduled.as_ref() == transaction_id)
        {
            return false;
        }
        self.by_deadline.remove(&(expires_at, version));
        true
    }

    fn take_due_and_overflow(
        &mut self,
        now: Instant,
        capacity: usize,
        max_records: usize,
    ) -> Vec<RetiredClientDeadlineEntry> {
        let mut removals = Vec::with_capacity(max_records.min(self.by_deadline.len()));
        while removals.len() < max_records {
            let Some((&(expires_at, version), transaction_id)) = self.by_deadline.first_key_value()
            else {
                break;
            };
            if expires_at > now && self.by_deadline.len() <= capacity {
                break;
            }

            let transaction_id = Arc::clone(transaction_id);
            self.by_deadline.pop_first();
            removals.push(RetiredClientDeadlineEntry {
                transaction_id,
                expires_at,
                version,
            });
        }
        removals
    }

    fn next_wake_at(&self, now: Instant, capacity: usize) -> Option<Instant> {
        if self.by_deadline.len() > capacity {
            return Some(now);
        }
        self.by_deadline
            .first_key_value()
            .map(|(&(expires_at, _), _)| expires_at)
    }

    fn has_due_or_overflow(&self, now: Instant, capacity: usize) -> bool {
        self.next_wake_at(now, capacity)
            .is_some_and(|deadline| deadline <= now)
    }

    fn len(&self) -> usize {
        self.by_deadline.len()
    }

    fn clear(&mut self) {
        self.by_deadline.clear();
    }
}

/// Retained INVITE routes and exact completion cells share one wakeable,
/// manager-owned deadline worker. A synchronized high-CPS retention horizon
/// must never turn into one unbounded maintenance pass, so each turn removes
/// at most this many records from each compact deadline index before yielding.
const RETAINED_CLIENT_DEADLINE_BATCH_MAX: usize = 1_024;

#[derive(Default)]
struct RetainedClientDeadlineWorkerCounters {
    wakeups: AtomicU64,
    batches: AtomicU64,
    records: AtomicU64,
    yields: AtomicU64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RetainedClientDeadlineWorkerSnapshot {
    wakeups: u64,
    batches: u64,
    records: u64,
    yields: u64,
}

struct RetainedClientDeadlineWorkerInner {
    wake: Arc<tokio::sync::Notify>,
    stopping: Arc<AtomicBool>,
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    counters: Arc<RetainedClientDeadlineWorkerCounters>,
}

impl Drop for RetainedClientDeadlineWorkerInner {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_one();
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct RetainedClientDeadlineWorker {
    inner: Arc<RetainedClientDeadlineWorkerInner>,
}

impl RetainedClientDeadlineWorker {
    fn new() -> Self {
        Self {
            inner: Arc::new(RetainedClientDeadlineWorkerInner {
                wake: Arc::new(tokio::sync::Notify::new()),
                stopping: Arc::new(AtomicBool::new(false)),
                task: std::sync::Mutex::new(None),
                counters: Arc::new(RetainedClientDeadlineWorkerCounters::default()),
            }),
        }
    }

    fn start(&self, context: RetainedClientDeadlineContext) {
        let mut task = self
            .inner
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if task.is_some() {
            return;
        }
        let wake = Arc::clone(&self.inner.wake);
        let stopping = Arc::clone(&self.inner.stopping);
        let counters = Arc::clone(&self.inner.counters);
        *task = Some(tokio::spawn(run_retained_client_deadline_worker(
            context, wake, stopping, counters,
        )));
    }

    fn wake(&self) {
        self.inner.counters.wakeups.fetch_add(1, Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    async fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::Release);
        self.inner.wake.notify_one();
        let task = self
            .inner
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> RetainedClientDeadlineWorkerSnapshot {
        RetainedClientDeadlineWorkerSnapshot {
            wakeups: self.inner.counters.wakeups.load(Ordering::Relaxed),
            batches: self.inner.counters.batches.load(Ordering::Relaxed),
            records: self.inner.counters.records.load(Ordering::Relaxed),
            yields: self.inner.counters.yields.load(Ordering::Relaxed),
        }
    }
}

struct RetainedClientDeadlineContext {
    client_completions:
        std::sync::Weak<DashMap<Arc<TransactionKey>, ClientTransactionCompletionEntry>>,
    client_completion_deadlines:
        std::sync::Weak<std::sync::Mutex<ClientCompletionDeadlineScheduler>>,
    client_completion_capacity: usize,
    transaction_destinations:
        std::sync::Weak<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
    retired_client_deadlines: std::sync::Weak<std::sync::Mutex<RetiredClientDeadlineScheduler>>,
    retired_client_transaction_capacity: Arc<AtomicUsize>,
    retired_client_transaction_count: std::sync::Weak<AtomicUsize>,
}

struct RetainedClientDeadlineBatch {
    processed: usize,
    more_due: bool,
    next_wake_at: Option<Instant>,
    context_alive: bool,
}

fn process_retained_client_deadline_batch(
    context: &RetainedClientDeadlineContext,
    now: Instant,
) -> RetainedClientDeadlineBatch {
    let (
        Some(client_completions),
        Some(client_completion_deadlines),
        Some(transaction_destinations),
        Some(retired_client_deadlines),
        Some(retired_client_transaction_count),
    ) = (
        context.client_completions.upgrade(),
        context.client_completion_deadlines.upgrade(),
        context.transaction_destinations.upgrade(),
        context.retired_client_deadlines.upgrade(),
        context.retired_client_transaction_count.upgrade(),
    )
    else {
        return RetainedClientDeadlineBatch {
            processed: 0,
            more_due: false,
            next_wake_at: None,
            context_alive: false,
        };
    };

    let (completion_removals, completion_more_due, completion_next) = {
        let mut deadlines = client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let removals = deadlines.take_due_and_overflow(
            now,
            context.client_completion_capacity,
            RETAINED_CLIENT_DEADLINE_BATCH_MAX,
        );
        let more_due = deadlines.has_due_or_overflow(now, context.client_completion_capacity);
        let next = deadlines.next_wake_at(now, context.client_completion_capacity);
        (removals, more_due, next)
    };
    let completion_processed = completion_removals.len();
    for deadline in completion_removals {
        client_completions.remove_if(deadline.transaction_id.as_ref(), |_, completion| {
            completion.retained_deadline() == Some((deadline.expires_at, deadline.version))
        });
    }

    let retired_client_transaction_capacity = context
        .retired_client_transaction_capacity
        .load(Ordering::Acquire);
    let (retired_removals, retired_more_due, retired_next) = {
        let mut deadlines = retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let removals = deadlines.take_due_and_overflow(
            now,
            retired_client_transaction_capacity,
            RETAINED_CLIENT_DEADLINE_BATCH_MAX,
        );
        let more_due = deadlines.has_due_or_overflow(now, retired_client_transaction_capacity);
        let next = deadlines.next_wake_at(now, retired_client_transaction_capacity);
        (removals, more_due, next)
    };
    let retired_processed = retired_removals.len();
    for deadline in retired_removals {
        if transaction_destinations
            .remove_if(deadline.transaction_id.as_ref(), |_, state| {
                state.retired().is_some_and(|retired| {
                    retired.deadline_version == deadline.version
                        && retired.expires_at == deadline.expires_at
                })
            })
            .is_some()
        {
            let _ = retired_client_transaction_count.fetch_update(
                Ordering::AcqRel,
                Ordering::Acquire,
                |count| Some(count.saturating_sub(1)),
            );
        }
    }

    RetainedClientDeadlineBatch {
        processed: completion_processed + retired_processed,
        more_due: completion_more_due || retired_more_due,
        next_wake_at: completion_next.into_iter().chain(retired_next).min(),
        context_alive: true,
    }
}

async fn run_retained_client_deadline_worker(
    context: RetainedClientDeadlineContext,
    wake: Arc<tokio::sync::Notify>,
    stopping: Arc<AtomicBool>,
    counters: Arc<RetainedClientDeadlineWorkerCounters>,
) {
    loop {
        if stopping.load(Ordering::Acquire) {
            break;
        }

        let now = Instant::now();
        let batch = process_retained_client_deadline_batch(&context, now);
        if !batch.context_alive {
            break;
        }
        if batch.processed != 0 {
            counters.batches.fetch_add(1, Ordering::Relaxed);
            counters
                .records
                .fetch_add(batch.processed as u64, Ordering::Relaxed);
        }
        if batch.more_due {
            counters.yields.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
            continue;
        }

        // Notify stores a permit, so a deadline scheduled between computing
        // `next_wake_at` and polling this future cannot be lost.
        let notified = wake.notified();
        if stopping.load(Ordering::Acquire) {
            break;
        }
        match batch.next_wake_at {
            Some(deadline) if deadline <= Instant::now() => {
                tokio::task::yield_now().await;
            }
            Some(deadline) => {
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep_until(deadline.into()) => {}
                }
            }
            None => notified.await,
        }
    }
}

/// One linearizable response-route record for the complete lifetime of a
/// client transaction. Keeping active and retained routes in the same DashMap
/// entry prevents cleanup from exposing a transient "unknown transaction"
/// window between removing the live route and publishing its tombstone.
pub(crate) enum ClientResponseRouteState {
    Active {
        route: TransportRoute,
        /// Allocation identity of the client transaction data that installed
        /// this route. Compact Timer K cleanup retains this word-sized proof
        /// rather than a second complete `TransportRoute`.
        owner: usize,
    },
    Retired(RetiredClientTransaction),
}

impl ClientResponseRouteState {
    pub(crate) fn active(route: TransportRoute, owner: usize) -> Self {
        Self::Active { route, owner }
    }

    fn route(&self) -> &TransportRoute {
        match self {
            Self::Active { route, .. } => route,
            Self::Retired(retired) => &retired.route,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    fn retired(&self) -> Option<&RetiredClientTransaction> {
        match self {
            Self::Active { .. } => None,
            Self::Retired(retired) => Some(retired),
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct RetiredClientTransitionTestGate {
    transaction_id: TransactionKey,
    transitioned: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
static RETIRED_CLIENT_TRANSITION_TEST_GATE: std::sync::LazyLock<
    std::sync::Mutex<Option<RetiredClientTransitionTestGate>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
#[derive(Clone)]
struct TerminationTakeoverTestGate {
    transaction_id: TransactionKey,
    runner_joined: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
static TERMINATION_TAKEOVER_TEST_GATE: std::sync::LazyLock<
    std::sync::Mutex<Option<TerminationTakeoverTestGate>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[derive(Clone)]
pub(crate) struct InboundPrincipalBinding {
    principal: AuthenticatedPrincipal,
    source: SocketAddr,
    transport_type: rvoip_sip_transport::transport::TransportType,
    flow_id: Option<rvoip_sip_transport::TransportFlowId>,
    tls_leaf_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InboundPrincipalLease {
    pub(crate) inserted_at: Instant,
    pub(crate) generation: u64,
}

impl InboundPrincipalBinding {
    fn new(principal: AuthenticatedPrincipal, context: &SipRequestIngressContext) -> Self {
        Self {
            principal,
            source: context.source,
            transport_type: context.transport_type,
            flow_id: context.flow_id,
            tls_leaf_sha256: context.connection_metadata.as_ref().map(|metadata| {
                metadata
                    .tls_peer_identity
                    .leaf_certificate_sha256
                    .to_ascii_lowercase()
            }),
        }
    }

    fn matches(&self, context: &SipRequestIngressContext) -> bool {
        self.source == context.source
            && self.transport_type == context.transport_type
            && self.flow_id == context.flow_id
            && self.tls_leaf_sha256
                == context.connection_metadata.as_ref().map(|metadata| {
                    metadata
                        .tls_peer_identity
                        .leaf_certificate_sha256
                        .to_ascii_lowercase()
                })
    }
}

const MIN_TRANSACTION_INDEX_CAPACITY: usize = 1024;
/// Maximum number of entries eagerly reserved in each transaction index.
///
/// The configured index capacity is a logical admission/retention limit and
/// can be large (128,000 in the canonical PBX profile). Reserving that full
/// amount independently in every DashMap multiplies idle memory without
/// changing the maps' ability to grow. Keep a modest hot working set and let
/// each sharded table expand on demand.
const MAX_EAGER_TRANSACTION_INDEX_CAPACITY: usize = 4096;
const DEFAULT_TRANSACTION_DISPATCH_WORKERS: usize = 1;
pub const MAX_TRANSACTION_DISPATCH_WORKERS: usize = 64;
// Keep successful INVITE responses available long enough for high-load UAC
// retransmission windows. Entries are removed as soon as the 2xx ACK arrives,
// so this is a tail bound for lossy/missing-ACK calls, not a full call-volume
// retention period.
const INVITE_2XX_RESPONSE_CACHE_TTL: Duration = Duration::from_secs(90);
const RETIRED_CLIENT_TRANSACTION_TTL: Duration = Duration::from_secs(90);
const CLIENT_TRANSACTION_COMPLETION_TTL: Duration = Duration::from_secs(90);
// Reliable non-INVITE transactions have a zero-second RFC Timer K, but the
// async public API still needs a short scheduling grace between `send_request`
// returning and the caller registering its exact waiter.
const RELIABLE_CLIENT_COMPLETION_RACE_GRACE: Duration = Duration::from_secs(1);
const INVITE_2XX_ACKED_RESPONSE_RETENTION: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const BYE_FINAL_RESPONSE_RECOVERY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const BYE_FINAL_RESPONSE_RECOVERY_TIMEOUT: Duration = Duration::from_millis(50);
const PENDING_INBOUND_BYTES_TTL: Duration = Duration::from_secs(30);
const PENDING_INBOUND_PRINCIPAL_TTL: Duration = Duration::from_secs(90);
/// Default bounded number of due INVITE 2xx retransmissions processed per
/// maintenance tick.
pub const DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK: usize = 2048;
const MIN_INVITE_2XX_RESPONSE_CACHE_CAPACITY: usize = 65_536;
const MIN_RETIRED_CLIENT_TRANSACTION_CAPACITY: usize = 65_536;
const TERMINATED_CLEANUP_BATCH_MAX: usize = 1024;
const EXPLICIT_TERMINATION_CONCURRENCY_MAX: usize = 64;
/// Maximum ACK-index deadlines consumed by one maintenance pass. Counting
/// stale generations toward the budget keeps cleanup work bounded even when a
/// dialog key is replaced repeatedly before its old retention windows expire.
const SERVER_INVITE_ACK_EXPIRY_BATCH_MAX: usize = 2048;
/// Default maximum consecutive ACK/BYE priority-lane events before a ready
/// normal transaction event receives a turn.
pub const DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX: usize = 64;

fn transaction_index_capacity(capacity: Option<usize>) -> usize {
    capacity.unwrap_or(100).max(MIN_TRANSACTION_INDEX_CAPACITY)
}

fn transaction_index_initial_capacity(logical_capacity: usize) -> usize {
    logical_capacity.min(MAX_EAGER_TRANSACTION_INDEX_CAPACITY)
}

fn invite_2xx_response_cache_capacity(index_capacity: usize) -> usize {
    index_capacity.max(MIN_INVITE_2XX_RESPONSE_CACHE_CAPACITY)
}

fn retired_client_transaction_capacity(index_capacity: usize) -> usize {
    index_capacity.max(MIN_RETIRED_CLIENT_TRANSACTION_CAPACITY)
}

fn transaction_dispatch_worker_count(workers: Option<usize>) -> usize {
    workers
        .unwrap_or(DEFAULT_TRANSACTION_DISPATCH_WORKERS)
        .clamp(1, MAX_TRANSACTION_DISPATCH_WORKERS)
}

fn transaction_dispatch_queue_capacity(capacity: Option<usize>, default_capacity: usize) -> usize {
    capacity.unwrap_or(default_capacity).max(1)
}

fn transport_token_for_request(request: &Request) -> &'static str {
    match select_transport_for_request(request) {
        TransportType::Udp => "UDP",
        TransportType::Tcp => "TCP",
        TransportType::Tls => "TLS",
        TransportType::Ws => "WS",
        TransportType::Wss => "WSS",
    }
}

fn is_rport_param(param: &Param) -> bool {
    match param {
        Param::Rport(_) => true,
        Param::Other(name, _) => name.eq_ignore_ascii_case("rport"),
        _ => false,
    }
}

fn normalize_top_client_via(request: &mut Request, branch: &str) -> bool {
    for header in &mut request.headers {
        if let TypedHeader::Via(via) = header {
            let Some(top_via) = via.0.first_mut() else {
                return false;
            };

            top_via
                .params
                .retain(|param| !matches!(param, Param::Branch(_)));
            top_via.params.push(Param::branch(branch.to_string()));

            if !top_via.params.iter().any(is_rport_param) {
                top_via.params.push(Param::Rport(None));
            }

            return true;
        }
    }

    false
}

fn sip_diagnostics_enabled() -> bool {
    diagnostics::enabled()
}

/// Defines the public API for the RFC 3261 SIP Transaction Manager.
///
/// The TransactionManager coordinates all SIP transaction activities, including
/// creation, processing of messages, and event delivery.
///
/// It implements the four core transaction types defined in RFC 3261:
/// - INVITE client transactions (ICT)
/// - Non-INVITE client transactions (NICT)
/// - INVITE server transactions (IST)
/// - Non-INVITE server transactions (NIST)
///
/// Special methods (CANCEL, ACK for 2xx, UPDATE) are handled through utility
/// functions that work with these four core transaction types.
#[derive(Clone, Copy)]
enum TransactionEventChannelMode {
    Owned,
    Shared,
}

enum TransactionManagerEventReceiver {
    Owned(mpsc::Receiver<TransactionEvent>),
    Shared(mpsc::Receiver<Arc<TransactionEvent>>),
}

struct TransactionAdmissionRegistry {
    entries: DashMap<TransactionKey, u64>,
    next_generation: AtomicU64,
    /// Exact-key cleanup invoked while the retiring admission generation still
    /// owns the wire key. The dialog layer uses this as a backstop when its
    /// authoritative terminal event cannot be observed (for example, because
    /// the primary event receiver closed).
    final_release_hook:
        std::sync::RwLock<Option<Arc<dyn Fn(&TransactionKey) + Send + Sync + 'static>>>,
}

const MANAGER_ADMISSION_RUNNING: u8 = 0;
const MANAGER_ADMISSION_DRAINING: u8 = 1;
const MANAGER_ADMISSION_STOPPING: u8 = 2;
const MANAGER_ADMISSION_STOPPED: u8 = 3;

pub(crate) struct TransactionManagerAdmissionLifecycle {
    state: AtomicU8,
    in_flight: AtomicUsize,
    idle: tokio::sync::Notify,
}

struct TransactionManagerOperationCancellation {
    cancelled: AtomicBool,
    notify: tokio::sync::Notify,
}

impl TransactionManagerOperationCancellation {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

impl TransactionManagerAdmissionLifecycle {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(MANAGER_ADMISSION_RUNNING),
            in_flight: AtomicUsize::new(0),
            idle: tokio::sync::Notify::new(),
        })
    }

    fn try_enter(self: &Arc<Self>) -> Option<TransactionManagerAdmissionGuard> {
        if self.state.load(Ordering::Acquire) != MANAGER_ADMISSION_RUNNING {
            return None;
        }
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        if self.state.load(Ordering::Acquire) != MANAGER_ADMISSION_RUNNING {
            if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            return None;
        }
        Some(TransactionManagerAdmissionGuard {
            lifecycle: Arc::clone(self),
        })
    }

    /// Track cleanup of already-admitted work while fail-closed draining has
    /// rejected all new transaction creation. Shutdown advances to Stopping
    /// before its final idle wait, closing this second gate as well.
    pub(crate) fn try_enter_existing(self: &Arc<Self>) -> Option<TransactionManagerAdmissionGuard> {
        if self.state.load(Ordering::Acquire) > MANAGER_ADMISSION_DRAINING {
            return None;
        }
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        if self.state.load(Ordering::Acquire) > MANAGER_ADMISSION_DRAINING {
            if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            return None;
        }
        Some(TransactionManagerAdmissionGuard {
            lifecycle: Arc::clone(self),
        })
    }

    fn begin_draining(&self) -> bool {
        self.state
            .compare_exchange(
                MANAGER_ADMISSION_RUNNING,
                MANAGER_ADMISSION_DRAINING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn begin_stopping(&self) {
        loop {
            let state = self.state.load(Ordering::Acquire);
            if state >= MANAGER_ADMISSION_STOPPING {
                return;
            }
            if self
                .state
                .compare_exchange(
                    state,
                    MANAGER_ADMISSION_STOPPING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.idle.notify_waiters();
                return;
            }
        }
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.in_flight.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }

    fn mark_stopped(&self) {
        self.state
            .store(MANAGER_ADMISSION_STOPPED, Ordering::Release);
        self.idle.notify_waiters();
    }

    fn state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }
}

pub(crate) struct TransactionManagerAdmissionGuard {
    lifecycle: Arc<TransactionManagerAdmissionLifecycle>,
}

impl Drop for TransactionManagerAdmissionGuard {
    fn drop(&mut self) {
        if self.lifecycle.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.lifecycle.idle.notify_waiters();
        }
    }
}

impl TransactionAdmissionRegistry {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: DashMap::new(),
            next_generation: AtomicU64::new(1),
            final_release_hook: std::sync::RwLock::new(None),
        })
    }

    fn install_final_release_hook(
        &self,
        hook: Arc<dyn Fn(&TransactionKey) + Send + Sync + 'static>,
    ) {
        *self
            .final_release_hook
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
    }

    fn try_claim(self: &Arc<Self>, key: &TransactionKey) -> Option<TransactionAdmissionOwner> {
        use dashmap::mapref::entry::Entry;

        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        match self.entries.entry(key.clone()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(generation);
                Some(TransactionAdmissionOwner {
                    _inner: Arc::new(TransactionAdmissionOwnerInner {
                        registry: Arc::clone(self),
                        key: key.clone(),
                        generation,
                    }),
                })
            }
        }
    }
}

struct TransactionAdmissionOwnerInner {
    registry: Arc<TransactionAdmissionRegistry>,
    key: TransactionKey,
    generation: u64,
}

impl Drop for TransactionAdmissionOwnerInner {
    fn drop(&mut self) {
        use dashmap::mapref::entry::Entry;

        // Keep the registry entry occupied through the callback. A replacement
        // generation therefore cannot publish a same-key dialog route between
        // cleanup and admission release (the ABA case this fence prevents).
        if let Entry::Occupied(entry) = self.registry.entries.entry(self.key.clone()) {
            if *entry.get() != self.generation {
                return;
            }
            let hook = self
                .registry
                .final_release_hook
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(hook) = hook {
                hook(&self.key);
            }
            entry.remove();
        }
    }
}

/// Exact, cloneable ownership fence spanning transaction construction,
/// active maps, compact Timer J/K state, terminal event delivery, and dialog
/// acknowledgement. The registry itself stores only a compact generation;
/// dropping the final owner exact-removes that generation.
#[doc(hidden)]
#[derive(Clone)]
pub struct TransactionAdmissionOwner {
    _inner: Arc<TransactionAdmissionOwnerInner>,
}

impl TransactionAdmissionOwner {
    pub(crate) fn generation(&self) -> u64 {
        self._inner.generation
    }
}

impl fmt::Debug for TransactionAdmissionOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransactionAdmissionOwner")
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
enum TransactionSubscriptionAuthority {
    Client(ArcClientTransaction),
    Server(Arc<dyn ServerTransaction>),
    Compact(u64),
}

impl PartialEq for TransactionSubscriptionAuthority {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Client(left), Self::Client(right)) => Arc::ptr_eq(left, right),
            (Self::Server(left), Self::Server(right)) => Arc::ptr_eq(left, right),
            (Self::Compact(left), Self::Compact(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for TransactionSubscriptionAuthority {}

#[derive(Clone)]
pub struct TransactionManager {
    /// Transport to use for messages
    transport: Arc<dyn Transport>,
    /// Active client transactions. DashMap for sharded lock-free reads
    /// across cores; per-transaction state is owned by the
    /// `Arc<dyn ClientTransaction>` itself (its impls hold their own
    /// internal `Arc<Mutex<...>>` over data + timers + state machine).
    /// Hot-path call sites clone the Arc out, drop the shard guard,
    /// then await — no map-wide serialization on transport I/O.
    client_transactions: Arc<DashMap<TransactionKey, ArcClientTransaction>>,
    /// Atomic per-wire-key admission fence. Unlike the representation-specific
    /// active/tombstone maps, this owner spans every lifecycle representation.
    transaction_admissions: Arc<TransactionAdmissionRegistry>,
    /// Independent of the transport receive-loop `running` flag: outbound-only
    /// managers also start Running and must reject publication after drain.
    admission_lifecycle: Arc<TransactionManagerAdmissionLifecycle>,
    operation_cancellation: Arc<TransactionManagerOperationCancellation>,
    shutdown_gate: Arc<Mutex<()>>,
    /// Configured logical capacity used by admission and retained-state
    /// bounds. This remains independent from the initial DashMap reserve.
    transaction_index_logical_capacity: usize,
    /// Per-map initial reserve. DashMaps grow past this value on demand up to
    /// the logical limits enforced by their owning protocol paths.
    transaction_index_initial_capacity: usize,
    /// Exact state/final-response cells for client transactions. Entries are
    /// retained briefly after transaction removal so response-before-wait and
    /// response-during-cleanup races remain observable without subscriptions.
    client_completions: Arc<DashMap<Arc<TransactionKey>, ClientTransactionCompletionEntry>>,
    client_completion_deadlines: Arc<std::sync::Mutex<ClientCompletionDeadlineScheduler>>,
    client_completion_capacity: usize,
    /// One wakeable deadline owner for retained client completions and INVITE
    /// route tombstones. It replaces one-second polling and unbounded expiry
    /// sweeps with bounded, immediately re-queued batches.
    retained_client_deadline_worker: Option<RetainedClientDeadlineWorker>,
    /// Active server transactions. Same pattern as `client_transactions`.
    server_transactions: Arc<DashMap<TransactionKey, Arc<dyn ServerTransaction>>>,
    /// Indexed queue of transactions that reached `Terminated`.
    /// Runtime cleanup drains this bounded index instead of scanning either
    /// active transaction table. The public full-scan repair API is reserved
    /// for explicit diagnostics and is never scheduled automatically.
    terminated_transactions: Arc<DashMap<TransactionKey, ()>>,
    /// Fast dialog-id lookup for 2xx ACKs targeting INVITE server transactions.
    /// Entries remain briefly after transaction removal so end-to-end ACKs
    /// never need to scan active transactions.
    server_invite_dialog_index: Arc<DashMap<ServerInviteDialogKey, ServerInviteAckIndexEntry>>,
    /// Reverse index used to retire only the ACK keys belonging to a
    /// transaction, avoiding full-map scans during transaction cleanup.
    server_invite_dialog_keys_by_tx: Arc<DashMap<TransactionKey, Vec<ServerInviteDialogKey>>>,
    /// Due-driven retirement queue for server INVITE ACK bindings. The heap is
    /// allocated lazily and contains compact keys rather than another copy of
    /// the server transaction.
    server_invite_dialog_expiry_queue:
        Arc<std::sync::Mutex<BinaryHeap<ServerInviteAckExpiryEntry>>>,
    server_invite_dialog_deadline_generation: Arc<AtomicU64>,
    /// Cache of successful INVITE responses for retransmitted INVITEs after
    /// the INVITE server transaction has entered the RFC 3261 2xx path.
    invite_2xx_response_cache: Arc<DashMap<TransactionKey, Invite2xxResponseCacheEntry>>,
    invite_2xx_response_cache_capacity: usize,
    /// Exact, deadline-ordered maintenance index. It contains one compact
    /// record per cached response and removes superseded deadlines eagerly.
    invite_2xx_response_due_queue: Arc<std::sync::Mutex<Invite2xxDeadlineScheduler>>,
    /// Exact explicit-termination operations admitted before the shared
    /// cleanup key is queued. Keeping these private to the manager preserves
    /// the public `HasLifecycle` cleanup-sender contract while allowing the
    /// one cleanup worker to own cancellation-safe API work.
    explicit_termination_operations:
        Arc<DashMap<TransactionKey, Vec<Arc<functions::ExplicitTerminationOperation>>>>,
    terminated_cleanup_tx: Option<mpsc::Sender<TransactionKey>>,
    /// Explicit stop signal for the cleanup worker. The worker clears its
    /// cloned sender fields before spawning, so it never keeps its own input
    /// channel (or the lifecycle scheduler) alive.
    terminated_cleanup_shutdown: Option<Arc<tokio::sync::Notify>>,
    /// One due-driven lifecycle worker per manager. Transactions receive a
    /// clone of this handle when they are admitted; no process-global lookup
    /// or lock is involved on the terminal hot path.
    lifecycle_scheduler: Option<crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle>,
    /// Compact RFC Timer J/K state for completed UDP non-INVITE
    /// transactions. Full runners leave the active maps as soon as this
    /// tombstone is installed.
    compact_non_invite_tombstones:
        Arc<crate::transaction::lifecycle_scheduler::CompactNonInviteTombstones>,
    /// Client response routes transition atomically from active to a bounded,
    /// expiring INVITE tombstone. Retired entries authenticate late forked or
    /// retransmitted 2xx responses and retain the request template needed to
    /// ACK them without keeping the transaction runner alive.
    transaction_destinations: Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
    retired_client_transaction_capacity: Arc<AtomicUsize>,
    retired_client_transaction_count: Arc<AtomicUsize>,
    retired_client_deadlines: Arc<std::sync::Mutex<RetiredClientDeadlineScheduler>>,
    /// Event sender
    events_tx: TransactionEventSender,
    /// Global observational subscribers. ArcSwap so the broadcast hot path
    /// (every transaction state change, every retransmit) reads via a
    /// single atomic load instead of acquiring an async mutex. Writes
    /// (subscribe/unsubscribe) use copy-on-write RCU.
    event_subscribers: Arc<ArcSwap<Vec<EventSubscriber>>>,
    /// Maps subscribers to transactions they're interested in.
    /// DashMap — guards never held across `.await`.
    subscriber_to_transactions: Arc<DashMap<usize, Vec<TransactionKey>>>,
    /// Direct transaction-keyed observational subscribers. Keyed event
    /// delivery reads only this transaction's small vector; it never scans the
    /// global subscriber population.
    transaction_to_subscribers: Arc<DashMap<TransactionKey, Vec<EventSubscriber>>>,
    /// Subscriber counter for assigning unique IDs. `AtomicUsize` —
    /// the previous `Mutex<usize>` only ever did fetch-and-increment.
    next_subscriber_id: Arc<AtomicUsize>,
    /// Transport message channel
    transport_rx: Option<Arc<Mutex<mpsc::Receiver<TransportEvent>>>>,
    /// Separately reserved lifecycle/control channel.
    control_transport_rx: Option<Arc<Mutex<mpsc::Receiver<TransportEvent>>>>,
    /// Running flag. `AtomicBool` — the previous `Mutex<bool>` was
    /// locked per-iteration of the message loop.
    running: Arc<AtomicBool>,
    /// Timer configuration
    timer_settings: TimerSettings,
    /// Centralized timer manager
    timer_manager: Arc<TimerManager>,
    /// Timer factory. Held by the manager so future per-transaction
    /// timer creation calls land on the same factory instance; today
    /// the timer dispatch routes through `timer_manager` directly.
    #[allow(dead_code)]
    timer_factory: TimerFactory,
    /// Optional forwarder for transport-side events that dialog-core's
    /// RFC 5626 outbound-flow monitor needs to observe
    /// (`KeepAlivePongReceived`, `ConnectionClosed`). Installed by
    /// `DialogManager::with_global_events` at boot; `None` keeps the
    /// transaction manager transport-agnostic when no dialog layer is
    /// wired up (bare transaction tests, examples).
    flow_event_sender: Arc<
        tokio::sync::RwLock<
            Option<mpsc::Sender<crate::manager::outbound_flow::FlowTransportEvent>>,
        >,
    >,
    /// Optional SIP trace publisher for inbound transport-boundary events.
    sip_trace: Option<Arc<SipTraceRuntime>>,
    /// Cache of original wire bytes for inbound messages, keyed by
    /// transaction key. Populated by `handle_transport_event` when the
    /// transport supplies `TransportEvent::MessageReceived.raw_bytes`,
    /// consumed by cross-crate event bridges (`IncomingCall`,
    /// `IncomingRegister`, `CallEstablished`, etc.) so STIR/SHAKEN
    /// signatures and other byte-exact consumers see the upstream
    /// form without an intermediate `Message::to_bytes()` round-trip.
    /// See `SIP_API_DESIGN_2.md` §7.5.
    pub(crate) pending_inbound_bytes: Arc<DashMap<TransactionKey, bytes::Bytes>>,
    pending_inbound_inserted_at: Arc<DashMap<TransactionKey, Instant>>,
    pub(crate) pending_inbound_transport: Arc<DashMap<TransactionKey, SipTransportContext>>,
    /// Optional receive timing diagnostics keyed by transaction. Populated only
    /// when transport diagnostics are enabled and consumed by dialog-core
    /// instrumentation when it emits higher-level events or BYE responses.
    pub(crate) pending_inbound_timing: Arc<DashMap<TransactionKey, TransportReceiveTiming>>,
    /// Optional listener policy evaluated for every new request transaction
    /// before the transaction user sees it. `None` preserves the historical
    /// unauthenticated-listener behavior.
    request_ingress_authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
    /// Successful ingress identities awaiting dialog/session consumption.
    pending_inbound_principals: Arc<DashMap<TransactionKey, InboundPrincipalBinding>>,
    pending_inbound_principal_inserted_at: Arc<DashMap<TransactionKey, InboundPrincipalLease>>,
    pending_inbound_principal_generation: Arc<AtomicU64>,
    transaction_dispatch_workers: usize,
    transaction_dispatch_queue_capacity: usize,
    transaction_command_channel_capacity: usize,
    transaction_dispatch_priority_burst_max: Arc<AtomicUsize>,
    invite_2xx_retransmit_max_due_per_tick: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionIngressKind {
    Control,
    Invite,
    Ack,
    Bye,
    Cancel,
    Other,
}

impl TransactionIngressKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Invite => "invite",
            Self::Ack => "ack",
            Self::Bye => "bye",
            Self::Cancel => "cancel",
            Self::Other => "other",
        }
    }
}

struct QueuedTransactionDispatch {
    event: TransportEvent,
    queued_at: Option<Instant>,
    kind: TransactionIngressKind,
    worker_id: usize,
}

#[derive(Clone)]
struct TransactionDispatchWorkerSender {
    control: mpsc::Sender<QueuedTransactionDispatch>,
    high: mpsc::Sender<QueuedTransactionDispatch>,
    normal: mpsc::Sender<QueuedTransactionDispatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionDispatchLane {
    Control,
    High,
    Normal,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Invite2xxDeadline {
    due_at: Instant,
    expires_at: Instant,
    generation: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Invite2xxDueEntry {
    transaction_id: Arc<TransactionKey>,
    due_at: Instant,
    expires_at: Instant,
    generation: u64,
}

/// Exact deadline index for cached INVITE 2xx responses.
///
/// `by_due` drives retransmission, `by_expiry` drives expiry and bounded
/// capacity eviction, and `by_transaction` makes replacement exact. Keeping
/// one entry in each index avoids the stale heap growth and periodic full-heap
/// rebuild previously caused by ACKs and retransmission backoff.
#[derive(Debug, Default)]
struct Invite2xxDeadlineScheduler {
    by_due: BTreeMap<(Instant, u64), Arc<TransactionKey>>,
    by_expiry: BTreeMap<(Instant, u64), Arc<TransactionKey>>,
    by_transaction: HashMap<Arc<TransactionKey>, Invite2xxDeadline>,
    next_generation: u64,
}

impl Invite2xxDeadlineScheduler {
    fn next_generation(&mut self, due_at: Instant, expires_at: Instant) -> u64 {
        loop {
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1);
            if !self.by_due.contains_key(&(due_at, generation))
                && !self.by_expiry.contains_key(&(expires_at, generation))
            {
                return generation;
            }
        }
    }

    fn schedule(
        &mut self,
        transaction_id: TransactionKey,
        due_at: Instant,
        expires_at: Instant,
    ) -> u64 {
        let previous = self
            .by_transaction
            .get_key_value(&transaction_id)
            .map(|(key, deadline)| (Arc::clone(key), *deadline));

        if let Some((_, previous)) = previous.as_ref() {
            if previous.due_at == due_at && previous.expires_at == expires_at {
                return previous.generation;
            }
        }

        let transaction_id = previous
            .as_ref()
            .map(|(key, _)| Arc::clone(key))
            .unwrap_or_else(|| Arc::new(transaction_id));
        if let Some((_, previous)) = previous {
            self.by_due.remove(&(previous.due_at, previous.generation));
            self.by_expiry
                .remove(&(previous.expires_at, previous.generation));
        }

        let generation = self.next_generation(due_at, expires_at);
        let deadline = Invite2xxDeadline {
            due_at,
            expires_at,
            generation,
        };
        self.by_transaction
            .insert(Arc::clone(&transaction_id), deadline);
        self.by_due
            .insert((due_at, generation), Arc::clone(&transaction_id));
        self.by_expiry
            .insert((expires_at, generation), transaction_id);
        generation
    }

    fn unschedule(&mut self, transaction_id: &TransactionKey, generation: u64) -> bool {
        let Some(deadline) = self.by_transaction.get(transaction_id).copied() else {
            return false;
        };
        if deadline.generation != generation {
            return false;
        }
        self.remove_exact(transaction_id, deadline).is_some()
    }

    fn remove_exact(
        &mut self,
        transaction_id: &TransactionKey,
        deadline: Invite2xxDeadline,
    ) -> Option<Invite2xxDueEntry> {
        if !self
            .by_transaction
            .get(transaction_id)
            .is_some_and(|current| *current == deadline)
        {
            return None;
        }

        let (transaction_id, _) = self.by_transaction.remove_entry(transaction_id)?;
        self.by_due.remove(&(deadline.due_at, deadline.generation));
        self.by_expiry
            .remove(&(deadline.expires_at, deadline.generation));
        Some(Invite2xxDueEntry {
            transaction_id,
            due_at: deadline.due_at,
            expires_at: deadline.expires_at,
            generation: deadline.generation,
        })
    }

    fn take_due(&mut self, now: Instant, max_work: usize) -> (Vec<Invite2xxDueEntry>, bool) {
        let mut due = Vec::with_capacity(max_work.min(self.len()));
        while due.len() < max_work {
            let Some((&(due_at, generation), transaction_id)) = self.by_due.first_key_value()
            else {
                break;
            };
            if due_at > now {
                break;
            }

            let transaction_id = Arc::clone(transaction_id);
            let Some(deadline) = self.by_transaction.get(transaction_id.as_ref()).copied() else {
                // This invariant is defended in release builds as well as by
                // debug assertions: discard only the inconsistent index row.
                self.by_due.pop_first();
                continue;
            };
            if deadline.due_at != due_at || deadline.generation != generation {
                self.by_due.pop_first();
                continue;
            }
            if let Some(entry) = self.remove_exact(transaction_id.as_ref(), deadline) {
                due.push(entry);
            }
        }

        let capped = due.len() == max_work
            && self
                .by_due
                .first_key_value()
                .is_some_and(|(&(due_at, _), _)| due_at <= now);
        (due, capped)
    }

    fn take_expired_and_overflow(
        &mut self,
        now: Instant,
        capacity: usize,
        max_work: usize,
    ) -> Vec<Invite2xxDueEntry> {
        let mut removals = Vec::with_capacity(max_work.min(self.len()));
        while removals.len() < max_work {
            let Some((&(expires_at, generation), transaction_id)) =
                self.by_expiry.first_key_value()
            else {
                break;
            };
            if expires_at > now && self.len() <= capacity {
                break;
            }

            let transaction_id = Arc::clone(transaction_id);
            let Some(deadline) = self.by_transaction.get(transaction_id.as_ref()).copied() else {
                self.by_expiry.pop_first();
                continue;
            };
            if deadline.expires_at != expires_at || deadline.generation != generation {
                self.by_expiry.pop_first();
                continue;
            }
            if let Some(entry) = self.remove_exact(transaction_id.as_ref(), deadline) {
                removals.push(entry);
            }
        }
        removals
    }

    fn len(&self) -> usize {
        debug_assert_eq!(self.by_due.len(), self.by_transaction.len());
        debug_assert_eq!(self.by_expiry.len(), self.by_transaction.len());
        self.by_transaction.len()
    }

    fn clear(&mut self) {
        self.by_due.clear();
        self.by_expiry.clear();
        self.by_transaction.clear();
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ServerInviteAckExpiryEntry {
    due_at: Instant,
    generation: u64,
    dialog_key: ServerInviteDialogKey,
}

impl Ord for ServerInviteAckExpiryEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap, so reverse deadline/sequence ordering to
        // expose the earliest deadline at `peek()`.
        other
            .due_at
            .cmp(&self.due_at)
            .then_with(|| other.generation.cmp(&self.generation))
    }
}

impl PartialOrd for ServerInviteAckExpiryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Define RFC3261 Branch magic cookie
pub const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";

/// Build default `TimerSettings`, with an opt-in test hook that lets
/// integration tests shorten Timer F (non-INVITE transaction timeout) so
/// they don't have to wait the full 32 s for a dead peer to surface as a
/// timeout. Reads `RVOIP_TEST_TRANSACTION_TIMEOUT_MS` once at construction;
/// production callers are unaffected unless the env var is explicitly set
/// in the child process environment.
fn build_timer_settings() -> TimerSettings {
    let mut settings = TimerSettings::default();
    if let Ok(raw) = std::env::var("RVOIP_TEST_TRANSACTION_TIMEOUT_MS") {
        if let Ok(ms) = raw.parse::<u64>() {
            settings.transaction_timeout = std::time::Duration::from_millis(ms);
        }
    }
    settings
}

fn transaction_ingress_kind(event: &TransportEvent) -> TransactionIngressKind {
    match event {
        TransportEvent::MessageReceived { message, .. } => match message {
            Message::Request(request) => match request.method() {
                Method::Invite => TransactionIngressKind::Invite,
                Method::Ack => TransactionIngressKind::Ack,
                Method::Bye => TransactionIngressKind::Bye,
                Method::Cancel => TransactionIngressKind::Cancel,
                _ => TransactionIngressKind::Other,
            },
            Message::Response(_) => TransactionIngressKind::Other,
        },
        _ => TransactionIngressKind::Control,
    }
}

fn transaction_dispatch_lane(kind: TransactionIngressKind) -> TransactionDispatchLane {
    match kind {
        TransactionIngressKind::Control => TransactionDispatchLane::Control,
        TransactionIngressKind::Ack | TransactionIngressKind::Bye => TransactionDispatchLane::High,
        TransactionIngressKind::Invite
        | TransactionIngressKind::Cancel
        | TransactionIngressKind::Other => TransactionDispatchLane::Normal,
    }
}

fn request_dialog_route_hash(request: &Request) -> Option<u64> {
    let call_id = request.call_id()?;
    let from_tag = request.from_tag()?;
    let mut hasher = DefaultHasher::new();
    call_id.value().hash(&mut hasher);
    from_tag.hash(&mut hasher);
    Some(hasher.finish())
}

fn transaction_event_route_hash(event: &TransportEvent) -> Option<u64> {
    let TransportEvent::MessageReceived { message, .. } = event else {
        return None;
    };

    if let Message::Request(request) = message {
        if let Some(hash) = request_dialog_route_hash(request) {
            return Some(hash);
        }
    }

    let key = transaction_key_from_message(message)?;
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    Some(hasher.finish())
}

fn transaction_dispatch_worker_index(
    event: &TransportEvent,
    worker_count: usize,
    fallback_worker: &AtomicUsize,
) -> usize {
    if worker_count <= 1 {
        return 0;
    }

    if let Some(hash) = transaction_event_route_hash(event) {
        return (hash as usize) % worker_count;
    }

    fallback_worker.fetch_add(1, Ordering::Relaxed) % worker_count
}

fn start_transaction_dispatch_workers(
    manager: TransactionManager,
    worker_count: usize,
    queue_capacity: usize,
    priority_burst_max: Arc<AtomicUsize>,
) -> Arc<Vec<TransactionDispatchWorkerSender>> {
    let worker_count = worker_count.clamp(1, MAX_TRANSACTION_DISPATCH_WORKERS);
    let per_worker_capacity = (queue_capacity / worker_count).max(1);
    let mut senders = Vec::with_capacity(worker_count);

    for worker_id in 0..worker_count {
        let (control_tx, mut control_rx) =
            mpsc::channel::<QueuedTransactionDispatch>(per_worker_capacity.max(8));
        let (high_tx, mut high_rx) =
            mpsc::channel::<QueuedTransactionDispatch>(per_worker_capacity);
        let (normal_tx, mut normal_rx) =
            mpsc::channel::<QueuedTransactionDispatch>(per_worker_capacity);
        let manager_for_worker = manager.clone();
        let priority_burst_max_for_worker = priority_burst_max.clone();
        tokio::spawn(async move {
            let mut high_burst_count = 0usize;
            while let Some(queued) = recv_transaction_dispatch_event(
                &mut control_rx,
                &mut high_rx,
                &mut normal_rx,
                &mut high_burst_count,
                priority_burst_max_for_worker.load(Ordering::Relaxed).max(1),
            )
            .await
            {
                if let Some(queued_at) = queued.queued_at {
                    diagnostics::record_transaction_dispatch_queue_by_worker_and_kind(
                        queued.worker_id,
                        queued.kind.as_str(),
                        queued_at.elapsed(),
                        control_rx.len() + high_rx.len() + normal_rx.len(),
                    );
                }
                process_transaction_dispatch_event(&manager_for_worker, queued).await;
            }
            debug!(worker_id, "Transaction dispatch worker terminated");
        });
        senders.push(TransactionDispatchWorkerSender {
            control: control_tx,
            high: high_tx,
            normal: normal_tx,
        });
    }

    info!(
        workers = worker_count,
        per_worker_capacity, "Transaction manager dispatch workers enabled"
    );

    Arc::new(senders)
}

async fn recv_transaction_dispatch_event(
    control_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    high_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    normal_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    high_burst_count: &mut usize,
    priority_burst_max: usize,
) -> Option<QueuedTransactionDispatch> {
    loop {
        match control_rx.try_recv() {
            Ok(queued) => return Some(queued),
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {}
        }
        if *high_burst_count >= priority_burst_max.max(1) {
            match normal_rx.try_recv() {
                Ok(queued) => {
                    *high_burst_count = 0;
                    return Some(queued);
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return recv_high_transaction_dispatch_event(high_rx, high_burst_count).await;
                }
            }
        }

        match high_rx.try_recv() {
            Ok(queued) => {
                *high_burst_count += 1;
                return Some(queued);
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return recv_normal_transaction_dispatch_event(normal_rx, high_burst_count).await;
            }
        }

        match normal_rx.try_recv() {
            Ok(queued) => {
                *high_burst_count = 0;
                return Some(queued);
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return recv_high_transaction_dispatch_event(high_rx, high_burst_count).await;
            }
        }

        tokio::select! {
            biased;
            Some(queued) = control_rx.recv() => return Some(queued),
            queued = high_rx.recv() => {
                if let Some(queued) = queued {
                    *high_burst_count += 1;
                    return Some(queued);
                }
            }
            queued = normal_rx.recv() => {
                if let Some(queued) = queued {
                    *high_burst_count = 0;
                    return Some(queued);
                }
            }
        }
    }
}

async fn recv_high_transaction_dispatch_event(
    high_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    high_burst_count: &mut usize,
) -> Option<QueuedTransactionDispatch> {
    let queued = high_rx.recv().await?;
    *high_burst_count += 1;
    Some(queued)
}

async fn recv_normal_transaction_dispatch_event(
    normal_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    high_burst_count: &mut usize,
) -> Option<QueuedTransactionDispatch> {
    let queued = normal_rx.recv().await?;
    *high_burst_count = 0;
    Some(queued)
}

async fn dispatch_transaction_event(
    event: TransportEvent,
    dispatch_senders: &Arc<Vec<TransactionDispatchWorkerSender>>,
    fallback_worker: &AtomicUsize,
) {
    let worker_index =
        transaction_dispatch_worker_index(&event, dispatch_senders.len(), fallback_worker);
    let timing_enabled = diagnostics::transaction_timing_enabled();
    let kind = transaction_ingress_kind(&event);
    let queued = QueuedTransactionDispatch {
        event,
        kind,
        queued_at: timing_enabled.then(Instant::now),
        worker_id: worker_index,
    };
    let lane = transaction_dispatch_lane(kind);
    let sender = match lane {
        TransactionDispatchLane::Control => &dispatch_senders[worker_index].control,
        TransactionDispatchLane::High => &dispatch_senders[worker_index].high,
        TransactionDispatchLane::Normal => &dispatch_senders[worker_index].normal,
    };

    match sender.try_send(queued) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(queued)) => {
            if lane == TransactionDispatchLane::Control {
                warn!(
                    worker_index,
                    "Transaction control dispatch lane full; bounding lifecycle delivery"
                );
                let _ = tokio::time::timeout(Duration::from_millis(100), sender.send(queued)).await;
                return;
            }
            let backpressure_started = timing_enabled.then(Instant::now);
            warn!(
                worker_index,
                kind = queued.kind.as_str(),
                lane = ?lane,
                "Transaction dispatch worker queue full; applying backpressure"
            );
            if sender.send(queued).await.is_err() {
                warn!(worker_index, "Transaction dispatch worker channel closed");
            } else if let Some(started) = backpressure_started {
                diagnostics::record_transaction_dispatch_backpressure(started.elapsed());
            }
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            warn!(worker_index, "Transaction dispatch worker channel closed");
        }
    }
}

async fn process_transaction_dispatch_event(
    manager: &TransactionManager,
    queued: QueuedTransactionDispatch,
) {
    let timing_enabled = diagnostics::transaction_timing_enabled();
    let kind = queued.kind;
    let handler_started = timing_enabled.then(Instant::now);
    if let Err(e) = manager.handle_transport_event(queued.event).await {
        error!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error handling transport message");
    }
    if let Some(started) = handler_started {
        diagnostics::record_transaction_handler(kind.as_str(), started.elapsed());
    }
}

impl TransactionManager {
    pub(crate) fn try_enter_existing_operation(&self) -> Option<TransactionManagerAdmissionGuard> {
        self.admission_lifecycle.try_enter_existing()
    }

    fn install_terminal_delivery_failure_hook(&self) {
        let lifecycle = Arc::clone(&self.admission_lifecycle);
        self.events_tx
            .install_terminal_delivery_failure_hook(Arc::new(move || {
                lifecycle.begin_draining();
            }));
    }

    fn retained_client_deadline_context(&self) -> RetainedClientDeadlineContext {
        RetainedClientDeadlineContext {
            client_completions: Arc::downgrade(&self.client_completions),
            client_completion_deadlines: Arc::downgrade(&self.client_completion_deadlines),
            client_completion_capacity: self.client_completion_capacity,
            transaction_destinations: Arc::downgrade(&self.transaction_destinations),
            retired_client_deadlines: Arc::downgrade(&self.retired_client_deadlines),
            retired_client_transaction_capacity: Arc::clone(
                &self.retired_client_transaction_capacity,
            ),
            retired_client_transaction_count: Arc::downgrade(
                &self.retired_client_transaction_count,
            ),
        }
    }

    fn start_retained_client_deadline_worker(&self) {
        if let Some(worker) = self.retained_client_deadline_worker.as_ref() {
            worker.start(self.retained_client_deadline_context());
        }
    }

    fn wake_retained_client_deadline_worker(&self) {
        if let Some(worker) = self.retained_client_deadline_worker.as_ref() {
            worker.wake();
        }
    }

    #[cfg(test)]
    fn retained_client_deadline_worker_snapshot(
        &self,
    ) -> Option<RetainedClientDeadlineWorkerSnapshot> {
        self.retained_client_deadline_worker
            .as_ref()
            .map(RetainedClientDeadlineWorker::snapshot)
    }

    fn start_terminated_cleanup_worker(&self, mut rx: mpsc::Receiver<TransactionKey>) {
        let mut manager = self.clone();
        manager.terminated_cleanup_tx = None;
        manager.lifecycle_scheduler = None;
        manager.terminated_cleanup_shutdown = None;
        let shutdown = self
            .terminated_cleanup_shutdown
            .as_ref()
            .expect("runtime transaction managers install cleanup shutdown")
            .clone();
        tokio::spawn(async move {
            diagnostics::record_termination_cleanup_worker_spawned();
            let mut batch = Vec::with_capacity(TERMINATED_CLEANUP_BATCH_MAX);
            let mut seen = HashSet::with_capacity(TERMINATED_CLEANUP_BATCH_MAX);
            loop {
                let transaction_id = tokio::select! {
                    biased;
                    _ = shutdown.notified() => break,
                    transaction_id = rx.recv() => {
                        let Some(transaction_id) = transaction_id else {
                            break;
                        };
                        transaction_id
                    }
                };
                // Deduplicate both StateChanged(Terminated) and the runner's
                // authoritative TransactionTerminated notification without a
                // retained per-transaction retry/timer object.
                batch.clear();
                seen.clear();
                seen.insert(transaction_id.clone());
                batch.push(transaction_id);
                for _ in 1..TERMINATED_CLEANUP_BATCH_MAX {
                    match rx.try_recv() {
                        Ok(transaction_id) => {
                            if seen.insert(transaction_id.clone()) {
                                batch.push(transaction_id);
                            }
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                manager.process_transaction_cleanup_batch(&batch).await;
            }

            debug!("Terminated transaction cleanup worker stopped");
        });
    }

    fn enqueue_terminated_transaction_cleanup(&self, transaction_id: TransactionKey) {
        let Some(tx) = &self.terminated_cleanup_tx else {
            return;
        };

        match tx.try_send(transaction_id) {
            Ok(()) => diagnostics::record_termination_cleanup_enqueued(),
            Err(mpsc::error::TrySendError::Full(_)) => {
                diagnostics::record_termination_cleanup_queue_full();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    async fn process_terminated_cleanup_batch(&self, batch: &[TransactionKey]) {
        for transaction_id in batch {
            diagnostics::record_termination_cleanup_in_flight(1);
            if self.transaction_lifecycle_destroyed(transaction_id) {
                self.remove_terminated_transaction(transaction_id).await;
                diagnostics::record_termination_cleanup_removed();
            } else {
                // StateChanged(Terminated) can be observed before the shared
                // lifecycle scheduler finishes its 600 ms fence. The runner
                // submits a second, authoritative notification after it exits;
                // retain only the compact safety index in the meantime.
                self.mark_transaction_terminated_indexed(transaction_id);
            }
            diagnostics::record_termination_cleanup_in_flight(-1);
        }
    }

    async fn process_transaction_cleanup_batch(&self, batch: &[TransactionKey]) {
        diagnostics::record_termination_cleanup_batch(batch.len());

        // Explicit API operations transfer their admission guards into the
        // private index before their cleanup key is queued. Drain them first
        // without reacquiring admission: shutdown advances to Stopping before
        // `wait_idle`, so requiring a new guard here would strand the accepted
        // operation and deadlock shutdown on its transferred guard.
        let mut explicit = Vec::new();
        for transaction_id in batch {
            if let Some((_, mut operations)) =
                self.explicit_termination_operations.remove(transaction_id)
            {
                explicit.append(&mut operations);
            }
        }
        futures::stream::iter(explicit)
            .for_each_concurrent(
                Some(EXPLICIT_TERMINATION_CONCURRENCY_MAX),
                |operation| async move {
                    operation.execute(self).await;
                },
            )
            .await;

        if self.explicit_termination_operations.is_empty()
            && self.explicit_termination_operations.capacity()
                > self.transaction_index_initial_capacity.max(4_096)
        {
            self.explicit_termination_operations.shrink_to_fit();
        }

        // Ordinary runner notifications do not own a transferred guard. Keep
        // their existing fail-closed shutdown behavior, but never stop the
        // worker merely because admission or cancellation closed: a queued
        // explicit key may still follow and must release its own guard.
        let Some(_cleanup_operation) = self.admission_lifecycle.try_enter_existing() else {
            return;
        };
        tokio::select! {
            _ = self.operation_cancellation.cancelled() => {}
            _ = self.process_terminated_cleanup_batch(batch) => {}
        }
    }

    fn transaction_lifecycle_destroyed(&self, transaction_id: &TransactionKey) -> bool {
        let client_state = self
            .client_transactions
            .get(transaction_id)
            .map(|r| r.value().data().get_lifecycle());
        let server_state = self
            .server_transactions
            .get(transaction_id)
            .map(|r| r.value().data().get_lifecycle());

        match (client_state, server_state) {
            (None, None) => true,
            (Some(TransactionLifecycle::Destroyed), _) => true,
            (_, Some(TransactionLifecycle::Destroyed)) => true,
            _ => false,
        }
    }

    /// Enable or disable the INVITE server transaction automatic `100 Trying`
    /// timer used by newly-created server transactions.
    pub fn set_auto_100_trying(&mut self, enabled: bool) {
        self.timer_settings.timer_100_interval = if enabled {
            TimerSettings::default().timer_100_interval
        } else {
            std::time::Duration::ZERO
        };
    }

    /// Install a listener-level authorizer before accepting requests.
    ///
    /// The default is `None`, preserving server-only SIP behavior. Production
    /// callers should install the policy before exposing the transport
    /// listener; constructor variants that accept an authorizer avoid a boot
    /// race entirely.
    pub fn set_request_ingress_authorizer(
        &mut self,
        authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
    ) {
        self.request_ingress_authorizer = authorizer;
    }

    /// Return the currently installed listener authorizer.
    pub fn request_ingress_authorizer(&self) -> Option<Arc<dyn SipRequestIngressAuthorizer>> {
        self.request_ingress_authorizer.clone()
    }

    /// Set the maximum number of consecutive priority-lane ACK/BYE events a
    /// transaction dispatch worker may process before giving one ready normal
    /// item a turn. Values below `1` are clamped to `1`.
    pub fn set_transaction_dispatch_priority_burst_max(&self, max_burst: usize) {
        self.transaction_dispatch_priority_burst_max
            .store(max_burst.max(1), Ordering::Relaxed);
    }

    /// Set the capacity used for newly-created transaction command channels.
    ///
    /// The command channel is private to one transaction runner. Values below
    /// `1` are clamped to `1`; callers should prefer the default unless a
    /// measured high-CPS profile proves a larger per-transaction timer/state
    /// command buffer is needed.
    pub fn set_transaction_command_channel_capacity(&mut self, capacity: usize) {
        self.transaction_command_channel_capacity = capacity.max(1);
    }

    /// Return the capacity used for newly-created transaction command channels.
    pub fn transaction_command_channel_capacity(&self) -> usize {
        self.transaction_command_channel_capacity
    }

    /// Register the integrated dialog consumer's extra dispatch queue. Compact
    /// terminal key fences then remain live until the dialog worker confirms
    /// that it has finished processing `TransactionTerminated`.
    pub(crate) fn require_compact_dialog_terminal_ack(&self) {
        if self.events_tx.supports_exact_compact_terminal_ack() {
            self.events_tx.require_terminal_ack();
            if let Some(scheduler) = self.lifecycle_scheduler.as_ref() {
                scheduler.require_dialog_terminal_ack();
            }
        }
    }

    /// Install the integrated dialog consumer's exact-key route cleanup
    /// backstop. The callback runs before the final admission generation is
    /// released, after every owner spanning active, compact, delivery, and
    /// dialog-ack state has gone away.
    pub(crate) fn install_transaction_admission_release_hook<F>(&self, hook: F)
    where
        F: Fn(&TransactionKey) + Send + Sync + 'static,
    {
        self.transaction_admissions
            .install_final_release_hook(Arc::new(hook));
    }

    pub(crate) fn dialog_terminal_consumer_attach_state_is_clean(&self) -> bool {
        self.transaction_admissions.entries.is_empty()
            && self.client_transactions.is_empty()
            && self.server_transactions.is_empty()
            && self.client_completions.is_empty()
            && self.compact_non_invite_tombstones.is_empty()
            && self.transaction_destinations.is_empty()
            && self.terminated_transactions.is_empty()
            && self.server_invite_dialog_index.is_empty()
            && self.invite_2xx_response_cache.is_empty()
    }

    pub(crate) fn take_terminal_event_fence(
        &self,
        event: &Arc<TransactionEvent>,
    ) -> Option<crate::transaction::event_sender::TerminalEventFence> {
        self.events_tx.take_terminal_event_fence(event)
    }

    /// Acknowledge the exact compact generation carried by the processed
    /// shared event. The pointer-bound sidecar prevents any same-key legacy
    /// event from finalizing a replacement generation.
    pub(crate) fn acknowledge_compact_dialog_terminal(
        &self,
        transaction_id: &TransactionKey,
        generation: u64,
    ) {
        if self
            .compact_non_invite_tombstones
            .get(transaction_id)
            .is_none_or(|entry| entry.value().state().get() != TransactionState::Terminated)
        {
            return;
        }
        crate::transaction::lifecycle_scheduler::acknowledge_dialog_terminal_generation(
            &self.compact_non_invite_tombstones,
            &self.transaction_destinations,
            &self.pending_inbound_principals,
            &self.pending_inbound_principal_inserted_at,
            transaction_id,
            generation,
            self.events_tx.observer_fanout(),
        );
    }

    /// Set the maximum number of cached INVITE 2xx responses retransmitted by
    /// each proactive maintenance tick. Values below `1` are clamped to `1`.
    pub fn set_invite_2xx_retransmit_max_due_per_tick(&self, max_due_per_tick: usize) {
        self.invite_2xx_retransmit_max_due_per_tick
            .store(max_due_per_tick.max(1), Ordering::Relaxed);
    }

    /// Return retained transaction-manager state counts for perf leak gates.
    ///
    /// This prunes expired short-lived indexes first so idle post-drain samples
    /// reflect live retention rather than expired tombstone/cache residue.
    pub fn retention_counts(&self) -> TransactionManagerRetentionCounts {
        self.maintenance_prune_retained_state();
        // This API is an explicit diagnostic snapshot, so repair any expired
        // entry that predates the due queue or survived a corrupted/stale
        // deadline. Runtime maintenance itself never scans the full map.
        self.repair_expired_server_invite_dialog_index();

        let invite_2xx_response_due_queue = self
            .invite_2xx_response_due_queue
            .lock()
            .map(|queue| queue.len())
            .unwrap_or(0);
        let client_transactions = self.client_transactions.len();
        let server_transactions = self.server_transactions.len();

        TransactionManagerRetentionCounts {
            client_transactions,
            server_transactions,
            active_transactions_total: client_transactions + server_transactions,
            terminated_transactions: self.terminated_transactions.len(),
            server_invite_dialog_index: self.server_invite_dialog_index.len(),
            server_invite_dialog_keys_by_tx: self.server_invite_dialog_keys_by_tx.len(),
            invite_2xx_response_cache: self.invite_2xx_response_cache.len(),
            invite_2xx_response_due_queue,
            transaction_destinations: self
                .transaction_destinations
                .iter()
                .filter(|entry| entry.value().is_active())
                .count(),
            compact_non_invite_tombstones: self.compact_non_invite_tombstones.len(),
            compact_non_invite_deadlines: self
                .lifecycle_scheduler
                .as_ref()
                .map(|scheduler| scheduler.compact_deadline_count())
                .unwrap_or(0),
            event_subscribers: self.event_subscribers.load().len()
                + self.subscriber_to_transactions.len(),
            subscriber_to_transactions: self.subscriber_to_transactions.len(),
            transaction_to_subscribers: self.transaction_to_subscribers.len(),
            pending_inbound_bytes: self.pending_inbound_bytes.len(),
            pending_inbound_transport: self.pending_inbound_transport.len(),
            pending_inbound_timing: self.pending_inbound_timing.len(),
            pending_inbound_principals: self.pending_inbound_principals.len(),
        }
    }

    /// Number of unexpired INVITE client tombstones retained for authenticated
    /// late-2xx handling. Kept separate from `TransactionManagerRetentionCounts`
    /// so adding this diagnostic does not break downstream exhaustive struct
    /// literals of that compatibility type.
    pub fn retired_client_transaction_count(&self) -> usize {
        self.prune_retired_client_transactions();
        self.retired_client_transaction_count_unpruned()
    }

    /// Return aggregate-safe counts for exact completion storage.
    pub fn client_completion_retention_counts(&self) -> ClientCompletionRetentionCounts {
        let mut counts = ClientCompletionRetentionCounts::default();
        for entry in self.client_completions.iter() {
            if entry.value().retained_deadline().is_some() {
                counts.retained += 1;
            } else {
                counts.active += 1;
            }
            let diagnostics = entry.value().diagnostics();
            counts.compact += diagnostics.compact;
            counts.parsed_responses += diagnostics.parsed_responses;
            counts.wire_responses += diagnostics.wire_responses;
            counts.wire_response_bytes += diagnostics.wire_response_bytes;
        }
        // INVITE request/route/auth state and its exact completion now share
        // one retained record and one deadline. Count that completion here
        // even though it no longer occupies `client_completions`.
        for entry in self.transaction_destinations.iter() {
            let Some(retired) = entry.value().retired() else {
                continue;
            };
            if self.client_completions.contains_key(entry.key().as_ref()) {
                // A transition observer caught the tiny route-published,
                // active-completion-not-yet-removed interval. Report the
                // authoritative active map record once rather than double
                // counting the same exact completion.
                continue;
            }
            counts.retained += 1;
            let diagnostics = retired.completion.diagnostics();
            counts.compact += diagnostics.compact;
            counts.parsed_responses += diagnostics.parsed_responses;
            counts.wire_responses += diagnostics.wire_responses;
            counts.wire_response_bytes += diagnostics.wire_response_bytes;
        }
        counts.deadlines = self
            .client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
            + self
                .retired_client_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len();
        counts
    }

    /// Return aggregate-safe retained INVITE storage diagnostics without
    /// cloning any tombstone. The parsed ACK-template count intentionally
    /// remains zero because templates are reconstructed only while handling a
    /// rare late/forked 2xx response.
    pub fn retired_client_retention_counts(&self) -> RetiredClientRetentionCounts {
        self.prune_retired_client_transactions();
        let mut counts = RetiredClientRetentionCounts::default();
        for entry in self.transaction_destinations.iter() {
            if let Some(retired) = entry.value().retired() {
                counts.transactions += 1;
                counts.request_wire_bytes += retired.request_wire.len();
            }
        }
        counts.deadlines = self.retired_client_deadline_count();
        counts
    }

    /// Return retained transaction breakdowns for perf diagnostics.
    ///
    /// This is diagnostic-only data used to attribute retained transactions by
    /// method, RFC state, transaction kind, and lifecycle after soak drains.
    pub fn retention_breakdown(&self) -> serde_json::Value {
        self.maintenance_prune_retained_state();
        self.repair_expired_server_invite_dialog_index();

        fn increment(counts: &mut BTreeMap<String, usize>, key: impl Into<String>) {
            *counts.entry(key.into()).or_default() += 1;
        }

        let mut client_by_method = BTreeMap::new();
        let mut client_by_state = BTreeMap::new();
        let mut client_by_kind = BTreeMap::new();
        let mut client_by_lifecycle = BTreeMap::new();
        for entry in self.client_transactions.iter() {
            let key = entry.key();
            let tx = entry.value();
            increment(
                &mut client_by_method,
                crate::transaction::safe_diagnostics::SafeMethod::new(key.method()).to_string(),
            );
            increment(&mut client_by_state, format!("{:?}", tx.state()));
            increment(&mut client_by_kind, tx.kind().to_string());
            increment(
                &mut client_by_lifecycle,
                format!("{:?}", tx.data().get_lifecycle()),
            );
        }

        let mut server_by_method = BTreeMap::new();
        let mut server_by_state = BTreeMap::new();
        let mut server_by_kind = BTreeMap::new();
        let mut server_by_lifecycle = BTreeMap::new();
        for entry in self.server_transactions.iter() {
            let key = entry.key();
            let tx = entry.value();
            increment(
                &mut server_by_method,
                crate::transaction::safe_diagnostics::SafeMethod::new(key.method()).to_string(),
            );
            increment(&mut server_by_state, format!("{:?}", tx.state()));
            increment(&mut server_by_kind, tx.kind().to_string());
            increment(
                &mut server_by_lifecycle,
                format!("{:?}", tx.data().get_lifecycle()),
            );
        }

        let completion_counts = self.client_completion_retention_counts();
        let retired_client_counts = self.retired_client_retention_counts();

        // These values deliberately separate payload bytes and inline record
        // sizes from hash/tree node overhead. Together with table capacities
        // they make a diagnostic profile actionable without pretending that
        // `size_of` is an allocator-accurate heap measurement.
        let transaction_key_payload_bytes = |key: &TransactionKey| key.branch.capacity();
        let server_dialog_key_payload_bytes = |key: &ServerInviteDialogKey| {
            key.call_id.capacity()
                + key.from_tag.capacity()
                + key.to_tag.as_ref().map_or(0, String::capacity)
        };

        let mut compact_client_tombstones = 0_usize;
        let mut compact_server_tombstones = 0_usize;
        let mut compact_tombstone_key_bytes = 0_usize;
        let mut compact_server_response_wire_bytes = 0_usize;
        let mut compact_client_completion_wire_bytes = 0_usize;
        let mut compact_client_live_completion_cells = 0_usize;
        for entry in self.compact_non_invite_tombstones.iter() {
            compact_tombstone_key_bytes += transaction_key_payload_bytes(entry.key());
            match entry.value() {
                crate::transaction::lifecycle_scheduler::CompactNonInviteTombstone::Client {
                    completion,
                    live_completion,
                    ..
                } => {
                    compact_client_tombstones += 1;
                    compact_client_completion_wire_bytes +=
                        completion.diagnostics().wire_response_bytes;
                    compact_client_live_completion_cells +=
                        usize::from(live_completion.strong_count() > 0);
                }
                crate::transaction::lifecycle_scheduler::CompactNonInviteTombstone::Server {
                    response_wire,
                    ..
                } => {
                    compact_server_tombstones += 1;
                    compact_server_response_wire_bytes += response_wire.len();
                }
            }
        }

        let mut completion_key_bytes = 0_usize;
        let mut completion_route_shared_key_records = 0_usize;
        for entry in self.client_completions.iter() {
            completion_key_bytes += transaction_key_payload_bytes(entry.key());
            completion_route_shared_key_records += self
                .transaction_destinations
                .get(entry.key().as_ref())
                .is_some_and(|route| Arc::ptr_eq(entry.key(), route.key()))
                as usize;
        }
        let (
            completion_deadline_count,
            completion_deadline_key_bytes,
            completion_deadline_shared_key_records,
        ) = {
            let deadlines = self
                .client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                deadlines.by_deadline.len(),
                deadlines
                    .by_deadline
                    .values()
                    .map(|key| transaction_key_payload_bytes(key.as_ref()))
                    .sum::<usize>(),
                deadlines
                    .by_deadline
                    .values()
                    .filter(|key| {
                        self.client_completions
                            .get(key.as_ref())
                            .is_some_and(|completion| Arc::ptr_eq(completion.key(), key))
                    })
                    .count(),
            )
        };

        let mut retired_route_key_bytes = 0_usize;
        let mut retained_invite_key_bytes = 0_usize;
        let mut retained_invite_records = 0_usize;
        let mut retained_invite_response_records = 0_usize;
        let mut retired_completion_wire_bytes = 0_usize;
        let mut retired_shared_wire_records = 0_usize;
        for entry in self.transaction_destinations.iter() {
            retired_route_key_bytes += transaction_key_payload_bytes(entry.key());
            if let Some(retired) = entry.value().retired() {
                retained_invite_records += 1;
                retained_invite_key_bytes += transaction_key_payload_bytes(entry.key());
                retained_invite_response_records += usize::from(retired.has_completion_wire());
                retired_completion_wire_bytes += retired.completion_wire_len();
                retired_shared_wire_records +=
                    usize::from(retired.has_completion_wire() && retired.shares_wire_allocation());
            }
        }
        let (
            retired_deadline_count,
            retired_deadline_key_bytes,
            retired_deadline_shared_key_records,
        ) = {
            let deadlines = self
                .retired_client_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                deadlines.by_deadline.len(),
                deadlines
                    .by_deadline
                    .values()
                    .map(|key| transaction_key_payload_bytes(key.as_ref()))
                    .sum::<usize>(),
                deadlines
                    .by_deadline
                    .values()
                    .filter(|key| {
                        self.transaction_destinations
                            .get(key.as_ref())
                            .is_some_and(|route| Arc::ptr_eq(route.key(), key))
                    })
                    .count(),
            )
        };

        let mut server_dialog_index_key_bytes = 0_usize;
        let mut server_dialog_index_transaction_key_bytes = 0_usize;
        for entry in self.server_invite_dialog_index.iter() {
            server_dialog_index_key_bytes += server_dialog_key_payload_bytes(entry.key());
            server_dialog_index_transaction_key_bytes +=
                transaction_key_payload_bytes(&entry.value().transaction_id);
        }
        let (server_dialog_expiry_count, server_dialog_expiry_key_bytes) = {
            let deadlines = self
                .server_invite_dialog_expiry_queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                deadlines.len(),
                deadlines
                    .iter()
                    .map(|deadline| server_dialog_key_payload_bytes(&deadline.dialog_key))
                    .sum::<usize>(),
            )
        };
        let mut server_dialog_reverse_transaction_key_bytes = 0_usize;
        let mut server_dialog_reverse_dialog_key_bytes = 0_usize;
        for entry in self.server_invite_dialog_keys_by_tx.iter() {
            server_dialog_reverse_transaction_key_bytes +=
                transaction_key_payload_bytes(entry.key());
            server_dialog_reverse_dialog_key_bytes += entry
                .value()
                .iter()
                .map(server_dialog_key_payload_bytes)
                .sum::<usize>();
        }

        let consolidated_retired_invite_layout = serde_json::json!({
            "records": retained_invite_records,
            "legacy_map_records": retained_invite_records.saturating_mul(2),
            "current_map_records": retained_invite_records,
            "legacy_deadline_records": retained_invite_records.saturating_mul(2),
            "current_deadline_records": retired_deadline_count,
            "legacy_transaction_key_payload_allocated_bytes": retained_invite_key_bytes.saturating_mul(4),
            "current_transaction_key_payload_allocated_bytes": retained_invite_key_bytes,
            "legacy_wire_backing_allocations": retained_invite_records.saturating_add(retained_invite_response_records),
            "current_wire_backing_allocations": retained_invite_records,
            "shared_deadline_key_records": retired_deadline_shared_key_records,
            "shared_request_response_records": retired_shared_wire_records,
            "legacy_value_inline_bytes_per_record": std::mem::size_of::<LegacyRetiredClientTransactionLayout>()
                + std::mem::size_of::<ClientTransactionCompletionEntry>(),
            "current_value_inline_bytes_per_record": std::mem::size_of::<RetiredClientTransaction>(),
            "legacy_index_inline_bytes_per_record": std::mem::size_of::<TransactionKey>().saturating_mul(2)
                + std::mem::size_of::<Arc<TransactionKey>>().saturating_mul(2),
            "current_index_inline_bytes_per_record": std::mem::size_of::<Arc<TransactionKey>>().saturating_mul(2),
        });

        serde_json::json!({
            "client_by_method": client_by_method,
            "client_by_state": client_by_state,
            "client_by_kind": client_by_kind,
            "client_by_lifecycle": client_by_lifecycle,
            "server_by_method": server_by_method,
            "server_by_state": server_by_state,
            "server_by_kind": server_by_kind,
            "server_by_lifecycle": server_by_lifecycle,
            "compact_non_invite_tombstones": self.compact_non_invite_tombstones.len(),
            "compact_non_invite_deadlines": self.lifecycle_scheduler.as_ref().map(|scheduler| scheduler.compact_deadline_count()).unwrap_or(0),
            "retired_client_transactions": retired_client_counts.transactions,
            "retired_client_deadlines": retired_client_counts.deadlines,
            "retired_client_request_wire_bytes": retired_client_counts.request_wire_bytes,
            "retired_client_ack_template_allocations": retired_client_counts.ack_template_allocations,
            "retired_client": {
                "transactions": retired_client_counts.transactions,
                "request_wire_bytes": retired_client_counts.request_wire_bytes,
                "ack_template_allocations": retired_client_counts.ack_template_allocations,
                "deadlines": retired_client_counts.deadlines,
            },
            "client_completions": {
                "active": completion_counts.active,
                "retained": completion_counts.retained,
                "compact": completion_counts.compact,
                "parsed_responses": completion_counts.parsed_responses,
                "wire_responses": completion_counts.wire_responses,
                "wire_response_bytes": completion_counts.wire_response_bytes,
                "deadlines": completion_counts.deadlines,
            },
            "storage": {
                "transaction_index_capacity": {
                    "logical": self.transaction_index_logical_capacity,
                    "initial_per_map": self.transaction_index_initial_capacity,
                    "max_eager_per_map": MAX_EAGER_TRANSACTION_INDEX_CAPACITY,
                },
                "record_inline_bytes": {
                    "transaction_key": std::mem::size_of::<TransactionKey>(),
                    "server_invite_dialog_key": std::mem::size_of::<ServerInviteDialogKey>(),
                    "server_invite_ack_index_entry": std::mem::size_of::<ServerInviteAckIndexEntry>(),
                    "server_invite_ack_expiry_entry": std::mem::size_of::<ServerInviteAckExpiryEntry>(),
                    "client_completion_entry": std::mem::size_of::<ClientTransactionCompletionEntry>(),
                    "retained_client_completion": std::mem::size_of::<crate::transaction::completion::RetainedClientTransactionCompletion>(),
                    "live_client_completion": std::mem::size_of::<crate::transaction::completion::ClientTransactionCompletion>(),
                    "retired_client_transaction": std::mem::size_of::<RetiredClientTransaction>(),
                    "client_response_route_state": std::mem::size_of::<ClientResponseRouteState>(),
                    "compact_non_invite_tombstone": std::mem::size_of::<crate::transaction::lifecycle_scheduler::CompactNonInviteTombstone>(),
                    "transport_route": std::mem::size_of::<TransportRoute>(),
                },
                "table_capacity": {
                    "client_transactions": self.client_transactions.capacity(),
                    "server_transactions": self.server_transactions.capacity(),
                    "client_completions": self.client_completions.capacity(),
                    "terminated_transactions": self.terminated_transactions.capacity(),
                    "server_invite_dialog_index": self.server_invite_dialog_index.capacity(),
                    "server_invite_dialog_keys_by_tx": self.server_invite_dialog_keys_by_tx.capacity(),
                    "compact_non_invite_tombstones": self.compact_non_invite_tombstones.capacity(),
                    "transaction_destinations": self.transaction_destinations.capacity(),
                    "transaction_to_subscribers": self.transaction_to_subscribers.capacity(),
                    "pending_inbound_bytes": self.pending_inbound_bytes.capacity(),
                    "pending_inbound_inserted_at": self.pending_inbound_inserted_at.capacity(),
                    "pending_inbound_transport": self.pending_inbound_transport.capacity(),
                    "pending_inbound_timing": self.pending_inbound_timing.capacity(),
                    "pending_inbound_principals": self.pending_inbound_principals.capacity(),
                    "pending_inbound_principal_inserted_at": self.pending_inbound_principal_inserted_at.capacity(),
                },
                "compact_non_invite": {
                    "client_tombstones": compact_client_tombstones,
                    "server_tombstones": compact_server_tombstones,
                    "transaction_key_payload_bytes": compact_tombstone_key_bytes,
                    "server_response_wire_bytes": compact_server_response_wire_bytes,
                    "client_completion_wire_bytes": compact_client_completion_wire_bytes,
                    "client_live_completion_cells": compact_client_live_completion_cells,
                    "client_route_owner_proofs": compact_client_tombstones,
                },
                "client_completion": {
                    "map_transaction_key_payload_bytes": completion_key_bytes,
                    "map_route_shared_key_records": completion_route_shared_key_records,
                    "wire_response_bytes": completion_counts.wire_response_bytes,
                    "deadline_records": completion_deadline_count,
                    "deadline_transaction_key_payload_bytes": completion_deadline_key_bytes,
                    "deadline_shared_key_records": completion_deadline_shared_key_records,
                },
                "retired_client_route": {
                    "map_transaction_key_payload_bytes": retired_route_key_bytes,
                    "request_wire_bytes": retired_client_counts.request_wire_bytes,
                    "completion_wire_bytes": retired_completion_wire_bytes,
                    "shared_request_response_wire_records": retired_shared_wire_records,
                    "deadline_records": retired_deadline_count,
                    "deadline_transaction_key_payload_bytes": retired_deadline_key_bytes,
                    "deadline_shared_key_records": retired_deadline_shared_key_records,
                    "consolidated_layout": consolidated_retired_invite_layout,
                },
                "server_invite_ack_index": {
                    "index_records": self.server_invite_dialog_index.len(),
                    "index_dialog_key_payload_bytes": server_dialog_index_key_bytes,
                    "index_transaction_key_payload_bytes": server_dialog_index_transaction_key_bytes,
                    "expiry_records": server_dialog_expiry_count,
                    "expiry_dialog_key_payload_bytes": server_dialog_expiry_key_bytes,
                    "reverse_records": self.server_invite_dialog_keys_by_tx.len(),
                    "reverse_transaction_key_payload_bytes": server_dialog_reverse_transaction_key_bytes,
                    "reverse_dialog_key_payload_bytes": server_dialog_reverse_dialog_key_bytes,
                },
                "scope": "payload_and_inline_estimates_exclude_container_node_and_allocator_overhead",
            },
        })
    }

    /// Returns the timer settings in effect for this manager. Session-timer
    /// refresh logic needs this to pick a deadline for awaiting UPDATE /
    /// re-INVITE responses.
    pub fn timer_settings(&self) -> &TimerSettings {
        &self.timer_settings
    }

    /// Take (and remove) the original wire bytes for an inbound message
    /// keyed by transaction. Returns `Some` once for the first caller; a
    /// subsequent call returns `None`. Cross-crate event bridges call
    /// this when constructing `IncomingCall` / `IncomingRegister` /
    /// response-side variants so STIR/SHAKEN consumers see the
    /// upstream byte form. See `SIP_API_DESIGN_2.md` §7.5.
    pub fn take_inbound_bytes(&self, key: &TransactionKey) -> Option<bytes::Bytes> {
        self.pending_inbound_inserted_at.remove(key);
        self.pending_inbound_bytes.remove(key).map(|(_, v)| v)
    }

    /// Peek at the original wire bytes for an inbound message without
    /// removing the cache entry. Useful when multiple bridge sites
    /// derive views from the same inbound message (e.g., NOTIFY routes
    /// that emit both a transaction event and a SubscriptionUpdate).
    pub fn peek_inbound_bytes(&self, key: &TransactionKey) -> Option<bytes::Bytes> {
        // `Bytes::clone` is a refcount bump — no heap alloc.
        self.pending_inbound_bytes
            .get(key)
            .map(|r| r.value().clone())
    }

    /// Take (and remove) transport metadata for an inbound message keyed by
    /// transaction.
    pub fn take_inbound_transport(&self, key: &TransactionKey) -> Option<SipTransportContext> {
        self.pending_inbound_transport.remove(key).map(|(_, v)| v)
    }

    /// Peek at transport metadata for an inbound message without removing it.
    pub fn peek_inbound_transport(&self, key: &TransactionKey) -> Option<SipTransportContext> {
        self.pending_inbound_transport
            .get(key)
            .map(|r| r.value().clone())
    }

    /// Take (and remove) receive timing diagnostics for an inbound message
    /// keyed by transaction.
    pub fn take_inbound_timing(&self, key: &TransactionKey) -> Option<TransportReceiveTiming> {
        self.pending_inbound_timing.remove(key).map(|(_, v)| v)
    }

    /// Peek at receive timing diagnostics without removing the cache entry.
    pub fn peek_inbound_timing(&self, key: &TransactionKey) -> Option<TransportReceiveTiming> {
        self.pending_inbound_timing.get(key).map(|r| *r.value())
    }

    /// Consume the authenticated principal attached to a new inbound
    /// transaction. Dialog/session ingress uses this once when promoting an
    /// INVITE into an application call.
    pub fn take_inbound_principal(&self, key: &TransactionKey) -> Option<AuthenticatedPrincipal> {
        self.pending_inbound_principal_inserted_at.remove(key);
        self.pending_inbound_principals
            .remove(key)
            .map(|(_, binding)| binding.principal)
    }

    /// Clone the principal retained for an inbound transaction without
    /// consuming its ACK/CANCEL authorization binding.
    pub fn peek_inbound_principal(&self, key: &TransactionKey) -> Option<AuthenticatedPrincipal> {
        self.pending_inbound_principals
            .get(key)
            .map(|entry| entry.value().principal.clone())
    }

    pub(crate) fn inbound_principal_for_context(
        &self,
        key: &TransactionKey,
        context: &SipRequestIngressContext,
    ) -> Option<AuthenticatedPrincipal> {
        self.pending_inbound_principals.get(key).and_then(|entry| {
            entry
                .value()
                .matches(context)
                .then(|| entry.value().principal.clone())
        })
    }

    pub(crate) fn retain_inbound_principal(
        &self,
        key: TransactionKey,
        principal: AuthenticatedPrincipal,
        context: &SipRequestIngressContext,
    ) {
        let generation = self
            .pending_inbound_principal_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.pending_inbound_principal_inserted_at.insert(
            key.clone(),
            InboundPrincipalLease {
                inserted_at: Instant::now(),
                generation,
            },
        );
        self.pending_inbound_principals
            .insert(key, InboundPrincipalBinding::new(principal, context));
    }

    /// Install a forwarder for transport-side events (pong received,
    /// connection closed) that the RFC 5626 outbound-flow monitor in
    /// dialog-core needs to observe. Called by
    /// `DialogManager::with_global_events` once the consumer task is
    /// wired up; leaves the manager in a no-op state otherwise.
    pub async fn set_flow_event_sender(
        &self,
        sender: mpsc::Sender<crate::manager::outbound_flow::FlowTransportEvent>,
    ) {
        let Some(_operation) = self.admission_lifecycle.try_enter_existing() else {
            return;
        };
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {}
            mut slot = self.flow_event_sender.write() => {
                *slot = Some(sender);
            }
        }
    }

    /// Close the dialog-layer flow event channel during an orderly manager
    /// drain so its consumer can be joined rather than detached.
    pub(crate) async fn clear_flow_event_sender(&self) {
        self.flow_event_sender.write().await.take();
    }

    /// Creates a new transaction manager with default settings.
    ///
    /// This async constructor sets up the transaction manager with default timer settings
    /// and starts the message processing loop. It is the preferred way to create a
    /// transaction manager in an async context.
    ///
    /// ## Transaction Manager Initialization
    ///
    /// The initialization process:
    /// 1. Sets up internal data structures for tracking transactions
    /// 2. Initializes the timer management system
    /// 3. Starts the message processing loop to handle transport events
    /// 4. Returns the manager and an event receiver for transaction events
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17: The transaction layer requires proper initialization
    /// - RFC 3261 Section 17.1.1.2 and 17.1.2.2: Timer initialization for retransmissions
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use tokio::sync::mpsc;
    /// # use rvoip_sip_transport::{Transport, TransportEvent};
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # async fn example(transport: Arc<dyn Transport>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a transport event channel
    /// let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(100);
    ///
    /// // Create transaction manager
    /// let (manager, event_rx) = TransactionManager::new(
    ///     transport,
    ///     transport_rx,
    ///     Some(200), // Buffer up to 200 events
    /// ).await?;
    ///
    /// // Now use the manager and listen for events
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        let index_capacity = transaction_index_capacity(Some(events_capacity));
        let index_initial_capacity = transaction_index_initial_capacity(index_capacity);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principals = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principal_inserted_at =
            Arc::new(DashMap::with_capacity(index_initial_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        // Observation is optional. Keep its indexes allocation-light until a
        // caller actually subscribes instead of sizing them like protocol
        // transaction tables.
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let transaction_to_subscribers = Arc::new(DashMap::new());
        let events_tx = TransactionEventSender::with_observers(
            events_tx,
            TransactionObserverFanout::new(
                event_subscribers.clone(),
                subscriber_to_transactions.clone(),
                transaction_to_subscribers.clone(),
            ),
        );
        let compact_non_invite_tombstones = Arc::new(DashMap::new());
        let lifecycle_scheduler =
            crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed(
                &compact_non_invite_tombstones,
                &transaction_destinations,
                &pending_inbound_principals,
                &pending_inbound_principal_inserted_at,
                &events_tx,
            );
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(AtomicBool::new(false));

        let timer_settings = build_timer_settings();

        // Setup timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));

        // Create timer factory with the timer manager
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        let manager = Self {
            transport,
            client_transactions,
            transaction_admissions: TransactionAdmissionRegistry::new(),
            admission_lifecycle: TransactionManagerAdmissionLifecycle::new(),
            operation_cancellation: TransactionManagerOperationCancellation::new(),
            shutdown_gate: Arc::new(Mutex::new(())),
            transaction_index_logical_capacity: index_capacity,
            transaction_index_initial_capacity: index_initial_capacity,
            client_completions: Arc::new(DashMap::new()),
            client_completion_deadlines: Arc::new(std::sync::Mutex::new(
                ClientCompletionDeadlineScheduler::default(),
            )),
            client_completion_capacity: retired_client_transaction_capacity(index_capacity),
            retained_client_deadline_worker: Some(RetainedClientDeadlineWorker::new()),
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_initial_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::new()),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(
                index_initial_capacity,
            )),
            server_invite_dialog_expiry_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            server_invite_dialog_deadline_generation: Arc::new(AtomicU64::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::new()),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(
                Invite2xxDeadlineScheduler::default(),
            )),
            explicit_termination_operations: Arc::new(DashMap::new()),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            terminated_cleanup_shutdown: Some(Arc::new(tokio::sync::Notify::new())),
            lifecycle_scheduler: Some(lifecycle_scheduler),
            compact_non_invite_tombstones,
            transaction_destinations,
            retired_client_transaction_capacity: Arc::new(AtomicUsize::new(
                retired_client_transaction_capacity(index_capacity),
            )),
            retired_client_transaction_count: Arc::new(AtomicUsize::new(0)),
            retired_client_deadlines: Arc::new(std::sync::Mutex::new(
                RetiredClientDeadlineScheduler::default(),
            )),
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Some(transport_rx),
            control_transport_rx: None,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            request_ingress_authorizer: None,
            pending_inbound_principals,
            pending_inbound_principal_inserted_at,
            pending_inbound_principal_generation: Arc::new(AtomicU64::new(0)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: events_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

        manager.install_terminal_delivery_failure_hook();
        manager.start_retained_client_deadline_worker();
        // Start the message processing loop
        manager.start_terminated_cleanup_worker(terminated_cleanup_rx);
        manager.start_message_loop();

        Ok((manager, events_rx))
    }

    /// Creates a new transaction manager with custom timer configuration.
    ///
    /// This async constructor allows customizing the timer settings, which affect
    /// retransmission intervals and timeouts. This is useful for fine-tuning SIP
    /// transaction behavior in different network environments.
    ///
    /// ## Timer Configuration Importance
    ///
    /// SIP transactions rely heavily on timers for reliability:
    /// - Timer A, B: Control INVITE retransmissions and timeouts
    /// - Timer E, F: Control non-INVITE retransmissions and timeouts
    /// - Timer G, H: Control INVITE response retransmissions
    /// - Timer I, J, K: Control various cleanup behaviors
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction timers
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction timers
    /// - RFC 3261 Section 17.2.1: INVITE server transaction timers
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction timers
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    /// * `timer_settings` - Optional custom timer settings
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use tokio::sync::mpsc;
    /// # use rvoip_sip_transport::{Transport, TransportEvent};
    /// # use rvoip_sip_dialog::transaction::{TransactionManager, timer::TimerSettings};
    /// # async fn example(transport: Arc<dyn Transport>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create custom timer settings for high-latency networks
    /// let mut timer_settings = TimerSettings::default();
    /// timer_settings.t1 = Duration::from_millis(1000); // Increase base timer
    ///
    /// // Create a transport event channel
    /// let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(100);
    ///
    /// // Create transaction manager with custom settings
    /// let (manager, event_rx) = TransactionManager::new_with_config(
    ///     transport,
    ///     transport_rx,
    ///     Some(200),
    ///     Some(timer_settings),
    /// ).await?;
    ///
    /// // Now use the manager with custom timer behavior
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new_with_config(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        timer_settings: Option<TimerSettings>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        let index_capacity = transaction_index_capacity(Some(events_capacity));
        let index_initial_capacity = transaction_index_initial_capacity(index_capacity);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principals = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principal_inserted_at =
            Arc::new(DashMap::with_capacity(index_initial_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        // Observation is optional; these maps grow only on subscription.
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let transaction_to_subscribers = Arc::new(DashMap::new());
        let events_tx = TransactionEventSender::with_observers(
            events_tx,
            TransactionObserverFanout::new(
                event_subscribers.clone(),
                subscriber_to_transactions.clone(),
                transaction_to_subscribers.clone(),
            ),
        );
        let compact_non_invite_tombstones = Arc::new(DashMap::new());
        let lifecycle_scheduler =
            crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed(
                &compact_non_invite_tombstones,
                &transaction_destinations,
                &pending_inbound_principals,
                &pending_inbound_principal_inserted_at,
                &events_tx,
            );
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(AtomicBool::new(false));

        // Create timer settings
        let timer_settings = timer_settings.unwrap_or_default();

        // Create the timer manager with custom config
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        let manager = Self {
            transport,
            client_transactions,
            transaction_admissions: TransactionAdmissionRegistry::new(),
            admission_lifecycle: TransactionManagerAdmissionLifecycle::new(),
            operation_cancellation: TransactionManagerOperationCancellation::new(),
            shutdown_gate: Arc::new(Mutex::new(())),
            transaction_index_logical_capacity: index_capacity,
            transaction_index_initial_capacity: index_initial_capacity,
            client_completions: Arc::new(DashMap::new()),
            client_completion_deadlines: Arc::new(std::sync::Mutex::new(
                ClientCompletionDeadlineScheduler::default(),
            )),
            client_completion_capacity: retired_client_transaction_capacity(index_capacity),
            retained_client_deadline_worker: Some(RetainedClientDeadlineWorker::new()),
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_initial_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::new()),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(
                index_initial_capacity,
            )),
            server_invite_dialog_expiry_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            server_invite_dialog_deadline_generation: Arc::new(AtomicU64::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::new()),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(
                Invite2xxDeadlineScheduler::default(),
            )),
            explicit_termination_operations: Arc::new(DashMap::new()),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            terminated_cleanup_shutdown: Some(Arc::new(tokio::sync::Notify::new())),
            lifecycle_scheduler: Some(lifecycle_scheduler),
            compact_non_invite_tombstones,
            transaction_destinations,
            retired_client_transaction_capacity: Arc::new(AtomicUsize::new(
                retired_client_transaction_capacity(index_capacity),
            )),
            retired_client_transaction_count: Arc::new(AtomicUsize::new(0)),
            retired_client_deadlines: Arc::new(std::sync::Mutex::new(
                RetiredClientDeadlineScheduler::default(),
            )),
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Some(transport_rx),
            control_transport_rx: None,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            request_ingress_authorizer: None,
            pending_inbound_principals,
            pending_inbound_principal_inserted_at,
            pending_inbound_principal_generation: Arc::new(AtomicU64::new(0)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: events_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

        manager.install_terminal_delivery_failure_hook();
        manager.start_retained_client_deadline_worker();
        // Start the message processing loop
        manager.start_terminated_cleanup_worker(terminated_cleanup_rx);
        manager.start_message_loop();

        Ok((manager, events_rx))
    }

    /// Creates a transaction manager synchronously (without async).
    ///
    /// This constructor is provided for code that cannot make its constructor
    /// async but is already executing inside a Tokio runtime. It installs the
    /// same manager-owned lifecycle scheduler and terminal cleanup worker as
    /// the async constructors. Calling it outside an entered Tokio runtime
    /// panics with an actionable message; use [`TransactionManager::new`] when
    /// transport ingress and a public primary event receiver are required.
    ///
    /// Note: Using the async `new()` method is preferred in async contexts.
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use rvoip_sip_transport::Transport;
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # fn example(transport: Arc<dyn Transport>) {
    /// // Create a transaction manager without async
    /// let manager = TransactionManager::new_sync(transport);
    ///
    /// // Manager can now be used or passed to an async context
    /// # }
    /// ```
    pub fn new_sync(transport: Arc<dyn Transport>) -> Self {
        Self::with_config(transport, None)
    }

    /// Creates a new TransactionManager that uses a TransportManager for SIP transport.
    ///
    /// This method integrates the transaction layer with the transport manager, allowing
    /// for advanced transport capabilities such as multiple transport types, failover,
    /// and transport selection based on destination.
    ///
    /// # Arguments
    /// * `transport_manager` - The TransportManager to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::net::SocketAddr;
    /// # use tokio::sync::mpsc;
    /// # use rvoip_sip_dialog::transaction::{TransactionManager, transport::TransportManager, transport::TransportManagerConfig};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a transport manager configuration
    /// let config = TransportManagerConfig {
    ///    bind_addresses: vec!["127.0.0.1:5060".parse().unwrap()],
    ///    enable_udp: true,
    ///    enable_tcp: true,
    ///    ..Default::default()
    /// };
    ///
    /// // Create and initialize the transport manager
    /// let (mut transport_manager, transport_rx) = TransportManager::new(config).await?;
    /// transport_manager.initialize().await?;
    ///
    /// // Create transaction manager with the transport manager
    /// let (transaction_manager, event_rx) = TransactionManager::with_transport_manager(
    ///     transport_manager,
    ///     transport_rx,
    ///     Some(100),
    /// ).await?;
    ///
    /// // Now use the transaction manager
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_transport_manager(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        Self::with_transport_manager_and_index_capacity(
            transport_manager,
            transport_rx,
            capacity,
            None,
        )
        .await
    }

    /// Creates a transaction manager whose authoritative TU event channel
    /// stores pointer-sized shared events.
    pub async fn with_transport_manager_shared(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<Arc<TransactionEvent>>)> {
        Self::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_shared(
            transport_manager,
            transport_rx,
            capacity,
            None,
            None,
            None,
            None,
        )
        .await
    }

    /// Creates a transaction manager with separate event-queue and hot-index
    /// capacities.
    ///
    /// Server profiles often need large event queues for SIP bursts without
    /// reserving every transaction lookup map at the same size. `index_capacity`
    /// controls active transaction maps and retransmission indexes; `capacity`
    /// remains the transaction event queue size.
    pub async fn with_transport_manager_and_index_capacity(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        Self::with_transport_manager_and_index_capacity_and_dispatch(
            transport_manager,
            transport_rx,
            capacity,
            index_capacity,
            None,
            None,
        )
        .await
    }

    /// Creates a transaction manager with separate event/index capacities and
    /// optional receive-side transaction dispatch workers.
    pub async fn with_transport_manager_and_index_capacity_and_dispatch(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
        dispatch_workers: Option<usize>,
        dispatch_queue_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        Self::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer(
            transport_manager,
            transport_rx,
            capacity,
            index_capacity,
            dispatch_workers,
            dispatch_queue_capacity,
            None,
        )
        .await
    }

    /// Creates a transaction manager with listener authorization installed
    /// before its receive loop starts.
    pub async fn with_transport_manager_and_index_capacity_and_dispatch_and_authorizer(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
        dispatch_workers: Option<usize>,
        dispatch_queue_capacity: Option<usize>,
        request_ingress_authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let (manager, events) =
            Self::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_mode(
                transport_manager,
                transport_rx,
                capacity,
                index_capacity,
                dispatch_workers,
                dispatch_queue_capacity,
                request_ingress_authorizer,
                None,
                TransactionEventChannelMode::Owned,
            )
            .await?;
        match events {
            TransactionManagerEventReceiver::Owned(events) => Ok((manager, events)),
            TransactionManagerEventReceiver::Shared(_) => unreachable!("owned event mode"),
        }
    }

    /// Canonical pointer-sized transaction-event path used by the integrated
    /// dialog stack. Legacy constructors select an independent owned-value
    /// primary, preserving their original single-queue semantics.
    pub async fn with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_shared(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
        dispatch_workers: Option<usize>,
        dispatch_queue_capacity: Option<usize>,
        request_ingress_authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
    ) -> Result<(Self, mpsc::Receiver<Arc<TransactionEvent>>)> {
        let (manager, events) =
            Self::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_mode(
                transport_manager,
                transport_rx,
                capacity,
                index_capacity,
                dispatch_workers,
                dispatch_queue_capacity,
                request_ingress_authorizer,
                None,
                TransactionEventChannelMode::Shared,
            )
            .await?;
        match events {
            TransactionManagerEventReceiver::Shared(events) => Ok((manager, events)),
            TransactionManagerEventReceiver::Owned(_) => unreachable!("shared event mode"),
        }
    }

    /// Shared-event constructor with an explicit, allocation-lazy logical
    /// bound for active UDP non-INVITE transactions plus retained Timer J/K
    /// tombstones. This bound is deliberately independent from hot-index
    /// sizing and must cover the expected anti-reuse/retransmission horizon.
    pub async fn with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_shared_with_compact_retention_capacity(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
        dispatch_workers: Option<usize>,
        dispatch_queue_capacity: Option<usize>,
        request_ingress_authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
        compact_retention_capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<Arc<TransactionEvent>>)> {
        let (manager, events) =
            Self::with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_mode(
                transport_manager,
                transport_rx,
                capacity,
                index_capacity,
                dispatch_workers,
                dispatch_queue_capacity,
                request_ingress_authorizer,
                Some(compact_retention_capacity.max(1)),
                TransactionEventChannelMode::Shared,
            )
            .await?;
        match events {
            TransactionManagerEventReceiver::Shared(events) => Ok((manager, events)),
            TransactionManagerEventReceiver::Owned(_) => unreachable!("shared event mode"),
        }
    }

    async fn with_transport_manager_and_index_capacity_and_dispatch_and_authorizer_mode(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        index_capacity: Option<usize>,
        dispatch_workers: Option<usize>,
        dispatch_queue_capacity: Option<usize>,
        request_ingress_authorizer: Option<Arc<dyn SipRequestIngressAuthorizer>>,
        compact_retention_capacity: Option<usize>,
        event_mode: TransactionEventChannelMode,
    ) -> Result<(Self, TransactionManagerEventReceiver)> {
        // Wrap the manager's per-flavour registry behind a
        // `MultiplexedTransport` so outbound requests get URI-aware
        // transport selection (RFC 3261 §18.1.1, §26.2). When the
        // manager only has a single flavour registered (typical
        // UDP-only setup) the multiplexer transparently delegates to it.
        let sip_trace = transport_manager.sip_trace_runtime();
        let default_transport = transport_manager
            .build_multiplexed_transport()
            .await
            .map_err(|e| {
                Error::Transport(format!(
                    "Failed to build multiplexed transport from TransportManager: {}",
                    e
                ))
            })?;
        let control_transport_rx = transport_manager
            .take_control_event_receiver()
            .await
            .map(|receiver| Arc::new(Mutex::new(receiver)));

        // Create the transaction manager using the default transport and event channel
        let events_capacity = capacity.unwrap_or(100);
        let (owned_events_tx, shared_events_tx, events_rx) = match event_mode {
            TransactionEventChannelMode::Owned => {
                let (sender, receiver) = mpsc::channel(events_capacity);
                (
                    Some(sender),
                    None,
                    TransactionManagerEventReceiver::Owned(receiver),
                )
            }
            TransactionEventChannelMode::Shared => {
                let (sender, receiver) = mpsc::channel(events_capacity);
                (
                    None,
                    Some(sender),
                    TransactionManagerEventReceiver::Shared(receiver),
                )
            }
        };
        let index_capacity = transaction_index_capacity(index_capacity.or(Some(events_capacity)));
        let index_initial_capacity = transaction_index_initial_capacity(index_capacity);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let transaction_dispatch_workers = transaction_dispatch_worker_count(dispatch_workers);
        let transaction_dispatch_queue_capacity =
            transaction_dispatch_queue_capacity(dispatch_queue_capacity, events_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principals = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principal_inserted_at =
            Arc::new(DashMap::with_capacity(index_initial_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        // Observation is optional; these maps grow only on subscription.
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let transaction_to_subscribers = Arc::new(DashMap::new());
        let observers = TransactionObserverFanout::new(
            event_subscribers.clone(),
            subscriber_to_transactions.clone(),
            transaction_to_subscribers.clone(),
        );
        let events_tx = match (owned_events_tx, shared_events_tx) {
            (Some(sender), None) => TransactionEventSender::with_observers(sender, observers),
            (None, Some(sender)) => {
                TransactionEventSender::with_shared_observers(sender, observers)
            }
            _ => unreachable!("exactly one transaction-event sender mode"),
        };
        let compact_non_invite_tombstones = Arc::new(DashMap::new());
        let lifecycle_scheduler = match compact_retention_capacity {
            Some(capacity) => crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed_with_retention_capacity(
                &compact_non_invite_tombstones,
                &transaction_destinations,
                &pending_inbound_principals,
                &pending_inbound_principal_inserted_at,
                &events_tx,
                capacity,
            ),
            None => crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed(
                &compact_non_invite_tombstones,
                &transaction_destinations,
                &pending_inbound_principals,
                &pending_inbound_principal_inserted_at,
                &events_tx,
            ),
        };
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(AtomicBool::new(false));

        let timer_settings = build_timer_settings();

        // Setup timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));

        // Create timer factory with the timer manager
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        let manager = Self {
            transport: default_transport,
            client_transactions,
            transaction_admissions: TransactionAdmissionRegistry::new(),
            admission_lifecycle: TransactionManagerAdmissionLifecycle::new(),
            operation_cancellation: TransactionManagerOperationCancellation::new(),
            shutdown_gate: Arc::new(Mutex::new(())),
            transaction_index_logical_capacity: index_capacity,
            transaction_index_initial_capacity: index_initial_capacity,
            client_completions: Arc::new(DashMap::new()),
            client_completion_deadlines: Arc::new(std::sync::Mutex::new(
                ClientCompletionDeadlineScheduler::default(),
            )),
            client_completion_capacity: retired_client_transaction_capacity(index_capacity),
            retained_client_deadline_worker: Some(RetainedClientDeadlineWorker::new()),
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_initial_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::new()),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(
                index_initial_capacity,
            )),
            server_invite_dialog_expiry_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            server_invite_dialog_deadline_generation: Arc::new(AtomicU64::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::new()),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(
                Invite2xxDeadlineScheduler::default(),
            )),
            explicit_termination_operations: Arc::new(DashMap::new()),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            terminated_cleanup_shutdown: Some(Arc::new(tokio::sync::Notify::new())),
            lifecycle_scheduler: Some(lifecycle_scheduler),
            compact_non_invite_tombstones,
            transaction_destinations,
            retired_client_transaction_capacity: Arc::new(AtomicUsize::new(
                retired_client_transaction_capacity(index_capacity),
            )),
            retired_client_transaction_count: Arc::new(AtomicUsize::new(0)),
            retired_client_deadlines: Arc::new(std::sync::Mutex::new(
                RetiredClientDeadlineScheduler::default(),
            )),
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Some(transport_rx),
            control_transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            request_ingress_authorizer,
            pending_inbound_principals,
            pending_inbound_principal_inserted_at,
            pending_inbound_principal_generation: Arc::new(AtomicU64::new(0)),
            transaction_dispatch_workers,
            transaction_dispatch_queue_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

        manager.install_terminal_delivery_failure_hook();
        manager.start_retained_client_deadline_worker();
        // Start the message processing loop
        manager.start_terminated_cleanup_worker(terminated_cleanup_rx);
        manager.start_message_loop();

        Ok((manager, events_rx))
    }

    /// Creates a transaction manager with custom timer configuration (sync version).
    ///
    /// This synchronous constructor allows customizing the timer settings
    /// in contexts where async initialization isn't possible.
    ///
    /// ## Timer Configuration
    ///
    /// The custom timer settings allow tuning:
    /// - T1: Base retransmission interval (default 500ms)
    /// - T2: Maximum retransmission interval (default 4s)
    /// - T4: Maximum duration a message remains in the network (default 5s)
    /// - TD: Wait time for response retransmissions (default 32s)
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `timer_settings_opt` - Optional custom timer settings
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use rvoip_sip_transport::Transport;
    /// # use rvoip_sip_dialog::transaction::{TransactionManager, timer::TimerSettings};
    /// # fn example(transport: Arc<dyn Transport>) {
    /// // Create custom timer settings
    /// let mut timer_settings = TimerSettings::default();
    /// timer_settings.t1 = Duration::from_millis(1000);
    ///
    /// // Create transaction manager with custom settings
    /// let manager = TransactionManager::with_config(
    ///     transport,
    ///     Some(timer_settings)
    /// );
    /// # }
    /// ```
    pub fn with_config(
        transport: Arc<dyn Transport>,
        timer_settings_opt: Option<TimerSettings>,
    ) -> Self {
        tokio::runtime::Handle::try_current().expect(
            "TransactionManager::with_config/new_sync requires an active Tokio runtime; use TransactionManager::new for async initialization",
        );
        let index_capacity = transaction_index_capacity(None);
        let index_initial_capacity = transaction_index_initial_capacity(index_capacity);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));
        let client_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principals = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let pending_inbound_principal_inserted_at =
            Arc::new(DashMap::with_capacity(index_initial_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        // Observation is optional; these maps grow only on subscription.
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let transaction_to_subscribers = Arc::new(DashMap::new());
        let events_tx =
            TransactionEventSender::detached_with_observers(TransactionObserverFanout::new(
                event_subscribers.clone(),
                subscriber_to_transactions.clone(),
                transaction_to_subscribers.clone(),
            ));
        let compact_non_invite_tombstones = Arc::new(DashMap::new());
        let lifecycle_scheduler =
            crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed(
                &compact_non_invite_tombstones,
                &transaction_destinations,
                &pending_inbound_principals,
                &pending_inbound_principal_inserted_at,
                &events_tx,
            );
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));
        // No receive loop or primary TU receiver exists for this historical
        // outbound-only constructor. Optional fields model that topology
        // directly instead of allocating inert channels and sink tasks.
        let running = Arc::new(AtomicBool::new(false));

        // Create timer settings
        let timer_settings = timer_settings_opt.unwrap_or_default();

        // Create the timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        let manager = Self {
            transport,
            client_transactions,
            transaction_admissions: TransactionAdmissionRegistry::new(),
            admission_lifecycle: TransactionManagerAdmissionLifecycle::new(),
            operation_cancellation: TransactionManagerOperationCancellation::new(),
            shutdown_gate: Arc::new(Mutex::new(())),
            transaction_index_logical_capacity: index_capacity,
            transaction_index_initial_capacity: index_initial_capacity,
            client_completions: Arc::new(DashMap::new()),
            client_completion_deadlines: Arc::new(std::sync::Mutex::new(
                ClientCompletionDeadlineScheduler::default(),
            )),
            client_completion_capacity: retired_client_transaction_capacity(index_capacity),
            retained_client_deadline_worker: Some(RetainedClientDeadlineWorker::new()),
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_initial_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::new()),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(
                index_initial_capacity,
            )),
            server_invite_dialog_expiry_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            server_invite_dialog_deadline_generation: Arc::new(AtomicU64::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::new()),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(
                Invite2xxDeadlineScheduler::default(),
            )),
            explicit_termination_operations: Arc::new(DashMap::new()),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            terminated_cleanup_shutdown: Some(Arc::new(tokio::sync::Notify::new())),
            lifecycle_scheduler: Some(lifecycle_scheduler),
            compact_non_invite_tombstones,
            transaction_destinations,
            retired_client_transaction_capacity: Arc::new(AtomicUsize::new(
                retired_client_transaction_capacity(index_capacity),
            )),
            retired_client_transaction_count: Arc::new(AtomicUsize::new(0)),
            retired_client_deadlines: Arc::new(std::sync::Mutex::new(
                RetiredClientDeadlineScheduler::default(),
            )),
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: None,
            control_transport_rx: None,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            request_ingress_authorizer: None,
            pending_inbound_principals,
            pending_inbound_principal_inserted_at,
            pending_inbound_principal_generation: Arc::new(AtomicU64::new(0)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: 100,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };
        manager.install_terminal_delivery_failure_hook();
        manager.start_retained_client_deadline_worker();
        manager.start_terminated_cleanup_worker(terminated_cleanup_rx);
        manager
    }

    /// Creates a minimal transaction manager for testing purposes.
    ///
    /// This constructor creates a transaction manager with the minimal
    /// required components for testing. It doesn't start message loops
    /// or perform other initialization that might complicate testing.
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance configured for testing
    pub fn dummy(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
    ) -> Self {
        // Setup basic channels
        let (events_tx, _) = mpsc::channel(10);
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));

        // Transaction registries
        let index_capacity = transaction_index_capacity(Some(10));
        let index_initial_capacity = transaction_index_initial_capacity(index_capacity);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let client_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_initial_capacity));

        // Setup timer manager
        let timer_settings = build_timer_settings();
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        // Initialize running state
        let running = Arc::new(AtomicBool::new(false));

        // Track destinations
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_initial_capacity));

        // Initialize subscriber-related fields
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let transaction_to_subscribers = Arc::new(DashMap::new());
        let events_tx = TransactionEventSender::with_observers(
            events_tx,
            TransactionObserverFanout::new(
                event_subscribers.clone(),
                subscriber_to_transactions.clone(),
                transaction_to_subscribers.clone(),
            ),
        );
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));

        let manager = Self {
            transport,
            events_tx,
            event_subscribers,
            client_transactions,
            transaction_admissions: TransactionAdmissionRegistry::new(),
            admission_lifecycle: TransactionManagerAdmissionLifecycle::new(),
            operation_cancellation: TransactionManagerOperationCancellation::new(),
            shutdown_gate: Arc::new(Mutex::new(())),
            transaction_index_logical_capacity: index_capacity,
            transaction_index_initial_capacity: index_initial_capacity,
            client_completions: Arc::new(DashMap::new()),
            client_completion_deadlines: Arc::new(std::sync::Mutex::new(
                ClientCompletionDeadlineScheduler::default(),
            )),
            client_completion_capacity: retired_client_transaction_capacity(index_capacity),
            retained_client_deadline_worker: None,
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_initial_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::new()),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(
                index_initial_capacity,
            )),
            server_invite_dialog_expiry_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            server_invite_dialog_deadline_generation: Arc::new(AtomicU64::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::new()),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(
                Invite2xxDeadlineScheduler::default(),
            )),
            explicit_termination_operations: Arc::new(DashMap::new()),
            terminated_cleanup_tx: None,
            terminated_cleanup_shutdown: None,
            lifecycle_scheduler: None,
            compact_non_invite_tombstones: Arc::new(DashMap::new()),
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            timer_manager,
            timer_settings,
            running,
            transaction_destinations,
            retired_client_transaction_capacity: Arc::new(AtomicUsize::new(
                retired_client_transaction_capacity(index_capacity),
            )),
            retired_client_transaction_count: Arc::new(AtomicUsize::new(0)),
            retired_client_deadlines: Arc::new(std::sync::Mutex::new(
                RetiredClientDeadlineScheduler::default(),
            )),
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Some(Arc::new(Mutex::new(transport_rx))),
            control_transport_rx: None,
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            request_ingress_authorizer: None,
            pending_inbound_principals: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_principal_inserted_at: Arc::new(dashmap::DashMap::with_capacity(
                index_initial_capacity,
            )),
            pending_inbound_principal_generation: Arc::new(AtomicU64::new(0)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: 10,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };
        manager.install_terminal_delivery_failure_hook();
        manager
    }

    /// Sends a request through a client transaction.
    ///
    /// This method initiates a client transaction by sending its request
    /// according to the transaction state machine rules in RFC 3261.
    /// It triggers the transition from Initial to Calling (for INVITE)
    /// or Trying (for non-INVITE) state.
    ///
    /// ## Transaction State Transition
    ///
    /// This method triggers the following state transitions:
    /// - INVITE client: Initial → Calling
    /// - Non-INVITE client: Initial → Trying
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction initiation
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction initiation
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the client transaction to send
    ///
    /// # Returns
    /// * `Result<()>` - Success or error if the transaction cannot be sent
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::net::SocketAddr;
    /// # use std::str::FromStr;
    /// # use rvoip_sip_core::Request;
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # use rvoip_sip_dialog::transaction::TransactionKey;
    /// # async fn example(manager: &TransactionManager, request: Request) -> Result<(), Box<dyn std::error::Error>> {
    /// let destination = SocketAddr::from_str("192.168.1.100:5060")?;
    ///
    /// // First, create a client transaction
    /// let tx_id = manager.create_client_transaction(request, destination).await?;
    ///
    /// // Then, send the request through the transaction
    /// manager.send_request(&tx_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_request(&self, transaction_id: &TransactionKey) -> Result<()> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped request send".into()))
            }
            result = self.send_request_within_operation(transaction_id) => result,
        }
    }

    async fn send_request_within_operation(&self, transaction_id: &TransactionKey) -> Result<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "TransactionManager::send_request - sending request");

        // Clone the Arc<dyn ClientTransaction> out of the shard so we
        // don't hold the DashMap guard across `initiate().await` —
        // previously a global mutex pinned the whole client_transactions
        // map for the duration of one in-flight send.
        let tx_arc = self
            .client_transactions
            .get(transaction_id)
            .map(|r| r.value().clone());
        let Some(tx) = tx_arc else {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "TransactionManager::send_request - transaction not found");
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "send_request - transaction not found",
            ));
        };
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), kind=?tx.kind(), state=?tx.state(), "TransactionManager::send_request - found transaction");

        // Use the TransactionExt trait to safely downcast
        use crate::transaction::client::TransactionExt;

        if let Some(client_tx) = tx.as_client_transaction() {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "TransactionManager::send_request - initiating client transaction");

            // We're holding a per-tx Arc; the DashMap shard guard
            // released when `.get()` returned above.
            let result = client_tx.initiate().await;
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), success=?result.is_ok(), "TransactionManager::send_request - initiate result");

            if let Err(e) = result {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "TransactionManager::send_request - initiate failed immediately");
                return Err(e);
            }

            // The transport returns the selected opaque flow as part of the
            // successful send. Persist it before draining any queued response
            // events so an immediate peer close cannot erase the identity we
            // need to authenticate that response.
            let bound_route = client_tx.data().request_route.lock().await.clone();
            if let Some(mut state) = self.transaction_destinations.get_mut(transaction_id) {
                if let ClientResponseRouteState::Active { route, .. } = state.value_mut() {
                    *route = bound_route;
                }
            }

            Ok(())
        } else {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "TransactionManager::send_request - failed to downcast to client transaction");
            Err(Error::Other(
                "Failed to downcast to client transaction".to_string(),
            ))
        }
    }

    /// Sends a response through a server transaction.
    ///
    /// This method sends a SIP response through an existing server transaction,
    /// which will handle retransmissions and state transitions according to
    /// RFC 3261 rules.
    ///
    /// ## Transaction State Transitions
    ///
    /// This method can trigger the following state transitions:
    /// - INVITE server with provisional response: Proceeding → Proceeding
    /// - INVITE server with final response: Proceeding → Completed
    /// - Non-INVITE server with provisional response: Trying/Proceeding → Proceeding
    /// - Non-INVITE server with final response: Trying/Proceeding → Completed
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.2.1: INVITE server transaction response handling
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction response handling
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the server transaction
    /// * `response` - The SIP response to send
    ///
    /// # Returns
    /// * `Result<()>` - Success or error if the response cannot be sent
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_core::{Response, StatusCode};
    /// # use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # use rvoip_sip_dialog::transaction::TransactionKey;
    /// # async fn example(
    /// #    manager: &TransactionManager,
    /// #    tx_id: &TransactionKey,
    /// #    request: &rvoip_sip_core::Request
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a 200 OK response
    /// let response = SimpleResponseBuilder::response_from_request(
    ///     request,
    ///     StatusCode::Ok,
    ///     Some("OK")
    /// ).build();
    ///
    /// // Send the response through the transaction
    /// manager.send_response(tx_id, response).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped response send".into()))
            }
            result = self.send_response_within_operation(transaction_id, response) => result,
        }
    }

    /// Classify the authoritative completion of the final response generation
    /// currently owned by an exact server transaction.
    ///
    /// This is intentionally generation-scoped. If the original API waiter was
    /// cancelled after runner admission, a replacement waiter observes the same
    /// runner-owned write rather than either losing completion or following a
    /// later retry generation. Absence of the transaction is terminal because
    /// the exact generation can no longer be retried safely.
    pub(crate) async fn classify_final_response_completion(
        &self,
        transaction_id: &TransactionKey,
    ) -> crate::transaction::server::FinalResponseCompletionDisposition {
        use crate::transaction::server::FinalResponseCompletionDisposition;

        let transaction = self
            .server_transactions
            .get(transaction_id)
            .map(|entry| Arc::clone(entry.value()));
        let Some(transaction) = transaction else {
            return FinalResponseCompletionDisposition::WireUnknownErrorTerminal;
        };
        let data = transaction.data();
        let Some(generation) = data.current_final_response_supervision_generation() else {
            return FinalResponseCompletionDisposition::ZeroWireRetryable;
        };
        data.await_final_response_completion_for_generation(generation)
            .await
    }

    async fn send_response_within_operation(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        rvoip_sip_core::validation::validate_wire_response(&response)?;
        let is_200_ok = response.status().as_u16() == 200;

        // Same pattern as send_request: extract the Arc<dyn ServerTransaction>
        // from the DashMap shard before awaiting `send_response()`.
        let tx_arc = self
            .server_transactions
            .get(transaction_id)
            .map(|r| r.value().clone());
        let Some(tx) = tx_arc else {
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "send_response - transaction not found",
            ));
        };

        use crate::transaction::server::TransactionExt;
        if let Some(server_tx) = tx.as_server_transaction() {
            let original_method = if is_200_ok {
                server_tx
                    .original_request_sync()
                    .map(|request| request.method().clone())
            } else {
                None
            };
            let result = server_tx.send_response(response).await;
            if result.is_ok() {
                self.cache_invite_2xx_response_for(transaction_id).await;
                if is_200_ok && !matches!(original_method, Some(Method::Invite) | Some(Method::Bye))
                {
                    diagnostics::record_200_ok_other();
                }
            }
            result
        } else {
            Err(Error::Other(
                "Failed to downcast to server transaction".to_string(),
            ))
        }
    }

    /// Recover the natural non-INVITE Completed -> Timer J lifecycle after a
    /// caller was cancelled between the exact final transport write and the
    /// Completed command enqueue. Returns `false` only when no final response
    /// was proven on wire, so the caller may force-terminate that generation.
    pub(crate) async fn recover_bye_final_response_lifecycle(
        &self,
        transaction_id: &TransactionKey,
    ) -> bool {
        if self
            .compact_non_invite_tombstones
            .contains_key(transaction_id)
        {
            return true;
        }
        let transaction = self
            .server_transactions
            .get(transaction_id)
            .map(|entry| entry.value().clone());
        let Some(transaction) = transaction else {
            // This API is called only for an already-admitted BYE generation.
            // Absence from both active and compact indexes proves that its
            // lifecycle has already retired; it is not an active pre-wire leak.
            return true;
        };
        let data = transaction.data();
        if data.request.method() != Method::Bye {
            return false;
        }
        let wire_outcome = tokio::time::timeout(
            BYE_FINAL_RESPONSE_RECOVERY_TIMEOUT,
            data.await_final_response_wire_outcome(),
        )
        .await;
        if !matches!(wire_outcome, Ok(true)) {
            return false;
        }
        if matches!(
            data.state.get(),
            TransactionState::Completed | TransactionState::Terminated
        ) {
            return true;
        }
        // Subscribe before enqueue so a fast runner cannot advance Completed
        // between the state check and watch registration. This is an exact
        // state-cell wait, not the removed global event subscription/polling
        // path.
        let state = Arc::clone(&data.state);
        let mut state_changes = state.subscribe();
        if data
            .cmd_tx
            .send(InternalTransactionCommand::TransitionTo(
                TransactionState::Completed,
            ))
            .await
            .is_err()
        {
            return self
                .compact_non_invite_tombstones
                .contains_key(transaction_id)
                || self
                    .server_transactions
                    .get(transaction_id)
                    .map_or(true, |transaction| {
                        matches!(
                            transaction.value().state(),
                            TransactionState::Completed | TransactionState::Terminated
                        )
                    });
        }
        let deadline = tokio::time::Instant::now() + BYE_FINAL_RESPONSE_RECOVERY_TIMEOUT;
        loop {
            if matches!(
                state.get(),
                TransactionState::Completed | TransactionState::Terminated
            ) || self
                .compact_non_invite_tombstones
                .contains_key(transaction_id)
                || !self.server_transactions.contains_key(transaction_id)
            {
                return true;
            }
            match tokio::time::timeout_at(deadline, state_changes.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    return matches!(
                        state.get(),
                        TransactionState::Completed | TransactionState::Terminated
                    ) || self
                        .compact_non_invite_tombstones
                        .contains_key(transaction_id)
                        || !self.server_transactions.contains_key(transaction_id);
                }
            }
        }
    }

    pub(crate) fn bye_final_response_may_have_reached_wire(
        &self,
        transaction_id: &TransactionKey,
    ) -> bool {
        self.compact_non_invite_tombstones
            .contains_key(transaction_id)
            || self
                .server_transactions
                .get(transaction_id)
                .is_some_and(|transaction| {
                    let data = transaction.value().data();
                    data.request.method() == Method::Bye
                        && data.final_response_may_have_reached_wire()
                })
    }

    /// Checks if a transaction with the given ID exists.
    ///
    /// This method looks for the transaction in both client and server
    /// transaction collections. It's useful for verifying that a transaction
    /// exists before attempting operations on it.
    ///
    /// # Arguments
    /// * `transaction_id` - The transaction ID to check
    ///
    /// # Returns
    /// * `bool` - True if the transaction exists, false otherwise
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # use rvoip_sip_dialog::transaction::TransactionKey;
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) {
    /// if manager.transaction_exists(tx_id).await {
    ///     println!("Transaction {} exists", tx_id);
    /// } else {
    ///     println!("Transaction {} not found", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn transaction_exists(&self, transaction_id: &TransactionKey) -> bool {
        self.client_transactions.contains_key(transaction_id)
            || self.server_transactions.contains_key(transaction_id)
            || self
                .compact_non_invite_tombstones
                .contains_key(transaction_id)
    }

    /// Gets the current state of a transaction.
    ///
    /// This method retrieves the current state of a transaction according to
    /// the state machines defined in RFC 3261. The state determines what
    /// operations are valid and how the transaction will respond to messages.
    ///
    /// ## Transaction States
    ///
    /// The possible states are:
    /// - **Initial**: Transaction created but not started
    /// - **Calling**: INVITE client waiting for response
    /// - **Trying**: Non-INVITE client waiting for response
    /// - **Proceeding**: Received provisional response, waiting for final
    /// - **Completed**: Received final response, waiting for reliability
    /// - **Terminated**: Transaction is done
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1: INVITE client transaction states
    /// - RFC 3261 Section 17.1.2: Non-INVITE client transaction states
    /// - RFC 3261 Section 17.2.1: INVITE server transaction states
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction states
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionState>` - The transaction state or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # use rvoip_sip_dialog::transaction::{TransactionKey, TransactionState};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let state = manager.transaction_state(tx_id).await?;
    ///
    /// match state {
    ///     TransactionState::Proceeding => println!("Transaction is in Proceeding state"),
    ///     TransactionState::Completed => println!("Transaction is in Completed state"),
    ///     TransactionState::Terminated => println!("Transaction is terminated"),
    ///     _ => println!("Transaction is in state: {:?}", state),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_state(
        &self,
        transaction_id: &TransactionKey,
    ) -> Result<TransactionState> {
        if let Some(entry) = self.client_transactions.get(transaction_id) {
            return Ok(entry.value().state());
        }
        if let Some(entry) = self.server_transactions.get(transaction_id) {
            return Ok(entry.value().state());
        }
        if let Some(state) = self.compact_non_invite_state(transaction_id) {
            return Ok(state.get());
        }
        Err(Error::transaction_not_found(
            transaction_id.clone(),
            "transaction_state - transaction not found",
        ))
    }

    /// Gets the transaction type (kind) for the specified transaction.
    ///
    /// This method returns the type of transaction as defined in RFC 3261:
    /// - INVITE client transaction (ICT)
    /// - Non-INVITE client transaction (NICT)
    /// - INVITE server transaction (IST)
    /// - Non-INVITE server transaction (NIST)
    ///
    /// Knowing the transaction kind is important because each type follows
    /// different state machines and behavior rules.
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17: Four transaction types with different state machines
    /// - RFC 3261 Section 17.1: Client transaction types
    /// - RFC 3261 Section 17.2: Server transaction types
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionKind>` - The transaction kind or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # use rvoip_sip_dialog::transaction::{TransactionKey, TransactionKind};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let kind = manager.transaction_kind(tx_id).await?;
    ///
    /// match kind {
    ///     TransactionKind::InviteClient => println!("This is an INVITE client transaction"),
    ///     TransactionKind::NonInviteClient => println!("This is a non-INVITE client transaction"),
    ///     TransactionKind::InviteServer => println!("This is an INVITE server transaction"),
    ///     TransactionKind::NonInviteServer => println!("This is a non-INVITE server transaction"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_kind(
        &self,
        transaction_id: &TransactionKey,
    ) -> Result<TransactionKind> {
        if let Some(entry) = self.client_transactions.get(transaction_id) {
            return Ok(entry.value().kind());
        }
        if let Some(entry) = self.server_transactions.get(transaction_id) {
            return Ok(entry.value().kind());
        }
        if let Some(entry) = self.compact_non_invite_tombstones.get(transaction_id) {
            return Ok(if entry.value().is_client() {
                TransactionKind::NonInviteClient
            } else {
                TransactionKind::NonInviteServer
            });
        }
        Err(Error::transaction_not_found(
            transaction_id.clone(),
            "transaction kind lookup failed",
        ))
    }

    /// Gets a list of all active transaction IDs.
    ///
    /// This method returns separate lists of client and server transaction IDs,
    /// which can be useful for monitoring, debugging, or cleanup operations.
    ///
    /// # Returns
    /// * `(Vec<TransactionKey>, Vec<TransactionKey>)` - Client and server transaction IDs
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_dialog::transaction::TransactionManager;
    /// # async fn example(manager: &TransactionManager) {
    /// let (client_txs, server_txs) = manager.active_transactions().await;
    ///
    /// println!("Active client transactions: {}", client_txs.len());
    /// println!("Active server transactions: {}", server_txs.len());
    ///
    /// // Process each transaction ID
    /// for tx_id in client_txs {
    ///     println!("Client transaction: {}", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn active_transactions(&self) -> (Vec<TransactionKey>, Vec<TransactionKey>) {
        (
            self.client_transactions
                .iter()
                .map(|r| r.key().clone())
                .collect(),
            self.server_transactions
                .iter()
                .map(|r| r.key().clone())
                .collect(),
        )
    }

    /// Gets a reference to the transport layer used by this transaction manager.
    ///
    /// This method provides access to the underlying transport layer,
    /// which can be useful for operations outside the transaction layer.
    ///
    /// # Returns
    /// * `Arc<dyn Transport>` - The transport layer
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    /// Look up the destination `SocketAddr` a given transaction's
    /// request was originally sent to. Used by dialog-layer features
    /// that need to talk to the same peer outside the transaction
    /// envelope — e.g., RFC 5626 §3.5.1 CRLFCRLF keep-alive pings
    /// target the REGISTER's destination.
    pub async fn transaction_destination(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<SocketAddr> {
        self.with_client_response_route_state(transaction_id, |state| state.route().destination)
    }

    /// Return the authority- and flow-bearing route used by a client transaction.
    pub async fn transaction_route(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<TransportRoute> {
        self.with_client_response_route_state(transaction_id, |state| state.route().clone())
    }

    /// Read one active/retired response-route record without cloning the
    /// complete retired request tombstone. Expired records are removed only
    /// when their exact deadline generation is still authoritative; a
    /// concurrent replacement is retried instead of being reported missing.
    fn with_client_response_route_state<T>(
        &self,
        transaction_id: &TransactionKey,
        read: impl Fn(&ClientResponseRouteState) -> T,
    ) -> Option<T> {
        loop {
            let state = self.transaction_destinations.get(transaction_id)?;
            let Some(retired) = state.retired() else {
                return Some(read(state.value()));
            };
            if retired.expires_at > Instant::now() {
                return Some(read(state.value()));
            }
            let expires_at = retired.expires_at;
            let deadline_version = retired.deadline_version;
            drop(state);
            if self
                .transaction_destinations
                .remove_if(transaction_id, |_, current| {
                    current.retired().is_some_and(|retired| {
                        retired.deadline_version == deadline_version
                            && retired.expires_at == expires_at
                            && retired.expires_at <= Instant::now()
                    })
                })
                .is_some()
            {
                self.decrement_retired_client_transaction_count();
                self.unschedule_retired_client_deadline(
                    transaction_id,
                    expires_at,
                    deadline_version,
                );
                return None;
            }
            // A newer generation replaced the expired value after our read.
            // Resolve that exact current generation instead of exposing a
            // transient unknown-transaction result.
        }
    }

    fn retired_client_request_wire(&self, transaction_id: &TransactionKey) -> Option<bytes::Bytes> {
        self.with_client_response_route_state(transaction_id, |state| {
            state.retired().map(|retired| retired.request_wire.clone())
        })
        .flatten()
    }

    fn retired_client_original_request(
        &self,
        transaction_id: &TransactionKey,
    ) -> Result<Option<Request>> {
        self.retired_client_request_wire(transaction_id)
            .map(|wire| RetiredClientTransaction::original_request_from_wire(wire.as_ref()))
            .transpose()
    }

    #[cfg(test)]
    fn retired_client_transaction(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<RetiredClientTransaction> {
        self.with_client_response_route_state(transaction_id, |state| state.retired().cloned())
            .flatten()
    }

    fn install_client_completion(
        &self,
        transaction_id: Arc<TransactionKey>,
        transaction: &ArcClientTransaction,
    ) {
        let completion = transaction.data().completion.clone();
        if let Some(previous) = self.client_completions.insert(
            Arc::clone(&transaction_id),
            ClientTransactionCompletionEntry::Active(completion),
        ) {
            if let Some((expires_at, version)) = previous.retained_deadline() {
                self.client_completion_deadlines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .unschedule(transaction_id.as_ref(), expires_at, version);
            }
        }
    }

    /// Resolve an exact completion cell without touching the global deadline
    /// queue on the ordinary active/retained read path. A waiter that wins the
    /// expiry race keeps its cloned Arc; a new lookup lazily removes only its
    /// own expired generation.
    fn client_completion(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<ClientTransactionCompletionEntry> {
        let completion = self
            .client_completions
            .get(transaction_id)
            .map(|entry| entry.value().clone());
        let Some(completion) = completion else {
            // UDP Timer K already owns the exact completion Arc and its
            // authoritative expiry. Avoid duplicating the transaction key,
            // wire response, and deadline in the manager retention maps.
            let compact = self
                .compact_non_invite_tombstones
                .get(transaction_id)
                .filter(|entry| entry.value().expires_at() > Instant::now())
                .and_then(|entry| entry.value().client_completion().cloned())
                .map(ClientTransactionCompletionEntry::Retained);
            if compact.is_some() {
                return compact;
            }
            return self
                .with_client_response_route_state(transaction_id, |state| {
                    state.retired().map(|retired| retired.completion.clone())
                })
                .flatten()
                .map(ClientTransactionCompletionEntry::Retained);
        };
        if !completion.is_expired(Instant::now()) {
            return Some(completion);
        }

        let Some((expires_at, version)) = completion.retained_deadline() else {
            return Some(completion);
        };
        if self
            .client_completions
            .remove_if(transaction_id, |_, current| {
                current.retained_deadline() == Some((expires_at, version))
                    && current.is_expired(Instant::now())
            })
            .is_some()
        {
            self.client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .unschedule(transaction_id, expires_at, version);
        }
        None
    }

    /// Keep the exact result cell available after the transaction runner and
    /// route are removed. This is intentionally separate from INVITE late-2xx
    /// route retention because BYE, MESSAGE, and authentication retries also
    /// need race-free completion observation.
    async fn client_completion_retention(
        &self,
        transaction_id: &TransactionKey,
        transaction: &ArcClientTransaction,
    ) -> (Instant, bool) {
        if transaction_id.method() == &Method::Invite {
            return (Instant::now() + CLIENT_TRANSACTION_COMPLETION_TTL, false);
        }

        // A backed-up dialog shard may consume a losslessly delivered 401/407
        // after the ordinary reliable-transport race grace. Preserve only
        // challenge-bearing completions for the full retry horizon; ordinary
        // TCP/TLS non-INVITEs still retire after the one-second grace.
        if transaction
            .data()
            .completion
            .has_auth_challenge_request_uri()
        {
            return (Instant::now() + CLIENT_TRANSACTION_COMPLETION_TTL, false);
        }

        // A UDP non-INVITE runner is replaced by the compact Timer K
        // tombstone before cleanup. Use that exact deadline so the completion
        // cannot outlive the RFC retransmission-absorption window.
        if let Some(expires_at) = self
            .compact_non_invite_tombstones
            .get(transaction_id)
            .filter(|entry| entry.value().is_client())
            .map(|entry| entry.value().expires_at())
        {
            return (expires_at, true);
        }

        let route = transaction.data().request_route.lock().await;
        if crate::transaction::timer_utils::uses_unreliable_transport(
            &route,
            self.transport.default_transport_type(),
        ) {
            (
                Instant::now() + transaction.data().timer_config.wait_time_k,
                false,
            )
        } else {
            (
                Instant::now() + RELIABLE_CLIENT_COMPLETION_RACE_GRACE,
                false,
            )
        }
    }

    fn retain_client_completion(
        &self,
        transaction_id: &TransactionKey,
        expires_at: Instant,
        keep_live_until_expiry: bool,
        admission_owner: Option<TransactionAdmissionOwner>,
    ) {
        let Some((completion, shared_transaction_id)) = self
            .client_completions
            .get(transaction_id)
            .and_then(|entry| match entry.value() {
                ClientTransactionCompletionEntry::Active(completion) => {
                    Some((Arc::clone(completion), Arc::clone(entry.key())))
                }
                ClientTransactionCompletionEntry::Retained(_) => None,
            })
        else {
            return;
        };

        if keep_live_until_expiry {
            // The compact Timer K tombstone already owns an immutable exact
            // completion and a weak bridge to this cell. Remove the duplicate
            // manager-map owner. Existing waiters keep their Arc; new waiters
            // resolve the immutable tombstone through `client_completion`.
            self.client_completions
                .remove_if(transaction_id, |_, current| {
                    matches!(
                        current,
                        ClientTransactionCompletionEntry::Active(active)
                            if Arc::ptr_eq(active, &completion)
                    )
                });
            return;
        }

        let version = self
            .client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_version(expires_at);
        // Response serialization is transaction-local and can allocate. Keep
        // it outside the shared deadline critical section so unrelated
        // completions retire concurrently.
        let retained = completion
            .retained(expires_at, version)
            .with_admission_owner(admission_owner);
        let mut deadlines = self
            .client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let previous_wake = deadlines.next_wake_at(now, self.client_completion_capacity);
        let installed = self.client_completions.get_mut(transaction_id).is_some_and(
            |mut current| match current.value() {
                ClientTransactionCompletionEntry::Active(active)
                    if Arc::ptr_eq(active, &completion) =>
                {
                    *current = ClientTransactionCompletionEntry::Retained(retained);
                    true
                }
                ClientTransactionCompletionEntry::Active(_)
                | ClientTransactionCompletionEntry::Retained(_) => false,
            },
        );
        if installed {
            deadlines.schedule(shared_transaction_id, expires_at, version);
        }
        let should_wake = installed
            && deadlines.next_wake_at(now, self.client_completion_capacity) != previous_wake;
        drop(deadlines);
        if should_wake {
            self.wake_retained_client_deadline_worker();
        }
    }

    /// Replace the active route in-place with a tombstone built from the
    /// transaction's exact post-send route. The live transaction Arc remains
    /// published until this transition completes, so response authentication
    /// always observes either Active or Retired for a successfully sent
    /// INVITE.
    async fn retire_client_transaction(
        &self,
        transaction_id: &TransactionKey,
        transaction: &ArcClientTransaction,
    ) -> bool {
        // Completed UDP non-INVITE transactions have already published a
        // compact Timer K tombstone. Keep the exact active response route
        // through T4 so retransmitted final responses remain authenticated;
        // the due scheduler removes both records atomically at expiry.
        if self
            .compact_non_invite_tombstones
            .get(transaction_id)
            .is_some_and(|entry| entry.value().is_client())
        {
            return false;
        }

        // Only INVITE can legitimately produce dialog-forming 2xx responses
        // after its client transaction has terminated. Other methods discard
        // their route immediately instead of broadening the retained surface.
        if transaction_id.is_server()
            || transaction_id.method() != &Method::Invite
            || !transaction.data().initial_send_attempted()
        {
            self.transaction_destinations
                .remove_if(transaction_id, |_, state| state.is_active());
            return false;
        }

        let exact_route = transaction.data().request_route.lock().await.clone();
        let expires_at = Instant::now() + RETIRED_CLIENT_TRANSACTION_TTL;
        // Reserve a unique deadline identity, then serialize the request and
        // exact completion together outside the shared deadline critical
        // section. Skipped versions are harmless; publishing a record and its
        // deadline remains one linearizable mutation below.
        let deadline_version = self
            .retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_version(expires_at);
        let retired = RetiredClientTransaction::new(
            transaction.data().request.as_ref(),
            transaction.data().completion.as_ref(),
            exact_route,
            expires_at,
            deadline_version,
            transaction.data().transaction_admission_owner(),
        );
        let mut transitioned = false;
        // Scheduling and the Active -> Retired mutation share one short
        // critical section. Maintenance can therefore never observe a newly
        // retired route without its exact deadline generation.
        let mut deadlines = self
            .retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let retired_capacity = self
            .retired_client_transaction_capacity
            .load(Ordering::Acquire);
        let previous_wake = deadlines.next_wake_at(now, retired_capacity);
        if let Some(mut state) = self.transaction_destinations.get_mut(transaction_id) {
            if state.is_active() {
                let shared_transaction_id = Arc::clone(state.key());
                *state = ClientResponseRouteState::Retired(retired);
                deadlines.schedule(shared_transaction_id, expires_at, deadline_version);
                self.retired_client_transaction_count
                    .fetch_add(1, Ordering::AcqRel);
                transitioned = true;
            }
        }
        let should_wake =
            transitioned && deadlines.next_wake_at(now, retired_capacity) != previous_wake;
        drop(deadlines);
        if should_wake {
            self.wake_retained_client_deadline_worker();
        }
        transitioned
    }

    async fn retire_and_remove_client_transaction(&self, transaction_id: &TransactionKey) -> bool {
        let Some(transaction) = self
            .client_transactions
            .get(transaction_id)
            .map(|entry| entry.value().clone())
        else {
            return false;
        };

        let transitioned = self
            .retire_client_transaction(transaction_id, &transaction)
            .await;

        // Active -> Retired is the linearization point for a successfully
        // sent INVITE. If another cleanup caller already performed that
        // transition, only that owner may remove the live transaction Arc.
        // Otherwise the second caller can remove it while the owner is still
        // publishing the tombstone/deadline, exposing a transient unknown
        // authentication window for a late 2xx response.
        if !transitioned
            && !transaction_id.is_server()
            && transaction_id.method() == &Method::Invite
            && transaction.data().initial_send_attempted()
            && self
                .transaction_destinations
                .get(transaction_id)
                .is_some_and(|state| state.retired().is_some())
        {
            return false;
        }

        if transitioned {
            let completion = &transaction.data().completion;
            self.client_completions
                .remove_if(transaction_id, |_, current| {
                    matches!(
                        current,
                        ClientTransactionCompletionEntry::Active(active)
                            if Arc::ptr_eq(active, completion)
                    )
                });
        } else {
            let (completion_expires_at, keep_live_until_expiry) = self
                .client_completion_retention(transaction_id, &transaction)
                .await;
            self.retain_client_completion(
                transaction_id,
                completion_expires_at,
                keep_live_until_expiry,
                transaction.data().transaction_admission_owner(),
            );
        }

        #[cfg(test)]
        if transitioned {
            let gate = RETIRED_CLIENT_TRANSITION_TEST_GATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .filter(|gate| gate.transaction_id == *transaction_id);
            if let Some(gate) = gate {
                gate.transitioned.notify_one();
                gate.release.notified().await;
            }
        }

        let removed = self
            .client_transactions
            .remove_if(transaction_id, |_, current| {
                Arc::ptr_eq(current, &transaction)
            })
            .is_some();

        if !transitioned {
            return removed;
        }

        if self
            .retired_client_transaction_count
            .load(Ordering::Acquire)
            > self
                .retired_client_transaction_capacity
                .load(Ordering::Acquire)
        {
            self.prune_retired_client_transactions();
        }
        removed
    }

    fn unschedule_retired_client_deadline(
        &self,
        transaction_id: &TransactionKey,
        expires_at: Instant,
        deadline_version: u64,
    ) {
        self.retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unschedule(transaction_id, expires_at, deadline_version);
    }

    #[cfg(test)]
    fn reschedule_retired_client_deadline_for_test(
        &self,
        transaction_id: &TransactionKey,
        expires_at: Instant,
    ) -> bool {
        let mut deadlines = self
            .retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline_version = deadlines.next_version(expires_at);
        let Some(mut state) = self.transaction_destinations.get_mut(transaction_id) else {
            return false;
        };
        let shared_transaction_id = Arc::clone(state.key());
        let ClientResponseRouteState::Retired(retired) = state.value_mut() else {
            return false;
        };
        deadlines.unschedule(transaction_id, retired.expires_at, retired.deadline_version);
        retired.expires_at = expires_at;
        retired.deadline_version = deadline_version;
        retired
            .completion
            .set_deadline(expires_at, deadline_version);
        deadlines.schedule(shared_transaction_id, expires_at, deadline_version);
        drop(deadlines);
        drop(state);
        self.wake_retained_client_deadline_worker();
        true
    }

    #[cfg(test)]
    fn install_retired_client_transition_test_gate(
        &self,
        transaction_id: TransactionKey,
        transitioned: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *RETIRED_CLIENT_TRANSITION_TEST_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(RetiredClientTransitionTestGate {
                transaction_id,
                transitioned,
                release,
            });
    }

    #[cfg(test)]
    fn clear_retired_client_transition_test_gate(&self) {
        *RETIRED_CLIENT_TRANSITION_TEST_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    #[cfg(test)]
    fn install_termination_takeover_test_gate(
        &self,
        transaction_id: TransactionKey,
        runner_joined: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *TERMINATION_TAKEOVER_TEST_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(TerminationTakeoverTestGate {
            transaction_id,
            runner_joined,
            release,
        });
    }

    #[cfg(test)]
    fn clear_termination_takeover_test_gate(&self) {
        *TERMINATION_TAKEOVER_TEST_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Subscribe to events from all transactions.
    ///
    /// This method creates a new subscription to all transaction events.
    /// The returned receiver will get all events regardless of which transaction
    /// generated them. This is useful for monitoring or logging all transaction activity.
    ///
    /// # Returns
    /// * `mpsc::Receiver<TransactionEvent>` - The event receiver
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        let (tx, rx) = mpsc::channel(100);
        let Some(_operation) = self.admission_lifecycle.try_enter_existing() else {
            return rx;
        };
        let id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        // ArcSwap RCU.
        self.event_subscribers.rcu(|current| {
            let mut next = Vec::with_capacity(current.len() + 1);
            next.extend(current.iter().cloned());
            next.push(EventSubscriber::new(id, tx.clone(), true));
            next
        });

        debug!("Added global subscriber with ID {}", id);

        rx
    }

    /// Subscribe to events from a specific transaction.
    ///
    /// This method creates a subscription to events from a single transaction.
    /// The returned receiver will only get events from the specified transaction,
    /// reducing noise and unnecessary event processing.
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transaction(
        &self,
        transaction_id: &TransactionKey,
    ) -> Result<mpsc::Receiver<TransactionEvent>> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        // Capture an allocation/generation authority, not only the bare SIP
        // key. Timer J/K cleanup may permit that key to be reused while this
        // method is inserting its two observer indexes.
        let Some(authority) = self.transaction_subscription_authority(transaction_id) else {
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "subscribe_to_transaction - transaction not found or terminal",
            ));
        };

        let (tx, rx) = mpsc::channel(100);

        let subscriber_id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        let subscriber = EventSubscriber::new(subscriber_id, tx, false);
        let observers = self
            .events_tx
            .observer_fanout()
            .expect("transaction manager installs observer fanout");
        observers.add_transaction_subscriber(transaction_id.clone(), subscriber);

        // Insert then revalidate. Compact expiry sets the tombstone state to
        // Terminated before taking its terminal observer snapshot, so either
        // this subscriber is included in that snapshot or it removes only its
        // own indexes and reports the race. It can never attach to a later
        // same-key transaction generation.
        if self.transaction_subscription_authority(transaction_id) != Some(authority) {
            observers.remove_subscriber(subscriber_id);
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "subscribe_to_transaction - transaction became terminal or was replaced",
            ));
        }

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), subscriber_id, "Added transaction-specific subscriber");

        Ok(rx)
    }

    /// Subscribe to events from multiple transactions.
    ///
    /// This method creates a subscription to events from multiple transactions.
    /// The returned receiver will only get events from the specified transactions.
    ///
    /// # Arguments
    /// * `transaction_ids` - The IDs of the transactions to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transactions(
        &self,
        transaction_ids: &[TransactionKey],
    ) -> Result<mpsc::Receiver<TransactionEvent>> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        // Capture exact authorities before mutating either observer index.
        let mut authorities = Vec::with_capacity(transaction_ids.len());
        for tx_id in transaction_ids {
            if let Some(authority) = self.transaction_subscription_authority(tx_id) {
                authorities.push((tx_id.clone(), authority));
            } else {
                return Err(Error::transaction_not_found(
                    tx_id.clone(),
                    "subscribe_to_transactions - transaction not found or terminal",
                ));
            }
        }

        let (tx, rx) = mpsc::channel(100);

        let subscriber_id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        let subscriber = EventSubscriber::new(subscriber_id, tx, false);
        let observers = self
            .events_tx
            .observer_fanout()
            .expect("transaction manager installs observer fanout");

        for tx_id in transaction_ids {
            observers.add_transaction_subscriber(tx_id.clone(), subscriber.clone());
        }

        if authorities.iter().any(|(transaction_id, authority)| {
            self.transaction_subscription_authority(transaction_id) != Some(authority.clone())
        }) {
            observers.remove_subscriber(subscriber_id);
            return Err(Error::Other(
                "one or more transaction subscriptions became terminal or were replaced".into(),
            ));
        }

        debug!(
            subscriber_id,
            transaction_count = transaction_ids.len(),
            "Added multi-transaction subscriber"
        );

        Ok(rx)
    }

    fn transaction_subscription_authority(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<TransactionSubscriptionAuthority> {
        if let Some(tombstone) = self.compact_non_invite_tombstones.get(transaction_id) {
            return (tombstone.value().state().get() != TransactionState::Terminated).then(|| {
                TransactionSubscriptionAuthority::Compact(tombstone.value().generation())
            });
        }
        if let Some(transaction) = self.client_transactions.get(transaction_id) {
            if transaction.value().state() == TransactionState::Terminated {
                return None;
            }
            return Some(TransactionSubscriptionAuthority::Client(
                transaction.value().clone(),
            ));
        }
        self.server_transactions
            .get(transaction_id)
            .and_then(|transaction| {
                if transaction.value().state() == TransactionState::Terminated {
                    return None;
                }
                Some(TransactionSubscriptionAuthority::Server(
                    transaction.value().clone(),
                ))
            })
    }

    /// Shutdown the transaction manager gracefully - BOTTOM-UP
    ///
    /// This performs a graceful shutdown in BOTTOM-UP order:
    /// 1. Close the transport layer (UDP) first
    /// 2. Stop the message processing loop
    /// 3. Drain any remaining messages
    /// 4. Clear active transactions
    /// 5. Clear event subscribers
    pub(crate) fn begin_shutdown_drain(&self) {
        self.admission_lifecycle.begin_draining();
        // Wake potentially unbounded sends/authorizers before an owning
        // dialog-layer operation fence is awaited. Existing fail-closed
        // cleanup remains admissible until `shutdown` advances to Stopping.
        self.operation_cancellation.cancel();
    }

    pub async fn shutdown(&self) {
        let _shutdown_guard = self.shutdown_gate.lock().await;
        if self.admission_lifecycle.state() == MANAGER_ADMISSION_STOPPED {
            return;
        }
        self.begin_shutdown_drain();
        self.admission_lifecycle.begin_stopping();
        // Drop every potentially unbounded transport-handler future (slow
        // external authorizer, saturated primary TU, blocked response send)
        // before waiting for the operation fence. Staged publication guards
        // run synchronously as those futures are cancelled.
        // Every creator holds its guard through map/index publication or
        // complete rollback, and every cleanup operation admitted during
        // fail-closed draining holds the same counter. Closing both gates
        // before waiting prevents late mutation after force-clear.
        self.admission_lifecycle.wait_idle().await;
        info!("TransactionManager shutting down gracefully");

        // Step 1: Stop the message processing loop FIRST.
        // AtomicBool — was an async Mutex<bool> before the perf pass.
        self.running.store(false, Ordering::Relaxed);
        debug!("Message processing loop signaled to stop");

        // Step 2: Transport should already be closed by this point via events
        // But ensure it's closed just in case
        if let Err(e) = self.transport.close().await {
            debug!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Transport close during shutdown");
        }

        // Step 2.5: Tell every in-flight transaction to terminate so its event
        // loop runs `cancel_all_specific_timers` and aborts pending timers NOW.
        // Otherwise the force-clear below merely drops the transaction `Arc`,
        // whose `Drop` aborts only the event-loop task; that detaches (does not
        // abort) the timer tasks, so a pending Timer B on an INVITE to a
        // non-responsive peer sleeps out its full ~64*T1 and holds the bound
        // port that long. `Terminate` drives the graceful path, which reaches
        // the `Destroyed` lifecycle in milliseconds.
        //
        // Collect `Arc` clones first so we never hold a DashMap shard guard
        // across the loop. Use `try_send` (never `.await`) so shutdown cannot
        // block if a transaction's event loop is momentarily not draining its
        // command channel; the wait-poll + force-clear below remain the fallback.
        let in_flight_clients: Vec<_> = self
            .client_transactions
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let in_flight_servers: Vec<_> = self
            .server_transactions
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        for tx in &in_flight_clients {
            let _ = tx
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate);
        }
        for tx in &in_flight_servers {
            let _ = tx
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate);
        }

        // Give every runner one bounded, manager-wide grace interval, then
        // abort and join any producer that did not exit. Joining is mandatory:
        // clearing maps while a runner still owns Data permits post-clear
        // events, wire writes, and timer registration.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut runner_handles =
            Vec::with_capacity(in_flight_clients.len() + in_flight_servers.len());
        for transaction in &in_flight_clients {
            if let Some(handle) = transaction.data().event_loop_handle.lock().await.take() {
                runner_handles.push(handle);
            }
        }
        for transaction in &in_flight_servers {
            if let Some(handle) = transaction.data().event_loop_handle.lock().await.take() {
                runner_handles.push(handle);
            }
        }
        for handle in &runner_handles {
            if !handle.is_finished() {
                handle.abort();
            }
        }
        for handle in runner_handles {
            let _ = handle.await;
        }
        self.timer_manager.shutdown().await;

        // Stop the manager-owned due queue after active runners have received
        // their direct termination command. Pending grace entries are moved
        // straight to Destroyed and woken, so the scheduler cannot retain
        // transaction data after manager shutdown.
        if let Some(scheduler) = self.lifecycle_scheduler.as_ref() {
            scheduler.shutdown().await;
        }
        if let Some(worker) = self.retained_client_deadline_worker.as_ref() {
            worker.shutdown().await;
        }
        if let Some(shutdown) = self.terminated_cleanup_shutdown.as_ref() {
            shutdown.notify_one();
        }

        // Step 4: Wait for all transactions to reach Destroyed lifecycle state
        let client_count = self.client_transactions.len();
        let server_count = self.server_transactions.len();
        if client_count > 0 || server_count > 0 {
            debug!(
                "Waiting for {} client and {} server transactions to reach Destroyed state",
                client_count, server_count
            );

            // Give transactions time to process their lifecycle transitions
            let mut wait_iterations = 0;
            loop {
                // Check if all transactions have reached Destroyed state
                let mut all_destroyed = true;

                for entry in self.client_transactions.iter() {
                    if entry.value().data().get_lifecycle() != TransactionLifecycle::Destroyed {
                        all_destroyed = false;
                        break;
                    }
                }

                if all_destroyed {
                    for entry in self.server_transactions.iter() {
                        if entry.value().data().get_lifecycle() != TransactionLifecycle::Destroyed {
                            all_destroyed = false;
                            break;
                        }
                    }
                }

                if all_destroyed {
                    debug!("All transactions reached Destroyed state");
                    break;
                }

                wait_iterations += 1;
                if wait_iterations > 20 {
                    // 2 second timeout
                    warn!(
                        "Timeout waiting for transactions to reach Destroyed state, forcing cleanup"
                    );
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        // Now clear the transaction maps
        self.client_transactions.clear();
        self.client_completions.clear();
        self.client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.server_transactions.clear();
        self.terminated_transactions.clear();
        debug_assert!(self.explicit_termination_operations.is_empty());
        self.explicit_termination_operations.clear();
        self.server_invite_dialog_index.clear();
        self.server_invite_dialog_keys_by_tx.clear();
        self.server_invite_dialog_expiry_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        {
            let mut scheduler = self
                .invite_2xx_response_due_queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.invite_2xx_response_cache.clear();
            scheduler.clear();
        }
        self.transaction_destinations.clear();
        self.compact_non_invite_tombstones.clear();
        self.retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.retired_client_transaction_count
            .store(0, Ordering::Release);
        self.pending_inbound_bytes.clear();
        self.pending_inbound_inserted_at.clear();
        self.pending_inbound_transport.clear();
        self.pending_inbound_timing.clear();
        self.pending_inbound_principals.clear();
        self.pending_inbound_principal_inserted_at.clear();
        // Admission is closed before shutdown cleanup begins. Any shared
        // terminal event left queued after its dialog receiver disappeared is
        // now unreachable, so release its external owner fence explicitly.
        self.events_tx.clear_terminal_event_fences();

        // Step 5: Emit TransactionEvent::ShutdownComplete
        // Broadcast to all event subscribers
        if tokio::time::timeout(
            Duration::from_millis(100),
            Self::broadcast_event(
                TransactionEvent::ShutdownComplete,
                &self.events_tx,
                &self.event_subscribers,
                Some(&self.subscriber_to_transactions),
                Some(&self.transaction_to_subscribers),
                Some(self.clone()),
            ),
        )
        .await
        .is_err()
        {
            warn!("Timed out publishing ShutdownComplete to a stalled primary TU");
        }

        // Step 5: Clear event subscribers
        self.event_subscribers.store(Arc::new(Vec::new()));
        self.subscriber_to_transactions.clear();
        self.transaction_to_subscribers.clear();

        // The registry stores only plain generations, so explicit clear is
        // non-reentrant and safe even when caller-held transaction Arcs later
        // drop their external owners. Stopped managers never admit reuse.
        self.transaction_admissions.entries.clear();
        debug_assert_eq!(self.transaction_admissions.entries.len(), 0);
        self.admission_lifecycle.mark_stopped();

        info!("TransactionManager shutdown complete - BOTTOM-UP");
    }

    /// Broadcasts a transaction event to all subscribers.
    ///
    /// This method is responsible for delivering transaction events to subscribers.
    /// It implements event filtering based on subscriber preferences, ensuring
    /// subscribers only receive events they're interested in.
    ///
    /// # Arguments
    /// * `event` - The transaction event to broadcast
    /// * `primary_tx` - The primary event channel
    /// * `subscribers` - Additional event subscribers
    /// * `transaction_to_subscribers` - Maps transactions to interested subscribers
    /// * `manager` - Optional manager for processing termination events
    async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &TransactionEventSender,
        _subscribers: &Arc<ArcSwap<Vec<EventSubscriber>>>,
        _subscriber_to_transactions: Option<&Arc<DashMap<usize, Vec<TransactionKey>>>>,
        _transaction_to_subscribers: Option<&Arc<DashMap<TransactionKey, Vec<EventSubscriber>>>>,
        manager: Option<TransactionManager>,
    ) {
        let broadcast_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        if let Some(manager_instance) = manager.as_ref() {
            match &event {
                TransactionEvent::StateChanged {
                    transaction_id,
                    new_state: TransactionState::Terminated,
                    ..
                }
                | TransactionEvent::TransactionTerminated { transaction_id } => {
                    manager_instance
                        .cache_invite_2xx_response_for(transaction_id)
                        .await;
                }
                _ => {}
            }
        }

        // Send to primary channel if it's available
        if let Err(e) = primary_tx.send(event.clone()).await {
            // During shutdown, channel closed errors are expected
            if e.to_string().contains("channel closed") {
                debug!("Primary event channel closed during shutdown (expected)");
            } else {
                warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send event to primary channel");
            }
        }

        if let Some(manager_instance) = manager.as_ref() {
            match &event {
                TransactionEvent::StateChanged {
                    transaction_id,
                    new_state: TransactionState::Terminated,
                    ..
                }
                | TransactionEvent::TransactionTerminated { transaction_id } => {
                    manager_instance.mark_transaction_terminated_indexed(transaction_id);
                    manager_instance
                        .pending_inbound_bytes
                        .remove(transaction_id);
                    manager_instance
                        .pending_inbound_inserted_at
                        .remove(transaction_id);
                    manager_instance
                        .pending_inbound_transport
                        .remove(transaction_id);
                    manager_instance
                        .pending_inbound_timing
                        .remove(transaction_id);
                    manager_instance.enqueue_terminated_transaction_cleanup(transaction_id.clone());
                }
                _ => {}
            }
        }

        if let Some(started) = broadcast_started {
            diagnostics::record_transaction_event_broadcast(started.elapsed());
        }
    }

    /// Actually remove a terminated transaction from all maps
    async fn remove_terminated_transaction(&self, transaction_id: &TransactionKey) {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Removing terminated transaction after grace period");

        let mut terminated = false;
        self.request_transaction_runner_stop(transaction_id);

        // Keyed observers remain until the exact terminal sender snapshots
        // and publishes them. Authoritative map cleanup is independent of a
        // stalled primary channel, while the admission owner prevents a new
        // same-key transaction from inheriting this observer bucket.

        if self
            .retire_and_remove_client_transaction(transaction_id)
            .await
        {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Removed terminated client transaction");
            terminated = true;
        }

        // Defensive: also remove from server in case of duplication.
        if self.server_transactions.remove(transaction_id).is_some() {
            self.retire_server_invite_dialog_index_for(transaction_id);
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Removed terminated server transaction");
            terminated = true;
        }

        if (transaction_id.is_server() || !terminated)
            && self
                .transaction_destinations
                .remove_if(transaction_id, |_, state| state.is_active())
                .is_some()
        {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Removed transaction from destinations map");
        }
        self.terminated_transactions.remove(transaction_id);
        self.pending_inbound_bytes.remove(transaction_id);
        self.pending_inbound_inserted_at.remove(transaction_id);
        self.pending_inbound_transport.remove(transaction_id);
        self.pending_inbound_timing.remove(transaction_id);

        // Unregister from timer manager (defensive - it should auto-unregister)
        let unregister_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        self.timer_manager
            .unregister_transaction(transaction_id)
            .await;
        if let Some(started) = unregister_started {
            diagnostics::record_termination_cleanup_timer_unregister(started.elapsed());
        }
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Unregistered transaction from timer manager");

        if terminated {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Successfully cleaned up terminated transaction");
        } else {
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Transaction not found for termination - may have been already removed");
        }
    }

    /// Start the message processing loop for handling incoming transport events
    fn start_message_loop(&self) {
        let Some(transport_rx) = self.transport_rx.clone() else {
            debug!("Transaction manager has no transport ingress; message loop not started");
            return;
        };
        // Publish synchronously so an immediate shutdown cannot be undone by
        // a later-scheduled task storing `true` after drain has begun.
        self.running.store(true, Ordering::Release);
        let control_transport_rx = self.control_transport_rx.clone();
        let running = self.running.clone();
        let manager_arc = self.clone();
        let dispatch_workers = self.transaction_dispatch_workers;
        let dispatch_queue_capacity = self.transaction_dispatch_queue_capacity;
        let dispatch_priority_burst_max = self.transaction_dispatch_priority_burst_max.clone();

        tokio::spawn(async move {
            debug!("Starting transaction message loop");

            let dispatch_senders = if dispatch_workers > DEFAULT_TRANSACTION_DISPATCH_WORKERS {
                Some(start_transaction_dispatch_workers(
                    manager_arc.clone(),
                    dispatch_workers,
                    dispatch_queue_capacity,
                    dispatch_priority_burst_max,
                ))
            } else {
                None
            };
            let fallback_dispatch_worker = Arc::new(AtomicUsize::new(0));

            // Get the transport receiver
            let mut receiver = transport_rx.lock().await;
            let mut control_receiver = match control_transport_rx.as_ref() {
                Some(receiver) => Some(receiver.lock().await),
                None => None,
            };
            let mut cleanup_interval = tokio::time::interval(Duration::from_secs(1));
            cleanup_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let cleanup_running = Arc::new(AtomicBool::new(false));
            let mut invite_2xx_retransmit_interval =
                tokio::time::interval(Duration::from_millis(100));
            invite_2xx_retransmit_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let invite_2xx_retransmit_running = Arc::new(AtomicBool::new(false));

            // Run the message processing loop
            loop {
                // Check if we should continue running. AtomicBool load
                // is one instruction; the previous async-mutex acquire
                // was per-iteration.
                if !running.load(Ordering::Relaxed) {
                    debug!("Transaction manager stopping message loop");
                    break;
                }

                // Wait for transport work and bounded maintenance ticks. The
                // former internal transaction-event channel had no producer;
                // retaining both halves allocated a dummy queue forever.
                tokio::select! {
                    biased;
                    control_event = async {
                        match control_receiver.as_mut() {
                            Some(receiver) => receiver.recv().await,
                            None => std::future::pending::<Option<TransportEvent>>().await,
                        }
                    } => {
                        if let Some(control_event) = control_event {
                            if manager_arc.running.load(Ordering::Relaxed) {
                                if let Some(dispatch_senders) = dispatch_senders.as_ref() {
                                    dispatch_transaction_event(
                                        control_event,
                                        dispatch_senders,
                                        &fallback_dispatch_worker,
                                    ).await;
                                } else if let Err(e) = manager_arc.handle_transport_event(control_event).await {
                                    error!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error handling transport control event");
                                }
                            }
                        } else {
                            control_receiver = None;
                        }
                    }
                    Some(mut message_event) = receiver.recv() => {
                        // Check if we're still running before processing
                        let still_running = manager_arc.running.load(Ordering::Relaxed);
                        if still_running {
                            mark_transaction_manager_received(&mut message_event, Instant::now());
                            if let Some(dispatch_senders) = dispatch_senders.as_ref() {
                                dispatch_transaction_event(
                                    message_event,
                                    dispatch_senders,
                                    &fallback_dispatch_worker,
                                ).await;
                            } else if diagnostics::transaction_timing_enabled() {
                                process_transaction_dispatch_event(
                                    &manager_arc,
                                    QueuedTransactionDispatch {
                                        kind: transaction_ingress_kind(&message_event),
                                        event: message_event,
                                        queued_at: None,
                                        worker_id: 0,
                                    },
                                ).await;
                            } else {
                                if let Err(e) = manager_arc.handle_transport_event(message_event).await {
                                    error!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error handling transport message");
                                }
                            }
                        } else {
                            debug!("Skipping transport event processing - shutting down");
                        }
                    }
                    _ = cleanup_interval.tick() => {
                        if cleanup_running
                            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                            .is_ok()
                        {
                            let manager_clone = manager_arc.clone();
                            let cleanup_running = cleanup_running.clone();
                            tokio::spawn(async move {
                                let Some(_maintenance_operation) = manager_clone
                                    .admission_lifecycle
                                    .try_enter_existing()
                                else {
                                    cleanup_running.store(false, Ordering::Release);
                                    return;
                                };
                                tokio::select! {
                                    _ = manager_clone.operation_cancellation.cancelled() => {}
                                    cleanup_result = async {
                                        manager_clone.maintenance_prune_auxiliary_retained_state();
                                        // Runtime repair is bounded to the authoritative
                                        // terminal index. A full active-table scan remains
                                        // available only through the explicit diagnostic API.
                                        manager_clone
                                            .cleanup_indexed_terminated_transactions_within_operation()
                                            .await
                                    } => {
                                        match cleanup_result {
                                            Ok(count) if count > 0 => {
                                                debug!(
                                                    "Periodic cleanup removed {} terminated transactions",
                                                    count
                                                );
                                            }
                                            Err(e) => {
                                                error!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Periodic transaction cleanup failed");
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                cleanup_running.store(false, Ordering::Release);
                            });
                        }
                    }
                    _ = invite_2xx_retransmit_interval.tick() => {
                        if invite_2xx_retransmit_running
                            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                            .is_ok()
                        {
                            let manager_clone = manager_arc.clone();
                            let retransmit_running = invite_2xx_retransmit_running.clone();
                            tokio::spawn(async move {
                                let Some(_retransmit_operation) = manager_clone
                                    .admission_lifecycle
                                    .try_enter_existing()
                                else {
                                    retransmit_running.store(false, Ordering::Release);
                                    return;
                                };
                                tokio::select! {
                                    _ = manager_clone.operation_cancellation.cancelled() => {}
                                    count = manager_clone.retransmit_due_invite_2xx_responses() => {
                                        if count > 0 {
                                            trace!("Retransmitted {} cached INVITE 2xx responses", count);
                                        }
                                    }
                                }
                                retransmit_running.store(false, Ordering::Release);
                            });
                        }
                    }
                    else => {
                        // Both channels have been closed; exit loop
                        debug!("All message channels closed, exiting transaction message loop");
                        break;
                    }
                }
            }

            debug!("Transaction message loop exited");
        });
    }

    /// Helper function to get timer settings for a request
    fn timer_settings_for_request(&self, _request: &Request) -> Option<TimerSettings> {
        // In the future, we could customize timer settings based on request properties
        // For now, just return a clone of the default settings
        Some(self.timer_settings.clone())
    }

    /// Create a client transaction for sending a SIP request
    /// The caller is responsible for calling send_request() to initiate the transaction.
    pub async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        let route = crate::transaction::transport::multiplexed::transport_route_for_request(
            &request,
            destination,
        )?;
        self.create_client_transaction_on_route(request, route)
            .await
    }

    /// Create a client transaction and return its exact completion authority.
    pub async fn create_client_transaction_with_completion(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<(TransactionKey, ClientTransactionCompletionHandle)> {
        let route = crate::transaction::transport::multiplexed::transport_route_for_request(
            &request,
            destination,
        )?;
        self.create_client_transaction_on_route_with_completion(request, route)
            .await
    }

    /// Create a client transaction using an explicit resolver-selected route.
    /// The route's transport and authenticated authority govern every initial
    /// send and retransmission; they are not reconstructed from the request.
    pub async fn create_client_transaction_on_route(
        &self,
        request: Request,
        request_route: TransportRoute,
    ) -> Result<TransactionKey> {
        self.create_client_transaction_on_route_inner(request, request_route, None)
            .await
            .map(|(key, _owner, _completion)| key)
    }

    /// Create a client transaction and atomically return its exact completion
    /// authority with the key.
    ///
    /// Protocol owners that must observe a terminal result after manager
    /// retirement should retain this handle instead of resolving completion
    /// later through the transaction-key index.
    pub async fn create_client_transaction_on_route_with_completion(
        &self,
        request: Request,
        request_route: TransportRoute,
    ) -> Result<(TransactionKey, ClientTransactionCompletionHandle)> {
        self.create_client_transaction_on_route_inner(request, request_route, None)
            .await
            .map(|(key, _owner, completion)| (key, completion))
    }

    /// Atomically return the exact allocation owner with the key. Failover
    /// plans retain this owner beyond ordinary transaction-map cleanup and
    /// must not recover it through a second, racy map lookup.
    pub(crate) async fn create_client_transaction_on_route_with_timeout_and_owner(
        &self,
        request: Request,
        request_route: TransportRoute,
        transaction_timeout: Duration,
    ) -> Result<(TransactionKey, TransactionAdmissionOwner)> {
        let mut timer_settings = self.timer_settings.clone();
        timer_settings.transaction_timeout = transaction_timeout.max(Duration::from_millis(1));
        self.create_client_transaction_on_route_inner(request, request_route, Some(timer_settings))
            .await
            .map(|(key, owner, _completion)| (key, owner))
    }

    async fn create_client_transaction_on_route_inner(
        &self,
        request: Request,
        mut request_route: TransportRoute,
        timer_settings_override: Option<TimerSettings>,
    ) -> Result<(
        TransactionKey,
        TransactionAdmissionOwner,
        ClientTransactionCompletionHandle,
    )> {
        crate::transaction::transport::multiplexed::validate_request_route_security(
            &request,
            &request_route,
        )?;
        let destination = request_route.destination;
        // Reject caller-controlled start lines and fields before route/Via
        // inspection, normalization, transaction allocation, or any route log.
        rvoip_sip_core::validation::validate_typed_outbound_message(&Message::Request(
            request.clone(),
        ))?;
        debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
            destination=%destination,
            via_count=request.via_headers().len(),
            "Creating client transaction"
        );

        tracing::trace!(
            via_count = request.via_headers().len(),
            has_top_route = top_route_uri(&request).is_some(),
            "Client transaction request routing metadata before normalization"
        );

        // Extract branch parameter from the top Via header or generate a new one
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) => b.to_string(),
                    None => {
                        // Generate a branch parameter if none exists
                        format!(
                            "{}{}",
                            RFC3261_BRANCH_MAGIC_COOKIE,
                            uuid::Uuid::new_v4().as_simple()
                        )
                    }
                }
            }
            None => {
                // No Via header - should not happen, but we'll handle it by generating a branch
                // and a Via header will be added by the transaction
                format!(
                    "{}{}",
                    RFC3261_BRANCH_MAGIC_COOKIE,
                    uuid::Uuid::new_v4().as_simple()
                )
            }
        };

        // We'll create the transaction key directly
        let key = TransactionKey::new(branch.clone(), request.method().clone(), false);
        let _admission_guard = self.admission_lifecycle.try_enter().ok_or_else(|| {
            Error::Other("transaction manager is draining; new transactions are closed".into())
        })?;
        if self.compact_non_invite_tombstones.contains_key(&key) {
            return Err(Error::Other(
                "transaction key is retained by an active UDP Timer K tombstone".into(),
            ));
        }
        let duplicate_kind = if request.method() == Method::Invite {
            TransactionKind::InviteClient
        } else {
            TransactionKind::NonInviteClient
        };
        let admission_owner = self.transaction_admissions.try_claim(&key).ok_or_else(|| {
            Error::TransactionExists {
                key: key.clone(),
                kind: duplicate_kind,
            }
        })?;
        let returned_admission_owner = admission_owner.clone();

        // For CANCEL method, make sure we don't add a new Via header if one already exists
        // This is already checked in create_cancel_request, but we'll verify here as well
        let mut modified_request = request.clone();

        // For CANCEL requests, the Via header should be preserved exactly as it was created
        // No need to add or modify it
        if request.method() == Method::Cancel {
            // Since CANCEL already has a Via header with the correct branch from create_cancel_request,
            // we don't need to modify it further
            tracing::trace!("CANCEL request detected - not adding Via header");
        } else {
            // For non-CANCEL methods, preserve the request builder's selected
            // Via transport and sent-by address. The transaction layer owns only
            // the branch/rport normalization needed for transaction matching and
            // symmetric response routing.
            if !normalize_top_client_via(&mut modified_request, &branch) {
                let local_addr = self.transport.local_addr().map_err(|e| {
                    Error::transport_error(e, "Failed to get local address for Via header")
                })?;
                let via_transport = transport_token_for_request(&modified_request);
                let via_header =
                    handlers::create_via_header_for_transport(&local_addr, &branch, via_transport)?;
                modified_request = modified_request.with_header(via_header);
            }
        }

        if sip_diagnostics_enabled() {
            if let Some(via) = modified_request.first_via() {
                info!(
                    method=%crate::transaction::safe_diagnostics::SafeMethod::new(&modified_request.method()),
                    has_top_route=top_route_uri(&modified_request).is_some(),
                    destination = %destination,
                    top_via_has_branch=via.branch().is_some(),
                    via_count=modified_request.via_headers().len(),
                    "RVOIP_SIP_DIAG outgoing request routing metadata after transaction normalization"
                );
            }
        }

        tracing::trace!(
            via_count = modified_request.via_headers().len(),
            "Client transaction request Via metadata after normalization"
        );

        let derived_route =
            crate::transaction::transport::multiplexed::transport_route_for_request(
                &modified_request,
                destination,
            )?;
        if request_route.transport_type.is_none() {
            request_route.transport_type = derived_route.transport_type;
        }
        if request_route.authority.is_none() {
            request_route.authority = derived_route.authority;
        }
        if request_route.transport_type == Some(TransportType::Udp)
            && Message::Request(modified_request.clone()).to_bytes().len()
                > self.transport.max_safe_message_size()
            && self.transport.supports_tcp()
        {
            request_route.transport_type = Some(TransportType::Tcp);
        }
        if request.method() != Method::Cancel {
            let via_transport = match request_route.transport_type {
                Some(TransportType::Udp) => "UDP",
                Some(TransportType::Tcp) => "TCP",
                Some(TransportType::Tls) => "TLS",
                Some(TransportType::Ws) => "WS",
                Some(TransportType::Wss) => "WSS",
                None => transport_token_for_request(&modified_request),
            };
            crate::transaction::utils::set_top_via_protocol(&mut modified_request, via_transport);
        }

        rvoip_sip_core::validation::validate_wire_request(&modified_request)?;
        let timer_settings =
            timer_settings_override.or_else(|| self.timer_settings_for_request(&modified_request));

        // Reserve the complete UDP Timer K horizon before the transaction
        // constructor starts its runner or any request can reach the wire.
        // Saturation is an admission failure; an accepted transaction must
        // never shorten its RFC retransmission-absorption fence.
        let compact_retention_reservation = if modified_request.method() != Method::Invite
            && crate::transaction::timer_utils::uses_unreliable_transport(
                &request_route,
                self.transport.default_transport_type(),
            ) {
            match self.lifecycle_scheduler.as_ref() {
                Some(scheduler) => Some(scheduler.try_reserve_compact_retention().ok_or(
                    Error::TransactionCapacityExhausted {
                        resource: "UDP non-INVITE Timer K retention",
                        limit: scheduler.compact_retention_limit(),
                    },
                )?),
                None => None,
            }
        } else {
            None
        };

        // Create the appropriate transaction. Returns Arc<dyn ClientTransaction>
        // so the map can shard (DashMap) and call sites can clone the
        // Arc out before any `.await`.
        let transaction: ArcClientTransaction = match modified_request.method() {
            Method::Invite => {
                tracing::trace!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Creating ClientInviteTransaction");
                let tx =
                    ClientInviteTransaction::new_with_route_command_capacity_and_timer_manager(
                        key.clone(),
                        modified_request.clone(),
                        request_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        timer_settings,
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?;
                tracing::trace!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Created ClientInviteTransaction");
                Arc::new(tx)
            }
            Method::Cancel => {
                if let Err(e) = cancel::validate_cancel_request(&modified_request) {
                    warn!(
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&modified_request.method()),
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                        "Creating transaction for CANCEL with possible validation issues"
                    );
                }
                let tx =
                    ClientNonInviteTransaction::new_with_route_command_capacity_and_timer_manager(
                        key.clone(),
                        modified_request.clone(),
                        request_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        timer_settings,
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?;
                Arc::new(tx)
            }
            Method::Update => {
                if let Err(e) = update::validate_update_request(&modified_request) {
                    warn!(
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&modified_request.method()),
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                        "Creating transaction for UPDATE with possible validation issues"
                    );
                }
                let tx =
                    ClientNonInviteTransaction::new_with_route_command_capacity_and_timer_manager(
                        key.clone(),
                        modified_request.clone(),
                        request_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        timer_settings,
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?;
                Arc::new(tx)
            }
            _ => {
                let tx =
                    ClientNonInviteTransaction::new_with_route_command_capacity_and_timer_manager(
                        key.clone(),
                        modified_request.clone(),
                        request_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        timer_settings,
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?;
                Arc::new(tx)
            }
        };

        if let Some(cleanup_tx) = self.terminated_cleanup_tx.as_ref() {
            transaction
                .data()
                .install_termination_cleanup_sender(cleanup_tx.clone());
        }
        if let Some(reservation) = compact_retention_reservation {
            transaction
                .data()
                .install_compact_retention_reservation(reservation);
        }
        if let Some(scheduler) = self.lifecycle_scheduler.as_ref() {
            transaction
                .data()
                .install_lifecycle_scheduler(scheduler.clone());
        }
        transaction
            .data()
            .install_transaction_admission_owner(admission_owner);

        // Capture the exact completion authority from the newly constructed
        // transaction before publishing either the live runner or its key.
        // This closes the create/send/retire-versus-observer-lookup race for
        // protocol owners that carry the returned handle.
        let completion =
            ClientTransactionCompletionHandle::new(Arc::clone(&transaction.data().completion));

        // Store the authoritative completion cell before publishing the live
        // transaction. Internal waiters never need to subscribe to events.
        let shared_retention_key = Arc::new(key.clone());
        self.install_client_completion(Arc::clone(&shared_retention_key), &transaction);
        let response_route_owner = Arc::as_ptr(transaction.data()) as usize;
        self.client_transactions.insert(key.clone(), transaction);
        if let Some(previous) = self.transaction_destinations.insert(
            shared_retention_key,
            ClientResponseRouteState::active(request_route, response_route_owner),
        ) {
            if let Some(retired) = previous.retired() {
                self.decrement_retired_client_transaction_count();
                self.unschedule_retired_client_deadline(
                    &key,
                    retired.expires_at,
                    retired.deadline_version,
                );
            }
        }

        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Created client transaction");

        if request.method() == Method::Cancel {
            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Created CANCEL transaction");
        }

        Ok((key, returned_admission_owner, completion))
    }

    /// Creates and sends an ACK request for a 2xx response to an INVITE.
    pub async fn send_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped ACK send".into()))
            }
            result = self.send_ack_for_2xx_within_operation(invite_tx_id, response) => result,
        }
    }

    async fn send_ack_for_2xx_within_operation(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        // Create the ACK request
        let ack_request = self.create_ack_for_2xx(invite_tx_id, response).await?;
        let original_route = self.transaction_route(invite_tx_id).await.ok_or_else(|| {
            Error::transaction_not_found(invite_tx_id.clone(), "ACK route lookup failed")
        })?;

        // ACK follows the established dialog route set: top Route if present,
        // otherwise the remote target in the Contact-derived Request-URI.
        let destination = utils::socket_addr_from_uri(&next_hop_uri_for_request(&ack_request));

        // If the ACK has no route-set destination, try Contact explicitly.
        let contact_destination =
            if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
                if let Some(contact_addr) = contact.addresses().next() {
                    // Try to parse the URI as a socket address
                    if let Some(addr) = utils::socket_addr_from_uri(&contact_addr.uri) {
                        Some(addr)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

        // If we couldn't get a route or Contact destination, use the original destination.
        let destination = if let Some(dest) = destination.or(contact_destination) {
            dest
        } else {
            original_route.destination
        };

        let mut ack_route = original_route;
        if ack_route.destination != destination {
            ack_route.destination = destination;
            // An opaque stream flow is valid only for the peer it was bound
            // to. A Contact-selected destination must establish/select its
            // own flow rather than inheriting the INVITE's flow identity.
            ack_route.flow_id = None;
        }

        // Send the ACK directly without creating a transaction, while
        // preserving the authenticated transport/authority/flow selected by
        // the original INVITE whenever its route remains the next hop.
        rvoip_sip_core::validation::validate_wire_request(&ack_request)?;
        self.transport
            .send_message_via(Message::Request(ack_request), ack_route)
            .await
            .map_err(|e| Error::transport_error(e, "Failed to send ACK"))?;

        Ok(())
    }

    /// Find transaction by message.
    ///
    /// This method tries to find a transaction that matches the given message.
    /// For requests, it looks for server transactions.
    /// For responses, it looks for client transactions.
    ///
    /// # Arguments
    /// * `message` - The message to match
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching transaction key if found
    pub async fn find_transaction_by_message(
        &self,
        message: &Message,
    ) -> Result<Option<TransactionKey>> {
        let Some(key) = transaction_key_from_message(message) else {
            return Ok(None);
        };

        let found = match message {
            Message::Request(_) => {
                self.server_transactions.contains_key(&key)
                    || self
                        .compact_non_invite_tombstones
                        .get(&key)
                        .is_some_and(|entry| !entry.value().is_client())
            }
            Message::Response(_) => {
                self.client_transactions.contains_key(&key)
                    || self
                        .compact_non_invite_tombstones
                        .get(&key)
                        .is_some_and(|entry| entry.value().is_client())
            }
        };

        Ok(found.then_some(key))
    }

    /// Find the matching client-side INVITE transaction for a CANCEL request.
    ///
    /// Used when *we* are sending CANCEL to cancel our outgoing INVITE.
    ///
    /// # Arguments
    /// * `cancel_request` - The CANCEL request
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching INVITE transaction key if found
    pub async fn find_invite_transaction_for_cancel(
        &self,
        cancel_request: &Request,
    ) -> Result<Option<TransactionKey>> {
        if cancel_request.method() != Method::Cancel {
            return Err(Error::Other("Not a CANCEL request".to_string()));
        }

        let Some(cancel_key) = TransactionKey::from_request(cancel_request) else {
            return Ok(None);
        };
        let invite_key = TransactionKey::new(cancel_key.branch, Method::Invite, false);

        Ok(self
            .client_transactions
            .contains_key(&invite_key)
            .then_some(invite_key))
    }

    /// Find the matching server-side INVITE transaction for an inbound
    /// CANCEL request.
    ///
    /// Used when a peer is cancelling an INVITE *they* sent us. The match
    /// is by branch + sent-by per RFC 3261 §9.2 — the same algorithm as
    /// the client-side search, just against the server transaction pool.
    pub async fn find_invite_server_transaction_for_cancel(
        &self,
        cancel_request: &Request,
    ) -> Result<Option<TransactionKey>> {
        if cancel_request.method() != Method::Cancel {
            return Err(Error::Other("Not a CANCEL request".to_string()));
        }

        let Some(cancel_key) = TransactionKey::from_request(cancel_request) else {
            return Ok(None);
        };
        let invite_key = cancel_key.with_method(Method::Invite);

        Ok(self
            .server_transactions
            .contains_key(&invite_key)
            .then_some(invite_key))
    }

    /// Retrieve the original request that created a server transaction.
    ///
    /// Mirrors the client-side `utils::get_transaction_request` helper.
    /// Needed so the CANCEL handler can generate a 487 response based on
    /// the pending INVITE.
    pub async fn get_server_transaction_request(&self, tx_id: &TransactionKey) -> Result<Request> {
        let tx_arc = self
            .server_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        if let Some(tx) = tx_arc {
            if let Some(req) = tx.original_request().await {
                return Ok(req);
            }
        }
        Err(Error::transaction_not_found(
            tx_id.clone(),
            "get_server_transaction_request - transaction not found",
        ))
    }

    /// Creates an ACK request for a 2xx response to an INVITE.
    pub async fn create_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<Request> {
        // Verify this is an INVITE client transaction
        if *invite_tx_id.method() != Method::Invite || invite_tx_id.is_server {
            return Err(Error::Other(
                "Can only create ACK for INVITE client transactions".to_string(),
            ));
        }

        // Active transactions own the parsed request directly. Retired
        // INVITEs reconstruct it lazily from their immutable wire image only
        // when a rare late/forked 2xx actually needs an ACK.
        let invite_request =
            if let Some(request) = self.retired_client_original_request(invite_tx_id)? {
                request
            } else {
                self.original_request(invite_tx_id).await?.ok_or_else(|| {
                    Error::transaction_not_found(
                        invite_tx_id.clone(),
                        "ACK request template is unavailable",
                    )
                })?
            };

        // Use the original INVITE top Via sent-by for the ACK's sent-by
        // address. With multiplexed transports, `transport.local_addr()`
        // reports the default transport address, which may be UDP even
        // when the original INVITE was sent over TLS.
        let local_addr = invite_request
            .first_via()
            .and_then(|via| match via.0.first() {
                Some(via_header) => match via_header.host() {
                    Host::Address(ip) => {
                        Some(SocketAddr::new(*ip, via_header.port().unwrap_or(5060)))
                    }
                    Host::Domain(_) => None,
                },
                None => None,
            })
            .map(Ok)
            .unwrap_or_else(|| {
                self.transport
                    .local_addr()
                    .map_err(|e| Error::transport_error(e, "Failed to get local address"))
            })?;

        // Create the ACK request using our utility
        let ack_request = crate::transaction::method::ack::create_ack_for_2xx(
            &invite_request,
            response,
            &local_addr,
        )?;

        Ok(ack_request)
    }

    /// Create a server transaction from an incoming request.
    ///
    /// This is called when a new request is received from the transport layer.
    /// It creates an appropriate transaction based on the request method.
    /// Deprecated compatibility entry point: address-only construction is
    /// valid only for UDP. New code must use
    /// [`Self::create_server_transaction_on_route`] for stream transports.
    pub async fn create_server_transaction(
        &self,
        request: Request,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerTransaction>> {
        self.create_server_transaction_inner(
            request,
            TransportRoute::new(remote_addr).with_transport_type(TransportType::Udp),
            true,
        )
        .await
    }

    /// Create a server transaction bound to the exact ingress transport flow.
    pub async fn create_server_transaction_on_route(
        &self,
        request: Request,
        response_route: TransportRoute,
    ) -> Result<Arc<dyn ServerTransaction>> {
        self.create_server_transaction_inner(request, response_route, true)
            .await
    }

    /// Transaction-ingress variant used while listener authorization is still
    /// pending. CANCEL publication is deferred until the authorization gate
    /// succeeds, preventing rejected requests from reaching the TU.
    #[allow(dead_code)]
    pub(crate) async fn create_server_transaction_deferred_events(
        &self,
        request: Request,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerTransaction>> {
        self.create_server_transaction_inner(
            request,
            TransportRoute::new(remote_addr).with_transport_type(TransportType::Udp),
            false,
        )
        .await
    }

    /// Create a server transaction bound to the exact ingress route while TU
    /// publication remains deferred behind authorization.
    pub(crate) async fn create_server_transaction_deferred_events_on_route(
        &self,
        request: Request,
        response_route: TransportRoute,
    ) -> Result<Arc<dyn ServerTransaction>> {
        self.create_server_transaction_inner(request, response_route, false)
            .await
    }

    async fn create_server_transaction_inner(
        &self,
        request: Request,
        response_route: TransportRoute,
        publish_cancel_event: bool,
    ) -> Result<Arc<dyn ServerTransaction>> {
        let remote_addr = response_route.destination;
        // Extract branch parameter from the top Via header
        let branch = match request.first_via() {
            Some(via) => match via.branch() {
                Some(b) => b.to_string(),
                None => {
                    return Err(Error::Other(
                        "Missing branch parameter in Via header".to_string(),
                    ));
                }
            },
            None => return Err(Error::Other("Missing Via header in request".to_string())),
        };

        // Create the transaction key directly with is_server: true
        let key = TransactionKey::new(branch, request.method().clone(), true);
        let _admission_guard = self.admission_lifecycle.try_enter().ok_or_else(|| {
            Error::Other("transaction manager is draining; new transactions are closed".into())
        })?;
        if self.compact_non_invite_tombstones.contains_key(&key) {
            return Err(Error::Other(
                "transaction key is retained by an active UDP Timer J tombstone".into(),
            ));
        }
        let mut cancel_target_invite_tx_id = None;

        // Check if this is a retransmission of an existing transaction.
        // Extract the Arc out of the shard before awaiting `process_request`.
        let existing = self
            .server_transactions
            .get(&key)
            .map(|r| r.value().clone());
        if let Some(transaction) = existing {
            drop(_admission_guard);
            transaction.process_request(request.clone()).await?;
            debug!(
                id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key),
                method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                "Processed retransmitted request in existing transaction"
            );
            return Ok(transaction);
        }

        let duplicate_kind = if request.method() == Method::Invite {
            TransactionKind::InviteServer
        } else {
            TransactionKind::NonInviteServer
        };
        let admission_owner = match self.transaction_admissions.try_claim(&key) {
            Some(owner) => owner,
            None => {
                // A concurrent first request may have published after our
                // initial lookup. Preserve retransmission semantics when that
                // publication is already visible; otherwise report the exact
                // admission collision without constructing a second runner.
                if let Some(transaction) = self
                    .server_transactions
                    .get(&key)
                    .map(|entry| entry.value().clone())
                {
                    drop(_admission_guard);
                    transaction.process_request(request.clone()).await?;
                    return Ok(transaction);
                }
                return Err(Error::TransactionExists {
                    key,
                    kind: duplicate_kind,
                });
            }
        };

        // Reserve the complete UDP Timer J replay horizon before constructing
        // the runner or publishing the transaction. A saturated listener may
        // reject new work, but it must not accept a request and later discard
        // its final-response replay state early.
        let compact_retention_reservation = if request.method() != Method::Invite
            && crate::transaction::timer_utils::uses_unreliable_transport(
                &response_route,
                self.transport.default_transport_type(),
            ) {
            match self.lifecycle_scheduler.as_ref() {
                Some(scheduler) => Some(scheduler.try_reserve_compact_retention().ok_or(
                    Error::TransactionCapacityExhausted {
                        resource: "UDP non-INVITE Timer J retention",
                        limit: scheduler.compact_retention_limit(),
                    },
                )?),
                None => None,
            }
        } else {
            None
        };

        // Create a new transaction based on the request method
        let transaction: Arc<dyn ServerTransaction> = match request.method() {
            Method::Invite => {
                let tx = Arc::new(
                    ServerInviteTransaction::new_with_response_route_command_capacity_and_timer_manager(
                        key.clone(),
                        request.clone(),
                        response_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?,
                );

                info!(
                    id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(tx.id()),
                    method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                    "Created new ServerInviteTransaction"
                );
                self.index_server_invite_dialog(&request, &key, admission_owner.clone());
                tx
            }
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&request) {
                    warn!(
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                        "Creating transaction for CANCEL with possible validation issues"
                    );
                }

                let invite_tx_id = key.with_method(Method::Invite);
                cancel_target_invite_tx_id = if self.client_transactions.contains_key(&invite_tx_id)
                    || self.server_transactions.contains_key(&invite_tx_id)
                {
                    Some(invite_tx_id)
                } else {
                    None
                };

                // Create a non-INVITE server transaction for CANCEL
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_response_route_command_capacity_and_timer_manager(
                        key.clone(),
                        request.clone(),
                        response_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?,
                );

                info!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(tx.id()), method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()), "Created new ServerNonInviteTransaction for CANCEL");

                tx
            }
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&request) {
                    warn!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Creating transaction for UPDATE with possible validation issues");
                }

                // Create a non-INVITE server transaction for UPDATE
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_response_route_command_capacity_and_timer_manager(
                        key.clone(),
                        request.clone(),
                        response_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?,
                );

                info!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(tx.id()), method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()), "Created new ServerNonInviteTransaction for UPDATE");
                tx
            }
            _ => {
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_response_route_command_capacity_and_timer_manager(
                        key.clone(),
                        request.clone(),
                        response_route.clone(),
                        self.transport.clone(),
                        self.events_tx.clone_for_transaction(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                        self.timer_manager.clone(),
                    )?,
                );

                info!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(tx.id()), method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()), "Created new ServerNonInviteTransaction");
                tx
            }
        };

        if let Some(cleanup_tx) = self.terminated_cleanup_tx.as_ref() {
            transaction
                .data()
                .install_termination_cleanup_sender(cleanup_tx.clone());
        }
        if let Some(reservation) = compact_retention_reservation {
            transaction
                .data()
                .install_compact_retention_reservation(reservation);
        }
        if let Some(scheduler) = self.lifecycle_scheduler.as_ref() {
            transaction
                .data()
                .install_lifecycle_scheduler(scheduler.clone());
        }
        transaction
            .data()
            .install_transaction_admission_owner(admission_owner);
        transaction
            .data()
            .install_manager_admission_lifecycle(&self.admission_lifecycle);

        // Store the transaction
        self.server_transactions
            .insert(transaction.id().clone(), transaction.clone());

        // Start the transaction in Trying state (for non-INVITE) or Proceeding (for INVITE)
        let initial_state = match transaction.kind() {
            TransactionKind::InviteServer => TransactionState::Proceeding,
            _ => TransactionState::Trying,
        };

        // The transaction and its private runner are constructed in the same
        // initial state. Publish the matching initialization command without
        // an await: once the transaction and its derived INVITE index are
        // visible, cancellation of this create future must not strand a
        // partially published generation. The new private queue has at least
        // one slot; Full/Closed is therefore an initialization failure and is
        // rolled back synchronously.
        if let Err(e) = transaction
            .data()
            .cmd_tx
            .try_send(InternalTransactionCommand::TransitionTo(initial_state))
        {
            let e = Error::Other(format!(
                "failed to initialize new server transaction command queue: {e}"
            ));
            error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction.id()), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to initialize new server transaction");
            // The transaction was published before the initialization
            // command so retransmissions could find it. On channel failure,
            // close subscription admission, remove its observer bucket while
            // the exact map entry still fences same-key reuse, then remove
            // only this allocation. Dropping the final data Arc releases its
            // compact-retention reservation.
            self.rollback_failed_server_initialization(&transaction);
            return Err(e);
        }
        drop(_admission_guard);

        if publish_cancel_event && request.method() == Method::Cancel {
            let transaction_id = transaction.id().clone();
            let event = match cancel_target_invite_tx_id {
                Some(invite_tx_id) => TransactionEvent::CancelRequest {
                    transaction_id: transaction_id.clone(),
                    target_transaction_id: invite_tx_id,
                    request: request.clone(),
                    source: remote_addr,
                },
                None => TransactionEvent::NonInviteRequest {
                    transaction_id: transaction_id.clone(),
                    request: request.clone(),
                    source: remote_addr,
                },
            };
            if let Err(error) = self.events_tx.send(event).await {
                warn!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error), "Failed to publish CANCEL transaction event");
                let _ = self.terminate_transaction(&transaction_id).await;
            }
        }

        Ok(transaction)
    }

    pub(super) fn rollback_failed_server_initialization(
        &self,
        transaction: &Arc<dyn ServerTransaction>,
    ) {
        // Stop the runner while this exact Arc is still discoverable, then
        // make every subsequent command silent before removing indexes.
        self.request_transaction_runner_stop(transaction.id());
        transaction.data().state.set(TransactionState::Terminated);
        transaction
            .data()
            .set_lifecycle(TransactionLifecycle::Destroyed);
        if let Some(observers) = self.events_tx.observer_fanout() {
            observers.remove_transaction(transaction.id());
        }
        self.pending_inbound_principals.remove(transaction.id());
        self.pending_inbound_principal_inserted_at
            .remove(transaction.id());
        self.pending_inbound_bytes.remove(transaction.id());
        self.pending_inbound_inserted_at.remove(transaction.id());
        self.pending_inbound_transport.remove(transaction.id());
        self.pending_inbound_timing.remove(transaction.id());
        self.transaction_destinations
            .remove_if(transaction.id(), |_, route| route.is_active());
        self.remove_server_invite_dialog_index_exact(transaction.id());
        self.server_transactions
            .remove_if(transaction.id(), |_, current| {
                Arc::ptr_eq(current, transaction)
            });
        self.terminated_transactions.remove(transaction.id());

        if let (Ok(runtime), Some(operation)) = (
            tokio::runtime::Handle::try_current(),
            self.admission_lifecycle.try_enter_existing(),
        ) {
            let manager = self.clone();
            let transaction_id = transaction.id().clone();
            runtime.spawn(async move {
                let _operation = operation;
                manager
                    .timer_manager
                    .unregister_transaction(&transaction_id)
                    .await;
            });
        }
    }

    fn index_server_invite_dialog(
        &self,
        request: &Request,
        key: &TransactionKey,
        admission_owner: TransactionAdmissionOwner,
    ) {
        if let Some(dialog_key) = ServerInviteDialogKey::from_request(request) {
            self.insert_server_invite_dialog_index_entry(
                dialog_key,
                ServerInviteAckIndexEntry::active_with_owner(key.clone(), Some(admission_owner)),
            );
        }
    }

    pub(crate) fn insert_server_invite_dialog_index_entry(
        &self,
        dialog_key: ServerInviteDialogKey,
        mut entry: ServerInviteAckIndexEntry,
    ) {
        let transaction_id = entry.transaction_id.clone();
        let is_active = entry.expires_at.is_none();
        let generation = self.next_server_invite_dialog_deadline_generation();
        entry.deadline_generation = generation;
        let expires_at = entry.expires_at;

        let previous = self
            .server_invite_dialog_index
            .insert(dialog_key.clone(), entry);

        if let Some(previous) = previous {
            if previous.expires_at.is_none()
                && (!is_active || previous.transaction_id != transaction_id)
            {
                self.remove_server_invite_dialog_reverse_key(&previous.transaction_id, &dialog_key);
            }
        }

        if is_active {
            let mut keys = self
                .server_invite_dialog_keys_by_tx
                .entry(transaction_id.clone())
                .or_default();
            if !keys.iter().any(|existing| existing == &dialog_key) {
                keys.push(dialog_key.clone());
            }
        }

        if let Some(due_at) = expires_at {
            self.schedule_server_invite_ack_expiry(ServerInviteAckExpiryEntry {
                due_at,
                generation,
                dialog_key,
            });
        }
    }

    pub(crate) fn retire_server_invite_dialog_index_for(&self, transaction_id: &TransactionKey) {
        let expires_at = Instant::now() + self.timer_settings.t4;

        if let Some((_, keys)) = self.server_invite_dialog_keys_by_tx.remove(transaction_id) {
            for key in keys {
                let generation = self.next_server_invite_dialog_deadline_generation();
                let mut should_schedule = false;
                if let Some(mut indexed) = self.server_invite_dialog_index.get_mut(&key) {
                    if indexed.transaction_id == *transaction_id {
                        indexed.expires_at = Some(expires_at);
                        indexed.deadline_generation = generation;
                        should_schedule = true;
                    }
                }

                if should_schedule {
                    self.schedule_server_invite_ack_expiry(ServerInviteAckExpiryEntry {
                        due_at: expires_at,
                        generation,
                        dialog_key: key,
                    });
                }
            }
        }
    }

    fn remove_server_invite_dialog_index_exact(&self, transaction_id: &TransactionKey) {
        let Some((_, keys)) = self.server_invite_dialog_keys_by_tx.remove(transaction_id) else {
            return;
        };
        for key in keys {
            self.server_invite_dialog_index
                .remove_if(&key, |_, indexed| indexed.transaction_id == *transaction_id);
        }
    }

    fn next_server_invite_dialog_deadline_generation(&self) -> u64 {
        self.server_invite_dialog_deadline_generation
            .fetch_add(1, Ordering::Relaxed)
    }

    fn schedule_server_invite_ack_expiry(&self, deadline: ServerInviteAckExpiryEntry) {
        self.server_invite_dialog_expiry_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(deadline);
    }

    fn remove_server_invite_dialog_reverse_key(
        &self,
        transaction_id: &TransactionKey,
        dialog_key: &ServerInviteDialogKey,
    ) {
        let mut became_empty = false;
        if let Some(mut keys) = self.server_invite_dialog_keys_by_tx.get_mut(transaction_id) {
            keys.retain(|key| key != dialog_key);
            became_empty = keys.is_empty();
        }
        if became_empty {
            self.server_invite_dialog_keys_by_tx
                .remove_if(transaction_id, |_, keys| keys.is_empty());
        }
    }

    async fn send_cached_response(
        &self,
        response: Response,
        wire_bytes: bytes::Bytes,
        route: TransportRoute,
        context: &'static str,
    ) -> std::result::Result<(), TransportError> {
        let destination = route.destination;
        match self
            .transport
            .send_message_raw_via(wire_bytes, route.clone())
            .await
        {
            Ok(()) => Ok(()),
            Err(TransportError::NotImplemented(reason)) => {
                trace!(
                    reason_present=true,
                    reason_len=reason.len(),
                    destination = %destination,
                    context,
                    "Cached response raw send unavailable; falling back to structured send"
                );
                self.transport
                    .send_message_via(Message::Response(response), route)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn cache_invite_2xx_response_for(&self, transaction_id: &TransactionKey) {
        let Some(tx) = self
            .server_transactions
            .get(transaction_id)
            .map(|entry| entry.value().clone())
        else {
            return;
        };

        if tx.kind() != TransactionKind::InviteServer {
            return;
        }

        let response = {
            let response_guard = tx.data().last_response.lock().await;
            response_guard.clone()
        };

        let Some(response) = response else {
            return;
        };

        if !response.status().is_success() {
            return;
        }

        let now = Instant::now();
        let wire_bytes =
            bytes::Bytes::from(rvoip_sip_core::Message::Response(response.clone()).to_bytes());
        self.insert_invite_2xx_response_cache_entry(
            transaction_id.clone(),
            Invite2xxResponseCacheEntry {
                response,
                wire_bytes,
                route: tx.data().response_route.clone(),
                created_at: now,
                acked_at: None,
                expires_at: now + INVITE_2XX_RESPONSE_CACHE_TTL,
                next_retransmit_at: now + self.timer_settings.t1,
                retransmit_interval: self.timer_settings.t1,
                deadline_generation: 0,
                _admission_owner: tx.data().transaction_admission_owner(),
            },
        );
    }

    fn insert_invite_2xx_response_cache_entry(
        &self,
        transaction_id: TransactionKey,
        mut entry: Invite2xxResponseCacheEntry,
    ) {
        let scheduled_len = {
            // All paths that modify both structures take the scheduler lock
            // before a cache shard. A maintenance candidate therefore either
            // observes this complete generation or an older generation, never
            // a half-installed cache/deadline pair.
            let mut scheduler = self
                .invite_2xx_response_due_queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            entry.deadline_generation = scheduler.schedule(
                transaction_id.clone(),
                entry.next_retransmit_at,
                entry.expires_at,
            );
            self.invite_2xx_response_cache.insert(transaction_id, entry);
            scheduler.len()
        };
        diagnostics::record_invite_2xx_cache_insert();

        // Overflow is normally at most one entry because every insertion
        // checks the exact scheduler length. The bounded prune also handles a
        // lowered runtime/test capacity without an unbounded catch-up pass.
        if scheduled_len > self.invite_2xx_response_cache_capacity {
            self.prune_invite_2xx_response_cache();
        }
    }

    pub(crate) async fn retransmit_cached_invite_2xx_response_on_route(
        &self,
        transaction_id: &TransactionKey,
        ingress_route: TransportRoute,
    ) -> Result<bool> {
        let entry = self
            .invite_2xx_response_cache
            .get(transaction_id)
            .map(|entry| entry.value().clone());

        let Some(entry) = entry else {
            return Ok(false);
        };

        if entry.is_expired(Instant::now()) {
            if self.remove_invite_2xx_response_cache_generation(
                transaction_id,
                entry.deadline_generation,
            ) {
                diagnostics::record_invite_2xx_cache_expired();
            }
            return Ok(false);
        }

        // A cached response remains bound to the exact authenticated ingress
        // flow that created it. Address equality is insufficient when TLS or
        // WSS virtual authorities share an IP:port; a duplicate on another
        // flow must pass through normal authorization instead of redirecting
        // cached bytes.
        if ingress_route != entry.route {
            return Ok(false);
        }

        diagnostics::record_duplicate_invite_cache_hit();
        let is_200_ok = entry.response.status().as_u16() == 200;

        self.send_cached_response(
            entry.response,
            entry.wire_bytes,
            entry.route,
            "Failed to retransmit cached INVITE 2xx",
        )
        .await
        .map_err(|e| Error::transport_error(e, "Failed to retransmit cached INVITE 2xx"))?;
        if is_200_ok {
            diagnostics::record_200_ok_invite_duplicate_cache();
        }

        Ok(true)
    }

    fn remove_invite_2xx_response_cache_generation(
        &self,
        transaction_id: &TransactionKey,
        generation: u64,
    ) -> bool {
        let mut scheduler = self
            .invite_2xx_response_due_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        scheduler.unschedule(transaction_id, generation);
        self.invite_2xx_response_cache
            .remove_if(transaction_id, |_, entry| {
                entry.deadline_generation == generation
            })
            .is_some()
    }

    fn maintenance_prune_retained_state(&self) {
        self.maintenance_prune_auxiliary_retained_state();
        // Diagnostic callers may request a synchronous snapshot. Keep that
        // assist bounded as well; the due-driven worker owns any remainder.
        self.prune_retired_client_transactions();
        self.prune_client_completions();
    }

    fn maintenance_prune_auxiliary_retained_state(&self) {
        self.expire_due_server_invite_dialog_index(
            Instant::now(),
            SERVER_INVITE_ACK_EXPIRY_BATCH_MAX,
        );
        self.prune_invite_2xx_response_cache();
        self.prune_closed_event_subscribers_now();
        self.prune_stale_pending_inbound_bytes();
        self.prune_stale_pending_inbound_principals();
    }

    fn prune_retired_client_transactions(&self) {
        let now = Instant::now();
        let retired_capacity = self
            .retired_client_transaction_capacity
            .load(Ordering::Acquire);
        let (removals, more_due) = {
            let mut deadlines = self
                .retired_client_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let removals = deadlines.take_due_and_overflow(
                now,
                retired_capacity,
                RETAINED_CLIENT_DEADLINE_BATCH_MAX,
            );
            let more_due = deadlines.has_due_or_overflow(now, retired_capacity);
            (removals, more_due)
        };

        for deadline in removals {
            if self
                .transaction_destinations
                .remove_if(deadline.transaction_id.as_ref(), |_, state| {
                    state.retired().is_some_and(|retired| {
                        retired.deadline_version == deadline.version
                            && retired.expires_at == deadline.expires_at
                    })
                })
                .is_some()
            {
                self.decrement_retired_client_transaction_count();
            }
        }
        if more_due {
            self.wake_retained_client_deadline_worker();
        }
    }

    fn prune_client_completions(&self) {
        let now = Instant::now();
        let (removals, more_due) = {
            let mut deadlines = self
                .client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let removals = deadlines.take_due_and_overflow(
                now,
                self.client_completion_capacity,
                RETAINED_CLIENT_DEADLINE_BATCH_MAX,
            );
            let more_due = deadlines.has_due_or_overflow(now, self.client_completion_capacity);
            (removals, more_due)
        };

        for deadline in removals {
            self.client_completions
                .remove_if(deadline.transaction_id.as_ref(), |_, completion| {
                    completion.retained_deadline() == Some((deadline.expires_at, deadline.version))
                });
        }
        if more_due {
            self.wake_retained_client_deadline_worker();
        }
    }

    fn retired_client_transaction_count_unpruned(&self) -> usize {
        self.retired_client_transaction_count
            .load(Ordering::Acquire)
    }

    fn retired_client_deadline_count(&self) -> usize {
        self.retired_client_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    fn decrement_retired_client_transaction_count(&self) {
        let _ = self.retired_client_transaction_count.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |count| Some(count.saturating_sub(1)),
        );
    }

    fn prune_closed_event_subscribers_now(&self) {
        if let Some(observers) = self.events_tx.observer_fanout() {
            observers.prune_closed_now();
        }
    }

    fn prune_stale_pending_inbound_bytes(&self) {
        if self.pending_inbound_bytes.is_empty() {
            return;
        }

        let now = Instant::now();
        let stale_keys: Vec<TransactionKey> = self
            .pending_inbound_bytes
            .iter()
            .filter(|entry| {
                self.pending_inbound_inserted_at
                    .get(entry.key())
                    .map(|inserted_at| {
                        now.saturating_duration_since(*inserted_at.value())
                            >= PENDING_INBOUND_BYTES_TTL
                    })
                    .unwrap_or(true)
            })
            .map(|entry| entry.key().clone())
            .collect();

        for key in stale_keys {
            let should_remove = self
                .pending_inbound_inserted_at
                .get(&key)
                .map(|inserted_at| {
                    now.saturating_duration_since(*inserted_at.value()) >= PENDING_INBOUND_BYTES_TTL
                })
                .unwrap_or(true);
            if should_remove {
                self.pending_inbound_bytes.remove(&key);
                self.pending_inbound_inserted_at.remove(&key);
                self.pending_inbound_transport.remove(&key);
                self.pending_inbound_timing.remove(&key);
            }
        }
    }

    fn prune_stale_pending_inbound_principals(&self) {
        if self.pending_inbound_principals.is_empty() {
            return;
        }

        let now = Instant::now();
        let stale_keys: Vec<TransactionKey> = self
            .pending_inbound_principals
            .iter()
            .filter(|entry| {
                self.pending_inbound_principal_inserted_at
                    .get(entry.key())
                    .map(|inserted_at| {
                        now.saturating_duration_since(inserted_at.value().inserted_at)
                            >= PENDING_INBOUND_PRINCIPAL_TTL
                    })
                    .unwrap_or(true)
            })
            .map(|entry| entry.key().clone())
            .collect();

        for key in stale_keys {
            let expired_lease =
                self.pending_inbound_principal_inserted_at
                    .remove_if(&key, |_, lease| {
                        now.saturating_duration_since(lease.inserted_at)
                            >= PENDING_INBOUND_PRINCIPAL_TTL
                    });
            if expired_lease.is_some() {
                self.pending_inbound_principals.remove(&key);
            }
        }
    }

    async fn retransmit_due_invite_2xx_responses(&self) -> usize {
        let started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let cache_len = self.invite_2xx_response_cache.len();
        let now = Instant::now();
        let max_due_per_tick = self
            .invite_2xx_retransmit_max_due_per_tick
            .load(Ordering::Relaxed)
            .max(1);
        let (due_queue_len, due_entries, capped) = {
            let mut scheduler = self
                .invite_2xx_response_due_queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let due_queue_len = scheduler.len();
            let (due_entries, capped) = scheduler.take_due(now, max_due_per_tick);
            (due_queue_len, due_entries, capped)
        };

        let scanned = due_entries.len();
        let mut expired_count = 0usize;
        let mut sends = Vec::new();

        for due_entry in due_entries {
            let mut expired = false;
            let mut send = None;

            {
                // The due record has already left the scheduler. Reacquire it
                // before the cache shard so a concurrent ACK/replacement uses
                // the same lock order and receives a distinct generation.
                let mut scheduler = self
                    .invite_2xx_response_due_queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(mut entry) = self
                    .invite_2xx_response_cache
                    .get_mut(due_entry.transaction_id.as_ref())
                {
                    if entry.deadline_generation != due_entry.generation {
                        // A replacement or ACK already installed a newer exact
                        // deadline while this bounded batch was being handled.
                    } else if entry.next_retransmit_at != due_entry.due_at
                        || entry.expires_at != due_entry.expires_at
                    {
                        // Defensive repair for state written by old snapshots
                        // or test instrumentation. Runtime writers update both
                        // indexes atomically under this lock.
                        entry.deadline_generation = scheduler.schedule(
                            due_entry.transaction_id.as_ref().clone(),
                            entry.next_retransmit_at,
                            entry.expires_at,
                        );
                    } else if entry.is_expired(now) {
                        expired = true;
                    } else if entry.acked_at.is_some() {
                        // ACKed entries normally have due_at == expires_at.
                        // Preserve exact retention even if a clock boundary
                        // made the due check win just before expiry.
                        entry.deadline_generation = scheduler.schedule(
                            due_entry.transaction_id.as_ref().clone(),
                            entry.expires_at,
                            entry.expires_at,
                        );
                    } else {
                        send = Some((
                            entry.response.clone(),
                            entry.wire_bytes.clone(),
                            entry.route.clone(),
                        ));
                        entry.retransmit_interval = entry
                            .retransmit_interval
                            .saturating_mul(2)
                            .min(self.timer_settings.t2);
                        entry.next_retransmit_at = now + entry.retransmit_interval;
                        entry.deadline_generation = scheduler.schedule(
                            due_entry.transaction_id.as_ref().clone(),
                            entry.next_retransmit_at,
                            entry.expires_at,
                        );
                    }
                }

                if expired
                    && self
                        .invite_2xx_response_cache
                        .remove_if(due_entry.transaction_id.as_ref(), |_, entry| {
                            entry.deadline_generation == due_entry.generation
                        })
                        .is_some()
                {
                    diagnostics::record_invite_2xx_cache_expired();
                    expired_count += 1;
                }
            }

            if let Some((response, wire_bytes, route)) = send {
                sends.push((response, wire_bytes, route));
            }
        }

        let mut retransmitted = 0usize;
        for (response, wire_bytes, route) in sends {
            let is_200_ok = response.status().as_u16() == 200;
            let send_started = diagnostics::transaction_timing_enabled().then(Instant::now);
            let destination = route.destination;
            match self
                .send_cached_response(
                    response,
                    wire_bytes,
                    route,
                    "Failed to retransmit cached INVITE 2xx response",
                )
                .await
            {
                Ok(()) => {
                    retransmitted += 1;
                    diagnostics::record_invite_2xx_proactive_retransmit();
                    if is_200_ok {
                        diagnostics::record_200_ok_invite_proactive_retransmit();
                    }
                    if let Some(started) = send_started {
                        diagnostics::record_invite_2xx_proactive_send(started.elapsed());
                    }
                }
                Err(e) => {
                    debug!(
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                        destination = %destination,
                        "Failed to retransmit cached INVITE 2xx response"
                    );
                }
            }
        }

        if let Some(started) = started {
            diagnostics::record_invite_2xx_maintenance(
                cache_len,
                due_queue_len,
                scanned,
                retransmitted,
                expired_count,
                capped,
                started.elapsed(),
            );
        }

        retransmitted
    }

    pub(crate) fn mark_invite_2xx_response_cache_acked(&self, transaction_id: &TransactionKey) {
        let now = Instant::now();
        let mut scheduler = self
            .invite_2xx_response_due_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mut entry) = self.invite_2xx_response_cache.get_mut(transaction_id) {
            if entry.acked_at.is_none() {
                diagnostics::record_invite_2xx_ack_removed(entry.created_at.elapsed());
                let retained_until =
                    (now + INVITE_2XX_ACKED_RESPONSE_RETENTION).min(entry.expires_at);
                entry.acked_at = Some(now);
                entry.expires_at = retained_until;
                entry.next_retransmit_at = retained_until;
                entry.deadline_generation =
                    scheduler.schedule(transaction_id.clone(), retained_until, retained_until);
            }
        }
    }

    pub(crate) fn remove_invite_2xx_response_cache(&self, transaction_id: &TransactionKey) {
        let mut scheduler = self
            .invite_2xx_response_due_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, entry)) = self.invite_2xx_response_cache.remove(transaction_id) {
            scheduler.unschedule(transaction_id, entry.deadline_generation);
        }
    }

    pub(crate) fn mark_transaction_terminated_indexed(&self, transaction_id: &TransactionKey) {
        // Synthetic compact Timer J/K lifecycle observations arrive after the
        // active runner has already been removed. Re-indexing those keys would
        // create a 2x event-rate cleanup backlog (and can outpace the bounded
        // 1 Hz repair pass). Only live active maps require this wake index.
        if self.client_transactions.contains_key(transaction_id)
            || self.server_transactions.contains_key(transaction_id)
        {
            self.terminated_transactions
                .insert(transaction_id.clone(), ());
        }
    }

    fn expire_due_server_invite_dialog_index(&self, now: Instant, max_work: usize) -> usize {
        if max_work == 0 {
            return 0;
        }

        let due = {
            let mut queue = self
                .server_invite_dialog_expiry_queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut due = Vec::with_capacity(max_work.min(queue.len()));
            while due.len() < max_work && queue.peek().is_some_and(|entry| entry.due_at <= now) {
                if let Some(deadline) = queue.pop() {
                    due.push(deadline);
                }
            }
            due
        };

        let processed = due.len();
        for deadline in due {
            self.server_invite_dialog_index
                .remove_if(&deadline.dialog_key, |_, current| {
                    current.deadline_generation == deadline.generation
                        && current.expires_at == Some(deadline.due_at)
                        && current.is_expired(now)
                });
        }
        processed
    }

    /// Full-map repair for explicit diagnostics only. Normal cleanup is
    /// exclusively driven by `server_invite_dialog_expiry_queue`.
    fn repair_expired_server_invite_dialog_index(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<(ServerInviteDialogKey, u64)> = self
            .server_invite_dialog_index
            .iter()
            .filter(|entry| entry.value().is_expired(now))
            .map(|entry| (entry.key().clone(), entry.value().deadline_generation))
            .collect();

        let mut removed = 0;
        for (key, generation) in expired {
            if self
                .server_invite_dialog_index
                .remove_if(&key, |_, current| {
                    current.deadline_generation == generation && current.is_expired(now)
                })
                .is_some()
            {
                removed += 1;
            }
        }
        removed
    }

    fn prune_invite_2xx_response_cache(&self) {
        let now = Instant::now();
        let max_work = self
            .invite_2xx_retransmit_max_due_per_tick
            .load(Ordering::Relaxed)
            .max(1);
        let removals = self
            .invite_2xx_response_due_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take_expired_and_overflow(now, self.invite_2xx_response_cache_capacity, max_work);

        for deadline in removals {
            if self
                .invite_2xx_response_cache
                .remove_if(deadline.transaction_id.as_ref(), |_, entry| {
                    entry.deadline_generation == deadline.generation
                })
                .is_some()
                && deadline.expires_at <= now
            {
                diagnostics::record_invite_2xx_cache_expired();
            }
        }
    }

    /// Cancel an active INVITE client transaction
    ///
    /// Creates a CANCEL request based on the original INVITE and creates
    /// a new client transaction to send it.
    ///
    /// Returns the transaction ID of the new CANCEL transaction.
    pub async fn cancel_invite_transaction(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> Result<TransactionKey> {
        self.cancel_invite_transaction_with_extras(invite_tx_id, Vec::new())
            .await
    }

    /// CANCEL with caller-supplied `extra_headers` (RFC 3326 `Reason:`,
    /// X-* application headers, etc.) appended after the RFC 3261
    /// §9.1 mandatory header copy from the targeted INVITE.
    pub async fn cancel_invite_transaction_with_extras(
        &self,
        invite_tx_id: &TransactionKey,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<TransactionKey> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped CANCEL composition".into()))
            }
            result = self.cancel_invite_transaction_with_extras_within_operation(
                invite_tx_id,
                extra_headers,
            ) => result,
        }
    }

    async fn cancel_invite_transaction_with_extras_within_operation(
        &self,
        invite_tx_id: &TransactionKey,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<TransactionKey> {
        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_tx_id), "Canceling invite transaction");

        // Check that this is an INVITE client transaction
        if invite_tx_id.method() != &Method::Invite || invite_tx_id.is_server() {
            return Err(Error::Other(format!(
                "Transaction {} is not an INVITE client transaction",
                invite_tx_id
            )));
        }

        // Get the original INVITE request
        // The active runner may already have been replaced by a compact
        // retired INVITE after an ambiguous transport error. The compatibility
        // accessor parses its immutable wire image lazily, preserving exact
        // CANCEL construction without retaining the heavy transaction.
        let invite_request = self.original_request(invite_tx_id).await?.ok_or_else(|| {
            Error::transaction_not_found(
                invite_tx_id.clone(),
                "CANCEL request template is unavailable",
            )
        })?;

        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_tx_id), "Got INVITE request for cancellation");

        // Create a CANCEL request from the INVITE
        let local_addr = self
            .transport
            .local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;

        // Use the method utility to create the CANCEL request
        let mut cancel_request = cancel::create_cancel_request(&invite_request, &local_addr)?;

        // SIP_API_DESIGN_2 §5.2 — append application extras after the
        // stack-managed slice (Via/From/To/CSeq/Call-ID/Max-Forwards).
        for hdr in extra_headers {
            cancel_request.headers.push(hdr);
        }

        // Log and validate the CANCEL request to help with debugging
        if let Err(e) = cancel::validate_cancel_request(&cancel_request) {
            warn!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(&cancel_request.method()), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "CANCEL request validation issue - proceeding anyway");
        }

        // CANCEL uses the complete INVITE route, including resolver-selected
        // transport/authority and the established opaque flow.
        let request_route = self.transaction_route(invite_tx_id).await.ok_or_else(|| {
            Error::Other(format!(
                "No transport route found for transaction {}",
                invite_tx_id
            ))
        })?;

        // Create a transaction for the CANCEL request
        let cancel_tx_id = self
            .create_client_transaction_on_route(cancel_request, request_route)
            .await?;

        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&cancel_tx_id), original_id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_tx_id), "Created CANCEL transaction");

        // Send the CANCEL request immediately
        self.send_request(&cancel_tx_id).await?;

        Ok(cancel_tx_id)
    }

    /// Creates a client transaction for a non-INVITE request.
    ///
    /// # Arguments
    /// * `request` - The non-INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error
    pub async fn create_non_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() == Method::Invite {
            return Err(Error::Other(
                "Cannot create non-INVITE transaction for INVITE request".to_string(),
            ));
        }

        self.create_client_transaction(request, destination).await
    }

    /// Creates a client transaction for an INVITE request.
    ///
    /// # Arguments
    /// * `request` - The INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error
    pub async fn create_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() != Method::Invite {
            return Err(Error::Other(
                "Cannot create INVITE transaction for non-INVITE request".to_string(),
            ));
        }

        self.create_client_transaction(request, destination).await
    }

    /// Get information about available transport types and their capabilities
    ///
    /// This method returns information about which transport types are available
    /// and their capabilities. This is useful for session-level components that
    /// need to know what transport options are available.
    pub fn get_transport_capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            supports_udp: self.transport.supports_udp(),
            supports_tcp: self.transport.supports_tcp(),
            supports_tls: self.transport.supports_tls(),
            supports_ws: self.transport.supports_ws(),
            supports_wss: self.transport.supports_wss(),
            local_addr: self.transport.local_addr().ok(),
            default_transport: self.transport.default_transport_type(),
        }
    }

    /// Get detailed information about a specific transport type
    ///
    /// This method returns detailed information about a specific transport type,
    /// such as connection status, local address, etc.
    pub fn get_transport_info(&self, transport_type: TransportType) -> Option<TransportInfo> {
        if !self.transport.supports_transport(transport_type) {
            return None;
        }

        Some(TransportInfo {
            transport_type,
            is_connected: self.transport.is_transport_connected(transport_type),
            local_addr: self.transport.get_transport_local_addr(transport_type).ok(),
            connection_count: self.transport.get_connection_count(transport_type),
        })
    }

    /// Check if a specific transport type is available
    pub fn is_transport_available(&self, transport_type: TransportType) -> bool {
        self.transport.supports_transport(transport_type)
    }

    /// Get network information for SDP generation
    ///
    /// This method returns network information that can be used for SDP generation,
    /// such as the local IP address and ports for different media types.
    pub fn get_network_info_for_sdp(&self) -> NetworkInfoForSdp {
        NetworkInfoForSdp {
            local_ip: self
                .transport
                .local_addr()
                .map(|addr| addr.ip())
                .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
            rtp_port_range: (10000, 20000), // Default port range, could be configurable
        }
    }

    /// Get the best transport type for a given URI
    ///
    /// This method analyzes a URI and returns the best transport type to use
    /// based on the URI scheme and available transports.
    pub fn get_best_transport_for_uri(&self, uri: &rvoip_sip_core::Uri) -> TransportType {
        // Determine the best transport based on the URI scheme
        let scheme = uri.scheme().to_string();

        match scheme.as_str() {
            "sips" => {
                if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    // Fallback to another secure transport if TLS is not available
                    if self.transport.supports_wss() {
                        TransportType::Wss
                    } else {
                        // Last resort: use any available transport
                        self.transport.default_transport_type()
                    }
                }
            }
            "ws" => {
                if self.transport.supports_ws() {
                    TransportType::Ws
                } else {
                    self.transport.default_transport_type()
                }
            }
            "wss" => {
                if self.transport.supports_wss() {
                    TransportType::Wss
                } else if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    self.transport.default_transport_type()
                }
            }
            // Default for "sip:" and any other schemes
            _ => self.transport.default_transport_type(),
        }
    }

    /// Get WebSocket connection status if available
    ///
    /// This method returns information about WebSocket connections if WebSocket
    /// transport is supported and enabled.
    pub fn get_websocket_status(&self) -> Option<WebSocketStatus> {
        if !self.transport.supports_ws() && !self.transport.supports_wss() {
            return None;
        }

        Some(WebSocketStatus {
            ws_connections: self.transport.get_connection_count(TransportType::Ws),
            wss_connections: self.transport.get_connection_count(TransportType::Wss),
            has_active_connection: self.transport.is_transport_connected(TransportType::Ws)
                || self.transport.is_transport_connected(TransportType::Wss),
        })
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Avoid trying to print the Mutex contents directly or requiring Debug on contents
        f.debug_struct("TransactionManager")
            .field("transport", &"Arc<dyn Transport>")
            .field("client_transactions", &"Arc<Mutex<HashMap<...>>>") // Indicate map exists
            .field("server_transactions", &"Arc<Mutex<HashMap<...>>>")
            .field(
                "terminated_transactions",
                &self.terminated_transactions.len(),
            )
            .field(
                "server_invite_dialog_index",
                &self.server_invite_dialog_index.len(),
            )
            .field("transaction_destinations", &"Arc<Mutex<HashMap<...>>>")
            .field(
                "retired_client_transactions",
                &self.retired_client_transaction_count_unpruned(),
            )
            .field(
                "retired_client_deadlines",
                &self.retired_client_deadline_count(),
            )
            .field("events_tx", &self.events_tx) // Sender might be Debug
            .field("event_subscribers", &"Arc<Mutex<Vec<Sender>>>")
            .field("transport_rx", &"Arc<Mutex<Receiver>>")
            .field("running", &self.running)
            .field("timer_settings", &self.timer_settings)
            .field("timer_manager", &"Arc<TimerManager>")
            .field("timer_factory", &"TimerFactory")
            .finish()
    }
}

fn mark_transaction_manager_received(event: &mut TransportEvent, received_at: Instant) {
    let TransportEvent::MessageReceived {
        timing: Some(timing),
        ..
    } = event
    else {
        return;
    };

    if let Some(forwarded_at) = timing.transport_manager_forwarded_at {
        udp_diagnostics::record_transport_manager_to_transaction(
            received_at.duration_since(forwarded_at),
        );
    }
    timing.transaction_manager_received_at = Some(received_at);
}
