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
use crate::state_table::types::SessionId;
use crate::types::CallState;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

const TERMINAL_EVENT_TTL: Duration = Duration::from_secs(60);
const MAX_PROGRESS_EVENTS: usize = 8;

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Provisional call-progress evidence observed for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAnsweredInfo {
    /// Session identifier for the answered call.
    pub call_id: SessionId,
    /// SDP body from the answer, if present.
    pub sdp: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
struct LifecycleEntry {
    progress: VecDeque<CallProgressInfo>,
    answered: Option<CallAnsweredInfo>,
    media_security: Option<MediaSecurityState>,
    terminal: Option<(CallTerminalInfo, Instant)>,
    latest_transfer_outcome: Option<TransferOutcome>,
}

impl Default for LifecycleEntry {
    fn default() -> Self {
        Self {
            progress: VecDeque::with_capacity(MAX_PROGRESS_EVENTS),
            answered: None,
            media_security: None,
            terminal: None,
            latest_transfer_outcome: None,
        }
    }
}

impl LifecycleEntry {
    fn drop_retained_media_payloads(&mut self) {
        for progress in &mut self.progress {
            progress.sdp = None;
        }
        if let Some(answered) = &mut self.answered {
            answered.sdp = None;
        }
    }

    #[cfg(feature = "perf-tests")]
    fn retained_sdp_bytes(&self) -> usize {
        let progress_bytes: usize = self
            .progress
            .iter()
            .filter_map(|progress| progress.sdp.as_ref())
            .map(String::len)
            .sum();
        let answered_bytes = self
            .answered
            .as_ref()
            .and_then(|answered| answered.sdp.as_ref())
            .map(String::len)
            .unwrap_or(0);
        progress_bytes + answered_bytes
    }
}

/// Internal lifecycle index keyed by session id.
#[derive(Debug, Clone, Default)]
pub(crate) struct LifecycleIndex {
    entries: Arc<DashMap<SessionId, LifecycleEntry>>,
    waiters: Arc<DashMap<SessionId, watch::Sender<u64>>>,
    last_prune_unix_ms: Arc<AtomicU64>,
    pruner_started: Arc<AtomicBool>,
}

impl LifecycleIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Arc::new(DashMap::with_capacity(capacity)),
            waiters: Arc::new(DashMap::with_capacity(capacity)),
            last_prune_unix_ms: Arc::new(AtomicU64::new(0)),
            pruner_started: Arc::new(AtomicBool::new(false)),
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
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(entries) = entries.upgrade() else {
                    break;
                };
                let Some(waiters) = waiters.upgrade() else {
                    break;
                };
                prune_expired_terminal_entries_from(&entries, &waiters);
            }
        });
    }

    pub(crate) fn record_event(&self, event: &Event) {
        self.prune_expired_terminal_entries_throttled();

        let Some(call_id) = event.call_id().cloned() else {
            return;
        };

        #[cfg(feature = "perf-tests")]
        let lifecycle_entry_was_new = !self.entries.contains_key(&call_id);
        let mut entry = self.entries.entry(call_id.clone()).or_default();
        #[cfg(feature = "perf-tests")]
        if lifecycle_entry_was_new {
            rvoip_infra_common::memory_diagnostics::record_created(
                "sip.lifecycle.entry",
                std::mem::size_of::<LifecycleEntry>(),
            );
        }

        if let Some(progress) = CallProgressInfo::from_event(event) {
            if entry.progress.len() == MAX_PROGRESS_EVENTS {
                entry.progress.pop_front();
            }
            entry.progress.push_back(progress);
        }

        if let Some(answered) = CallAnsweredInfo::from_event(event) {
            rvoip_sip_dialog::diagnostics::record_call_timing_lifecycle_call_answered(
                call_id.as_str(),
            );
            entry.answered = Some(answered);
        }

        if let Event::MediaSecurityNegotiated {
            keying,
            suite,
            profile,
            contexts_installed,
            ..
        } = event
        {
            entry.media_security = Some(MediaSecurityState {
                keying: *keying,
                suite: *suite,
                profile: *profile,
                contexts_installed: *contexts_installed,
            });
        }

        if let Ok(outcome) = TransferOutcome::try_from(event.clone()) {
            entry.latest_transfer_outcome = Some(outcome);
        }

        let is_terminal = if let Some((_, terminal)) = CallTerminalInfo::from_event(event) {
            entry.drop_retained_media_payloads();
            entry.terminal = Some((terminal, Instant::now()));
            true
        } else {
            false
        };

        self.notify_waiters(&call_id, is_terminal);
    }

    fn prune_expired_terminal_entries_throttled(&self) {
        const PRUNE_INTERVAL_MS: u64 = 1_000;

        let now = unix_time_millis();
        let last = self.last_prune_unix_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < PRUNE_INTERVAL_MS {
            return;
        }
        if self
            .last_prune_unix_ms
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.prune_expired_terminal_entries();
        }
    }

    fn prune_expired_terminal_entries(&self) -> usize {
        prune_expired_terminal_entries_from(&self.entries, &self.waiters)
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
        for entry in self.entries.iter() {
            progress_events += entry.value().progress.len() as u64;
            if entry.value().answered.is_some() {
                answered_entries += 1;
            }
            retained_sdp_bytes += entry.value().retained_sdp_bytes() as u64;
            if let Some((_, stored_at)) = &entry.value().terminal {
                terminal_entries += 1;
                if stored_at.elapsed() > TERMINAL_EVENT_TTL {
                    expired_terminal_entries += 1;
                }
            }
        }

        serde_json::json!({
            "entries": self.entries.len(),
            "waiters": self.waiters.len(),
            "terminal_entries": terminal_entries,
            "expired_terminal_entries": expired_terminal_entries,
            "progress_events": progress_events,
            "answered_entries": answered_entries,
            "retained_sdp_bytes": retained_sdp_bytes,
            "terminal_ttl_secs": TERMINAL_EVENT_TTL.as_secs(),
            "pruned_expired_terminal_entries": pruned_expired_terminal_entries,
        })
    }

    pub(crate) fn watcher(&self, call_id: &SessionId) -> watch::Receiver<u64> {
        #[cfg(feature = "perf-tests")]
        let waiter_was_new = !self.waiters.contains_key(call_id);
        let receiver = self
            .waiters
            .entry(call_id.clone())
            .or_insert_with(|| {
                let (tx, _) = watch::channel(0);
                tx
            })
            .subscribe();
        #[cfg(feature = "perf-tests")]
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

        if terminal {
            if self.waiters.remove(call_id).is_some() {
                #[cfg(feature = "perf-tests")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.lifecycle.waiter",
                    std::mem::size_of::<watch::Sender<u64>>(),
                );
            }
        }
    }

    pub(crate) fn snapshot(
        &self,
        call_id: &SessionId,
        state: Option<CallState>,
    ) -> CallLifecycleSnapshot {
        let mut terminal_expired = false;
        let snapshot = if let Some(entry) = self.entries.get(call_id) {
            let terminal = entry.terminal.as_ref().and_then(|(terminal, stored_at)| {
                if stored_at.elapsed() <= TERMINAL_EVENT_TTL {
                    Some(terminal.clone())
                } else {
                    terminal_expired = true;
                    None
                }
            });
            CallLifecycleSnapshot {
                call_id: call_id.clone(),
                state,
                progress: entry.progress.iter().cloned().collect(),
                answered: entry.answered.clone(),
                media_security: entry.media_security.clone(),
                terminal,
                latest_transfer_outcome: entry.latest_transfer_outcome.clone(),
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

        if terminal_expired {
            if self.entries.remove(call_id).is_some() {
                #[cfg(feature = "perf-tests")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.lifecycle.entry",
                    std::mem::size_of::<LifecycleEntry>(),
                );
            }
            if self.waiters.remove(call_id).is_some() {
                #[cfg(feature = "perf-tests")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "sip.lifecycle.waiter",
                    std::mem::size_of::<watch::Sender<u64>>(),
                );
            }
        }

        snapshot
    }
}

