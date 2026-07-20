//! Session Event Handler - Central hub for ALL cross-crate event handling
//!
//! This is the ONLY place where cross-crate events are handled.
//! - Receives events from dialog-core and media-core
//! - Routes them to the state machine
//! - Publishes events to dialog-core and media-core
//!
//! NO OTHER MODULE should interact with the GlobalEventCoordinator directly.

use crate::adapters::outbound_request_tracker::{
    DeferredTrackedRequestEvent, ExactTransactionLookup, OutboundInDialogRequestTracker,
    TrackedInDialogMethod,
};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::api::lifecycle::{
    ExactTerminalClaim, ExactTerminalCompletion, LifecycleIndex, SessionEventPublisher,
};
use crate::cleanup_diag::{self, CleanupStage};
use crate::errors::{Result as SessionResult, SessionError};
use crate::session_lifecycle::SessionAdmissionError;
use crate::session_registry::{PendingInboundBundle, SessionRegistry, SessionRegistryHandle};
use crate::state_machine::{ProcessEventResult, StateMachine as StateMachineExecutor};
use crate::state_table::types::{EventTemplate, EventType, Role, SessionId};
use crate::types::{CallState, DialogId};
use anyhow::Result;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::{
    CrossCrateEvent, DialogToSessionEvent, MediaToSessionEvent, OutboundRequestOutcome,
    RvoipCrossCrateEvent, SipTraceEvent,
};
use rvoip_infra_common::planes::routing::RoutableEvent;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

const STATE_MACHINE_DISPATCH_JOIN_FAILURE: &str =
    "SIP state-machine dispatch task failed (class=join)";

type StateMachineProcessResult =
    std::result::Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>>;

/// Owns a state-machine task that performs outbound signaling.
///
/// Dropping Tokio's `JoinHandle` detaches the task. Keep an armed owner around
/// the await instead so dispatcher shutdown or cancellation also cancels the
/// state-machine task and its signaling work.
struct AbortStateMachineTaskOnDrop {
    handle: tokio::task::JoinHandle<StateMachineProcessResult>,
    armed: bool,
}

impl AbortStateMachineTaskOnDrop {
    fn new(handle: tokio::task::JoinHandle<StateMachineProcessResult>) -> Self {
        Self {
            handle,
            armed: true,
        }
    }

    async fn join(
        mut self,
    ) -> std::result::Result<StateMachineProcessResult, tokio::task::JoinError> {
        let result = (&mut self.handle).await;
        self.armed = false;
        result
    }
}

impl Drop for AbortStateMachineTaskOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.handle.abort();
        }
    }
}

async fn join_state_machine_task(task: AbortStateMachineTaskOnDrop) -> StateMachineProcessResult {
    task.join().await.map_err(|_| {
        Box::new(SessionError::InternalError(
            STATE_MACHINE_DISPATCH_JOIN_FAILURE.to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

/// Poll a signaling state-machine future from the root of a fresh Tokio task
/// while retaining strict per-session ordering in the caller.
///
/// Dialog events already enter through a sharded worker, but that worker polls
/// the complete cross-crate handler before it reaches `process_event`. The
/// resulting transport → dialog → session → state-machine → outbound signaling
/// poll chain can exhaust the default stack. Awaiting this owned child task lets
/// the parent poll unwind before the signaling action is polled; it does not
/// detach the action or weaken completion/error semantics.
async fn process_event_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    session_id: SessionId,
    event: EventType,
) -> StateMachineProcessResult {
    let task_session_id = session_id.clone();
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine.process_event(&task_session_id, event).await
    }));
    join_state_machine_task(task).await
}

fn is_missing_credentials_for_auth_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    matches!(
        error.downcast_ref::<SessionError>(),
        Some(SessionError::MissingCredentialsForInviteAuth)
            | Some(SessionError::MissingCredentialsForRequestAuth { .. })
    )
}

fn is_session_lifecycle_capacity_exhaustion(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    matches!(
        error.downcast_ref::<SessionAdmissionError>(),
        Some(
            SessionAdmissionError::CapacityExhausted
                | SessionAdmissionError::RetainedCapacityExhausted
        )
    )
}

fn safe_auth_method_label(method: &str) -> &'static str {
    match method.trim().to_ascii_uppercase().as_str() {
        "INVITE" => "INVITE",
        "REGISTER" => "REGISTER",
        "BYE" => "BYE",
        "REFER" => "REFER",
        "NOTIFY" => "NOTIFY",
        "INFO" => "INFO",
        "UPDATE" => "UPDATE",
        "MESSAGE" => "MESSAGE",
        "OPTIONS" => "OPTIONS",
        "SUBSCRIBE" => "SUBSCRIBE",
        _ => "extension",
    }
}

fn outbound_request_outcome_label(outcome: OutboundRequestOutcome) -> &'static str {
    match outcome {
        OutboundRequestOutcome::FinalResponse { .. } => "final-response",
        OutboundRequestOutcome::Timeout => "timeout",
        OutboundRequestOutcome::TransportFailure => "transport-failure",
    }
}

fn clear_exact_tracked_auth_fields(
    session: &mut crate::session_store::SessionState,
    transaction_id: &str,
) {
    if session.pending_auth_transaction_id.as_deref() != Some(transaction_id) {
        return;
    }
    session.pending_auth = None;
    session.pending_auth_method = None;
    session.pending_auth_transport = None;
    session.pending_auth_transaction_id = None;
    session.pending_auth_request_uri = None;
    session.auth_challenge = None;
    session.auth_challenge_raw = None;
    session.auth_challenge_stale = false;
    session.auth_challenge_replaces_nonce = None;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboundAuthTerminalClass {
    MissingCredentials,
    ChallengeResponse,
    RetryLimit,
    StateMachine,
}

impl OutboundAuthTerminalClass {
    fn from_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Self {
        match error.downcast_ref::<SessionError>() {
            Some(
                SessionError::MissingCredentialsForInviteAuth
                | SessionError::MissingCredentialsForRequestAuth { .. },
            ) => Self::MissingCredentials,
            Some(
                SessionError::InviteAuthConstructionFailed
                | SessionError::RequestAuthConstructionFailed
                | SessionError::RegisterAuthConstructionFailed
                | SessionError::AuthError(_),
            ) => Self::ChallengeResponse,
            Some(
                SessionError::InviteAuthRetryExhausted
                | SessionError::RequestAuthRetryExhausted { .. },
            ) => Self::RetryLimit,
            _ => Self::StateMachine,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MissingCredentials => "missing-credentials",
            Self::ChallengeResponse => "challenge-response",
            Self::RetryLimit => "retry-limit",
            Self::StateMachine => "state-machine",
        }
    }

    fn invite_reason(self) -> String {
        format!("INVITE authentication failed (class={})", self.label())
    }
}

enum CallFailureReason {
    Protocol(String),
    OutboundInviteAuth(OutboundAuthTerminalClass),
}

impl CallFailureReason {
    fn into_event_reason(self) -> String {
        match self {
            Self::Protocol(reason) => reason,
            Self::OutboundInviteAuth(class) => class.invite_reason(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CallFailureDiagnostics {
    reason_present: bool,
    reason_bytes: usize,
}

impl CallFailureDiagnostics {
    fn new(reason: &str) -> Self {
        Self {
            reason_present: !reason.is_empty(),
            reason_bytes: reason.len(),
        }
    }
}

/// Window within which repeated RFC 5626 flow-failure events for the
/// same AoR collapse to a single re-REGISTER. Matches the guidance in
/// RFC 5626 §4.4.1 (flow recovery should not storm the registrar).
const OUTBOUND_FLOW_REFRESH_DEBOUNCE: Duration = Duration::from_secs(1);

fn transaction_id_diagnostics(value: &str) -> (&'static str, usize) {
    let class = if value
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .is_ok()
    {
        "valid"
    } else {
        "invalid"
    };
    (class, value.len())
}

fn sip_trace_owner_matches(configured_owner_id: Option<&str>, event_owner_id: &str) -> bool {
    configured_owner_id.is_some_and(|owner_id| owner_id == event_owner_id)
}

fn map_sip_trace_session_id(
    event: &SipTraceEvent,
    callid_to_session: &DashMap<String, SessionId>,
) -> Option<SessionId> {
    event
        .session_id
        .as_ref()
        .map(|id| SessionId(id.clone()))
        .or_else(|| {
            event.sip_call_id.as_ref().and_then(|sip_call_id| {
                callid_to_session
                    .get(sip_call_id)
                    .map(|entry| entry.value().clone())
            })
        })
}

fn dialog_dispatch_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(4))
        .unwrap_or(16)
        .clamp(8, 64)
}

fn session_dispatch_shard(session_id: &str, shard_count: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    (hasher.finish() as usize) % shard_count.max(1)
}

fn shutdown_change_requests_stop(
    changed: std::result::Result<(), tokio::sync::watch::error::RecvError>,
    receiver: &tokio::sync::watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *receiver.borrow()
}

fn dialog_bye_requires_terminal_release(result: &ProcessEventResult) -> bool {
    result.transition.is_none()
}

fn should_publish_call_cancelled(
    role: Role,
    initial_state: CallState,
    state_machine_published_cancelled: bool,
) -> bool {
    if state_machine_published_cancelled {
        return true;
    }
    if initial_state.is_final() {
        return false;
    }
    match role {
        // The UAC path is cancellation only while local cancel intent is
        // pending or already on the wire. In particular, the response event
        // hub and the explicit transaction coordination event can both report
        // the same 487; after the first one commits Cancelled, the second must
        // not publish or release again.
        Role::UAC => matches!(
            initial_state,
            CallState::CancelPending | CallState::Cancelling
        ),
        // Dialog-core already sends 200(CANCEL)+487(INVITE) for inbound
        // cancellation. Its upper notification therefore has no matching
        // Dialog487 UAS table row, but still must surface CallCancelled.
        Role::UAS => true,
        Role::Both => false,
    }
}

enum QueuedDialogPayload {
    CrossCrate(Arc<dyn CrossCrateEvent>),
    Deferred(DeferredReplayDelivery),
}

struct DeferredReplayDelivery {
    tracker: OutboundInDialogRequestTracker,
    event: Option<DeferredTrackedRequestEvent>,
}

struct StartedDeferredReplay {
    tracker: OutboundInDialogRequestTracker,
    event: DeferredTrackedRequestEvent,
}

impl StartedDeferredReplay {
    fn event(&self) -> &DeferredTrackedRequestEvent {
        &self.event
    }
}

impl Drop for StartedDeferredReplay {
    fn drop(&mut self) {
        self.tracker.abort_started_replay(&self.event);
    }
}

impl DeferredReplayDelivery {
    fn new(tracker: OutboundInDialogRequestTracker, event: DeferredTrackedRequestEvent) -> Self {
        Self {
            tracker,
            event: Some(event),
        }
    }

    fn begin(mut self) -> Option<StartedDeferredReplay> {
        let event = self.event.as_ref()?;
        let accepted = self.tracker.mark_deferred_replay_started(event);
        let event = self.event.take();
        accepted.then(|| StartedDeferredReplay {
            tracker: self.tracker.clone(),
            event: event.expect("accepted delivery retains its event"),
        })
    }
}

impl Drop for DeferredReplayDelivery {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            self.tracker.abort_deferred_replay(&event);
        }
    }
}

struct QueuedDialogToSessionEvent {
    payload: QueuedDialogPayload,
    queued_at: Instant,
    kind: &'static str,
    route_key: Option<String>,
    authoritative_completion: Option<oneshot::Sender<std::result::Result<(), String>>>,
}

struct ServerCallAdmissionGuard {
    pending: Arc<AtomicUsize>,
}

impl Drop for ServerCallAdmissionGuard {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::Relaxed);
    }
}

enum ServerCallAdmissionDecision {
    Admit(Option<ServerCallAdmissionGuard>),
    Reject {
        observed_sessions: usize,
        hard_limit: usize,
    },
}

#[derive(Clone)]
struct DialogToSessionDirectRouter {
    shard_senders: Arc<Vec<mpsc::Sender<QueuedDialogToSessionEvent>>>,
    fallback_shard: Arc<AtomicUsize>,
    deferred_tracker: OutboundInDialogRequestTracker,
}

impl DialogToSessionDirectRouter {
    fn new(
        handler: SessionCrossCrateEventHandler,
        worker_count: usize,
        queue_capacity: usize,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        let deferred_tracker = handler.dialog_adapter.outbound_request_tracker.clone();
        let per_shard_capacity = (queue_capacity / worker_count.max(1)).max(1);
        let mut shard_senders = Vec::with_capacity(worker_count);

        for shard in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<QueuedDialogToSessionEvent>(per_shard_capacity);
            let handler_for_shard = handler.clone();
            let mut shutdown = shutdown_rx.clone();
            tokio::spawn(async move {
                let mut draining = false;
                loop {
                    tokio::select! {
                        biased;
                        changed = shutdown.changed(), if !draining => {
                            if shutdown_change_requests_stop(changed, &shutdown) {
                                info!(
                                    shard,
                                    queued = rx.len(),
                                    "🔔 [session_event_handler] Direct dialog-to-session shard draining"
                                );
                                // Reject new enqueue operations, then run every
                                // accepted envelope to completion. In particular,
                                // an authoritative ByeReceived acknowledgment may
                                // never be discarded at the shutdown boundary.
                                rx.close();
                                draining = true;
                            }
                        }
                        queued = rx.recv() => {
                            let Some(queued) = queued else { break };
                            let QueuedDialogToSessionEvent {
                                payload,
                                queued_at,
                                kind,
                                route_key,
                                authoritative_completion,
                            } = queued;
                            let queue_delay = queued_at.elapsed();
                            cleanup_diag::record_queue_depth(
                                CleanupStage::SessionEventDispatch,
                                rx.len(),
                            );
                            rvoip_sip_dialog::diagnostics::record_dialog_to_session_queue_delay(
                                kind,
                                queue_delay,
                            );

                            let label = route_key
                                .as_deref()
                                .unwrap_or(kind);
                            let dispatch_guard =
                                cleanup_diag::stage_guard(CleanupStage::SessionEventDispatch, label);
                            let result = match payload {
                                QueuedDialogPayload::CrossCrate(event) => {
                                    handler_for_shard.handle(event).await
                                }
                                QueuedDialogPayload::Deferred(delivery) => {
                                    match delivery.begin() {
                                        Some(started) => {
                                            let event = started.event().clone();
                                            let result = handler_for_shard
                                                .handle_deferred_tracked_request(event)
                                                .await;
                                            drop(started);
                                            result
                                        }
                                        None => Ok(()),
                                    }
                                }
                            };
                            if let Some(completion) = authoritative_completion {
                                let acknowledgement = result
                                    .as_ref()
                                    .map(|_| ())
                                    .map_err(ToString::to_string);
                                // Cancellation of the publisher after enqueue
                                // does not cancel processing of the accepted
                                // event; it only means nobody awaits this ACK.
                                let _ = completion.send(acknowledgement);
                            }
                            match result {
                                Ok(()) => dispatch_guard.finish_success(),
                                Err(e) => {
                                    error!(
                                        shard,
                                        kind,
                                        "Error handling direct dialog-to-session event: {}",
                                        e
                                    );
                                    dispatch_guard.finish_failure();
                                }
                            }
                        }
                    }
                    if draining && rx.is_empty() {
                        break;
                    }
                }
            });
            shard_senders.push(tx);
        }

        info!(
            workers = worker_count,
            per_shard_capacity,
            "🔔 [session_event_handler] Registered direct dialog-to-session dispatcher"
        );

        Self {
            shard_senders: Arc::new(shard_senders),
            fallback_shard: Arc::new(AtomicUsize::new(0)),
            deferred_tracker,
        }
    }

    fn shard_for(&self, route_key: Option<&str>) -> usize {
        match route_key {
            Some(session_id) => session_dispatch_shard(session_id, self.shard_senders.len()),
            None => self.fallback_shard.fetch_add(1, Ordering::Relaxed) % self.shard_senders.len(),
        }
    }

    async fn enqueue(&self, queued: QueuedDialogToSessionEvent) -> Result<()> {
        let shard = self.shard_for(queued.route_key.as_deref());
        match self.shard_senders[shard].try_send(queued) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(queued)) => {
                warn!(
                    shard,
                    kind = queued.kind,
                    route_key = queued.route_key.as_deref().unwrap_or("<none>"),
                    "Direct dialog-to-session shard is full; applying backpressure"
                );
                cleanup_diag::record_queue_depth(
                    CleanupStage::SessionEventDispatch,
                    self.shard_senders[shard].max_capacity(),
                );
                self.shard_senders[shard]
                    .send(queued)
                    .await
                    .map_err(|e| anyhow::anyhow!("dialog-to-session shard closed: {}", e))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(anyhow::anyhow!("dialog-to-session shard is closed"))
            }
        }
    }

    fn enqueue_authoritative(&self, queued: QueuedDialogToSessionEvent) -> Result<()> {
        let shard = self.shard_for(queued.route_key.as_deref());
        self.shard_senders[shard]
            .try_send(queued)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    anyhow::anyhow!("authoritative dialog-to-session shard is full")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    anyhow::anyhow!("authoritative dialog-to-session shard is closed")
                }
            })
    }

    async fn handle_deferred(&self, event: DeferredTrackedRequestEvent) -> Result<()> {
        let route_key = Some(event.session_id().0.clone());
        let kind = match &event {
            DeferredTrackedRequestEvent::AuthRequired { .. } => "deferred_auth_required",
            DeferredTrackedRequestEvent::Completed { .. } => "deferred_request_completed",
        };
        self.enqueue(QueuedDialogToSessionEvent {
            payload: QueuedDialogPayload::Deferred(DeferredReplayDelivery::new(
                self.deferred_tracker.clone(),
                event,
            )),
            queued_at: Instant::now(),
            kind,
            route_key,
            authoritative_completion: None,
        })
        .await
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for DialogToSessionDirectRouter {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        let kind = dialog_to_session_event_kind(&event);
        let route_key = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .and_then(RoutableEvent::session_id)
            .map(ToOwned::to_owned);
        let authoritative = matches!(kind, "bye_received" | "info_received");
        let (authoritative_completion, acknowledgement) = if authoritative {
            let (completion, acknowledgement) = oneshot::channel();
            (Some(completion), Some(acknowledgement))
        } else {
            (None, None)
        };
        let queued = QueuedDialogToSessionEvent {
            payload: QueuedDialogPayload::CrossCrate(event),
            queued_at: Instant::now(),
            kind,
            route_key,
            authoritative_completion,
        };
        if kind == "info_received" {
            self.enqueue_authoritative(queued)?;
        } else {
            self.enqueue(queued).await?;
        }
        let Some(acknowledgement) = acknowledgement else {
            return Ok(());
        };
        match acknowledgement.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(anyhow::anyhow!(error)),
            Err(_) => Err(anyhow::anyhow!(
                "authoritative dialog-to-session shard closed before acknowledgment"
            )),
        }
    }
}

fn dialog_to_session_event_kind(event: &Arc<dyn CrossCrateEvent>) -> &'static str {
    match event.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingCall {
            ..
        })) => "incoming_call",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::AckReceived {
            ..
        })) => "ack_received",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::ByeReceived {
            ..
        })) => "bye_received",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::InfoReceived {
            ..
        })) => "info_received",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallTerminated {
            ..
        })) => "call_terminated",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallFailed {
            ..
        })) => "call_failed",
        Some(RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallCancelled {
            ..
        })) => "call_cancelled",
        Some(RvoipCrossCrateEvent::DialogToSession(_)) => "dialog_to_session_other",
        _ => "non_dialog_to_session",
    }
}

/// Handler for processing cross-crate events in rvoip-sip
#[derive(Clone)]
#[allow(dead_code)]
pub struct SessionCrossCrateEventHandler {
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,

    /// Global event coordinator
    global_coordinator: Arc<GlobalEventCoordinator>,

    /// Dialog adapter for setting up backward compatibility channels
    dialog_adapter: Arc<DialogAdapter>,

    /// Media adapter for setting up backward compatibility channels
    media_adapter: Arc<MediaAdapter>,

    /// Session registry for mappings
    registry: Arc<SessionRegistry>,

    /// Channel to send incoming call notifications
    incoming_call_tx: Option<mpsc::Sender<crate::types::IncomingCallInfo>>,

    /// Immediately accept inbound calls after the state machine records them.
    fast_auto_accept_incoming_calls: bool,

    /// Config-owned cap for server-side inbound call admission.
    server_call_admission_limit: Option<usize>,

    /// Soft threshold where server-side admission starts pacing.
    server_call_admission_soft_limit: Option<usize>,

    /// Delay used while above the soft threshold and below hard overload.
    server_call_admission_pacing_delay_ms: Option<u64>,

    /// Retry-After seconds for SIP overload rejections.
    server_overload_retry_after_secs: Option<u32>,

    /// Hysteresis state: once hard overload is reached, reject until below soft.
    server_call_admission_overloaded: Arc<AtomicBool>,

    /// Inbound INVITEs admitted but not yet inserted into the session store.
    server_call_admission_pending: Arc<AtomicUsize>,

    /// Serializes admission check/reserve so the hard limit is meaningful with
    /// multiple dialog-to-session workers.
    server_call_admission_lock: Arc<Mutex<()>>,

    /// Total capacity for the direct dialog-to-session dispatcher queues.
    dialog_event_dispatch_queue_capacity: usize,

    /// Internal state-machine event stream owned by rvoip-sip.
    state_machine_event_rx:
        Option<Arc<Mutex<mpsc::Receiver<crate::state_machine::executor::SessionEvent>>>>,

    /// Last RFC 5626 `OutboundFlowFailed`-driven refresh per AoR, used
    /// to debounce storms of pong-timeout / connection-closed events
    /// (multiple transport signals can observe the same underlying
    /// failure within a handful of milliseconds). Entries live
    /// indefinitely — this map grows with the number of unique AoRs
    /// the peer has ever registered, which in practice is 1.
    outbound_flow_last_refresh: Arc<DashMap<String, Instant>>,

