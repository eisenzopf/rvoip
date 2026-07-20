//! Manages the lifecycle of SIP transaction timers and dispatches timer events.
//!
//! The [`TimerManager`] is responsible for:
//! - Registering and unregistering transactions that require timer services.
//! - Scheduling one-shot deadlines that send an
//!   [`InternalTransactionCommand::Timer`] event to the associated transaction.
//! - Holding timer settings applicable to its operations.
//!
//! # Timer Management in SIP
//!
//! RFC 3261 requires precise timer management for ensuring reliability in SIP transactions.
//! Both client and server transactions rely on various timers (A-K) to handle:
//!
//! - Message retransmissions over unreliable transports (e.g., UDP)
//! - Transaction timeouts
//! - Waiting periods for absorbing message retransmissions
//!
//! # Implementation Details
//!
//! This `TimerManager` provides a mechanism for scheduling a single notification after a
//! specified duration. For timers that require periodic firing or complex backoff strategies
//! (like RFC 3261 Timer A or E), the transaction itself, upon receiving a timer event,
//! is responsible for performing its action (e.g., retransmission) and then requesting the
//! `TimerManager` to start a new timer with the next appropriate duration.
//!
//! # Usage Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use tokio::sync::mpsc;
//! use rvoip_sip_dialog::transaction::timer::TimerManager;
//! use rvoip_sip_dialog::transaction::timer::TimerType;
//! use rvoip_sip_dialog::transaction::{TransactionKey, InternalTransactionCommand};
//! use rvoip_sip_core::Method;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a timer manager
//! let timer_manager = Arc::new(TimerManager::new(None));
//!
//! // Create a transaction key and command channel
//! let tx_key = TransactionKey::new("z9hG4bK.456".to_string(), Method::Invite, false);
//! let (cmd_tx, mut cmd_rx) = mpsc::channel(10);
//!
//! // Register the transaction with the timer manager
//! timer_manager.register_transaction(tx_key.clone(), cmd_tx).await;
//!
//! // Start Timer A for this transaction (initial INVITE retransmission timer)
//! let timer_handle = timer_manager.start_timer(
//!     tx_key.clone(),
//!     TimerType::A,
//!     Duration::from_millis(500)
//! ).await?;
//!
//! // In your transaction event loop, handle timer events
//! tokio::spawn(async move {
//!     while let Some(cmd) = cmd_rx.recv().await {
//!         match cmd {
//!             InternalTransactionCommand::Timer(timer_name) => {
//!                 println!("Timer fired: {}", timer_name);
//!                 // Handle timer event (e.g., retransmit request, timeout transaction)
//!             },
//!             // Handle other commands...
//!             _ => {}
//!         }
//!     }
//! });
//!
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::Duration;

use crate::transaction::{InternalTransactionCommand, TransactionKey};
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, trace};
// Ensure TimerSettings is correctly imported if it was moved to super::types
use super::types::{TimerSettings, TimerType};
// Timer struct from types.rs is not directly used by TimerManager methods but is related contextually.

const MAX_DUE_TIMERS_PER_BATCH: usize = 1_024;
const FULL_CHANNEL_RETRY_DELAY: Duration = Duration::from_millis(1);

static NEXT_TIMER_MANAGER_ID: AtomicU64 = AtomicU64::new(1);
static RUNTIME_TIMER_SCHEDULERS: OnceLock<
    StdMutex<HashMap<tokio::runtime::Id, Arc<SharedTimerScheduler>>>,
> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TimerDeadlineKey {
    manager_id: u64,
    transaction_id: TransactionKey,
    timer_type: TimerType,
}

struct TimerDeadline {
    key: TimerDeadlineKey,
    generation: u64,
    commands: Vec<InternalTransactionCommand>,
    next_command: usize,
    command_tx: mpsc::WeakSender<InternalTransactionCommand>,
    cancelled: Arc<AtomicBool>,
    completion: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
struct ReverseTimerDeadline {
    due_at: Instant,
    sequence: u64,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct TimerDeadlineQueue {
    by_deadline: BTreeMap<(Instant, u64), TimerDeadline>,
    by_key: HashMap<TimerDeadlineKey, ReverseTimerDeadline>,
    by_transaction: HashMap<(u64, TransactionKey), HashSet<TimerType>>,
    next_sequence: u64,
    next_generation: u64,
}

impl TimerDeadlineQueue {
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
        key: TimerDeadlineKey,
        due_at: Instant,
        commands: Vec<InternalTransactionCommand>,
        command_tx: mpsc::WeakSender<InternalTransactionCommand>,
        completion: Option<oneshot::Sender<()>>,
    ) -> (u64, Arc<AtomicBool>) {
        self.cancel_key(&key);
        let sequence = self.next_sequence(due_at);
        let generation = self.next_generation();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.by_transaction
            .entry((key.manager_id, key.transaction_id.clone()))
            .or_default()
            .insert(key.timer_type);
        self.by_key.insert(
            key.clone(),
            ReverseTimerDeadline {
                due_at,
                sequence,
                generation,
                cancelled: cancelled.clone(),
            },
        );
        self.by_deadline.insert(
            (due_at, sequence),
            TimerDeadline {
                key,
                generation,
                commands,
                next_command: 0,
                command_tx,
                cancelled: cancelled.clone(),
                completion,
            },
        );
        (generation, cancelled)
    }