fn prune_expired_terminal_entries_from(
    entries: &DashMap<SessionId, LifecycleEntry>,
    waiters: &DashMap<SessionId, watch::Sender<u64>>,
) -> usize {
    let expired: Vec<SessionId> = entries
        .iter()
        .filter_map(|entry| {
            entry.value().terminal.as_ref().and_then(|(_, stored_at)| {
                if stored_at.elapsed() > TERMINAL_EVENT_TTL {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
        })
        .collect();
    let removed = expired.len();
    for call_id in expired {
        if entries.remove(&call_id).is_some() {
            #[cfg(feature = "perf-tests")]
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.lifecycle.entry",
                std::mem::size_of::<LifecycleEntry>(),
            );
        }
        if waiters.remove(&call_id).is_some() {
            #[cfg(feature = "perf-tests")]
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "sip.lifecycle.waiter",
                std::mem::size_of::<watch::Sender<u64>>(),
            );
        }
    }
    removed
}

#[derive(Clone)]
struct SessionEventDispatcher {
    workers: Arc<Vec<mpsc::Sender<Arc<SessionApiCrossCrateEvent>>>>,
    next_worker: Arc<AtomicUsize>,
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

        for _ in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<Arc<SessionApiCrossCrateEvent>>(channel_capacity);
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let stage = cleanup_stage_for_event(&event.event);
                    let label = cleanup_label_for_event(&event.event);
                    let guard = cleanup_diag::stage_guard(stage, label);
                    match coordinator.publish(event).await {
                        Ok(()) => guard.finish_success(),
                        Err(e) => {
                            guard.finish_failure();
                            tracing::warn!("Failed to publish app-level event: {}", e);
                        }
                    }
                }
            });
            workers.push(tx);
        }

        Self {
            workers: Arc::new(workers),
            next_worker: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn publish(&self, event: Arc<SessionApiCrossCrateEvent>) {
        let idx = self.worker_index(&event.event);
        let tx = self.workers[idx].clone();
        cleanup_diag::record_queue_depth(
            cleanup_stage_for_event(&event.event),
            tx.max_capacity().saturating_sub(tx.capacity()),
        );
        match tx.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(event)) => {
                tokio::spawn(async move {
                    cleanup_diag::record_queue_depth(
                        cleanup_stage_for_event(&event.event),
                        tx.max_capacity(),
                    );
                    if tx.send(event).await.is_err() {
                        tracing::warn!("Session event dispatcher closed while publishing event");
                    }
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("Session event dispatcher closed while publishing event");
            }
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

/// Publishes app-level session events and updates lifecycle first.
#[derive(Clone)]
pub(crate) struct SessionEventPublisher {
    coordinator: Arc<GlobalEventCoordinator>,
    lifecycle: LifecycleIndex,
    dispatcher: SessionEventDispatcher,
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
            coordinator,
            lifecycle,
            dispatcher,
        }
    }

    pub(crate) fn publish(&self, event: Event) {
        self.lifecycle.record_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        self.dispatcher.publish(wrapped);
    }

    pub(crate) async fn publish_now(&self, event: Event) -> Result<()> {
        self.lifecycle.record_event(&event);
        let stage = cleanup_stage_for_event(&event);
        let label = cleanup_label_for_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        let guard = cleanup_diag::stage_guard(stage, label);
        match self.coordinator.publish(wrapped).await {
            Ok(()) => {
                guard.finish_success();
                Ok(())
            }
            Err(e) => {
                guard.finish_failure();
                Err(SessionError::Other(format!(
                    "Failed to publish app-level event: {}",
                    e
                )))
            }
        }
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
        index.record_event(&Event::CallCancelled {
            call_id: call_id.clone(),
        });

        let snapshot = index.snapshot(&call_id, Some(CallState::Cancelled));
        assert_eq!(snapshot.progress.len(), 1);
        assert_eq!(snapshot.progress[0].status_code, 183);
        assert_eq!(snapshot.progress[0].sdp, None);
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