    /// App-level event publisher. Updates lifecycle before global bus delivery.
    app_event_publisher: SessionEventPublisher,

    /// Optional owner id for SIP trace events emitted by this coordinator's transport stack.
    sip_trace_owner_id: Option<String>,

    /// SIP_API_DESIGN_2 Phase D — weak handle back to the
    /// `UnifiedCoordinator` so the bus-path `IncomingRegister`
    /// construction can supply `RegisterResponseBuilder` with the
    /// coordinator hook it needs to publish responses back to
    /// dialog-core. `Weak` breaks the circular ownership
    /// (coordinator -> handler -> coordinator). Populated after
    /// construction via [`Self::set_coordinator`]; cloning the handler
    /// shares the underlying once-cell.
    coordinator: Arc<std::sync::OnceLock<std::sync::Weak<crate::api::unified::UnifiedCoordinator>>>,
}

#[allow(dead_code)]
impl SessionCrossCrateEventHandler {
    async fn handle_deferred_tracked_request(
        &self,
        event: DeferredTrackedRequestEvent,
    ) -> Result<()> {
        match event {
            DeferredTrackedRequestEvent::AuthRequired {
                session_id,
                transaction_id,
                request_uri,
                status,
                challenge,
                method,
                outbound_transport,
            } => {
                self.handle_auth_required_parts(
                    session_id,
                    transaction_id,
                    request_uri,
                    status,
                    challenge,
                    method,
                    outbound_transport,
                    true,
                )
                .await
            }
            DeferredTrackedRequestEvent::Completed {
                session_id,
                transaction_id,
                method,
                outcome,
            } => {
                self.handle_outbound_request_completed_parts(
                    session_id,
                    &transaction_id,
                    &method,
                    outcome,
                    true,
                )
                .await
            }
        }
    }

    async fn handle_dialog_to_session_event(&self, event: &DialogToSessionEvent) -> Result<()> {
        match event {
            DialogToSessionEvent::DialogCreated { dialog_id, call_id } => {
                self.handle_dialog_created_parts(dialog_id.clone(), call_id.clone())
                    .await
            }
            DialogToSessionEvent::IncomingCall {
                session_id,
                call_id,
                from,
                to,
                sdp_offer,
                headers,
                transaction_id,
                source_addr,
                raw_request,
                transport,
                identity_verification: _,
            } => {
                self.handle_incoming_call_parts(
                    session_id.clone(),
                    call_id.clone(),
                    from.clone(),
                    to.clone(),
                    sdp_offer.clone(),
                    headers,
                    transaction_id,
                    source_addr,
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallStateChanged {
                session_id,
                new_state,
                ..
            } => {
                self.handle_call_state_changed_parts(SessionId(session_id.clone()), new_state)
                    .await
            }
            DialogToSessionEvent::CallProgress {
                session_id,
                status_code,
                reason_phrase,
                sdp,
                raw_response,
            } => {
                self.handle_call_progress_parts(
                    SessionId(session_id.clone()),
                    *status_code,
                    reason_phrase.clone(),
                    sdp.clone(),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallEstablished {
                session_id,
                sdp_answer,
                raw_response,
            } => {
                self.handle_call_established_parts(
                    SessionId(session_id.clone()),
                    sdp_answer.clone(),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::ByeReceived { session_id } => {
                self.handle_bye_received_parts(SessionId(session_id.clone()))
                    .await
            }
            DialogToSessionEvent::CallTerminated { session_id, reason } => {
                self.handle_call_terminated_parts(
                    SessionId(session_id.clone()),
                    termination_reason_to_string(reason),
                )
                .await
            }
            DialogToSessionEvent::CallFailed {
                session_id,
                status_code,
                reason_phrase,
                raw_response,
            } => {
                self.handle_call_failed_parts(
                    SessionId(session_id.clone()),
                    *status_code,
                    CallFailureReason::Protocol(reason_phrase.clone()),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallCancelled { session_id } => {
                self.handle_call_cancelled_session(SessionId(session_id.clone()))
                    .await
            }
            DialogToSessionEvent::SessionRefreshed {
                session_id,
                expires_secs,
            } => {
                self.handle_session_refreshed_parts(SessionId(session_id.clone()), *expires_secs)
                    .await
            }
            DialogToSessionEvent::SessionRefreshFailed { session_id, reason } => {
                self.handle_session_refresh_failed_parts(
                    SessionId(session_id.clone()),
                    reason.clone(),
                )
                .await
            }
            DialogToSessionEvent::AuthRequired {
                session_id,
                transaction_id,
                request_uri,
                status_code,
                challenge,
                method,
                outbound_transport,
                ..
            } => {
                self.handle_auth_required_parts(
                    SessionId(session_id.clone()),
                    transaction_id.clone(),
                    request_uri.clone(),
                    *status_code,
                    challenge.clone(),
                    method.clone(),
                    outbound_transport.clone(),
                    false,
                )
                .await
            }
            DialogToSessionEvent::OutboundRequestCompleted {
                session_id,
                transaction_id,
                method,
                outcome,
            } => {
                self.handle_outbound_request_completed_parts(
                    SessionId(session_id.clone()),
                    transaction_id,
                    method,
                    *outcome,
                    false,
                )
                .await
            }
            DialogToSessionEvent::CallRedirected {
                session_id,
                status_code,
                targets,
                q_values,
            } => {
                self.handle_call_redirected_typed(session_id, *status_code, targets, q_values)
                    .await
            }
            DialogToSessionEvent::ReinviteGlare { session_id } => {
                self.handle_reinvite_glare_session(SessionId(session_id.clone()))
                    .await
            }
            DialogToSessionEvent::SessionIntervalTooSmall {
                session_id,
                min_se_secs,
            } => {
                self.handle_session_interval_too_small_parts(
                    SessionId(session_id.clone()),
                    *min_se_secs,
                )
                .await
            }
            DialogToSessionEvent::DtmfReceived { session_id, tones } => {
                self.handle_dtmf_received_parts(SessionId(session_id.clone()), tones.clone())
                    .await
            }
            DialogToSessionEvent::DialogError {
                session_id, error, ..
            } => {
                self.handle_dialog_error_parts(SessionId(session_id.clone()), error.clone())
                    .await
            }
            DialogToSessionEvent::DialogStateChanged {
                session_id,
                old_state,
                new_state,
            } => {
                self.handle_dialog_state_changed_parts(
                    SessionId(session_id.clone()),
                    format!("{:?}", old_state),
                    format!("{:?}", new_state),
                )
                .await
            }
            DialogToSessionEvent::ReinviteReceived {
                session_id,
                sdp,
                method,
                raw_request,
                transport,
            } => {
                let sid = SessionId(session_id.clone());
                // SIP_API_DESIGN_2 Phase E: surface UPDATE separately
                // via `Event::UpdateReceived` so subscribers can
                // distinguish a re-INVITE from an UPDATE without
                // string-matching on `method`. INVITE keeps the
                // legacy hold/resume state-machine path.
                if method.eq_ignore_ascii_case("UPDATE") {
                    if let Some(incoming) = build_incoming_request_from_bytes(
                        sid.clone(),
                        raw_request.clone(),
                        transport.clone(),
                    ) {
                        publish_api_event(
                            &self.app_event_publisher,
                            crate::api::events::Event::UpdateReceived {
                                call_id: sid.clone(),
                                request: incoming,
                            },
                        );
                    }
                }
                self.handle_reinvite_received_parts(sid, sdp.clone(), method.clone())
                    .await
            }
            DialogToSessionEvent::TransferRequested {
                session_id,
                refer_to,
                transfer_type,
                transaction_id,
                referred_by,
                replaces,
                raw_request,
                transport,
            } => {
                self.handle_transfer_requested_parts(
                    SessionId(session_id.clone()),
                    refer_to.clone(),
                    transfer_type_to_string(transfer_type),
                    transaction_id.clone(),
                    referred_by.clone(),
                    replaces.clone(),
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::AckReceived { session_id, .. } => {
                self.handle_ack_received_session(SessionId(session_id.clone()))
                    .await
            }
            DialogToSessionEvent::RegistrationSuccess { session_id } => {
                self.handle_registration_success_parts(SessionId(session_id.clone()))
                    .await
            }
            DialogToSessionEvent::RegistrationFailed {
                session_id,
                status_code,
            } => {
                self.handle_registration_failed_parts(SessionId(session_id.clone()), *status_code)
                    .await
            }
            DialogToSessionEvent::SubscriptionAccepted { session_id } => {
                self.handle_state_event_if_ours(
                    SessionId(session_id.clone()),
                    EventType::SubscriptionAccepted,
                    "SubscriptionAccepted",
                )
                .await
            }
            DialogToSessionEvent::SubscriptionFailed {
                session_id,
                status_code,
            } => {
                self.handle_state_event_if_ours(
                    SessionId(session_id.clone()),
                    EventType::SubscriptionFailed(*status_code),
                    "SubscriptionFailed",
                )
                .await
            }
            DialogToSessionEvent::NotifyReceived {
                session_id,
                event_package,
                subscription_state,
                content_type,
                body,
                raw_request,
                transport,
            } => {
                if raw_request.is_none() {
                    tracing::warn!(
                        "NotifyReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve NOTIFY bytes for \
                         session {}",
                        session_id
                    );
                }
                self.handle_notify_received_parts(
                    SessionId(session_id.clone()),
                    event_package.clone(),
                    subscription_state.clone(),
                    content_type.clone(),
                    body.clone(),
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::MessageDelivered { session_id } => {
                self.handle_state_event_if_ours(
                    SessionId(session_id.clone()),
                    EventType::MessageDelivered,
                    "MessageDelivered",
                )
                .await
            }
            DialogToSessionEvent::MessageFailed {
                session_id,
                status_code,
            } => {
                self.handle_state_event_if_ours(
                    SessionId(session_id.clone()),
                    EventType::MessageFailed(*status_code),
                    "MessageFailed",
                )
                .await
            }
            DialogToSessionEvent::IncomingRegister {
                transaction_id,
                from_uri,
                to_uri,
                contact_uri,
                expires,
                authorization,
                call_id,
                raw_request,
                transport,
            } => {
                // SIP_API_DESIGN_2 Phase D — surface inbound REGISTER as a
                // typed `IncomingRegister` so registrar applications can
                // author responses via `accept_builder()` / `challenge_builder()`
                // / `reject_builder()`. When the bus carries the original
                // wire bytes, re-parse them once into an `Arc<Request>`
                // for typed-header inspection; otherwise fall through to
                // the synthesized view (legacy publish path).
                let coordinator = self.coordinator.get().and_then(|w| w.upgrade());
                let parsed_request: Option<Arc<rvoip_sip_core::Request>> =
                    raw_request.as_ref().and_then(|bytes| {
                        match rvoip_sip_core::parse_message(bytes.as_ref()) {
                            Ok(rvoip_sip_core::Message::Request(req)) => Some(Arc::new(req)),
                            _ => None,
                        }
                    });

                let register = match (parsed_request, coordinator) {
                    (Some(req), Some(coord)) => {
                        crate::api::incoming::IncomingRegister::with_request_and_coordinator(
                            transaction_id.clone(),
                            from_uri.clone(),
                            to_uri.clone(),
                            contact_uri.clone(),
                            *expires,
                            authorization.clone(),
                            call_id.clone(),
                            req,
                            coord,
                        )
                    }
                    (Some(req), None) => crate::api::incoming::IncomingRegister::with_request(
                        transaction_id.clone(),
                        from_uri.clone(),
                        to_uri.clone(),
                        contact_uri.clone(),
                        *expires,
                        authorization.clone(),
                        call_id.clone(),
                        req,
                    ),
                    (None, _) => crate::api::incoming::IncomingRegister::synthetic(
                        transaction_id.clone(),
                        from_uri.clone(),
                        to_uri.clone(),
                        contact_uri.clone(),
                        *expires,
                        authorization.clone(),
                        call_id.clone(),
                    ),
                };
                let register = register.with_transport_context(sip_transport_context(transport));
                publish_api_event(
                    &self.app_event_publisher,
                    crate::api::events::Event::IncomingRegister { register },
                );
                Ok(())
            }
            DialogToSessionEvent::OutboundFlowFailed { aor, reason, .. } => {
                self.handle_outbound_flow_failed_parts(aor.clone(), reason.clone())
                    .await
            }
            // SIP_API_DESIGN_2 Phase E — inbound mid-dialog INFO / MESSAGE / OPTIONS.
            // Each variant reaches session-core with the original
            // inbound bytes preserved; we re-parse them once via
            // `parse_message` into an `Arc<Request>` and surface a
            // typed `Event::*Received` carrying the `IncomingRequest`
            // view.
            DialogToSessionEvent::InfoReceived {
                session_id,
                transaction_id,
                raw_request,
                transport,
            } => {
                if raw_request.is_none() {
                    tracing::warn!(
                        "InfoReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve INFO bytes for \
                         session {}",
                        session_id
                    );
                }
                let sid = SessionId(session_id.clone());
                let Some(mut incoming) = build_incoming_request_from_bytes(
                    sid.clone(),
                    raw_request.clone(),
                    transport.clone(),
                ) else {
                    // Dialog-core already created this server transaction. If
                    // its preserved request cannot be reconstructed, fail it
                    // explicitly instead of leaving a non-INVITE transaction
                    // in Trying forever. The adapter boundary verifies that
                    // the carried transaction belongs to this session's
                    // dialog before it can write the response.
                    let failure_transaction = validated_inbound_info_event_transaction(
                        transaction_id,
                    )
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "unusable inbound INFO has no valid exact event transaction"
                        )
                    })?;
                    self.dialog_adapter
                        .send_response_for_transaction(&sid, &failure_transaction, 500, None)
                        .await?;
                    return Ok(());
                };
                let exact_response_transaction = match incoming.raw_request() {
                    Some(request) => {
                        match correlate_inbound_info_transaction(transaction_id, request) {
                            Ok(transaction) => transaction,
                            Err(failure_transaction) => {
                                tracing::warn!(
                                    session_id = %sid,
                                    "Rejecting InfoReceived whose exact transaction does not match the wire request"
                                );
                                // The request-derived key is authoritative.
                                // Never fall back to the event key here: a
                                // stale same-dialog event could otherwise
                                // author a response on a different INFO.
                                let failure_transaction = failure_transaction.ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "inbound INFO has no safe exact response transaction"
                                    )
                                })?;
                                self.dialog_adapter
                                    .send_response_for_transaction(
                                        &sid,
                                        &failure_transaction,
                                        500,
                                        None,
                                    )
                                    .await?;
                                return Ok(());
                            }
                        }
                    }
                    None => {
                        tracing::warn!(
                            session_id = %sid,
                            "Rejecting InfoReceived without an authoritative wire request"
                        );
                        return Err(anyhow::anyhow!(
                            "inbound INFO has no authoritative wire request"
                        ));
                    }
                };
                incoming.set_response_transaction(exact_response_transaction.clone());
                let coordinator = self
                    .coordinator
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .ok_or_else(|| {
                        anyhow::anyhow!("inbound INFO response owner has no coordinator")
                    })?;
                match incoming.set_coordinator(coordinator) {
                    crate::api::unified::ExactResponseRegistration::Registered => {}
                    crate::api::unified::ExactResponseRegistration::Closed => {
                        tracing::warn!(
                            session_id = %sid,
                            "Rejecting inbound INFO while exact-response registry is draining"
                        );
                        return Err(anyhow::anyhow!(
                            "inbound INFO exact-response registry is draining"
                        ));
                    }
                    crate::api::unified::ExactResponseRegistration::Collision => {
                        tracing::warn!(
                            session_id = %sid,
                            "Discarding duplicate inbound INFO exact-response obligation"
                        );
                        return Ok(());
                    }
                }
                let rejected_delivery_response = incoming.clone();
                let event = crate::api::events::Event::InfoReceived {
                    call_id: sid.clone(),
                    request: incoming,
                };
                if let Err(error) = self.app_event_publisher.publish_control_now(event).await {
                    tracing::warn!(
                        session_id = %sid,
                        "Inbound INFO application delivery was rejected: {}",
                        error
                    );
                    rejected_delivery_response.respond(503)?.send().await?;
                }
                Ok(())
            }
            DialogToSessionEvent::MessageReceived {
                session_id,
                raw_request,
                transport,
            } => {
                if raw_request.is_none() {
                    tracing::warn!(
                        "MessageReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve MESSAGE bytes for \
                         session {}",
                        session_id
                    );
                }
                let sid = SessionId(session_id.clone());
                if let Some(incoming) = build_incoming_request_from_bytes(
                    sid.clone(),
                    raw_request.clone(),
                    transport.clone(),
                ) {
                    publish_api_event(
                        &self.app_event_publisher,
                        crate::api::events::Event::MessageReceived {
                            call_id: sid,
                            request: incoming,
                        },
                    );
                }
                Ok(())
            }
            DialogToSessionEvent::OptionsReceived {
                session_id,
                raw_request,
                transport,
            } => {
                if raw_request.is_none() {
                    tracing::warn!(
                        "OptionsReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve OPTIONS bytes for \
                         session {:?}",
                        session_id
                    );
                }
                // Out-of-dialog OPTIONS arrives with an empty
                // session_id; in-dialog OPTIONS rides the session
                // mapping established during INVITE.
                let sid_opt = if session_id.is_empty() {
                    None
                } else {
                    Some(SessionId(session_id.clone()))
                };
                let sid_for_request = sid_opt
                    .clone()
                    .unwrap_or_else(|| SessionId(String::from("options-oob")));
                if let Some(incoming) = build_incoming_request_from_bytes(
                    sid_for_request,
                    raw_request.clone(),
                    transport.clone(),
                ) {
                    publish_api_event(
                        &self.app_event_publisher,
                        crate::api::events::Event::OptionsReceived {
                            call_id: sid_opt,
                            request: incoming,
                        },
                    );
                }
                Ok(())
            }
        }
    }

    async fn handle_media_to_session_event(&self, event: &MediaToSessionEvent) -> Result<()> {
        match event {
            MediaToSessionEvent::MediaStreamStarted { session_id, .. } => {
                self.handle_media_stream_started_session(SessionId(session_id.clone()))
                    .await
            }
            MediaToSessionEvent::MediaStreamStopped { session_id, reason } => {
                self.handle_media_stream_stopped_parts(
                    SessionId(session_id.clone()),
                    reason.clone(),
                )
                .await
            }
            MediaToSessionEvent::MediaQualityUpdate {
                session_id,
                quality_metrics,
            } => {
                self.handle_media_quality_update_parts(
                    SessionId(session_id.clone()),
                    quality_metrics,
                )
                .await
            }
            MediaToSessionEvent::RecordingStarted { .. }
            | MediaToSessionEvent::RecordingStopped { .. }
            | MediaToSessionEvent::AudioPlaybackFinished { .. } => {
                debug!(
                    "Media lifecycle event has no session-core state transition: {:?}",
                    event
                );
                Ok(())
            }
            MediaToSessionEvent::MediaError {
                session_id, error, ..
            } => {
                self.handle_media_error_parts(SessionId(session_id.clone()), error.clone())
                    .await
            }
            MediaToSessionEvent::MediaFlowEstablished { session_id } => {
                self.handle_media_flow_established_session(SessionId(session_id.clone()))
                    .await
            }
            MediaToSessionEvent::MediaQualityDegraded {
                session_id,
                metrics,
                severity,
            } => {
                self.handle_media_quality_degraded_parts(
                    SessionId(session_id.clone()),
                    (metrics.packet_loss * 100.0) as u32,
                    metrics.jitter_ms as u32,
                    format!("{:?}", severity).to_ascii_lowercase(),
                )
                .await
            }
            MediaToSessionEvent::DtmfDetected {
                session_id,
                digit,
                duration_ms,
            } => {
                self.handle_dtmf_detected_parts(SessionId(session_id.clone()), *digit, *duration_ms)
                    .await
            }
            MediaToSessionEvent::RtpTimeout {
                session_id,
                last_packet_time,
            } => {
                self.handle_rtp_timeout_parts(
                    SessionId(session_id.clone()),
                    last_packet_time.to_string(),
                )
                .await
            }
            MediaToSessionEvent::PacketLossThresholdExceeded {
                session_id,
                loss_percentage,
            } => {
                self.handle_packet_loss_threshold_exceeded_parts(
                    SessionId(session_id.clone()),
                    (*loss_percentage * 100.0) as u32,
                )
                .await
            }
        }
    }

    async fn handle_transport_to_session_event(&self, event: &SipTraceEvent) -> Result<()> {
        if !sip_trace_owner_matches(self.sip_trace_owner_id.as_deref(), &event.owner_id) {
            return Ok(());
        }

        let session_id = map_sip_trace_session_id(event, &self.dialog_adapter.callid_to_session);

        let trace = crate::api::events::SipTrace {
            direction: event.direction.clone(),
            transport: event.transport.clone(),
            local_addr: event.local_addr.clone(),
            remote_addr: event.remote_addr.clone(),
            timestamp_unix_millis: event.timestamp_unix_millis,
            start_line: event.start_line.clone(),
            sip_call_id: event.sip_call_id.clone(),
            session_id,
            raw_message: event.raw_message.clone(),
            original_len: event.original_len,
            truncated: event.truncated,
            redacted: event.redacted,
        };

        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::SipTrace(trace),
        );
        Ok(())
    }

    pub fn new(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            state_machine,
            global_coordinator: global_coordinator.clone(),
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: None,
            fast_auto_accept_incoming_calls: false,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            server_call_admission_overloaded: Arc::new(AtomicBool::new(false)),
            server_call_admission_pending: Arc::new(AtomicUsize::new(0)),
            server_call_admission_lock: Arc::new(Mutex::new(())),
            dialog_event_dispatch_queue_capacity: 1024,
            state_machine_event_rx: None,
            outbound_flow_last_refresh: Arc::new(DashMap::new()),
            app_event_publisher: SessionEventPublisher::new(
                global_coordinator.clone(),
                LifecycleIndex::new(),
            ),
            sip_trace_owner_id: None,
            coordinator: Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub(crate) fn with_incoming_call_channel(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        app_event_publisher: SessionEventPublisher,
    ) -> Self {
        Self {
            state_machine,
            global_coordinator: global_coordinator.clone(),
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: Some(incoming_call_tx),
            fast_auto_accept_incoming_calls: false,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            server_call_admission_overloaded: Arc::new(AtomicBool::new(false)),
            server_call_admission_pending: Arc::new(AtomicUsize::new(0)),
            server_call_admission_lock: Arc::new(Mutex::new(())),
            dialog_event_dispatch_queue_capacity: 1024,
            state_machine_event_rx: None,
            outbound_flow_last_refresh: Arc::new(DashMap::new()),
            app_event_publisher,
            sip_trace_owner_id: None,
            coordinator: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Preferred constructor — events are published to the global coordinator's
    /// "session_to_app" channel automatically; no separate broadcast sender needed.
    pub(crate) fn with_event_broadcast(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        app_event_publisher: SessionEventPublisher,
    ) -> Self {
        Self::with_incoming_call_channel(
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx,
            app_event_publisher,
        )
    }

    /// Preferred constructor for UnifiedCoordinator. In addition to
    /// cross-crate bus subscriptions, this owns the internal state-machine
    /// event stream that publishes app-visible call state events.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_event_broadcast_and_state_machine_events(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        state_machine_event_rx: mpsc::Receiver<crate::state_machine::executor::SessionEvent>,
        app_event_publisher: SessionEventPublisher,
        sip_trace_owner_id: Option<String>,
    ) -> Self {
        let mut handler = Self::with_incoming_call_channel(
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx,
            app_event_publisher,
        );
        handler.state_machine_event_rx = Some(Arc::new(Mutex::new(state_machine_event_rx)));
        handler.sip_trace_owner_id = sip_trace_owner_id;
        handler
    }

    pub(crate) fn with_fast_auto_accept_incoming_calls(
        mut self,
        enabled: bool,
        queue_capacity: usize,
    ) -> Self {
        self.fast_auto_accept_incoming_calls = enabled;
        self.dialog_event_dispatch_queue_capacity = queue_capacity.max(1);
        self
    }

    pub(crate) fn with_server_call_admission(
        mut self,
        limit: Option<usize>,
        soft_limit: Option<usize>,
        pacing_delay_ms: Option<u64>,
        retry_after_secs: Option<u32>,
    ) -> Self {
        self.server_call_admission_limit = limit;
        self.server_call_admission_soft_limit = soft_limit;
        self.server_call_admission_pacing_delay_ms = pacing_delay_ms;
        self.server_overload_retry_after_secs = retry_after_secs;
        self
    }

    /// SIP_API_DESIGN_2 Phase D — pin the coordinator handle so the
    /// bus-path `IncomingRegister` branch can build a
    /// `RegisterResponseBuilder` that can publish responses back to
    /// dialog-core. Idempotent; subsequent calls are no-ops.
    pub(crate) fn set_coordinator(
        &self,
        coordinator: &Arc<crate::api::unified::UnifiedCoordinator>,
    ) {
        let _ = self.coordinator.set(Arc::downgrade(coordinator));
    }

    /// Publish a terminal app-level event, then release the session from the
    /// store + registry.
    ///
    /// Terminal events are `CallEnded`, `CallFailed`, `CallCancelled`. Publish
    /// runs first so any subscriber that queries session state in response to
    /// the event still sees a populated entry; the release then happens in the
    /// same spawned task after publish returns. Without this, long-running
    /// peers (and especially b2bua, which multiplies sessions) would leak
    /// `SessionStore` entries indefinitely.
    async fn publish_and_release_session(
        &self,
        api_event: crate::api::events::Event,
        session_id: SessionId,
    ) {
        let publisher = self.app_event_publisher.clone();
        let store = self.state_machine.store.clone();
        let dialog_adapter = self.dialog_adapter.clone();
        let media_adapter = self.media_adapter.clone();
        let Some(handle) = store.lifecycle_handle(&session_id) else {
            debug!(
                session = %session_id,
                "exact terminal lifetime was already released before publication"
            );
            return;
        };
        let claim_owner = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Owner(owner) => {
                debug!(session = %session_id, "dialog handler owns exact terminal publication");
                owner
            }
            ExactTerminalClaim::Observer(_) => {
                debug!(
                    session = %session_id,
                    "exact terminal publication is already owned by another path"
                );
                return;
            }
        };
        let coordinator = self.coordinator.get().and_then(|weak| weak.upgrade());
        tokio::spawn(async move {
            let release_guard =
                cleanup_diag::stage_guard(CleanupStage::TerminalRelease, &session_id.0);
            let publication_succeeded = if let Err(e) = publisher.publish_now(api_event).await {
                tracing::warn!(
                    "Failed to publish terminal event to global coordinator: {}",
                    e
                );
                false
            } else {
                true
            };
            let release = match coordinator {
                Some(coordinator) => {
                    crate::api::unified::release_exact_local_resources(
                        store,
                        Arc::clone(&coordinator.helpers),
                        dialog_adapter,
                        media_adapter,
                        handle,
                    )
                    .await
                }
                None => Err(SessionError::InternalError(
                    "terminal cleanup had no exact session owner".to_string(),
                )),
            };
            let release_succeeded = match release {
                Ok(()) => {
                    release_guard.finish_success();
                    true
                }
                Err(error) => {
                    tracing::debug!(%error, "exact terminal cleanup was incomplete");
                    release_guard.finish_failure();
                    false
                }
            };
            let completion = match (publication_succeeded, release_succeeded) {
                (true, true) => ExactTerminalCompletion::PublishedAndReleased,
                (false, true) => ExactTerminalCompletion::PublicationFailed,
                (true, false) => ExactTerminalCompletion::ReleaseFailed,
                (false, false) => ExactTerminalCompletion::PublicationAndReleaseFailed,
            };
            claim_owner.finish(completion);
        });
    }

    /// Commit the exact registry bundle and the adapter compatibility maps as
    /// one compensated operation. The lower inbound dialog already owns its
    /// session mapping, so this deliberately does not call the public
    /// unconditional dialog-core mapping API.
    fn commit_exact_inbound_dialog(
        &self,
        handle: &SessionRegistryHandle,
        dialog_id: DialogId,
        lower_dialog_id: rvoip_sip_dialog::DialogId,
        call_id: String,
        incoming_info: crate::types::IncomingCallInfo,
    ) -> SessionResult<()> {
        let receipt = self
            .registry
            .commit_inbound_dialog_exact(
                handle.key(),
                handle.slot_revision(),
                dialog_id,
                incoming_info,
            )
            .map_err(|error| {
                SessionError::InternalError(format!(
                    "exact inbound registry commit failed (class=registry): {error}"
                ))
            })?;

        let session_id = handle.session_id().clone();
        let session_to_dialog = Arc::clone(&self.dialog_adapter.session_to_dialog);
        let dialog_to_session = Arc::clone(&self.dialog_adapter.dialog_to_session);
        let callid_to_session = Arc::clone(&self.dialog_adapter.callid_to_session);
        let compensation_session_to_dialog = Arc::clone(&session_to_dialog);
        let compensation_dialog_to_session = Arc::clone(&dialog_to_session);
        let compensation_callid_to_session = Arc::clone(&callid_to_session);
        let compensation_session_id = session_id.clone();
        let compensation_dialog_id = lower_dialog_id.clone();
        let compensation_call_id = call_id.clone();
        let compensation = move || {
            compensation_session_to_dialog.remove_if(&compensation_session_id, |_, mapped| {
                mapped == &compensation_dialog_id
            });
            compensation_dialog_to_session.remove_if(&compensation_dialog_id, |_, mapped| {
                mapped == &compensation_session_id
            });
            compensation_callid_to_session.remove_if(&compensation_call_id, |_, mapped| {
                mapped == &compensation_session_id
            });
        };

        let external = catch_unwind(AssertUnwindSafe(|| {
            session_to_dialog.insert(session_id.clone(), lower_dialog_id.clone());
            dialog_to_session.insert(lower_dialog_id, session_id.clone());
            callid_to_session.insert(call_id, session_id);
        }));
        match external {
            Ok(()) => receipt
                .finalize_with_compensation(compensation)
                .map_err(|error| {
                    SessionError::InternalError(format!(
                        "exact inbound mapping finalize failed (class=registry): {error}"
                    ))
                }),
            Err(_) => {
                let _ = receipt.rollback_with_compensation(compensation);
                Err(SessionError::InternalError(
                    "exact inbound compatibility mapping panicked".to_string(),
                ))
            }
        }
    }

    /// Start event processing loops.
    ///
    /// Background tasks will stop when `shutdown_rx` receives `true`.
    pub async fn start(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<()> {
        self.start_global_event_subscriptions(shutdown_rx).await?;
        Ok(())
    }

    /// Start subscriptions to global cross-crate events
    async fn start_global_event_subscriptions(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<()> {
        // Session lifecycle correctness must not depend on broadcast delivery.
        // Register a direct handler that only enqueues into bounded sharded
        // workers; the global broadcast remains available for observers.
        let dialog_router = DialogToSessionDirectRouter::new(
            self.clone(),
            dialog_dispatch_worker_count(),
            self.dialog_event_dispatch_queue_capacity,
            shutdown_rx.clone(),
        );
        self.global_coordinator
            .register_handler("dialog_to_session", dialog_router.clone())
            .await
            .map_err(|e| {
                SessionError::InternalError(format!(
                    "Failed to register direct dialog event handler: {}",
                    e
                ))
            })?;

        if let Some(mut deferred_replay_rx) = self
            .dialog_adapter
            .outbound_request_tracker
            .take_deferred_replay_receiver()
        {
            let replay_router = dialog_router;
            let tracker = self.dialog_adapter.outbound_request_tracker.clone();
            let mut replay_shutdown = shutdown_rx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        changed = replay_shutdown.changed() => {
                            if shutdown_change_requests_stop(changed, &replay_shutdown) {
                                deferred_replay_rx.close();
                                while let Ok(deferred) = deferred_replay_rx.try_recv() {
                                    tracker.abort_deferred_replay(&deferred);
                                }
                                break;
                            }
                        }
                        deferred = deferred_replay_rx.recv() => {
                            let Some(deferred) = deferred else { break };
                            if let Err(error) = replay_router.handle_deferred(deferred).await {
                                error!(
                                    "Failed to replay deferred exact in-dialog event: {}",
                                    error
                                );
                            }
                        }
                    }
                }
            });
        }

        // Subscribe to transport-to-session diagnostics such as SIP trace.
        let mut transport_sub = self
            .global_coordinator
            .subscribe("transport_to_session")
            .await
            .map_err(|e| {
                SessionError::InternalError(format!(
                    "Failed to subscribe to transport diagnostics: {}",
                    e
                ))
            })?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if shutdown_change_requests_stop(changed, &shutdown) { break; }
                    }
                    event = transport_sub.recv() => {
                        let Some(event) = event else { break };
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling transport-to-session event: {}", e);
                        }
                    }
                }
            }
        });

        // Subscribe to media-to-session events
        let mut media_sub = self
            .global_coordinator
            .subscribe("media_to_session")
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to subscribe to media events: {}", e))
            })?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if shutdown_change_requests_stop(changed, &shutdown) { break; }
                    }
                    event = media_sub.recv() => {
                        let Some(event) = event else { break };
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling media-to-session event: {}", e);
                        }
                    }
                }
            }
        });

        if let Some(state_machine_event_rx) = &self.state_machine_event_rx {
            let state_machine_event_rx = state_machine_event_rx.clone();
            let handler = self.clone();
            let mut shutdown = shutdown_rx;
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if shutdown_change_requests_stop(changed, &shutdown) { break; }
                        }
                        event = async {
                            let mut rx = state_machine_event_rx.lock().await;
                            rx.recv().await
                        } => {
                            let Some(event) = event else { break };
                            handler.handle_state_machine_event(event).await;
                        }
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_state_machine_event(
        &self,
        event: crate::state_machine::executor::SessionEvent,
    ) {
        let api_event = match event {
            crate::state_machine::executor::SessionEvent::CallCancelled { session_id } => {
                debug!(
                    "Ignoring state-machine CallCancelled for {}; terminal cancellation is published by the dialog event handler after wire teardown",
                    session_id
                );
                return;
            }
            crate::state_machine::executor::SessionEvent::CallOnHold { session_id } => {
                Some(crate::api::events::Event::CallOnHold {
                    call_id: session_id,
                })
            }
            crate::state_machine::executor::SessionEvent::CallResumed { session_id } => {
                Some(crate::api::events::Event::CallResumed {
                    call_id: session_id,
                })
            }
            _ => None,
        };

        if let Some(api_event) = api_event {
            publish_api_event(&self.app_event_publisher, api_event);
        }
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for SessionCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());

        match event.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
            Some(RvoipCrossCrateEvent::DialogToSession(typed)) => {
                self.handle_dialog_to_session_event(typed).await?;
            }
            Some(RvoipCrossCrateEvent::MediaToSession(typed)) => {
                self.handle_media_to_session_event(typed).await?;
            }
            Some(RvoipCrossCrateEvent::TransportToSession(typed)) => {
                self.handle_transport_to_session_event(typed).await?;
            }
            Some(other) => {
                debug!(
                    "Ignoring cross-crate event not targeted at session-core: {:?}",
                    other
                );
            }
            None => {
                debug!(
                    "Ignoring non-rvoip cross-crate event on session-core handler: {}",
                    event.event_type()
                );
            }
        }

        Ok(())
    }
}