    fn cancel_key(&mut self, key: &TimerDeadlineKey) -> bool {
        let Some(reverse) = self.by_key.remove(key) else {
            return false;
        };
        reverse.cancelled.store(true, Ordering::Release);
        self.by_deadline.remove(&(reverse.due_at, reverse.sequence));
        self.remove_transaction_timer(key);
        true
    }

    fn cancel_generation(&mut self, key: &TimerDeadlineKey, generation: u64) -> bool {
        if !self
            .by_key
            .get(key)
            .is_some_and(|reverse| reverse.generation == generation)
        {
            return false;
        }
        self.cancel_key(key)
    }

    fn cancel_transaction(&mut self, manager_id: u64, transaction_id: &TransactionKey) -> usize {
        let timer_types = self
            .by_transaction
            .get(&(manager_id, transaction_id.clone()))
            .cloned()
            .unwrap_or_default();
        let mut cancelled = 0;
        for timer_type in timer_types {
            let key = TimerDeadlineKey {
                manager_id,
                transaction_id: transaction_id.clone(),
                timer_type,
            };
            cancelled += usize::from(self.cancel_key(&key));
        }
        cancelled
    }

    fn cancel_manager(&mut self, manager_id: u64) -> usize {
        let transactions: Vec<_> = self
            .by_transaction
            .keys()
            .filter(|(candidate, _)| *candidate == manager_id)
            .map(|(_, transaction_id)| transaction_id.clone())
            .collect();
        transactions
            .iter()
            .map(|transaction_id| self.cancel_transaction(manager_id, transaction_id))
            .sum()
    }

    fn remove_transaction_timer(&mut self, key: &TimerDeadlineKey) {
        let transaction_key = (key.manager_id, key.transaction_id.clone());
        let mut empty = false;
        if let Some(timer_types) = self.by_transaction.get_mut(&transaction_key) {
            timer_types.remove(&key.timer_type);
            empty = timer_types.is_empty();
        }
        if empty {
            self.by_transaction.remove(&transaction_key);
        }
    }

    fn finish_generation(&mut self, key: &TimerDeadlineKey, generation: u64) {
        if self
            .by_key
            .get(key)
            .is_some_and(|reverse| reverse.generation == generation)
        {
            self.by_key.remove(key);
            self.remove_transaction_timer(key);
        }
    }

    fn retry(&mut self, deadline: TimerDeadline, due_at: Instant) -> bool {
        let Some(reverse) = self.by_key.get(&deadline.key) else {
            return false;
        };
        if reverse.generation != deadline.generation || reverse.cancelled.load(Ordering::Acquire) {
            return false;
        }
        let sequence = self.next_sequence(due_at);
        if let Some(reverse) = self.by_key.get_mut(&deadline.key) {
            if reverse.generation != deadline.generation {
                return false;
            }
            reverse.due_at = due_at;
            reverse.sequence = sequence;
        } else {
            return false;
        }
        self.by_deadline.insert((due_at, sequence), deadline);
        true
    }

    fn take_due(&mut self, now: Instant, limit: usize) -> Vec<TimerDeadline> {
        let mut due = Vec::with_capacity(limit.min(self.by_deadline.len()));
        while due.len() < limit {
            let Some((&(due_at, sequence), _)) = self.by_deadline.first_key_value() else {
                break;
            };
            if due_at > now {
                break;
            }
            if let Some(deadline) = self.by_deadline.remove(&(due_at, sequence)) {
                due.push(deadline);
            }
        }
        due
    }

    fn next_due_at(&self) -> Option<Instant> {
        self.by_deadline.first_key_value().map(|(key, _)| key.0)
    }

    fn len(&self) -> usize {
        self.by_key.len()
    }
}

struct SharedTimerScheduler {
    queue: StdMutex<TimerDeadlineQueue>,
    notify: Arc<Notify>,
    worker_running: AtomicBool,
}

impl std::fmt::Debug for SharedTimerScheduler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedTimerScheduler")
            .field("active_timers", &self.active_timer_count())
            .field(
                "worker_running",
                &self.worker_running.load(Ordering::Acquire),
            )
            .finish()
    }
}

