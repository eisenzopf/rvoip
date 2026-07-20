//! Consolidated terminal-lifecycle scheduling for SIP transactions.
//!
//! RFC state machines stop at `Terminated`, but the transaction manager keeps
//! a short grace/drain fence before removing their routing state. Historically
//! every transaction spawned a second task containing two sleeps. At high call
//! rates those tasks and timer entries became a substantial retained set.
//!
//! Each `TransactionManager` owns one due-driven worker and installs its handle
//! into every transaction it creates. Runners submit compact deadline entries
//! to that handle; entries are keyed by the underlying transaction allocation,
//! so a repeated schedule replaces the exact previous deadline instead of
//! accumulating stale heap entries. Standalone transactions retain the legacy
//! grace semantics through one runtime-keyed compatibility scheduler instead
//! of creating a sleeper task for every low-level transaction.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant as StdInstant};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tracing::{debug, trace};

use dashmap::DashMap;
use rvoip_sip_transport::TransportRoute;

use crate::diagnostics;
use crate::transaction::manager::ClientResponseRouteState;
use crate::transaction::runner::{AsRefKey, HasCommandSender, HasLifecycle};
use crate::transaction::{
    InternalTransactionCommand, TransactionEvent, TransactionKey, TransactionLifecycle,
    TransactionState,
};

const TERMINATING_GRACE: Duration = Duration::from_millis(500);
const DRAINING_GRACE: Duration = Duration::from_millis(100);
// A 65k-call acceptance burst can produce one Timer J and one Timer K
// retirement. Tokio's bounded channel allocates message blocks lazily, so
// this logical headroom does not eagerly reserve 262k tombstones.
const SCHEDULE_CHANNEL_CAPACITY: usize = 262_144;
// Terminal observations are protocol-significant and therefore lossless, but
// the TU is allowed to apply backpressure.  Keep only a bounded number of
// delivery batches between the deadline worker and the primary event channel;
// expired tombstones remain exact generation fences until their batch is
// delivered.
const COMPACT_EVENT_BATCH_CAPACITY: usize = 1_024;
const MAX_COMPACT_RETAINED: usize = SCHEDULE_CHANNEL_CAPACITY;
const MAX_DUE_PER_BATCH: usize = 1_024;
const MAX_SCHEDULES_PER_BATCH: usize = 1_024;
const COMMAND_RETRY_DELAY: Duration = Duration::from_millis(1);

/// Minimal RFC state retained while UDP Timer J or K is active.
///
/// Client Timer K needs only an authenticated transaction key so duplicate
/// final responses can be absorbed. Server Timer J additionally needs the
/// immutable final response bytes and exact ingress route to replay it. The
/// parsed request, progress history, runner, timer factory, and command queue
/// are deliberately absent.
#[derive(Clone)]
pub(crate) enum CompactNonInviteTombstone {
    Client {
        /// Admission-time capacity lease shared with the active transaction.
        /// Keeping the same lease through Timer K prevents active and retired
        /// representations from double-counting the bounded retention slot.
        _retention_reservation: CompactRetentionReservation,
        _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
        terminal_event_publication: Arc<crate::transaction::event_sender::TerminalEventPublication>,
        state: Arc<crate::transaction::AtomicTransactionState>,
        /// Immutable exact final response/failure used by lookups that race
        /// with or follow runner removal. This deliberately does not retain
        /// the live completion cell's mutex/notify allocation for all of T4.
        completion: crate::transaction::completion::RetainedClientTransactionCompletion,
        /// Non-owning bridge to a waiter that cloned the live cell before
        /// retirement. If such a waiter still exists at Timer K expiry it
        /// must observe the exact terminal transition, but the tombstone must
        /// not keep the live cell alive merely to support that uncommon race.
        live_completion: Weak<crate::transaction::completion::ClientTransactionCompletion>,
        auth_lease: Option<crate::transaction::manager::InboundPrincipalLease>,
        /// Allocation-scoped owner installed with the authoritative active
        /// response route. This is an exact cleanup proof without cloning the
        /// complete route (and its optional DNS authority) into the tombstone.
        response_route_owner: usize,
        expires_at: StdInstant,
        generation: u64,
    },
    Server {
        /// Admission-time capacity lease shared with the active transaction
        /// and retained through the complete Timer J replay horizon.
        _retention_reservation: CompactRetentionReservation,
        _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
        terminal_event_publication: Arc<crate::transaction::event_sender::TerminalEventPublication>,
        state: Arc<crate::transaction::AtomicTransactionState>,
        auth_lease: Option<crate::transaction::manager::InboundPrincipalLease>,
        response_wire: bytes::Bytes,
        response_route: TransportRoute,
        expires_at: StdInstant,
        generation: u64,
    },
}

impl CompactNonInviteTombstone {
    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::Client { generation, .. } | Self::Server { generation, .. } => *generation,
        }
    }

    pub(crate) fn expires_at(&self) -> StdInstant {
        match self {
            Self::Client { expires_at, .. } | Self::Server { expires_at, .. } => *expires_at,
        }
    }

    pub(crate) fn is_client(&self) -> bool {
        matches!(self, Self::Client { .. })
    }

    pub(crate) fn client_completion(
        &self,
    ) -> Option<&crate::transaction::completion::RetainedClientTransactionCompletion> {
        match self {
            Self::Client { completion, .. } => Some(completion),
            Self::Server { .. } => None,
        }
    }

    pub(crate) fn server_replay(&self) -> Option<(&bytes::Bytes, &TransportRoute)> {
        match self {
            Self::Server {
                response_wire,
                response_route,
                ..
            } => Some((response_wire, response_route)),
            Self::Client { .. } => None,
        }
    }

    fn client_route_owner(&self) -> Option<usize> {
        match self {
            Self::Client {
                response_route_owner,
                ..
            } => Some(*response_route_owner),
            Self::Server { .. } => None,
        }
    }

    fn wake_live_client_completion(&self, state: TransactionState) {
        if let Self::Client {
            live_completion, ..
        } = self
        {
            if let Some(completion) = live_completion.upgrade() {
                completion.record_state(state);
            }
        }
    }

    pub(crate) fn state(&self) -> &Arc<crate::transaction::AtomicTransactionState> {
        match self {
            Self::Client { state, .. } | Self::Server { state, .. } => state,
        }
    }

    fn auth_lease(&self) -> Option<crate::transaction::manager::InboundPrincipalLease> {
        match self {
            Self::Client { auth_lease, .. } | Self::Server { auth_lease, .. } => *auth_lease,
        }
    }

    fn admission_owner(&self) -> Option<crate::transaction::manager::TransactionAdmissionOwner> {
        match self {
            Self::Client {
                _admission_owner, ..
            }
            | Self::Server {
                _admission_owner, ..
            } => _admission_owner.clone(),
        }
    }

    fn claim_terminal_event(
        &self,
    ) -> Option<crate::transaction::event_sender::TerminalEventPublicationClaim> {
        let publication = match self {
            Self::Client {
                terminal_event_publication,
                ..
            }
            | Self::Server {
                terminal_event_publication,
                ..
            } => terminal_event_publication,
        };
        publication.try_claim()
    }
}

pub(crate) type CompactNonInviteTombstones = DashMap<TransactionKey, CompactNonInviteTombstone>;

/// Shared logical capacity for active UDP non-INVITE transactions and their
/// compact Timer J/K successors. The counter is intentionally allocation-lazy:
/// configuring 168k slots reserves no tombstone storage up front.
struct CompactRetentionCapacity {
    in_use: AtomicUsize,
    limit: usize,
}

impl CompactRetentionCapacity {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            in_use: AtomicUsize::new(0),
            limit: limit.max(1),
        })
    }

    fn try_reserve(self: &Arc<Self>) -> Option<CompactRetentionReservation> {
        let mut current = self.in_use.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return None;
            }
            match self.in_use.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(CompactRetentionReservation {
                        _lease: Arc::new(CompactRetentionLease {
                            capacity: Arc::clone(self),
                        }),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    #[cfg(test)]
    fn in_use(&self) -> usize {
        self.in_use.load(Ordering::Acquire)
    }
}

struct CompactRetentionLease {
    capacity: Arc<CompactRetentionCapacity>,
}

impl Drop for CompactRetentionLease {
    fn drop(&mut self) {
        let previous = self.capacity.in_use.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "compact retention reservation underflow");
    }
}

/// Cloneable lease acquired before a managed UDP non-INVITE transaction is
/// admitted. Clones share one counted slot, allowing ownership to move from
/// the active runner to its compact tombstone without a capacity race.
#[derive(Clone)]
pub(crate) struct CompactRetentionReservation {
    _lease: Arc<CompactRetentionLease>,
}

trait LifecycleTarget: Send + Sync {
    fn transaction_id(&self) -> &TransactionKey;
    fn set_lifecycle(&self, lifecycle: TransactionLifecycle);
    fn wake_destroyed_runner(&self);
}

struct DataLifecycleTarget<D> {
    data: Arc<D>,
}

