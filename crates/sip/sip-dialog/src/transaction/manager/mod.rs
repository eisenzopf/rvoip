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

use std::collections::{BinaryHeap, HashSet, VecDeque};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use tokio::sync::{mpsc, Mutex};
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, trace, warn};

use rvoip_infra_common::events::cross_crate::SipTransportContext;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};
use rvoip_sip_transport::diagnostics as udp_diagnostics;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    Error as TransportError, Transport, TransportEvent, TransportReceiveTiming,
};

use crate::diagnostics;
use crate::transaction::client::{
    ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction,
};
use crate::transaction::error::{Error, Result};
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
    InternalTransactionCommand, Transaction, TransactionEvent, TransactionKey, TransactionKind,
    TransactionState, DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
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
    pub event_subscribers: usize,
    pub subscriber_to_transactions: usize,
    pub transaction_to_subscribers: usize,
    pub pending_inbound_bytes: usize,
    pub pending_inbound_transport: usize,
    pub pending_inbound_timing: usize,
}

#[derive(Clone)]
pub(crate) struct EventSubscriber {
    id: usize,
    sender: mpsc::Sender<TransactionEvent>,
    global: bool,
}

const MIN_TRANSACTION_INDEX_CAPACITY: usize = 1024;
const DEFAULT_TRANSACTION_DISPATCH_WORKERS: usize = 1;
pub const MAX_TRANSACTION_DISPATCH_WORKERS: usize = 64;
// Keep successful INVITE responses available long enough for high-load UAC
// retransmission windows. Entries are removed as soon as the 2xx ACK arrives,
// so this is a tail bound for lossy/missing-ACK calls, not a full call-volume
// retention period.
const INVITE_2XX_RESPONSE_CACHE_TTL: Duration = Duration::from_secs(90);
const INVITE_2XX_ACKED_RESPONSE_RETENTION: Duration = Duration::from_secs(2);
const PENDING_INBOUND_BYTES_TTL: Duration = Duration::from_secs(30);
const INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK: usize = 2048;
const MIN_INVITE_2XX_RESPONSE_CACHE_CAPACITY: usize = 65_536;
const TERMINATED_CLEANUP_BATCH_MAX: usize = 1024;
const TERMINATED_CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(100);
const TERMINATED_CLEANUP_MAX_ATTEMPTS: u16 = 50;
const TRANSACTION_DISPATCH_PRIORITY_BURST_MAX: usize = 64;

fn transaction_index_capacity(capacity: Option<usize>) -> usize {
    capacity.unwrap_or(100).max(MIN_TRANSACTION_INDEX_CAPACITY)
}