impl SharedTimerScheduler {
    fn new() -> Self {
        Self {
            queue: StdMutex::new(TimerDeadlineQueue::default()),
            notify: Arc::new(Notify::new()),
            worker_running: AtomicBool::new(false),
        }
    }

    fn ensure_worker(self: &Arc<Self>) {
        if self
            .worker_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let scheduler = Arc::downgrade(self);
        tokio::spawn(async move {
            let _guard = TimerWorkerGuard {
                scheduler: scheduler.clone(),
            };
            run_timer_scheduler(scheduler).await;
        });
    }

    fn schedule(
        self: &Arc<Self>,
        key: TimerDeadlineKey,
        due_at: Instant,
        commands: Vec<InternalTransactionCommand>,
        command_tx: mpsc::WeakSender<InternalTransactionCommand>,
        completion: Option<oneshot::Sender<()>>,
    ) -> ManagedTimerHandle {
        let (generation, cancelled) = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone(), due_at, commands, command_tx, completion);
        self.notify.notify_one();
        ManagedTimerHandle {
            scheduler: Arc::downgrade(self),
            key,
            generation,
            cancelled,
            cancel_on_drop: true,
        }
    }

    fn cancel_generation(&self, key: &TimerDeadlineKey, generation: u64) -> bool {
        let cancelled = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_generation(key, generation);
        if cancelled {
            self.notify.notify_one();
        }
        cancelled
    }

    fn cancel_transaction(&self, manager_id: u64, transaction_id: &TransactionKey) -> usize {
        let cancelled = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_transaction(manager_id, transaction_id);
        if cancelled > 0 {
            self.notify.notify_one();
        }
        cancelled
    }

    fn cancel_manager(&self, manager_id: u64) -> usize {
        let cancelled = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_manager(manager_id);
        if cancelled > 0 {
            self.notify.notify_one();
        }
        cancelled
    }

    fn active_timer_count(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl Drop for SharedTimerScheduler {
    fn drop(&mut self) {
        self.notify.notify_waiters();
    }
}

struct TimerWorkerGuard {
    scheduler: Weak<SharedTimerScheduler>,
}

impl Drop for TimerWorkerGuard {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.worker_running.store(false, Ordering::Release);
            scheduler.notify.notify_waiters();
        }
    }
}

/// Cancellation token used by production transaction paths. Unlike the
/// compatibility `JoinHandle`, this handle allocates no Tokio task.
pub(crate) struct ManagedTimerHandle {
    scheduler: Weak<SharedTimerScheduler>,
    key: TimerDeadlineKey,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

impl std::fmt::Debug for ManagedTimerHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedTimerHandle")
            .field("timer_type", &self.key.timer_type)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl ManagedTimerHandle {
    pub(crate) fn abort(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.cancel_generation(&self.key, self.generation);
        }
    }

    fn disarm(mut self) {
        self.cancel_on_drop = false;
    }
}

impl Drop for ManagedTimerHandle {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.abort();
        }
    }
}

struct CancelTimerOnDrop(Option<ManagedTimerHandle>);

impl CancelTimerOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelTimerOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

fn shared_timer_scheduler() -> Arc<SharedTimerScheduler> {
    let runtime_id = tokio::runtime::Handle::current().id();
    let schedulers = RUNTIME_TIMER_SCHEDULERS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut schedulers = schedulers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    schedulers.retain(|_, scheduler| scheduler.worker_running.load(Ordering::Acquire));
    let scheduler = schedulers
        .entry(runtime_id)
        .or_insert_with(|| Arc::new(SharedTimerScheduler::new()))
        .clone();
    scheduler.ensure_worker();
    scheduler
}

fn cancel_registered_timers(manager_id: u64, transaction_id: &TransactionKey) {
    let Some(schedulers) = RUNTIME_TIMER_SCHEDULERS.get() else {
        return;
    };
    let schedulers = schedulers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for scheduler in schedulers.values() {
        scheduler.cancel_transaction(manager_id, transaction_id);
    }
}

fn cancel_manager_timers(manager_id: u64) {
    let Some(schedulers) = RUNTIME_TIMER_SCHEDULERS.get() else {
        return;
    };
    let schedulers = schedulers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for scheduler in schedulers.values() {
        scheduler.cancel_manager(manager_id);
    }
}

