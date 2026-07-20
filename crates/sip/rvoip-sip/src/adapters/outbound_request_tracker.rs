use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::mpsc;

use crate::errors::{Result, SessionError};
use crate::state_table::SessionId;
use rvoip_sip_core::Method;
use rvoip_sip_dialog::transaction::TransactionKey;

const DEFAULT_NON_INVITE_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(32);
const DEFERRED_REPLAY_CHANNEL_CAPACITY: usize = 16_384;
const MAX_DEFERRED_EVENTS_PER_REQUEST: usize = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TrackedInDialogMethod {
    Refer,
    Notify,
    Info,
    Update,
}

impl TrackedInDialogMethod {
    pub(crate) fn from_sip_method(method: &Method) -> Option<Self> {
        match method {
            Method::Refer => Some(Self::Refer),
            Method::Notify => Some(Self::Notify),
            Method::Info => Some(Self::Info),
            Method::Update => Some(Self::Update),
            _ => None,
        }
    }

    pub(crate) fn from_label(method: &str) -> Option<Self> {
        match method.trim().to_ascii_uppercase().as_str() {
            "REFER" => Some(Self::Refer),
            "NOTIFY" => Some(Self::Notify),
            "INFO" => Some(Self::Info),
            "UPDATE" => Some(Self::Update),
            _ => None,
        }
    }

    pub(crate) fn as_sip_method(self) -> Method {
        match self {
            Self::Refer => Method::Refer,
            Self::Notify => Method::Notify,
            Self::Info => Method::Info,
            Self::Update => Method::Update,
        }
    }
}

#[derive(Clone)]
pub(crate) enum TrackedInDialogOptions {
    Refer(Arc<rvoip_sip_dialog::api::unified::ReferRequestOptions>),
    Notify(Arc<rvoip_sip_dialog::api::unified::NotifyRequestOptions>),
    Info(Arc<rvoip_sip_dialog::api::unified::InfoRequestOptions>),
    Update(Arc<rvoip_sip_dialog::api::unified::UpdateRequestOptions>),
}

impl TrackedInDialogOptions {
    pub(crate) fn method(&self) -> TrackedInDialogMethod {
        match self {
            Self::Refer(_) => TrackedInDialogMethod::Refer,
            Self::Notify(_) => TrackedInDialogMethod::Notify,
            Self::Info(_) => TrackedInDialogMethod::Info,
            Self::Update(_) => TrackedInDialogMethod::Update,
        }
    }
}

#[derive(Clone, Eq)]
struct TrackedRequestKey {
    session_id: SessionId,
    method: TrackedInDialogMethod,
}

impl PartialEq for TrackedRequestKey {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id && self.method == other.method
    }
}

impl Hash for TrackedRequestKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.session_id.hash(state);
        self.method.hash(state);
    }
}

enum ActivationState {
    Prepared,
    Active(TransactionKey),
    Aborted,
}

struct TrackedRequestLifecycle {
    activation: ActivationState,
    deferred: Vec<DeferredTrackedRequestEvent>,
    replays_in_flight: usize,
}

struct TrackedRequestEntry {
    generation: u64,
    options: TrackedInDialogOptions,
    auth_retry_count: u8,
    last_nonce: Option<String>,
    lifecycle: Mutex<TrackedRequestLifecycle>,
}

struct TrackerInner {
    entries: DashMap<TrackedRequestKey, Arc<TrackedRequestEntry>>,
    next_generation: AtomicU64,
    deferred_replay_tx: mpsc::Sender<DeferredTrackedRequestEvent>,
    deferred_replay_rx: Mutex<Option<mpsc::Receiver<DeferredTrackedRequestEvent>>>,
    deferred_event_count: AtomicUsize,
    max_deferred_events: usize,
}

impl Drop for TrackerInner {
    fn drop(&mut self) {
        for entry in self.entries.iter() {
            let mut lifecycle = entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle.activation = ActivationState::Aborted;
        }
    }
}

/// Immediate exact-correlation result for an in-dialog transaction event.
/// `Prepared` means the exact event was buffered without awaiting on the
/// direct dialog-to-session shard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExactTransactionLookup {
    Matched,
    Mismatched,
    Prepared,
    Rejected,
}

#[derive(Clone)]
pub(crate) enum DeferredTrackedRequestEvent {
    AuthRequired {
        session_id: SessionId,
        transaction_id: String,
        request_uri: String,
        status: u16,
        challenge: String,
        method: String,
        outbound_transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    },
    Completed {
        session_id: SessionId,
        transaction_id: String,
        method: String,
        outcome: rvoip_infra_common::events::cross_crate::OutboundRequestOutcome,
    },
}

