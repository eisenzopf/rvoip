//! Ordered primary transaction delivery and bounded optional observation.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::transaction::{TransactionEvent, TransactionKey, TransactionState};

const TERMINAL_PUBLICATION_AVAILABLE: u8 = 0;
const TERMINAL_PUBLICATION_IN_FLIGHT: u8 = 1;
const TERMINAL_PUBLICATION_DELIVERED: u8 = 2;
const TERMINAL_PUBLICATION_FAILED_CLOSED: u8 = 3;

/// Exact, cancellation-safe ownership of the one authoritative terminal
/// observation for a transaction. A task that is aborted while blocked on
/// the primary channel drops its lease and makes publication available to the
/// joined manager fallback instead of leaving a sticky boolean claim.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct TerminalEventPublication {
    state: AtomicU8,
    prefix_previous_state: AtomicU8,
    prefix_delivered: AtomicBool,
}

impl TerminalEventPublication {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn try_claim(self: &Arc<Self>) -> Option<TerminalEventPublicationClaim> {
        self.state
            .compare_exchange(
                TERMINAL_PUBLICATION_AVAILABLE,
                TERMINAL_PUBLICATION_IN_FLIGHT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| TerminalEventPublicationClaim {
                publication: Arc::clone(self),
                settled: false,
            })
    }

    pub(crate) fn is_delivered(&self) -> bool {
        self.state.load(Ordering::Acquire) == TERMINAL_PUBLICATION_DELIVERED
    }