async fn deliver_due_timer(scheduler: &Arc<SharedTimerScheduler>, mut deadline: TimerDeadline) {
    if deadline.cancelled.load(Ordering::Acquire) {
        scheduler
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish_generation(&deadline.key, deadline.generation);
        return;
    }

    let Some(command_tx) = deadline.command_tx.upgrade() else {
        scheduler
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish_generation(&deadline.key, deadline.generation);
        return;
    };
    let mut blocked = false;
    while deadline.next_command < deadline.commands.len() {
        if deadline.cancelled.load(Ordering::Acquire) {
            break;
        }
        match command_tx.try_send(deadline.commands[deadline.next_command].clone()) {
            Ok(()) => deadline.next_command += 1,
            Err(mpsc::error::TrySendError::Full(_)) => {
                blocked = true;
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }

    if blocked && !deadline.cancelled.load(Ordering::Acquire) {
        let retry_at = Instant::now() + FULL_CHANNEL_RETRY_DELAY;
        let retried = scheduler
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retry(deadline, retry_at);
        if retried {
            scheduler.notify.notify_one();
        }
        return;
    }

    scheduler
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish_generation(&deadline.key, deadline.generation);
    if let Some(completion) = deadline.completion.take() {
        let _ = completion.send(());
    }
}

async fn run_timer_scheduler(scheduler: Weak<SharedTimerScheduler>) {
    loop {
        let Some(scheduler_arc) = scheduler.upgrade() else {
            break;
        };
        let now = Instant::now();
        let due = scheduler_arc
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take_due(now, MAX_DUE_TIMERS_PER_BATCH);
        let processed = due.len();
        for deadline in due {
            deliver_due_timer(&scheduler_arc, deadline).await;
        }

        let notify = scheduler_arc.notify.clone();
        let mut notified = Box::pin(notify.notified_owned());
        notified.as_mut().enable();
        let next_due_at = scheduler_arc
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_due_at();
        let more_due = processed == MAX_DUE_TIMERS_PER_BATCH
            && next_due_at.is_some_and(|due_at| due_at <= Instant::now());
        drop(scheduler_arc);

        if more_due {
            tokio::task::yield_now().await;
            continue;
        }
        match next_due_at {
            Some(due_at) => {
                tokio::select! {
                    _ = &mut notified => {}
                    _ = tokio::time::sleep_until(due_at) => {}
                }
            }
            None => notified.await,
        }
    }
}

/// Manages active timers for SIP transactions.
///
/// The `TimerManager` is a central component of the SIP transaction layer that handles:
///
/// 1. **Timer Registration**: Associates transactions with their command channels
/// 2. **Timer Scheduling**: Stores and manages one-shot deadlines
/// 3. **Event Delivery**: Notifies transactions when their timers expire
///
/// When a timer fires, the `TimerManager` sends an `InternalTransactionCommand::Timer` message
/// to the `mpsc::Sender<InternalTransactionCommand>` that was registered for that transaction.
/// It does not directly manage `Timer` struct instances. One due-driven worker
/// services all deadlines in a Tokio runtime, so core transaction paths do not
/// allocate one task per timer.
///
/// # RFC 3261 Compliance
///
/// This implementation satisfies the timing requirements of RFC 3261 Section 17, which
/// defines the behavior of client and server transaction state machines. The `TimerManager`
/// provides the underlying mechanism for:
///
/// - Retransmission timers (A, E, G)
/// - Transaction timeout timers (B, F, H)
/// - Wait timers for absorbing retransmissions (D, I, J, K)
#[derive(Debug)]
pub struct TimerManager {
    /// Namespace for exact timer-key deduplication inside the runtime-shared
    /// scheduler. Two independent managers may legitimately use the same SIP
    /// transaction key without replacing each other's deadlines.
    manager_id: u64,
    /// Stores sender channels for `InternalTransactionCommand`s, keyed by `TransactionKey`.
    /// Used to notify a specific transaction when one of its timers fires.
    transaction_channels:
        Arc<Mutex<HashMap<TransactionKey, mpsc::Sender<InternalTransactionCommand>>>>,
    /// Configuration settings for timers, such as default durations (T1, T2 etc.).
    /// While `TimerManager` itself mostly deals with given durations, these settings might inform
    /// those durations if not provided directly to `start_timer` or by a `TimerFactory`.
    settings: TimerSettings,
}

impl TimerManager {
    /// Creates a new `TimerManager`.
    ///
    /// # Arguments
    /// * `settings` - Optional [`TimerSettings`]. If `None`, default settings are used.
    ///   The default settings follow RFC 3261 recommendations (T1=500ms, etc.).
    pub fn new(settings: Option<TimerSettings>) -> Self {
        Self {
            manager_id: NEXT_TIMER_MANAGER_ID.fetch_add(1, Ordering::Relaxed),
            transaction_channels: Arc::new(Mutex::new(HashMap::new())),
            settings: settings.unwrap_or_default(),
        }
    }

    /// Registers a transaction with the `TimerManager`.
    ///
    /// This allows the `TimerManager` to send timer-fired events to the transaction via the provided `command_tx` channel.
    /// Typically called when a new transaction is created and needs timer supervision.
    ///
    /// If a transaction with the same ID is already registered, this method will replace the existing
    /// command channel with the new one. This is a normal operation in some cases, such as when a transaction
    /// is being processed through multiple functions or when timers are reset.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction to register.
    /// * `command_tx` - The `mpsc::Sender` channel for sending [`InternalTransactionCommand`]s to the transaction.
    ///
    /// # SIP Transaction Lifecycle
    ///
    /// In the SIP transaction model, registration occurs when a transaction is created,
    /// either by a client initiating a request or a server receiving one. The registration
    /// enables timer management for the transaction's entire lifecycle.
    pub async fn register_transaction(
        &self,
        transaction_id: TransactionKey,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let mut channels = self.transaction_channels.lock().await;
        if channels
            .insert(transaction_id.clone(), command_tx)
            .is_some()
        {
            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Transaction channel replaced for already registered transaction.");
        }
        trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Transaction registered with TimerManager.");
    }

    /// Unregisters a transaction from the `TimerManager`.
    ///
    /// After unregistering, the transaction will no longer receive timer events,
    /// and its queued deadlines are cancelled immediately.
    /// Typically called when a transaction terminates.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction to unregister.
    ///
    /// # SIP Transaction Termination
    ///
    /// In SIP, transactions eventually reach a terminated state when:
    /// - A final response is received (client transactions)
    /// - An ACK is received or timeout occurs (server INVITE transactions)
    /// - Cleanup timers expire (all transaction types)
    ///
    /// This method should be called when a transaction reaches its terminated state
    /// to prevent memory leaks and ensure proper cleanup.
    pub async fn unregister_transaction(&self, transaction_id: &TransactionKey) {
        let mut channels = self.transaction_channels.lock().await;
        if channels.remove(transaction_id).is_some() {
            trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Transaction unregistered from TimerManager.");
        } else {
            trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Attempted to unregister a non-existent transaction.");
        }
        drop(channels);
        cancel_registered_timers(self.manager_id, transaction_id);
    }

    /// Starts a one-shot timer for a specific transaction.
    ///
    /// A deadline is inserted into the runtime-shared queue. Upon expiry, the
    /// shared worker sends an [`InternalTransactionCommand::Timer`] containing
    /// the `timer_type` (as a string) to the registered transaction channel.
    ///
    /// If the transaction is unregistered before the timer fires, or if its command channel
    /// is closed, the event delivery will fail silently or log an error, respectively.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction this timer belongs to.
    /// * `timer_type` - The [`TimerType`] of this timer, used for generating the event payload.
    /// * `duration` - The [`Duration`] for which the timer should sleep before firing.
    ///
    /// # Returns
    /// `Ok(JoinHandle<()>)` for API compatibility. This lightweight proxy can
    /// be awaited for deadline completion or aborted to cancel the exact queued
    /// generation; it owns no sleep or timer-wheel entry. Core transaction paths
    /// use [`ManagedTimerHandle`] and avoid even this proxy task.
    /// Returns `crate::error::Error` if the underlying transaction channel is not found *immediately*
    /// (the current implementation checks delivery when the deadline becomes due).
    ///
    /// # RFC 3261 Timer Types
    ///
    /// RFC 3261 defines several timer types that will commonly be used with this method:
    ///
    /// - For INVITE client transactions: Timers A, B, and D
    /// - For non-INVITE client transactions: Timers E, F, and K
    /// - For INVITE server transactions: Timers G, H, and I
    /// - For non-INVITE server transactions: Timer J
    pub async fn start_timer(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
    ) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
        let timer_name = timer_type.to_string();
        self.start_compatibility_timer(
            transaction_id,
            timer_type,
            duration,
            vec![InternalTransactionCommand::Timer(timer_name)],
        )
        .await
    }

    /// Schedule a timer for core transaction paths without allocating a Tokio
    /// proxy task. Cancellation remains exact through `ManagedTimerHandle`.
    pub(crate) async fn start_timer_managed(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<ManagedTimerHandle, crate::transaction::error::Error> {
        let timer_name = timer_type.to_string();
        Ok(self.schedule_timer(
            transaction_id,
            timer_type,
            duration,
            vec![InternalTransactionCommand::Timer(timer_name)],
            command_tx.downgrade(),
            None,
        ))
    }

    pub(crate) async fn start_timer_managed_with_transition(
        &self,
        transaction_id: TransactionKey,
        timer_name: String,
        timer_type: TimerType,
        duration: Duration,
        target_state: crate::transaction::TransactionState,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<ManagedTimerHandle, crate::transaction::error::Error> {
        Ok(self.schedule_timer(
            transaction_id,
            timer_type,
            duration,
            vec![
                InternalTransactionCommand::Timer(timer_name),
                InternalTransactionCommand::TransitionTo(target_state),
            ],
            command_tx.downgrade(),
            None,
        ))
    }

    pub(crate) async fn start_timer_detached(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
    ) -> Result<(), crate::transaction::error::Error> {
        let command_tx = self.compatibility_sender(&transaction_id).await?;
        let handle = self.schedule_timer(
            transaction_id,
            timer_type,
            duration,
            vec![InternalTransactionCommand::Timer(timer_type.to_string())],
            command_tx.downgrade(),
            None,
        );
        handle.disarm();
        Ok(())
    }

    /// Cancel every deadline and release every transaction channel in this
    /// manager namespace. Manager shutdown invokes this only after transaction
    /// runners have been quiesced.
    pub(crate) async fn shutdown(&self) {
        self.transaction_channels.lock().await.clear();
        cancel_manager_timers(self.manager_id);
    }

    pub(crate) async fn start_timer_with_transition(
        &self,
        transaction_id: TransactionKey,
        timer_name: String,
        timer_type: TimerType,
        duration: Duration,
        target_state: crate::transaction::TransactionState,
    ) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
        self.start_compatibility_timer(
            transaction_id,
            timer_type,
            duration,
            vec![
                InternalTransactionCommand::Timer(timer_name),
                InternalTransactionCommand::TransitionTo(target_state),
            ],
        )
        .await
    }

    async fn start_compatibility_timer(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
        commands: Vec<InternalTransactionCommand>,
    ) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
        let (completion_tx, completion_rx) = oneshot::channel();
        let command_tx = self.compatibility_sender(&transaction_id).await?;
        let managed = self.schedule_timer(
            transaction_id,
            timer_type,
            duration,
            commands,
            command_tx.downgrade(),
            Some(completion_tx),
        );
        let cancellation = CancelTimerOnDrop(Some(managed));
        Ok(tokio::spawn(async move {
            let mut cancellation = cancellation;
            let _ = completion_rx.await;
            cancellation.disarm();
        }))
    }

    fn schedule_timer(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
        commands: Vec<InternalTransactionCommand>,
        command_tx: mpsc::WeakSender<InternalTransactionCommand>,
        completion: Option<oneshot::Sender<()>>,
    ) -> ManagedTimerHandle {
        let scheduler = shared_timer_scheduler();
        let key = TimerDeadlineKey {
            manager_id: self.manager_id,
            transaction_id,
            timer_type,
        };
        let due_at = Instant::now() + duration;
        scheduler.schedule(key, due_at, commands, command_tx, completion)
    }

    async fn compatibility_sender(
        &self,
        transaction_id: &TransactionKey,
    ) -> Result<mpsc::Sender<InternalTransactionCommand>, crate::transaction::error::Error> {
        self.transaction_channels
            .lock()
            .await
            .get(transaction_id)
            .cloned()
            .ok_or_else(|| {
                crate::transaction::error::Error::transaction_not_found(
                    transaction_id.clone(),
                    "timer transaction is not registered",
                )
            })
    }

    /// Returns a reference to the [`TimerSettings`] used by this manager.
    pub fn settings(&self) -> &TimerSettings {
        &self.settings
    }
}

/// Provides a default `TimerManager` with default [`TimerSettings`].
impl Default for TimerManager {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::TransactionKey;
    use rvoip_sip_core::Method;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    // Helper to create a dummy TransactionKey for tests
    fn dummy_tm_tx_key(name: &str) -> TransactionKey {
        TransactionKey::new(format!("branch-manager-{}", name), Method::Options, false)
    }

    async fn settle_scheduler() {
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn timer_manager_new_and_default() {
        let settings = TimerSettings {
            t1: Duration::from_millis(100),
            ..Default::default()
        };
        let manager = TimerManager::new(Some(settings.clone()));
        assert_eq!(manager.settings(), &settings);
        assert!(manager.transaction_channels.try_lock().unwrap().is_empty());

        let default_manager = TimerManager::default();
        assert_eq!(*default_manager.settings(), TimerSettings::default());
    }

    #[tokio::test]
    async fn timer_manager_register_unregister_transaction() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("reg_unreg");
        let (cmd_tx, _) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;
        assert!(manager
            .transaction_channels
            .lock()
            .await
            .contains_key(&tx_key));

        manager.unregister_transaction(&tx_key).await;
        assert!(!manager
            .transaction_channels
            .lock()
            .await
            .contains_key(&tx_key));

        // Test unregistering a non-existent key (should not panic)
        manager
            .unregister_transaction(&dummy_tm_tx_key("non_existent"))
            .await;
    }

    #[tokio::test]
    async fn timer_manager_start_timer_sends_event() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("send_event");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(10); // Increased buffer for safety

        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let timer_duration = Duration::from_millis(50);
        let timer_type = TimerType::Custom;

        let handle = manager
            .start_timer(tx_key.clone(), timer_type, timer_duration)
            .await
            .unwrap();

        // Wait for the timer event
        match timeout(timer_duration + Duration::from_millis(50), cmd_rx.recv()).await {
            Ok(Some(InternalTransactionCommand::Timer(payload))) => {
                assert_eq!(payload, timer_type.to_string());
            }
            Ok(Some(other_cmd)) => panic!("Received unexpected command: {:?}", other_cmd),
            Ok(None) => panic!("Command channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for timer event"),
        }

        handle.await.expect("Timer task panicked");
    }

    #[tokio::test]
    async fn timer_manager_timer_fires_for_unregistered_transaction() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("unregistered_fire");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let timer_duration = Duration::from_millis(20);
        let handle = manager
            .start_timer(tx_key.clone(), TimerType::A, timer_duration)
            .await
            .unwrap();

        // Unregister immediately after starting
        manager.unregister_transaction(&tx_key).await;

        // The timer task will run, but it shouldn't find the channel to send the event.
        // We check that no event is received.
        match timeout(timer_duration + Duration::from_millis(50), cmd_rx.recv()).await {
            Ok(Some(_)) => {
                panic!("Should not have received a timer event for unregistered transaction")
            }
            Ok(None) => { /* Channel closed or empty, expected */ }
            Err(_) => {
                /* Timeout, also expected as no event should arrive */
                trace!("Timeout as expected for unregistered timer test.")
            }
        }
        handle
            .await
            .expect("Timer task for unregistered tx panicked");
    }

    #[tokio::test]
    async fn timer_manager_timer_receiver_dropped() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("rx_dropped");
        let (cmd_tx, cmd_rx) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;
        drop(cmd_rx); // Drop the receiver

        let timer_duration = Duration::from_millis(20);
        // The start_timer itself should succeed.
        let handle = manager
            .start_timer(tx_key.clone(), TimerType::B, timer_duration)
            .await
            .unwrap();

        // The spawned task will attempt to send, but it will fail because the receiver is dropped.
        // This should be handled gracefully within the task (e.g., logged error).
        // We just await the handle to ensure the task completes without panicking.
        match timeout(timer_duration + Duration::from_millis(50), handle).await {
            Ok(Ok(())) => { /* Task completed */ }
            Ok(Err(e)) => panic!("Timer task join error: {}", e),
            Err(_) => panic!("Timeout waiting for timer task to complete after receiver dropped"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn managed_deadline_uses_tokio_clock() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("paused_clock");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(2);
        let _timer = manager
            .start_timer_managed(
                tx_key,
                TimerType::A,
                Duration::from_secs(10),
                cmd_tx.clone(),
            )
            .await
            .unwrap();

        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(matches!(
            cmd_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            cmd_rx.recv().await,
            Some(InternalTransactionCommand::Timer(name)) if name == "A"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn exact_timer_key_replaces_older_generation() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("deduplicate");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(2);
        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let first = manager
            .start_timer(tx_key.clone(), TimerType::B, Duration::from_secs(10))
            .await
            .unwrap();
        let second = manager
            .start_timer(tx_key, TimerType::B, Duration::from_secs(20))
            .await
            .unwrap();
        settle_scheduler().await;
        assert!(
            first.is_finished(),
            "replaced compatibility handle must finish"
        );
        assert_eq!(shared_timer_scheduler().active_timer_count(), 1);

        tokio::time::advance(Duration::from_secs(10)).await;
        settle_scheduler().await;
        assert!(matches!(
            cmd_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        tokio::time::advance(Duration::from_secs(10)).await;
        settle_scheduler().await;
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(InternalTransactionCommand::Timer(name)) if name == "B"
        ));
        assert!(matches!(
            cmd_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        first.await.expect("replaced proxy task panicked");
        second.await.expect("active proxy task panicked");
    }

    #[tokio::test(start_paused = true)]
    async fn separate_managers_do_not_deduplicate_each_other() {
        let first_manager = TimerManager::new(None);
        let second_manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("manager_namespace");
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        first_manager
            .register_transaction(tx_key.clone(), first_tx.clone())
            .await;
        second_manager
            .register_transaction(tx_key.clone(), second_tx.clone())
            .await;

        let _first = first_manager
            .start_timer_managed(
                tx_key.clone(),
                TimerType::F,
                Duration::from_secs(1),
                first_tx,
            )
            .await
            .unwrap();
        let _second = second_manager
            .start_timer_managed(tx_key, TimerType::F, Duration::from_secs(1), second_tx)
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        settle_scheduler().await;

        assert!(matches!(
            first_rx.try_recv(),
            Ok(InternalTransactionCommand::Timer(name)) if name == "F"
        ));
        assert!(matches!(
            second_rx.try_recv(),
            Ok(InternalTransactionCommand::Timer(name)) if name == "F"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn aborting_compatibility_handle_cancels_deadline() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("abort");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let handle = manager
            .start_timer(tx_key, TimerType::H, Duration::from_secs(5))
            .await
            .unwrap();
        handle.abort();
        let join_error = handle
            .await
            .expect_err("aborted proxy unexpectedly completed");
        assert!(join_error.is_cancelled());
        settle_scheduler().await;
        assert_eq!(shared_timer_scheduler().active_timer_count(), 0);

        tokio::time::advance(Duration::from_secs(5)).await;
        settle_scheduler().await;
        assert!(matches!(
            cmd_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn full_transaction_channel_does_not_block_other_deadlines() {
        let manager = TimerManager::new(None);
        let blocked_key = dummy_tm_tx_key("blocked");
        let ready_key = dummy_tm_tx_key("ready");
        let (blocked_tx, mut blocked_rx) = mpsc::channel(1);
        let (ready_tx, mut ready_rx) = mpsc::channel(1);
        blocked_tx
            .send(InternalTransactionCommand::Terminate)
            .await
            .unwrap();
        manager
            .register_transaction(blocked_key.clone(), blocked_tx.clone())
            .await;
        manager
            .register_transaction(ready_key.clone(), ready_tx.clone())
            .await;

        let _blocked = manager
            .start_timer_managed(
                blocked_key.clone(),
                TimerType::A,
                Duration::from_secs(1),
                blocked_tx,
            )
            .await
            .unwrap();
        let _ready = manager
            .start_timer_managed(ready_key, TimerType::B, Duration::from_secs(1), ready_tx)
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        settle_scheduler().await;

        assert!(matches!(
            ready_rx.try_recv(),
            Ok(InternalTransactionCommand::Timer(name)) if name == "B"
        ));
        assert!(matches!(
            blocked_rx.try_recv(),
            Ok(InternalTransactionCommand::Terminate)
        ));

        manager.unregister_transaction(&blocked_key).await;
    }

    #[tokio::test(start_paused = true)]
    async fn transition_retry_resumes_without_repeating_timer_event() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("transition_progress");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let _timer = manager
            .start_timer_managed_with_transition(
                tx_key,
                "D".to_string(),
                TimerType::D,
                Duration::from_secs(1),
                crate::transaction::TransactionState::Terminated,
                cmd_tx.clone(),
            )
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            cmd_rx.recv().await,
            Some(InternalTransactionCommand::Timer(name)) if name == "D"
        ));

        tokio::time::advance(FULL_CHANNEL_RETRY_DELAY).await;
        assert!(matches!(
            cmd_rx.recv().await,
            Some(InternalTransactionCommand::TransitionTo(
                crate::transaction::TransactionState::Terminated
            ))
        ));
        assert!(matches!(
            cmd_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn due_work_is_bounded_per_scheduler_batch() {
        let mut queue = TimerDeadlineQueue::default();
        let due_at = Instant::now();
        let (command_tx, _command_rx) = mpsc::channel(1);
        for index in 0..=MAX_DUE_TIMERS_PER_BATCH {
            let transaction_id = dummy_tm_tx_key(&format!("batch-{index}"));
            queue.insert(
                TimerDeadlineKey {
                    manager_id: 1,
                    transaction_id,
                    timer_type: TimerType::A,
                },
                due_at,
                vec![InternalTransactionCommand::Timer("A".to_string())],
                command_tx.downgrade(),
                None,
            );
        }

        assert_eq!(
            queue.take_due(due_at, MAX_DUE_TIMERS_PER_BATCH).len(),
            MAX_DUE_TIMERS_PER_BATCH
        );
        assert_eq!(queue.by_deadline.len(), 1);
    }

    #[test]
    fn timer_manager_settings_accessor() {
        let custom_settings = TimerSettings {
            t1: Duration::from_secs(10),
            ..Default::default()
        };
        let manager = TimerManager::new(Some(custom_settings.clone()));
        assert_eq!(manager.settings(), &custom_settings);
    }
}