impl DeferredTrackedRequestEvent {
    fn transaction_id(&self) -> &str {
        match self {
            Self::AuthRequired { transaction_id, .. } | Self::Completed { transaction_id, .. } => {
                transaction_id
            }
        }
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        match self {
            Self::AuthRequired { session_id, .. } | Self::Completed { session_id, .. } => {
                session_id
            }
        }
    }

    pub(crate) fn tracked_method(&self) -> Option<TrackedInDialogMethod> {
        match self {
            Self::AuthRequired { method, .. } | Self::Completed { method, .. } => {
                TrackedInDialogMethod::from_label(method)
            }
        }
    }

    fn same_kind_and_transaction(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.transaction_id() == other.transaction_id()
    }
}

#[derive(Clone)]
pub(crate) struct OutboundInDialogRequestTracker {
    inner: Arc<TrackerInner>,
}

pub(crate) struct TrackedRequestLease {
    tracker: Weak<TrackerInner>,
    key: TrackedRequestKey,
    generation: u64,
    armed: bool,
}

impl Drop for TrackedRequestLease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(inner) = self.tracker.upgrade() {
            abort_exact(&inner, &self.key, self.generation);
        }
    }
}

impl Default for OutboundInDialogRequestTracker {
    fn default() -> Self {
        Self::new(DEFAULT_NON_INVITE_TRANSACTION_TIMEOUT)
    }
}

impl OutboundInDialogRequestTracker {
    pub(crate) fn new(_non_invite_transaction_timeout: Duration) -> Self {
        Self::with_replay_capacity(DEFERRED_REPLAY_CHANNEL_CAPACITY)
    }

    fn with_replay_capacity(replay_capacity: usize) -> Self {
        let replay_capacity = replay_capacity.max(1);
        let (deferred_replay_tx, deferred_replay_rx) = mpsc::channel(replay_capacity);
        Self {
            inner: Arc::new(TrackerInner {
                entries: DashMap::new(),
                next_generation: AtomicU64::new(1),
                deferred_replay_tx,
                deferred_replay_rx: Mutex::new(Some(deferred_replay_rx)),
                deferred_event_count: AtomicUsize::new(0),
                max_deferred_events: replay_capacity,
            }),
        }
    }

    pub(crate) fn prepare(
        &self,
        session_id: &SessionId,
        options: TrackedInDialogOptions,
    ) -> Result<TrackedRequestLease> {
        let method = options.method();
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let (entry, lease) = self.new_prepared(key.clone(), options, 0, None);
        match self.inner.entries.entry(key) {
            Entry::Vacant(vacant) => {
                vacant.insert(Arc::clone(&entry));
            }
            Entry::Occupied(_) => {
                return Err(SessionError::Conflict {
                    method: method.as_sip_method(),
                });
            }
        }
        Ok(lease)
    }

    pub(crate) fn prepare_retry(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        challenged_transaction: &TransactionKey,
        challenge_nonce: Option<String>,
    ) -> Result<(TrackedRequestLease, TrackedInDialogOptions)> {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let mut occupied = match self.inner.entries.entry(key.clone()) {
            Entry::Occupied(occupied) => occupied,
            Entry::Vacant(_) => {
                return Err(SessionError::InvalidTransition(
                    "authentication retry has no exact outbound request".to_string(),
                ));
            }
        };
        let current = Arc::clone(occupied.get());
        let owns_challenge = matches!(
            &current
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .activation,
            ActivationState::Active(transaction) if transaction == challenged_transaction
        );
        if !owns_challenge {
            return Err(SessionError::InvalidTransition(
                "authentication retry does not own the challenged request".to_string(),
            ));
        }
        let options = current.options.clone();
        let auth_retry_count = current.auth_retry_count.saturating_add(1);
        let last_nonce = challenge_nonce.or_else(|| current.last_nonce.clone());
        let (replacement, lease) =
            self.new_prepared(key, options.clone(), auth_retry_count, last_nonce);
        let discarded = {
            let mut lifecycle = current
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle.activation = ActivationState::Aborted;
            std::mem::take(&mut lifecycle.deferred)
        };
        self.release_deferred_count(discarded.len());
        occupied.insert(Arc::clone(&replacement));
        drop(occupied);
        Ok((lease, options))
    }