#[allow(dead_code)]
impl SessionCrossCrateEventHandler {
    /// Check if a session belongs to this handler's store.
    /// Returns false (and logs at debug) if the session was created by a different peer.
    async fn is_our_session(&self, session_id: &SessionId) -> bool {
        self.state_machine
            .store
            .lifecycle_handle(session_id)
            .is_some()
    }

    /// Extract session ID from event debug string (temporary workaround)
    fn extract_session_id(&self, event_str: &str) -> Option<String> {
        // Look for session_id in the debug output
        if let Some(start) = event_str.find("session_id: \"") {
            let start = start + 13;
            if let Some(end) = event_str[start..].find('"') {
                let session_id = event_str[start..start + end].to_string();
                info!(
                    "✅ [extract_session_id] Successfully extracted: {}",
                    session_id
                );
                return Some(session_id);
            }
        }
        warn!(
            "⚠️ [extract_session_id] Failed to extract session_id (event_present={}, event_bytes={})",
            !event_str.is_empty(),
            event_str.len()
        );
        None
    }

    /// Extract a field value from event debug string (temporary workaround)
    fn extract_field(&self, event_str: &str, field_prefix: &str) -> Option<String> {
        if let Some(start) = event_str.find(field_prefix) {
            let start = start + field_prefix.len();
            if let Some(end) = event_str[start..].find('"') {
                return Some(event_str[start..start + end].to_string());
            }
        }
        None
    }

    /// Extract a quoted Debug string field and unescape its contents.
    ///
    /// `AuthRequired.challenge` carries a header value such as
    /// `Digest realm="asterisk", nonce="..."`. In a derived `Debug`
    /// representation those inner quotes are escaped, so the simpler
    /// `extract_field` helper stops after `Digest realm=\` and drops the
    /// nonce. Keep this helper local to the Debug-string bridge until these
    /// handlers are moved to typed events.
    fn extract_debug_string_field(&self, event_str: &str, field_prefix: &str) -> Option<String> {
        let start = event_str.find(field_prefix)? + field_prefix.len();
        let mut value = String::new();
        let mut escaped = false;

        for ch in event_str[start..].chars() {
            if escaped {
                value.push(ch);
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '"' => return Some(value),
                _ => value.push(ch),
            }
        }

        None
    }