fn invite_2xx_response_cache_capacity(index_capacity: usize) -> usize {
    index_capacity.max(MIN_INVITE_2XX_RESPONSE_CACHE_CAPACITY)
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
    /// Active server transactions. Same pattern as `client_transactions`.
    server_transactions: Arc<DashMap<TransactionKey, Arc<dyn ServerTransaction>>>,
    /// Indexed queue of transactions that reached `Terminated`.
    /// The periodic cleanup task drains this instead of scanning every active
    /// transaction map on each tick; occasional full sweeps remain as a
    /// defensive fallback.
    terminated_transactions: Arc<DashMap<TransactionKey, ()>>,
    /// Fast dialog-id lookup for 2xx ACKs targeting INVITE server transactions.
    /// Entries remain briefly after transaction removal so end-to-end ACKs
    /// never need to scan active transactions.
    server_invite_dialog_index: Arc<DashMap<ServerInviteDialogKey, ServerInviteAckIndexEntry>>,
    /// Reverse index used to retire only the ACK keys belonging to a
    /// transaction, avoiding full-map scans during transaction cleanup.
    server_invite_dialog_keys_by_tx: Arc<DashMap<TransactionKey, Vec<ServerInviteDialogKey>>>,
    server_invite_dialog_index_capacity: usize,
    server_invite_dialog_index_insert_count: Arc<AtomicUsize>,
    /// Cache of successful INVITE responses for retransmitted INVITEs after
    /// the INVITE server transaction has entered the RFC 3261 2xx path.
    invite_2xx_response_cache: Arc<DashMap<TransactionKey, Invite2xxResponseCacheEntry>>,
    invite_2xx_response_cache_capacity: usize,
    invite_2xx_response_cache_insert_count: Arc<AtomicUsize>,
    invite_2xx_response_due_queue: Arc<std::sync::Mutex<BinaryHeap<Invite2xxDueEntry>>>,
    invite_2xx_response_due_sequence: Arc<AtomicU64>,
    terminated_cleanup_tx: Option<mpsc::Sender<TerminatedCleanupItem>>,
    /// Transaction destinations — `transaction_id → SocketAddr`.
    /// DashMap for sharded lock-free reads.
    transaction_destinations: Arc<DashMap<TransactionKey, SocketAddr>>,
    /// Event sender
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Additional event subscribers. ArcSwap so the broadcast hot path
    /// (every transaction state change, every retransmit) reads via a
    /// single atomic load instead of acquiring an async mutex. Writes
    /// (subscribe/unsubscribe) use copy-on-write RCU.
    event_subscribers: Arc<ArcSwap<Vec<EventSubscriber>>>,
    /// Maps subscribers to transactions they're interested in.
    /// DashMap — guards never held across `.await`.
    subscriber_to_transactions: Arc<DashMap<usize, Vec<TransactionKey>>>,
    /// Maps transactions to subscribers interested in them.
    /// DashMap — guards never held across `.await`.
    transaction_to_subscribers: Arc<DashMap<TransactionKey, Vec<usize>>>,
    /// Subscriber counter for assigning unique IDs. `AtomicUsize` —
    /// the previous `Mutex<usize>` only ever did fetch-and-increment.
    next_subscriber_id: Arc<AtomicUsize>,
    /// Transport message channel
    transport_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
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
    transaction_dispatch_workers: usize,
    transaction_dispatch_queue_capacity: usize,
    transaction_command_channel_capacity: usize,
    transaction_dispatch_priority_burst_max: Arc<AtomicUsize>,
    invite_2xx_retransmit_max_due_per_tick: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionIngressKind {
    Invite,
    Ack,
    Bye,
    Cancel,
    Other,
}

impl TransactionIngressKind {
    fn as_str(self) -> &'static str {
        match self {
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
    high: mpsc::Sender<QueuedTransactionDispatch>,
    normal: mpsc::Sender<QueuedTransactionDispatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionDispatchLane {
    High,
    Normal,
}

#[derive(Debug, Clone)]
struct TerminatedCleanupItem {
    transaction_id: TransactionKey,
    attempts: u16,
    due_at: Instant,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Invite2xxDueEntry {
    due_at: Instant,
    sequence: u64,
    transaction_id: TransactionKey,
}

impl Ord for Invite2xxDueEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .due_at
            .cmp(&self.due_at)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for Invite2xxDueEntry {
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
        _ => TransactionIngressKind::Other,
    }
}

fn transaction_dispatch_lane(kind: TransactionIngressKind) -> TransactionDispatchLane {
    match kind {
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
        let (high_tx, mut high_rx) =
            mpsc::channel::<QueuedTransactionDispatch>(per_worker_capacity);
        let (normal_tx, mut normal_rx) =
            mpsc::channel::<QueuedTransactionDispatch>(per_worker_capacity);
        let manager_for_worker = manager.clone();
        let priority_burst_max_for_worker = priority_burst_max.clone();
        tokio::spawn(async move {
            let mut high_burst_count = 0usize;
            while let Some(queued) = recv_transaction_dispatch_event(
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
                        high_rx.len() + normal_rx.len(),
                    );
                }
                process_transaction_dispatch_event(&manager_for_worker, queued).await;
            }
            debug!(worker_id, "Transaction dispatch worker terminated");
        });
        senders.push(TransactionDispatchWorkerSender {
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
    high_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    normal_rx: &mut mpsc::Receiver<QueuedTransactionDispatch>,
    high_burst_count: &mut usize,
    priority_burst_max: usize,
) -> Option<QueuedTransactionDispatch> {
    loop {
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
        TransactionDispatchLane::High => &dispatch_senders[worker_index].high,
        TransactionDispatchLane::Normal => &dispatch_senders[worker_index].normal,
    };

    match sender.try_send(queued) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(queued)) => {
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
        error!("Error handling transport message: {}", e);
    }
    if let Some(started) = handler_started {
        diagnostics::record_transaction_handler(kind.as_str(), started.elapsed());
    }
}

fn drain_due_cleanup_items(
    delayed: &mut VecDeque<TerminatedCleanupItem>,
    batch: &mut Vec<TerminatedCleanupItem>,
) {
    let now = Instant::now();
    while batch.len() < TERMINATED_CLEANUP_BATCH_MAX {
        let Some(item) = delayed.front() else {
            break;
        };
        if item.due_at > now {
            break;
        }
        if let Some(item) = delayed.pop_front() {
            batch.push(item);
        }
    }
}

impl TransactionManager {
    fn start_terminated_cleanup_worker(&self, mut rx: mpsc::Receiver<TerminatedCleanupItem>) {
        let manager = self.clone();
        tokio::spawn(async move {
            diagnostics::record_termination_cleanup_worker_spawned();
            let mut delayed = VecDeque::new();
            let mut rx_closed = false;
            let mut retry_tick = tokio::time::interval(TERMINATED_CLEANUP_RETRY_DELAY);
            retry_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

            loop {
                let mut batch = Vec::with_capacity(TERMINATED_CLEANUP_BATCH_MAX);
                drain_due_cleanup_items(&mut delayed, &mut batch);

                if batch.is_empty() {
                    tokio::select! {
                        item = rx.recv(), if !rx_closed => {
                            match item {
                                Some(item) => batch.push(item),
                                None => rx_closed = true,
                            }
                        }
                        _ = retry_tick.tick() => {}
                    }
                    drain_due_cleanup_items(&mut delayed, &mut batch);
                }

                while !rx_closed && batch.len() < TERMINATED_CLEANUP_BATCH_MAX {
                    match rx.try_recv() {
                        Ok(item) => batch.push(item),
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            rx_closed = true;
                            break;
                        }
                    }
                }

                if batch.is_empty() {
                    if rx_closed && delayed.is_empty() {
                        break;
                    }
                    continue;
                }

                manager
                    .process_terminated_cleanup_batch(batch, &mut delayed)
                    .await;
            }

            debug!("Terminated transaction cleanup worker stopped");
        });
    }

    fn enqueue_terminated_transaction_cleanup(&self, transaction_id: TransactionKey) {
        let item = TerminatedCleanupItem {
            transaction_id,
            attempts: 0,
            due_at: Instant::now(),
        };

        let Some(tx) = &self.terminated_cleanup_tx else {
            return;
        };

        match tx.try_send(item) {
            Ok(()) => diagnostics::record_termination_cleanup_enqueued(),
            Err(mpsc::error::TrySendError::Full(_)) => {
                diagnostics::record_termination_cleanup_queue_full();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    async fn process_terminated_cleanup_batch(
        &self,
        batch: Vec<TerminatedCleanupItem>,
        delayed: &mut VecDeque<TerminatedCleanupItem>,
    ) {
        diagnostics::record_termination_cleanup_batch(batch.len());

        for mut item in batch {
            diagnostics::record_termination_cleanup_in_flight(1);
            let ready = self.transaction_lifecycle_destroyed(&item.transaction_id);
            if ready || item.attempts >= TERMINATED_CLEANUP_MAX_ATTEMPTS {
                if !ready {
                    warn!(
                        transaction_id = %item.transaction_id,
                        attempts = item.attempts,
                        "Lifecycle cleanup timeout, forcing transaction removal"
                    );
                }
                diagnostics::record_termination_cleanup_poll_attempts(item.attempts as u64);
                self.remove_terminated_transaction(&item.transaction_id)
                    .await;
                diagnostics::record_termination_cleanup_removed();
            } else {
                item.attempts += 1;
                item.due_at = Instant::now() + TERMINATED_CLEANUP_RETRY_DELAY;
                delayed.push_back(item);
            }
            diagnostics::record_termination_cleanup_in_flight(-1);
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
            transaction_destinations: self.transaction_destinations.len(),
            event_subscribers: self.event_subscribers.load().len(),
            subscriber_to_transactions: self.subscriber_to_transactions.len(),
            transaction_to_subscribers: self.transaction_to_subscribers.len(),
            pending_inbound_bytes: self.pending_inbound_bytes.len(),
            pending_inbound_transport: self.pending_inbound_transport.len(),
            pending_inbound_timing: self.pending_inbound_timing.len(),
        }
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

    /// Install a forwarder for transport-side events (pong received,
    /// connection closed) that the RFC 5626 outbound-flow monitor in
    /// dialog-core needs to observe. Called by
    /// `DialogManager::with_global_events` once the consumer task is
    /// wired up; leaves the manager in a no-op state otherwise.
    pub async fn set_flow_event_sender(
        &self,
        sender: mpsc::Sender<crate::manager::outbound_flow::FlowTransportEvent>,
    ) {
        *self.flow_event_sender.write().await = Some(sender);
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
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let subscriber_to_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_to_subscribers = Arc::new(DashMap::with_capacity(index_capacity));
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
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::with_capacity(
                index_capacity.saturating_mul(2),
            )),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index_capacity: index_capacity,
            server_invite_dialog_index_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::with_capacity(invite_2xx_cache_capacity)),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_cache_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            invite_2xx_response_due_sequence: Arc::new(AtomicU64::new(0)),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: events_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

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
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let subscriber_to_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_to_subscribers = Arc::new(DashMap::with_capacity(index_capacity));
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
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::with_capacity(
                index_capacity.saturating_mul(2),
            )),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index_capacity: index_capacity,
            server_invite_dialog_index_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::with_capacity(invite_2xx_cache_capacity)),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_cache_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            invite_2xx_response_due_sequence: Arc::new(AtomicU64::new(0)),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: events_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

        // Start the message processing loop
        manager.start_terminated_cleanup_worker(terminated_cleanup_rx);
        manager.start_message_loop();

        Ok((manager, events_rx))
    }

    /// Creates a transaction manager synchronously (without async).
    ///
    /// This constructor is provided for contexts where async initialization
    /// isn't possible. It creates a minimal transaction manager with dummy
    /// channels that will need to be properly connected later.
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

        // Create the transaction manager using the default transport and event channel
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        let index_capacity = transaction_index_capacity(index_capacity.or(Some(events_capacity)));
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let transaction_dispatch_workers = transaction_dispatch_worker_count(dispatch_workers);
        let transaction_dispatch_queue_capacity =
            transaction_dispatch_queue_capacity(dispatch_queue_capacity, events_capacity);
        let (terminated_cleanup_tx, terminated_cleanup_rx) =
            mpsc::channel(index_capacity.max(TERMINATED_CLEANUP_BATCH_MAX));

        let client_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let subscriber_to_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_to_subscribers = Arc::new(DashMap::with_capacity(index_capacity));
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
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::with_capacity(
                index_capacity.saturating_mul(2),
            )),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index_capacity: index_capacity,
            server_invite_dialog_index_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::with_capacity(invite_2xx_cache_capacity)),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_cache_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            invite_2xx_response_due_sequence: Arc::new(AtomicU64::new(0)),
            terminated_cleanup_tx: Some(terminated_cleanup_tx),
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            transaction_dispatch_workers,
            transaction_dispatch_queue_capacity,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        };

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
        let (events_tx, _) = mpsc::channel(100); // Dummy receiver, will be ignored
        let index_capacity = transaction_index_capacity(None);
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let client_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_capacity));
        let event_subscribers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let subscriber_to_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_to_subscribers = Arc::new(DashMap::with_capacity(index_capacity));
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));
        let (_, transport_rx) = mpsc::channel(100); // Dummy channel
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(AtomicBool::new(false));

        // Create timer settings
        let timer_settings = timer_settings_opt.unwrap_or_default();

        // Create the timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        Self {
            transport,
            client_transactions,
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::with_capacity(
                index_capacity.saturating_mul(2),
            )),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index_capacity: index_capacity,
            server_invite_dialog_index_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::with_capacity(invite_2xx_cache_capacity)),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_cache_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            invite_2xx_response_due_sequence: Arc::new(AtomicU64::new(0)),
            terminated_cleanup_tx: None,
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: 100,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        }
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
        let invite_2xx_cache_capacity = invite_2xx_response_cache_capacity(index_capacity);
        let client_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let server_transactions = Arc::new(DashMap::with_capacity(index_capacity));

        // Setup timer manager
        let timer_settings = build_timer_settings();
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());

        // Initialize running state
        let running = Arc::new(AtomicBool::new(false));

        // Track destinations
        let transaction_destinations = Arc::new(DashMap::with_capacity(index_capacity));

        // Initialize subscriber-related fields
        let subscriber_to_transactions = Arc::new(DashMap::with_capacity(index_capacity));
        let transaction_to_subscribers = Arc::new(DashMap::with_capacity(index_capacity));
        let next_subscriber_id = Arc::new(AtomicUsize::new(0));

        Self {
            transport,
            events_tx,
            event_subscribers,
            client_transactions,
            server_transactions,
            terminated_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index: Arc::new(DashMap::with_capacity(
                index_capacity.saturating_mul(2),
            )),
            server_invite_dialog_keys_by_tx: Arc::new(DashMap::with_capacity(index_capacity)),
            server_invite_dialog_index_capacity: index_capacity,
            server_invite_dialog_index_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_cache: Arc::new(DashMap::with_capacity(invite_2xx_cache_capacity)),
            invite_2xx_response_cache_capacity: invite_2xx_cache_capacity,
            invite_2xx_response_cache_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_2xx_response_due_queue: Arc::new(std::sync::Mutex::new(BinaryHeap::new())),
            invite_2xx_response_due_sequence: Arc::new(AtomicU64::new(0)),
            terminated_cleanup_tx: None,
            timer_factory,
            flow_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            timer_manager,
            timer_settings,
            running,
            transaction_destinations,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Arc::new(Mutex::new(transport_rx)),
            sip_trace: None,
            pending_inbound_bytes: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_inserted_at: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_transport: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            pending_inbound_timing: Arc::new(dashmap::DashMap::with_capacity(index_capacity)),
            transaction_dispatch_workers: DEFAULT_TRANSACTION_DISPATCH_WORKERS,
            transaction_dispatch_queue_capacity: 10,
            transaction_command_channel_capacity: DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
            transaction_dispatch_priority_burst_max: Arc::new(AtomicUsize::new(
                TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
            )),
            invite_2xx_retransmit_max_due_per_tick: Arc::new(AtomicUsize::new(
                INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
            )),
        }
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
        debug!(%transaction_id, "TransactionManager::send_request - sending request");

        // Clone the Arc<dyn ClientTransaction> out of the shard so we
        // don't hold the DashMap guard across `initiate().await` —
        // previously a global mutex pinned the whole client_transactions
        // map for the duration of one in-flight send.
        let tx_arc = self
            .client_transactions
            .get(transaction_id)
            .map(|r| r.value().clone());
        let Some(tx) = tx_arc else {
            debug!(%transaction_id, "TransactionManager::send_request - transaction not found");
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "send_request - transaction not found",
            ));
        };
        debug!(%transaction_id, kind=?tx.kind(), state=?tx.state(), "TransactionManager::send_request - found transaction");

        let tx_kind = tx.kind();
        let _initial_state = tx.state();

        // First subscribe to events BEFORE initiating the transaction
        // so we don't miss any events that happen during initiation
        let mut event_rx = self.subscribe();

        // Use the TransactionExt trait to safely downcast
        use crate::transaction::client::TransactionExt;

        if let Some(client_tx) = tx.as_client_transaction() {
            debug!(%transaction_id, "TransactionManager::send_request - initiating client transaction");

            // We're holding a per-tx Arc; the DashMap shard guard
            // released when `.get()` returned above.
            let result = client_tx.initiate().await;
            debug!(%transaction_id, success=?result.is_ok(), "TransactionManager::send_request - initiate result");

            if let Err(e) = result {
                debug!(%transaction_id, error=%e, "TransactionManager::send_request - initiate failed immediately");
                return Err(e);
            }

            let current_state = tx.state();
            if current_state == TransactionState::Terminated {
                if tx_kind == TransactionKind::InviteClient {
                    if tx
                        .last_response()
                        .await
                        .is_some_and(|response| response.status().is_success())
                    {
                        debug!(%transaction_id, "INVITE client terminated during initiate - treating as success (fast 2xx)");
                        return Ok(());
                    }
                }
                debug!(%transaction_id, "Transaction terminated immediately during initiate - likely transport error");
                return Err(Error::transport_error(
                    rvoip_sip_transport::Error::ProtocolError(
                        "Transaction terminated immediately".into(),
                    ),
                    "Failed to send request - transaction terminated immediately",
                ));
            }

            // Now wait for a short time to catch any asynchronous errors
            // We'll use a timeout to avoid hanging if no events are received
            let timeout_duration = tokio::time::Duration::from_millis(100);

            match tokio::time::timeout(timeout_duration, async {
                // Wait for events until timeout
                while let Some(event) = event_rx.recv().await {
                    match event {
                        TransactionEvent::TransportError { transaction_id: tx_id, .. } if tx_id == *transaction_id => {
                            debug!(%transaction_id, "Received TransportError event");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transport error during request send".into()),
                                "Failed to send request - transport error"
                            ));
                        },
                        TransactionEvent::StateChanged { transaction_id: tx_id, previous_state, new_state }
                            if tx_id == *transaction_id => {
                            debug!(%transaction_id, previous=?previous_state, new=?new_state, "Transaction state changed");

                            // If transaction moved directly to Terminated state.
                            // For an INVITE client, Calling → Terminated is the
                            // RFC 3261 §17.1.1.2 path for a 2xx final response —
                            // that's success, not a transport error. For
                            // non-INVITE, Trying/Calling → Terminated within
                            // the 100 ms window is unusual; the normal path is
                            // Trying → Completed → Terminated (after Timer K).
                            if new_state == TransactionState::Terminated {
                                let is_invite_2xx_path = tx_kind == TransactionKind::InviteClient
                                    && previous_state == TransactionState::Calling;
                                if is_invite_2xx_path {
                                    debug!(%transaction_id, "INVITE client Calling → Terminated (RFC 3261 §17.1.1.2 2xx) - treating as success");
                                    return Ok(());
                                }
                                if previous_state == TransactionState::Initial
                                    || previous_state == TransactionState::Calling
                                    || previous_state == TransactionState::Trying
                                {
                                    debug!(%transaction_id, kind=?tx_kind, ?previous_state, "Transaction moved to Terminated state - likely transport error");
                                    return Err(Error::transport_error(
                                        rvoip_sip_transport::Error::ProtocolError("Transaction terminated unexpectedly".into()),
                                        "Failed to send request - transaction terminated"
                                    ));
                                }
                            }
                        },
                        _ => {} // Ignore other events
                    }
                }

                // Check final transaction state via the DashMap.
                if let Some(entry) = self.client_transactions.get(transaction_id) {
                    let final_state = entry.value().state();
                    if final_state == TransactionState::Terminated {
                        debug!(%transaction_id, "Transaction is terminated after events processed");
                        return Err(Error::transport_error(
                            rvoip_sip_transport::Error::ProtocolError("Transaction terminated after processing".into()),
                            "Failed to send request - transaction terminated"
                        ));
                    }
                } else {
                    debug!(%transaction_id, "Transaction was removed - likely due to termination");
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::ProtocolError("Transaction was removed".into()),
                        "Failed to send request - transaction removed"
                    ));
                }

                Ok(())
            }).await {
                // Timeout occurred — recv loop drained or 100 ms elapsed.
                Err(_) => {
                    if let Some(entry) = self.client_transactions.get(transaction_id) {
                        let final_state = entry.value().state();
                        if final_state == TransactionState::Terminated {
                            // For INVITE, Calling → Terminated is the 2xx path
                            // (RFC 3261 §17.1.1.2). For non-INVITE, normal flow
                            // is Trying → Completed → Terminated only after
                            // Timer K (5 s for UDP), so Terminated this fast
                            // is suspicious.
                            if tx_kind == TransactionKind::InviteClient {
                                debug!(%transaction_id, "INVITE client terminated within 100 ms safety wait - treating as success (fast 2xx)");
                                return Ok(());
                            }
                            debug!(%transaction_id, "Non-INVITE terminated within 100 ms safety wait - likely transport error");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transaction terminated after timeout".into()),
                                "Failed to send request - transaction terminated"
                            ));
                        }

                        debug!(%transaction_id, state=?final_state, "Transaction still exists and is not terminated after timeout");
                        Ok(())
                    } else {
                        // Transaction was removed. For INVITE, a fast 2xx
                        // legitimately removes the transaction quickly. For
                        // non-INVITE, this would be abnormal in 100 ms.
                        if tx_kind == TransactionKind::InviteClient {
                            debug!(%transaction_id, "INVITE client transaction removed within 100 ms safety wait - treating as success (fast 2xx)");
                            Ok(())
                        } else {
                            debug!(%transaction_id, "Non-INVITE transaction was removed after timeout");
                            Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transaction was removed after timeout".into()),
                                "Failed to send request - transaction removed"
                            ))
                        }
                    }
                },
                // Got a result from the event processing
                Ok(result) => result,
            }
        } else {
            debug!(%transaction_id, "TransactionManager::send_request - failed to downcast to client transaction");
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
        self.transaction_destinations
            .get(transaction_id)
            .map(|r| *r.value())
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
        let id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        // ArcSwap RCU.
        self.event_subscribers.rcu(|current| {
            let mut next = Vec::with_capacity(current.len() + 1);
            next.extend(current.iter().cloned());
            next.push(EventSubscriber {
                id,
                sender: tx.clone(),
                global: true,
            });
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
        // Validate that transaction exists
        if !self.transaction_exists(transaction_id).await {
            return Err(Error::transaction_not_found(
                transaction_id.clone(),
                "subscribe_to_transaction - transaction not found",
            ));
        }

        let (tx, rx) = mpsc::channel(100);

        let subscriber_id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        // Add to global subscribers list — ArcSwap RCU.
        self.event_subscribers.rcu(|current| {
            let mut next = Vec::with_capacity(current.len() + 1);
            next.extend(current.iter().cloned());
            next.push(EventSubscriber {
                id: subscriber_id,
                sender: tx.clone(),
                global: false,
            });
            next
        });

        self.transaction_to_subscribers
            .entry(transaction_id.clone())
            .or_insert_with(Vec::new)
            .push(subscriber_id);

        self.subscriber_to_transactions
            .entry(subscriber_id)
            .or_insert_with(Vec::new)
            .push(transaction_id.clone());

        debug!(%transaction_id, subscriber_id, "Added transaction-specific subscriber");

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
        // Validate that all transactions exist
        for tx_id in transaction_ids {
            if !self.transaction_exists(tx_id).await {
                return Err(Error::transaction_not_found(
                    tx_id.clone(),
                    "subscribe_to_transactions - transaction not found",
                ));
            }
        }

        let (tx, rx) = mpsc::channel(100);

        let subscriber_id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        // Add to global subscribers list — ArcSwap RCU.
        self.event_subscribers.rcu(|current| {
            let mut next = Vec::with_capacity(current.len() + 1);
            next.extend(current.iter().cloned());
            next.push(EventSubscriber {
                id: subscriber_id,
                sender: tx.clone(),
                global: false,
            });
            next
        });

        for tx_id in transaction_ids {
            self.transaction_to_subscribers
                .entry(tx_id.clone())
                .or_insert_with(Vec::new)
                .push(subscriber_id);
        }

        {
            let mut entry = self
                .subscriber_to_transactions
                .entry(subscriber_id)
                .or_insert_with(Vec::new);
            for tx_id in transaction_ids {
                entry.push(tx_id.clone());
            }
        }

        debug!(
            subscriber_id,
            transaction_count = transaction_ids.len(),
            "Added multi-transaction subscriber"
        );

        Ok(rx)
    }

    /// Shutdown the transaction manager gracefully - BOTTOM-UP
    ///
    /// This performs a graceful shutdown in BOTTOM-UP order:
    /// 1. Close the transport layer (UDP) first
    /// 2. Stop the message processing loop
    /// 3. Drain any remaining messages
    /// 4. Clear active transactions
    /// 5. Clear event subscribers
    pub async fn shutdown(&self) {
        info!("TransactionManager shutting down gracefully");

        // Step 1: Stop the message processing loop FIRST.
        // AtomicBool — was an async Mutex<bool> before the perf pass.
        self.running.store(false, Ordering::Relaxed);
        debug!("Message processing loop signaled to stop");

        // Step 2: Transport should already be closed by this point via events
        // But ensure it's closed just in case
        if let Err(e) = self.transport.close().await {
            debug!("Transport close during shutdown: {}", e);
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
        for tx in in_flight_clients {
            let _ = tx
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate);
        }
        for tx in in_flight_servers {
            let _ = tx
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate);
        }

        // Step 3: Small drain period for in-flight messages
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
        self.server_transactions.clear();
        self.terminated_transactions.clear();
        self.server_invite_dialog_index.clear();
        self.server_invite_dialog_keys_by_tx.clear();
        self.invite_2xx_response_cache.clear();
        if let Ok(mut queue) = self.invite_2xx_response_due_queue.lock() {
            queue.clear();
        }
        self.transaction_destinations.clear();
        self.pending_inbound_bytes.clear();
        self.pending_inbound_inserted_at.clear();
        self.pending_inbound_transport.clear();
        self.pending_inbound_timing.clear();

        // Step 5: Emit TransactionEvent::ShutdownComplete
        // Broadcast to all event subscribers
        Self::broadcast_event(
            TransactionEvent::ShutdownComplete,
            &self.events_tx,
            &self.event_subscribers,
            Some(&self.subscriber_to_transactions),
            Some(&self.transaction_to_subscribers),
            Some(self.clone()),
        )
        .await;

        // Step 5: Clear event subscribers
        self.event_subscribers.store(Arc::new(Vec::new()));
        self.subscriber_to_transactions.clear();
        self.transaction_to_subscribers.clear();

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
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<ArcSwap<Vec<EventSubscriber>>>,
        subscriber_to_transactions: Option<&Arc<DashMap<usize, Vec<TransactionKey>>>>,
        transaction_to_subscribers: Option<&Arc<DashMap<TransactionKey, Vec<usize>>>>,
        manager: Option<TransactionManager>,
    ) {
        let broadcast_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        // Extract transaction ID from the event for filtering
        let transaction_id = match &event {
            TransactionEvent::StateChanged { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::SuccessResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::FailureResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::TransactionTerminated { transaction_id } => Some(transaction_id),
            TransactionEvent::TimerTriggered { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::InviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelRequest { transaction_id, .. } => Some(transaction_id),
            // These events don't have a specific transaction ID
            TransactionEvent::StrayResponse { .. } => None,
            TransactionEvent::StrayAck { .. } => None,
            TransactionEvent::StrayCancel { .. } => None,
            TransactionEvent::StrayAckRequest { .. } => None,
            // Add other event types for completeness
            _ => None,
        };

        // Get list of interested subscribers for this transaction
        let interested_subscribers = if let (Some(tx_id), Some(tx_to_subs_map)) =
            (transaction_id, transaction_to_subscribers)
        {
            tx_to_subs_map
                .get(tx_id)
                .map(|r| r.value().clone())
                .unwrap_or_default()
        } else {
            Vec::new() // No specific subscribers for this transaction or global event
        };

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
                warn!("Failed to send event to primary channel: {}", e);
            }
        }

        // Send to interested subscribers only. ArcSwap::load is a
        // single atomic load — no mutex acquire on the hot path.
        let subs_snapshot = subscribers.load();
        let subs: &Vec<EventSubscriber> = &subs_snapshot;
        let mut closed_subscribers = Vec::new();

        // If we have transaction-specific subscribers, filter events
        if transaction_to_subscribers.is_some() {
            for sub in subs.iter() {
                // Send to this subscriber if:
                // 1. It subscribed globally
                // 2. It is explicitly interested in this transaction
                // 3. The event is not tied to a transaction and the subscriber is global
                let should_send = sub.global
                    || transaction_id.is_some_and(|_| interested_subscribers.contains(&sub.id));

                if should_send {
                    if let Err(e) = sub.sender.send(event.clone()).await {
                        closed_subscribers.push(sub.id);
                        // During shutdown, channel closed errors are expected - use debug level
                        // Check if this is a channel closed error during shutdown
                        if e.to_string().contains("channel closed") {
                            debug!(
                                "Subscriber {} channel closed during shutdown (expected)",
                                sub.id
                            );
                        } else {
                            warn!("Failed to send event to subscriber {}: {}", sub.id, e);
                        }
                    }
                }
            }
        } else {
            // No transaction filtering, send to all (backward compatibility)
            for sub in subs.iter() {
                if let Err(e) = sub.sender.send(event.clone()).await {
                    closed_subscribers.push(sub.id);
                    // During shutdown, channel closed errors are expected - use debug level
                    if e.to_string().contains("channel closed") {
                        debug!(
                            "Subscriber {} channel closed during shutdown (expected)",
                            sub.id
                        );
                    } else {
                        warn!("Failed to send event to subscriber {}: {}", sub.id, e);
                    }
                }
            }
        }

        Self::prune_event_subscribers(
            subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            &closed_subscribers,
        );

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

    fn prune_event_subscribers(
        subscribers: &Arc<ArcSwap<Vec<EventSubscriber>>>,
        subscriber_to_transactions: Option<&Arc<DashMap<usize, Vec<TransactionKey>>>>,
        transaction_to_subscribers: Option<&Arc<DashMap<TransactionKey, Vec<usize>>>>,
        closed_subscriber_ids: &[usize],
    ) {
        let explicit_closed: HashSet<usize> = closed_subscriber_ids.iter().copied().collect();
        let snapshot = subscribers.load();
        let removed_ids: Vec<usize> = snapshot
            .iter()
            .filter(|subscriber| {
                explicit_closed.contains(&subscriber.id) || subscriber.sender.is_closed()
            })
            .map(|subscriber| subscriber.id)
            .collect();

        if removed_ids.is_empty() {
            return;
        }

        let removed: HashSet<usize> = removed_ids.iter().copied().collect();
        subscribers.rcu(|current| {
            let mut next = Vec::with_capacity(current.len().saturating_sub(removed.len()));
            next.extend(
                current
                    .iter()
                    .filter(|subscriber| !removed.contains(&subscriber.id))
                    .cloned(),
            );
            next
        });

        for subscriber_id in removed_ids {
            Self::remove_subscriber_indexes(
                subscriber_id,
                subscriber_to_transactions,
                transaction_to_subscribers,
            );
        }
    }

    fn remove_subscriber_indexes(
        subscriber_id: usize,
        subscriber_to_transactions: Option<&Arc<DashMap<usize, Vec<TransactionKey>>>>,
        transaction_to_subscribers: Option<&Arc<DashMap<TransactionKey, Vec<usize>>>>,
    ) {
        let Some(subscriber_to_transactions) = subscriber_to_transactions else {
            return;
        };

        let Some((_, transaction_ids)) = subscriber_to_transactions.remove(&subscriber_id) else {
            return;
        };

        let Some(transaction_to_subscribers) = transaction_to_subscribers else {
            return;
        };

        for transaction_id in transaction_ids {
            let mut empty = false;
            if let Some(mut entry) = transaction_to_subscribers.get_mut(&transaction_id) {
                entry.value_mut().retain(|id| *id != subscriber_id);
                empty = entry.value().is_empty();
            }
            if empty {
                transaction_to_subscribers.remove(&transaction_id);
            }
        }
    }

    /// Actually remove a terminated transaction from all maps
    async fn remove_terminated_transaction(&self, transaction_id: &TransactionKey) {
        debug!(%transaction_id, "Removing terminated transaction after grace period");

        let mut terminated = false;

        if self.client_transactions.remove(transaction_id).is_some() {
            debug!(%transaction_id, "Removed terminated client transaction");
            terminated = true;
        }

        // Defensive: also remove from server in case of duplication.
        if self.server_transactions.remove(transaction_id).is_some() {
            self.retire_server_invite_dialog_index_for(transaction_id);
            debug!(%transaction_id, "Removed terminated server transaction");
            terminated = true;
        }

        if self
            .transaction_destinations
            .remove(transaction_id)
            .is_some()
        {
            debug!(%transaction_id, "Removed transaction from destinations map");
        }
        self.terminated_transactions.remove(transaction_id);
        self.pending_inbound_bytes.remove(transaction_id);
        self.pending_inbound_inserted_at.remove(transaction_id);
        self.pending_inbound_transport.remove(transaction_id);
        self.pending_inbound_timing.remove(transaction_id);

        // **CRITICAL FIX**: Clean up subscriber mappings to prevent memory leak
        if let Some((_, subscriber_ids)) = self.transaction_to_subscribers.remove(transaction_id) {
            debug!(%transaction_id, subscriber_count = subscriber_ids.len(), "Removed transaction from subscriber mappings");

            for subscriber_id in subscriber_ids {
                let mut empty = false;
                if let Some(mut entry) = self.subscriber_to_transactions.get_mut(&subscriber_id) {
                    let tx_list = entry.value_mut();
                    tx_list.retain(|tx_id| tx_id != transaction_id);
                    empty = tx_list.is_empty();
                }
                if empty {
                    self.subscriber_to_transactions.remove(&subscriber_id);
                    debug!(%transaction_id, subscriber_id, "Removed empty subscriber mapping");
                }
            }
        }

        // Unregister from timer manager (defensive - it should auto-unregister)
        let unregister_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        self.timer_manager
            .unregister_transaction(transaction_id)
            .await;
        if let Some(started) = unregister_started {
            diagnostics::record_termination_cleanup_timer_unregister(started.elapsed());
        }
        debug!(%transaction_id, "Unregistered transaction from timer manager");

        if terminated {
            debug!(%transaction_id, "Successfully cleaned up terminated transaction");
        } else {
            debug!(%transaction_id, "Transaction not found for termination - may have been already removed");
        }
    }

    /// Start the message processing loop for handling incoming transport events
    fn start_message_loop(&self) {
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();
        let manager_arc = self.clone();
        let dispatch_workers = self.transaction_dispatch_workers;
        let dispatch_queue_capacity = self.transaction_dispatch_queue_capacity;
        let dispatch_priority_burst_max = self.transaction_dispatch_priority_burst_max.clone();

        tokio::spawn(async move {
            debug!("Starting transaction message loop");

            // Create a separate channel to receive events from transactions
            let (internal_tx, mut internal_rx) = mpsc::channel(100);
            let _internal_tx = internal_tx;

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

            // Set running flag (AtomicBool — single store instruction)
            running.store(true, Ordering::Relaxed);

            // Get the transport receiver
            let mut receiver = transport_rx.lock().await;
            let mut cleanup_interval = tokio::time::interval(Duration::from_secs(1));
            cleanup_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let cleanup_running = Arc::new(AtomicBool::new(false));
            let mut cleanup_ticks = 0usize;
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

                // Use tokio::select to wait for a message from either the transport or internal channel
                tokio::select! {
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
                                    error!("Error handling transport message: {}", e);
                                }
                            }
                        } else {
                            debug!("Skipping transport event processing - shutting down");
                        }
                    }
                    Some(transaction_event) = internal_rx.recv() => {
                        // Handle transaction events, particularly termination events
                        Self::broadcast_event(
                            transaction_event,
                            &events_tx,
                            &event_subscribers,
                            Some(&manager_arc.subscriber_to_transactions),
                            Some(&manager_arc.transaction_to_subscribers),
                            Some(manager_arc.clone()),
                        ).await;
                    }
                    _ = cleanup_interval.tick() => {
                        cleanup_ticks = cleanup_ticks.wrapping_add(1);
                        let full_sweep = cleanup_ticks % 10 == 0;
                        if cleanup_running
                            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                            .is_ok()
                        {
                            let manager_clone = manager_arc.clone();
                            let cleanup_running = cleanup_running.clone();
                            tokio::spawn(async move {
                                manager_clone.maintenance_prune_retained_state();
                                let cleanup_result = if full_sweep {
                                    manager_clone.cleanup_terminated_transactions().await
                                } else {
                                    manager_clone.cleanup_indexed_terminated_transactions().await
                                };

                                match cleanup_result {
                                    Ok(count) if count > 0 => {
                                        debug!(
                                            full_sweep,
                                            "Periodic cleanup removed {} terminated transactions",
                                            count
                                        );
                                    }
                                    Err(e) => {
                                        error!("Periodic transaction cleanup failed: {}", e);
                                    }
                                    _ => {}
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
                                let count = manager_clone.retransmit_due_invite_2xx_responses().await;
                                if count > 0 {
                                    trace!("Retransmitted {} cached INVITE 2xx responses", count);
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
        debug!(method=%request.method(), destination=%destination, "Creating client transaction");

        // Debug the Via headers in the request
        tracing::trace!("Request Via headers before transaction creation:");
        for (i, via) in request.via_headers().iter().enumerate() {
            tracing::trace!("  Via[{}]: {}", i, via);
        }

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
                let top_route = top_route_uri(&modified_request)
                    .map(|uri| uri.to_string())
                    .unwrap_or_else(|| "<none>".to_string());
                info!(
                    method = %modified_request.method(),
                    uri = %modified_request.uri(),
                    top_route = %top_route,
                    next_hop = %next_hop_uri_for_request(&modified_request),
                    destination = %destination,
                    top_via = %via,
                    "RVOIP_SIP_DIAG final outgoing request after transaction normalization"
                );
            }
        }

        tracing::trace!("Request Via headers after potential modification:");
        for (i, via) in modified_request.via_headers().iter().enumerate() {
            tracing::trace!("  Via[{}]: {}", i, via);
        }

        rvoip_sip_core::validation::validate_wire_request(&modified_request)?;

        // Create the appropriate transaction. Returns Arc<dyn ClientTransaction>
        // so the map can shard (DashMap) and call sites can clone the
        // Arc out before any `.await`.
        let transaction: ArcClientTransaction = match modified_request.method() {
            Method::Invite => {
                tracing::trace!("Creating ClientInviteTransaction: {}", key);
                let tx = ClientInviteTransaction::new_with_command_channel_capacity(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request),
                    self.transaction_command_channel_capacity,
                )?;
                tracing::trace!("Created ClientInviteTransaction: {}", key);
                Arc::new(tx)
            }
            Method::Cancel => {
                if let Err(e) = cancel::validate_cancel_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }
                let tx = ClientNonInviteTransaction::new_with_command_channel_capacity(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request),
                    self.transaction_command_channel_capacity,
                )?;
                Arc::new(tx)
            }
            Method::Update => {
                if let Err(e) = update::validate_update_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }
                let tx = ClientNonInviteTransaction::new_with_command_channel_capacity(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request),
                    self.transaction_command_channel_capacity,
                )?;
                Arc::new(tx)
            }
            _ => {
                let tx = ClientNonInviteTransaction::new_with_command_channel_capacity(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request),
                    self.transaction_command_channel_capacity,
                )?;
                Arc::new(tx)
            }
        };

        // Store the transaction + destination
        self.client_transactions.insert(key.clone(), transaction);
        self.transaction_destinations
            .insert(key.clone(), destination);

        debug!(id=%key, "Created client transaction");

        if request.method() == Method::Cancel {
            debug!(id=%key, original_id=%branch, "Created CANCEL transaction");
        }

        Ok(key)
    }

    /// Creates and sends an ACK request for a 2xx response to an INVITE.
    pub async fn send_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        // Create the ACK request
        let ack_request = self.create_ack_for_2xx(invite_tx_id, response).await?;

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
            match self.transaction_destinations.get(invite_tx_id) {
                Some(entry) => *entry.value(),
                None => {
                    return Err(Error::Other(format!(
                        "Destination for transaction {:?} not found",
                        invite_tx_id
                    )));
                }
            }
        };

        // Send the ACK directly without creating a transaction
        rvoip_sip_core::validation::validate_wire_request(&ack_request)?;
        self.transport
            .send_message(Message::Request(ack_request), destination)
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
            Message::Request(_) => self.server_transactions.contains_key(&key),
            Message::Response(_) => self.client_transactions.contains_key(&key),
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

        // Get the original INVITE request
        let invite_request =
            utils::get_transaction_request(&self.client_transactions, invite_tx_id).await?;

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
    pub async fn create_server_transaction(
        &self,
        request: Request,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerTransaction>> {
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

        // Check if this is a retransmission of an existing transaction.
        // Extract the Arc out of the shard before awaiting `process_request`.
        let existing = self
            .server_transactions
            .get(&key)
            .map(|r| r.value().clone());
        if let Some(transaction) = existing {
            transaction.process_request(request.clone()).await?;
            debug!(id=%key, method=%request.method(), "Processed retransmitted request in existing transaction");
            return Ok(transaction);
        }

        // Create a new transaction based on the request method
        let transaction: Arc<dyn ServerTransaction> = match request.method() {
            Method::Invite => {
                let tx = Arc::new(ServerInviteTransaction::new_with_command_channel_capacity(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    Some(self.timer_settings.clone()),
                    self.transaction_command_channel_capacity,
                )?);

                info!(id=%tx.id(), method=%request.method(), "Created new ServerInviteTransaction");
                self.index_server_invite_dialog(&request, &key);
                tx
            }
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }

                // For CANCEL, the matching INVITE has the same transaction
                // key with method rewritten to INVITE. Keep this indexed
                // instead of scanning all active transactions.
                let invite_tx_id = key.with_method(Method::Invite);
                let target_invite_tx_id = if self.client_transactions.contains_key(&invite_tx_id)
                    || self.server_transactions.contains_key(&invite_tx_id)
                {
                    debug!(method=%request.method(), "Found matching INVITE transaction for CANCEL");
                    Some(invite_tx_id)
                } else {
                    debug!(method=%request.method(), "No matching INVITE transaction found for CANCEL");
                    None
                };

                // Create a non-INVITE server transaction for CANCEL
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_command_channel_capacity(
                        key.clone(),
                        request.clone(),
                        remote_addr,
                        self.transport.clone(),
                        self.events_tx.clone(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                    )?,
                );

                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for CANCEL");

                // If we found a matching INVITE transaction, notify the TU
                if let Some(invite_tx_id) = target_invite_tx_id {
                    self.events_tx
                        .send(TransactionEvent::CancelRequest {
                            transaction_id: tx.id().clone(),
                            target_transaction_id: invite_tx_id,
                            request: request.clone(),
                            source: remote_addr,
                        })
                        .await
                        .ok();
                }

                tx
            }
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }

                // Create a non-INVITE server transaction for UPDATE
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_command_channel_capacity(
                        key.clone(),
                        request.clone(),
                        remote_addr,
                        self.transport.clone(),
                        self.events_tx.clone(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                    )?,
                );

                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for UPDATE");
                tx
            }
            _ => {
                let tx = Arc::new(
                    ServerNonInviteTransaction::new_with_command_channel_capacity(
                        key.clone(),
                        request.clone(),
                        remote_addr,
                        self.transport.clone(),
                        self.events_tx.clone(),
                        Some(self.timer_settings.clone()),
                        self.transaction_command_channel_capacity,
                    )?,
                );

                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction");
                tx
            }
        };

        // Store the transaction
        self.server_transactions
            .insert(transaction.id().clone(), transaction.clone());

        // Start the transaction in Trying state (for non-INVITE) or Proceeding (for INVITE)
        let initial_state = match transaction.kind() {
            TransactionKind::InviteServer => TransactionState::Proceeding,
            _ => TransactionState::Trying,
        };

        // Transition to the initial active state
        if let Err(e) = transaction
            .send_command(InternalTransactionCommand::TransitionTo(initial_state))
            .await
        {
            error!(id=%transaction.id(), error=%e, "Failed to initialize new server transaction");
            return Err(e);
        }

        Ok(transaction)
    }

    fn index_server_invite_dialog(&self, request: &Request, key: &TransactionKey) {
        if let Some(dialog_key) = ServerInviteDialogKey::from_request(request) {
            self.insert_server_invite_dialog_index_entry(
                dialog_key,
                ServerInviteAckIndexEntry::active(key.clone()),
            );
        }
    }

    pub(crate) fn insert_server_invite_dialog_index_entry(
        &self,
        dialog_key: ServerInviteDialogKey,
        entry: ServerInviteAckIndexEntry,
    ) {
        let transaction_id = entry.transaction_id.clone();
        let is_active = entry.expires_at.is_none();

        self.server_invite_dialog_index
            .insert(dialog_key.clone(), entry);

        if is_active {
            let mut keys = self
                .server_invite_dialog_keys_by_tx
                .entry(transaction_id)
                .or_default();
            if !keys.iter().any(|existing| existing == &dialog_key) {
                keys.push(dialog_key);
            }
        }

        let inserts = self
            .server_invite_dialog_index_insert_count
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let prune_interval = (self.server_invite_dialog_index_capacity / 4).max(1024);
        if inserts % prune_interval == 0
            || self.server_invite_dialog_index.len()
                > self.server_invite_dialog_index_capacity.saturating_mul(2)
        {
            self.prune_server_invite_dialog_index();
        }
    }

    pub(crate) fn retire_server_invite_dialog_index_for(&self, transaction_id: &TransactionKey) {
        let expires_at = Instant::now() + self.timer_settings.t4;

        if let Some((_, keys)) = self.server_invite_dialog_keys_by_tx.remove(transaction_id) {
            for key in keys {
                if let Some(mut indexed) = self.server_invite_dialog_index.get_mut(&key) {
                    if indexed.transaction_id == *transaction_id {
                        indexed.expires_at = Some(expires_at);
                    }
                }
            }
        }

        if self.server_invite_dialog_index.len()
            > self.server_invite_dialog_index_capacity.saturating_mul(2)
        {
            self.prune_server_invite_dialog_index();
        }
    }

    async fn send_cached_response(
        &self,
        response: Response,
        wire_bytes: bytes::Bytes,
        destination: SocketAddr,
        context: &'static str,
    ) -> std::result::Result<(), TransportError> {
        match self
            .transport
            .send_message_raw(wire_bytes, destination)
            .await
        {
            Ok(()) => Ok(()),
            Err(TransportError::NotImplemented(reason)) => {
                trace!(
                    reason = %reason,
                    destination = %destination,
                    context,
                    "Cached response raw send unavailable; falling back to structured send"
                );
                self.transport
                    .send_message(Message::Response(response), destination)
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
                destination: tx.data().remote_addr,
                created_at: now,
                acked_at: None,
                expires_at: now + INVITE_2XX_RESPONSE_CACHE_TTL,
                next_retransmit_at: now + self.timer_settings.t1,
                retransmit_interval: self.timer_settings.t1,
            },
        );
    }

    fn insert_invite_2xx_response_cache_entry(
        &self,
        transaction_id: TransactionKey,
        entry: Invite2xxResponseCacheEntry,
    ) {
        let next_retransmit_at = entry.next_retransmit_at;
        self.invite_2xx_response_cache
            .insert(transaction_id.clone(), entry);
        diagnostics::record_invite_2xx_cache_insert();
        self.schedule_invite_2xx_response_retransmit(transaction_id, next_retransmit_at);

        let inserts = self
            .invite_2xx_response_cache_insert_count
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let prune_interval = (self.invite_2xx_response_cache_capacity / 4).max(1024);
        if inserts % prune_interval == 0
            || self.invite_2xx_response_cache.len() > self.invite_2xx_response_cache_capacity
        {
            self.prune_invite_2xx_response_cache();
        }
    }

    pub(crate) async fn retransmit_cached_invite_2xx_response(
        &self,
        transaction_id: &TransactionKey,
        source: SocketAddr,
    ) -> Result<bool> {
        let entry = self
            .invite_2xx_response_cache
            .get(transaction_id)
            .map(|entry| entry.value().clone());

        let Some(entry) = entry else {
            return Ok(false);
        };

        if entry.is_expired(Instant::now()) {
            self.invite_2xx_response_cache.remove(transaction_id);
            diagnostics::record_invite_2xx_cache_expired();
            return Ok(false);
        }

        diagnostics::record_duplicate_invite_cache_hit();
        let is_200_ok = entry.response.status().as_u16() == 200;
        let destination = if source == entry.destination {
            entry.destination
        } else {
            source
        };

        self.send_cached_response(
            entry.response,
            entry.wire_bytes,
            destination,
            "Failed to retransmit cached INVITE 2xx",
        )
        .await
        .map_err(|e| Error::transport_error(e, "Failed to retransmit cached INVITE 2xx"))?;
        if is_200_ok {
            diagnostics::record_200_ok_invite_duplicate_cache();
        }

        Ok(true)
    }

    fn schedule_invite_2xx_response_retransmit(
        &self,
        transaction_id: TransactionKey,
        due_at: Instant,
    ) {
        let sequence = self
            .invite_2xx_response_due_sequence
            .fetch_add(1, Ordering::Relaxed);
        if let Ok(mut queue) = self.invite_2xx_response_due_queue.lock() {
            queue.push(Invite2xxDueEntry {
                due_at,
                sequence,
                transaction_id,
            });
        }
    }

    fn maintenance_prune_retained_state(&self) {
        self.prune_server_invite_dialog_index();
        self.prune_invite_2xx_response_cache();
        self.compact_invite_2xx_response_due_queue();
        self.prune_closed_event_subscribers_now();
        self.prune_stale_pending_inbound_bytes();
    }

    fn prune_closed_event_subscribers_now(&self) {
        Self::prune_event_subscribers(
            &self.event_subscribers,
            Some(&self.subscriber_to_transactions),
            Some(&self.transaction_to_subscribers),
            &[],
        );
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

    fn compact_invite_2xx_response_due_queue(&self) {
        let now = Instant::now();
        let mut expired_cache_keys = Vec::new();

        if let Ok(mut queue) = self.invite_2xx_response_due_queue.lock() {
            if queue.is_empty() {
                return;
            }

            let mut retained = BinaryHeap::with_capacity(queue.len());
            while let Some(due_entry) = queue.pop() {
                let keep = self
                    .invite_2xx_response_cache
                    .get(&due_entry.transaction_id)
                    .map(|entry| {
                        if entry.is_expired(now) {
                            expired_cache_keys.push(due_entry.transaction_id.clone());
                            false
                        } else {
                            due_entry.due_at == entry.next_retransmit_at
                        }
                    })
                    .unwrap_or(false);

                if keep {
                    retained.push(due_entry);
                }
            }
            *queue = retained;
        }

        for key in expired_cache_keys {
            if self
                .invite_2xx_response_cache
                .get(&key)
                .is_some_and(|entry| entry.value().is_expired(now))
            {
                self.invite_2xx_response_cache.remove(&key);
                diagnostics::record_invite_2xx_cache_expired();
            }
        }
    }

    async fn retransmit_due_invite_2xx_responses(&self) -> usize {
        let started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let cache_len = self.invite_2xx_response_cache.len();
        let now = Instant::now();
        let mut due_entries = Vec::new();
        let mut due_queue_len = 0usize;
        let mut capped = false;
        let max_due_per_tick = self
            .invite_2xx_retransmit_max_due_per_tick
            .load(Ordering::Relaxed)
            .max(1);

        if let Ok(mut queue) = self.invite_2xx_response_due_queue.lock() {
            due_queue_len = queue.len();
            while queue.peek().is_some_and(|entry| entry.due_at <= now) {
                if due_entries.len() >= max_due_per_tick {
                    capped = true;
                    break;
                }
                if let Some(entry) = queue.pop() {
                    due_entries.push(entry);
                }
            }
        }

        let scanned = due_entries.len();
        let mut expired_count = 0usize;
        let mut sends = Vec::new();
        let mut reschedule = Vec::new();

        for due_entry in due_entries {
            let mut expired = false;
            let mut send = None;
            let mut next_due = None;

            if let Some(mut entry) = self
                .invite_2xx_response_cache
                .get_mut(&due_entry.transaction_id)
            {
                if entry.is_expired(now) {
                    expired = true;
                } else if entry.acked_at.is_some() {
                    // ACKed 2xx responses are retained only for duplicate INVITEs.
                    // Proactive retransmission must stop once the ACK arrives.
                } else if entry.next_retransmit_at <= due_entry.due_at
                    && now >= entry.next_retransmit_at
                {
                    send = Some((
                        entry.response.clone(),
                        entry.wire_bytes.clone(),
                        entry.destination,
                    ));
                    entry.retransmit_interval = entry
                        .retransmit_interval
                        .saturating_mul(2)
                        .min(self.timer_settings.t2);
                    entry.next_retransmit_at = now + entry.retransmit_interval;
                    next_due = Some(entry.next_retransmit_at);
                }
            }

            if expired {
                self.invite_2xx_response_cache
                    .remove(&due_entry.transaction_id);
                diagnostics::record_invite_2xx_cache_expired();
                expired_count += 1;
            }
            if let Some((response, wire_bytes, destination)) = send {
                sends.push((response, wire_bytes, destination));
            }
            if let Some(next_due) = next_due {
                reschedule.push((due_entry.transaction_id, next_due));
            }
        }

        for (transaction_id, due_at) in reschedule {
            self.schedule_invite_2xx_response_retransmit(transaction_id, due_at);
        }

        let mut retransmitted = 0usize;
        for (response, wire_bytes, destination) in sends {
            let is_200_ok = response.status().as_u16() == 200;
            let send_started = diagnostics::transaction_timing_enabled().then(Instant::now);
            match self
                .send_cached_response(
                    response,
                    wire_bytes,
                    destination,
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
                        error = %e,
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
        let mut ack_retention_until = None;

        if let Some(mut entry) = self.invite_2xx_response_cache.get_mut(transaction_id) {
            if entry.acked_at.is_none() {
                diagnostics::record_invite_2xx_ack_removed(entry.created_at.elapsed());
                let retained_until =
                    (now + INVITE_2XX_ACKED_RESPONSE_RETENTION).min(entry.expires_at);
                entry.acked_at = Some(now);
                entry.expires_at = retained_until;
                entry.next_retransmit_at = retained_until;
                ack_retention_until = Some(retained_until);
            }
        }

        if let Some(retained_until) = ack_retention_until {
            self.schedule_invite_2xx_response_retransmit(transaction_id.clone(), retained_until);
        }
    }

    pub(crate) fn remove_invite_2xx_response_cache(&self, transaction_id: &TransactionKey) {
        self.invite_2xx_response_cache.remove(transaction_id);
    }

    pub(crate) fn mark_transaction_terminated_indexed(&self, transaction_id: &TransactionKey) {
        self.terminated_transactions
            .insert(transaction_id.clone(), ());
    }

    fn prune_server_invite_dialog_index(&self) {
        let now = Instant::now();
        let expired_keys: Vec<ServerInviteDialogKey> = self
            .server_invite_dialog_index
            .iter()
            .filter(|entry| entry.value().is_expired(now))
            .map(|entry| entry.key().clone())
            .collect();

        for key in expired_keys {
            if self
                .server_invite_dialog_index
                .get(&key)
                .is_some_and(|entry| entry.value().is_expired(now))
            {
                self.server_invite_dialog_index.remove(&key);
            }
        }
    }

    fn prune_invite_2xx_response_cache(&self) {
        let now = Instant::now();
        let expired_keys: Vec<TransactionKey> = self
            .invite_2xx_response_cache
            .iter()
            .filter(|entry| entry.value().is_expired(now))
            .map(|entry| entry.key().clone())
            .collect();

        for key in expired_keys {
            if self
                .invite_2xx_response_cache
                .get(&key)
                .is_some_and(|entry| entry.value().is_expired(now))
            {
                self.invite_2xx_response_cache.remove(&key);
            }
        }

        let len = self.invite_2xx_response_cache.len();
        if len <= self.invite_2xx_response_cache_capacity {
            return;
        }

        let mut entries: Vec<(TransactionKey, Instant)> = self
            .invite_2xx_response_cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().expires_at))
            .collect();
        entries.sort_by_key(|(_, expires_at)| *expires_at);

        for (key, _) in entries
            .into_iter()
            .take(len.saturating_sub(self.invite_2xx_response_cache_capacity))
        {
            self.invite_2xx_response_cache.remove(&key);
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
        debug!(id=%invite_tx_id, "Canceling invite transaction");

        // Check that this is an INVITE client transaction
        if invite_tx_id.method() != &Method::Invite || invite_tx_id.is_server() {
            return Err(Error::Other(format!(
                "Transaction {} is not an INVITE client transaction",
                invite_tx_id
            )));
        }

        // Get the original INVITE request
        let invite_request =
            utils::get_transaction_request(&self.client_transactions, invite_tx_id).await?;

        debug!(id=%invite_tx_id, "Got INVITE request for cancellation");

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
            warn!(method = %cancel_request.method(), error = %e, "CANCEL request validation issue - proceeding anyway");
        }

        // Get the destination for the CANCEL request (same as the INVITE)
        let destination = match self.transaction_destinations.get(invite_tx_id) {
            Some(entry) => *entry.value(),
            None => {
                return Err(Error::Other(format!(
                    "No destination found for transaction {}",
                    invite_tx_id
                )));
            }
        };

        // Create a transaction for the CANCEL request
        let cancel_tx_id = self
            .create_client_transaction(cancel_request, destination)
            .await?;

        debug!(id=%cancel_tx_id, original_id=%invite_tx_id, "Created CANCEL transaction");

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