    pub(crate) fn auth_retry_state_for_transaction(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction_id: &TransactionKey,
    ) -> Result<(u8, Option<String>)> {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let entry = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| {
                SessionError::InvalidTransition(
                    "authentication retry has no exact outbound request".to_string(),
                )
            })?;
        if !matches!(
            &entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .activation,
            ActivationState::Active(active) if active == transaction_id
        ) {
            return Err(SessionError::InvalidTransition(
                "authentication retry does not own the challenged request".to_string(),
            ));
        }
        Ok((entry.auth_retry_count, entry.last_nonce.clone()))
    }

    pub(crate) fn request_body_for_transaction(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction_id: &TransactionKey,
    ) -> Result<Option<bytes::Bytes>> {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let entry = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| {
                SessionError::InvalidTransition(
                    "authentication retry has no exact outbound request body".to_string(),
                )
            })?;
        if !matches!(
            &entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .activation,
            ActivationState::Active(active) if active == transaction_id
        ) {
            return Err(SessionError::InvalidTransition(
                "authentication retry does not own the challenged request body".to_string(),
            ));
        }
        Ok(match &entry.options {
            // RFC 7616 auth-int hashes the entity body even when its length is
            // zero. `Some(empty)` distinguishes a known empty SIP body from an
            // unavailable body and permits REFER plus bodyless NOTIFY/UPDATE
            // to answer an auth-int-only challenge correctly.
            TrackedInDialogOptions::Refer(_) => Some(bytes::Bytes::new()),
            TrackedInDialogOptions::Notify(options) => {
                Some(options.body.clone().unwrap_or_default())
            }
            TrackedInDialogOptions::Info(options) => Some(options.body.clone()),
            TrackedInDialogOptions::Update(options) => Some(
                options
                    .sdp
                    .as_ref()
                    .map(|sdp| bytes::Bytes::copy_from_slice(sdp.as_bytes()))
                    .unwrap_or_default(),
            ),
        })
    }

    pub(crate) fn activate(
        &self,
        mut lease: TrackedRequestLease,
        transaction_id: TransactionKey,
    ) -> Result<()> {
        if transaction_id.method() != &lease.key.method.as_sip_method()
            || transaction_id.is_server()
        {
            return Err(SessionError::InvalidTransition(
                "outbound request activation returned a mismatched transaction".to_string(),
            ));
        }
        let occupied = match self.inner.entries.entry(lease.key.clone()) {
            Entry::Occupied(occupied) if occupied.get().generation == lease.generation => occupied,
            Entry::Occupied(_) | Entry::Vacant(_) => {
                return Err(SessionError::InvalidTransition(
                    "outbound request ownership changed before transaction activation".to_string(),
                ));
            }
        };
        let current = Arc::clone(occupied.get());
        {
            let mut lifecycle = current
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle.activation = ActivationState::Active(transaction_id.clone());
            self.flush_lifecycle_replays(&mut lifecycle, &transaction_id)?;
        }
        drop(occupied);
        lease.armed = false;
        Ok(())
    }

    pub(crate) fn correlate_or_defer(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction_id: &TransactionKey,
        deferred_event: DeferredTrackedRequestEvent,
    ) -> ExactTransactionLookup {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let Some(entry) = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return ExactTransactionLookup::Mismatched;
        };
        let mut lifecycle = entry
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &lifecycle.activation {
            ActivationState::Prepared => {
                if lifecycle
                    .deferred
                    .iter()
                    .any(|existing| existing.same_kind_and_transaction(&deferred_event))
                {
                    return ExactTransactionLookup::Prepared;
                }
                if lifecycle.deferred.len() >= MAX_DEFERRED_EVENTS_PER_REQUEST
                    || !self.reserve_deferred_slot()
                {
                    drop(lifecycle);
                    abort_exact(&self.inner, &key, entry.generation);
                    tracing::error!(
                        session_id = %session_id,
                        method = ?method,
                        "Aborted in-dialog request after deferred event capacity violation"
                    );
                    return ExactTransactionLookup::Rejected;
                }
                lifecycle.deferred.push(deferred_event);
                ExactTransactionLookup::Prepared
            }
            ActivationState::Active(active) if active == transaction_id => {
                if lifecycle.replays_in_flight == 0 {
                    ExactTransactionLookup::Matched
                } else if lifecycle
                    .deferred
                    .iter()
                    .any(|existing| existing.same_kind_and_transaction(&deferred_event))
                {
                    ExactTransactionLookup::Prepared
                } else if lifecycle.deferred.len() >= MAX_DEFERRED_EVENTS_PER_REQUEST
                    || !self.reserve_deferred_slot()
                {
                    drop(lifecycle);
                    abort_exact(&self.inner, &key, entry.generation);
                    tracing::error!(
                        session_id = %session_id,
                        method = ?method,
                        "Aborted in-dialog request after replay ordering capacity violation"
                    );
                    ExactTransactionLookup::Rejected
                } else {
                    lifecycle.deferred.push(deferred_event);
                    ExactTransactionLookup::Prepared
                }
            }
            ActivationState::Active(_) | ActivationState::Aborted => {
                ExactTransactionLookup::Mismatched
            }
        }
    }

    pub(crate) fn complete_if_matches(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction_id: &TransactionKey,
    ) -> bool {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let Some(entry) = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return false;
        };
        let matches = matches!(
            &entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .activation,
            ActivationState::Active(active) if active == transaction_id
        );
        if !matches {
            return false;
        }
        let removed = self
            .inner
            .entries
            .remove_if(&key, |_, current| current.generation == entry.generation)
            .map(|(_, removed)| removed);
        if let Some(removed) = removed {
            let discarded = {
                let mut lifecycle = removed
                    .lifecycle
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                lifecycle.activation = ActivationState::Aborted;
                std::mem::take(&mut lifecycle.deferred)
            };
            self.release_deferred_count(discarded.len());
            true
        } else {
            false
        }
    }

    pub(crate) fn abort_matching(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction_id: &TransactionKey,
    ) -> bool {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        let Some(entry) = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return false;
        };
        let matches = matches!(
            &entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .activation,
            ActivationState::Active(active) if active == transaction_id
        );
        if !matches {
            return false;
        }
        abort_exact(&self.inner, &key, entry.generation)
    }

    pub(crate) fn has_request(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
    ) -> bool {
        let key = TrackedRequestKey {
            session_id: session_id.clone(),
            method,
        };
        self.inner.entries.contains_key(&key)
    }

    pub(crate) fn clear_session(&self, session_id: &SessionId) {
        for method in [
            TrackedInDialogMethod::Refer,
            TrackedInDialogMethod::Notify,
            TrackedInDialogMethod::Info,
            TrackedInDialogMethod::Update,
        ] {
            let key = TrackedRequestKey {
                session_id: session_id.clone(),
                method,
            };
            if let Some((_, entry)) = self.inner.entries.remove(&key) {
                let discarded = {
                    let mut lifecycle = entry
                        .lifecycle
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    lifecycle.activation = ActivationState::Aborted;
                    std::mem::take(&mut lifecycle.deferred)
                };
                self.release_deferred_count(discarded.len());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.entries.len()
    }

    fn new_prepared(
        &self,
        key: TrackedRequestKey,
        options: TrackedInDialogOptions,
        auth_retry_count: u8,
        last_nonce: Option<String>,
    ) -> (Arc<TrackedRequestEntry>, TrackedRequestLease) {
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(TrackedRequestEntry {
            generation,
            options,
            auth_retry_count,
            last_nonce,
            lifecycle: Mutex::new(TrackedRequestLifecycle {
                activation: ActivationState::Prepared,
                deferred: Vec::new(),
                replays_in_flight: 0,
            }),
        });
        let lease = TrackedRequestLease {
            tracker: Arc::downgrade(&self.inner),
            key,
            generation,
            armed: true,
        };
        (entry, lease)
    }

    pub(crate) fn take_deferred_replay_receiver(
        &self,
    ) -> Option<mpsc::Receiver<DeferredTrackedRequestEvent>> {
        self.inner
            .deferred_replay_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub(crate) fn mark_deferred_replay_started(&self, event: &DeferredTrackedRequestEvent) -> bool {
        self.release_deferred_count(1);
        let Some(method) = event.tracked_method() else {
            return false;
        };
        let Ok(transaction) = event.transaction_id().parse::<TransactionKey>() else {
            return false;
        };
        let key = TrackedRequestKey {
            session_id: event.session_id().clone(),
            method,
        };
        let Some(entry) = self
            .inner
            .entries
            .get(&key)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return false;
        };
        let mut lifecycle = entry
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(
            &lifecycle.activation,
            ActivationState::Active(active) if active == &transaction
        ) || lifecycle.replays_in_flight == 0
        {
            return false;
        }
        lifecycle.replays_in_flight -= 1;
        if let Err(error) = self.flush_lifecycle_replays(&mut lifecycle, &transaction) {
            tracing::error!(
                error = %error,
                "Deferred in-dialog replay channel violated its capacity invariant"
            );
            drop(lifecycle);
            abort_exact(&self.inner, &key, entry.generation);
            return false;
        }
        true
    }

    pub(crate) fn abort_deferred_replay(&self, event: &DeferredTrackedRequestEvent) {
        self.release_deferred_count(1);
        self.abort_replay_owner(event);
    }

    pub(crate) fn abort_started_replay(&self, event: &DeferredTrackedRequestEvent) {
        self.abort_replay_owner(event);
    }

    fn abort_replay_owner(&self, event: &DeferredTrackedRequestEvent) {
        let Some(method) = event.tracked_method() else {
            return;
        };
        let Ok(transaction) = event.transaction_id().parse::<TransactionKey>() else {
            return;
        };
        let key = TrackedRequestKey {
            session_id: event.session_id().clone(),
            method,
        };
        let generation = self.inner.entries.get(&key).and_then(|entry| {
            let lifecycle = entry
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            matches!(
                &lifecycle.activation,
                ActivationState::Active(active) if active == &transaction
            )
            .then_some(entry.generation)
        });
        if let Some(generation) = generation {
            abort_exact(&self.inner, &key, generation);
        }
    }

    #[cfg(test)]
    pub(crate) fn deferred_event_count(&self) -> usize {
        self.inner.deferred_event_count.load(Ordering::Acquire)
    }

    fn reserve_deferred_slot(&self) -> bool {
        self.inner
            .deferred_event_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < self.inner.max_deferred_events).then_some(current + 1)
            })
            .is_ok()
    }

    fn release_deferred_count(&self, count: usize) {
        if count != 0 {
            self.inner
                .deferred_event_count
                .fetch_sub(count, Ordering::AcqRel);
        }
    }

    fn flush_lifecycle_replays(
        &self,
        lifecycle: &mut TrackedRequestLifecycle,
        transaction_id: &TransactionKey,
    ) -> Result<()> {
        let transaction_id = transaction_id.to_string();
        let index = 0;
        while index < lifecycle.deferred.len() {
            if lifecycle.deferred[index].transaction_id() != transaction_id {
                lifecycle.deferred.swap_remove(index);
                self.release_deferred_count(1);
                continue;
            }
            let event = lifecycle.deferred.swap_remove(index);
            match self.inner.deferred_replay_tx.try_send(event) {
                Ok(()) => lifecycle.replays_in_flight += 1,
                Err(mpsc::error::TrySendError::Full(event)) => {
                    lifecycle.deferred.push(event);
                    return Err(SessionError::InternalError(
                        "deferred replay channel capacity invariant violated".to_string(),
                    ));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.release_deferred_count(1);
                    return Err(SessionError::InternalError(
                        "deferred replay channel closed during activation".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn abort_exact(inner: &TrackerInner, key: &TrackedRequestKey, generation: u64) -> bool {
    let removed = inner
        .entries
        .remove_if(key, |_, current| current.generation == generation);
    if let Some((_, entry)) = removed {
        let mut lifecycle = entry
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lifecycle.activation = ActivationState::Aborted;
        let discarded = lifecycle.deferred.len();
        lifecycle.deferred.clear();
        inner
            .deferred_event_count
            .fetch_sub(discarded, Ordering::AcqRel);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_options() -> TrackedInDialogOptions {
        TrackedInDialogOptions::Info(Arc::new(Default::default()))
    }

    fn notify_options() -> TrackedInDialogOptions {
        TrackedInDialogOptions::Notify(Arc::new(Default::default()))
    }

    fn completed_event(
        session: &SessionId,
        transaction: &TransactionKey,
    ) -> DeferredTrackedRequestEvent {
        DeferredTrackedRequestEvent::Completed {
            session_id: session.clone(),
            transaction_id: transaction.to_string(),
            method: "INFO".to_string(),
            outcome:
                rvoip_infra_common::events::cross_crate::OutboundRequestOutcome::FinalResponse {
                    status_code: 200,
                },
        }
    }

    fn auth_event(
        session: &SessionId,
        transaction: &TransactionKey,
    ) -> DeferredTrackedRequestEvent {
        DeferredTrackedRequestEvent::AuthRequired {
            session_id: session.clone(),
            transaction_id: transaction.to_string(),
            request_uri: "sip:bob@example.com".to_string(),
            status: 401,
            challenge: r#"Digest realm="example", nonce="n-1""#.to_string(),
            method: "INFO".to_string(),
            outbound_transport: None,
        }
    }

    #[tokio::test]
    async fn response_before_activation_is_buffered_and_replayed_exactly() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-fast-response".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let transaction =
            TransactionKey::new("z9hG4bK-tracker-fast".to_string(), Method::Info, false);
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(
                &session,
                TrackedInDialogMethod::Info,
                &transaction,
                completed_event(&session, &transaction),
            ),
            ExactTransactionLookup::Prepared
        );
        assert_eq!(tracker.len(), 1);
        tracker.activate(lease, transaction.clone()).unwrap();
        let deferred = replay.recv().await.expect("exact replay was not queued");
        assert_eq!(deferred.transaction_id(), transaction.to_string());
        assert!(tracker.mark_deferred_replay_started(&deferred));
        assert!(tracker.complete_if_matches(&session, TrackedInDialogMethod::Info, &transaction));
        assert_eq!(tracker.len(), 0);
        assert_eq!(tracker.deferred_event_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activation_race_preserves_auth_before_later_completion() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-activation-order".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let transaction =
            TransactionKey::new("z9hG4bK-activation-order".to_string(), Method::Info, false);
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(
                &session,
                TrackedInDialogMethod::Info,
                &transaction,
                auth_event(&session, &transaction),
            ),
            ExactTransactionLookup::Prepared
        );

        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let activate_task = {
            let tracker = tracker.clone();
            let transaction = transaction.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                tracker.activate(lease, transaction)
            })
        };
        let completion_task = {
            let tracker = tracker.clone();
            let session = session.clone();
            let transaction = transaction.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                tracker.correlate_or_defer(
                    &session,
                    TrackedInDialogMethod::Info,
                    &transaction,
                    completed_event(&session, &transaction),
                )
            })
        };
        barrier.wait().await;
        activate_task.await.unwrap().unwrap();
        assert_eq!(
            completion_task.await.unwrap(),
            ExactTransactionLookup::Prepared,
            "a later completion must queue behind the buffered auth replay"
        );

        let first = replay.recv().await.expect("auth replay missing");
        assert!(matches!(
            &first,
            DeferredTrackedRequestEvent::AuthRequired { .. }
        ));
        assert!(tracker.mark_deferred_replay_started(&first));
        let second = replay.recv().await.expect("completion replay missing");
        assert!(matches!(
            &second,
            DeferredTrackedRequestEvent::Completed { .. }
        ));
        assert!(tracker.mark_deferred_replay_started(&second));
        assert_eq!(tracker.deferred_event_count(), 0);
    }

    #[tokio::test]
    async fn lease_drop_aborts_prepared_request() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-drop".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        assert!(tracker.has_request(&session, TrackedInDialogMethod::Info));
        drop(lease);
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_same_method_prepare_admits_exactly_one_owner() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-concurrent".to_string());
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut attempts = Vec::new();
        for _ in 0..2 {
            let tracker = tracker.clone();
            let session = session.clone();
            let barrier = Arc::clone(&barrier);
            attempts.push(tokio::spawn(async move {
                barrier.wait().await;
                tracker.prepare(&session, info_options())
            }));
        }
        barrier.wait().await;
        let first = attempts.remove(0).await.unwrap();
        let second = attempts.remove(0).await.unwrap();
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn activation_rejects_wrong_method_and_server_transaction() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-activation-correlation".to_string());

        let wrong_method = tracker.prepare(&session, info_options()).unwrap();
        assert!(tracker
            .activate(
                wrong_method,
                TransactionKey::new("z9hG4bK-wrong-method".to_string(), Method::Notify, false,),
            )
            .is_err());
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));

        let server_transaction = tracker.prepare(&session, info_options()).unwrap();
        assert!(tracker
            .activate(
                server_transaction,
                TransactionKey::new("z9hG4bK-server-direction".to_string(), Method::Info, true,),
            )
            .is_err());
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));
    }

    #[tokio::test]
    async fn retry_replacement_rejects_interposition_and_stale_generation() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-retry".to_string());
        let initial = tracker.prepare(&session, info_options()).unwrap();
        let challenged = TransactionKey::new(
            "z9hG4bK-tracker-challenged".to_string(),
            Method::Info,
            false,
        );
        tracker.activate(initial, challenged.clone()).unwrap();
        assert_eq!(
            tracker
                .auth_retry_state_for_transaction(
                    &session,
                    TrackedInDialogMethod::Info,
                    &challenged,
                )
                .unwrap(),
            (0, None)
        );

        let (retry, _) = tracker
            .prepare_retry(
                &session,
                TrackedInDialogMethod::Info,
                &challenged,
                Some("info-nonce-1".to_string()),
            )
            .unwrap();
        assert!(tracker.prepare(&session, info_options()).is_err());
        assert!(tracker
            .prepare_retry(
                &session,
                TrackedInDialogMethod::Info,
                &challenged,
                Some("stale-interposition".to_string()),
            )
            .is_err());

        let retry_transaction =
            TransactionKey::new("z9hG4bK-tracker-retry".to_string(), Method::Info, false);
        tracker.activate(retry, retry_transaction.clone()).unwrap();
        assert_eq!(
            tracker
                .auth_retry_state_for_transaction(
                    &session,
                    TrackedInDialogMethod::Info,
                    &retry_transaction,
                )
                .unwrap(),
            (1, Some("info-nonce-1".to_string()))
        );
        assert!(!tracker.complete_if_matches(&session, TrackedInDialogMethod::Info, &challenged,));
        assert!(tracker.complete_if_matches(
            &session,
            TrackedInDialogMethod::Info,
            &retry_transaction,
        ));
    }

    #[tokio::test]
    async fn retry_budgets_are_independent_across_methods() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-independent-auth".to_string());
        let info = tracker.prepare(&session, info_options()).unwrap();
        let notify = tracker.prepare(&session, notify_options()).unwrap();
        let info_transaction =
            TransactionKey::new("z9hG4bK-info-auth".to_string(), Method::Info, false);
        let notify_transaction =
            TransactionKey::new("z9hG4bK-notify-auth".to_string(), Method::Notify, false);
        tracker.activate(info, info_transaction.clone()).unwrap();
        tracker
            .activate(notify, notify_transaction.clone())
            .unwrap();

        let (info_retry, _) = tracker
            .prepare_retry(
                &session,
                TrackedInDialogMethod::Info,
                &info_transaction,
                Some("info-nonce".to_string()),
            )
            .unwrap();
        let info_retry_transaction =
            TransactionKey::new("z9hG4bK-info-auth-retry".to_string(), Method::Info, false);
        tracker
            .activate(info_retry, info_retry_transaction.clone())
            .unwrap();

        let (notify_retry, _) = tracker
            .prepare_retry(
                &session,
                TrackedInDialogMethod::Notify,
                &notify_transaction,
                Some("notify-nonce".to_string()),
            )
            .unwrap();
        let notify_retry_transaction = TransactionKey::new(
            "z9hG4bK-notify-auth-retry".to_string(),
            Method::Notify,
            false,
        );
        tracker
            .activate(notify_retry, notify_retry_transaction.clone())
            .unwrap();

        let info_auth_state = tracker
            .auth_retry_state_for_transaction(
                &session,
                TrackedInDialogMethod::Info,
                &info_retry_transaction,
            )
            .unwrap();
        assert_eq!(info_auth_state, (1, Some("info-nonce".to_string())));
        assert_eq!(
            tracker
                .auth_retry_state_for_transaction(
                    &session,
                    TrackedInDialogMethod::Notify,
                    &notify_retry_transaction,
                )
                .unwrap(),
            (1, Some("notify-nonce".to_string()))
        );

        let stale_info_challenge = rvoip_auth_core::DigestAuthenticator::parse_challenge(
            r#"Digest realm="test", nonce="info-nonce-2", algorithm=MD5, qop="auth""#,
        )
        .unwrap();
        assert!(crate::state_machine::actions::auth_retry_allowed(
            info_auth_state.0,
            1,
            Some(&stale_info_challenge),
            true,
            info_auth_state.1.as_deref(),
        ));
    }

    #[tokio::test]
    async fn auth_int_bodies_are_read_from_exact_tracked_snapshots() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-auth-int-bodies".to_string());
        let cases = [
            (
                TrackedInDialogOptions::Refer(Arc::new(Default::default())),
                Method::Refer,
                bytes::Bytes::new(),
            ),
            (
                TrackedInDialogOptions::Info(Arc::new(
                    rvoip_sip_dialog::api::unified::InfoRequestOptions {
                        content_type: "application/dtmf-relay".to_string(),
                        body: bytes::Bytes::from_static(b"Signal=5\r\nDuration=160\r\n"),
                        ..Default::default()
                    },
                )),
                Method::Info,
                bytes::Bytes::from_static(b"Signal=5\r\nDuration=160\r\n"),
            ),
            (
                TrackedInDialogOptions::Notify(Arc::new(
                    rvoip_sip_dialog::api::unified::NotifyRequestOptions {
                        body: Some(bytes::Bytes::from_static(b"<presence/>")),
                        ..Default::default()
                    },
                )),
                Method::Notify,
                bytes::Bytes::from_static(b"<presence/>"),
            ),
            (
                TrackedInDialogOptions::Update(Arc::new(
                    rvoip_sip_dialog::api::unified::UpdateRequestOptions {
                        sdp: Some("v=0\r\ns=auth-int\r\n".to_string()),
                        ..Default::default()
                    },
                )),
                Method::Update,
                bytes::Bytes::from_static(b"v=0\r\ns=auth-int\r\n"),
            ),
        ];

        for (index, (options, sip_method, expected_body)) in cases.into_iter().enumerate() {
            let tracked_method = options.method();
            let lease = tracker.prepare(&session, options).unwrap();
            let transaction =
                TransactionKey::new(format!("z9hG4bK-auth-int-{index}"), sip_method, false);
            tracker.activate(lease, transaction.clone()).unwrap();
            assert_eq!(
                tracker
                    .request_body_for_transaction(&session, tracked_method, &transaction)
                    .unwrap(),
                Some(expected_body)
            );
            assert!(tracker.complete_if_matches(&session, tracked_method, &transaction));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn active_owner_has_no_speculative_time_expiry() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-slow-connect".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let transaction =
            TransactionKey::new("z9hG4bK-slow-connect".to_string(), Method::Info, false);
        tracker.activate(lease, transaction.clone()).unwrap();
        tokio::time::advance(Duration::from_secs(3_600)).await;
        assert!(tracker.has_request(&session, TrackedInDialogMethod::Info));
        assert!(tracker.complete_if_matches(&session, TrackedInDialogMethod::Info, &transaction));
    }

    #[tokio::test]
    async fn stale_prepared_transaction_is_not_replayed_after_activation() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-stale-prepared".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let stale = TransactionKey::new("z9hG4bK-stale".to_string(), Method::Info, false);
        let active = TransactionKey::new("z9hG4bK-active".to_string(), Method::Info, false);
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(
                &session,
                TrackedInDialogMethod::Info,
                &stale,
                completed_event(&session, &stale),
            ),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, active).unwrap();
        assert!(replay.try_recv().is_err());
        assert_eq!(tracker.deferred_event_count(), 0);
    }

    #[tokio::test]
    async fn clear_session_drops_buffered_replay_without_a_task() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-deferred-clear".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let transaction =
            TransactionKey::new("z9hG4bK-deferred-clear".to_string(), Method::Info, false);
        assert_eq!(
            tracker.correlate_or_defer(
                &session,
                TrackedInDialogMethod::Info,
                &transaction,
                completed_event(&session, &transaction),
            ),
            ExactTransactionLookup::Prepared
        );
        assert_eq!(tracker.deferred_event_count(), 1);
        tracker.clear_session(&session);
        assert_eq!(tracker.deferred_event_count(), 0);
        drop(lease);
    }

    #[tokio::test]
    async fn global_replay_overflow_aborts_exact_prepared_owner() {
        let tracker = OutboundInDialogRequestTracker::with_replay_capacity(1);
        let first_session = SessionId("tracker-cap-first".to_string());
        let first_lease = tracker.prepare(&first_session, info_options()).unwrap();
        let first_transaction =
            TransactionKey::new("z9hG4bK-cap-first".to_string(), Method::Info, false);
        assert_eq!(
            tracker.correlate_or_defer(
                &first_session,
                TrackedInDialogMethod::Info,
                &first_transaction,
                completed_event(&first_session, &first_transaction),
            ),
            ExactTransactionLookup::Prepared
        );

        let overflow_session = SessionId("tracker-cap-overflow".to_string());
        let overflow_lease = tracker.prepare(&overflow_session, info_options()).unwrap();
        let overflow_transaction =
            TransactionKey::new("z9hG4bK-cap-overflow".to_string(), Method::Info, false);
        assert_eq!(
            tracker.correlate_or_defer(
                &overflow_session,
                TrackedInDialogMethod::Info,
                &overflow_transaction,
                completed_event(&overflow_session, &overflow_transaction),
            ),
            ExactTransactionLookup::Rejected
        );
        assert!(!tracker.has_request(&overflow_session, TrackedInDialogMethod::Info));
        assert!(tracker
            .activate(overflow_lease, overflow_transaction)
            .is_err());

        tracker.clear_session(&first_session);
        drop(first_lease);
        assert_eq!(tracker.deferred_event_count(), 0);
        let replacement = tracker.prepare(&overflow_session, info_options()).unwrap();
        drop(replacement);
    }

    #[tokio::test]
    async fn replay_enqueue_failure_aborts_owner_and_releases_payload() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("tracker-replay-enqueue-failure".to_string());
        let lease = tracker.prepare(&session, info_options()).unwrap();
        let transaction = TransactionKey::new(
            "z9hG4bK-replay-enqueue-failure".to_string(),
            Method::Info,
            false,
        );
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(
                &session,
                TrackedInDialogMethod::Info,
                &transaction,
                completed_event(&session, &transaction),
            ),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, transaction).unwrap();
        let deferred = replay.recv().await.expect("deferred replay missing");
        tracker.abort_deferred_replay(&deferred);
        assert_eq!(tracker.deferred_event_count(), 0);
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));
    }
}