impl<D> LifecycleTarget for DataLifecycleTarget<D>
where
    D: AsRefKey + HasCommandSender + HasLifecycle + Send + Sync + 'static,
{
    fn transaction_id(&self) -> &TransactionKey {
        self.data.as_ref_key()
    }

    fn set_lifecycle(&self, lifecycle: TransactionLifecycle) {
        self.data.set_lifecycle(lifecycle);
    }

    fn wake_destroyed_runner(&self) {
        match self
            .data
            .get_self_command_sender()
            .try_send(InternalTransactionCommand::Terminate)
        {
            Ok(()) => diagnostics::record_transaction_runner_destroy_wake_sent(),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // A queued command already wakes the runner. It observes the
                // Destroyed lifecycle after processing that command and exits.
                trace!(
                    id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(self.transaction_id()),
                    "Transaction command channel full after consolidated lifecycle destroy"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                diagnostics::record_transaction_runner_destroy_wake_failed();
                trace!(
                    id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(self.transaction_id()),
                    "Transaction runner already closed after consolidated lifecycle destroy"
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleDeadlinePhase {
    BeginDraining,
    Destroy,
}

struct LifecycleDeadline {
    identity: usize,
    target: Arc<dyn LifecycleTarget>,
    phase: LifecycleDeadlinePhase,
}

struct ScheduleRequest {
    identity: usize,
    target: Arc<dyn LifecycleTarget>,
    scheduled_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompactNonInviteTimer {
    J,
    K,
}

impl CompactNonInviteTimer {
    fn name(self) -> &'static str {
        match self {
            Self::J => "J",
            Self::K => "K",
        }
    }
}

struct CompactScheduleRequest {
    identity: usize,
    transaction_id: TransactionKey,
    timer: CompactNonInviteTimer,
    delay: Duration,
    scheduled_at: Instant,
    server_replay: Option<(bytes::Bytes, TransportRoute)>,
    state: Arc<crate::transaction::AtomicTransactionState>,
    completion: Option<Arc<crate::transaction::completion::ClientTransactionCompletion>>,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
    retention_reservation: CompactRetentionReservation,
    admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    terminal_event_publication: Arc<crate::transaction::event_sender::TerminalEventPublication>,
    accepted: oneshot::Sender<bool>,
}

struct StandaloneTimerScheduleRequest {
    identity: usize,
    transaction_id: TransactionKey,
    timer: CompactNonInviteTimer,
    delay: Duration,
    scheduled_at: Instant,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
}

struct CommandScheduleRequest {
    identity: usize,
    transaction_id: TransactionKey,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
    commands: VecDeque<InternalTransactionCommand>,
    scheduled_at: Instant,
}

struct CompactExpiryRequest {
    transaction_id: TransactionKey,
    generation: u64,
}

enum SchedulerCommand {
    Schedule(ScheduleRequest),
    ScheduleCompact(CompactScheduleRequest),
    ScheduleStandaloneTimer(StandaloneTimerScheduleRequest),
    ScheduleCommands(CommandScheduleRequest),
    ExpireCompact(CompactExpiryRequest),
    Shutdown(oneshot::Sender<()>),
}

struct CompactTerminalEventBatch {
    transaction_id: TransactionKey,
    timer: CompactNonInviteTimer,
    generation: u64,
    admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    publication_claim: crate::transaction::event_sender::TerminalEventPublicationClaim,
}

#[derive(Clone, Default)]
struct ManagedCompactContext {
    tombstones: Weak<CompactNonInviteTombstones>,
    client_routes: Weak<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
    inbound_principals:
        Weak<DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalBinding>>,
    inbound_principal_inserted_at:
        Weak<DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>>,
    compact_events_tx: Option<mpsc::Sender<CompactTerminalEventBatch>>,
    dialog_ack_required: Option<Arc<AtomicBool>>,
    observer_fanout: Option<crate::transaction::event_sender::TransactionObserverFanout>,
    protocol_failure_sender: Option<crate::transaction::event_sender::TransactionEventSender>,
    max_compact_retained: usize,
}

/// Cloneable sender for the single lifecycle worker owned by a
/// `TransactionManager`.
///
/// The worker never retains a clone of this sender, so an explicit shutdown
/// closes it deterministically and ordinary manager drop cannot create a
/// permanent sender/receiver ownership cycle.
#[derive(Clone)]
pub(crate) struct LifecycleSchedulerHandle {
    sender: mpsc::Sender<SchedulerCommand>,
    compact_deadline_count: Arc<AtomicUsize>,
    compact_event_shutdown: Option<watch::Sender<bool>>,
    compact_event_completion: Option<Arc<SchedulerWorkerCompletion>>,
    dialog_ack_required: Option<Arc<AtomicBool>>,
    compact_retention_capacity: Arc<CompactRetentionCapacity>,
}

#[derive(Default)]
struct SchedulerWorkerCompletion {
    done: AtomicBool,
    notify: tokio::sync::Notify,
}

impl SchedulerWorkerCompletion {
    fn complete(&self) {
        self.done.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.done.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

/// Non-owning scheduler reference installed in transaction data.
///
/// The manager is the sole lifecycle owner. A transaction may enqueue work
/// while that owner is alive, but keeping a transaction in a manager map must
/// not in turn keep the scheduler task (and its receiver) alive.
#[derive(Clone)]
pub(crate) struct WeakLifecycleSchedulerHandle {
    sender: mpsc::WeakSender<SchedulerCommand>,
    compact_retention_capacity: Weak<CompactRetentionCapacity>,
}

impl LifecycleSchedulerHandle {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel(SCHEDULE_CHANNEL_CAPACITY);
        let compact_deadline_count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(run_scheduler(
            receiver,
            ManagedCompactContext::default(),
            compact_deadline_count.clone(),
        ));
        Self {
            sender,
            compact_deadline_count,
            compact_event_shutdown: None,
            compact_event_completion: None,
            dialog_ack_required: None,
            compact_retention_capacity: CompactRetentionCapacity::new(MAX_COMPACT_RETAINED),
        }
    }

    pub(crate) fn new_managed(
        tombstones: &Arc<CompactNonInviteTombstones>,
        client_routes: &Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
        inbound_principals: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalBinding>,
        >,
        inbound_principal_inserted_at: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>,
        >,
        events_tx: &crate::transaction::event_sender::TransactionEventSender,
    ) -> Self {
        Self::new_managed_with_limits(
            tombstones,
            client_routes,
            inbound_principals,
            inbound_principal_inserted_at,
            events_tx,
            COMPACT_EVENT_BATCH_CAPACITY,
            MAX_COMPACT_RETAINED,
        )
    }

    /// Manager constructor that binds compact Timer J/K admission to the same
    /// logical capacity configured for transaction indexes. Storage remains
    /// lazy; the value is only a protocol-retention admission bound.
    pub(crate) fn new_managed_with_retention_capacity(
        tombstones: &Arc<CompactNonInviteTombstones>,
        client_routes: &Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
        inbound_principals: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalBinding>,
        >,
        inbound_principal_inserted_at: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>,
        >,
        events_tx: &crate::transaction::event_sender::TransactionEventSender,
        max_compact_retained: usize,
    ) -> Self {
        Self::new_managed_with_limits(
            tombstones,
            client_routes,
            inbound_principals,
            inbound_principal_inserted_at,
            events_tx,
            COMPACT_EVENT_BATCH_CAPACITY,
            max_compact_retained,
        )
    }

    fn new_managed_with_limits(
        tombstones: &Arc<CompactNonInviteTombstones>,
        client_routes: &Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
        inbound_principals: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalBinding>,
        >,
        inbound_principal_inserted_at: &Arc<
            DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>,
        >,
        events_tx: &crate::transaction::event_sender::TransactionEventSender,
        event_batch_capacity: usize,
        max_compact_retained: usize,
    ) -> Self {
        let event_batch_capacity = event_batch_capacity.max(1);
        let max_compact_retained = max_compact_retained.max(1);
        let (sender, receiver) = mpsc::channel(SCHEDULE_CHANNEL_CAPACITY);
        let (compact_events_tx, compact_events_rx) = mpsc::channel(event_batch_capacity);
        let (compact_event_shutdown, compact_event_shutdown_rx) = watch::channel(false);
        let compact_deadline_count = Arc::new(AtomicUsize::new(0));
        let dialog_ack_required = Arc::new(AtomicBool::new(false));
        let compact_retention_capacity = CompactRetentionCapacity::new(max_compact_retained);
        let compact_event_completion = Arc::new(SchedulerWorkerCompletion::default());
        let dispatcher_completion = Arc::clone(&compact_event_completion);
        let dispatcher_events_tx = events_tx.clone();
        let dispatcher_context = ManagedCompactContext {
            tombstones: Arc::downgrade(tombstones),
            client_routes: Arc::downgrade(client_routes),
            inbound_principals: Arc::downgrade(inbound_principals),
            inbound_principal_inserted_at: Arc::downgrade(inbound_principal_inserted_at),
            compact_events_tx: None,
            dialog_ack_required: Some(dialog_ack_required.clone()),
            observer_fanout: events_tx.observer_fanout(),
            protocol_failure_sender: Some(events_tx.clone()),
            max_compact_retained,
        };
        tokio::spawn(async move {
            run_compact_event_dispatcher(
                compact_events_rx,
                dispatcher_events_tx,
                compact_event_shutdown_rx,
                dispatcher_context,
            )
            .await;
            dispatcher_completion.complete();
        });
        tokio::spawn(run_scheduler(
            receiver,
            ManagedCompactContext {
                tombstones: Arc::downgrade(tombstones),
                client_routes: Arc::downgrade(client_routes),
                inbound_principals: Arc::downgrade(inbound_principals),
                inbound_principal_inserted_at: Arc::downgrade(inbound_principal_inserted_at),
                compact_events_tx: Some(compact_events_tx),
                dialog_ack_required: Some(dialog_ack_required.clone()),
                observer_fanout: events_tx.observer_fanout(),
                protocol_failure_sender: Some(events_tx.clone()),
                max_compact_retained,
            },
            compact_deadline_count.clone(),
        ));
        Self {
            sender,
            compact_deadline_count,
            compact_event_shutdown: Some(compact_event_shutdown),
            compact_event_completion: Some(compact_event_completion),
            dialog_ack_required: Some(dialog_ack_required),
            compact_retention_capacity,
        }
    }

    pub(crate) fn downgrade(&self) -> WeakLifecycleSchedulerHandle {
        WeakLifecycleSchedulerHandle {
            sender: self.sender.downgrade(),
            compact_retention_capacity: Arc::downgrade(&self.compact_retention_capacity),
        }
    }

    /// Reserve one UDP non-INVITE Timer J/K slot before protocol admission.
    /// Failure is an overload decision, not permission to shorten an already
    /// accepted transaction's retransmission fence.
    pub(crate) fn try_reserve_compact_retention(&self) -> Option<CompactRetentionReservation> {
        self.compact_retention_capacity.try_reserve()
    }

    #[cfg(test)]
    pub(crate) fn compact_retention_in_use(&self) -> usize {
        self.compact_retention_capacity.in_use()
    }

    pub(crate) fn compact_retention_limit(&self) -> usize {
        self.compact_retention_capacity.limit
    }

    pub(crate) fn compact_deadline_count(&self) -> usize {
        self.compact_deadline_count.load(Ordering::Acquire)
    }

    /// Ask the authoritative lifecycle worker to expire one exact compact
    /// generation. This is intentionally best-effort: the original deadline
    /// remains authoritative when the command queue is already saturated.
    pub(crate) fn request_compact_expiry(&self, transaction_id: TransactionKey, generation: u64) {
        match self
            .sender
            .try_send(SchedulerCommand::ExpireCompact(CompactExpiryRequest {
                transaction_id,
                generation,
            })) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Integrated dialog managers have an additional sharded queue after the
    /// transaction primary. Keep compact key fences until that consumer has
    /// actually processed `TransactionTerminated`.
    pub(crate) fn require_dialog_terminal_ack(&self) {
        if let Some(required) = self.dialog_ack_required.as_ref() {
            required.store(true, Ordering::Release);
        }
    }

    fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    async fn schedule_lifecycle<D>(&self, data: Arc<D>) -> bool
    where
        D: AsRefKey + HasCommandSender + HasLifecycle + Send + Sync + 'static,
    {
        let identity = Arc::as_ptr(&data) as usize;
        let target: Arc<dyn LifecycleTarget> = Arc::new(DataLifecycleTarget { data });
        let command = SchedulerCommand::Schedule(ScheduleRequest {
            identity,
            target,
            scheduled_at: Instant::now(),
        });
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(command)) => {
                self.sender.send(command).await.is_ok()
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    async fn schedule_standalone_timer(
        &self,
        identity: usize,
        transaction_id: TransactionKey,
        timer: CompactNonInviteTimer,
        delay: Duration,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> bool {
        let command = SchedulerCommand::ScheduleStandaloneTimer(StandaloneTimerScheduleRequest {
            identity,
            transaction_id,
            timer,
            delay,
            scheduled_at: Instant::now(),
            command_tx,
        });
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(command)) => {
                self.sender.send(command).await.is_ok()
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    async fn schedule_commands(&self, request: CommandScheduleRequest) -> bool {
        let command = SchedulerCommand::ScheduleCommands(request);
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(command)) => {
                self.sender.send(command).await.is_ok()
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Stop this manager's lifecycle worker and release every scheduled
    /// transaction immediately. This is idempotent across cloned manager
    /// handles: later calls observe the already-closed channel.
    pub(crate) async fn shutdown(&self) {
        if let Some(shutdown) = self.compact_event_shutdown.as_ref() {
            let _ = shutdown.send(true);
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        if self
            .sender
            .send(SchedulerCommand::Shutdown(ack_tx))
            .await
            .is_ok()
        {
            let _ = ack_rx.await;
        }
        if let Some(completion) = self.compact_event_completion.as_ref() {
            completion.wait().await;
        }
    }
}

/// One compatibility scheduler per live Tokio runtime for transactions built
/// directly through the public low-level constructors. Runtime IDs are unique
/// while a runtime is alive; closed workers are discarded before lookup so a
/// later runtime that reuses an ID cannot inherit a dead channel.
static STANDALONE_SCHEDULERS: OnceLock<
    StdMutex<HashMap<tokio::runtime::Id, LifecycleSchedulerHandle>>,
> = OnceLock::new();

fn standalone_scheduler() -> LifecycleSchedulerHandle {
    let runtime_id = tokio::runtime::Handle::current().id();
    let schedulers = STANDALONE_SCHEDULERS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut schedulers = schedulers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    schedulers.retain(|_, scheduler| !scheduler.is_closed());
    schedulers
        .entry(runtime_id)
        .or_insert_with(LifecycleSchedulerHandle::new)
        .clone()
}

pub(crate) async fn schedule_standalone_non_invite_timer(
    identity: usize,
    transaction_id: TransactionKey,
    timer: CompactNonInviteTimer,
    delay: Duration,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
) -> bool {
    standalone_scheduler()
        .schedule_standalone_timer(identity, transaction_id, timer, delay, command_tx)
        .await
}

pub(crate) async fn schedule_standalone_commands(
    identity: usize,
    transaction_id: TransactionKey,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
    commands: VecDeque<InternalTransactionCommand>,
) -> bool {
    standalone_scheduler()
        .schedule_commands(CommandScheduleRequest {
            identity,
            transaction_id,
            command_tx,
            commands,
            scheduled_at: Instant::now(),
        })
        .await
}

impl WeakLifecycleSchedulerHandle {
    pub(crate) async fn schedule<D>(&self, data: Arc<D>) -> bool
    where
        D: AsRefKey + HasCommandSender + HasLifecycle + Send + Sync + 'static,
    {
        let Some(sender) = self.sender.upgrade() else {
            return false;
        };
        let identity = Arc::as_ptr(&data) as usize;
        let target: Arc<dyn LifecycleTarget> = Arc::new(DataLifecycleTarget { data });
        let command = SchedulerCommand::Schedule(ScheduleRequest {
            identity,
            target,
            scheduled_at: Instant::now(),
        });
        match sender.try_send(command) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(command)) => sender.send(command).await.is_ok(),
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    #[cfg(test)]
    pub(crate) async fn schedule_compact_non_invite(
        &self,
        identity: usize,
        transaction_id: TransactionKey,
        timer: CompactNonInviteTimer,
        delay: Duration,
        server_replay: Option<(bytes::Bytes, TransportRoute)>,
        state: Arc<crate::transaction::AtomicTransactionState>,
        completion: Option<Arc<crate::transaction::completion::ClientTransactionCompletion>>,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> bool {
        self.schedule_compact_non_invite_with_reservation(
            identity,
            transaction_id,
            timer,
            delay,
            server_replay,
            state,
            completion,
            command_tx,
            None,
            None,
            crate::transaction::event_sender::TerminalEventPublication::new(),
        )
        .await
    }

    pub(crate) async fn schedule_compact_non_invite_with_reservation(
        &self,
        identity: usize,
        transaction_id: TransactionKey,
        timer: CompactNonInviteTimer,
        delay: Duration,
        server_replay: Option<(bytes::Bytes, TransportRoute)>,
        state: Arc<crate::transaction::AtomicTransactionState>,
        completion: Option<Arc<crate::transaction::completion::ClientTransactionCompletion>>,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
        retention_reservation: Option<CompactRetentionReservation>,
        admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
        terminal_event_publication: Arc<crate::transaction::event_sender::TerminalEventPublication>,
    ) -> bool {
        let Some(sender) = self.sender.upgrade() else {
            return false;
        };
        // Managed transactions normally carry an admission-time lease. Tests
        // and other direct scheduler users acquire defensively here so the
        // compact queue can never exceed the same logical bound.
        let retention_reservation = match retention_reservation {
            Some(reservation) => reservation,
            None => {
                let Some(capacity) = self.compact_retention_capacity.upgrade() else {
                    return false;
                };
                let Some(reservation) = capacity.try_reserve() else {
                    return false;
                };
                reservation
            }
        };
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let command = SchedulerCommand::ScheduleCompact(CompactScheduleRequest {
            identity,
            transaction_id,
            timer,
            delay,
            scheduled_at: Instant::now(),
            server_replay,
            state,
            completion,
            command_tx,
            retention_reservation,
            admission_owner,
            terminal_event_publication,
            accepted: accepted_tx,
        });
        match sender.try_send(command) {
            Ok(()) => accepted_rx.await.unwrap_or(false),
            Err(mpsc::error::TrySendError::Full(command)) => {
                if sender.send(command).await.is_err() {
                    return false;
                }
                accepted_rx.await.unwrap_or(false)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    pub(crate) async fn schedule_commands(
        &self,
        identity: usize,
        transaction_id: TransactionKey,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
        commands: VecDeque<InternalTransactionCommand>,
    ) -> bool {
        let Some(sender) = self.sender.upgrade() else {
            return false;
        };
        let command = SchedulerCommand::ScheduleCommands(CommandScheduleRequest {
            identity,
            transaction_id,
            command_tx,
            commands,
            scheduled_at: Instant::now(),
        });
        match sender.try_send(command) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(command)) => sender.send(command).await.is_ok(),
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

#[derive(Clone, Copy)]
struct ReverseDeadline {
    due_at: Instant,
    sequence: u64,
}

#[derive(Default)]
struct LifecycleDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), LifecycleDeadline>,
    by_identity: HashMap<usize, ReverseDeadline>,
    next_sequence: u64,
}

struct CompactDeadline {
    transaction_id: Arc<TransactionKey>,
    timer: CompactNonInviteTimer,
    generation: u64,
}

#[derive(Clone, Copy)]
struct CompactReverseDeadline {
    due_at: Instant,
    sequence: u64,
    generation: u64,
}

struct CompactDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), CompactDeadline>,
    by_transaction: HashMap<Arc<TransactionKey>, CompactReverseDeadline>,
    next_sequence: u64,
    next_generation: u64,
    max_entries: usize,
}

impl Default for CompactDeadlineQueue {
    fn default() -> Self {
        Self::with_capacity(MAX_COMPACT_RETAINED)
    }
}

struct StandaloneTimerDeadline {
    identity: usize,
    transaction_id: TransactionKey,
    timer: CompactNonInviteTimer,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
}

#[derive(Default)]
struct StandaloneTimerDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), StandaloneTimerDeadline>,
    by_identity: HashMap<usize, ReverseDeadline>,
    next_sequence: u64,
}

struct PendingCommandDelivery {
    identity: usize,
    transaction_id: TransactionKey,
    command_tx: mpsc::Sender<InternalTransactionCommand>,
    commands: VecDeque<InternalTransactionCommand>,
}

#[derive(Default)]
struct PendingCommandQueue {
    by_deadline: BTreeMap<(Instant, u64), PendingCommandDelivery>,
    by_identity: HashMap<usize, ReverseDeadline>,
    next_sequence: u64,
}

impl CompactDeadlineQueue {
    fn with_capacity(max_entries: usize) -> Self {
        Self {
            by_deadline: BTreeMap::new(),
            by_transaction: HashMap::new(),
            next_sequence: 0,
            next_generation: 0,
            max_entries,
        }
    }

    fn next_sequence(&mut self, due_at: Instant) -> u64 {
        loop {
            let sequence = self.next_sequence;
            self.next_sequence = self.next_sequence.wrapping_add(1);
            if !self.by_deadline.contains_key(&(due_at, sequence)) {
                return sequence;
            }
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.next_generation
    }

    fn insert(
        &mut self,
        transaction_id: TransactionKey,
        timer: CompactNonInviteTimer,
        due_at: Instant,
    ) -> Option<u64> {
        if !self.by_transaction.contains_key(&transaction_id)
            && self.by_transaction.len() >= self.max_entries
        {
            return None;
        }
        self.unschedule(&transaction_id);
        let transaction_id = Arc::new(transaction_id);
        let sequence = self.next_sequence(due_at);
        let generation = self.next_generation();
        self.by_transaction.insert(
            transaction_id.clone(),
            CompactReverseDeadline {
                due_at,
                sequence,
                generation,
            },
        );
        self.by_deadline.insert(
            (due_at, sequence),
            CompactDeadline {
                transaction_id,
                timer,
                generation,
            },
        );
        Some(generation)
    }

    fn unschedule(&mut self, transaction_id: &TransactionKey) -> bool {
        let Some(previous) = self.by_transaction.remove(transaction_id) else {
            return false;
        };
        self.by_deadline
            .remove(&(previous.due_at, previous.sequence));
        true
    }

    fn remove_generation(&mut self, transaction_id: &TransactionKey, generation: u64) {
        let should_remove = self
            .by_transaction
            .get(transaction_id)
            .is_some_and(|entry| entry.generation == generation);
        if should_remove {
            self.unschedule(transaction_id);
        }
    }

    fn expedite_generation(
        &mut self,
        transaction_id: &TransactionKey,
        generation: u64,
        now: Instant,
    ) -> bool {
        let Some(previous) = self.by_transaction.get(transaction_id).copied() else {
            return false;
        };
        if previous.generation != generation {
            return false;
        }
        if previous.due_at <= now {
            return true;
        }
        let Some(deadline) = self
            .by_deadline
            .remove(&(previous.due_at, previous.sequence))
        else {
            return false;
        };
        let sequence = self.next_sequence(now);
        self.by_transaction.insert(
            deadline.transaction_id.clone(),
            CompactReverseDeadline {
                due_at: now,
                sequence,
                generation,
            },
        );
        self.by_deadline.insert((now, sequence), deadline);
        true
    }

    fn next_due_at(&self) -> Option<Instant> {
        self.by_deadline.first_key_value().map(|(key, _)| key.0)
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<CompactDeadline> {
        let mut due = Vec::with_capacity(limit.min(self.by_deadline.len()));
        while due.len() < limit {
            let Some((&(due_at, sequence), deadline)) = self.by_deadline.first_key_value() else {
                break;
            };
            if due_at > now {
                break;
            }
            let transaction_id = deadline.transaction_id.clone();
            let Some(deadline) = self.by_deadline.remove(&(due_at, sequence)) else {
                continue;
            };
            if self
                .by_transaction
                .get(transaction_id.as_ref())
                .is_some_and(|reverse| {
                    reverse.due_at == due_at
                        && reverse.sequence == sequence
                        && reverse.generation == deadline.generation
                })
            {
                self.by_transaction.remove(transaction_id.as_ref());
                due.push(deadline);
            }
        }
        due
    }

    fn clear(&mut self) {
        self.by_deadline.clear();
        self.by_transaction.clear();
    }

    fn len(&self) -> usize {
        debug_assert_eq!(self.by_deadline.len(), self.by_transaction.len());
        self.by_deadline.len()
    }
}

impl StandaloneTimerDeadlineQueue {
    fn next_sequence(&mut self, due_at: Instant) -> u64 {
        loop {
            let sequence = self.next_sequence;
            self.next_sequence = self.next_sequence.wrapping_add(1);
            if !self.by_deadline.contains_key(&(due_at, sequence)) {
                return sequence;
            }
        }
    }

    fn insert(&mut self, request: StandaloneTimerScheduleRequest) -> bool {
        let replaced = self.unschedule(request.identity);
        let due_at = request.scheduled_at + request.delay;
        let sequence = self.next_sequence(due_at);
        self.by_identity
            .insert(request.identity, ReverseDeadline { due_at, sequence });
        self.by_deadline.insert(
            (due_at, sequence),
            StandaloneTimerDeadline {
                identity: request.identity,
                transaction_id: request.transaction_id,
                timer: request.timer,
                command_tx: request.command_tx,
            },
        );
        replaced
    }

    fn unschedule(&mut self, identity: usize) -> bool {
        let Some(previous) = self.by_identity.remove(&identity) else {
            return false;
        };
        self.by_deadline
            .remove(&(previous.due_at, previous.sequence));
        true
    }

    fn next_due_at(&self) -> Option<Instant> {
        self.by_deadline.first_key_value().map(|(key, _)| key.0)
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<StandaloneTimerDeadline> {
        let mut due = Vec::with_capacity(limit.min(self.by_deadline.len()));
        while due.len() < limit {
            let Some((&(due_at, sequence), deadline)) = self.by_deadline.first_key_value() else {
                break;
            };
            if due_at > now {
                break;
            }
            let identity = deadline.identity;
            let Some(deadline) = self.by_deadline.remove(&(due_at, sequence)) else {
                continue;
            };
            if self
                .by_identity
                .get(&identity)
                .is_some_and(|reverse| reverse.due_at == due_at && reverse.sequence == sequence)
            {
                self.by_identity.remove(&identity);
                due.push(deadline);
            }
        }
        due
    }

    fn clear(&mut self) {
        self.by_deadline.clear();
        self.by_identity.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        debug_assert_eq!(self.by_deadline.len(), self.by_identity.len());
        self.by_deadline.len()
    }
}

impl PendingCommandQueue {
    fn next_sequence(&mut self, due_at: Instant) -> u64 {
        loop {
            let sequence = self.next_sequence;
            self.next_sequence = self.next_sequence.wrapping_add(1);
            if !self.by_deadline.contains_key(&(due_at, sequence)) {
                return sequence;
            }
        }
    }

    fn insert(&mut self, delivery: PendingCommandDelivery, due_at: Instant) -> bool {
        let replaced = self.unschedule(delivery.identity);
        let sequence = self.next_sequence(due_at);
        self.by_identity
            .insert(delivery.identity, ReverseDeadline { due_at, sequence });
        self.by_deadline.insert((due_at, sequence), delivery);
        replaced
    }

    fn unschedule(&mut self, identity: usize) -> bool {
        let Some(previous) = self.by_identity.remove(&identity) else {
            return false;
        };
        self.by_deadline
            .remove(&(previous.due_at, previous.sequence));
        true
    }

    fn next_due_at(&self) -> Option<Instant> {
        self.by_deadline.first_key_value().map(|(key, _)| key.0)
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<PendingCommandDelivery> {
        let mut due = Vec::with_capacity(limit.min(self.by_deadline.len()));
        while due.len() < limit {
            let Some((&(due_at, sequence), deadline)) = self.by_deadline.first_key_value() else {
                break;
            };
            if due_at > now {
                break;
            }
            let identity = deadline.identity;
            let Some(deadline) = self.by_deadline.remove(&(due_at, sequence)) else {
                continue;
            };
            if self
                .by_identity
                .get(&identity)
                .is_some_and(|reverse| reverse.due_at == due_at && reverse.sequence == sequence)
            {
                self.by_identity.remove(&identity);
                due.push(deadline);
            }
        }
        due
    }

    fn clear(&mut self) {
        self.by_deadline.clear();
        self.by_identity.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        debug_assert_eq!(self.by_deadline.len(), self.by_identity.len());
        self.by_deadline.len()
    }
}

impl LifecycleDeadlineQueue {
    fn next_sequence(&mut self, due_at: Instant) -> u64 {
        loop {
            let sequence = self.next_sequence;
            self.next_sequence = self.next_sequence.wrapping_add(1);
            if !self.by_deadline.contains_key(&(due_at, sequence)) {
                return sequence;
            }
        }
    }

    fn insert(
        &mut self,
        identity: usize,
        target: Arc<dyn LifecycleTarget>,
        phase: LifecycleDeadlinePhase,
        due_at: Instant,
    ) -> bool {
        let replaced = self.unschedule(identity);
        let sequence = self.next_sequence(due_at);
        self.by_identity
            .insert(identity, ReverseDeadline { due_at, sequence });
        self.by_deadline.insert(
            (due_at, sequence),
            LifecycleDeadline {
                identity,
                target,
                phase,
            },
        );
        replaced
    }

    fn unschedule(&mut self, identity: usize) -> bool {
        let Some(previous) = self.by_identity.remove(&identity) else {
            return false;
        };
        self.by_deadline
            .remove(&(previous.due_at, previous.sequence));
        true
    }

    fn next_due_at(&self) -> Option<Instant> {
        self.by_deadline.first_key_value().map(|(key, _)| key.0)
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<(Instant, LifecycleDeadline)> {
        let mut due = Vec::with_capacity(limit.min(self.by_deadline.len()));
        while due.len() < limit {
            let Some((&(due_at, sequence), deadline)) = self.by_deadline.first_key_value() else {
                break;
            };
            if due_at > now {
                break;
            }

            let identity = deadline.identity;
            let Some(deadline) = self.by_deadline.remove(&(due_at, sequence)) else {
                continue;
            };
            if self
                .by_identity
                .get(&identity)
                .is_some_and(|reverse| reverse.due_at == due_at && reverse.sequence == sequence)
            {
                self.by_identity.remove(&identity);
                due.push((due_at, deadline));
            }
        }
        due
    }

    fn destroy_all(&mut self) -> usize {
        let deadlines = std::mem::take(&mut self.by_deadline);
        self.by_identity.clear();
        let count = deadlines.len();
        for deadline in deadlines.into_values() {
            deadline
                .target
                .set_lifecycle(TransactionLifecycle::Destroyed);
            deadline.target.wake_destroyed_runner();
        }
        count
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        debug_assert_eq!(self.by_deadline.len(), self.by_identity.len());
        self.by_deadline.len()
    }
}

fn process_due(queue: &mut LifecycleDeadlineQueue, now: Instant) -> usize {
    let due = queue.take_due(now, MAX_DUE_PER_BATCH);
    let processed = due.len();
    for (due_at, deadline) in due {
        match deadline.phase {
            LifecycleDeadlinePhase::BeginDraining => {
                deadline
                    .target
                    .set_lifecycle(TransactionLifecycle::Draining);
                queue.insert(
                    deadline.identity,
                    deadline.target,
                    LifecycleDeadlinePhase::Destroy,
                    due_at + DRAINING_GRACE,
                );
            }
            LifecycleDeadlinePhase::Destroy => {
                deadline
                    .target
                    .set_lifecycle(TransactionLifecycle::Destroyed);
                deadline.target.wake_destroyed_runner();
            }
        }
    }
    processed
}

fn apply_schedule(queue: &mut LifecycleDeadlineQueue, request: ScheduleRequest) {
    let due_at = request.scheduled_at + TERMINATING_GRACE;
    let replaced = queue.insert(
        request.identity,
        request.target,
        LifecycleDeadlinePhase::BeginDraining,
        due_at,
    );
    if replaced {
        trace!("Replaced duplicate transaction lifecycle deadline");
    }
}

fn next_due_at(
    lifecycle: &LifecycleDeadlineQueue,
    compact: &CompactDeadlineQueue,
    standalone: &StandaloneTimerDeadlineQueue,
    pending_commands: &PendingCommandQueue,
    include_compact: bool,
) -> Option<Instant> {
    [
        lifecycle.next_due_at(),
        include_compact.then(|| compact.next_due_at()).flatten(),
        standalone.next_due_at(),
        pending_commands.next_due_at(),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn process_standalone_due(
    queue: &mut StandaloneTimerDeadlineQueue,
    pending_commands: &mut PendingCommandQueue,
    now: Instant,
) -> usize {
    let due = queue.take_due(now, MAX_DUE_PER_BATCH);
    let processed = due.len();
    for deadline in due {
        pending_commands.insert(
            PendingCommandDelivery {
                identity: deadline.identity,
                transaction_id: deadline.transaction_id,
                command_tx: deadline.command_tx,
                commands: VecDeque::from([
                    InternalTransactionCommand::Timer(deadline.timer.name().to_string()),
                    InternalTransactionCommand::TransitionTo(TransactionState::Terminated),
                ]),
            },
            now,
        );
    }
    processed
}

fn process_pending_commands(queue: &mut PendingCommandQueue, now: Instant) -> usize {
    let due = queue.take_due(now, MAX_DUE_PER_BATCH);
    let processed = due.len();
    for mut delivery in due {
        let Some(command) = delivery.commands.pop_front() else {
            continue;
        };
        match delivery.command_tx.try_send(command) {
            Ok(()) => {
                if !delivery.commands.is_empty() {
                    queue.insert(delivery, now);
                }
            }
            Err(mpsc::error::TrySendError::Full(command)) => {
                delivery.commands.push_front(command);
                queue.insert(delivery, now + COMMAND_RETRY_DELAY);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                trace!(
                    id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&delivery.transaction_id),
                    "Deferred transaction command target closed"
                );
            }
        }
    }
    processed
}

async fn apply_compact_schedule(
    queue: &mut CompactDeadlineQueue,
    pending_commands: &mut PendingCommandQueue,
    context: &ManagedCompactContext,
    request: CompactScheduleRequest,
) {
    let Some(tombstones) = context.tombstones.upgrade() else {
        let _ = request.accepted.send(false);
        return;
    };
    // A retained tombstone may already be waiting for its lossless terminal
    // observations to reach the TU. Reusing the wire key before that exact
    // generation is delivered would let the old event batch target a new
    // transaction (an ABA bug), so reject the replacement here as well as at
    // manager admission.
    if tombstones.contains_key(&request.transaction_id)
        || tombstones.len() >= context.max_compact_retained
    {
        let _ = request.accepted.send(false);
        return;
    }

    let due_at = request.scheduled_at + request.delay;
    let remaining = due_at.saturating_duration_since(Instant::now());
    let expires_at = StdInstant::now() + remaining;
    let auth_lease = context
        .inbound_principal_inserted_at
        .upgrade()
        .and_then(|leases| {
            leases
                .get(&request.transaction_id)
                .map(|entry| *entry.value())
        });
    let client_route_owner = if request.server_replay.is_none() {
        context.client_routes.upgrade().and_then(|routes| {
            routes
                .get(&request.transaction_id)
                .and_then(|entry| match entry.value() {
                    ClientResponseRouteState::Active { owner, .. }
                        if *owner == request.identity =>
                    {
                        Some(*owner)
                    }
                    ClientResponseRouteState::Active { .. } => None,
                    ClientResponseRouteState::Retired(_) => None,
                })
        })
    } else {
        None
    };
    if request.server_replay.is_none() && client_route_owner.is_none() {
        let _ = request.accepted.send(false);
        return;
    }
    let Some(generation) = queue.insert(request.transaction_id.clone(), request.timer, due_at)
    else {
        // The compact path is an optimization. Refusing it at the explicit
        // retention bound leaves the caller's full transaction runner in
        // place, where ordinary event-channel backpressure remains lossless.
        let _ = request.accepted.send(false);
        return;
    };
    let tombstone = match request.server_replay {
        Some((response_wire, response_route)) => CompactNonInviteTombstone::Server {
            _retention_reservation: request.retention_reservation,
            _admission_owner: request.admission_owner,
            terminal_event_publication: request.terminal_event_publication,
            state: request.state,
            auth_lease,
            response_wire,
            response_route,
            expires_at,
            generation,
        },
        None => {
            let live_completion = request
                .completion
                .expect("client compact schedules carry an exact completion cell");
            let completion = live_completion.retained(expires_at, generation);
            CompactNonInviteTombstone::Client {
                _retention_reservation: request.retention_reservation,
                _admission_owner: request.admission_owner,
                terminal_event_publication: request.terminal_event_publication,
                state: request.state,
                completion,
                live_completion: Arc::downgrade(&live_completion),
                auth_lease,
                response_route_owner: client_route_owner
                    .expect("validated client compact schedule carries a route owner"),
                expires_at,
                generation,
            }
        }
    };
    tombstones.insert(request.transaction_id.clone(), tombstone);

    // Never await this transaction's own command queue: its runner is waiting
    // for the acceptance acknowledgement before it can drain the command.
    match request
        .command_tx
        .try_send(InternalTransactionCommand::CompactRetire)
    {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(command)) => {
            pending_commands.insert(
                PendingCommandDelivery {
                    identity: request.identity,
                    transaction_id: request.transaction_id.clone(),
                    command_tx: request.command_tx,
                    commands: VecDeque::from([command]),
                },
                Instant::now(),
            );
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            queue.remove_generation(&request.transaction_id, generation);
            tombstones.remove_if(&request.transaction_id, |_, tombstone| {
                tombstone.generation() == generation
            });
            let _ = request.accepted.send(false);
            return;
        }
    }

    let _ = request.accepted.send(true);
}

fn apply_compact_expiry(queue: &mut CompactDeadlineQueue, request: CompactExpiryRequest) {
    queue.expedite_generation(&request.transaction_id, request.generation, Instant::now());
}

async fn publish_compact_terminal_events(
    events_tx: &crate::transaction::event_sender::TransactionEventSender,
    transaction_id: &TransactionKey,
    timer: CompactNonInviteTimer,
    generation: u64,
    admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    publication_claim: crate::transaction::event_sender::TerminalEventPublicationClaim,
    exact_dialog_ack: bool,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    let mut publication_claim = Some(publication_claim);
    // These are the same primary TU events the full runner produced at Timer
    // J/K expiry, in the same order. This channel is the lossless protocol
    // path; optional observers are fanned out separately and bounded.
    for (index, event) in [
        TransactionEvent::TimerTriggered {
            transaction_id: transaction_id.clone(),
            timer: timer.name().to_string(),
        },
        TransactionEvent::StateChanged {
            transaction_id: transaction_id.clone(),
            previous_state: TransactionState::Completed,
            new_state: TransactionState::Terminated,
        },
        TransactionEvent::TransactionTerminated {
            transaction_id: transaction_id.clone(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let send = async {
            if index == 2 {
                events_tx
                    .send_terminal(
                        event,
                        exact_dialog_ack.then_some(generation),
                        admission_owner.clone(),
                    )
                    .await
            } else if index == 1 {
                events_tx
                    .send_terminal_prefix(
                        event,
                        publication_claim
                            .as_ref()
                            .expect("compact terminal publication claim is live"),
                    )
                    .await
            } else {
                events_tx.send(event).await
            }
        };
        tokio::select! {
            result = send => {
                if result.is_err() {
                    // Keep `admission_owner` alive until admission is closed.
                    // Losing any prefix of the authoritative three-event
                    // sequence is fail-closed, not an ordinary observer loss.
                    events_tx.fail_closed_terminal_batch();
                    publication_claim
                        .take()
                        .expect("compact terminal publication claim is live")
                        .mark_failed_closed();
                    trace!(
                        id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                        timer = timer.name(),
                        "Compact non-INVITE primary TU channel closed"
                    );
                    return false;
                }
                if index == 2 {
                    publication_claim
                        .take()
                        .expect("compact terminal publication claim is live")
                        .mark_delivered();
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    events_tx.fail_closed_terminal_batch();
                    publication_claim
                        .take()
                        .expect("compact terminal publication claim is live")
                        .mark_failed_closed();
                    return false;
                }
            }
        }
    }
    true
}

async fn run_compact_event_dispatcher(
    mut batches: mpsc::Receiver<CompactTerminalEventBatch>,
    events_tx: crate::transaction::event_sender::TransactionEventSender,
    mut shutdown: watch::Receiver<bool>,
    context: ManagedCompactContext,
) {
    loop {
        let batch = tokio::select! {
            batch = batches.recv() => batch,
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    while let Ok(batch) = batches.try_recv() {
                        batch.publication_claim.mark_failed_closed();
                        cleanup_compact_generation(
                            &context,
                            &batch.transaction_id,
                            batch.generation,
                        );
                    }
                    return;
                }
                continue;
            }
        };
        let Some(batch) = batch else {
            if let Some(events_tx) = context.protocol_failure_sender.as_ref() {
                events_tx.fail_closed_terminal_batch();
            }
            return;
        };
        let exact_dialog_ack = context
            .dialog_ack_required
            .as_ref()
            .is_some_and(|required| required.load(Ordering::Acquire));
        let delivered = publish_compact_terminal_events(
            &events_tx,
            &batch.transaction_id,
            batch.timer,
            batch.generation,
            batch.admission_owner,
            batch.publication_claim,
            exact_dialog_ack,
            &mut shutdown,
        )
        .await;
        if !delivered || !exact_dialog_ack {
            cleanup_compact_generation(&context, &batch.transaction_id, batch.generation);
        }
        if !delivered {
            // The primary TU channel or shutdown signal ended delivery. Every
            // already-staged batch must release its exact fence; future due
            // entries observe the closed bounded channel and clean directly.
            while let Ok(batch) = batches.try_recv() {
                batch.publication_claim.mark_failed_closed();
                cleanup_compact_generation(&context, &batch.transaction_id, batch.generation);
            }
            return;
        }
    }
}

/// Release one compact generation and all of its derived indexes.  Keeping
/// this operation in one exact-generation helper ensures deadline expiry,
/// event completion, and shutdown cannot remove a replacement transaction.
fn cleanup_compact_generation(
    context: &ManagedCompactContext,
    transaction_id: &TransactionKey,
    generation: u64,
) -> bool {
    let Some(tombstones) = context.tombstones.upgrade() else {
        return false;
    };
    let tombstone = tombstones
        .get(transaction_id)
        .filter(|entry| entry.value().generation() == generation)
        .map(|entry| entry.value().clone());
    let Some(tombstone) = tombstone else {
        return false;
    };

    if tombstone.is_client() {
        if let Some(client_routes) = context.client_routes.upgrade() {
            client_routes.remove_if(transaction_id, |_, state| {
                matches!(
                    (state, tombstone.client_route_owner()),
                    (ClientResponseRouteState::Active { owner, .. }, Some(expected))
                        if *owner == expected
                )
            });
        }
    }
    if let (Some(expected_lease), Some(inserted_at)) = (
        tombstone.auth_lease(),
        context.inbound_principal_inserted_at.upgrade(),
    ) {
        if inserted_at
            .remove_if(transaction_id, |_, lease| *lease == expected_lease)
            .is_some()
        {
            if let Some(inbound_principals) = context.inbound_principals.upgrade() {
                inbound_principals.remove(transaction_id);
            }
        }
    }

    // Clear the bare-key observer bucket while the exact terminal tombstone
    // still fences same-key admission. A racing subscriber revalidates the
    // Terminated generation and removes only itself; generation B cannot be
    // admitted until after this cleanup is complete.
    if let Some(observers) = context.observer_fanout.as_ref() {
        observers.remove_transaction(transaction_id);
    }
    tombstones
        .remove_if(transaction_id, |_, current| {
            current.generation() == generation
        })
        .is_some()
}

/// Finalize a compact generation after the integrated dialog worker has
/// actually processed its terminal event. This direct exact operation cannot
/// be starved behind the normal 262k-entry scheduling channel.
pub(crate) fn acknowledge_dialog_terminal_generation(
    tombstones: &Arc<CompactNonInviteTombstones>,
    client_routes: &Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
    inbound_principals: &Arc<
        DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalBinding>,
    >,
    inbound_principal_inserted_at: &Arc<
        DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>,
    >,
    transaction_id: &TransactionKey,
    generation: u64,
    observer_fanout: Option<crate::transaction::event_sender::TransactionObserverFanout>,
) -> bool {
    cleanup_compact_generation(
        &ManagedCompactContext {
            tombstones: Arc::downgrade(tombstones),
            client_routes: Arc::downgrade(client_routes),
            inbound_principals: Arc::downgrade(inbound_principals),
            inbound_principal_inserted_at: Arc::downgrade(inbound_principal_inserted_at),
            compact_events_tx: None,
            dialog_ack_required: None,
            observer_fanout,
            protocol_failure_sender: None,
            max_compact_retained: MAX_COMPACT_RETAINED,
        },
        transaction_id,
        generation,
    )
}

fn process_compact_due(
    queue: &mut CompactDeadlineQueue,
    context: &ManagedCompactContext,
    now: Instant,
) -> usize {
    let tombstones = context.tombstones.upgrade();
    let mut processed = 0;

    while processed < MAX_DUE_PER_BATCH {
        // Reserve bounded staging capacity before removing the authoritative
        // deadline. A stalled TU therefore leaves the generation in the due
        // queue instead of accumulating an unbounded side queue.
        let event_permit = match context.compact_events_tx.as_ref() {
            Some(events_tx) if !events_tx.is_closed() => match events_tx.try_reserve() {
                Ok(permit) => Some(permit),
                Err(mpsc::error::TrySendError::Full(_)) => break,
                Err(mpsc::error::TrySendError::Closed(_)) => None,
            },
            _ => None,
        };
        let Some(deadline) = queue.take_due(now, 1).pop() else {
            break;
        };
        processed += 1;
        let tombstone = tombstones.as_ref().and_then(|tombstones| {
            tombstones
                .get(deadline.transaction_id.as_ref())
                .filter(|entry| entry.value().generation() == deadline.generation)
                .map(|entry| entry.value().clone())
        });
        let Some(tombstone) = tombstone else {
            continue;
        };
        let Some(publication_claim) = tombstone.claim_terminal_event() else {
            cleanup_compact_generation(
                context,
                deadline.transaction_id.as_ref(),
                deadline.generation,
            );
            continue;
        };
        publication_claim
            .publication()
            .record_prefix(TransactionState::Completed);
        tombstone.state().set(TransactionState::Terminated);
        // Authoritative exact state always precedes the matching public
        // compact Timer K observations. The weak upgrade wakes only a waiter
        // that already owned the live cell; ordinary retirement retains no
        // mutex/notify allocation through T4.
        tombstone.wake_live_client_completion(TransactionState::Terminated);
        let batch = CompactTerminalEventBatch {
            transaction_id: deadline.transaction_id.as_ref().clone(),
            timer: deadline.timer,
            generation: deadline.generation,
            admission_owner: tombstone.admission_owner(),
            publication_claim,
        };
        if let Some(permit) = event_permit {
            permit.send(batch);
        } else {
            // Once the event dispatcher has closed there is no TU capable of
            // receiving the observation. Fail closed while the batch still
            // owns the exact admission fence, then release the undeliverable
            // generation.
            if let Some(events_tx) = context.protocol_failure_sender.as_ref() {
                events_tx.fail_closed_terminal_batch();
            }
            batch.publication_claim.mark_failed_closed();
            cleanup_compact_generation(context, &batch.transaction_id, batch.generation);
        }
    }
    processed
}

fn clear_compact_state(queue: &mut CompactDeadlineQueue, context: &ManagedCompactContext) {
    queue.clear();
    if let Some(tombstones) = context.tombstones.upgrade() {
        let entries: Vec<_> = tombstones
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        if let Some(client_routes) = context.client_routes.upgrade() {
            for (key, tombstone) in &entries {
                client_routes.remove_if(key, |_, route| {
                    matches!(
                        (route, tombstone.client_route_owner()),
                        (ClientResponseRouteState::Active { owner, .. }, Some(expected))
                            if *owner == expected
                    )
                });
            }
        }
        if let (Some(inbound_principals), Some(inserted_at)) = (
            context.inbound_principals.upgrade(),
            context.inbound_principal_inserted_at.upgrade(),
        ) {
            for (key, tombstone) in &entries {
                if let Some(expected_lease) = tombstone.auth_lease() {
                    if inserted_at
                        .remove_if(key, |_, lease| *lease == expected_lease)
                        .is_some()
                    {
                        inbound_principals.remove(key);
                    }
                }
            }
        }
        for (_, tombstone) in &entries {
            tombstone.state().set(TransactionState::Terminated);
        }
        if let Some(observers) = context.observer_fanout.as_ref() {
            for (key, _) in &entries {
                observers.remove_transaction(key);
            }
        }
        tombstones.clear();
    }
}

async fn run_scheduler(
    mut rx: mpsc::Receiver<SchedulerCommand>,
    context: ManagedCompactContext,
    compact_deadline_count: Arc<AtomicUsize>,
) {
    let mut queue = LifecycleDeadlineQueue::default();
    let compact_retention_limit = if context.max_compact_retained == 0 {
        MAX_COMPACT_RETAINED
    } else {
        context.max_compact_retained
    };
    let mut compact_queue = CompactDeadlineQueue::with_capacity(compact_retention_limit);
    let mut standalone_timer_queue = StandaloneTimerDeadlineQueue::default();
    let mut pending_commands = PendingCommandQueue::default();
    debug!("Consolidated transaction lifecycle scheduler started");

    loop {
        let now = Instant::now();
        let lifecycle_processed = process_due(&mut queue, now);
        let compact_processed = process_compact_due(&mut compact_queue, &context, now);
        let standalone_processed =
            process_standalone_due(&mut standalone_timer_queue, &mut pending_commands, now);
        let command_deliveries_processed = process_pending_commands(&mut pending_commands, now);
        compact_deadline_count.store(compact_queue.len(), Ordering::Release);
        if (lifecycle_processed == MAX_DUE_PER_BATCH
            && queue.next_due_at().is_some_and(|due_at| due_at <= now))
            || (compact_processed == MAX_DUE_PER_BATCH
                && compact_queue
                    .next_due_at()
                    .is_some_and(|due_at| due_at <= now))
            || (standalone_processed == MAX_DUE_PER_BATCH
                && standalone_timer_queue
                    .next_due_at()
                    .is_some_and(|due_at| due_at <= now))
            || (command_deliveries_processed == MAX_DUE_PER_BATCH
                && pending_commands
                    .next_due_at()
                    .is_some_and(|due_at| due_at <= now))
        {
            tokio::task::yield_now().await;
            continue;
        }

        let compact_backpressured = compact_queue
            .next_due_at()
            .is_some_and(|due_at| due_at <= now)
            && context
                .compact_events_tx
                .as_ref()
                .is_some_and(|events_tx| !events_tx.is_closed() && events_tx.capacity() == 0);
        let first = if compact_backpressured {
            let events_tx = context
                .compact_events_tx
                .as_ref()
                .expect("compact backpressure requires an event sender");
            match next_due_at(
                &queue,
                &compact_queue,
                &standalone_timer_queue,
                &pending_commands,
                false,
            ) {
                Some(due_at) => {
                    tokio::select! {
                        command = rx.recv() => command,
                        permit = events_tx.reserve() => {
                            drop(permit);
                            continue;
                        }
                        _ = tokio::time::sleep_until(due_at) => continue,
                    }
                }
                None => {
                    tokio::select! {
                        command = rx.recv() => command,
                        permit = events_tx.reserve() => {
                            drop(permit);
                            continue;
                        }
                    }
                }
            }
        } else {
            match next_due_at(
                &queue,
                &compact_queue,
                &standalone_timer_queue,
                &pending_commands,
                true,
            ) {
                Some(due_at) => {
                    tokio::select! {
                        command = rx.recv() => command,
                        _ = tokio::time::sleep_until(due_at) => continue,
                    }
                }
                None => rx.recv().await,
            }
        };

        let Some(command) = first else {
            // Every manager/data handle was dropped. Do not retain transaction
            // targets merely because their grace deadline was in the future.
            queue.destroy_all();
            clear_compact_state(&mut compact_queue, &context);
            standalone_timer_queue.clear();
            pending_commands.clear();
            compact_deadline_count.store(0, Ordering::Release);
            break;
        };

        match command {
            SchedulerCommand::Schedule(request) => apply_schedule(&mut queue, request),
            SchedulerCommand::ScheduleCompact(request) => {
                apply_compact_schedule(
                    &mut compact_queue,
                    &mut pending_commands,
                    &context,
                    request,
                )
                .await;
                compact_deadline_count.store(compact_queue.len(), Ordering::Release);
            }
            SchedulerCommand::ScheduleStandaloneTimer(request) => {
                if standalone_timer_queue.insert(request) {
                    trace!("Replaced duplicate standalone Timer J/K deadline");
                }
            }
            SchedulerCommand::ScheduleCommands(request) => {
                pending_commands.insert(
                    PendingCommandDelivery {
                        identity: request.identity,
                        transaction_id: request.transaction_id,
                        command_tx: request.command_tx,
                        commands: request.commands,
                    },
                    request.scheduled_at,
                );
            }
            SchedulerCommand::ExpireCompact(request) => {
                apply_compact_expiry(&mut compact_queue, request);
            }
            SchedulerCommand::Shutdown(ack) => {
                queue.destroy_all();
                clear_compact_state(&mut compact_queue, &context);
                standalone_timer_queue.clear();
                pending_commands.clear();
                compact_deadline_count.store(0, Ordering::Release);
                let _ = ack.send(());
                break;
            }
        }

        for _ in 1..MAX_SCHEDULES_PER_BATCH {
            match rx.try_recv() {
                Ok(SchedulerCommand::Schedule(request)) => apply_schedule(&mut queue, request),
                Ok(SchedulerCommand::ScheduleCompact(request)) => {
                    apply_compact_schedule(
                        &mut compact_queue,
                        &mut pending_commands,
                        &context,
                        request,
                    )
                    .await;
                    compact_deadline_count.store(compact_queue.len(), Ordering::Release);
                }
                Ok(SchedulerCommand::ScheduleStandaloneTimer(request)) => {
                    if standalone_timer_queue.insert(request) {
                        trace!("Replaced duplicate standalone Timer J/K deadline");
                    }
                }
                Ok(SchedulerCommand::ScheduleCommands(request)) => {
                    pending_commands.insert(
                        PendingCommandDelivery {
                            identity: request.identity,
                            transaction_id: request.transaction_id,
                            command_tx: request.command_tx,
                            commands: request.commands,
                        },
                        request.scheduled_at,
                    );
                }
                Ok(SchedulerCommand::ExpireCompact(request)) => {
                    apply_compact_expiry(&mut compact_queue, request);
                }
                Ok(SchedulerCommand::Shutdown(ack)) => {
                    queue.destroy_all();
                    clear_compact_state(&mut compact_queue, &context);
                    standalone_timer_queue.clear();
                    pending_commands.clear();
                    compact_deadline_count.store(0, Ordering::Release);
                    let _ = ack.send(());
                    debug!("Consolidated transaction lifecycle scheduler stopped");
                    return;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
    }

    debug!("Consolidated transaction lifecycle scheduler stopped");
}

/// Schedule the exact terminal lifecycle for one transaction without creating
/// a transaction-specific task or Tokio sleep entry. Manager-owned
/// transactions use their manager's worker; low-level public constructors use
/// one shared compatibility worker per live Tokio runtime.
pub(super) async fn schedule<D>(data: Arc<D>)
where
    D: AsRefKey + HasCommandSender + HasLifecycle + Send + Sync + 'static,
{
    let manager_owned = data.lifecycle_scheduler_installed();
    if data.clone().schedule_lifecycle().await {
        return;
    }

    if manager_owned {
        // Manager shutdown won the race with this terminal transition.
        // Finish immediately instead of recreating a scheduler that the
        // manager can no longer own or drain.
        data.set_lifecycle(TransactionLifecycle::Destroyed);
        DataLifecycleTarget { data }.wake_destroyed_runner();
        return;
    }

    if standalone_scheduler()
        .schedule_lifecycle(data.clone())
        .await
    {
        return;
    }

    // Runtime shutdown won the enqueue race. There is no owner left to retain
    // a compatibility grace period, so release the runner synchronously.
    data.set_lifecycle(TransactionLifecycle::Destroyed);
    DataLifecycleTarget { data }.wake_destroyed_runner();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    fn managed_scheduler_fixture() -> (
        LifecycleSchedulerHandle,
        Arc<CompactNonInviteTombstones>,
        Arc<DashMap<Arc<TransactionKey>, ClientResponseRouteState>>,
        Arc<DashMap<TransactionKey, crate::transaction::manager::InboundPrincipalLease>>,
        mpsc::Sender<TransactionEvent>,
        mpsc::Receiver<TransactionEvent>,
    ) {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, events_rx) = mpsc::channel(16);
        let event_sender =
            crate::transaction::event_sender::TransactionEventSender::new(events_tx.clone());
        let scheduler = LifecycleSchedulerHandle::new_managed(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
        );
        (scheduler, tombstones, routes, leases, events_tx, events_rx)
    }

    struct TestTarget {
        id: TransactionKey,
        lifecycle: AtomicU8,
        wakes: AtomicUsize,
    }

    impl TestTarget {
        fn new(branch: &str) -> Self {
            Self {
                id: TransactionKey::new(branch.to_string(), rvoip_sip_core::Method::Options, false),
                lifecycle: AtomicU8::new(0),
                wakes: AtomicUsize::new(0),
            }
        }
    }

    impl LifecycleTarget for TestTarget {
        fn transaction_id(&self) -> &TransactionKey {
            &self.id
        }

        fn set_lifecycle(&self, lifecycle: TransactionLifecycle) {
            self.lifecycle.store(lifecycle as u8, Ordering::Release);
        }

        fn wake_destroyed_runner(&self) {
            self.wakes.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn deadlines_are_exact_and_duplicate_schedules_replace() {
        let mut queue = LifecycleDeadlineQueue::default();
        let now = Instant::now();
        let first = Arc::new(TestTarget::new("first"));
        let replacement = Arc::new(TestTarget::new("replacement"));

        assert!(!queue.insert(
            7,
            first.clone(),
            LifecycleDeadlinePhase::BeginDraining,
            now + Duration::from_millis(10),
        ));
        assert!(queue.insert(
            7,
            replacement.clone(),
            LifecycleDeadlinePhase::BeginDraining,
            now + Duration::from_millis(20),
        ));
        assert_eq!(queue.len(), 1);
        assert_eq!(process_due(&mut queue, now + Duration::from_millis(15)), 0);
        assert_eq!(first.lifecycle.load(Ordering::Acquire), 0);

        assert_eq!(process_due(&mut queue, now + Duration::from_millis(20)), 1);
        assert_eq!(replacement.lifecycle.load(Ordering::Acquire), 2);
        assert_eq!(queue.len(), 1);
        assert_eq!(
            process_due(&mut queue, now + Duration::from_millis(20) + DRAINING_GRACE,),
            1
        );
        assert_eq!(replacement.lifecycle.load(Ordering::Acquire), 3);
        assert_eq!(replacement.wakes.load(Ordering::Relaxed), 1);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn compact_deadline_capacity_and_expiry_are_generation_exact() {
        let mut queue = CompactDeadlineQueue::with_capacity(2);
        let now = Instant::now();
        let first = TransactionKey::new(
            "compact-cap-first".into(),
            rvoip_sip_core::Method::Bye,
            true,
        );
        let second = TransactionKey::new(
            "compact-cap-second".into(),
            rvoip_sip_core::Method::Bye,
            true,
        );
        let third = TransactionKey::new(
            "compact-cap-third".into(),
            rvoip_sip_core::Method::Bye,
            true,
        );
        let first_generation = queue
            .insert(
                first.clone(),
                CompactNonInviteTimer::J,
                now + Duration::from_secs(10),
            )
            .expect("first compact deadline");
        queue
            .insert(
                second,
                CompactNonInviteTimer::J,
                now + Duration::from_secs(10),
            )
            .expect("second compact deadline");
        assert!(
            queue
                .insert(
                    third,
                    CompactNonInviteTimer::J,
                    now + Duration::from_secs(10),
                )
                .is_none(),
            "compact retention must reject new keys at its explicit bound"
        );
        assert!(!queue.expedite_generation(&first, first_generation + 1, now));
        assert_eq!(queue.next_due_at(), Some(now + Duration::from_secs(10)));
        assert!(queue.expedite_generation(&first, first_generation, now));
        let due = queue.take_due(now, 1);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].generation, first_generation);
    }

    #[test]
    fn stale_compact_cleanup_cannot_remove_replacement_generation() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let retention_capacity = CompactRetentionCapacity::new(1);
        let key = TransactionKey::new(
            "compact-expiry-replacement".into(),
            rvoip_sip_core::Method::Bye,
            true,
        );
        tombstones.insert(
            key.clone(),
            CompactNonInviteTombstone::Server {
                _retention_reservation: retention_capacity
                    .try_reserve()
                    .expect("test retention reservation"),
                _admission_owner: None,
                terminal_event_publication:
                    crate::transaction::event_sender::TerminalEventPublication::new(),
                state: Arc::new(crate::transaction::AtomicTransactionState::new(
                    TransactionState::Completed,
                )),
                auth_lease: None,
                response_wire: bytes::Bytes::from_static(
                    b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n",
                ),
                response_route: TransportRoute::new("127.0.0.1:5098".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                expires_at: StdInstant::now() + Duration::from_secs(1),
                generation: 2,
            },
        );

        assert!(!acknowledge_dialog_terminal_generation(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &key,
            1,
            None,
        ));
        assert_eq!(
            tombstones
                .get(&key)
                .expect("replacement retained")
                .generation(),
            2
        );
    }

    #[test]
    fn due_work_is_bounded_per_batch() {
        let mut queue = LifecycleDeadlineQueue::default();
        let now = Instant::now();
        for identity in 0..(MAX_DUE_PER_BATCH + 7) {
            queue.insert(
                identity,
                Arc::new(TestTarget::new(&format!("batch-{identity}"))),
                LifecycleDeadlinePhase::Destroy,
                now,
            );
        }

        assert_eq!(process_due(&mut queue, now), MAX_DUE_PER_BATCH);
        assert_eq!(queue.len(), 7);
        assert_eq!(process_due(&mut queue, now), 7);
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn standalone_timer_deadlines_replace_and_fire_without_sleeper_tasks() {
        let mut queue = StandaloneTimerDeadlineQueue::default();
        let key = TransactionKey::new(
            "standalone-timer".into(),
            rvoip_sip_core::Method::Bye,
            false,
        );
        let (old_tx, mut old_rx) = mpsc::channel(2);
        let (replacement_tx, mut replacement_rx) = mpsc::channel(2);

        assert!(!queue.insert(StandaloneTimerScheduleRequest {
            identity: 1,
            transaction_id: key.clone(),
            timer: CompactNonInviteTimer::K,
            delay: Duration::from_secs(60),
            scheduled_at: Instant::now(),
            command_tx: old_tx,
        }));
        assert!(queue.insert(StandaloneTimerScheduleRequest {
            identity: 1,
            transaction_id: key,
            timer: CompactNonInviteTimer::K,
            delay: Duration::ZERO,
            scheduled_at: Instant::now(),
            command_tx: replacement_tx,
        }));
        assert_eq!(queue.len(), 1);

        let mut pending = PendingCommandQueue::default();
        let now = Instant::now();
        assert_eq!(process_standalone_due(&mut queue, &mut pending, now), 1);
        assert_eq!(pending.len(), 1);
        assert_eq!(process_pending_commands(&mut pending, now), 1);
        assert_eq!(process_pending_commands(&mut pending, now), 1);
        assert_eq!(pending.len(), 0);
        assert_eq!(queue.len(), 0);
        assert!(old_rx.try_recv().is_err());
        assert!(matches!(
            replacement_rx.recv().await,
            Some(InternalTransactionCommand::Timer(ref name)) if name == "K"
        ));
        assert!(matches!(
            replacement_rx.recv().await,
            Some(InternalTransactionCommand::TransitionTo(
                TransactionState::Terminated
            ))
        ));
    }

    #[test]
    fn standalone_timer_identity_is_allocation_scoped_not_sip_key_scoped() {
        let mut queue = StandaloneTimerDeadlineQueue::default();
        let key = TransactionKey::new("shared-wire-key".into(), rvoip_sip_core::Method::Bye, false);
        let (first_tx, _first_rx) = mpsc::channel(1);
        let (second_tx, _second_rx) = mpsc::channel(1);
        let scheduled_at = Instant::now();

        assert!(!queue.insert(StandaloneTimerScheduleRequest {
            identity: 10,
            transaction_id: key.clone(),
            timer: CompactNonInviteTimer::K,
            delay: Duration::from_secs(1),
            scheduled_at,
            command_tx: first_tx,
        }));
        assert!(!queue.insert(StandaloneTimerScheduleRequest {
            identity: 11,
            transaction_id: key,
            timer: CompactNonInviteTimer::K,
            delay: Duration::from_secs(1),
            scheduled_at,
            command_tx: second_tx,
        }));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn standalone_timer_deadline_starts_at_enqueue_not_worker_dequeue() {
        let mut queue = StandaloneTimerDeadlineQueue::default();
        let scheduled_at = Instant::now();
        let delay = Duration::from_secs(5);
        let (command_tx, _command_rx) = mpsc::channel(1);
        queue.insert(StandaloneTimerScheduleRequest {
            identity: 12,
            transaction_id: TransactionKey::new(
                "enqueue-time".into(),
                rvoip_sip_core::Method::Bye,
                false,
            ),
            timer: CompactNonInviteTimer::K,
            delay,
            scheduled_at,
            command_tx,
        });
        assert_eq!(queue.next_due_at(), Some(scheduled_at + delay));
    }

    #[tokio::test(start_paused = true)]
    async fn paused_tokio_clock_drives_shared_deadlines_exactly() {
        let scheduler = LifecycleSchedulerHandle::new();
        let (command_tx, mut command_rx) = mpsc::channel(4);
        assert!(
            scheduler
                .schedule_standalone_timer(
                    20,
                    TransactionKey::new("paused-clock".into(), rvoip_sip_core::Method::Bye, false,),
                    CompactNonInviteTimer::K,
                    Duration::from_secs(10),
                    command_tx,
                )
                .await
        );
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert!(command_rx.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::Timer(ref name)) if name == "K"
        ));
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::TransitionTo(
                TransactionState::Terminated
            ))
        ));
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn full_command_queue_does_not_block_other_deadlines_or_shutdown() {
        let scheduler = LifecycleSchedulerHandle::new();
        let (blocked_tx, mut blocked_rx) = mpsc::channel(1);
        blocked_tx
            .send(InternalTransactionCommand::Terminate)
            .await
            .unwrap();
        let (ready_tx, mut ready_rx) = mpsc::channel(4);

        assert!(
            scheduler
                .schedule_standalone_timer(
                    30,
                    TransactionKey::new("blocked".into(), rvoip_sip_core::Method::Bye, false,),
                    CompactNonInviteTimer::K,
                    Duration::ZERO,
                    blocked_tx,
                )
                .await
        );
        assert!(
            scheduler
                .schedule_standalone_timer(
                    31,
                    TransactionKey::new("ready".into(), rvoip_sip_core::Method::Bye, false,),
                    CompactNonInviteTimer::K,
                    Duration::ZERO,
                    ready_tx,
                )
                .await
        );

        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), ready_rx.recv())
                .await
                .expect("unblocked transaction deadline must fire"),
            Some(InternalTransactionCommand::Timer(ref name)) if name == "K"
        ));
        tokio::time::timeout(Duration::from_millis(100), scheduler.shutdown())
            .await
            .expect("full peer queue must not block scheduler shutdown");
        assert!(matches!(
            blocked_rx.recv().await,
            Some(InternalTransactionCommand::Terminate)
        ));
    }

    #[tokio::test]
    async fn low_level_transactions_share_one_scheduler_per_runtime() {
        let first = standalone_scheduler();
        let second = standalone_scheduler();
        assert!(first.sender.same_channel(&second.sender));
    }

    #[test]
    fn shutdown_destroys_future_deadlines_without_waiting() {
        let mut queue = LifecycleDeadlineQueue::default();
        let now = Instant::now();
        let target = Arc::new(TestTarget::new("shutdown"));
        queue.insert(
            42,
            target.clone(),
            LifecycleDeadlinePhase::BeginDraining,
            now + Duration::from_secs(60),
        );

        assert_eq!(queue.destroy_all(), 1);
        assert_eq!(target.lifecycle.load(Ordering::Acquire), 3);
        assert_eq!(target.wakes.load(Ordering::Relaxed), 1);
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn compact_timer_k_absorber_expires_with_ordered_terminal_events() {
        let (scheduler, tombstones, routes, _leases, _events_tx, mut events_rx) =
            managed_scheduler_fixture();
        let weak = scheduler.downgrade();
        let key = TransactionKey::new("compact-k".into(), rvoip_sip_core::Method::Bye, false);
        let route = TransportRoute::new("127.0.0.1:5090".parse().unwrap())
            .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp);
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(route, 1),
        );
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let (command_tx, mut command_rx) = mpsc::channel(4);

        assert!(
            weak.schedule_compact_non_invite(
                1,
                key.clone(),
                CompactNonInviteTimer::K,
                Duration::from_millis(20),
                None,
                state.clone(),
                Some(Arc::new(
                    crate::transaction::completion::ClientTransactionCompletion::new(
                        TransactionState::Completed,
                    ),
                )),
                command_tx,
            )
            .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        assert!(tombstones.contains_key(&key));
        assert_eq!(scheduler.compact_deadline_count(), 1);

        tokio::time::sleep(Duration::from_millis(40)).await;
        let first = events_rx.recv().await.expect("Timer K event");
        let second = events_rx.recv().await.expect("terminal state event");
        let third = events_rx.recv().await.expect("termination event");
        assert!(matches!(
            first,
            TransactionEvent::TimerTriggered { ref transaction_id, ref timer }
                if transaction_id == &key && timer == "K"
        ));
        assert!(matches!(
            second,
            TransactionEvent::StateChanged {
                ref transaction_id,
                previous_state: TransactionState::Completed,
                new_state: TransactionState::Terminated,
            } if transaction_id == &key
        ));
        assert!(matches!(
            third,
            TransactionEvent::TransactionTerminated { ref transaction_id }
                if transaction_id == &key
        ));
        assert_eq!(state.get(), TransactionState::Terminated);
        assert!(!tombstones.contains_key(&key));
        assert!(!routes.contains_key(&key));
        assert_eq!(scheduler.compact_deadline_count(), 0);
        scheduler.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn saturated_timer_k_reservation_absorbs_duplicates_for_full_t4_horizon() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, mut events_rx) = mpsc::channel(8);
        let event_sender = crate::transaction::event_sender::TransactionEventSender::new(events_tx);
        let scheduler = LifecycleSchedulerHandle::new_managed_with_limits(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
            8,
            1,
        );
        let key = TransactionKey::new(
            "timer-k-full-horizon".into(),
            rvoip_sip_core::Method::Message,
            false,
        );
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(
                TransportRoute::new("127.0.0.1:5090".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                81,
            ),
        );
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let (command_tx, mut command_rx) = mpsc::channel(2);
        let horizon = Duration::from_secs(5);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    81,
                    key.clone(),
                    CompactNonInviteTimer::K,
                    horizon,
                    None,
                    state.clone(),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    command_tx,
                )
                .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        assert_eq!(scheduler.compact_retention_in_use(), 1);
        assert!(scheduler.try_reserve_compact_retention().is_none());

        tokio::time::advance(horizon - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(tombstones.get(&key).is_some_and(|entry| entry.is_client()));
        assert_eq!(state.get(), TransactionState::Completed);

        tokio::time::advance(Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        for _ in 0..3 {
            events_rx.recv().await.expect("Timer K terminal event");
        }
        assert!(!tombstones.contains_key(&key));
        assert_eq!(scheduler.compact_retention_in_use(), 0);
        drop(
            scheduler
                .try_reserve_compact_retention()
                .expect("Timer K expiry returns its admission slot"),
        );
        scheduler.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn saturated_timer_j_replays_final_response_for_full_64_t1_horizon() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, mut events_rx) = mpsc::channel(8);
        let event_sender = crate::transaction::event_sender::TransactionEventSender::new(events_tx);
        let scheduler = LifecycleSchedulerHandle::new_managed_with_limits(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
            8,
            1,
        );
        let key = TransactionKey::new(
            "timer-j-full-horizon".into(),
            rvoip_sip_core::Method::Options,
            true,
        );
        let route = TransportRoute::new("127.0.0.1:5091".parse().unwrap())
            .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp);
        let wire = bytes::Bytes::from_static(b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        let (command_tx, mut command_rx) = mpsc::channel(2);
        let horizon = Duration::from_secs(32);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    82,
                    key.clone(),
                    CompactNonInviteTimer::J,
                    horizon,
                    Some((wire.clone(), route.clone())),
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    None,
                    command_tx,
                )
                .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        assert!(scheduler.try_reserve_compact_retention().is_none());

        tokio::time::advance(horizon - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        let retained = tombstones.get(&key).expect("Timer J replay fence");
        let (retained_wire, retained_route) = retained.server_replay().expect("server replay");
        assert_eq!(retained_wire, &wire);
        assert_eq!(retained_route, &route);
        drop(retained);

        tokio::time::advance(Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        for _ in 0..3 {
            events_rx.recv().await.expect("Timer J terminal event");
        }
        assert!(!tombstones.contains_key(&key));
        assert_eq!(scheduler.compact_retention_in_use(), 0);
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn shared_primary_without_dialog_ack_leaves_no_sidecar_or_fence() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, mut events_rx) = mpsc::channel(8);
        let event_sender =
            crate::transaction::event_sender::TransactionEventSender::new_shared(events_tx);
        let scheduler = LifecycleSchedulerHandle::new_managed(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
        );
        let key = TransactionKey::new(
            "compact-shared-raw".into(),
            rvoip_sip_core::Method::Bye,
            true,
        );
        let (command_tx, mut command_rx) = mpsc::channel(2);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    61_001,
                    key.clone(),
                    CompactNonInviteTimer::J,
                    Duration::ZERO,
                    Some((
                        bytes::Bytes::from_static(b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n",),
                        TransportRoute::new("127.0.0.1:5096".parse().unwrap()).with_transport_type(
                            rvoip_sip_transport::transport::TransportType::Udp,
                        ),
                    )),
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    None,
                    command_tx,
                )
                .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        let mut terminal = None;
        for index in 0..3 {
            let event = events_rx.recv().await.expect("raw shared terminal event");
            if index == 2 {
                terminal = Some(event);
            }
        }
        assert!(!tombstones.contains_key(&key));
        assert_eq!(
            event_sender.take_compact_terminal_generation(
                terminal.as_ref().expect("raw shared final event")
            ),
            None,
            "shared consumers without an integrated dialog ACK must not retain sidecars"
        );
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn dialog_ack_fence_blocks_same_key_reuse_after_primary_delivery() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, mut events_rx) = mpsc::channel(16);
        let event_sender =
            crate::transaction::event_sender::TransactionEventSender::new_shared(events_tx);
        let scheduler = LifecycleSchedulerHandle::new_managed(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
        );
        event_sender.require_terminal_ack();
        scheduler.require_dialog_terminal_ack();

        let key = TransactionKey::new(
            "compact-dialog-ack-fence".into(),
            rvoip_sip_core::Method::Bye,
            false,
        );
        let route = TransportRoute::new("127.0.0.1:5096".parse().unwrap())
            .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp);
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(route.clone(), 60_001),
        );
        let (first_tx, mut first_rx) = mpsc::channel(2);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    60_001,
                    key.clone(),
                    CompactNonInviteTimer::K,
                    Duration::ZERO,
                    None,
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    first_tx,
                )
                .await
        );
        assert!(matches!(
            first_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));

        // Receiving from the transaction primary only moves the events into
        // the dialog layer. Until its worker acknowledges the final event, the
        // exact generation remains the admission fence.
        let mut terminal_event = None;
        for index in 0..3 {
            let event = events_rx.recv().await.expect("compact terminal event");
            if index == 2 {
                terminal_event = Some(event);
            }
        }
        let generation = tombstones
            .get(&key)
            .expect("fence survives primary delivery")
            .generation();
        assert_eq!(
            event_sender.take_compact_terminal_generation(
                terminal_event.as_ref().expect("exact terminal event Arc")
            ),
            Some(generation),
            "only the exact shared terminal event may carry this generation"
        );
        let (replacement_tx, _replacement_rx) = mpsc::channel(2);
        assert!(
            !scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    60_002,
                    key.clone(),
                    CompactNonInviteTimer::K,
                    Duration::from_secs(1),
                    None,
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    replacement_tx,
                )
                .await,
            "same-key reuse must remain rejected while dialog delivery stalls"
        );

        assert!(acknowledge_dialog_terminal_generation(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &key,
            generation,
            None,
        ));
        assert!(!tombstones.contains_key(&key));

        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(route, 60_002),
        );
        let (replacement_tx, mut replacement_rx) = mpsc::channel(2);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    60_002,
                    key,
                    CompactNonInviteTimer::K,
                    Duration::from_secs(1),
                    None,
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    replacement_tx,
                )
                .await,
            "key may be reused only after exact dialog acknowledgement"
        );
        assert!(matches!(
            replacement_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn stalled_tu_bounds_compact_retention_and_shutdown_remains_prompt() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, _events_rx) = mpsc::channel(1);
        events_tx
            .send(TransactionEvent::Error {
                transaction_id: None,
                error: "stall primary".into(),
            })
            .await
            .unwrap();
        let event_sender = crate::transaction::event_sender::TransactionEventSender::new(events_tx);
        let scheduler = LifecycleSchedulerHandle::new_managed_with_limits(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
            2,
            4,
        );
        let weak = scheduler.downgrade();

        let mut accepted = 0;
        for index in 0..8_usize {
            let key = TransactionKey::new(
                format!("compact-bounded-{index}"),
                rvoip_sip_core::Method::Bye,
                false,
            );
            routes.insert(
                Arc::new(key.clone()),
                ClientResponseRouteState::active(
                    TransportRoute::new("127.0.0.1:5097".parse().unwrap())
                        .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                    70_000 + index,
                ),
            );
            let (command_tx, _command_rx) = mpsc::channel(2);
            if weak
                .schedule_compact_non_invite(
                    70_000 + index,
                    key,
                    CompactNonInviteTimer::K,
                    Duration::ZERO,
                    None,
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    command_tx,
                )
                .await
            {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 4);
        assert_eq!(tombstones.len(), 4);
        assert_eq!(scheduler.compact_retention_in_use(), 4);

        tokio::time::timeout(Duration::from_millis(200), scheduler.shutdown())
            .await
            .expect("bounded stalled TU must not delay scheduler shutdown");
        assert!(tombstones.is_empty());
        assert_eq!(scheduler.compact_retention_in_use(), 0);
    }

    #[tokio::test]
    async fn compact_timer_k_retains_immutable_result_and_only_weak_live_waiter_bridge() {
        let (scheduler, tombstones, routes, _leases, _events_tx, _events_rx) =
            managed_scheduler_fixture();
        let weak = scheduler.downgrade();
        let key = TransactionKey::new(
            "compact-k-weak-completion".into(),
            rvoip_sip_core::Method::Message,
            false,
        );
        let owner = 40_001;
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(
                TransportRoute::new("127.0.0.1:5097".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                owner,
            ),
        );
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let completion = Arc::new(
            crate::transaction::completion::ClientTransactionCompletion::new(
                TransactionState::Completed,
            ),
        );
        completion.record_response(rvoip_sip_core::Response::new(
            rvoip_sip_core::StatusCode::Ok,
        ));
        let existing_waiter = completion.clone();
        let (command_tx, mut command_rx) = mpsc::channel(2);

        assert!(
            weak.schedule_compact_non_invite(
                owner,
                key.clone(),
                CompactNonInviteTimer::K,
                Duration::from_millis(20),
                None,
                state,
                Some(completion.clone()),
                command_tx,
            )
            .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        drop(completion);

        {
            let tombstone = tombstones.get(&key).expect("Timer K tombstone");
            let retained = tombstone
                .client_completion()
                .expect("immutable client completion");
            assert!(matches!(
                retained.outcome().expect("valid compact response"),
                Some(crate::transaction::ClientTransactionOutcome::FinalResponse(response))
                    if response.status().as_u16() == 200
            ));
            let CompactNonInviteTombstone::Client {
                live_completion, ..
            } = tombstone.value()
            else {
                panic!("expected client Timer K tombstone");
            };
            assert_eq!(
                live_completion.strong_count(),
                1,
                "only the pre-retirement waiter may retain the live cell"
            );
        }

        let waiter = tokio::spawn(async move {
            existing_waiter
                .wait_for_state(TransactionState::Terminated, Duration::from_secs(1))
                .await
        });
        assert!(waiter.await.expect("existing completion waiter"));
        assert!(!tombstones.contains_key(&key));
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn compact_timer_k_expiry_does_not_remove_replacement_route_owner() {
        let (scheduler, tombstones, routes, _leases, _events_tx, _events_rx) =
            managed_scheduler_fixture();
        let weak = scheduler.downgrade();
        let key = TransactionKey::new(
            "compact-k-route-owner".into(),
            rvoip_sip_core::Method::Bye,
            false,
        );
        let original_owner = 50_001;
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(
                TransportRoute::new("127.0.0.1:5098".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                original_owner,
            ),
        );
        let (command_tx, mut command_rx) = mpsc::channel(2);
        assert!(
            weak.schedule_compact_non_invite(
                original_owner,
                key.clone(),
                CompactNonInviteTimer::K,
                Duration::from_millis(20),
                None,
                Arc::new(crate::transaction::AtomicTransactionState::new(
                    TransactionState::Completed,
                )),
                Some(Arc::new(
                    crate::transaction::completion::ClientTransactionCompletion::new(
                        TransactionState::Completed,
                    ),
                )),
                command_tx,
            )
            .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));

        let replacement_owner = 50_002;
        let replacement_route = TransportRoute::new("127.0.0.1:5099".parse().unwrap())
            .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp);
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(replacement_route.clone(), replacement_owner),
        );

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!tombstones.contains_key(&key));
        let retained = routes.get(&key).expect("replacement route must survive");
        match retained.value() {
            ClientResponseRouteState::Active { route, owner } => {
                assert_eq!(route, &replacement_route);
                assert_eq!(*owner, replacement_owner);
            }
            ClientResponseRouteState::Retired(_) => panic!("replacement route was retired"),
        }
        drop(retained);
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn compact_retire_full_self_queue_is_deferred_without_deadlock() {
        let (scheduler, _tombstones, routes, _leases, _events_tx, _events_rx) =
            managed_scheduler_fixture();
        let weak = scheduler.downgrade();
        let key = TransactionKey::new(
            "compact-full-self-queue".into(),
            rvoip_sip_core::Method::Bye,
            false,
        );
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(
                TransportRoute::new("127.0.0.1:5094".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                41,
            ),
        );
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let (command_tx, mut command_rx) = mpsc::channel(1);
        command_tx
            .send(InternalTransactionCommand::Terminate)
            .await
            .unwrap();

        assert!(tokio::time::timeout(
            Duration::from_millis(100),
            weak.schedule_compact_non_invite(
                41,
                key,
                CompactNonInviteTimer::K,
                Duration::from_secs(1),
                None,
                state,
                Some(Arc::new(
                    crate::transaction::completion::ClientTransactionCompletion::new(
                        TransactionState::Completed,
                    ),
                )),
                command_tx,
            ),
        )
        .await
        .expect("full self queue must not block compact acceptance"));
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::Terminate)
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), command_rx.recv())
                .await
                .expect("deferred compact command must be retried"),
            Some(InternalTransactionCommand::CompactRetire)
        ));
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn stalled_primary_event_channel_cannot_block_scheduler_shutdown() {
        let tombstones = Arc::new(DashMap::new());
        let routes = Arc::new(DashMap::new());
        let principals = Arc::new(DashMap::new());
        let leases = Arc::new(DashMap::new());
        let (events_tx, _events_rx) = mpsc::channel(1);
        let event_sender =
            crate::transaction::event_sender::TransactionEventSender::new(events_tx.clone());
        let scheduler = LifecycleSchedulerHandle::new_managed(
            &tombstones,
            &routes,
            &principals,
            &leases,
            &event_sender,
        );
        let key = TransactionKey::new("stalled-events".into(), rvoip_sip_core::Method::Bye, false);
        events_tx
            .send(TransactionEvent::TransactionTerminated {
                transaction_id: key.clone(),
            })
            .await
            .unwrap();
        routes.insert(
            Arc::new(key.clone()),
            ClientResponseRouteState::active(
                TransportRoute::new("127.0.0.1:5095".parse().unwrap())
                    .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp),
                42,
            ),
        );
        let (command_tx, mut command_rx) = mpsc::channel(2);
        assert!(
            scheduler
                .downgrade()
                .schedule_compact_non_invite(
                    42,
                    key,
                    CompactNonInviteTimer::K,
                    Duration::ZERO,
                    None,
                    Arc::new(crate::transaction::AtomicTransactionState::new(
                        TransactionState::Completed,
                    )),
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    command_tx,
                )
                .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        tokio::time::timeout(Duration::from_millis(100), scheduler.shutdown())
            .await
            .expect("stalled event consumer must not block lifecycle shutdown");
    }

    #[tokio::test]
    async fn compact_timer_j_keeps_replay_bytes_and_preserves_newer_auth_lease() {
        let (scheduler, tombstones, _routes, leases, _events_tx, _events_rx) =
            managed_scheduler_fixture();
        let weak = scheduler.downgrade();
        let key = TransactionKey::new("compact-j".into(), rvoip_sip_core::Method::Bye, true);
        let route = TransportRoute::new("127.0.0.1:5091".parse().unwrap())
            .with_transport_type(rvoip_sip_transport::transport::TransportType::Udp);
        let old_lease = crate::transaction::manager::InboundPrincipalLease {
            inserted_at: StdInstant::now(),
            generation: 7,
        };
        leases.insert(key.clone(), old_lease);
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let (command_tx, mut command_rx) = mpsc::channel(4);
        let wire = bytes::Bytes::from_static(b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n");

        assert!(
            weak.schedule_compact_non_invite(
                1,
                key.clone(),
                CompactNonInviteTimer::J,
                Duration::from_millis(20),
                Some((wire.clone(), route.clone())),
                state,
                None,
                command_tx,
            )
            .await
        );
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        let retained = tombstones.get(&key).expect("server tombstone");
        let (retained_wire, retained_route) = retained.server_replay().expect("server replay");
        assert_eq!(retained_wire, &wire);
        assert_eq!(retained_route, &route);
        drop(retained);

        let newer_lease = crate::transaction::manager::InboundPrincipalLease {
            inserted_at: StdInstant::now(),
            generation: 8,
        };
        leases.insert(key.clone(), newer_lease);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(
            *leases.get(&key).expect("newer lease retained").value(),
            newer_lease
        );
        assert!(!tombstones.contains_key(&key));
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn weak_scheduler_reference_does_not_keep_worker_alive() {
        let scheduler = LifecycleSchedulerHandle::new();
        let weak = scheduler.downgrade();
        drop(scheduler);
        tokio::task::yield_now().await;
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let (command_tx, _command_rx) = mpsc::channel(1);
        assert!(
            !weak
                .schedule_compact_non_invite(
                    1,
                    TransactionKey::new(
                        "closed-scheduler".into(),
                        rvoip_sip_core::Method::Bye,
                        false,
                    ),
                    CompactNonInviteTimer::K,
                    Duration::from_secs(1),
                    None,
                    state,
                    Some(Arc::new(
                        crate::transaction::completion::ClientTransactionCompletion::new(
                            TransactionState::Completed,
                        ),
                    )),
                    command_tx,
                )
                .await
        );
    }
}