    async fn handle_state_event_if_ours(
        &self,
        session_id: SessionId,
        event_type: EventType,
        label: &str,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring {} for session {} - not in our store",
                label, session_id
            );
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&session_id, event_type)
            .await
        {
            error!(
                "Failed to process {} for session {}: {}",
                label, session_id, e
            );
        }
        Ok(())
    }

    async fn handle_dialog_created_parts(&self, dialog_id: String, call_id: String) -> Result<()> {
        if call_id.contains("@session-core") {
            if let Some(session_id_str) = call_id.split('@').next() {
                let session_id = SessionId(session_id_str.to_string());
                if self
                    .state_machine
                    .store
                    .lifecycle_handle(&session_id)
                    .is_none()
                {
                    debug!(
                        "DialogCreated event arrived before session {} was fully created, will be handled by state machine later",
                        session_id
                    );
                    return Ok(());
                }

                if let Err(e) = self
                    .state_machine
                    .process_event(&session_id, EventType::DialogCreated { dialog_id, call_id })
                    .await
                {
                    error!("Failed to process DialogCreated event: {}", e);
                }
            }
        }
        Ok(())
    }

    async fn acquire_server_call_admission(&self) -> ServerCallAdmissionDecision {
        #[cfg(feature = "perf-tests")]
        crate::admission_diag::record_attempt();

        let Some(hard_limit) = self.server_call_admission_limit else {
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_no_limit_admit();
            return ServerCallAdmissionDecision::Admit(None);
        };
        let soft_limit = self
            .server_call_admission_soft_limit
            .unwrap_or(hard_limit)
            .min(hard_limit);
        #[cfg(feature = "perf-tests")]
        crate::admission_diag::record_limits(hard_limit, soft_limit);
        let pacing_delay = self
            .server_call_admission_pacing_delay_ms
            .map(Duration::from_millis);
        let mut paced_once = false;

        loop {
            #[cfg(feature = "perf-tests")]
            let admission_lock_wait_started = Instant::now();
            let _lock = self.server_call_admission_lock.lock().await;
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_lock_wait(admission_lock_wait_started.elapsed());
            let pending = self.server_call_admission_pending.load(Ordering::Relaxed);
            let observed_sessions = self
                .state_machine
                .store
                .sessions
                .len()
                .saturating_add(pending);
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_observed(observed_sessions, pending);

            if self
                .server_call_admission_overloaded
                .load(Ordering::Relaxed)
            {
                if observed_sessions < soft_limit {
                    self.server_call_admission_overloaded
                        .store(false, Ordering::Relaxed);
                    #[cfg(feature = "perf-tests")]
                    crate::admission_diag::record_overload_cleared();
                } else {
                    #[cfg(feature = "perf-tests")]
                    crate::admission_diag::record_reject_overloaded(observed_sessions);
                    return ServerCallAdmissionDecision::Reject {
                        observed_sessions,
                        hard_limit,
                    };
                }
            }

            if observed_sessions >= hard_limit {
                self.server_call_admission_overloaded
                    .store(true, Ordering::Relaxed);
                #[cfg(feature = "perf-tests")]
                {
                    crate::admission_diag::record_overload_entered();
                    crate::admission_diag::record_reject_hard_limit(observed_sessions);
                }
                return ServerCallAdmissionDecision::Reject {
                    observed_sessions,
                    hard_limit,
                };
            }

            if !paced_once {
                if let (Some(delay), Some(configured_soft_limit)) =
                    (pacing_delay, self.server_call_admission_soft_limit)
                {
                    if observed_sessions >= configured_soft_limit {
                        #[cfg(feature = "perf-tests")]
                        crate::admission_diag::record_pacing_decision();
                        drop(_lock);
                        #[cfg(feature = "perf-tests")]
                        let admission_pacing_started = Instant::now();
                        tokio::time::sleep(delay).await;
                        #[cfg(feature = "perf-tests")]
                        crate::admission_diag::record_pacing_sleep(
                            admission_pacing_started.elapsed(),
                        );
                        paced_once = true;
                        continue;
                    }
                }
            }

            #[cfg(feature = "perf-tests")]
            {
                let pending_after = self
                    .server_call_admission_pending
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                crate::admission_diag::record_admit(observed_sessions, pending_after);
            }
            #[cfg(not(feature = "perf-tests"))]
            self.server_call_admission_pending
                .fetch_add(1, Ordering::Relaxed);
            return ServerCallAdmissionDecision::Admit(Some(ServerCallAdmissionGuard {
                pending: self.server_call_admission_pending.clone(),
            }));
        }
    }

    async fn reject_incoming_call_for_overload(
        &self,
        transaction_id: &str,
        observed_sessions: usize,
        configured_admission_limit: Option<usize>,
    ) -> Result<()> {
        let transaction_id = transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .map_err(|_| anyhow::anyhow!("server admission transaction id is invalid"))?;

        let mut response = self
            .dialog_adapter
            .dialog_api
            .build_response(
                &transaction_id,
                rvoip_sip_core::StatusCode::ServiceUnavailable,
                None,
            )
            .await
            .map_err(|_| anyhow::anyhow!("failed to build server overload response"))?;

        if let Some(seconds) = self.server_overload_retry_after_secs {
            response
                .headers
                .push(rvoip_sip_core::types::TypedHeader::RetryAfter(
                    rvoip_sip_core::types::retry_after::RetryAfter::new(seconds),
                ));
        }

        self.dialog_adapter
            .dialog_api
            .send_response(&transaction_id, response)
            .await
            .map_err(|_| anyhow::anyhow!("failed to send server overload response"))?;

        warn!(
            observed_sessions,
            configured_admission_limit,
            soft_limit = ?self.server_call_admission_soft_limit,
            retry_after_secs = ?self.server_overload_retry_after_secs,
            "Rejected inbound INVITE with 503 because server admission capacity was reached"
        );
        Ok(())
    }

    async fn cleanup_rejected_incoming_call_routes(
        &self,
        incoming_dialog_id: Option<&rvoip_sip_dialog::DialogId>,
        dialog_id_str: &str,
        transaction_id: &str,
    ) {
        if let Some(dialog_id) = incoming_dialog_id {
            let removed = self
                .dialog_adapter
                .dialog_api
                .dialog_manager()
                .core()
                .cleanup_dialog_storage_and_transactions(dialog_id)
                .await;
            debug!(
                dialog_id = %dialog_id,
                removed,
                "Cleaned up rejected inbound INVITE dialog after admission overload response"
            );
        } else {
            warn!(
                dialog_id = %dialog_id_str,
                "Rejected inbound INVITE for overload without a parseable dialog id; dialog-core cleanup skipped"
            );
        }
        match transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>() {
            Ok(transaction_id) => {
                self.dialog_adapter
                    .dialog_api
                    .dialog_manager()
                    .core()
                    .cleanup_transaction_receiver(&transaction_id);
            }
            Err(_) => {
                let (transaction_class, transaction_bytes) =
                    transaction_id_diagnostics(transaction_id);
                warn!(
                    transaction_class,
                    transaction_bytes,
                    "Rejected inbound INVITE for overload without a parseable transaction id; route cleanup skipped"
                );
            }
        }
    }

    async fn reject_and_cleanup_incoming_call_for_overload(
        &self,
        transaction_id: &str,
        incoming_dialog_id: Option<&rvoip_sip_dialog::DialogId>,
        dialog_id_str: &str,
        observed_sessions: usize,
        configured_admission_limit: Option<usize>,
    ) -> Result<()> {
        // Cleanup must run even when response construction or transport send
        // fails. Otherwise a capacity rejection can retain the exact dialog
        // or transaction receiver and make overload self-amplifying.
        let reject_result = self
            .reject_incoming_call_for_overload(
                transaction_id,
                observed_sessions,
                configured_admission_limit,
            )
            .await;
        self.cleanup_rejected_incoming_call_routes(
            incoming_dialog_id,
            dialog_id_str,
            transaction_id,
        )
        .await;
        reject_result
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_incoming_call_parts(
        &self,
        session_id_str: String,
        call_id: String,
        from: String,
        to: String,
        sdp: Option<String>,
        headers: &std::collections::HashMap<String, String>,
        transaction_id: &str,
        _source_addr: &str,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        let dialog_id_str = headers
            .get("X-Dialog-Id")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let p_asserted_identity = headers.get("P-Asserted-Identity").cloned();

        let incoming_dialog_id = uuid::Uuid::parse_str(&dialog_id_str)
            .ok()
            .map(rvoip_sip_dialog::DialogId);
        if let Some(rvoip_dialog_id) = incoming_dialog_id.as_ref() {
            if self
                .dialog_adapter
                .dialog_to_session
                .contains_key(rvoip_dialog_id)
            {
                debug!(
                    "Ignoring IncomingCall for dialog {} - already handled by another peer",
                    dialog_id_str
                );
                return Ok(());
            }

            if !self
                .dialog_adapter
                .dialog_api
                .dialog_manager()
                .core()
                .has_dialog(rvoip_dialog_id)
            {
                debug!(
                    "Ignoring IncomingCall for dialog {} - not in our dialog-core",
                    dialog_id_str
                );
                return Ok(());
            }
        }

        let session_id = SessionId(session_id_str);
        let authenticated_principal = transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .ok()
            .and_then(|transaction_id| {
                self.dialog_adapter
                    .dialog_api
                    .dialog_manager()
                    .core()
                    .transaction_manager()
                    .peek_inbound_principal(&transaction_id)
            });
        let setup_guard = cleanup_diag::stage_guard(CleanupStage::IncomingCallSetup, &session_id.0);

        let admission_guard = match self.acquire_server_call_admission().await {
            ServerCallAdmissionDecision::Admit(guard) => guard,
            ServerCallAdmissionDecision::Reject {
                observed_sessions,
                hard_limit,
            } => {
                self.reject_and_cleanup_incoming_call_for_overload(
                    transaction_id,
                    incoming_dialog_id.as_ref(),
                    &dialog_id_str,
                    observed_sessions,
                    Some(hard_limit),
                )
                .await?;
                setup_guard.finish_success();
                return Ok(());
            }
        };

        let create_result = self
            .state_machine
            .store
            .create_session(session_id.clone(), Role::UAS, true)
            .await;
        drop(admission_guard);
        if let Err(error) = create_result {
            if is_session_lifecycle_capacity_exhaustion(error.as_ref()) {
                let observed_sessions = self.state_machine.store.sessions.len();
                warn!(
                    observed_sessions,
                    configured_admission_limit = ?self.server_call_admission_limit,
                    "Session lifecycle capacity rejected inbound INVITE; returning SIP overload"
                );
                self.reject_and_cleanup_incoming_call_for_overload(
                    transaction_id,
                    incoming_dialog_id.as_ref(),
                    &dialog_id_str,
                    observed_sessions,
                    self.server_call_admission_limit,
                )
                .await?;
                setup_guard.finish_success();
                return Ok(());
            }
            return Err(
                SessionError::InternalError(format!("Failed to create session: {error}")).into(),
            );
        }

        let pending_transaction =
            match transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>() {
                Ok(transaction_id) => Some(transaction_id),
                Err(_) => {
                    let (transaction_class, transaction_bytes) =
                        transaction_id_diagnostics(transaction_id);
                    debug!(
                        session = %session_id,
                        transaction_class,
                        transaction_bytes,
                        "IncomingCall carried an unparsable transaction id"
                    );
                    None
                }
            };
        let session_remote_sdp = self
            .state_machine
            .store
            .update_session_with(&session_id, |session| {
                session.local_uri = Some(to.clone());
                session.remote_uri = Some(from.clone());
                session.incoming_invite_received_at = Some(Instant::now());
                session.pending_inbound_invite_transaction_id = pending_transaction;
                session.remote_sdp.clone()
            })
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to update session URIs: {}", e))
            })?;

        let dialog_uuid =
            uuid::Uuid::parse_str(&dialog_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        let our_dialog_id = DialogId(dialog_uuid);
        let rvoip_dialog_id = rvoip_sip_dialog::DialogId::from(our_dialog_id);
        let lifecycle = self
            .state_machine
            .store
            .lifecycle_token(&session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
        let incoming_info = crate::types::IncomingCallInfo {
            session_id: session_id.clone(),
            from: from.clone(),
            to: to.clone(),
            call_id: call_id.clone(),
            dialog_id: our_dialog_id,
            p_asserted_identity: p_asserted_identity.clone(),
        };
        self.commit_exact_inbound_dialog(
            &lifecycle,
            our_dialog_id,
            rvoip_dialog_id,
            call_id.clone(),
            incoming_info,
        )?;

        let event_type = if self.fast_auto_accept_incoming_calls {
            EventType::IncomingCallAutoAccept {
                from: from.clone(),
                sdp,
            }
        } else {
            EventType::IncomingCall {
                from: from.clone(),
                sdp,
            }
        };

        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            event_type,
        )
        .await
        {
            error!("Failed to process incoming call event: {}", e);
            if let Some(coordinator) = self.coordinator.get().and_then(|weak| weak.upgrade()) {
                if let Err(cleanup_err) = crate::api::unified::release_exact_local_resources(
                    Arc::clone(&self.state_machine.store),
                    Arc::clone(&coordinator.helpers),
                    Arc::clone(&self.dialog_adapter),
                    Arc::clone(&self.media_adapter),
                    lifecycle,
                )
                .await
                {
                    warn!(%cleanup_err, "exact failed-inbound cleanup was incomplete");
                }
            } else {
                warn!("exact failed-inbound cleanup had no coordinator owner");
            }
        } else {
            if self.fast_auto_accept_incoming_calls {
                debug!("Fast auto-accepted inbound call {}", session_id);
            }
            if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
                coordinator
                    .schedule_inbound_setup_timeout(&session_id)
                    .await;
            }

            // SIP_API_DESIGN_2 Phase A: re-parse the inbound INVITE bytes
            // after the fast 200 OK path has completed, but before app
            // observation events are published. Failure to parse is never
            // fatal — we fall back to the legacy headers-only path.
            let mut observed_request = None;
            if let Some(bytes) = raw_request.as_ref() {
                match rvoip_sip_core::parse_message(bytes.as_ref()) {
                    Ok(rvoip_sip_core::Message::Request(req)) => {
                        let req = Arc::new(req);
                        observed_request = Some(req);
                    }
                    Ok(_) => {
                        tracing::warn!(
                            session_id = %session_id,
                            "IncomingCall raw_request was not a SIP request"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "Failed to re-parse inbound INVITE bytes; \
                             IncomingCall.raw_request() will be None"
                        );
                    }
                }
            }
            if let Some(coordinator) = self.coordinator.get().and_then(|weak| weak.upgrade()) {
                coordinator.notify_inbound_invite_observers(
                    crate::api::unified::InboundInviteObservation {
                        session_id: session_id.clone(),
                        request: observed_request.clone(),
                        principal: authenticated_principal.clone(),
                    },
                );
            }
            let mut pending = self
                .registry
                .pending_bundle_exact(lifecycle.key(), lifecycle.slot_revision())
                .map_err(|error| {
                    SessionError::InternalError(format!(
                        "exact inbound bundle read failed (class=registry): {error}"
                    ))
                })?;
            pending.request = observed_request;
            pending.transport = Some(Arc::new(sip_transport_context(&transport)));
            pending.principal = authenticated_principal.clone();
            self.registry
                .store_pending_bundle_exact(
                    lifecycle.key(),
                    lifecycle.slot_revision(),
                    PendingInboundBundle {
                        info: pending.info,
                        request: pending.request,
                        transport: pending.transport,
                        principal: pending.principal,
                    },
                )
                .map_err(|error| {
                    SessionError::InternalError(format!(
                        "exact inbound bundle commit failed (class=registry): {error}"
                    ))
                })?;

            publish_api_event(
                &self.app_event_publisher,
                crate::api::events::Event::IncomingCall {
                    call_id: session_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    sdp: session_remote_sdp,
                },
            );
            if let Some(principal) = authenticated_principal {
                publish_api_event(
                    &self.app_event_publisher,
                    crate::api::events::Event::IncomingCallAuthenticated {
                        call_id: session_id.clone(),
                        principal,
                    },
                );
            }

            if let Some(ref tx) = self.incoming_call_tx {
                let call_info = crate::types::IncomingCallInfo {
                    session_id: session_id.clone(),
                    from,
                    to,
                    call_id,
                    dialog_id: DialogId(dialog_uuid),
                    p_asserted_identity,
                };
                if let Err(e) = tx.try_send(call_info) {
                    debug!(
                        "Legacy incoming_call_tx not ready — caller is using app_event_publisher path: {}",
                        e
                    );
                }
            }
        }

        setup_guard.finish_success();
        Ok(())
    }

    async fn handle_call_established_parts(
        &self,
        session_id: SessionId,
        sdp_answer: Option<String>,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring CallEstablished for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        if let Some(sdp) = &sdp_answer {
            let _ = self
                .state_machine
                .store
                .update_session_with(&session_id, |session| {
                    session.remote_sdp = Some(sdp.clone());
                })
                .await;
        }

        let mut publish_answered = true;
        match process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::Dialog200OK,
        )
        .await
        {
            Ok(result) => {
                publish_answered = !matches!(
                    result.old_state,
                    CallState::CancelPending | CallState::Cancelling
                ) && !matches!(
                    result.next_state,
                    Some(CallState::CancelPending | CallState::Cancelling | CallState::Cancelled)
                );
            }
            Err(e) => {
                error!("Failed to process CallEstablished as Dialog200OK: {}", e);
                if let Ok(call_state) = self
                    .state_machine
                    .store
                    .with_session(&session_id, |session| session.call_state)
                {
                    publish_answered = !matches!(
                        call_state,
                        CallState::CancelPending | CallState::Cancelling | CallState::Cancelled
                    );
                }
            }
        }

        if publish_answered {
            publish_api_event(
                &self.app_event_publisher,
                crate::api::events::Event::CallAnswered {
                    call_id: session_id.clone(),
                    sdp: sdp_answer.clone(),
                },
            );

            // SIP_API_DESIGN_2 Phase A: parallel detailed event
            // carrying the parsed 200 OK so B2BUA / SBC code can
            // carry Allow / Supported / Session-Expires through
            // to the downstream leg.
            let detailed = build_incoming_response_from_bytes(
                session_id,
                200,
                "OK".to_string(),
                sdp_answer,
                raw_response,
            );
            publish_api_event(
                &self.app_event_publisher,
                crate::api::events::Event::CallEstablishedDetailed(detailed),
            );
        } else {
            info!(
                "Suppressing CallAnswered for {} because INVITE answer is on cancel cleanup path",
                session_id
            );
        }

        Ok(())
    }

    async fn handle_auth_required_parts(
        &self,
        session_id: SessionId,
        transaction_id: String,
        request_uri: String,
        status: u16,
        challenge: String,
        method: String,
        outbound_transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
        exact_replay: bool,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring AuthRequired for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        let tracked_method = TrackedInDialogMethod::from_label(&method);
        let exact_transaction = if tracked_method.is_some() || method.eq_ignore_ascii_case("BYE") {
            if transaction_id.is_empty() || request_uri.is_empty() {
                warn!(
                    session_id = %session_id,
                    method = safe_auth_method_label(&method),
                    "Ignoring in-dialog authentication challenge without exact correlation"
                );
                return Ok(());
            }
            let Ok(transaction) =
                transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            else {
                warn!(
                    session_id = %session_id,
                    method = safe_auth_method_label(&method),
                    "Ignoring in-dialog authentication challenge with invalid correlation"
                );
                return Ok(());
            };
            Some(transaction)
        } else {
            None
        };

        if let (Some(tracked_method), Some(transaction)) =
            (tracked_method, exact_transaction.as_ref())
        {
            if !exact_replay {
                let deferred_event = DeferredTrackedRequestEvent::AuthRequired {
                    session_id: session_id.clone(),
                    transaction_id: transaction_id.clone(),
                    request_uri: request_uri.clone(),
                    status,
                    challenge: challenge.clone(),
                    method: method.clone(),
                    outbound_transport: outbound_transport.clone(),
                };
                match self
                    .dialog_adapter
                    .outbound_request_tracker
                    .correlate_or_defer(&session_id, tracked_method, transaction, deferred_event)
                {
                    ExactTransactionLookup::Matched => {}
                    ExactTransactionLookup::Prepared => return Ok(()),
                    ExactTransactionLookup::Mismatched => {
                        debug!(
                            session_id = %session_id,
                            method = safe_auth_method_label(&method),
                            "Ignoring stale or foreign in-dialog authentication challenge"
                        );
                        return Ok(());
                    }
                    ExactTransactionLookup::Rejected => {
                        return Err(anyhow::anyhow!(
                            "in-dialog authentication replay admission rejected"
                        ));
                    }
                }
            }
        } else if method.eq_ignore_ascii_case("BYE")
            && !exact_transaction.as_ref().is_some_and(|transaction| {
                self.dialog_adapter
                    .outgoing_bye_transaction_matches(&session_id, transaction)
            })
        {
            debug!(
                session_id = %session_id,
                "Ignoring stale or foreign BYE authentication challenge"
            );
            return Ok(());
        }

        let exact_transaction_id = exact_transaction.as_ref().map(ToString::to_string);
        let exact_request_uri = (!request_uri.is_empty()).then_some(request_uri);
        self.state_machine
            .store
            .update_session_with(&session_id, |session| {
                session.pending_auth_transport = outbound_transport
                    .as_ref()
                    .map(crate::auth::SipTransportSecurityContext::from_transport_context);
                session.pending_auth_transaction_id = exact_transaction_id;
                session.pending_auth_request_uri = exact_request_uri;
            })
            .await
            .map_err(|_| anyhow::anyhow!("session disappeared while recording exact auth state"))?;

        let state_before_auth = self
            .state_machine
            .store
            .with_session(&session_id, |session| session.call_state)
            .ok();

        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::AuthRequired {
                status_code: status,
                challenge,
                method,
            },
        )
        .await
        {
            let failure_class = OutboundAuthTerminalClass::from_error(e.as_ref());
            if is_missing_credentials_for_auth_error(e.as_ref()) {
                debug!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            } else {
                error!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            }
            if matches!(state_before_auth, Some(crate::types::CallState::Initiating)) {
                self.handle_call_failed_parts(
                    session_id,
                    status,
                    CallFailureReason::OutboundInviteAuth(failure_class),
                    None,
                )
                .await?;
            } else if let (Some(tracked_method), Some(transaction)) =
                (tracked_method, exact_transaction.as_ref())
            {
                self.dialog_adapter.outbound_request_tracker.abort_matching(
                    &session_id,
                    tracked_method,
                    transaction,
                );
                self.clear_tracked_request_auth_state(&session_id, tracked_method, transaction)
                    .await;
            }
        }
        Ok(())
    }

    async fn handle_outbound_request_completed_parts(
        &self,
        session_id: SessionId,
        transaction_id: &str,
        method: &str,
        outcome: OutboundRequestOutcome,
        exact_replay: bool,
    ) -> Result<()> {
        let Some(tracked_method) = TrackedInDialogMethod::from_label(method) else {
            return Ok(());
        };
        let Ok(transaction) =
            transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        else {
            warn!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Ignoring outbound request completion with invalid correlation"
            );
            return Ok(());
        };
        if !exact_replay {
            let deferred_event = DeferredTrackedRequestEvent::Completed {
                session_id: session_id.clone(),
                transaction_id: transaction_id.to_string(),
                method: method.to_string(),
                outcome,
            };
            match self
                .dialog_adapter
                .outbound_request_tracker
                .correlate_or_defer(&session_id, tracked_method, &transaction, deferred_event)
            {
                ExactTransactionLookup::Prepared => return Ok(()),
                ExactTransactionLookup::Mismatched => {
                    debug!(
                        session_id = %session_id,
                        method = safe_auth_method_label(method),
                        "Ignoring stale or foreign outbound request completion"
                    );
                    return Ok(());
                }
                ExactTransactionLookup::Rejected => {
                    return Err(anyhow::anyhow!(
                        "in-dialog completion replay admission rejected"
                    ));
                }
                ExactTransactionLookup::Matched => {}
            }
        }
        if !self
            .dialog_adapter
            .outbound_request_tracker
            .complete_if_matches(&session_id, tracked_method, &transaction)
        {
            debug!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Exact outbound request changed before completion cleanup"
            );
            return Ok(());
        }

        self.clear_tracked_request_auth_state(&session_id, tracked_method, &transaction)
            .await;
        debug!(
            session_id = %session_id,
            method = safe_auth_method_label(method),
            outcome = outbound_request_outcome_label(outcome),
            "Released exact in-dialog request snapshot"
        );
        Ok(())
    }

    async fn clear_tracked_request_auth_state(
        &self,
        session_id: &SessionId,
        method: TrackedInDialogMethod,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        let transaction_id = transaction.to_string();
        let update = self
            .state_machine
            .store
            .update_session_with(session_id, |session| {
                clear_exact_tracked_auth_fields(session, &transaction_id);
            })
            .await;
        if update.is_err() {
            debug!(
                session_id = %session_id,
                method = ?method,
                "Exact in-dialog request completed after session removal"
            );
        }
    }

    // Dialog event handlers
    async fn handle_dialog_created(&self, event_str: &str) -> Result<()> {
        // Extract dialog_id and call_id
        let dialog_id = self
            .extract_field(event_str, "dialog_id: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let call_id = self
            .extract_field(event_str, "call_id: \"")
            .unwrap_or_else(|| "unknown".to_string());

        // Check if this is our call (session-core generated Call-ID)
        if call_id.contains("@session-core") {
            if let Some(session_id_str) = call_id.split('@').next() {
                let session_id = SessionId(session_id_str.to_string());

                // Check if session exists before processing event
                // DialogCreated may arrive before the MakeCall transition completes
                if self
                    .state_machine
                    .store
                    .lifecycle_handle(&session_id)
                    .is_none()
                {
                    debug!(
                        "DialogCreated event arrived before session {} was fully created, will be handled by state machine later",
                        session_id
                    );
                    return Ok(());
                }

                // Only trigger state transition - all logic should be in the state machine
                if let Err(e) = self
                    .state_machine
                    .process_event(&session_id, EventType::DialogCreated { dialog_id, call_id })
                    .await
                {
                    error!("Failed to process DialogCreated event: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn handle_incoming_call(&self, event_str: &str) -> Result<()> {
        // Extract fields from the event
        // Extract session_id from the event (dialog-core provides it)
        let session_id_str = self
            .extract_field(event_str, "session_id: \"")
            .unwrap_or_else(|| format!("session-{}", uuid::Uuid::new_v4()));

        // Extract dialog_id from headers since IncomingCall doesn't have a dialog_id field directly
        let (dialog_id_str, p_asserted_identity) = if let Some(headers_start) =
            event_str.find("headers: {")
        {
            let headers_section = &event_str[headers_start..];
            let dialog_id =
                if let Some(dialog_id_start) = headers_section.find("\"X-Dialog-Id\": \"") {
                    let start = dialog_id_start + "\"X-Dialog-Id\": \"".len();
                    if let Some(end) = headers_section[start..].find('"') {
                        headers_section[start..start + end].to_string()
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "unknown".to_string()
                };
            // RFC 3325 P-Asserted-Identity surfaced by dialog-core's
            // event_hub when the inbound INVITE carries one.
            let pai = if let Some(pai_start) = headers_section.find("\"P-Asserted-Identity\": \"") {
                let start = pai_start + "\"P-Asserted-Identity\": \"".len();
                headers_section[start..]
                    .find('"')
                    .map(|end| headers_section[start..start + end].to_string())
            } else {
                None
            };
            (dialog_id, pai)
        } else {
            ("unknown".to_string(), None)
        };

        // IMPORTANT: Check if this event is for OUR dialog instance.
        // Multiple peers in the same process share a GlobalEventCoordinator,
        // so every handler receives every IncomingCall event. We must only
        // process the event if the dialog was created by OUR dialog-core.
        if let Ok(dialog_uuid) = uuid::Uuid::parse_str(&dialog_id_str) {
            let rvoip_dialog_id = rvoip_sip_dialog::DialogId(dialog_uuid);

            // Check if this dialog exists in our dialog adapter's session_to_dialog map
            // If the dialog is already mapped, it means another peer is handling it
            if self
                .dialog_adapter
                .dialog_to_session
                .contains_key(&rvoip_dialog_id)
            {
                debug!(
                    "Ignoring IncomingCall for dialog {} - already handled by another peer",
                    dialog_id_str
                );
                return Ok(());
            }

            // Check if this dialog exists in our own dialog-core instance.
            // If it doesn't, the INVITE was received by a different peer's
            // dialog-core and we must not try to process it.
            if !self
                .dialog_adapter
                .dialog_api
                .dialog_manager()
                .core()
                .has_dialog(&rvoip_dialog_id)
            {
                debug!(
                    "Ignoring IncomingCall for dialog {} - not in our dialog-core",
                    dialog_id_str
                );
                return Ok(());
            }
        }

        let call_id = self
            .extract_field(event_str, "call_id: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let from = self
            .extract_field(event_str, "from: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let to = self
            .extract_field(event_str, "to: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let sdp = self
            .extract_field(event_str, "sdp_offer: Some(\"")
            .map(|s| {
                s.replace("\\r\\n", "\r\n")
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
            });
        let transaction_id = self
            .extract_field(event_str, "transaction_id: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let _source_addr = self
            .extract_field(event_str, "source_addr: \"")
            .unwrap_or_else(|| "127.0.0.1:5060".to_string());

        // Use the session ID provided by dialog-core
        let session_id = SessionId(session_id_str);
        let authenticated_principal = transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .ok()
            .and_then(|transaction_id| {
                self.dialog_adapter
                    .dialog_api
                    .dialog_manager()
                    .core()
                    .transaction_manager()
                    .peek_inbound_principal(&transaction_id)
            });

        // Create session in store - this is the ONLY place we create sessions outside state machine
        self.state_machine
            .store
            .create_session(session_id.clone(), Role::UAS, true)
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to create session: {}", e)))?;

        // IMPORTANT: Populate the session with URIs before processing events
        // The state machine's CreateDialog action requires these fields
        let pending_transaction =
            match transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>() {
                Ok(transaction_id) => Some(transaction_id),
                Err(_) => {
                    let (transaction_class, transaction_bytes) =
                        transaction_id_diagnostics(&transaction_id);
                    debug!(
                        session = %session_id,
                        transaction_class,
                        transaction_bytes,
                        "IncomingCall carried an unparsable transaction id"
                    );
                    None
                }
            };

        // Store session data for SimplePeer event while publishing one exact
        // revision rather than cloning and replacing the complete state.
        let session_remote_sdp = self
            .state_machine
            .store
            .update_session_with(&session_id, |session| {
                session.local_uri = Some(to.clone()); // The "To" header is us (answerer)
                session.remote_uri = Some(from.clone()); // The "From" header is the caller
                session.incoming_invite_received_at = Some(Instant::now());
                session.pending_inbound_invite_transaction_id = pending_transaction;
                session.remote_sdp.clone()
            })
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to update session URIs: {}", e))
            })?;

        // Parse dialog UUID for registry mapping
        let dialog_uuid =
            uuid::Uuid::parse_str(&dialog_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        let our_dialog_id = DialogId(dialog_uuid);
        let rvoip_dialog_id = rvoip_sip_dialog::DialogId::from(our_dialog_id);
        let lifecycle = self
            .state_machine
            .store
            .lifecycle_token(&session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
        let incoming_info = crate::types::IncomingCallInfo {
            session_id: session_id.clone(),
            from: from.clone(),
            to: to.clone(),
            call_id: call_id.clone(),
            dialog_id: our_dialog_id,
            p_asserted_identity: p_asserted_identity.clone(),
        };
        self.commit_exact_inbound_dialog(
            &lifecycle,
            our_dialog_id,
            rvoip_dialog_id,
            call_id.clone(),
            incoming_info,
        )?;

        // Process the event - state machine will handle the rest
        let event_type = if self.fast_auto_accept_incoming_calls {
            EventType::IncomingCallAutoAccept {
                from: from.clone(),
                sdp,
            }
        } else {
            EventType::IncomingCall {
                from: from.clone(),
                sdp,
            }
        };

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, event_type)
            .await
        {
            error!("Failed to process incoming call event: {}", e);
            if let Some(coordinator) = self.coordinator.get().and_then(|weak| weak.upgrade()) {
                if let Err(cleanup_err) = crate::api::unified::release_exact_local_resources(
                    Arc::clone(&self.state_machine.store),
                    Arc::clone(&coordinator.helpers),
                    Arc::clone(&self.dialog_adapter),
                    Arc::clone(&self.media_adapter),
                    lifecycle,
                )
                .await
                {
                    warn!(%cleanup_err, "exact failed-inbound cleanup was incomplete");
                }
            } else {
                warn!("exact failed-inbound cleanup had no coordinator owner");
            }
        } else {
            if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
                coordinator
                    .schedule_inbound_setup_timeout(&session_id)
                    .await;
            }
            // Publish IncomingCall event to the global coordinator's "session_to_app" channel.
            // All active subscribers (StreamPeer, CallbackPeer, etc.) will receive it.
            debug!("🔍 [DEBUG] Publishing IncomingCall event to global coordinator");
            self.app_event_publisher
                .publish(crate::api::events::Event::IncomingCall {
                    call_id: session_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    sdp: session_remote_sdp,
                });
            if let Some(principal) = authenticated_principal {
                self.app_event_publisher.publish(
                    crate::api::events::Event::IncomingCallAuthenticated {
                        call_id: session_id.clone(),
                        principal,
                    },
                );
            }

            // Legacy incoming call notification (keep for compatibility)
            if let Some(ref tx) = self.incoming_call_tx {
                info!(
                    "Sending incoming call notification for session {}",
                    session_id
                );
                let call_info = crate::types::IncomingCallInfo {
                    session_id: session_id.clone(),
                    from,
                    to,
                    call_id,
                    dialog_id: DialogId(dialog_uuid),
                    p_asserted_identity,
                };
                if let Err(e) = tx.try_send(call_info) {
                    debug!(
                        "Legacy incoming_call_tx not ready — caller is using app_event_publisher path: {}",
                        e
                    );
                } else {
                    info!("Successfully sent incoming call notification");
                }
            } else {
                warn!("No incoming_call_tx channel available to send notification");
            }
        }

        Ok(())
    }

    async fn handle_call_established(&self, event_str: &str) -> Result<()> {
        info!(
            "🎯 [handle_call_established] Called with event: {}",
            event_str
        );

        // Extract session_id field from event
        // Dialog-core's event_hub retrieves the actual session_id via dialog_manager.get_session_id()
        // This is the real session ID in "session-XXX" format, not a dialog_id!
        let session_id_str = self
            .extract_session_id(event_str)
            .unwrap_or_else(|| "unknown".to_string());

        info!(
            "🎯 [handle_call_established] Extracted session_id: {}",
            session_id_str
        );

        if session_id_str == "unknown" {
            error!("Cannot extract session_id from CallEstablished event");
            return Ok(());
        }

        let session_id = SessionId(session_id_str);

        // Skip if this session isn't ours — multiple peers share the global event bus
        if self
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .is_none()
        {
            debug!(
                "Ignoring CallEstablished for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        info!(
            "🎯 [handle_call_established] Processing CallEstablished for session {}",
            session_id
        );

        let sdp_answer = self
            .extract_field(event_str, "sdp_answer: Some(\"")
            .map(|s| {
                s.replace("\\r\\n", "\r\n")
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
            });

        // Store remote SDP if present
        if let Some(sdp) = &sdp_answer {
            info!(
                "Stored remote SDP from CallEstablished for session {}",
                session_id
            );
            // Update the session with remote SDP
            let _ = self
                .state_machine
                .store
                .update_session_with(&session_id, |session| {
                    session.remote_sdp = Some(sdp.clone());
                })
                .await;
        }

        // CallEstablished maps to Dialog200OK for state machine processing.
        // If this is a late 200 OK after local cancel intent, dialog-core has
        // already sent the required ACK; the state table sends BYE and we must
        // not surface the call as answered.
        let mut publish_answered = true;
        match self
            .state_machine
            .process_event(&session_id, EventType::Dialog200OK)
            .await
        {
            Ok(result) => {
                publish_answered = !matches!(
                    result.old_state,
                    CallState::CancelPending | CallState::Cancelling
                ) && !matches!(
                    result.next_state,
                    Some(CallState::CancelPending | CallState::Cancelling | CallState::Cancelled)
                );
            }
            Err(e) => {
                error!("Failed to process CallEstablished as Dialog200OK: {}", e);
                if let Ok(call_state) = self
                    .state_machine
                    .store
                    .with_session(&session_id, |session| session.call_state)
                {
                    publish_answered = !matches!(
                        call_state,
                        CallState::CancelPending | CallState::Cancelling | CallState::Cancelled
                    );
                }
            }
        }

        // Publish CallAnswered event to the global coordinator's "session_to_app" channel.
        if publish_answered {
            debug!("🔍 [DEBUG] Publishing CallAnswered event to global coordinator");
            let api_event = crate::api::events::Event::CallAnswered {
                call_id: session_id.clone(),
                sdp: sdp_answer,
            };
            self.app_event_publisher.publish(api_event);
        } else {
            info!(
                "Suppressing CallAnswered for {} because INVITE answer is on cancel cleanup path",
                session_id
            );
        }

        Ok(())
    }

    /// Handle a 401/407 SIP auth challenge (RFC 3261 §22.2) surfaced by
    /// dialog-core as `DialogToSessionEvent::AuthRequired`. Parses the raw
    /// challenge + status from the debug-formatted event string and drives
    /// the state machine through the shared `AuthRequired` transition. The
    /// action layer (`StoreAuthChallenge` + `SendINVITEWithAuth`,
    /// `SendREGISTERWithAuth`, or `SendRequestWithAuth`) takes it from there.
    ///
    /// Method-agnostic: session state (`Initiating` / `Registering`)
    /// disambiguates whether this retries INVITE or REGISTER.
    async fn handle_auth_required(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from AuthRequired event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring AuthRequired for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        let status = self
            .extract_field(event_str, "status_code: ")
            .and_then(|s| {
                s.split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse::<u16>().ok())
            })
            .unwrap_or(401);
        let challenge = self
            .extract_debug_string_field(event_str, "challenge: \"")
            .unwrap_or_default();
        let method = self
            .extract_debug_string_field(event_str, "method: \"")
            .unwrap_or_default();

        if TrackedInDialogMethod::from_label(&method).is_some()
            || method.eq_ignore_ascii_case("BYE")
        {
            warn!(
                session_id = %session_id,
                method = safe_auth_method_label(&method),
                "Ignoring legacy authentication event without exact transaction ownership"
            );
            return Ok(());
        }

        info!(
            "🎯 [handle_auth_required] session={} status={} method={} challenge.len={}",
            session_id,
            status,
            safe_auth_method_label(&method),
            challenge.len()
        );

        let _ = self
            .state_machine
            .store
            .update_session_with(&session_id, |session| {
                session.pending_auth_transport = None;
            })
            .await;

        let state_before_auth = self
            .state_machine
            .store
            .with_session(&session_id, |session| session.call_state)
            .ok();

        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::AuthRequired {
                status_code: status,
                challenge,
                method,
            },
        )
        .await
        {
            let failure_class = OutboundAuthTerminalClass::from_error(e.as_ref());
            if is_missing_credentials_for_auth_error(e.as_ref()) {
                debug!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            } else {
                error!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            }
            if matches!(state_before_auth, Some(crate::types::CallState::Initiating)) {
                self.handle_call_failed_parts(
                    session_id,
                    status,
                    CallFailureReason::OutboundInviteAuth(failure_class),
                    None,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Handle a 3xx/4xx/5xx/6xx final failure response for an outgoing request.
    /// Drives the state machine through the appropriate `Dialog{4,5,6}xxFailure`
    /// transition and publishes an app-level `CallFailed` event so peer
    /// subscribers (StreamPeer, CallbackPeer) learn the call was rejected.
    async fn handle_call_failed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from CallFailed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        let status = self
            .extract_field(event_str, "status_code: ")
            .and_then(|s| {
                s.split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse::<u16>().ok())
            })
            .unwrap_or(500);
        let reason = self
            .extract_field(event_str, "reason_phrase: \"")
            .unwrap_or_else(|| "Failure".to_string());

        self.handle_call_failed_parts(
            session_id,
            status,
            CallFailureReason::Protocol(reason),
            None,
        )
        .await
    }

    async fn handle_call_failed_parts(
        &self,
        session_id: SessionId,
        status: u16,
        reason: CallFailureReason,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring CallFailed for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        let reason = reason.into_event_reason();

        info!(
            "[handle_call_failed] session={} status={} {:?}",
            session_id,
            status,
            CallFailureDiagnostics::new(&reason)
        );

        // RFC 3261 §14.1 — a non-2xx response to a *re-INVITE* (e.g. during
        // hold/resume) is NOT terminal for the call. The session parameters
        // remain unchanged and the call continues. Check the state before
        // the state-machine transition so we read the pre-rollback state;
        // `HoldPending` / `Resuming` identify an in-flight re-INVITE.
        let is_mid_call_reinvite_failure = self
            .state_machine
            .store
            .with_session(&session_id, |session| {
                matches!(
                    session.call_state,
                    crate::types::CallState::HoldPending | crate::types::CallState::Resuming
                )
            })
            .unwrap_or(false);

        // Drive the existing Dialog{4,5,6}xxFailure state transitions. 3xx
        // currently maps onto the 4xx path because the default state table
        // has no dedicated redirect transition; proper 3xx/redirect handling
        // is a separate feature.
        let event_type = match status {
            300..=499 => EventType::Dialog4xxFailure(status),
            500..=599 => EventType::Dialog5xxFailure(status),
            600..=699 => EventType::Dialog6xxFailure(status),
            _ => EventType::DialogError(format!("unexpected CallFailed status {}", status)),
        };

        let mut state_machine_published_cancelled = false;
        match self
            .state_machine
            .process_event(&session_id, event_type)
            .await
        {
            Ok(result) => {
                state_machine_published_cancelled = result
                    .events_published
                    .iter()
                    .any(|event| matches!(event, EventTemplate::CallCancelled));
            }
            Err(e) => {
                error!(
                    "Failed to process CallFailed({}) for session {}: {}",
                    status, session_id, e
                );
            }
        }

        if is_mid_call_reinvite_failure {
            // Re-INVITE failed mid-call; state machine has already rolled
            // us back to Active / OnHold. Don't publish a terminal
            // `CallFailed` (the call is still alive) and don't release
            // the session from the store.
            debug!(
                "session {} re-INVITE failed with {}; rolled back per RFC 3261 §14.1 — not releasing session",
                session_id, status
            );
            return Ok(());
        }

        if state_machine_published_cancelled {
            let api_event = crate::api::events::Event::CallCancelled {
                call_id: session_id.clone(),
            };
            self.publish_and_release_session(api_event, session_id.clone())
                .await;
            return Ok(());
        }

        // RFC 3515 §2.4.5 — if this session is a transfer leg, surface
        // the failure back to the transferor via a final sipfrag NOTIFY
        // and publish `Event::TransferFailed`. Done here rather than in
        // the state-machine action so this adapter-level path reliably runs
        // for every terminal failure routed through `handle_call_failed`.
        if let Ok(Some(transferor)) = self
            .state_machine
            .store
            .with_session(&session_id, |session| session.transferor_session_id.clone())
        {
            if let Err(e) = self
                .dialog_adapter
                .send_refer_notify(&transferor, status, &reason)
                .await
            {
                tracing::warn!(
                    "Failed to send transfer-failure NOTIFY to transferor {}: {}",
                    transferor,
                    e
                );
            }
            let api_event = crate::api::events::Event::TransferFailed {
                call_id: transferor,
                reason: reason.clone(),
                status_code: status,
            };
            if let Err(e) = self.app_event_publisher.publish_now(api_event).await {
                tracing::warn!("Failed to publish TransferFailed event: {}", e);
            }
        }

        // Publish app-level CallFailed for any StreamPeer/CallbackPeer subscribers,
        // then release the session from the store + registry. Publish runs first
        // so subscribers receive the terminal event before the session vanishes.
        let api_event = crate::api::events::Event::CallFailed {
            call_id: session_id.clone(),
            status_code: status,
            reason: reason.clone(),
        };

        // SIP_API_DESIGN_2 Phase A: also publish the detailed view so
        // applications can inspect Retry-After / Warning / Reason on
        // the failure response. Published before the legacy variant so
        // subscribers see both before the session is released.
        let detailed = build_incoming_response_from_bytes(
            session_id.clone(),
            status,
            reason.clone(),
            None,
            raw_response,
        );
        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::CallFailedDetailed(detailed),
        );

        self.publish_and_release_session(api_event, session_id.clone())
            .await;

        Ok(())
    }

    /// Handle a 3xx redirect response (RFC 3261 §8.1.3.4) with the
    /// typed cross-crate event payload. Bypasses the legacy debug-
    /// string parser: `status_code` and `targets` arrive as already-
    /// structured fields from `DialogToSessionEvent::CallRedirected`,
    /// which dialog-core's event hub builds straight from typed
    /// Contact headers (with q-values per RFC 3261 §20.10).
    async fn handle_call_redirected_typed(
        &self,
        session_id_str: &str,
        status_code: u16,
        targets: &[String],
        _q_values: &[f32],
    ) -> Result<()> {
        let session_id = SessionId(session_id_str.to_string());

        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring CallRedirected for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        info!(
            session_id = %session_id,
            status_code,
            target_count = targets.len(),
            "Handling typed SIP redirect"
        );

        if targets.is_empty() {
            // No usable Contact URIs in the 3xx — fall back to the
            // generic failure path so the state machine tears the call
            // down cleanly instead of hanging waiting for a retry.
            warn!("3xx response with no Contact URIs — treating as failure");
            let _ = process_event_on_fresh_task(
                Arc::clone(&self.state_machine),
                session_id,
                EventType::Dialog4xxFailure(status_code),
            )
            .await;
            return Ok(());
        }

        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::Dialog3xxRedirect {
                status: status_code,
                targets: targets.to_vec(),
            },
        )
        .await
        {
            error!(
                "Failed to process CallRedirected for session {}: {}",
                session_id, e
            );
        }

        Ok(())
    }

    async fn handle_session_interval_too_small_parts(
        &self,
        session_id: SessionId,
        min_se_secs: u32,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring SessionIntervalTooSmall for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        const CAP: u8 = 2;
        let current_retries = self
            .state_machine
            .store
            .with_session(&session_id, |session| session.session_timer_retry_count)
            .unwrap_or(CAP);
        let can_retry = min_se_secs > 0 && current_retries < CAP;

        if can_retry {
            if let Err(e) = process_event_on_fresh_task(
                Arc::clone(&self.state_machine),
                session_id.clone(),
                EventType::SessionIntervalTooSmall { min_se_secs },
            )
            .await
            {
                error!(
                    "Failed to dispatch SessionIntervalTooSmall retry for session {}: {}",
                    session_id, e
                );
            } else {
                return Ok(());
            }
        }

        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::Dialog4xxFailure(422),
        )
        .await
        {
            error!(
                "Failed to process 422 SessionIntervalTooSmall fallback for session {}: {}",
                session_id, e
            );
        }

        let api_event = crate::api::events::Event::CallFailed {
            call_id: session_id.clone(),
            status_code: 422,
            reason: format!(
                "Session Interval Too Small (required Min-SE: {}s)",
                min_se_secs
            ),
        };
        self.publish_and_release_session(api_event, session_id)
            .await;

        Ok(())
    }

    async fn handle_reinvite_glare_session(&self, session_id: SessionId) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring ReinviteGlare for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::ReinviteGlare)
            .await
        {
            error!(
                "Failed to process ReinviteGlare for session {}: {}",
                session_id, e
            );
        }
        Ok(())
    }

    async fn handle_session_refreshed_parts(
        &self,
        session_id: SessionId,
        expires_secs: u32,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::SessionRefreshed {
                call_id: session_id,
                expires_secs,
            },
        );
        Ok(())
    }

    async fn handle_session_refresh_failed_parts(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::SessionRefreshFailed {
                call_id: session_id,
                reason,
            },
        );
        Ok(())
    }

    async fn handle_outbound_flow_failed_parts(&self, aor: String, reason: String) -> Result<()> {
        let now = Instant::now();
        if let Some(prev) = self
            .outbound_flow_last_refresh
            .get(&aor)
            .map(|e| *e.value())
        {
            if now.duration_since(prev) < OUTBOUND_FLOW_REFRESH_DEBOUNCE {
                debug!(
                    aor_present = !aor.is_empty(),
                    aor_bytes = aor.len(),
                    reason_present = !reason.is_empty(),
                    reason_bytes = reason.len(),
                    elapsed_ms = now.duration_since(prev).as_millis(),
                    "OutboundFlowFailed debounced"
                );
                return Ok(());
            }
        }
        self.outbound_flow_last_refresh.insert(aor.clone(), now);

        let matching_session_id = self.state_machine.store.sessions.iter().find_map(|entry| {
            let state = entry.value().snapshot();
            match state.local_uri.as_deref() {
                Some(uri) if uri == aor.as_str() => Some(entry.key().clone()),
                _ => None,
            }
        });

        let Some(session_id) = matching_session_id else {
            warn!(
                aor_present = !aor.is_empty(),
                aor_bytes = aor.len(),
                "OutboundFlowFailed had no matching registration session; dropping"
            );
            return Ok(());
        };

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::RefreshRegistration)
            .await
        {
            warn!(
                "Failed to dispatch RefreshRegistration for session {} after flow failure: {}",
                session_id, e
            );
        }
        Ok(())
    }

    /// Handle RFC 4028 §6 — 422 Session Interval Too Small. The UAS requires
    /// a session interval larger than we offered; its `Min-SE:` header
    /// (surfaced as `min_se_secs`) carries the floor.
    ///
    /// RFC 4028 §6 — UAS replied 422 Session Interval Too Small. Two paths:
    ///
    /// 1. **Auto-retry** (usual path): if the response carries a parseable
    ///    `Min-SE` and the session's retry counter is below the cap, dispatch
    ///    `SessionIntervalTooSmall { min_se_secs }` to the state machine.
    ///    `SendINVITEWithBumpedSessionExpires` re-issues the INVITE with the
    ///    peer's floor and the 2-retry cap lives in that action.
    ///
    /// 2. **Terminal fallback**: when `Min-SE` is missing/zero or the retry
    ///    cap has already been hit, route through the generic
    ///    `Dialog4xxFailure(422)` path and publish a terminal `CallFailed`
    ///    so the app can observe the 422 status. Mirrors how dialog-core's
    ///    `event_hub.rs` already degrades gracefully on malformed 422s.
    async fn handle_session_interval_too_small(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionIntervalTooSmall event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring SessionIntervalTooSmall for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        // Numeric fields in the Debug output aren't quoted, so extract_field
        // (which expects `"…"`-wrapped string values) returns None. Pull the
        // digits off manually — find "min_se_secs: ", then take the leading
        // run of ASCII digits that follows.
        let min_se_secs = event_str
            .find("min_se_secs: ")
            .and_then(|idx| {
                let start = idx + "min_se_secs: ".len();
                let digits: String = event_str[start..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                digits.parse::<u32>().ok()
            })
            .unwrap_or(0);

        // Read the retry counter before the state machine runs so we can
        // decide between auto-retry and terminal failure in one place.
        const CAP: u8 = 2;
        let current_retries = self
            .state_machine
            .store
            .with_session(&session_id, |session| session.session_timer_retry_count)
            .unwrap_or(CAP);
        let can_retry = min_se_secs > 0 && current_retries < CAP;

        if can_retry {
            info!(
                "⏱️  [422 Session Interval Too Small] session={} requires Min-SE={}s — retrying (attempt {}/{})",
                session_id,
                min_se_secs,
                current_retries + 1,
                CAP
            );
            if let Err(e) = process_event_on_fresh_task(
                Arc::clone(&self.state_machine),
                session_id.clone(),
                EventType::SessionIntervalTooSmall { min_se_secs },
            )
            .await
            {
                // Retry dispatch failed — surface as terminal 422. No
                // `CallFailed` publish needed; the error path below does it.
                error!(
                    "Failed to dispatch SessionIntervalTooSmall retry for session {}: {}",
                    session_id, e
                );
            } else {
                // Successful retry dispatched — don't publish CallFailed.
                // The retry will either succeed (Dialog200OK) or re-enter
                // this handler on a second 422.
                return Ok(());
            }
        } else {
            warn!(
                "⏱️  [422 Session Interval Too Small] session={} — giving up (min_se={}s, retries={}/{}), surfacing as CallFailed",
                session_id, min_se_secs, current_retries, CAP
            );
        }

        // Terminal path: route through generic 4xx failure + publish
        // CallFailed so the session cleans up and the app observes the 422.
        if let Err(e) = process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::Dialog4xxFailure(422),
        )
        .await
        {
            error!(
                "Failed to process 422 SessionIntervalTooSmall fallback for session {}: {}",
                session_id, e
            );
        }

        let api_event = crate::api::events::Event::CallFailed {
            call_id: session_id.clone(),
            status_code: 422,
            reason: format!(
                "Session Interval Too Small (required Min-SE: {}s)",
                min_se_secs
            ),
        };
        self.publish_and_release_session(api_event, session_id.clone())
            .await;

        Ok(())
    }

    /// Handle 491 Request Pending (RFC 3261 §14.1) on a re-INVITE. The
    /// state machine's ReinviteGlare transition runs ScheduleReinviteRetry,
    /// which sleeps a random interval and re-issues the pending re-INVITE.
    async fn handle_reinvite_glare(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from ReinviteGlare event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring ReinviteGlare for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        info!(
            "🔄 [handle_reinvite_glare] session={} — scheduling re-INVITE retry",
            session_id
        );

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::ReinviteGlare)
            .await
        {
            error!(
                "Failed to process ReinviteGlare for session {}: {}",
                session_id, e
            );
        }
        Ok(())
    }

    /// Handle 487 Request Terminated — the caller CANCELed before the UAS
    /// answered. Distinct from the generic failure path so we can publish
    /// `Event::CallCancelled` (distinct "missed call" semantic for UIs).
    async fn handle_session_refreshed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionRefreshed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        // `extract_field` terminates on the next `"`, which works for quoted
        // string fields but not numeric ones — `expires_secs: 10 })` has no
        // trailing quote, so the helper returns None. Parse the digits directly.
        let expires_secs = event_str
            .find("expires_secs: ")
            .map(|idx| &event_str[idx + "expires_secs: ".len()..])
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<u32>().ok()
            })
            .unwrap_or(0);
        info!(
            "🎯 [handle_session_refreshed] session={} expires={}",
            session_id, expires_secs
        );

        let api_event = crate::api::events::Event::SessionRefreshed {
            call_id: session_id.clone(),
            expires_secs,
        };
        self.app_event_publisher.publish(api_event);
        Ok(())
    }

    /// RFC 5626 §4.4.1 — handle an OutboundFlowFailed event by triggering
    /// a fresh REGISTER against the matching session. Debounced per AoR
    /// over a 1s window so storms of pong-timeout + connection-closed
    /// events collapse to a single re-REGISTER, rather than hammering
    /// the registrar.
    async fn handle_outbound_flow_failed(&self, event_str: &str) -> Result<()> {
        let Some(aor) = self.extract_field(event_str, "aor: \"") else {
            warn!("Could not extract aor from OutboundFlowFailed event");
            return Ok(());
        };
        let reason = self
            .extract_field(event_str, "reason: \"")
            .unwrap_or_else(|| "Unknown".to_string());

        // Debounce: drop if we already kicked off a refresh for this
        // AoR within the last second. Otherwise stamp the refresh time
        // *before* dispatching so a parallel event racing in on the
        // same channel observes it.
        let now = Instant::now();
        if let Some(prev) = self
            .outbound_flow_last_refresh
            .get(&aor)
            .map(|e| *e.value())
        {
            if now.duration_since(prev) < OUTBOUND_FLOW_REFRESH_DEBOUNCE {
                debug!(
                    aor_present = !aor.is_empty(),
                    aor_bytes = aor.len(),
                    reason_present = !reason.is_empty(),
                    reason_bytes = reason.len(),
                    elapsed_ms = now.duration_since(prev).as_millis(),
                    "OutboundFlowFailed debounced"
                );
                return Ok(());
            }
        }
        self.outbound_flow_last_refresh.insert(aor.clone(), now);

        // Find the registration session whose local_uri matches the
        // AoR. Registrations are rare and typically 1 per coordinator,
        // so a linear scan is fine.
        let matching_session_id = self.state_machine.store.sessions.iter().find_map(|entry| {
            let state = entry.value().snapshot();
            match state.local_uri.as_deref() {
                Some(uri) if uri == aor.as_str() => Some(entry.key().clone()),
                _ => None,
            }
        });

        let Some(session_id) = matching_session_id else {
            warn!(
                aor_present = !aor.is_empty(),
                aor_bytes = aor.len(),
                "OutboundFlowFailed had no matching registration session; dropping"
            );
            return Ok(());
        };

        info!(
            aor_present = !aor.is_empty(),
            aor_bytes = aor.len(),
            reason_present = !reason.is_empty(),
            reason_bytes = reason.len(),
            session = %session_id,
            "OutboundFlowFailed triggered re-REGISTER"
        );
        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::RefreshRegistration)
            .await
        {
            warn!(
                "Failed to dispatch RefreshRegistration for session {} after flow failure: {}",
                session_id, e
            );
        }
        Ok(())
    }

    async fn handle_session_refresh_failed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionRefreshFailed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        let reason = self
            .extract_field(event_str, "reason: \"")
            .unwrap_or_else(|| "Session expired".to_string());
        debug!(
            "handle_session_refresh_failed: session={} reason={}",
            session_id, reason
        );

        let api_event = crate::api::events::Event::SessionRefreshFailed {
            call_id: session_id.clone(),
            reason,
        };
        self.app_event_publisher.publish(api_event);
        Ok(())
    }

    async fn handle_call_cancelled(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from CallCancelled event");
            return Ok(());
        };
        self.handle_call_cancelled_session(SessionId(session_id_str))
            .await
    }

    async fn handle_call_cancelled_session(&self, session_id: SessionId) -> Result<()> {
        let (initial_role, initial_state) = match self
            .state_machine
            .store
            .with_session(&session_id, |session| (session.role, session.call_state))
        {
            Ok(initial) => initial,
            Err(_) => {
                debug!(
                    "Ignoring CallCancelled for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        info!("🎯 [handle_call_cancelled] session={}", session_id);

        // Drive the Dialog487RequestTerminated transition. UAC cancellation
        // publishes only when the INVITE transaction is terminal; inbound UAS
        // CANCEL still falls back to direct publication after dialog-core has
        // already sent 200(CANCEL)+487(INVITE).
        let mut state_machine_published_cancelled = false;
        match self
            .state_machine
            .process_event(&session_id, EventType::Dialog487RequestTerminated)
            .await
        {
            Ok(result) => {
                state_machine_published_cancelled = result
                    .events_published
                    .iter()
                    .any(|event| matches!(event, EventTemplate::CallCancelled));
            }
            Err(e) => {
                error!(
                    "Failed to process CallCancelled for session {}: {}",
                    session_id, e
                );
            }
        }

        // Publish app-level CallCancelled for StreamPeer/CallbackPeer
        // subscribers, then release the session from the store + registry.
        // The response and transaction-coordination paths can both report one
        // UAC 487. The first notification commits Cancelled; reject the second
        // here while retaining the UAS fallback whose lower layer already
        // completed the wire responses.
        if !should_publish_call_cancelled(
            initial_role,
            initial_state,
            state_machine_published_cancelled,
        ) {
            debug!(
                session = %session_id,
                role = ?initial_role,
                initial_state = ?initial_state,
                "Ignoring duplicate or state-inapplicable CallCancelled notification"
            );
            return Ok(());
        }
        if state_machine_published_cancelled {
            debug!(
                "Publishing CallCancelled for {} after terminal state-table cancellation transition",
                session_id
            );
        }
        let api_event = crate::api::events::Event::CallCancelled {
            call_id: session_id.clone(),
        };
        self.publish_and_release_session(api_event, session_id.clone())
            .await;

        Ok(())
    }

    async fn handle_call_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if self.state_machine.store.lifecycle_handle(&sid).is_none() {
                debug!(
                    "Ignoring CallStateChanged for session {} - not in our store",
                    sid
                );
                return Ok(());
            }
            if event_str.contains("Ringing") {
                self.handle_call_progress_parts(sid, 180, "Ringing".to_string(), None, None)
                    .await?;
            } else if event_str.contains("Terminated") {
                // NEXT_STEPS B.2 — canonical termination event is
                // DialogTerminated. The previous dispatch of DialogBYE
                // here matched dead YAML rows; the state machine now
                // owns the resource-cleanup transitions for every
                // active-call state on DialogTerminated.
                if let Err(e) = self
                    .state_machine
                    .process_event(&sid, EventType::DialogTerminated)
                    .await
                {
                    error!("Failed to process DialogTerminated: {}", e);
                }
            }
        }
        Ok(())
    }

    async fn handle_call_progress_parts(
        &self,
        sid: SessionId,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            debug!(
                "Ignoring CallProgress for session {} - not in our store",
                sid
            );
            return Ok(());
        }

        if let Some(ref sdp_body) = sdp {
            let _ = self
                .state_machine
                .store
                .update_session_with(&sid, |session| {
                    session.remote_sdp = Some(sdp_body.clone());
                })
                .await;
        }

        let state_event = match status_code {
            183 if sdp.is_some() => Some(EventType::Dialog183SessionProgress),
            101..=199 => Some(EventType::Dialog180Ringing),
            _ => None,
        };

        if let Some(event_type) = state_event {
            if let Err(e) = self.state_machine.process_event(&sid, event_type).await {
                error!("Failed to process CallProgress for {}: {}", sid, e);
            }
        }

        let api_event = crate::api::events::Event::CallProgress {
            call_id: sid.clone(),
            status_code,
            reason: reason.clone(),
            sdp: sdp.clone(),
        };
        publish_api_event(&self.app_event_publisher, api_event);

        // SIP_API_DESIGN_2 Phase A: publish a parallel detailed event
        // carrying the parsed inbound response, so B2BUA / SBC code can
        // mirror Allow/Supported/Server/100rel markers to the
        // downstream 1xx without subscribing to a separate stream.
        let detailed =
            build_incoming_response_from_bytes(sid, status_code, reason, sdp, raw_response);
        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::CallProgressDetailed(detailed),
        );

        Ok(())
    }

    async fn handle_call_state_changed_parts(
        &self,
        sid: SessionId,
        new_state: &rvoip_infra_common::events::cross_crate::CallState,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            debug!(
                "Ignoring CallStateChanged for session {} - not in our store",
                sid
            );
            return Ok(());
        }

        let event_type = match new_state {
            rvoip_infra_common::events::cross_crate::CallState::Ringing => {
                return self
                    .handle_call_progress_parts(sid, 180, "Ringing".to_string(), None, None)
                    .await;
            }
            rvoip_infra_common::events::cross_crate::CallState::Terminated => {
                Some(EventType::DialogTerminated)
            }
            _ => None,
        };

        if let Some(event_type) = event_type {
            if let Err(e) = self.state_machine.process_event(&sid, event_type).await {
                error!("Failed to process CallStateChanged for {}: {}", sid, e);
            }
        }
        Ok(())
    }

    async fn handle_call_terminated(&self, event_str: &str) -> Result<()> {
        info!(
            "🎯 [handle_call_terminated] Called with event: {}",
            if event_str.len() > 200 {
                &event_str[..200]
            } else {
                event_str
            }
        );

        if let Some(session_id_str) = self.extract_session_id(event_str) {
            info!(
                "🎯 [handle_call_terminated] Extracted session_id: {}",
                session_id_str
            );
            let reason = self
                .extract_field(event_str, "reason: ")
                .unwrap_or_else(|| "Unknown".to_string());

            self.handle_call_terminated_parts(SessionId(session_id_str), reason)
                .await?;
        } else {
            warn!(
                "⚠️ [handle_call_terminated] Failed to extract session_id, cannot forward CallEnded event"
            );
        }

        info!("🏁 [handle_call_terminated] Completed");
        Ok(())
    }

    async fn handle_call_terminated_parts(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<()> {
        // Skip if this session isn't ours
        if self
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .is_none()
        {
            debug!(
                "Ignoring CallTerminated for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        info!(
            "🎯 [handle_call_terminated] Processing DialogTerminated for session {} with reason: {}",
            session_id, reason
        );

        // Process DialogTerminated to complete Terminating → Terminated, or
        // Cancelling → Cancelled for the late-200/ACK/BYE cleanup path.
        let mut state_machine_published_cancelled = false;
        let mut cancel_cleanup_fallback = false;
        match self
            .state_machine
            .process_event(&session_id, EventType::DialogTerminated)
            .await
        {
            Ok(result) => {
                state_machine_published_cancelled = result
                    .events_published
                    .iter()
                    .any(|event| matches!(event, EventTemplate::CallCancelled));
                cancel_cleanup_fallback = matches!(result.old_state, CallState::Cancelling)
                    || matches!(result.next_state, Some(CallState::Cancelled));
                info!(
                    "✅ [handle_call_terminated] DialogTerminated processed successfully for {}",
                    session_id
                );
            }
            Err(e) => {
                error!("Failed to process dialog terminated: {}", e);
                if let Ok(call_state) = self
                    .state_machine
                    .store
                    .with_session(&session_id, |session| session.call_state)
                {
                    cancel_cleanup_fallback =
                        matches!(call_state, CallState::Cancelling | CallState::Cancelled);
                }
            }
        }

        if state_machine_published_cancelled || cancel_cleanup_fallback {
            let api_event = crate::api::events::Event::CallCancelled {
                call_id: session_id.clone(),
            };
            self.publish_and_release_session(api_event, session_id.clone())
                .await;
            return Ok(());
        }

        // Publish CallEnded to the global coordinator's "session_to_app"
        // channel, then release the session from the store + registry.
        {
            info!(
                "🔔 [handle_call_terminated] Publishing CallEnded for session {}",
                session_id
            );
            let api_event = crate::api::events::Event::CallEnded {
                call_id: session_id.clone(),
                reason: reason.clone(),
            };
            self.publish_and_release_session(api_event, session_id.clone())
                .await;
        }

        Ok(())
    }

    async fn handle_bye_received_parts(&self, session_id: SessionId) -> Result<()> {
        let initial_state = match self
            .state_machine
            .store
            .with_session(&session_id, |session| session.call_state)
        {
            Ok(call_state) => call_state,
            Err(_) => {
                rvoip_sip_dialog::diagnostics::record_bye_cleanup_session_missing();
                debug!(
                    "Ignoring ByeReceived for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        rvoip_sip_dialog::diagnostics::record_bye_cleanup_delivered();
        let bye_guard = cleanup_diag::stage_guard(CleanupStage::ByeReceivedHandling, &session_id.0);
        match process_event_on_fresh_task(
            Arc::clone(&self.state_machine),
            session_id.clone(),
            EventType::DialogBYE,
        )
        .await
        {
            Ok(result) if !dialog_bye_requires_terminal_release(&result) => {
                let api_event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason: "BYE received".to_string(),
                };
                self.publish_and_release_session(api_event, session_id)
                    .await;
            }
            Ok(result) => {
                tracing::warn!(
                    session_id = %session_id,
                    initial_state = ?initial_state,
                    old_state = ?result.old_state,
                    next_state = ?result.next_state,
                    "DialogBYE had no matching state transition; releasing session as terminal"
                );
                let api_event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason: format!("BYE received after state mismatch ({initial_state:?})"),
                };
                self.publish_and_release_session(api_event, session_id)
                    .await;
            }
            Err(e) => {
                error!(
                    session_id = %session_id,
                    initial_state = ?initial_state,
                    error = %e,
                    "Failed to process DialogBYE; releasing session as terminal"
                );
                let api_event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason: format!(
                        "BYE received after state-machine error ({initial_state:?}): {e}"
                    ),
                };
                self.publish_and_release_session(api_event, session_id)
                    .await;
            }
        }
        bye_guard.finish_success();

        Ok(())
    }

    async fn handle_dialog_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if self.state_machine.store.lifecycle_handle(&sid).is_none() {
                debug!(
                    "Ignoring DialogError for session {} - not in our store",
                    sid
                );
                return Ok(());
            }
            let error = self
                .extract_field(event_str, "error: \"")
                .unwrap_or_else(|| "Unknown error".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::DialogError(error))
                .await
            {
                error!("Failed to process dialog error: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_dialog_error_parts(&self, sid: SessionId, error: String) -> Result<()> {
        if !self.is_our_session(&sid).await {
            debug!(
                "Ignoring DialogError for session {} - not in our store",
                sid
            );
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::DialogError(error))
            .await
        {
            error!("Failed to process dialog error: {}", e);
        }
        Ok(())
    }

    async fn handle_dtmf_received_parts(&self, sid: SessionId, tones: String) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        for digit in tones.chars() {
            publish_api_event(
                &self.app_event_publisher,
                crate::api::events::Event::DtmfReceived {
                    call_id: sid.clone(),
                    digit,
                },
            );
        }
        Ok(())
    }

    // Media event handlers
    async fn handle_media_stream_started(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::MediaSessionReady)
                .await
            {
                error!("Failed to process media stream started: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_stream_stopped(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let reason = self
                .extract_field(event_str, "reason: \"")
                .unwrap_or_else(|| "Unknown reason".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(
                    &sid,
                    EventType::MediaError(format!("Media stream stopped: {}", reason)),
                )
                .await
            {
                error!("Failed to process media stream stopped: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_flow_established(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::MediaFlowEstablished)
                .await
            {
                error!("Failed to process media flow established: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let error = self
                .extract_field(event_str, "error: \"")
                .unwrap_or_else(|| "Unknown error".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::MediaError(error))
                .await
            {
                error!("Failed to process media error: {}", e);
            }
        }
        Ok(())
    }

    // New dialog event handlers
    async fn handle_dialog_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let old_state = self
                .extract_field(event_str, "old_state: \"")
                .unwrap_or_else(|| "unknown".to_string());
            let new_state = self
                .extract_field(event_str, "new_state: \"")
                .unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(
                    &sid,
                    EventType::DialogStateChanged {
                        old_state,
                        new_state,
                    },
                )
                .await
            {
                error!("Failed to process DialogStateChanged: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_dialog_state_changed_parts(
        &self,
        sid: SessionId,
        old_state: String,
        new_state: String,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(
                &sid,
                EventType::DialogStateChanged {
                    old_state,
                    new_state,
                },
            )
            .await
        {
            error!("Failed to process DialogStateChanged: {}", e);
        }
        Ok(())
    }

    async fn handle_reinvite_received(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let previous_remote_direction = self
                .state_machine
                .store
                .with_session(&sid, |session| session.remote_media_direction)
                .ok();
            let sdp = self.extract_field(event_str, "sdp: Some(\"").map(|s| {
                s.replace("\\r\\n", "\r\n")
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
            });
            let has_sdp = sdp.is_some();
            // `method` is an uppercase SIP method string emitted by
            // dialog-core's cross-crate conversion ("INVITE" or "UPDATE").
            // Default to re-INVITE for backward compat if the field is
            // missing — INVITE is the historic payload of this event.
            let method = self
                .extract_field(event_str, "method: \"")
                .unwrap_or_else(|| "INVITE".to_string());
            let event = if method.eq_ignore_ascii_case("UPDATE") {
                EventType::UpdateReceived { sdp }
            } else {
                EventType::ReinviteReceived { sdp }
            };
            if let Err(e) = self.state_machine.process_event(&sid, event).await {
                error!(
                    "Failed to process {} (method {}): {}",
                    "ReinviteReceived/UpdateReceived", method, e
                );
            } else if method.eq_ignore_ascii_case("INVITE") && has_sdp {
                self.apply_inbound_reinvite_media_direction(&sid, previous_remote_direction)
                    .await;
            }
        }
        Ok(())
    }

    async fn handle_reinvite_received_parts(
        &self,
        sid: SessionId,
        sdp: Option<String>,
        method: String,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        let previous_remote_direction = self
            .state_machine
            .store
            .with_session(&sid, |session| session.remote_media_direction)
            .ok();
        let has_sdp = sdp.is_some();
        let event = if method.eq_ignore_ascii_case("UPDATE") {
            EventType::UpdateReceived { sdp }
        } else {
            EventType::ReinviteReceived { sdp }
        };
        if let Err(e) = self.state_machine.process_event(&sid, event).await {
            error!(
                "Failed to process ReinviteReceived/UpdateReceived (method {}): {}",
                method, e
            );
        } else if method.eq_ignore_ascii_case("INVITE") && has_sdp {
            self.apply_inbound_reinvite_media_direction(&sid, previous_remote_direction)
                .await;
        }
        Ok(())
    }

    async fn apply_inbound_reinvite_media_direction(
        &self,
        sid: &SessionId,
        previous_remote_direction: Option<crate::types::MediaDirection>,
    ) {
        let Ok((media_session_id, local_media_direction, remote_media_direction)) =
            self.state_machine.store.with_session(sid, |session| {
                (
                    session.media_session_id.clone(),
                    session.local_media_direction,
                    session.remote_media_direction,
                )
            })
        else {
            return;
        };

        if let Some(media_id) = media_session_id {
            if let Err(e) = self
                .media_adapter
                .set_media_direction(media_id, local_media_direction)
                .await
            {
                error!(
                    "Failed to apply inbound re-INVITE media direction for session {}: {}",
                    sid, e
                );
            }
        }

        let Some(previous_remote_direction) = previous_remote_direction else {
            return;
        };
        let was_remote_held = remote_direction_is_hold(previous_remote_direction);
        let is_remote_held = remote_direction_is_hold(remote_media_direction);

        let api_event = match (was_remote_held, is_remote_held) {
            (false, true) => Some(crate::api::events::Event::RemoteCallOnHold {
                call_id: sid.clone(),
            }),
            (true, false) => Some(crate::api::events::Event::RemoteCallResumed {
                call_id: sid.clone(),
            }),
            _ => None,
        };

        if let Some(api_event) = api_event {
            publish_api_event(&self.app_event_publisher, api_event);
        }
    }

    async fn handle_transfer_requested(&self, event_str: &str) -> Result<()> {
        if let Some(session_id_str) = self.extract_session_id(event_str) {
            let refer_to = self
                .extract_field(event_str, "refer_to: \"")
                .unwrap_or_else(|| "unknown".to_string());
            let transfer_type = self
                .extract_field(event_str, "transfer_type: \"")
                .unwrap_or_else(|| "blind".to_string());
            let transaction_id = self
                .extract_field(event_str, "transaction_id: \"")
                .unwrap_or_else(|| "unknown".to_string());

            self.handle_transfer_requested_parts(
                SessionId(session_id_str),
                refer_to,
                transfer_type,
                transaction_id,
                None,
                None,
                None,
                None,
            )
            .await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_transfer_requested_parts(
        &self,
        session_id: SessionId,
        refer_to: String,
        transfer_type: String,
        transaction_id: String,
        referred_by: Option<String>,
        replaces: Option<String>,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        // Skip if this session isn't ours
        if self
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .is_none()
        {
            debug!(
                "Ignoring TransferRequested for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        if let Err(e) = self
            .state_machine
            .store
            .update_session_with(&session_id, |session| {
                session.transfer_target = Some(refer_to.clone());
                session.transfer_notify_dialog = session.dialog_id;
                session.refer_transaction_id = Some(transaction_id.clone());
                session.referred_by = referred_by.clone();
                session.replaces_header = replaces.clone();
            })
            .await
        {
            error!("Failed to store transfer request fields: {}", e);
        }

        // SIP_API_DESIGN_2 Phase E: re-parse the inbound REFER bytes
        // into a typed `IncomingRequest`. The coordinator hook stays
        // `None` on the bus path; the surface consumer rehydrates it
        // before dispatching to application code.
        let request = build_incoming_request_from_bytes(session_id.clone(), raw_request, transport);

        // Publish ReferReceived event to the global coordinator's "session_to_app" channel.
        debug!("🔍 [DEBUG] Publishing ReferReceived event to global coordinator");
        self.app_event_publisher
            .publish(crate::api::events::Event::ReferReceived {
                call_id: session_id.clone(),
                refer_to: refer_to.clone(),
                referred_by: referred_by.clone(),
                replaces: replaces.clone(),
                transaction_id: transaction_id.clone(),
                transfer_type: transfer_type.clone(),
                request,
            });

        let state_machine = self.state_machine.clone();
        let session_for_default = session_id.clone();
        let refer_to_for_default = refer_to.clone();
        let transfer_type_for_default = transfer_type.clone();
        let transaction_for_default = transaction_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let should_accept = state_machine
                .store
                .with_session(&session_for_default, |session| {
                    session.refer_transaction_id.as_deref()
                        == Some(transaction_for_default.as_str())
                })
                .unwrap_or(false);

            if !should_accept {
                return;
            }

            if let Err(e) = state_machine
                .process_event(
                    &session_for_default,
                    EventType::TransferRequested {
                        refer_to: refer_to_for_default,
                        transfer_type: transfer_type_for_default,
                        transaction_id: transaction_for_default.clone(),
                    },
                )
                .await
            {
                tracing::error!(
                    "Failed to auto-accept pending TransferRequested for {}: {}",
                    session_for_default,
                    e
                );
                return;
            }

            let _ = state_machine
                .store
                .update_session_with(&session_for_default, |session| {
                    if session.refer_transaction_id.as_deref()
                        == Some(transaction_for_default.as_str())
                    {
                        session.refer_transaction_id = None;
                    }
                })
                .await;
        });
        Ok(())
    }

    async fn handle_ack_sent(&self, event_str: &str) -> Result<()> {
        // Extract dialog_id from the event
        let dialog_id_str = self
            .extract_field(event_str, "dialog_id: DialogId(")
            .or_else(|| self.extract_field(event_str, "dialog_id: \""))
            .unwrap_or_else(|| "unknown".to_string());

        // Parse the dialog ID to look up the session
        if let Ok(dialog_uuid) = uuid::Uuid::parse_str(dialog_id_str.trim_end_matches(')')) {
            let rvoip_dialog_id = rvoip_sip_dialog::DialogId(dialog_uuid);

            // Find the session ID from dialog ID
            if let Some(entry) = self.dialog_adapter.dialog_to_session.get(&rvoip_dialog_id) {
                let session_id = entry.value().clone();
                drop(entry);

                info!(
                    "ACK was sent by dialog-core for dialog {}, triggering DialogACK event for session {}",
                    dialog_id_str, session_id
                );

                // Trigger DialogACK event in state machine
                // This allows UAS to transition from "Answering" -> "Active"
                if let Err(e) = self
                    .state_machine
                    .process_event(&session_id, EventType::DialogACK)
                    .await
                {
                    error!("Failed to process DialogACK event after AckSent: {}", e);
                }
            } else {
                warn!("Received AckSent for unknown dialog {}", dialog_id_str);
            }
        }

        Ok(())
    }

    async fn handle_ack_received(&self, event_str: &str) -> Result<()> {
        // Extract session_id directly from the cross-crate event
        let session_id_str = self.extract_session_id(event_str).unwrap_or_else(|| {
            warn!("Could not extract session_id from AckReceived event");
            "unknown".to_string()
        });

        info!(
            "📨 ACK was received by dialog-core, triggering DialogACK event for session {}",
            session_id_str
        );

        // Check if this session belongs to us — multiple peers share the global event bus
        let session_id = SessionId(session_id_str.clone());
        if self
            .state_machine
            .store
            .lifecycle_handle(&session_id)
            .is_none()
        {
            debug!(
                "Ignoring AckReceived for session {} - not in our store",
                session_id_str
            );
            return Ok(());
        }

        info!("🔍 About to call process_event with DialogACK");

        // Trigger DialogACK event in state machine
        // This allows UAS to transition from "Answering" -> "Active"
        match self
            .state_machine
            .process_event(&session_id, EventType::DialogACK)
            .await
        {
            Ok(_) => {
                info!(
                    "✅ DialogACK processed successfully for session {}",
                    session_id_str
                );
                if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
                    coordinator
                        .schedule_active_call_media_timeout_if_current(&session_id)
                        .await;
                }
            }
            Err(e) => {
                error!(
                    "❌ Failed to process DialogACK event after AckReceived: {}",
                    e
                );
            }
        }

        info!(
            "🏁 Finished handle_ack_received for session {}",
            session_id_str
        );
        Ok(())
    }

    async fn handle_ack_received_session(&self, session_id: SessionId) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring AckReceived for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        rvoip_sip_dialog::diagnostics::record_ack_event_delivered();
        if let Err(error) = self
            .state_machine
            .process_event(&session_id, EventType::DialogACK)
            .await
        {
            error!(
                "Failed to process DialogACK event after AckReceived: {}",
                error
            );
        } else if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
            coordinator
                .schedule_active_call_media_timeout_if_current(&session_id)
                .await;
        }
        Ok(())
    }

    async fn handle_registration_success_parts(&self, session_id: SessionId) -> Result<()> {
        self.handle_state_event_if_ours(
            session_id,
            EventType::Registration200OK,
            "RegistrationSuccess",
        )
        .await
    }

    async fn handle_registration_failed_parts(
        &self,
        session_id: SessionId,
        status_code: u16,
    ) -> Result<()> {
        self.handle_state_event_if_ours(
            session_id,
            EventType::RegistrationFailed(status_code),
            "RegistrationFailed",
        )
        .await
    }

    // New media event handlers
    async fn handle_media_stream_started_session(&self, sid: SessionId) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::MediaSessionReady)
            .await
        {
            error!("Failed to process media stream started: {}", e);
        }
        Ok(())
    }

    async fn handle_media_stream_stopped_parts(
        &self,
        sid: SessionId,
        reason: String,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(
                &sid,
                EventType::MediaError(format!("Media stream stopped: {}", reason)),
            )
            .await
        {
            error!("Failed to process media stream stopped: {}", e);
        }
        Ok(())
    }

    async fn handle_media_flow_established_session(&self, sid: SessionId) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::MediaFlowEstablished)
            .await
        {
            error!("Failed to process media flow established: {}", e);
        }
        Ok(())
    }

    async fn handle_media_error_parts(&self, sid: SessionId, error: String) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::MediaError(error))
            .await
        {
            error!("Failed to process media error: {}", e);
        }
        Ok(())
    }

    async fn handle_media_quality_update_parts(
        &self,
        sid: SessionId,
        metrics: &rvoip_infra_common::events::cross_crate::MediaQualityMetrics,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::MediaQualityChanged {
                call_id: sid,
                packet_loss_percent: (metrics.packet_loss * 100.0) as u32,
                jitter_ms: metrics.jitter_ms as u32,
            },
        );
        Ok(())
    }

    async fn handle_media_quality_degraded_parts(
        &self,
        sid: SessionId,
        packet_loss_percent: u32,
        jitter_ms: u32,
        severity: String,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(
                &sid,
                EventType::MediaQualityDegraded {
                    packet_loss_percent,
                    jitter_ms,
                    severity,
                },
            )
            .await
        {
            error!("Failed to process MediaQualityDegraded: {}", e);
        }
        Ok(())
    }

    async fn handle_dtmf_detected_parts(
        &self,
        sid: SessionId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::DtmfDetected { digit, duration_ms })
            .await
        {
            error!("Failed to process DtmfDetected: {}", e);
        }
        Ok(())
    }

    async fn handle_rtp_timeout_parts(
        &self,
        sid: SessionId,
        last_packet_time: String,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(&sid, EventType::RtpTimeout { last_packet_time })
            .await
        {
            error!("Failed to process RtpTimeout: {}", e);
        }
        Ok(())
    }

    async fn handle_packet_loss_threshold_exceeded_parts(
        &self,
        sid: SessionId,
        loss_percentage: u32,
    ) -> Result<()> {
        if !self.is_our_session(&sid).await {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event(
                &sid,
                EventType::PacketLossThresholdExceeded { loss_percentage },
            )
            .await
        {
            error!("Failed to process PacketLossThresholdExceeded: {}", e);
        }
        Ok(())
    }

    async fn handle_media_quality_degraded(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let packet_loss_percent = self
                .extract_field(event_str, "packet_loss: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            let jitter_ms = self
                .extract_field(event_str, "jitter: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 1000.0) as u32)
                .unwrap_or(0);
            let severity = self
                .extract_field(event_str, "severity: \"")
                .unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(
                    &sid,
                    EventType::MediaQualityDegraded {
                        packet_loss_percent,
                        jitter_ms,
                        severity,
                    },
                )
                .await
            {
                error!("Failed to process MediaQualityDegraded: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_dtmf_detected(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let digit = self
                .extract_field(event_str, "digit: '")
                .and_then(|s| s.chars().next())
                .unwrap_or('?');
            let duration_ms = self
                .extract_field(event_str, "duration_ms: ")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::DtmfDetected { digit, duration_ms })
                .await
            {
                error!("Failed to process DtmfDetected: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_rtp_timeout(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let last_packet_time = self
                .extract_field(event_str, "last_packet_time: \"")
                .unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self
                .state_machine
                .process_event(&sid, EventType::RtpTimeout { last_packet_time })
                .await
            {
                error!("Failed to process RtpTimeout: {}", e);
            }
        }
        Ok(())
    }

    /// Handle `DialogToSessionEvent::NotifyReceived` (RFC 6665) — the
    /// cross-crate event dialog-core publishes after validating and
    /// 200-OK'ing an inbound NOTIFY.
    ///
    /// Always emits `Event::NotifyReceived` on the public event stream.
    /// For `event_package == "refer"` with a `message/sipfrag` body
    /// (RFC 3515 §2.4.5) additionally parses the sipfrag status line and
    /// emits `Event::ReferNotify` plus derived `ReferProgress`,
    /// `ReferCompleted`, or `TransferFailed` events so transferor apps
    /// (including b2bua wrappers) can observe the transferee's progress.
    async fn handle_notify_received(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from NotifyReceived event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);
        let event_package = self
            .extract_field(event_str, "event_package: \"")
            .unwrap_or_default();
        let subscription_state = self.extract_optional_field(event_str, "subscription_state: ");
        let content_type = self.extract_optional_field(event_str, "content_type: ");
        let body = self.extract_optional_field(event_str, "body: ");

        self.handle_notify_received_parts(
            session_id,
            event_package,
            subscription_state,
            content_type,
            body,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_notify_received_parts(
        &self,
        session_id: SessionId,
        event_package: String,
        subscription_state: Option<String>,
        content_type: Option<String>,
        body: Option<String>,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring NotifyReceived for session {} — not in our store",
                session_id
            );
            return Ok(());
        }

        // SIP_API_DESIGN_2 Phase E: re-parse the inbound NOTIFY bytes
        // into a typed `IncomingRequest`. The coordinator hook stays
        // `None`; the surface consumer rehydrates it on dispatch.
        let request = build_incoming_request_from_bytes(session_id.clone(), raw_request, transport);

        // Always surface the raw NOTIFY as a public event.
        let api_event = crate::api::events::Event::NotifyReceived {
            call_id: session_id.clone(),
            event_package: event_package.clone(),
            subscription_state: subscription_state.clone(),
            content_type: content_type.clone(),
            body: body.clone(),
            request,
        };
        publish_api_event(&self.app_event_publisher, api_event);

        if event_package.eq_ignore_ascii_case("dialog") {
            let is_dialog_info = content_type
                .as_deref()
                .map(|ct| {
                    ct.to_ascii_lowercase()
                        .contains("application/dialog-info+xml")
                })
                .unwrap_or(false);
            if is_dialog_info {
                if let Some(body) = body.as_deref() {
                    match crate::api::dialog_package::parse_dialog_info_xml(body) {
                        Ok(document) => {
                            let dialogs = document.dialogs.clone();
                            publish_api_event(
                                &self.app_event_publisher,
                                crate::api::events::Event::DialogPackageNotify {
                                    subscription_id: session_id.clone(),
                                    entity: document.entity.clone(),
                                    version: document.version,
                                    dialogs: dialogs.clone(),
                                    document,
                                },
                            );
                            for dialog in dialogs {
                                publish_api_event(
                                    &self.app_event_publisher,
                                    crate::api::events::Event::DialogStateChanged {
                                        subscription_id: session_id.clone(),
                                        dialog: dialog.clone(),
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            debug!(
                                "dialog NOTIFY body for session {} was not parseable dialog-info XML: {}",
                                session_id, e
                            );
                        }
                    }
                }
            }
        }

        // RFC 3515 §2.4.5 progress NOTIFYs carry a `message/sipfrag` body
        // containing the final-response status line of the transferee's
        // INVITE. Parse it so the transferor sees progress events
        // symmetric to what a transferee emits on the send side.
        if event_package.eq_ignore_ascii_case("refer") {
            let is_sipfrag = content_type
                .as_deref()
                .map(|ct| ct.to_ascii_lowercase().contains("message/sipfrag"))
                .unwrap_or(false);
            if is_sipfrag {
                if let Some(body) = body {
                    if let Some((status_code, reason)) = parse_sipfrag_status_line(&body) {
                        publish_api_event(
                            &self.app_event_publisher,
                            crate::api::events::Event::ReferNotify {
                                call_id: session_id.clone(),
                                status_code,
                                reason: reason.clone(),
                                subscription_state: subscription_state
                                    .clone()
                                    .map(crate::api::events::SubscriptionState::parse),
                                body: Some(body.clone()),
                            },
                        );
                        let transfer_target = self
                            .state_machine
                            .store
                            .with_session(&session_id, |session| session.transfer_target.clone())
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let transfer_event = match status_code {
                            100..=199 => {
                                let _ = self
                                    .state_machine
                                    .store
                                    .update_session_with(&session_id, |session| {
                                        session.transfer_target_progress_seen = true;
                                        session.transfer_target_last_progress =
                                            Some((status_code, reason.clone()));
                                    })
                                    .await;
                                Some(crate::api::events::Event::ReferProgress {
                                    call_id: session_id.clone(),
                                    status_code,
                                    reason,
                                })
                            }
                            200..=299 => {
                                let target_answered = self
                                    .state_machine
                                    .store
                                    .update_session_with(&session_id, |session| {
                                        let progress = session
                                            .transfer_target_progress_seen
                                            .then(|| {
                                                session.transfer_target_last_progress.clone()
                                            })
                                            .flatten();
                                        session.transfer_state = crate::session_store::state::TransferState::TransferCompleted;
                                        progress
                                    })
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|(progress_status_code, progress_reason)| {
                                        crate::api::events::Event::TransferTargetAnswered {
                                            transfer_call_id: session_id.clone(),
                                            target_uri: transfer_target.clone(),
                                            evidence: crate::api::events::TransferTargetEvidence::ReferProgressThenFinal {
                                                progress_status_code,
                                                progress_reason,
                                                final_status_code: status_code,
                                                final_reason: reason.clone(),
                                            },
                                        }
                                    });
                                if let Some(event) = target_answered {
                                    publish_api_event(&self.app_event_publisher, event);
                                }
                                Some(crate::api::events::Event::ReferCompleted {
                                    call_id: session_id.clone(),
                                    target: transfer_target,
                                    status_code,
                                    reason,
                                })
                            }
                            300..=699 => Some(crate::api::events::Event::TransferFailed {
                                call_id: session_id.clone(),
                                reason,
                                status_code,
                            }),
                            _ => None,
                        };
                        if let Some(ev) = transfer_event {
                            publish_api_event(&self.app_event_publisher, ev);
                        }
                    } else {
                        debug!(
                            "NOTIFY sipfrag body for session {} was not a parseable status line; skipping REFER-derived emission",
                            session_id
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Extract the inner value of a `field: None` / `field: Some("value")`
    /// pattern from a `{:?}` debug string. Used for optional string fields
    /// on `DialogToSessionEvent::NotifyReceived`.
    fn extract_optional_field(&self, event_str: &str, prefix: &str) -> Option<String> {
        let start = event_str.find(prefix)? + prefix.len();
        let rest = &event_str[start..];
        if rest.starts_with("None") {
            return None;
        }
        // "Some(\"...\")" — step past `Some("` and take up to the next `"`
        // not escaped. Mirrors the quick-and-dirty parsing used by the
        // other extract_* helpers on this struct; close-enough for debug
        // output roundtrip.
        let some_prefix = "Some(\"";
        if let Some(rel) = rest.find(some_prefix) {
            let val_start = rel + some_prefix.len();
            if let Some(end_rel) = rest[val_start..].find("\")") {
                return Some(rest[val_start..val_start + end_rel].to_string());
            }
        }
        None
    }

    async fn handle_packet_loss_threshold_exceeded(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await {
                return Ok(());
            }
            let loss_percentage = self
                .extract_field(event_str, "loss_percentage: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            if let Err(e) = self
                .state_machine
                .process_event(
                    &sid,
                    EventType::PacketLossThresholdExceeded { loss_percentage },
                )
                .await
            {
                error!("Failed to process PacketLossThresholdExceeded: {}", e);
            }
        }
        Ok(())
    }
}

/// Publish a non-terminal app-level event to the global coordinator's
/// `session_to_app` channel. Terminal events (`CallEnded` / `CallFailed` /
/// `CallCancelled`) go through `publish_and_release_session` instead,
/// which also frees the session-store entry after publish.
fn publish_api_event(publisher: &SessionEventPublisher, api_event: crate::api::events::Event) {
    publisher.publish(api_event);
}

/// SIP_API_DESIGN_2 Phase E — re-parse the inbound bytes carried on
/// the cross-crate variant into an `IncomingRequest`. Returns `None`
/// when the bytes are missing or unparseable; callers treat that as
/// "skip the typed event surface" rather than failing the bus
/// delivery.
fn build_incoming_request_from_bytes(
    call_id: SessionId,
    raw_request: Option<bytes::Bytes>,
    transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
) -> Option<crate::api::incoming::IncomingRequest> {
    let bytes = raw_request?;
    match rvoip_sip_core::parse_message(bytes.as_ref()) {
        Ok(rvoip_sip_core::Message::Request(req)) => {
            let from = req.from().map(|f| f.to_string()).unwrap_or_default();
            let to = req.to().map(|t| t.to_string()).unwrap_or_default();
            let method = req.method();
            Some(
                crate::api::incoming::IncomingRequest::from_bus_request(
                    call_id,
                    from,
                    to,
                    method,
                    std::sync::Arc::new(req),
                )
                .with_transport_context(sip_transport_context(&transport)),
            )
        }
        _ => None,
    }
}

/// Correlate an inbound INFO control event with the transaction encoded by
/// the preserved SIP request. The wire request is authoritative: a malformed,
/// stale, or wrong-branch event identifier is rejected and the caller may use
/// only the returned request-derived key for a fail-closed response.
fn correlate_inbound_info_transaction(
    event_transaction_id: &str,
    request: &rvoip_sip_core::Request,
) -> std::result::Result<
    rvoip_sip_dialog::transaction::TransactionKey,
    Option<rvoip_sip_dialog::transaction::TransactionKey>,
> {
    let wire_transaction = rvoip_sip_dialog::transaction::TransactionKey::from_request(request)
        .filter(|transaction| {
            transaction.is_server() && transaction.method() == &rvoip_sip_core::Method::Info
        });
    let event_transaction = event_transaction_id
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .ok();

    match (event_transaction, wire_transaction) {
        (Some(event), Some(wire)) if event == wire => Ok(wire),
        (_, wire) => Err(wire),
    }
}

fn validated_inbound_info_event_transaction(
    event_transaction_id: &str,
) -> Option<rvoip_sip_dialog::transaction::TransactionKey> {
    event_transaction_id
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .ok()
        .filter(|transaction| {
            transaction.is_server() && transaction.method() == &rvoip_sip_core::Method::Info
        })
}

fn sip_transport_context(
    transport: &Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
) -> crate::auth::SipTransportSecurityContext {
    transport
        .as_ref()
        .map(crate::auth::SipTransportSecurityContext::from_transport_context)
        .unwrap_or_default()
}

/// SIP_API_DESIGN_2 Phase A — construct an `IncomingResponse` from
/// the optional inbound bytes carried on the cross-crate variant.
/// When `raw_response` is `Some`, re-parse the bytes via
/// `rvoip_sip_core::parse_message` so applications can access typed
/// headers (Allow / Supported / Retry-After / Warning / …); when
/// `None`, fall back to a synthesized view that only carries the
/// status / reason / sdp fields.
fn build_incoming_response_from_bytes(
    call_id: SessionId,
    status_code: u16,
    reason_phrase: String,
    sdp: Option<String>,
    raw_response: Option<bytes::Bytes>,
) -> crate::api::incoming::IncomingResponse {
    use crate::api::handle::CallId;
    let call_id_view: CallId = call_id;
    match raw_response.as_ref() {
        Some(bytes) => {
            // Re-parse the inbound bytes back into a typed `Response`.
            // On parse failure (shouldn't happen — these are the
            // bytes we already accepted) fall back to the synthesized
            // view.
            match rvoip_sip_core::parse_message(bytes.as_ref()) {
                Ok(rvoip_sip_core::Message::Response(resp)) => {
                    crate::api::incoming::IncomingResponse::with_response(
                        call_id_view,
                        status_code,
                        reason_phrase,
                        sdp,
                        std::sync::Arc::new(resp),
                    )
                }
                _ => crate::api::incoming::IncomingResponse::synthetic(
                    call_id_view,
                    status_code,
                    reason_phrase,
                    sdp,
                ),
            }
        }
        None => crate::api::incoming::IncomingResponse::synthetic(
            call_id_view,
            status_code,
            reason_phrase,
            sdp,
        ),
    }
}

fn remote_direction_is_hold(direction: crate::types::MediaDirection) -> bool {
    matches!(
        direction,
        crate::types::MediaDirection::SendOnly | crate::types::MediaDirection::Inactive
    )
}

fn termination_reason_to_string(
    reason: &rvoip_infra_common::events::cross_crate::TerminationReason,
) -> String {
    match reason {
        rvoip_infra_common::events::cross_crate::TerminationReason::LocalHangup => {
            "LocalHangup".to_string()
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::RemoteHangup => {
            "RemoteHangup".to_string()
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Rejected(reason) => {
            format!("Rejected: {}", reason)
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Error(error) => {
            format!("Error: {}", error)
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Timeout => {
            "Timeout".to_string()
        }
    }
}

fn transfer_type_to_string(
    transfer_type: &rvoip_infra_common::events::cross_crate::TransferType,
) -> String {
    match transfer_type {
        rvoip_infra_common::events::cross_crate::TransferType::Blind => "blind".to_string(),
        rvoip_infra_common::events::cross_crate::TransferType::Attended => "attended".to_string(),
    }
}

/// Parse an RFC 3515 §2.4.5 sipfrag status line of the form
/// `SIP/2.0 NNN Reason\r\n...` into `(status_code, reason)`. Returns
/// `None` on any deviation (missing version, non-numeric status, empty
/// reason phrase).
fn parse_sipfrag_status_line(body: &str) -> Option<(u16, String)> {
    let first_line = body.lines().next()?.trim();
    let rest = first_line.strip_prefix("SIP/2.0")?.trim_start();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let code_part = parts.next()?;
    let reason = parts.next().unwrap_or("").trim().to_string();
    let status_code: u16 = code_part.parse().ok()?;
    if !(100..=699).contains(&status_code) {
        return None;
    }
    Some((status_code, reason))
}

#[cfg(test)]
mod tests {
    use super::{
        build_incoming_request_from_bytes, build_incoming_response_from_bytes,
        clear_exact_tracked_auth_fields, correlate_inbound_info_transaction,
        dialog_bye_requires_terminal_release, is_session_lifecycle_capacity_exhaustion,
        join_state_machine_task, map_sip_trace_session_id, parse_sipfrag_status_line,
        safe_auth_method_label, should_publish_call_cancelled, shutdown_change_requests_stop,
        sip_trace_owner_matches, validated_inbound_info_event_transaction,
        AbortStateMachineTaskOnDrop, CallFailureDiagnostics, CallFailureReason,
        DeferredReplayDelivery, OutboundAuthTerminalClass, StateMachineProcessResult,
        STATE_MACHINE_DISPATCH_JOIN_FAILURE,
    };
    use crate::adapters::outbound_request_tracker::{
        DeferredTrackedRequestEvent, ExactTransactionLookup, OutboundInDialogRequestTracker,
        TrackedInDialogMethod, TrackedInDialogOptions,
    };
    use crate::errors::SessionError;
    use crate::session_lifecycle::SessionAdmissionError;
    use crate::state_machine::ProcessEventResult;
    use crate::state_table::types::{Role, SessionId};
    use crate::state_table::{ConditionUpdates, Transition};
    use crate::types::CallState;
    use dashmap::DashMap;
    use rvoip_infra_common::events::cross_crate::{SipTraceDirection, SipTraceEvent};
    use std::sync::Arc;
    use tokio::sync::{oneshot, watch};

    fn parsed_info_request(branch: &str) -> rvoip_sip_core::Request {
        let raw = format!(
            "INFO sip:bob@example.invalid SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch={branch}\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.invalid>;tag=alice-tag\r\n\
             To: <sip:bob@example.invalid>;tag=bob-tag\r\n\
             Call-ID: info-correlation@example.invalid\r\n\
             CSeq: 2 INFO\r\n\
             Content-Length: 0\r\n\r\n"
        );
        match rvoip_sip_core::parse_message(raw.as_bytes()).expect("parse INFO request") {
            rvoip_sip_core::Message::Request(request) => request,
            rvoip_sip_core::Message::Response(_) => panic!("parsed INFO as a response"),
        }
    }

    #[test]
    fn inbound_info_correlation_accepts_only_the_wire_transaction() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");

        assert_eq!(
            correlate_inbound_info_transaction(&wire.to_string(), &request)
                .expect("matching transaction"),
            wire
        );
    }

    #[test]
    fn inbound_info_correlation_rejects_wrong_branch_with_wire_failure_target() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");
        let wrong = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-info-stale".to_string(),
            rvoip_sip_core::Method::Info,
            true,
        );

        assert_eq!(
            correlate_inbound_info_transaction(&wrong.to_string(), &request)
                .expect_err("wrong branch was accepted"),
            Some(wire)
        );
    }

    #[test]
    fn inbound_info_correlation_rejects_malformed_event_with_wire_failure_target() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");

        assert_eq!(
            correlate_inbound_info_transaction("not-a-transaction", &request)
                .expect_err("malformed event transaction was accepted"),
            Some(wire)
        );
    }

    #[test]
    fn missing_or_unparseable_info_request_uses_only_validated_event_failure_target() {
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-info-unusable-request".to_string(),
            rvoip_sip_core::Method::Info,
            true,
        );
        let transaction_id = transaction.to_string();
        let call_id = SessionId("unusable-info-request".to_string());

        assert!(build_incoming_request_from_bytes(call_id.clone(), None, None).is_none());
        assert!(build_incoming_request_from_bytes(
            call_id,
            Some(bytes::Bytes::from_static(b"not a SIP request")),
            None,
        )
        .is_none());
        assert_eq!(
            validated_inbound_info_event_transaction(&transaction_id),
            Some(transaction)
        );
        assert!(validated_inbound_info_event_transaction("not-a-transaction").is_none());
    }

    #[tokio::test]
    async fn closed_shutdown_watch_is_terminal() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        drop(shutdown_tx);
        let changed = shutdown_rx.changed().await;
        assert!(shutdown_change_requests_stop(changed, &shutdown_rx));
    }

    #[tokio::test]
    async fn queued_deferred_delivery_aborts_exact_owner_when_shard_drops_it() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("queued-deferred-shutdown".to_string());
        let lease = tracker
            .prepare(
                &session,
                TrackedInDialogOptions::Info(Arc::new(Default::default())),
            )
            .unwrap();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-queued-deferred-shutdown".to_string(),
            rvoip_sip_core::Method::Info,
            false,
        );
        let event = DeferredTrackedRequestEvent::Completed {
            session_id: session.clone(),
            transaction_id: transaction.to_string(),
            method: "INFO".to_string(),
            outcome: rvoip_infra_common::events::cross_crate::OutboundRequestOutcome::Timeout,
        };
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(&session, TrackedInDialogMethod::Info, &transaction, event,),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, transaction).unwrap();
        let queued = replay.recv().await.expect("deferred replay missing");
        drop(DeferredReplayDelivery::new(tracker.clone(), queued));
        assert_eq!(tracker.deferred_event_count(), 0);
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));
    }

    #[tokio::test]
    async fn started_deferred_delivery_aborts_owner_if_handler_is_cancelled() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("started-deferred-cancel".to_string());
        let lease = tracker
            .prepare(
                &session,
                TrackedInDialogOptions::Info(Arc::new(Default::default())),
            )
            .unwrap();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-started-deferred-cancel".to_string(),
            rvoip_sip_core::Method::Info,
            false,
        );
        let event = DeferredTrackedRequestEvent::Completed {
            session_id: session.clone(),
            transaction_id: transaction.to_string(),
            method: "INFO".to_string(),
            outcome: rvoip_infra_common::events::cross_crate::OutboundRequestOutcome::Timeout,
        };
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(&session, TrackedInDialogMethod::Info, &transaction, event,),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, transaction).unwrap();
        let queued = replay.recv().await.expect("deferred replay missing");
        let started = DeferredReplayDelivery::new(tracker.clone(), queued)
            .begin()
            .expect("exact replay should start");
        assert_eq!(tracker.deferred_event_count(), 0);
        drop(started);
        assert!(!tracker.has_request(&session, TrackedInDialogMethod::Info));
    }

    #[test]
    fn exact_auth_cleanup_preserves_newly_staged_same_method_request() {
        let mut session = crate::session_store::SessionState::new(
            SessionId("auth-cleanup-staging-race".to_string()),
            Role::UAC,
        );
        session.pending_auth_transaction_id = Some("old-tx".to_string());
        session.pending_auth_request_uri = Some("sip:target@example.invalid".to_string());
        session.pending_auth_method = Some("INFO".to_string());
        session.pending_auth = Some((401, "challenge".to_string()));
        session.request_auth_retry_count = 7;
        session.pending_info_options = Some(Arc::new(Default::default()));

        clear_exact_tracked_auth_fields(&mut session, "old-tx");

        assert!(session.pending_info_options.is_some());
        assert!(session.pending_auth_transaction_id.is_none());
        assert!(session.pending_auth_request_uri.is_none());
        assert!(session.pending_auth_method.is_none());
        assert!(session.pending_auth.is_none());
        assert_eq!(session.request_auth_retry_count, 7);
    }

    #[test]
    fn stale_completion_cannot_clear_newer_auth_owner() {
        let mut session = crate::session_store::SessionState::new(
            SessionId("auth-cleanup-exact-owner".to_string()),
            Role::UAC,
        );
        session.pending_auth_transaction_id = Some("new-tx".to_string());
        session.pending_auth_request_uri = Some("sip:new@example.invalid".to_string());
        session.pending_auth_method = Some("NOTIFY".to_string());

        clear_exact_tracked_auth_fields(&mut session, "old-tx");

        assert_eq!(
            session.pending_auth_transaction_id.as_deref(),
            Some("new-tx")
        );
        assert_eq!(session.pending_auth_method.as_deref(), Some("NOTIFY"));
    }

    #[test]
    fn lifecycle_capacity_errors_are_the_only_admission_errors_mapped_to_overload() {
        for error in [
            SessionAdmissionError::CapacityExhausted,
            SessionAdmissionError::RetainedCapacityExhausted,
        ] {
            assert!(is_session_lifecycle_capacity_exhaustion(&error));
        }

        assert!(!is_session_lifecycle_capacity_exhaustion(
            &SessionAdmissionError::AlreadyActive
        ));
        assert!(!is_session_lifecycle_capacity_exhaustion(
            &SessionError::InternalError("not-capacity".to_string())
        ));
    }

    struct DropSignal(Option<oneshot::Sender<()>>);

    #[test]
    fn duplicate_uac_cancel_is_suppressed_while_uas_fallback_is_preserved() {
        assert!(should_publish_call_cancelled(
            Role::UAC,
            CallState::Cancelling,
            true
        ));
        assert!(should_publish_call_cancelled(
            Role::UAC,
            CallState::CancelPending,
            false
        ));
        assert!(!should_publish_call_cancelled(
            Role::UAC,
            CallState::Cancelled,
            false
        ));
        assert!(should_publish_call_cancelled(
            Role::UAS,
            CallState::Ringing,
            false
        ));
        assert!(!should_publish_call_cancelled(
            Role::UAS,
            CallState::Cancelled,
            false
        ));
    }

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    #[tokio::test]
    async fn dropping_state_machine_join_aborts_the_owned_task() {
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<StateMachineProcessResult>().await
        }));

        started_rx.await.expect("auth-retry task started");
        let join = Box::pin(task.join());
        drop(join);

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("cancelled state-machine task did not stop")
            .expect("state-machine task drop signal closed");
    }

    #[tokio::test]
    async fn state_machine_task_panics_map_to_a_fixed_internal_error_class() {
        let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async {
            panic!("synthetic state-machine dispatch panic");
            #[allow(unreachable_code)]
            std::future::pending::<StateMachineProcessResult>().await
        }));

        let error = join_state_machine_task(task)
            .await
            .expect_err("panicked state-machine task must fail");
        match error.downcast_ref::<SessionError>() {
            Some(SessionError::InternalError(detail)) => {
                assert_eq!(detail, STATE_MACHINE_DISPATCH_JOIN_FAILURE);
            }
            other => panic!("unexpected state-machine join error: {other:?}"),
        }
    }

    #[test]
    fn outbound_auth_terminal_reasons_and_log_metadata_are_value_free() {
        const SECRET: &str = "terminal-auth-provider-secret-canary";
        let lower = SessionError::AuthError(format!("AKA provider failed: {SECRET}"));
        let class = OutboundAuthTerminalClass::from_error(&lower);
        assert_eq!(class, OutboundAuthTerminalClass::ChallengeResponse);

        let reason = CallFailureReason::OutboundInviteAuth(class).into_event_reason();
        assert_eq!(
            reason,
            "INVITE authentication failed (class=challenge-response)"
        );
        assert!(!reason.contains(SECRET));

        let legacy_event = crate::api::events::Event::CallFailed {
            call_id: SessionId("safe-call".to_string()),
            status_code: 401,
            reason: reason.clone(),
        };
        let detailed = build_incoming_response_from_bytes(
            SessionId("safe-call".to_string()),
            401,
            reason.clone(),
            None,
            None,
        );
        assert!(!format!("{legacy_event:?}").contains(SECRET));
        assert!(!detailed.reason_phrase.contains(SECRET));
        assert_eq!(detailed.reason_phrase, reason);

        let protocol_reason = format!("peer-controlled reason {SECRET}");
        let log_metadata = format!("{:?}", CallFailureDiagnostics::new(&protocol_reason));
        assert!(!log_metadata.contains(SECRET));
        assert!(log_metadata.contains(&format!("reason_bytes: {}", protocol_reason.len())));
        assert_eq!(safe_auth_method_label(SECRET), "extension");
    }

    #[test]
    fn auth_failure_source_has_no_lower_error_or_reason_log_relay() {
        let handler_source = include_str!("session_event_handler.rs");
        for forbidden in [
            ["Failed to process AuthRequired({}) for session {}", ": {}"].concat(),
            ["INVITE authentication failed", ": {}"].concat(),
            ["[handle_call_failed] session={} status={} ", "reason={}"].concat(),
            ["Failed to extract session_id from event", ": {}"].concat(),
        ] {
            assert!(
                !handler_source.contains(&forbidden),
                "auth diagnostic relay returned: {forbidden}"
            );
        }

        let actions_source = include_str!("../state_machine/actions.rs");
        assert_eq!(
            actions_source
                .matches("OutboundAuthOperation::Invite")
                .count(),
            1
        );
        assert_eq!(
            actions_source
                .matches("OutboundAuthOperation::Request")
                .count(),
            1
        );
        assert!(!actions_source.contains("realm={}, nonce={}"));

        let executor_source = include_str!("../state_machine/executor.rs");
        assert!(!executor_source.contains(&["Processing event ", "{:?}"].concat()));
        assert!(!executor_source.contains(&["Executing transition for {:?} + ", "{:?}"].concat()));
        assert!(!executor_source
            .contains(&["No transition defined for ", "{:?}", "\"", ", key"].concat()));

        let dialog_source = include_str!("dialog_adapter.rs");
        assert_eq!(
            dialog_source
                .matches("OutboundAuthOperation::Register")
                .count(),
            1
        );
        let unified_source = include_str!("../api/unified.rs");
        assert_eq!(
            unified_source
                .matches("OutboundAuthOperation::Request")
                .count(),
            1
        );
    }

    #[test]
    fn sipfrag_parses_progress_and_final() {
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 180 Ringing\r\n"),
            Some((180, "Ringing".into()))
        );
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 200 OK"),
            Some((200, "OK".into()))
        );
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 486 Busy Here\r\n"),
            Some((486, "Busy Here".into()))
        );
    }

    #[test]
    fn sipfrag_rejects_malformed_input() {
        assert!(parse_sipfrag_status_line("HTTP/1.1 200 OK").is_none());
        assert!(parse_sipfrag_status_line("SIP/2.0 notanumber Ringing").is_none());
        assert!(parse_sipfrag_status_line("").is_none());
    }

    #[test]
    fn sip_trace_owner_filter_accepts_only_matching_owner() {
        assert!(sip_trace_owner_matches(Some("owner-a"), "owner-a"));
        assert!(!sip_trace_owner_matches(Some("owner-a"), "owner-b"));
        assert!(!sip_trace_owner_matches(None, "owner-a"));
    }

    #[test]
    fn sip_trace_maps_sip_call_id_to_session_id() {
        let callid_to_session = DashMap::new();
        callid_to_session.insert("wire-call".into(), SessionId("session-1".into()));
        let event = trace_event(None, Some("wire-call"));

        assert_eq!(
            map_sip_trace_session_id(&event, &callid_to_session),
            Some(SessionId("session-1".into()))
        );
    }

    #[test]
    fn sip_trace_direct_session_id_wins_over_call_id_mapping() {
        let callid_to_session = DashMap::new();
        callid_to_session.insert("wire-call".into(), SessionId("mapped-session".into()));
        let event = trace_event(Some("direct-session"), Some("wire-call"));

        assert_eq!(
            map_sip_trace_session_id(&event, &callid_to_session),
            Some(SessionId("direct-session".into()))
        );
    }

    #[test]
    fn dialog_bye_without_transition_requires_terminal_release() {
        let result = ProcessEventResult {
            old_state: CallState::Ringing,
            next_state: None,
            transition: None,
            actions_executed: vec![],
            events_published: vec![],
        };

        assert!(dialog_bye_requires_terminal_release(&result));
    }

    #[test]
    fn dialog_bye_with_transition_uses_normal_state_table_cleanup() {
        let result = ProcessEventResult {
            old_state: CallState::Active,
            next_state: Some(CallState::Terminated),
            transition: Some(Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(CallState::Terminated),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![],
            }),
            actions_executed: vec![],
            events_published: vec![],
        };

        assert!(!dialog_bye_requires_terminal_release(&result));
    }

    fn trace_event(session_id: Option<&str>, sip_call_id: Option<&str>) -> SipTraceEvent {
        SipTraceEvent {
            owner_id: "owner-a".into(),
            direction: SipTraceDirection::Inbound,
            transport: "UDP".into(),
            local_addr: "127.0.0.1:5060".into(),
            remote_addr: "127.0.0.1:5080".into(),
            timestamp_unix_millis: 1,
            start_line: "INVITE sip:bob@example.com SIP/2.0".into(),
            sip_call_id: sip_call_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
            raw_message: "INVITE sip:bob@example.com SIP/2.0\n\n".into(),
            original_len: 40,
            truncated: false,
            redacted: true,
        }
    }
}