    pub(crate) fn record_prefix(&self, previous_state: TransactionState) {
        let encoded = encode_transaction_state(previous_state);
        let _ = self.prefix_previous_state.compare_exchange(
            0,
            encoded,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn pending_prefix(&self) -> Option<TransactionState> {
        if self.prefix_delivered.load(Ordering::Acquire) {
            return None;
        }
        decode_transaction_state(self.prefix_previous_state.load(Ordering::Acquire))
    }
}

#[derive(Debug)]
pub(crate) struct TerminalEventPublicationClaim {
    publication: Arc<TerminalEventPublication>,
    settled: bool,
}

impl TerminalEventPublicationClaim {
    pub(crate) fn publication(&self) -> &TerminalEventPublication {
        &self.publication
    }

    fn mark_prefix_delivered(&self) {
        self.publication
            .prefix_delivered
            .store(true, Ordering::Release);
    }

    pub(crate) fn mark_delivered(mut self) {
        self.publication
            .state
            .store(TERMINAL_PUBLICATION_DELIVERED, Ordering::Release);
        self.settled = true;
    }

    pub(crate) fn mark_failed_closed(mut self) {
        self.publication
            .state
            .store(TERMINAL_PUBLICATION_FAILED_CLOSED, Ordering::Release);
        self.settled = true;
    }
}

fn encode_transaction_state(state: TransactionState) -> u8 {
    match state {
        TransactionState::Initial => 1,
        TransactionState::Calling => 2,
        TransactionState::Trying => 3,
        TransactionState::Proceeding => 4,
        TransactionState::Completed => 5,
        TransactionState::Confirmed => 6,
        TransactionState::Terminated => 7,
    }
}

fn decode_transaction_state(encoded: u8) -> Option<TransactionState> {
    match encoded {
        1 => Some(TransactionState::Initial),
        2 => Some(TransactionState::Calling),
        3 => Some(TransactionState::Trying),
        4 => Some(TransactionState::Proceeding),
        5 => Some(TransactionState::Completed),
        6 => Some(TransactionState::Confirmed),
        7 => Some(TransactionState::Terminated),
        _ => None,
    }
}

impl Drop for TerminalEventPublicationClaim {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.publication.state.compare_exchange(
                TERMINAL_PUBLICATION_IN_FLIGHT,
                TERMINAL_PUBLICATION_AVAILABLE,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

fn into_owned_event(event: Arc<TransactionEvent>) -> TransactionEvent {
    Arc::try_unwrap(event).unwrap_or_else(|event| (*event).clone())
}

#[derive(Clone)]
struct TerminalEventSidecar {
    event: Weak<TransactionEvent>,
    compact_generation: Option<u64>,
    _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

struct OwnedTerminalEventSidecar {
    id: u64,
    compact_generation: Option<u64>,
    _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

/// Exact ownership fence transferred from a shared terminal event to the
/// integrated dialog consumer. Dropping this only after route/index cleanup
/// makes same-wire-key reuse impossible while an older terminal event is in
/// flight through the dialog shard queue.
pub(crate) struct TerminalEventFence {
    compact_generation: Option<u64>,
    _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

impl TerminalEventFence {
    pub(crate) fn compact_generation(&self) -> Option<u64> {
        self.compact_generation
    }
}

/// Cancellation guard for the interval between registering an exact compact
/// sidecar and successfully enqueueing its shared event. Dropping a blocked
/// send future, shutting down its dispatcher, or observing a closed receiver
/// removes only this exact Arc/generation registration.
struct TerminalEventRegistration {
    sidecars: Arc<DashMap<usize, TerminalEventSidecar>>,
    identity: usize,
    event: Weak<TransactionEvent>,
    enqueued: bool,
}

impl TerminalEventRegistration {
    fn mark_enqueued(&mut self) {
        self.enqueued = true;
    }
}

impl Drop for TerminalEventRegistration {
    fn drop(&mut self) {
        if self.enqueued {
            return;
        }
        self.sidecars.remove_if(&self.identity, |_, current| {
            current.event.ptr_eq(&self.event)
        });
    }
}

struct OwnedTerminalEventRegistration {
    sidecars: Arc<DashMap<TransactionKey, VecDeque<OwnedTerminalEventSidecar>>>,
    transaction_id: TransactionKey,
    id: u64,
    enqueued: bool,
}

impl OwnedTerminalEventRegistration {
    fn mark_enqueued(&mut self) {
        self.enqueued = true;
    }
}

impl Drop for OwnedTerminalEventRegistration {
    fn drop(&mut self) {
        if self.enqueued {
            return;
        }
        let mut empty = false;
        if let Some(mut sidecars) = self.sidecars.get_mut(&self.transaction_id) {
            sidecars.retain(|sidecar| sidecar.id != self.id);
            empty = sidecars.is_empty();
        }
        if empty {
            self.sidecars
                .remove_if(&self.transaction_id, |_, sidecars| sidecars.is_empty());
        }
    }
}

#[derive(Clone)]
pub(crate) struct EventSubscriber {
    pub(crate) id: usize,
    pub(crate) sender: mpsc::Sender<TransactionEvent>,
    pub(crate) global: bool,
    lagged_events: Arc<AtomicU64>,
    last_lag_warning_second: Arc<AtomicU64>,
}

impl EventSubscriber {
    pub(crate) fn new(id: usize, sender: mpsc::Sender<TransactionEvent>, global: bool) -> Self {
        Self {
            id,
            sender,
            global,
            lagged_events: Arc::new(AtomicU64::new(0)),
            last_lag_warning_second: Arc::new(AtomicU64::new(0)),
        }
    }

    fn record_lag(&self) -> Option<u64> {
        let dropped = self.lagged_events.fetch_add(1, Ordering::Relaxed) + 1;
        static LAG_EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
        let second = LAG_EPOCH
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_secs()
            .saturating_add(1);
        let previous = self.last_lag_warning_second.load(Ordering::Relaxed);
        if previous == second
            || self
                .last_lag_warning_second
                .compare_exchange(previous, second, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            None
        } else {
            Some(dropped)
        }
    }
}

/// Shared read-mostly optional observer indexes. This deliberately contains no
/// manager or transaction handle, so installing it in transaction data cannot
/// form an ownership cycle.
#[derive(Clone)]
pub(crate) struct TransactionObserverFanout {
    global: Arc<ArcSwap<Vec<EventSubscriber>>>,
    subscriber_to_transactions: Arc<DashMap<usize, Vec<TransactionKey>>>,
    keyed: Arc<DashMap<TransactionKey, Vec<EventSubscriber>>>,
    keyed_subscriber_count: Arc<AtomicUsize>,
}

impl TransactionObserverFanout {
    pub(crate) fn new(
        global: Arc<ArcSwap<Vec<EventSubscriber>>>,
        subscriber_to_transactions: Arc<DashMap<usize, Vec<TransactionKey>>>,
        keyed: Arc<DashMap<TransactionKey, Vec<EventSubscriber>>>,
    ) -> Self {
        Self {
            global,
            subscriber_to_transactions,
            keyed,
            keyed_subscriber_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn has_observers_for(&self, event: &TransactionEvent) -> bool {
        if !self.global.load().is_empty() {
            return true;
        }
        if self.keyed_subscriber_count.load(Ordering::Relaxed) == 0 {
            return false;
        }
        event
            .transaction_id()
            .is_some_and(|transaction_id| self.keyed.contains_key(transaction_id))
    }

    pub(crate) fn add_transaction_subscriber(
        &self,
        transaction_id: TransactionKey,
        subscriber: EventSubscriber,
    ) {
        // Reserve the exact count before publishing the keyed bucket. If
        // terminal cleanup races between the keyed and reverse-index writes,
        // either cleanup owns this decrement or subscription revalidation
        // removes it; the zero-observer fast path can never remain stuck on.
        self.keyed_subscriber_count.fetch_add(1, Ordering::Relaxed);
        self.keyed
            .entry(transaction_id.clone())
            .or_default()
            .push(subscriber.clone());
        self.subscriber_to_transactions
            .entry(subscriber.id)
            .or_default()
            .push(transaction_id);
    }

    pub(crate) fn remove_subscriber(&self, subscriber_id: usize) {
        let Some((_, transaction_ids)) = self.subscriber_to_transactions.remove(&subscriber_id)
        else {
            return;
        };
        for transaction_id in transaction_ids {
            let mut removed = 0usize;
            let mut empty = false;
            if let Some(mut entry) = self.keyed.get_mut(&transaction_id) {
                let previous = entry.value().len();
                entry
                    .value_mut()
                    .retain(|subscriber| subscriber.id != subscriber_id);
                removed = previous.saturating_sub(entry.value().len());
                empty = entry.value().is_empty();
            }
            if empty {
                self.keyed
                    .remove_if(&transaction_id, |_, subscribers| subscribers.is_empty());
            }
            if removed != 0 {
                self.keyed_subscriber_count
                    .fetch_sub(removed, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn prune_closed_now(&self) {
        let mut closed: Vec<_> = self
            .global
            .load()
            .iter()
            .filter(|subscriber| subscriber.sender.is_closed())
            .map(|subscriber| subscriber.id)
            .collect();
        closed.extend(self.subscriber_to_transactions.iter().filter_map(|entry| {
            let subscriber_id = *entry.key();
            entry.value().iter().find_map(|transaction_id| {
                self.keyed.get(transaction_id).and_then(|subscribers| {
                    subscribers
                        .iter()
                        .find(|subscriber| subscriber.id == subscriber_id)
                        .filter(|subscriber| subscriber.sender.is_closed())
                        .map(|_| subscriber_id)
                })
            })
        }));
        closed.sort_unstable();
        closed.dedup();
        self.prune_closed(&closed);
    }

    fn publish(&self, event: TransactionEvent) {
        let transaction_id = event.transaction_id();
        let keyed = transaction_id
            .and_then(|transaction_id| self.keyed.get(transaction_id))
            .map(|entry| entry.value().clone())
            .unwrap_or_default();
        let global = self.global.load();
        let subscribers: Vec<_> = global.iter().chain(keyed.iter()).cloned().collect();
        drop(global);
        self.publish_snapshot(event, subscribers);
    }

    fn snapshot(&self, event: &TransactionEvent) -> Vec<EventSubscriber> {
        let transaction_id = event.transaction_id();
        let keyed = transaction_id
            .and_then(|transaction_id| self.keyed.get(transaction_id))
            .map(|entry| entry.value().clone())
            .unwrap_or_default();
        let global = self.global.load();
        global.iter().chain(keyed.iter()).cloned().collect()
    }

    fn publish_snapshot(&self, event: TransactionEvent, subscribers: Vec<EventSubscriber>) {
        let transaction_id = event.transaction_id();
        let mut closed = Vec::new();

        for subscriber in &subscribers {
            debug_assert!(subscriber.global || transaction_id.is_some());
            match subscriber.sender.try_send(event.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Optional observers never backpressure the primary TU
                    // path. This warning is the explicit lag diagnostic.
                    if let Some(dropped) = subscriber.record_lag() {
                        warn!(
                            subscriber_id = subscriber.id,
                            transaction_scoped = transaction_id.is_some(),
                            dropped_events = dropped,
                            "Transaction observer lagged; dropping events (warning rate-limited)"
                        );
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    closed.push(subscriber.id);
                    debug!(
                        subscriber_id = subscriber.id,
                        transaction_scoped = transaction_id.is_some(),
                        "Transaction observer channel closed"
                    );
                }
            }
        }
        if !closed.is_empty() {
            self.prune_closed(&closed);
        }
    }

    /// Remove one transaction's keyed observer indexes after its exact final
    /// compact generation is no longer capable of publishing events.
    pub(crate) fn remove_transaction(&self, transaction_id: &TransactionKey) {
        let Some((_, subscribers)) = self.keyed.remove(transaction_id) else {
            return;
        };
        self.keyed_subscriber_count
            .fetch_sub(subscribers.len(), Ordering::Relaxed);
        for subscriber in subscribers {
            let mut empty = false;
            if let Some(mut entry) = self.subscriber_to_transactions.get_mut(&subscriber.id) {
                entry.value_mut().retain(|key| key != transaction_id);
                empty = entry.value().is_empty();
            }
            if empty {
                self.subscriber_to_transactions.remove(&subscriber.id);
            }
        }
    }

    fn prune_closed(&self, closed: &[usize]) {
        self.global.rcu(|current| {
            Arc::new(
                current
                    .iter()
                    .filter(|subscriber| !closed.contains(&subscriber.id))
                    .cloned()
                    .collect(),
            )
        });

        for subscriber_id in closed {
            self.remove_subscriber(*subscriber_id);
        }
    }
}

/// A single event path shared by manager-generated and runner-generated
/// events. Exact completion is updated by the caller before invoking this
/// sender. Primary delivery is lossless; optional fanout happens afterward
/// and is nonblocking.
#[derive(Clone)]
pub struct TransactionEventSender {
    primary: TransactionEventPrimary,
    observers: Arc<OnceLock<TransactionObserverFanout>>,
    /// Exact sidecar for compact terminal events on the canonical shared
    /// channel. The Arc pointer is preserved through the dialog sharded queue,
    /// so an older same-key event cannot acknowledge a newer generation.
    terminal_event_fences: Arc<DashMap<usize, TerminalEventSidecar>>,
    owned_terminal_event_fences: Arc<DashMap<TransactionKey, VecDeque<OwnedTerminalEventSidecar>>>,
    next_terminal_fence_id: Arc<AtomicU64>,
    terminal_ack_required: Arc<AtomicBool>,
    close_monitor_started: Arc<AtomicBool>,
    terminal_delivery_failure_hook: Arc<OnceLock<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
enum TransactionEventPrimary {
    /// Outbound-only compatibility managers historically spawned a lifetime
    /// task that drained and discarded this stream because their constructor
    /// does not return a TU receiver. Model that behavior directly: events are
    /// still delivered to optional observers, but no queue, receiver, or task
    /// exists solely to throw the primary copy away.
    Detached,
    /// Compatibility path retained for callers that construct transactions
    /// directly with the historical owned-value channel.
    Owned(mpsc::Sender<TransactionEvent>),
    /// Canonical transaction-to-dialog path. Tokio MPSC stores one pointer in
    /// each queue slot instead of the complete (large) SIP event value.
    Shared(mpsc::Sender<Arc<TransactionEvent>>),
}

impl std::fmt::Debug for TransactionEventPrimary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Detached => formatter.write_str("Detached"),
            Self::Owned(sender) => formatter.debug_tuple("Owned").field(sender).finish(),
            Self::Shared(sender) => formatter.debug_tuple("Shared").field(sender).finish(),
        }
    }
}

impl std::fmt::Debug for TransactionEventSender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransactionEventSender")
            .field("primary", &self.primary)
            .field("observers_installed", &self.observers.get().is_some())
            .finish()
    }
}

impl TransactionEventSender {
    pub(crate) fn detached_with_observers(observers: TransactionObserverFanout) -> Self {
        let sender = Self {
            primary: TransactionEventPrimary::Detached,
            observers: Arc::new(OnceLock::new()),
            terminal_event_fences: Arc::new(DashMap::new()),
            owned_terminal_event_fences: Arc::new(DashMap::new()),
            next_terminal_fence_id: Arc::new(AtomicU64::new(1)),
            terminal_ack_required: Arc::new(AtomicBool::new(false)),
            close_monitor_started: Arc::new(AtomicBool::new(false)),
            terminal_delivery_failure_hook: Arc::new(OnceLock::new()),
        };
        sender.install_observers(observers);
        sender
    }

    pub fn new(primary: mpsc::Sender<TransactionEvent>) -> Self {
        Self {
            primary: TransactionEventPrimary::Owned(primary),
            observers: Arc::new(OnceLock::new()),
            terminal_event_fences: Arc::new(DashMap::new()),
            owned_terminal_event_fences: Arc::new(DashMap::new()),
            next_terminal_fence_id: Arc::new(AtomicU64::new(1)),
            terminal_ack_required: Arc::new(AtomicBool::new(false)),
            close_monitor_started: Arc::new(AtomicBool::new(false)),
            terminal_delivery_failure_hook: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn new_shared(primary: mpsc::Sender<Arc<TransactionEvent>>) -> Self {
        Self {
            primary: TransactionEventPrimary::Shared(primary),
            observers: Arc::new(OnceLock::new()),
            terminal_event_fences: Arc::new(DashMap::new()),
            owned_terminal_event_fences: Arc::new(DashMap::new()),
            next_terminal_fence_id: Arc::new(AtomicU64::new(1)),
            terminal_ack_required: Arc::new(AtomicBool::new(false)),
            close_monitor_started: Arc::new(AtomicBool::new(false)),
            terminal_delivery_failure_hook: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn with_observers(
        primary: mpsc::Sender<TransactionEvent>,
        observers: TransactionObserverFanout,
    ) -> Self {
        let sender = Self::new(primary);
        sender.install_observers(observers);
        sender
    }

    pub(crate) fn with_shared_observers(
        primary: mpsc::Sender<Arc<TransactionEvent>>,
        observers: TransactionObserverFanout,
    ) -> Self {
        let sender = Self::new_shared(primary);
        sender.install_observers(observers);
        sender
    }

    pub(crate) fn install_observers(&self, observers: TransactionObserverFanout) {
        let _ = self.observers.set(observers);
    }

    pub(crate) fn observer_fanout(&self) -> Option<TransactionObserverFanout> {
        self.observers.get().cloned()
    }

    pub(crate) fn clone_for_transaction(&self) -> Self {
        self.clone()
    }

    #[cfg(test)]
    pub(crate) fn is_detached_primary(&self) -> bool {
        matches!(&self.primary, TransactionEventPrimary::Detached)
    }

    pub(crate) fn supports_exact_compact_terminal_ack(&self) -> bool {
        true
    }

    pub(crate) fn require_terminal_ack(&self) {
        self.terminal_ack_required.store(true, Ordering::Release);
    }

    pub(crate) fn install_terminal_delivery_failure_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        let _ = self.terminal_delivery_failure_hook.set(hook);
    }

    fn fail_closed_after_terminal_delivery_error(&self) {
        if let Some(hook) = self.terminal_delivery_failure_hook.get() {
            hook();
        }
    }

    /// Close transaction admission when any event in an authoritative
    /// terminal batch cannot reach the primary consumer. Compact Timer J/K
    /// publishes TimerTriggered and StateChanged before the final terminal
    /// event; losing either prefix is just as unsafe as losing the terminal
    /// observation itself.
    pub(crate) fn fail_closed_terminal_batch(&self) {
        self.fail_closed_after_terminal_delivery_error();
    }

    pub(crate) fn take_terminal_event_fence(
        &self,
        event: &Arc<TransactionEvent>,
    ) -> Option<TerminalEventFence> {
        let identity = Arc::as_ptr(event) as usize;
        let event_weak = Arc::downgrade(event);
        if let Some((_, sidecar)) = self
            .terminal_event_fences
            .remove_if(&identity, |_, current| current.event.ptr_eq(&event_weak))
        {
            return Some(TerminalEventFence {
                compact_generation: sidecar.compact_generation,
                _admission_owner: sidecar._admission_owner,
            });
        }
        let transaction_id = event.transaction_id()?;
        let mut sidecar = None;
        let mut empty = false;
        if let Some(mut sidecars) = self.owned_terminal_event_fences.get_mut(transaction_id) {
            sidecar = sidecars.pop_front();
            empty = sidecars.is_empty();
        }
        if empty {
            self.owned_terminal_event_fences
                .remove_if(transaction_id, |_, sidecars| sidecars.is_empty());
        }
        sidecar.map(|sidecar| TerminalEventFence {
            compact_generation: sidecar.compact_generation,
            _admission_owner: sidecar._admission_owner,
        })
    }

    #[cfg(test)]
    pub(crate) fn take_compact_terminal_generation(
        &self,
        event: &Arc<TransactionEvent>,
    ) -> Option<u64> {
        self.take_terminal_event_fence(event)
            .and_then(|fence| fence.compact_generation())
    }

    /// Clear sidecars when the compact dispatcher stops. At that boundary its
    /// exact tombstone generations are also released, so no later dialog ACK
    /// is protocol-authoritative.
    pub(crate) fn clear_terminal_event_fences(&self) {
        self.terminal_event_fences.clear();
        self.owned_terminal_event_fences.clear();
    }

    #[cfg(test)]
    fn compact_terminal_generation_count(&self) -> usize {
        // A dropped shared receiver destroys its queued Arcs without calling
        // `take`. Weak ownership prevents pointer reuse; prune those dead
        // registrations when inspecting the logical sidecar set.
        self.terminal_event_fences
            .retain(|_, sidecar| sidecar.event.strong_count() > 0);
        self.terminal_event_fences.len()
    }

    #[cfg(test)]
    fn owned_terminal_fence_count(&self) -> usize {
        self.owned_terminal_event_fences
            .iter()
            .map(|entry| entry.value().len())
            .sum()
    }

    /// Send the final compact terminal observation with an exact generation
    /// sidecar on the canonical shared path. Owned/raw consumers have no
    /// internal second queue, so ordinary send completion remains their
    /// compatibility boundary.
    #[cfg(test)]
    pub(crate) async fn send_compact_terminal(
        &self,
        event: TransactionEvent,
        generation: u64,
    ) -> std::result::Result<(), mpsc::error::SendError<TransactionEvent>> {
        self.send_terminal(event, Some(generation), None).await
    }

    pub(crate) async fn send_terminal(
        &self,
        event: TransactionEvent,
        compact_generation: Option<u64>,
        admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    ) -> std::result::Result<(), mpsc::error::SendError<TransactionEvent>> {
        if !self.terminal_ack_required.load(Ordering::Acquire) {
            let transaction_id = event.transaction_id().cloned();
            let result = self.send(event).await;
            if result.is_err() {
                self.fail_closed_after_terminal_delivery_error();
            }
            if let (Some(observers), Some(transaction_id)) =
                (self.observers.get(), transaction_id.as_ref())
            {
                observers.remove_transaction(transaction_id);
            }
            return result;
        }
        let terminal_transaction_id = event.transaction_id().cloned();
        let observation = self
            .observers
            .get()
            .filter(|observers| observers.has_observers_for(&event))
            .map(|observers| (event.clone(), observers.snapshot(&event)));
        self.ensure_receiver_close_monitor();
        let primary_result = match &self.primary {
            TransactionEventPrimary::Detached => Ok(()),
            TransactionEventPrimary::Shared(sender) => {
                let event = Arc::new(event);
                let identity = Arc::as_ptr(&event) as usize;
                let event_weak = Arc::downgrade(&event);
                self.terminal_event_fences.insert(
                    identity,
                    TerminalEventSidecar {
                        event: event_weak.clone(),
                        compact_generation,
                        _admission_owner: admission_owner,
                    },
                );
                let mut registration = TerminalEventRegistration {
                    sidecars: Arc::clone(&self.terminal_event_fences),
                    identity,
                    event: event_weak,
                    enqueued: false,
                };
                let result = sender
                    .send(event)
                    .await
                    .map_err(|error| mpsc::error::SendError(into_owned_event(error.0)));
                if result.is_err() {
                    // Close admission while the sidecar still owns the exact
                    // wire-key fence; no replacement can slip between owner
                    // release and fail-closed publication.
                    self.fail_closed_after_terminal_delivery_error();
                }
                if result.is_ok() {
                    registration.mark_enqueued();
                }
                result
            }
            TransactionEventPrimary::Owned(sender) => {
                let transaction_id = event
                    .transaction_id()
                    .cloned()
                    .expect("terminal event admission fences require a transaction identifier");
                let id = self.next_terminal_fence_id.fetch_add(1, Ordering::Relaxed);
                self.owned_terminal_event_fences
                    .entry(transaction_id.clone())
                    .or_default()
                    .push_back(OwnedTerminalEventSidecar {
                        id,
                        compact_generation,
                        _admission_owner: admission_owner,
                    });
                let mut registration = OwnedTerminalEventRegistration {
                    sidecars: Arc::clone(&self.owned_terminal_event_fences),
                    transaction_id,
                    id,
                    enqueued: false,
                };
                let result = sender.send(event).await;
                if result.is_err() {
                    self.fail_closed_after_terminal_delivery_error();
                }
                if result.is_ok() {
                    registration.mark_enqueued();
                }
                result
            }
        };
        if let (Some(observers), Some((observation, subscribers))) =
            (self.observers.get(), observation)
        {
            observers.publish_snapshot(observation, subscribers);
        }
        if let (Some(observers), Some(transaction_id)) =
            (self.observers.get(), terminal_transaction_id.as_ref())
        {
            observers.remove_transaction(transaction_id);
        }
        if primary_result.is_err() {
            self.fail_closed_after_terminal_delivery_error();
        }
        primary_result
    }

    fn ensure_receiver_close_monitor(&self) {
        if matches!(&self.primary, TransactionEventPrimary::Detached) {
            return;
        }
        if self
            .close_monitor_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let pointer_fences = Arc::clone(&self.terminal_event_fences);
        let owned_fences = Arc::clone(&self.owned_terminal_event_fences);
        let failure_hook = self.terminal_delivery_failure_hook.get().cloned();
        match &self.primary {
            TransactionEventPrimary::Detached => {}
            TransactionEventPrimary::Owned(sender) => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    sender.closed().await;
                    if let Some(hook) = failure_hook {
                        hook();
                    }
                    pointer_fences.clear();
                    owned_fences.clear();
                });
            }
            TransactionEventPrimary::Shared(sender) => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    sender.closed().await;
                    if let Some(hook) = failure_hook {
                        hook();
                    }
                    pointer_fences.clear();
                    owned_fences.clear();
                });
            }
        }
    }

    pub async fn send(
        &self,
        event: TransactionEvent,
    ) -> std::result::Result<(), mpsc::error::SendError<TransactionEvent>> {
        let observation = self
            .observers
            .get()
            .filter(|observers| observers.has_observers_for(&event))
            .map(|_| event.clone());
        let primary_result = match &self.primary {
            TransactionEventPrimary::Detached => Ok(()),
            TransactionEventPrimary::Owned(sender) => sender.send(event).await,
            TransactionEventPrimary::Shared(sender) => sender
                .send(Arc::new(event))
                .await
                .map_err(|error| mpsc::error::SendError(into_owned_event(error.0))),
        };
        if let (Some(observers), Some(observation)) = (self.observers.get(), observation) {
            observers.publish(observation);
        }
        if primary_result.is_err() {
            // The primary stream is the protocol handoff, not an optional
            // observer. Once it is closed the manager can no longer prove TU
            // ownership of newly published requests or terminal cleanup, so
            // close admission before any caller releases its exact owner.
            self.fail_closed_after_terminal_delivery_error();
        }
        primary_result
    }

    /// Reserve primary capacity before publishing the terminal StateChanged
    /// prefix, then mark the resumable publication stage synchronously after
    /// the reserved send. Cancellation is therefore unambiguous: either the
    /// prefix was not enqueued, or takeover observes it as delivered.
    pub(crate) async fn send_terminal_prefix(
        &self,
        event: TransactionEvent,
        claim: &TerminalEventPublicationClaim,
    ) -> std::result::Result<(), mpsc::error::SendError<TransactionEvent>> {
        let observation = self
            .observers
            .get()
            .filter(|observers| observers.has_observers_for(&event))
            .map(|_| event.clone());
        let result = match &self.primary {
            TransactionEventPrimary::Detached => {
                claim.mark_prefix_delivered();
                Ok(())
            }
            TransactionEventPrimary::Owned(sender) => match sender.reserve().await {
                Ok(permit) => {
                    permit.send(event);
                    claim.mark_prefix_delivered();
                    Ok(())
                }
                Err(_) => Err(mpsc::error::SendError(event)),
            },
            TransactionEventPrimary::Shared(sender) => match sender.reserve().await {
                Ok(permit) => {
                    permit.send(Arc::new(event));
                    claim.mark_prefix_delivered();
                    Ok(())
                }
                Err(_) => Err(mpsc::error::SendError(event)),
            },
        };
        if let (Some(observers), Some(observation)) = (self.observers.get(), observation) {
            observers.publish(observation);
        }
        if result.is_err() {
            self.fail_closed_after_terminal_delivery_error();
        }
        result
    }

    pub fn try_send(
        &self,
        event: TransactionEvent,
    ) -> std::result::Result<(), mpsc::error::TrySendError<TransactionEvent>> {
        let observation = self
            .observers
            .get()
            .filter(|observers| observers.has_observers_for(&event))
            .map(|_| event.clone());
        let primary_result =
            match &self.primary {
                TransactionEventPrimary::Detached => Ok(()),
                TransactionEventPrimary::Owned(sender) => sender.try_send(event),
                TransactionEventPrimary::Shared(sender) => sender
                    .try_send(Arc::new(event))
                    .map_err(|error| match error {
                        mpsc::error::TrySendError::Full(event) => {
                            mpsc::error::TrySendError::Full(into_owned_event(event))
                        }
                        mpsc::error::TrySendError::Closed(event) => {
                            mpsc::error::TrySendError::Closed(into_owned_event(event))
                        }
                    }),
            };
        if let (Some(observers), Some(observation)) = (self.observers.get(), observation) {
            observers.publish(observation);
        }
        if matches!(primary_result, Err(mpsc::error::TrySendError::Closed(_))) {
            self.fail_closed_after_terminal_delivery_error();
        }
        primary_result
    }
}

impl From<mpsc::Sender<TransactionEvent>> for TransactionEventSender {
    fn from(sender: mpsc::Sender<TransactionEvent>) -> Self {
        Self::new(sender)
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use rvoip_sip_core::Method;

    use super::*;

    fn timeout_event(branch: &str) -> TransactionEvent {
        TransactionEvent::TransactionTimeout {
            transaction_id: TransactionKey::new(branch.to_owned(), Method::Options, false),
        }
    }

    fn branch(event: &TransactionEvent) -> &str {
        event
            .transaction_id()
            .expect("transaction-scoped test event")
            .branch
            .as_str()
    }

    #[test]
    fn shared_queue_slot_is_pointer_sized() {
        assert_eq!(size_of::<Arc<TransactionEvent>>(), size_of::<usize>());
        assert!(size_of::<TransactionEvent>() > size_of::<Arc<TransactionEvent>>());
    }

    #[tokio::test]
    async fn shared_primary_preserves_order() {
        let (primary, mut events) = mpsc::channel(2);
        let sender = TransactionEventSender::new_shared(primary);

        sender.send(timeout_event("first")).await.unwrap();
        sender.send(timeout_event("second")).await.unwrap();

        assert_eq!(branch(&events.recv().await.unwrap()), "first");
        assert_eq!(branch(&events.recv().await.unwrap()), "second");
    }

    #[tokio::test]
    async fn shared_primary_reports_full_and_closed_with_original_event() {
        let (primary, events) = mpsc::channel(1);
        let sender = TransactionEventSender::new_shared(primary);
        sender.try_send(timeout_event("queued")).unwrap();
        let full = sender
            .try_send(timeout_event("full"))
            .expect_err("bounded shared queue must report full");
        match full {
            mpsc::error::TrySendError::Full(event) => assert_eq!(branch(&event), "full"),
            error => panic!("unexpected error: {error}"),
        }
        drop(events);
        let closed = sender
            .send(timeout_event("closed"))
            .await
            .expect_err("closed shared queue must reject event");
        assert_eq!(branch(&closed.0), "closed");
    }

    #[tokio::test]
    async fn owned_primary_retains_legacy_value_semantics() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        sender.send(timeout_event("owned")).await.unwrap();
        assert_eq!(branch(&events.recv().await.unwrap()), "owned");
    }

    #[tokio::test]
    async fn detached_primary_discards_without_queue_or_close_monitor_but_keeps_observers() {
        let (observer_tx, mut observer_rx) = mpsc::channel(4);
        let global = Arc::new(ArcSwap::from_pointee(vec![EventSubscriber::new(
            1,
            observer_tx,
            true,
        )]));
        let subscriber_to_transactions = Arc::new(DashMap::new());
        let keyed = Arc::new(DashMap::new());
        let sender = TransactionEventSender::detached_with_observers(
            TransactionObserverFanout::new(global, subscriber_to_transactions, keyed),
        );

        sender.send(timeout_event("detached-send")).await.unwrap();
        sender.try_send(timeout_event("detached-try-send")).unwrap();
        assert_eq!(branch(&observer_rx.recv().await.unwrap()), "detached-send");
        assert_eq!(
            branch(&observer_rx.recv().await.unwrap()),
            "detached-try-send"
        );

        sender.require_terminal_ack();
        sender
            .send_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: TransactionKey::new(
                        "detached-terminal".into(),
                        Method::Bye,
                        false,
                    ),
                },
                Some(17),
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            observer_rx.recv().await,
            Some(TransactionEvent::TransactionTerminated { .. })
        ));
        assert!(!sender.close_monitor_started.load(Ordering::Acquire));
        assert_eq!(sender.compact_terminal_generation_count(), 0);
        assert_eq!(sender.owned_terminal_fence_count(), 0);
    }

    #[tokio::test]
    async fn compact_terminal_generation_is_bound_to_exact_shared_arc() {
        let (primary, mut events) = mpsc::channel(2);
        let sender = TransactionEventSender::new_shared(primary);
        sender.require_terminal_ack();
        let event = TransactionEvent::TransactionTerminated {
            transaction_id: TransactionKey::new("compact-exact-arc".into(), Method::Bye, false),
        };
        sender.send_compact_terminal(event, 77).await.unwrap();
        let received = events.recv().await.expect("compact terminal event");
        let unrelated = Arc::new((*received).clone());

        assert_eq!(sender.take_compact_terminal_generation(&unrelated), None);
        assert_eq!(sender.take_compact_terminal_generation(&received), Some(77));
        assert_eq!(sender.take_compact_terminal_generation(&received), None);
    }

    #[tokio::test]
    async fn cancelling_blocked_compact_send_removes_exact_sidecar() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new_shared(primary);
        sender.require_terminal_ack();
        sender.send(timeout_event("queue-blocker")).await.unwrap();

        let blocked_sender = sender.clone();
        let blocked = tokio::spawn(async move {
            blocked_sender
                .send_compact_terminal(
                    TransactionEvent::TransactionTerminated {
                        transaction_id: TransactionKey::new(
                            "cancelled-compact-send".into(),
                            Method::Bye,
                            false,
                        ),
                    },
                    91,
                )
                .await
        });
        for _ in 0..100 {
            if sender.compact_terminal_generation_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(sender.compact_terminal_generation_count(), 1);
        blocked.abort();
        assert!(blocked.await.unwrap_err().is_cancelled());
        assert_eq!(sender.compact_terminal_generation_count(), 0);

        assert_eq!(branch(&events.recv().await.unwrap()), "queue-blocker");
    }

    #[tokio::test]
    async fn dropping_shared_receiver_releases_queued_compact_sidecars() {
        let (primary, events) = mpsc::channel(1);
        let sender = TransactionEventSender::new_shared(primary);
        sender.require_terminal_ack();
        sender
            .send_compact_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: TransactionKey::new(
                        "receiver-drop-compact".into(),
                        Method::Bye,
                        false,
                    ),
                },
                92,
            )
            .await
            .unwrap();
        assert_eq!(sender.compact_terminal_generation_count(), 1);

        drop(events);
        for _ in 0..100 {
            if sender.compact_terminal_generation_count() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(sender.compact_terminal_generation_count(), 0);
    }

    #[tokio::test]
    async fn owned_compact_terminal_uses_send_boundary_without_sidecar() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        assert!(sender.supports_exact_compact_terminal_ack());
        sender
            .send_compact_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: TransactionKey::new(
                        "compact-owned-fallback".into(),
                        Method::Bye,
                        false,
                    ),
                },
                88,
            )
            .await
            .unwrap();
        let received = Arc::new(events.recv().await.expect("owned terminal event"));
        assert_eq!(sender.take_compact_terminal_generation(&received), None);
    }

    #[tokio::test]
    async fn owned_terminal_fence_is_keyed_and_released_on_take() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        sender.require_terminal_ack();
        sender
            .send_compact_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: TransactionKey::new(
                        "owned-terminal-fence".into(),
                        Method::Bye,
                        false,
                    ),
                },
                93,
            )
            .await
            .unwrap();
        assert_eq!(sender.owned_terminal_fence_count(), 1);
        let event = Arc::new(events.recv().await.expect("owned terminal"));
        assert_eq!(sender.take_compact_terminal_generation(&event), Some(93));
        assert_eq!(sender.owned_terminal_fence_count(), 0);
    }

    #[tokio::test]
    async fn dropping_owned_receiver_releases_queued_terminal_fence() {
        let (primary, events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        sender.require_terminal_ack();
        sender
            .send_compact_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: TransactionKey::new(
                        "owned-receiver-drop".into(),
                        Method::Bye,
                        false,
                    ),
                },
                94,
            )
            .await
            .unwrap();
        assert_eq!(sender.owned_terminal_fence_count(), 1);
        drop(events);
        for _ in 0..100 {
            if sender.owned_terminal_fence_count() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(sender.owned_terminal_fence_count(), 0);
    }

    #[tokio::test]
    async fn cancelled_blocked_terminal_prefix_is_resumed_by_exact_takeover() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        sender.send(timeout_event("prefix-blocker")).await.unwrap();
        let transaction_id =
            TransactionKey::new("terminal-prefix-takeover".into(), Method::Invite, false);
        let publication = TerminalEventPublication::new();
        let claim = publication.try_claim().expect("initial publication claim");
        claim.publication().record_prefix(TransactionState::Calling);
        let blocked_sender = sender.clone();
        let blocked_transaction_id = transaction_id.clone();
        let blocked = tokio::spawn(async move {
            let _ = blocked_sender
                .send_terminal_prefix(
                    TransactionEvent::StateChanged {
                        transaction_id: blocked_transaction_id,
                        previous_state: TransactionState::Calling,
                        new_state: TransactionState::Terminated,
                    },
                    &claim,
                )
                .await;
            claim
        });
        tokio::task::yield_now().await;
        blocked.abort();
        assert!(blocked.await.unwrap_err().is_cancelled());

        assert_eq!(
            publication.pending_prefix(),
            Some(TransactionState::Calling)
        );
        let takeover = publication.try_claim().expect("takeover publication claim");
        assert_eq!(branch(&events.recv().await.unwrap()), "prefix-blocker");
        sender
            .send_terminal_prefix(
                TransactionEvent::StateChanged {
                    transaction_id: transaction_id.clone(),
                    previous_state: TransactionState::Calling,
                    new_state: TransactionState::Terminated,
                },
                &takeover,
            )
            .await
            .unwrap();
        assert!(matches!(
            events.recv().await,
            Some(TransactionEvent::StateChanged { transaction_id: observed, .. })
                if observed == transaction_id
        ));
        sender
            .send_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: transaction_id.clone(),
                },
                None,
                None,
            )
            .await
            .unwrap();
        takeover.mark_delivered();
        assert!(matches!(
            events.recv().await,
            Some(TransactionEvent::TransactionTerminated { transaction_id: observed })
                if observed == transaction_id
        ));
        assert!(publication.is_delivered());
        assert!(publication.try_claim().is_none());
    }

    #[tokio::test]
    async fn cancelled_terminal_send_takeover_does_not_duplicate_delivered_prefix() {
        let (primary, mut events) = mpsc::channel(1);
        let sender = TransactionEventSender::new(primary);
        let transaction_id = TransactionKey::new(
            "terminal-after-prefix-takeover".into(),
            Method::Invite,
            false,
        );
        let publication = TerminalEventPublication::new();
        let claim = publication.try_claim().expect("initial publication claim");
        claim.publication().record_prefix(TransactionState::Calling);
        sender
            .send_terminal_prefix(
                TransactionEvent::StateChanged {
                    transaction_id: transaction_id.clone(),
                    previous_state: TransactionState::Calling,
                    new_state: TransactionState::Terminated,
                },
                &claim,
            )
            .await
            .unwrap();
        assert_eq!(publication.pending_prefix(), None);

        let blocked_sender = sender.clone();
        let blocked_transaction_id = transaction_id.clone();
        let blocked = tokio::spawn(async move {
            let _ = blocked_sender
                .send_terminal(
                    TransactionEvent::TransactionTerminated {
                        transaction_id: blocked_transaction_id,
                    },
                    None,
                    None,
                )
                .await;
            claim
        });
        tokio::task::yield_now().await;
        blocked.abort();
        assert!(blocked.await.unwrap_err().is_cancelled());

        let takeover = publication.try_claim().expect("terminal takeover claim");
        assert!(matches!(
            events.recv().await,
            Some(TransactionEvent::StateChanged { transaction_id: observed, .. })
                if observed == transaction_id
        ));
        assert_eq!(publication.pending_prefix(), None);
        sender
            .send_terminal(
                TransactionEvent::TransactionTerminated {
                    transaction_id: transaction_id.clone(),
                },
                None,
                None,
            )
            .await
            .unwrap();
        takeover.mark_delivered();
        assert!(matches!(
            events.recv().await,
            Some(TransactionEvent::TransactionTerminated { transaction_id: observed })
                if observed == transaction_id
        ));
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn zero_keyed_observers_take_constant_time_empty_branch() {
        let fanout = TransactionObserverFanout::new(
            Arc::new(ArcSwap::from_pointee(Vec::new())),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        );
        assert_eq!(fanout.keyed_subscriber_count.load(Ordering::Relaxed), 0);
        assert!(!fanout.has_observers_for(&timeout_event("no-observers")));
    }

    #[tokio::test]
    async fn observer_lag_diagnostics_are_counted_and_rate_limited() {
        let (observer_tx, _observer_rx) = mpsc::channel(1);
        let subscriber = EventSubscriber::new(11, observer_tx, true);
        subscriber.sender.try_send(timeout_event("fill")).unwrap();
        for index in 0..16 {
            let _ = subscriber.record_lag().filter(|_| index == 0);
        }
        assert_eq!(subscriber.lagged_events.load(Ordering::Relaxed), 16);
        assert_eq!(
            subscriber.last_lag_warning_second.load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn lagging_observer_never_blocks_shared_primary() {
        let (observer_tx, mut observer_rx) = mpsc::channel(1);
        let observers = TransactionObserverFanout::new(
            Arc::new(ArcSwap::from_pointee(vec![EventSubscriber::new(
                7,
                observer_tx,
                true,
            )])),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        );
        let (primary, mut events) = mpsc::channel(2);
        let sender = TransactionEventSender::with_shared_observers(primary, observers);

        sender.send(timeout_event("first")).await.unwrap();
        sender
            .send(timeout_event("primary-must-not-block"))
            .await
            .unwrap();

        assert_eq!(branch(&events.recv().await.unwrap()), "first");
        assert_eq!(
            branch(&events.recv().await.unwrap()),
            "primary-must-not-block"
        );
        assert_eq!(branch(&observer_rx.recv().await.unwrap()), "first");
        assert!(matches!(
            observer_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
