//! Cross-transport entry point.
//!
//! Per CARVE_PLAN §6 step 4 ("Define ConnectionAdapter trait + Orchestrator
//! shell. Still no impls."): the trait surface is fully defined; the
//! Orchestrator dispatches every per-connection command through the
//! [`ConnectionAdapter`] for the connection's transport. Without a registered
//! adapter (steps 7+), commands return [`RvoipError::NoAdapterForTransport`].
//!
//! Bridging is intentionally still stubbed at this step: the cross-transport
//! frame-pump (INTERFACE_DESIGN §10.2) and the SIP-fast-path bridge strategy
//! (CARVE_PLAN §3) land in subsequent steps.

use crate::adapter::{
    AdapterEvent, AdapterLifecycleSink, ConnectionAdapter, ConnectionHandle, EndReason,
    InboundConnectionContext, OrchestratorAdapterEvent, OriginateRequest, PlaybackHandle,
    RejectReason, TransferTarget,
};
use crate::bridge::{codec_to_pt, frame_pump, BridgeManager, CrossBridgeHandle};
use crate::capability::{CapabilityDescriptor, CapabilityIntersection};
use crate::commands::{AudioSource, InboundAction, MuteDirection};
use crate::config::Config;
use crate::connection::{Direction, Transport};
use crate::conversation::{Conversation, ConversationPolicy, ConversationState};
use crate::error::{Result, RvoipError};
use crate::events::Event;
use crate::identity::AuthenticatedPrincipal;
use crate::ids::{
    BridgeId, ConnectionId, ConversationId, MediaRouteId, MessageId, ParticipantId, SessionId,
    StreamId, TenantId,
};
use crate::inbound_admission::{
    InboundAdmission, InboundAdmissionDecision, InboundAdmissionDisposition, InboundAdmissionGate,
};
use crate::media_graph::{
    start_media_graph, validate_media_graph_codec, ManagedMediaRoute, MediaGraphHandle,
    MediaGraphPolicy, MediaGraphRouteStatus,
};
use crate::message::{ContentType, Message, MessageOrigin, MessageRecipients};
use crate::participant::{Participant, ParticipantKind, ParticipantRole};
use crate::session::{ConnectionRef, Session, SessionMedium, SessionState};
use crate::stream::StreamKind;
use crate::vcon::VconBuilderHandle;
use crate::DataMessage;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent;
use rvoip_media_core::codec::transcoding::Transcoder;
use rvoip_media_core::processing::format::FormatConverter;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Weak;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock as TokioRwLock, Semaphore};
use tracing::{debug, instrument, warn};

/// Cross-crate observers must not be able to create one Tokio task per event.
/// A single lazy worker serializes publication and bounds the memory retained
/// when a coordinator or one of its handlers is slow.
const CROSS_CRATE_EVENT_QUEUE_CAPACITY: usize = 256;
const INBOUND_ADMISSION_ADAPTER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
/// Bounds active lifecycle identities plus process-lifetime retired IDs.
/// Roughly tens of MiB at typical DashMap/UUID overhead per worker.
const DEFAULT_CONNECTION_ID_BUDGET: usize = 262_144;

#[async_trait::async_trait]
trait CrossCrateEventSink: Send + Sync {
    async fn publish(&self, event: RvoipCrossCrateEvent) -> std::result::Result<(), String>;
}

#[async_trait::async_trait]
impl CrossCrateEventSink for GlobalEventCoordinator {
    async fn publish(&self, event: RvoipCrossCrateEvent) -> std::result::Result<(), String> {
        GlobalEventCoordinator::publish(self, Arc::new(event))
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CrossCrateEnqueueResult {
    Enqueued,
    DroppedFull,
    DroppedClosed,
}

struct CrossCrateEventPublisher {
    sink: Arc<dyn CrossCrateEventSink>,
    capacity: usize,
    sender: OnceLock<mpsc::Sender<RvoipCrossCrateEvent>>,
}

impl CrossCrateEventPublisher {
    fn new(sink: Arc<dyn CrossCrateEventSink>) -> Self {
        Self::with_capacity(sink, CROSS_CRATE_EVENT_QUEUE_CAPACITY)
    }

    fn with_capacity(sink: Arc<dyn CrossCrateEventSink>, capacity: usize) -> Self {
        assert!(capacity > 0, "cross-crate event queue must be non-empty");
        Self {
            sink,
            capacity,
            sender: OnceLock::new(),
        }
    }

    fn enqueue(&self, event: RvoipCrossCrateEvent) -> CrossCrateEnqueueResult {
        let sender = self.sender.get_or_init(|| {
            let (sender, mut receiver) = mpsc::channel(self.capacity);
            let sink = Arc::clone(&self.sink);
            tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    if let Err(error) = sink.publish(event).await {
                        metrics::counter!("rvoip_core_cross_crate_event_publish_failures_total")
                            .increment(1);
                        warn!(
                            %error,
                            "rvoip-core cross-crate event publish failed"
                        );
                    }
                }
            });
            sender
        });

        match sender.try_send(event) {
            Ok(()) => CrossCrateEnqueueResult::Enqueued,
            Err(mpsc::error::TrySendError::Full(_)) => {
                metrics::counter!(
                    "rvoip_core_cross_crate_events_dropped_total",
                    "reason" => "queue_full"
                )
                .increment(1);
                debug!(
                    capacity = self.capacity,
                    "cross-crate event queue full; dropping event"
                );
                CrossCrateEnqueueResult::DroppedFull
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                metrics::counter!(
                    "rvoip_core_cross_crate_events_dropped_total",
                    "reason" => "worker_closed"
                )
                .increment(1);
                warn!("cross-crate event worker closed; dropping event");
                CrossCrateEnqueueResult::DroppedClosed
            }
        }
    }
}

/// Per-connection registration tracked by the orchestrator so subsequent
/// commands (`end`, `hold`, `transfer`, `send_dtmf`, ...) can route to the
/// right adapter without the caller re-stating the transport.
#[derive(Debug)]
struct ConnectionEntry {
    transport: Transport,
    principal: Option<AuthenticatedPrincipal>,
    inbound_context: Option<InboundConnectionContext>,
    inbound_context_retired: bool,
    inbound_publication: InboundPublicationState,
    deferred_authentication: Option<DeferredAuthentication>,
    deferred_principal_authentication: Option<DeferredPrincipalAuthentication>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InboundPublicationState {
    NotInbound,
    Unseen,
    Pending(u64),
    Rejecting(u64),
    Published,
}

struct ForgottenConnection {
    was_tracked: bool,
    normalized_lifecycle_was_visible: bool,
}

struct DeferredAuthentication {
    identity_id: String,
    participant_id: String,
    assurance: crate::identity::IdentityAssurance,
    at: chrono::DateTime<Utc>,
}

impl std::fmt::Debug for DeferredAuthentication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeferredAuthentication")
            .field("identity_id", &"[redacted]")
            .field("participant_id", &"[redacted]")
            .field("assurance", &self.assurance.kind())
            .field("at", &self.at)
            .finish()
    }
}

struct DeferredPrincipalAuthentication {
    participant_id: String,
    principal: AuthenticatedPrincipal,
    at: chrono::DateTime<Utc>,
}

impl std::fmt::Debug for DeferredPrincipalAuthentication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeferredPrincipalAuthentication")
            .field("participant_id", &"[redacted]")
            .field("principal", &"[redacted]")
            .field("at", &self.at)
            .finish()
    }
}

struct PendingInboundPublication {
    connection_id: ConnectionId,
    transport: Transport,
    participant_id: Option<String>,
    principal: Option<AuthenticatedPrincipal>,
    observed_at: chrono::DateTime<Utc>,
    lifecycle: ConnectionLifecycleTicket,
}

struct ClaimedInboundRejection {
    connection_id: ConnectionId,
    transport: Transport,
    lifecycle: ConnectionLifecycleTicket,
    normalized_lifecycle_was_visible: bool,
}

enum PrincipalEventDecision {
    Handled,
    Publish,
    Drop,
    Reject(ClaimedInboundRejection),
}

enum OperationalEventDecision {
    Published,
    Drop,
    Reject(ClaimedInboundRejection),
}

enum AtomicPendingUpdate {
    Handled,
    Reject(ClaimedInboundRejection),
    TransportCollision,
}

#[derive(Debug)]
struct ConnectionLifecycleState {
    generation: u64,
    active: bool,
    retired: bool,
}

#[derive(Clone)]
struct ConnectionLifecycleTicket {
    connection_id: ConnectionId,
    generation: u64,
    state: Arc<Mutex<ConnectionLifecycleState>>,
}

/// Breaks the Orchestrator↔adapter ownership cycle while still providing a
/// direct cleanup path when an adapter's bounded event queue cannot accept a
/// terminal event.
struct OrchestratorLifecycleSink {
    orchestrator: Weak<Orchestrator>,
    transport: Transport,
}

#[async_trait::async_trait]
impl AdapterLifecycleSink for OrchestratorLifecycleSink {
    async fn deliver_terminal(&self, event: AdapterEvent) {
        let Some(orchestrator) = self.orchestrator.upgrade() else {
            return;
        };
        match &event {
            AdapterEvent::Ended { .. } | AdapterEvent::Failed { .. } => {}
            _ => {
                metrics::counter!(
                    "rvoip_core_adapter_lifecycle_fallback_rejected_total",
                    "transport" => format!("{:?}", self.transport)
                )
                .increment(1);
                warn!(
                    ?self.transport,
                    "adapter lifecycle fallback rejected a non-terminal event"
                );
                return;
            }
        }

        let transport = self.transport;
        orchestrator.handle_adapter_event(transport, event).await;
    }
}

/// RAII reservation for both connection slots in a pending bridge. The
/// ownership rows remain after commit and become the active bridge index;
/// cancellation or any setup error rolls them back automatically.
struct BridgeReservation {
    bridge_id: BridgeId,
    a: ConnectionId,
    b: ConnectionId,
    owners: Arc<DashMap<ConnectionId, BridgeId>>,
    lock: Arc<Mutex<()>>,
    committed: bool,
}

impl BridgeReservation {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for BridgeReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for connection_id in [&self.a, &self.b] {
            let owned_by_reservation = self
                .owners
                .get(connection_id)
                .is_some_and(|owner| owner.value() == &self.bridge_id);
            if owned_by_reservation {
                self.owners.remove(connection_id);
            }
        }
    }
}

pub struct Orchestrator {
    pub config: Config,
    pub bridges: BridgeManager,
    /// Cross-transport bridges — siblings of `bridges` (which holds the
    /// SIP-fast-path `BridgeHandle`s from media-core). Dropping a handle
    /// from this map aborts its two pump tasks.
    cross_bridges: Arc<DashMap<BridgeId, CrossBridgeHandle>>,
    /// Atomic connection ownership for both pending and active cross bridges.
    cross_bridge_owners: Arc<DashMap<ConnectionId, BridgeId>>,
    bridge_ownership_lock: Arc<Mutex<()>>,
    /// One graph per source Connection. Each graph owns that Connection's
    /// single-take `frames_in()` receiver and supports dynamic call,
    /// recording, UCTP, and MOQT sinks.
    media_graphs: Arc<DashMap<ConnectionId, MediaGraphHandle>>,
    /// Per-connection first-use serialization. Entries live only as long as
    /// the tracked Connection, so slow initialization on one call never
    /// blocks independent calls and the lock table remains bounded.
    media_graph_inits: Arc<DashMap<ConnectionId, Arc<tokio::sync::Mutex<()>>>>,
    pub admission: Arc<Semaphore>,
    adapters: Arc<DashMap<Transport, Arc<dyn ConnectionAdapter>>>,
    /// Transport reservations for adapter registrations whose callbacks are
    /// in progress. Trait methods are never called while this lock is held.
    adapter_registrations: Mutex<HashSet<Transport>>,
    /// Optional fail-closed policy channel installed before any adapter.
    inbound_admission_gate: OnceLock<InboundAdmissionGate>,
    /// Serializes the brief cross-map connection/lifecycle commit windows.
    /// No guard crosses an await; media and steady-state event paths do not
    /// acquire it.
    connection_registry_lock: Mutex<()>,
    connection_id_budget: AtomicUsize,
    connections: Arc<DashMap<ConnectionId, ConnectionEntry>>,
    /// Generation-checked setup commit barrier. Async setup can do slow work
    /// without holding a mutex, then atomically commit only if every source is
    /// still on the generation captured before setup began. Retired IDs stay
    /// as process-lifetime tombstones because adapter events do not yet carry
    /// a route epoch; adapters must therefore generate non-reusable IDs.
    connection_lifecycles: Arc<DashMap<ConnectionId, Arc<Mutex<ConnectionLifecycleState>>>>,
    events: broadcast::Sender<Event>,
    /// Optional bounded cross-crate publication. A single lazily-started FIFO
    /// worker publishes normalized events without peer-controlled task growth.
    cross_crate_publisher: Option<Arc<CrossCrateEventPublisher>>,
    /// Per-Session multi-party subscription routing tables. v0.x MP1 lands
    /// the data structure + API; MP2 wires the UCTP coordinator to call
    /// `add_subscription` on `stream.subscribe`; MP3 wires the media-path
    /// fanout that consults `subscribers_for`. See INTERFACE_DESIGN.md
    /// §10.6 and CONVERSATION_PROTOCOL.md §7.7.
    subscriptions: Arc<crate::subscriptions::SubscriptionRegistry>,
    /// Process-shared publisher registry — `(SessionId, strm_id) -> publisher
    /// ConnectionId`. Populated by the publishing coordinator at
    /// `stream.opened` time (MP2.6); consumed by the subscribing
    /// coordinator's `OrchestratorSubscriptionHandler` to resolve
    /// `stream.subscribe` requests. Lazily initialized via
    /// [`publisher_registry`].
    publisher_registry: std::sync::OnceLock<Arc<crate::subscriptions::PublisherRegistry>>,
    /// Per-(sid, subscriber, publisher, publisher_strm_id) →
    /// subscriber-side MediaStream allocated lazily by
    /// [`Self::fanout_frame`] (plan §12 MP3c / G4). The MediaStream is
    /// obtained via [`crate::adapter::ConnectionAdapter::allocate_subscriber_stream`]
    /// the first time a frame is fanned out on that subscription;
    /// subsequent fanouts reuse the same stream so the subscriber sees
    /// each publisher's media on a stable `stream_local_id`.
    ///
    /// For adapters that return `NotImplemented` (SIP, WebRTC, anything
    /// not UCTP-family) the map stays unused and `fanout_frame` falls
    /// back to the legacy pick-by-kind path so single-publisher rooms
    /// keep working everywhere.
    subscriber_streams: Arc<
        DashMap<
            (SessionId, ConnectionId, ConnectionId, StreamId),
            Arc<dyn crate::stream::MediaStream>,
        >,
    >,
    /// Per-Conversation live state (P1). Lookup key is the
    /// `ConversationId` returned by [`open_conversation`]. Each value is
    /// individually `RwLock`ed so lifecycle ops on different
    /// Conversations don't serialize through one global lock. The
    /// per-Conversation lock is held only for the brief read/mutate
    /// window inside a lifecycle method — never across an `.await`.
    conversations: Arc<DashMap<ConversationId, Arc<RwLock<Conversation>>>>,
    /// Per-Session live state (P1). Same locking discipline as
    /// `conversations`. Population by [`start_session`]; removal happens
    /// when the orchestrator forgets the last Connection bound to the
    /// Session (via the auto-end path in `detach_connection_from_session`)
    /// or on explicit [`end_session`] + later close.
    sessions: Arc<DashMap<SessionId, Arc<RwLock<Session>>>>,
    /// Reverse index `ConnectionId → SessionId`. Populated by
    /// [`route_inbound_connection`] when `InboundAction::Accept` carries
    /// a `session_id`; cleared in `forget_connection`. Drives
    /// [`session_of`] (P1.12) and the auto-end-on-last-leave path
    /// (P1.10).
    sessions_by_connection: Arc<DashMap<ConnectionId, SessionId>>,
    /// P3 — per-Session vCon builder.
    session_vcons: Arc<DashMap<SessionId, Arc<crate::vcon::DefaultVconBuilder>>>,
    /// P5 — provider registry (name → `Arc<dyn Provider>`). Populated
    /// by `register_asr_provider` etc. before `attach_ai` /
    /// `start_recording` / `start_transcription` resolve the name.
    asr_providers: Arc<DashMap<String, Arc<dyn crate::harness::AsrProvider>>>,
    tts_providers: Arc<DashMap<String, Arc<dyn crate::harness::TtsProvider>>>,
    dialog_managers: Arc<DashMap<String, Arc<dyn crate::harness::DialogManager>>>,
    recording_sinks: Arc<DashMap<String, Arc<dyn crate::harness::RecordingSink>>>,
    /// P5 — live recording sessions. Drop the JoinHandle on
    /// `stop_recording` to abort the pump.
    recordings: Arc<DashMap<crate::ids::RecordingId, RecordingHandle>>,
    /// P5 — live transcription sessions.
    transcriptions: Arc<DashMap<crate::ids::TranscriptionId, TranscriptionHandle>>,
    /// P5 — live AI attachments.
    ai_attachments: Arc<DashMap<crate::ids::AiAttachmentId, AiAttachmentHandle>>,
    /// P5 — per-listener channel receivers (for `ListenerSink::Channel`).
    listener_channels: Arc<
        DashMap<
            crate::ids::ListenerId,
            std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<crate::stream::MediaFrame>>>,
        >,
    >,
    /// P5 — abort handles for live listener tasks. `detach` /
    /// listener-target Connection ending fires the abort so the
    /// forwarder task doesn't leak after its source dies. Bug-fix
    /// round of the gap-plan completion sweep.
    listener_tasks: Arc<DashMap<crate::ids::ListenerId, MediaTaskHandle>>,
    /// P9 — per-Session quality accumulator. Each `AdapterEvent::Quality`
    /// updates the aggregator for the Session that owns the
    /// Connection; `end_session` snapshots + fills
    /// `SessionEnded.report`.
    session_quality: Arc<DashMap<SessionId, QualityAggregator>>,
    /// P6 — per-tenant quotas. Empty map = unlimited everywhere.
    tenant_quotas: Arc<DashMap<TenantId, crate::config::TenantQuotas>>,
    /// P6 — per-tenant Conversation index.
    conversations_by_tenant: Arc<DashMap<TenantId, dashmap::DashSet<ConversationId>>>,
    /// V2.B — per-tenant admission semaphores. When a tenant has a
    /// quota for `max_concurrent_recordings`, an `Arc<Semaphore>` is
    /// installed here with that capacity; `start_recording` acquires
    /// an `OwnedSemaphorePermit` that lives in the `RecordingHandle`
    /// and is released by Drop on `stop_recording`. Absent entry =
    /// unlimited (no admission check). Replaces the DashMap-shard-
    /// contention-bound check-then-increment from v1.
    recording_sems: Arc<DashMap<TenantId, Arc<Semaphore>>>,
    ai_sems: Arc<DashMap<TenantId, Arc<Semaphore>>>,
}

/// P5 — internal handles for live attachments.
pub(crate) struct RecordingHandle {
    pub sink: Arc<dyn crate::harness::RecordingSink>,
    pub media: MediaTapHandle,
    pub connection_ids: Vec<ConnectionId>,
    /// P5 — `false` while paused; pump task watches this and drops
    /// frames silently rather than writing them to the sink. Resumed
    /// by flipping back to `true`.
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    /// V2.B — admission permit; held while recording is live, released
    /// automatically on Drop (i.e. on `stop_recording` removal). `None`
    /// when the tenant had no `max_concurrent_recordings` quota at
    /// start time.
    pub _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
pub(crate) struct TranscriptionHandle {
    pub media: MediaTapHandle,
    pub connection_id: ConnectionId,
}

/// A graph route owned by an observer attachment. Dropping it removes only
/// that observer from the reusable source graph; bridge and broadcast routes
/// remain intact.
pub(crate) struct MediaTapRoute {
    route: Option<ManagedMediaRoute>,
}

impl MediaTapRoute {
    fn new(route: ManagedMediaRoute) -> Self {
        Self { route: Some(route) }
    }

    fn status(&self) -> Option<MediaGraphRouteStatus> {
        self.route.as_ref().map(ManagedMediaRoute::status)
    }

    fn take(&mut self) -> Option<ManagedMediaRoute> {
        self.route.take()
    }

    fn detach(&mut self) {
        self.route.take();
    }
}

impl Drop for MediaTapRoute {
    fn drop(&mut self) {
        self.detach();
    }
}

#[derive(Default)]
pub(crate) struct MediaTapHandle {
    routes: Vec<MediaTapRoute>,
    tasks: Vec<tokio::task::AbortHandle>,
}

impl MediaTapHandle {
    fn push(&mut self, route: MediaTapRoute, task: tokio::task::AbortHandle) {
        self.routes.push(route);
        self.tasks.push(task);
    }

    fn stop(&mut self) {
        drop(self.begin_stop());
    }

    fn statuses(&self) -> Vec<MediaGraphRouteStatus> {
        self.routes
            .iter()
            .filter_map(MediaTapRoute::status)
            .collect()
    }

    fn begin_stop(&mut self) -> Vec<ManagedMediaRoute> {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.routes
            .drain(..)
            .filter_map(|mut route| route.take())
            .collect()
    }

    async fn stop_and_wait(&mut self) {
        let routes = self.begin_stop();
        for route in routes {
            let _ = route.remove().await;
        }
    }
}

impl Drop for MediaTapHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct MediaTaskHandle {
    abort: tokio::task::AbortHandle,
    connection_ids: Vec<ConnectionId>,
}

impl Drop for MediaTaskHandle {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
/// P9 — running aggregator for per-Session quality samples.
/// Accumulated by `handle_adapter_event` on `AdapterEvent::Quality`
/// and snapshotted by `end_session` to populate
/// `Event::SessionEnded.report`.
#[derive(Debug, Default)]
pub(crate) struct QualityAggregator {
    pub samples: usize,
    pub jitter_ms_sum: f64,
    pub packet_loss_pct_sum: f64,
    pub mos_sum: f64,
    pub mos_samples: usize,
    pub codec: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl QualityAggregator {
    pub fn add(&mut self, snap: &crate::stream::QualitySnapshot, codec: Option<String>) {
        self.samples += 1;
        self.jitter_ms_sum += snap.jitter_ms as f64;
        self.packet_loss_pct_sum += snap.packet_loss_pct as f64;
        if let Some(mos) = snap.mos {
            self.mos_sum += mos as f64;
            self.mos_samples += 1;
        }
        if self.codec.is_none() {
            self.codec = codec;
        }
        if self.started_at.is_none() {
            self.started_at = Some(chrono::Utc::now());
        }
    }
    pub fn finish(self) -> Option<crate::events::SessionQualityReport> {
        if self.samples == 0 {
            return None;
        }
        let avg_jitter = (self.jitter_ms_sum / self.samples as f64) as f32;
        let avg_loss = (self.packet_loss_pct_sum / self.samples as f64) as f32;
        let avg_mos = if self.mos_samples > 0 {
            Some((self.mos_sum / self.mos_samples as f64) as f32)
        } else {
            None
        };
        Some(crate::events::SessionQualityReport {
            mos: avg_mos,
            packet_loss_pct: avg_loss,
            jitter_ms: avg_jitter,
            rtt_ms: None,
            codec: self.codec,
            bitrate_bps: None,
            talk_pct: None,
            silence_pct: None,
            pdd_ms: None,
            ring_time_ms: None,
            setup_time_ms: None,
            hangup_reason: None,
        })
    }
}

pub(crate) struct AiAttachmentHandle {
    pub media: MediaTapHandle,
    pub connection_id: ConnectionId,
    /// P5 — flips to `true` when a TTS playback is in flight and to
    /// `false` when it isn't. Barge-in inspects this to decide
    /// whether an incoming ASR partial should cancel a playback.
    /// Stored here only to keep the Arc alive at the orchestrator
    /// level; the dialog task holds its own clone and does all the
    /// reads. Retained so a future external "is speaking?" / "stop
    /// speaking" API can hook into it without re-plumbing the task.
    #[allow(dead_code)]
    pub speaking: Arc<std::sync::atomic::AtomicBool>,
    /// P5 — current playback cancel signal. When barge-in fires, the
    /// orchestrator sends `()` to abort the in-flight TTS pipe.
    /// Same lifetime/retention rationale as `speaking` above.
    #[allow(dead_code)]
    pub speak_cancel: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// V2.B — admission permit; released on detach via Drop.
    pub _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Orchestrator {
    pub fn new(config: Config) -> Arc<Self> {
        let admission = Arc::new(Semaphore::new(config.max_concurrent_setups));
        let (events, _rx) = broadcast::channel(1024);
        Arc::new(Self {
            config,
            bridges: BridgeManager::new(),
            cross_bridges: Arc::new(DashMap::new()),
            cross_bridge_owners: Arc::new(DashMap::new()),
            bridge_ownership_lock: Arc::new(Mutex::new(())),
            media_graphs: Arc::new(DashMap::new()),
            media_graph_inits: Arc::new(DashMap::new()),
            admission,
            adapters: Arc::new(DashMap::new()),
            adapter_registrations: Mutex::new(HashSet::new()),
            inbound_admission_gate: OnceLock::new(),
            connection_registry_lock: Mutex::new(()),
            connection_id_budget: AtomicUsize::new(DEFAULT_CONNECTION_ID_BUDGET),
            connections: Arc::new(DashMap::new()),
            connection_lifecycles: Arc::new(DashMap::new()),
            events,
            cross_crate_publisher: None,
            subscriptions: Arc::new(crate::subscriptions::SubscriptionRegistry::new()),
            publisher_registry: std::sync::OnceLock::new(),
            subscriber_streams: Arc::new(DashMap::new()),
            conversations: Arc::new(DashMap::new()),
            sessions: Arc::new(DashMap::new()),
            sessions_by_connection: Arc::new(DashMap::new()),
            session_vcons: Arc::new(DashMap::new()),
            asr_providers: Arc::new(DashMap::new()),
            tts_providers: Arc::new(DashMap::new()),
            dialog_managers: Arc::new(DashMap::new()),
            recording_sinks: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            transcriptions: Arc::new(DashMap::new()),
            ai_attachments: Arc::new(DashMap::new()),
            listener_channels: Arc::new(DashMap::new()),
            listener_tasks: Arc::new(DashMap::new()),
            session_quality: Arc::new(DashMap::new()),
            tenant_quotas: Arc::new(DashMap::new()),
            conversations_by_tenant: Arc::new(DashMap::new()),
            recording_sems: Arc::new(DashMap::new()),
            ai_sems: Arc::new(DashMap::new()),
        })
    }

    pub fn new_with_coordinator(
        config: Config,
        coordinator: Arc<GlobalEventCoordinator>,
    ) -> Arc<Self> {
        let admission = Arc::new(Semaphore::new(config.max_concurrent_setups));
        let (events, _rx) = broadcast::channel(1024);
        Arc::new(Self {
            config,
            bridges: BridgeManager::new(),
            cross_bridges: Arc::new(DashMap::new()),
            cross_bridge_owners: Arc::new(DashMap::new()),
            bridge_ownership_lock: Arc::new(Mutex::new(())),
            media_graphs: Arc::new(DashMap::new()),
            media_graph_inits: Arc::new(DashMap::new()),
            admission,
            adapters: Arc::new(DashMap::new()),
            adapter_registrations: Mutex::new(HashSet::new()),
            inbound_admission_gate: OnceLock::new(),
            connection_registry_lock: Mutex::new(()),
            connection_id_budget: AtomicUsize::new(DEFAULT_CONNECTION_ID_BUDGET),
            connections: Arc::new(DashMap::new()),
            connection_lifecycles: Arc::new(DashMap::new()),
            events,
            cross_crate_publisher: Some(Arc::new(CrossCrateEventPublisher::new(coordinator))),
            subscriptions: Arc::new(crate::subscriptions::SubscriptionRegistry::new()),
            publisher_registry: std::sync::OnceLock::new(),
            subscriber_streams: Arc::new(DashMap::new()),
            conversations: Arc::new(DashMap::new()),
            sessions: Arc::new(DashMap::new()),
            sessions_by_connection: Arc::new(DashMap::new()),
            session_vcons: Arc::new(DashMap::new()),
            asr_providers: Arc::new(DashMap::new()),
            tts_providers: Arc::new(DashMap::new()),
            dialog_managers: Arc::new(DashMap::new()),
            recording_sinks: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            transcriptions: Arc::new(DashMap::new()),
            ai_attachments: Arc::new(DashMap::new()),
            listener_channels: Arc::new(DashMap::new()),
            listener_tasks: Arc::new(DashMap::new()),
            session_quality: Arc::new(DashMap::new()),
            tenant_quotas: Arc::new(DashMap::new()),
            conversations_by_tenant: Arc::new(DashMap::new()),
            recording_sems: Arc::new(DashMap::new()),
            ai_sems: Arc::new(DashMap::new()),
        })
    }

    /// Register a transport adapter. Spawns a background task that pulls
    /// `AdapterEvent`s from the adapter's subscribe channel and normalizes
    /// them into rvoip-core [`Event`]s on the orchestrator's broadcast bus.
    /// Returns [`RvoipError::AdapterAlreadyRegistered`] on collision.
    pub fn register(self: &Arc<Self>, adapter: Arc<dyn ConnectionAdapter>) -> Result<()> {
        let transport = adapter.transport();
        {
            let mut registrations = self
                .adapter_registrations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.adapters.contains_key(&transport) || !registrations.insert(transport) {
                return Err(RvoipError::AdapterAlreadyRegistered(transport));
            }
        }
        if let Err(error) = adapter.install_lifecycle_sink(Arc::new(OrchestratorLifecycleSink {
            orchestrator: Arc::downgrade(self),
            transport,
        })) {
            self.adapter_registrations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&transport);
            return Err(error);
        }
        // The Orchestrator consumes the adapter's atomic lifecycle stream.
        // Public direct subscribers retain the pre-atomic normalized sequence.
        let mut events = adapter.subscribe_orchestrator_events();
        {
            let mut registrations = self
                .adapter_registrations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.adapters.insert(transport, adapter);
            registrations.remove(&transport);
        }

        // Spawn the per-adapter event-normalize loop. Each AdapterEvent is
        // translated into one or more rvoip-core Events and republished.
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                me.handle_orchestrator_adapter_event(transport, event).await;
            }
            debug!(?transport, "adapter event stream ended");
        });
        Ok(())
    }

    pub fn adapter(&self, transport: Transport) -> Result<Arc<dyn ConnectionAdapter>> {
        self.adapters
            .get(&transport)
            .map(|e| e.value().clone())
            .ok_or(RvoipError::NoAdapterForTransport(transport))
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Set the process-local budget for active and retired Connection IDs.
    ///
    /// Adapter events currently lack a route epoch, so retired IDs are kept
    /// for the process lifetime and may not be reused. Once this bounded
    /// registry is full, every unseen ID is rejected and drained fail-closed.
    /// Call this before the first connection is observed.
    pub fn configure_connection_id_budget(&self, maximum: usize) -> Result<()> {
        if maximum == 0 {
            return Err(RvoipError::InvalidState(
                "connection ID budget must be non-zero",
            ));
        }
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.connection_lifecycles.is_empty() {
            return Err(RvoipError::InvalidState(
                "connection ID budget must be configured before first use",
            ));
        }
        self.connection_id_budget.store(maximum, Ordering::Relaxed);
        Ok(())
    }

    /// Install one bounded, fail-closed inbound admission gate.
    ///
    /// Installation must happen before the first adapter is registered. When
    /// installed, every adapter-reported inbound connection is withheld from
    /// the normalized event bus until the receiver accepts its
    /// [`InboundAdmission`]. Queue saturation, a closed receiver, a dropped
    /// ticket, or `decision_timeout` rejects the route and erases its retained
    /// context. At most `capacity` decision waiter tasks can exist.
    ///
    /// Deployments that do not install a gate retain the historical immediate
    /// `ConnectionInbound` behavior.
    pub fn install_inbound_admission_gate(
        &self,
        capacity: usize,
        decision_timeout: Duration,
    ) -> Result<mpsc::Receiver<InboundAdmission>> {
        if capacity == 0 || capacity > Semaphore::MAX_PERMITS {
            return Err(RvoipError::InvalidState(
                "inbound admission capacity is invalid",
            ));
        }
        if decision_timeout.is_zero() {
            return Err(RvoipError::InvalidState(
                "inbound admission decision timeout is invalid",
            ));
        }

        let registrations = self
            .adapter_registrations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.adapters.is_empty() || !registrations.is_empty() {
            return Err(RvoipError::InvalidState(
                "inbound admission gate must be installed before adapters",
            ));
        }
        let (gate, receiver) = InboundAdmissionGate::new(capacity, decision_timeout);
        self.inbound_admission_gate
            .set(gate)
            .map_err(|_| RvoipError::InvalidState("inbound admission gate already installed"))?;
        Ok(receiver)
    }

    /// Look up which adapter owns a given connection. Returns
    /// [`RvoipError::ConnectionNotFound`] if the connection isn't registered.
    fn adapter_for(&self, conn: &ConnectionId) -> Result<Arc<dyn ConnectionAdapter>> {
        let entry = self
            .connections
            .get(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if entry.inbound_publication != InboundPublicationState::Published {
            return Err(RvoipError::AdmissionRejected(
                "connection is not operational",
            ));
        }
        let transport = entry.transport;
        drop(entry);
        self.adapter(transport)
    }

    /// Return the registered transport that owns a live connection. Useful to
    /// policy layers that pair heterogeneous inbound legs without depending
    /// on adapter-private route maps.
    pub fn connection_transport(&self, conn: &ConnectionId) -> Result<Transport> {
        self.connections
            .get(conn)
            .map(|entry| entry.transport)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
    }

    /// Return the complete principal last authenticated for this connection.
    ///
    /// The principal is cleared with the route on connection teardown. Policy
    /// layers should compare its ownership key rather than subject alone.
    pub fn connection_principal(&self, conn: &ConnectionId) -> Result<AuthenticatedPrincipal> {
        self.connections
            .get(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?
            .principal
            .clone()
            .ok_or(RvoipError::InvalidState(
                "connection has no authenticated principal",
            ))
    }

    /// Consume the inbound adapter context for `conn` exactly once.
    ///
    /// The authenticated caller must own the connection. A failed ownership
    /// check never consumes the context, so an unrelated tenant cannot race
    /// the legitimate policy layer. Untaken context is erased with the
    /// connection route during terminal cleanup.
    pub fn take_inbound_context(
        &self,
        conn: &ConnectionId,
        principal: &AuthenticatedPrincipal,
    ) -> Result<Option<InboundConnectionContext>> {
        if principal.is_expired() {
            return Err(RvoipError::AdmissionRejected(
                "inbound context principal is expired",
            ));
        }

        let mut entry = self
            .connections
            .get_mut(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if entry.inbound_publication != InboundPublicationState::Published {
            return Err(RvoipError::AdmissionRejected(
                "inbound context is reserved by admission policy",
            ));
        }
        let registered_principal = entry.principal.as_ref().ok_or(RvoipError::InvalidState(
            "connection has no authenticated principal",
        ))?;
        if registered_principal.is_expired()
            || !registered_principal.has_same_owner(principal)
            || registered_principal
                .tenant
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(RvoipError::AdmissionRejected(
                "inbound context principal mismatch",
            ));
        }

        if entry.inbound_context.as_ref().is_some_and(|context| {
            !context.is_bound_to(conn, entry.transport, registered_principal)
        }) {
            // A malformed adapter context is fail-closed and cannot be
            // recovered by retrying with a different principal.
            entry.inbound_context = None;
            entry.inbound_context_retired = true;
            return Err(RvoipError::AdmissionRejected(
                "inbound context binding mismatch",
            ));
        }
        let context = entry.inbound_context.take();
        entry.inbound_context_retired = true;
        Ok(context)
    }

    pub(crate) fn inbound_admission_principal(
        &self,
        conn: &ConnectionId,
        transport: Transport,
        lifecycle_generation: u64,
    ) -> Result<AuthenticatedPrincipal> {
        let lifecycle = self
            .connection_lifecycles
            .get(conn)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let lifecycle = lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active || lifecycle.retired || lifecycle.generation != lifecycle_generation {
            return Err(RvoipError::ConnectionNotFound(conn.clone()));
        }
        let entry = self
            .connections
            .get(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if entry.transport != transport {
            return Err(RvoipError::AdmissionRejected(
                "inbound admission transport mismatch",
            ));
        }
        if entry.inbound_publication != InboundPublicationState::Pending(lifecycle_generation) {
            return Err(RvoipError::AdmissionRejected(
                "inbound admission is no longer pending",
            ));
        }
        entry.principal.clone().ok_or(RvoipError::InvalidState(
            "connection has no authenticated principal",
        ))
    }

    pub(crate) fn take_inbound_admission_context(
        &self,
        conn: &ConnectionId,
        transport: Transport,
        lifecycle_generation: u64,
    ) -> Result<Option<InboundConnectionContext>> {
        let lifecycle = self
            .connection_lifecycles
            .get(conn)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let lifecycle = lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active || lifecycle.retired || lifecycle.generation != lifecycle_generation {
            return Err(RvoipError::ConnectionNotFound(conn.clone()));
        }
        let mut entry = self
            .connections
            .get_mut(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if entry.transport != transport {
            entry.inbound_context = None;
            entry.inbound_context_retired = true;
            return Err(RvoipError::AdmissionRejected(
                "inbound admission transport mismatch",
            ));
        }
        if entry.inbound_publication != InboundPublicationState::Pending(lifecycle_generation) {
            return Err(RvoipError::AdmissionRejected(
                "inbound admission is no longer pending",
            ));
        }
        let principal = entry.principal.as_ref().ok_or(RvoipError::InvalidState(
            "connection has no authenticated principal",
        ))?;
        if principal.is_expired()
            || principal.tenant.as_deref().is_none_or(str::is_empty)
            || entry
                .inbound_context
                .as_ref()
                .is_some_and(|context| !context.is_bound_to(conn, transport, principal))
        {
            entry.inbound_context = None;
            entry.inbound_context_retired = true;
            return Err(RvoipError::AdmissionRejected(
                "inbound admission context binding mismatch",
            ));
        }
        let context = entry.inbound_context.take();
        entry.inbound_context_retired = true;
        Ok(context)
    }

    fn track_connection(
        &self,
        conn: &ConnectionId,
        transport: Transport,
        inbound_context: Option<InboundConnectionContext>,
    ) -> bool {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.ensure_connection_lifecycle(conn) {
            return false;
        }
        if let Some(mut entry) = self.connections.get_mut(conn) {
            if entry.transport != transport {
                return false;
            }
            if !entry.inbound_context_retired && entry.inbound_context.is_none() {
                entry.inbound_context = match (inbound_context, entry.principal.as_ref()) {
                    (Some(context), Some(principal))
                        if !context.is_bound_to(conn, transport, principal) =>
                    {
                        entry.inbound_context_retired = true;
                        None
                    }
                    (candidate, _) => candidate,
                };
            }
        } else {
            self.connections.insert(
                conn.clone(),
                ConnectionEntry {
                    transport,
                    principal: None,
                    inbound_context,
                    inbound_context_retired: false,
                    inbound_publication: InboundPublicationState::NotInbound,
                    deferred_authentication: None,
                    deferred_principal_authentication: None,
                },
            );
        }
        true
    }

    fn adapter_connection_is_live(&self, transport: Transport, conn: &ConnectionId) -> bool {
        self.adapters
            .get(&transport)
            .is_some_and(|adapter| adapter.is_connection_live(conn))
    }

    fn connection_owned_by_other_transport(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
    ) -> bool {
        self.connections
            .get(connection_id)
            .is_some_and(|entry| entry.transport != transport)
    }

    async fn reject_colliding_adapter_route(
        &self,
        transport: Transport,
        connection_id: ConnectionId,
    ) {
        let Ok(adapter) = self.adapter(transport) else {
            return;
        };
        // A colliding adapter may already have retained an attachment token;
        // drain it without exposing or storing it in the owning core route.
        let _ = adapter.take_inbound_context(&connection_id);
        metrics::counter!(
            "rvoip_core_connection_transport_collision_total",
            "transport" => format!("{transport:?}")
        )
        .increment(1);
        let _ = tokio::time::timeout(INBOUND_ADMISSION_ADAPTER_CLEANUP_TIMEOUT, async {
            if adapter
                .reject(connection_id.clone(), RejectReason::ServerError)
                .await
                .is_err()
            {
                let _ = adapter
                    .end(
                        connection_id,
                        EndReason::Failed {
                            detail: "connection ID transport collision".into(),
                        },
                    )
                    .await;
            }
        })
        .await;
    }

    fn track_connection_principal(
        &self,
        conn: &ConnectionId,
        transport: Transport,
        principal: AuthenticatedPrincipal,
    ) -> bool {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.ensure_connection_lifecycle(conn) {
            return false;
        }
        if let Some(mut entry) = self.connections.get_mut(conn) {
            if entry.transport != transport {
                return false;
            }
            if entry
                .inbound_context
                .as_ref()
                .is_some_and(|context| !context.is_bound_to(conn, transport, &principal))
            {
                entry.inbound_context = None;
                entry.inbound_context_retired = true;
            }
            entry.principal = Some(principal);
        } else {
            self.connections.insert(
                conn.clone(),
                ConnectionEntry {
                    transport,
                    principal: Some(principal),
                    inbound_context: None,
                    inbound_context_retired: false,
                    inbound_publication: InboundPublicationState::NotInbound,
                    deferred_authentication: None,
                    deferred_principal_authentication: None,
                },
            );
        }
        true
    }

    fn mark_connection_inbound(&self, conn: &ConnectionId) -> Result<()> {
        let mut entry = self
            .connections
            .get_mut(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if entry.inbound_publication == InboundPublicationState::NotInbound {
            entry.inbound_publication = InboundPublicationState::Unseen;
        }
        Ok(())
    }

    fn ensure_connection_lifecycle(&self, connection_id: &ConnectionId) -> bool {
        if let Some(state) = self.connection_lifecycles.get(connection_id) {
            let state = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            return state.active && !state.retired;
        }
        let retained = self.connection_lifecycles.len();
        if retained >= self.connection_id_budget.load(Ordering::Relaxed) {
            metrics::counter!("rvoip_core_connection_id_budget_exhausted_total").increment(1);
            return false;
        }
        let state = self
            .connection_lifecycles
            .entry(connection_id.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(ConnectionLifecycleState {
                    generation: 1,
                    active: true,
                    retired: false,
                }))
            })
            .clone();
        let state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active && !state.retired
    }

    fn retire_untracked_connection_id(&self, connection_id: &ConnectionId) {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.connection_lifecycles.contains_key(connection_id) {
            return;
        }
        if self.connection_lifecycles.len() >= self.connection_id_budget.load(Ordering::Relaxed) {
            metrics::counter!("rvoip_core_connection_id_budget_exhausted_total").increment(1);
            return;
        }
        self.connection_lifecycles.insert(
            connection_id.clone(),
            Arc::new(Mutex::new(ConnectionLifecycleState {
                generation: 1,
                active: false,
                retired: true,
            })),
        );
    }

    fn capture_connection_lifecycles(
        &self,
        connection_ids: &[ConnectionId],
    ) -> Result<Vec<ConnectionLifecycleTicket>> {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut connection_ids = connection_ids.to_vec();
        connection_ids.sort();
        connection_ids.dedup();
        let mut tickets = Vec::with_capacity(connection_ids.len());
        for connection_id in connection_ids {
            if !self.connections.contains_key(&connection_id) {
                return Err(RvoipError::ConnectionNotFound(connection_id));
            }
            let state = self
                .connection_lifecycles
                .get(&connection_id)
                .map(|entry| Arc::clone(entry.value()))
                .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))?;
            let generation = {
                let state_guard = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !state_guard.active || state_guard.retired {
                    return Err(RvoipError::ConnectionNotFound(connection_id));
                }
                state_guard.generation
            };
            tickets.push(ConnectionLifecycleTicket {
                connection_id,
                generation,
                state,
            });
        }
        Ok(tickets)
    }

    fn lock_connection_lifecycles<'a>(
        &self,
        tickets: &'a [ConnectionLifecycleTicket],
    ) -> Result<Vec<std::sync::MutexGuard<'a, ConnectionLifecycleState>>> {
        let guards = tickets
            .iter()
            .map(|ticket| {
                ticket
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        for (ticket, state) in tickets.iter().zip(&guards) {
            if !state.active
                || state.retired
                || state.generation != ticket.generation
                || !self.connections.contains_key(&ticket.connection_id)
            {
                return Err(RvoipError::ConnectionNotFound(ticket.connection_id.clone()));
            }
        }
        Ok(guards)
    }

    fn validate_connection_lifecycles(&self, tickets: &[ConnectionLifecycleTicket]) -> Result<()> {
        drop(self.lock_connection_lifecycles(tickets)?);
        Ok(())
    }

    fn bind_connection_to_session(
        &self,
        connection_id: &ConnectionId,
        session_id: &SessionId,
        participant_id: ParticipantId,
    ) -> Result<()> {
        let sess_arc = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;
        {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            if matches!(sess.state, SessionState::Ended | SessionState::Failed) {
                return Err(RvoipError::InvalidState(
                    "bind_connection_to_session: target session is ended",
                ));
            }
            sess.connections.insert(
                connection_id.clone(),
                ConnectionRef {
                    id: connection_id.clone(),
                    participant_id,
                },
            );
            if sess.state == SessionState::Initiating {
                sess.state = SessionState::Active;
            }
        }
        self.sessions_by_connection
            .insert(connection_id.clone(), session_id.clone());
        Ok(())
    }

    fn bind_connection_to_session_probe(&self, session_id: &SessionId) -> Result<()> {
        let sess_arc = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;
        let sess = sess_arc.read().expect("session lock poisoned");
        if matches!(sess.state, SessionState::Ended | SessionState::Failed) {
            return Err(RvoipError::InvalidState(
                "originate_connection: target session is ended",
            ));
        }
        Ok(())
    }

    /// If `conn` is currently in a cross-transport bridge, return the
    /// peer `ConnectionId` on the other leg. Gap plan §4.3 / v1 punch
    /// list — used by the DTMF auto-route in the `AdapterEvent::Dtmf`
    /// handler to forward digits across the bridge when one side
    /// signals DTMF out-of-band (e.g. UCTP `dtmf.send` envelope) and
    /// the bridged peer needs to inject the corresponding RFC 4733
    /// telephone-event packets onto its outbound RTP.
    fn bridge_peer_of(&self, conn: &ConnectionId) -> Option<ConnectionId> {
        let bridge_id = self
            .cross_bridge_owners
            .get(conn)
            .map(|owner| owner.value().clone())?;
        self.cross_bridges.get(&bridge_id).and_then(|entry| {
            let bridge = entry.value();
            if &bridge.a == conn {
                Some(bridge.b.clone())
            } else if &bridge.b == conn {
                Some(bridge.a.clone())
            } else {
                None
            }
        })
    }

    fn reserve_cross_bridge(
        &self,
        bridge_id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
    ) -> Result<BridgeReservation> {
        let _guard = self
            .bridge_ownership_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cross_bridge_owners.contains_key(&a) || self.cross_bridge_owners.contains_key(&b) {
            return Err(RvoipError::AdmissionRejected("connection already bridged"));
        }
        self.cross_bridge_owners
            .insert(a.clone(), bridge_id.clone());
        self.cross_bridge_owners
            .insert(b.clone(), bridge_id.clone());
        Ok(BridgeReservation {
            bridge_id,
            a,
            b,
            owners: Arc::clone(&self.cross_bridge_owners),
            lock: Arc::clone(&self.bridge_ownership_lock),
            committed: false,
        })
    }

    fn release_cross_bridge_ownership(
        &self,
        bridge_id: &BridgeId,
        a: &ConnectionId,
        b: &ConnectionId,
    ) {
        let _guard = self
            .bridge_ownership_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for connection_id in [a, b] {
            let owned_by_bridge = self
                .cross_bridge_owners
                .get(connection_id)
                .is_some_and(|owner| owner.value() == bridge_id);
            if owned_by_bridge {
                self.cross_bridge_owners.remove(connection_id);
            }
        }
    }

    async fn remove_cross_bridge_internal(&self, bridge_id: &BridgeId) -> Result<bool> {
        let Some((_, mut handle)) = self.cross_bridges.remove(bridge_id) else {
            return Ok(false);
        };
        let a = handle.a.clone();
        let b = handle.b.clone();
        let result = handle.stop().await;
        self.release_cross_bridge_ownership(bridge_id, &a, &b);
        result.map(|_| true)
    }

    fn supervise_cross_bridge_routes(
        &self,
        bridge_id: BridgeId,
        statuses: (MediaGraphRouteStatus, MediaGraphRouteStatus),
    ) {
        for status in [statuses.0, statuses.1] {
            let cross_bridges = Arc::clone(&self.cross_bridges);
            let owners = Arc::clone(&self.cross_bridge_owners);
            let ownership_lock = Arc::clone(&self.bridge_ownership_lock);
            let events = self.events.clone();
            let cross_crate_publisher = self.cross_crate_publisher.clone();
            let bridge_id = bridge_id.clone();
            tokio::spawn(async move {
                let _ = status.wait_terminal().await;
                let Some((_, mut handle)) = cross_bridges.remove(&bridge_id) else {
                    return;
                };
                let a = handle.a.clone();
                let b = handle.b.clone();
                let result = handle.stop().await;
                {
                    let _guard = ownership_lock
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    for connection_id in [&a, &b] {
                        let owned = owners
                            .get(connection_id)
                            .is_some_and(|owner| owner.value() == &bridge_id);
                        if owned {
                            owners.remove(connection_id);
                        }
                    }
                }
                match result {
                    Ok(()) => {
                        let event = Event::ConnectionsUnbridged {
                            bridge_id,
                            at: Utc::now(),
                        };
                        let _ = Self::emit_to_channels(
                            &events,
                            cross_crate_publisher.as_deref(),
                            event,
                        );
                    }
                    Err(error) => warn!(
                        %bridge_id,
                        %error,
                        "failed to converge bridge after media route terminated"
                    ),
                }
            });
        }
    }

    /// Remove observer attachments that name an abruptly-ended Connection.
    ///
    /// Session-scoped attachments may own routes for several Connections. If
    /// any source ends, the whole attachment is stopped: silently continuing a
    /// partial recording/transcription would produce an artifact whose shape
    /// no longer matches the caller's request.
    fn spawn_route_removals(routes: Vec<ManagedMediaRoute>) {
        if routes.is_empty() {
            return;
        }
        tokio::spawn(async move {
            for route in routes {
                let _ = route.remove().await;
            }
        });
    }

    fn remove_recording_owner(&self, recording_id: &crate::ids::RecordingId) -> bool {
        let Some((_, mut handle)) = self.recordings.remove(recording_id) else {
            return false;
        };
        let routes = handle.media.begin_stop();
        let sink = Arc::clone(&handle.sink);
        drop(handle);
        self.emit(Event::RecordingStopped {
            recording_id: recording_id.clone(),
            at: Utc::now(),
        });
        let recording_id = recording_id.clone();
        let events = self.events.clone();
        let cross_crate_publisher = self.cross_crate_publisher.clone();
        tokio::spawn(async move {
            for route in routes {
                let _ = route.remove().await;
            }
            let Ok(artifact) = sink.close().await else {
                return;
            };
            let event = Event::RecordingComplete {
                recording_id,
                sink: artifact.url,
                vcon_ref: None,
                at: Utc::now(),
            };
            let _ = Self::emit_to_channels(&events, cross_crate_publisher.as_deref(), event);
        });
        true
    }

    fn remove_transcription_owner(&self, id: &crate::ids::TranscriptionId) -> bool {
        let Some((_, mut handle)) = self.transcriptions.remove(id) else {
            return false;
        };
        let routes = handle.media.begin_stop();
        drop(handle);
        Self::spawn_route_removals(routes);
        true
    }

    fn remove_ai_owner(&self, id: &crate::ids::AiAttachmentId) -> bool {
        let Some((_, mut handle)) = self.ai_attachments.remove(id) else {
            return false;
        };
        let routes = handle.media.begin_stop();
        drop(handle);
        Self::spawn_route_removals(routes);
        self.emit(Event::AiDetached {
            attachment_id: id.clone(),
            at: Utc::now(),
        });
        true
    }

    fn remove_listener_owner(&self, id: &crate::ids::ListenerId) -> bool {
        let removed = self.listener_tasks.remove(id).is_some();
        self.listener_channels.remove(id);
        if removed {
            self.emit(Event::ListenerDetached {
                listener_id: id.clone(),
                at: Utc::now(),
            });
        }
        removed
    }

    fn supervise_recording_routes(
        self: &Arc<Self>,
        recording_id: crate::ids::RecordingId,
        statuses: Vec<MediaGraphRouteStatus>,
    ) {
        for status in statuses {
            let weak = Arc::downgrade(self);
            let recording_id = recording_id.clone();
            tokio::spawn(async move {
                let _ = status.wait_terminal().await;
                if let Some(orchestrator) = weak.upgrade() {
                    orchestrator.remove_recording_owner(&recording_id);
                }
            });
        }
    }

    fn supervise_transcription_routes(
        self: &Arc<Self>,
        transcription_id: crate::ids::TranscriptionId,
        statuses: Vec<MediaGraphRouteStatus>,
    ) {
        for status in statuses {
            let weak = Arc::downgrade(self);
            let transcription_id = transcription_id.clone();
            tokio::spawn(async move {
                let _ = status.wait_terminal().await;
                if let Some(orchestrator) = weak.upgrade() {
                    orchestrator.remove_transcription_owner(&transcription_id);
                }
            });
        }
    }

    fn supervise_ai_routes(
        self: &Arc<Self>,
        attachment_id: crate::ids::AiAttachmentId,
        statuses: Vec<MediaGraphRouteStatus>,
    ) {
        for status in statuses {
            let weak = Arc::downgrade(self);
            let attachment_id = attachment_id.clone();
            tokio::spawn(async move {
                let _ = status.wait_terminal().await;
                if let Some(orchestrator) = weak.upgrade() {
                    orchestrator.remove_ai_owner(&attachment_id);
                }
            });
        }
    }

    fn supervise_listener_route(
        self: &Arc<Self>,
        listener_id: crate::ids::ListenerId,
        status: MediaGraphRouteStatus,
    ) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let _ = status.wait_terminal().await;
            if let Some(orchestrator) = weak.upgrade() {
                orchestrator.remove_listener_owner(&listener_id);
            }
        });
    }

    fn cleanup_media_attachments_for_connection(&self, conn: &ConnectionId) {
        let recording_ids: Vec<_> = self
            .recordings
            .iter()
            .filter(|entry| entry.value().connection_ids.contains(conn))
            .map(|entry| entry.key().clone())
            .collect();
        for recording_id in recording_ids {
            self.remove_recording_owner(&recording_id);
        }

        let transcription_ids: Vec<_> = self
            .transcriptions
            .iter()
            .filter(|entry| &entry.value().connection_id == conn)
            .map(|entry| entry.key().clone())
            .collect();
        for transcription_id in transcription_ids {
            self.remove_transcription_owner(&transcription_id);
        }

        let ai_ids: Vec<_> = self
            .ai_attachments
            .iter()
            .filter(|entry| &entry.value().connection_id == conn)
            .map(|entry| entry.key().clone())
            .collect();
        for attachment_id in ai_ids {
            self.remove_ai_owner(&attachment_id);
        }

        let listener_ids: Vec<_> = self
            .listener_tasks
            .iter()
            .filter(|entry| entry.value().connection_ids.contains(conn))
            .map(|entry| entry.key().clone())
            .collect();
        for listener_id in listener_ids {
            // MediaTaskHandle::drop aborts the parent; its JoinSet aborts each
            // child, and each child's MediaTapRoute removes its graph sink.
            self.remove_listener_owner(&listener_id);
        }
    }

    fn begin_connection_teardown(&self, conn: &ConnectionId) -> ForgottenConnection {
        let removed = {
            let _registry = self
                .connection_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Retain a process-lifetime tombstone. Adapter lifecycle events do
            // not carry a route epoch, so safely distinguishing a late event
            // from a reused external ConnectionId is otherwise impossible.
            if let Some(state) = self.connection_lifecycles.get(conn) {
                let mut state = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.active = false;
                state.retired = true;
                state.generation = state.generation.saturating_add(1);
            }
            self.connections.remove(conn)
        };
        let was_tracked = removed.is_some();
        let normalized_lifecycle_was_visible = removed.is_some_and(|(_, entry)| {
            !matches!(
                entry.inbound_publication,
                InboundPublicationState::Unseen
                    | InboundPublicationState::Pending(_)
                    | InboundPublicationState::Rejecting(_)
            )
        });
        self.drop_connection_subscriptions(conn);
        ForgottenConnection {
            was_tracked,
            normalized_lifecycle_was_visible,
        }
    }

    fn begin_claimed_inbound_teardown(
        &self,
        claimed: &ClaimedInboundRejection,
    ) -> Option<ForgottenConnection> {
        let removed = {
            let _registry = self
                .connection_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let current = self.connection_lifecycles.get(&claimed.connection_id)?;
            if !Arc::ptr_eq(current.value(), &claimed.lifecycle.state) {
                return None;
            }
            let mut lifecycle = claimed
                .lifecycle
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !lifecycle.active
                || lifecycle.retired
                || lifecycle.generation != claimed.lifecycle.generation
            {
                return None;
            }
            let removed = self
                .connections
                .remove_if(&claimed.connection_id, |_, entry| {
                    entry.inbound_publication
                        == InboundPublicationState::Rejecting(claimed.lifecycle.generation)
                })?;
            lifecycle.active = false;
            lifecycle.retired = true;
            lifecycle.generation = lifecycle.generation.saturating_add(1);
            removed
        };
        debug_assert!(matches!(
            removed.1.inbound_publication,
            InboundPublicationState::Rejecting(_)
        ));
        self.drop_connection_subscriptions(&claimed.connection_id);
        Some(ForgottenConnection {
            was_tracked: true,
            normalized_lifecycle_was_visible: claimed.normalized_lifecycle_was_visible,
        })
    }

    async fn forget_connection(&self, conn: &ConnectionId) -> ForgottenConnection {
        let forgotten = self.begin_connection_teardown(conn);
        self.finish_connection_teardown(conn, forgotten).await
    }

    async fn finish_connection_teardown(
        &self,
        conn: &ConnectionId,
        forgotten: ForgottenConnection,
    ) -> ForgottenConnection {
        // Tear down bridge routes before shutting down the source graph.
        if let Some(bridge_id) = self
            .cross_bridge_owners
            .get(conn)
            .map(|owner| owner.value().clone())
        {
            match self.remove_cross_bridge_internal(&bridge_id).await {
                Ok(true) => self.emit(Event::ConnectionsUnbridged {
                    bridge_id,
                    at: Utc::now(),
                }),
                Ok(false) => {}
                Err(error) => warn!(%error, "failed to converge bridge during disconnect"),
            }
        }
        self.cleanup_media_attachments_for_connection(conn);
        if let Some((_, graph)) = self.media_graphs.remove(conn) {
            let _ = graph.shutdown_and_wait().await;
        }
        self.media_graph_inits.remove(conn);
        // P1.10 — if this Connection was bound to a Session, detach it
        // and auto-end the Session when it loses its last Connection.
        // Must run before subscription cleanup so the Session lookup
        // sees a stable connection set.
        self.detach_connection_from_session(conn);
        forgotten
    }

    // --- Conversation / Session / Participant lifecycle (P1) -----------
    //
    // Implements the 7 lifecycle Commands (`OpenConversation`,
    // `CloseConversation`, `StartSession`, `EndSession`, `JoinSession`,
    // `LeaveSession`, `RouteInboundConnection::Accept`) per
    // INTERFACE_DESIGN.md §3 + PRD §10. Each method is `async` to match
    // the trait-friendly shape the GAP_PLAN promised, even though the
    // work today is purely synchronous lock acquisition + event emit.

    /// Open a new Conversation. Emits `Event::ConversationOpened`.
    /// Returns the freshly-allocated `ConversationId`.
    #[instrument(skip(self, metadata), fields(tenant = %tenant_id, conversation_id))]
    pub async fn open_conversation(
        &self,
        tenant_id: TenantId,
        policy: ConversationPolicy,
        metadata: HashMap<String, String>,
    ) -> Result<ConversationId> {
        let id = ConversationId::new();
        let now = Utc::now();
        let conv = Conversation {
            id: id.clone(),
            tenant_id,
            state: ConversationState::Open,
            policy,
            participants: Vec::new(),
            sessions: Vec::new(),
            messages: Vec::new(),
            opened_at: now,
            closed_at: None,
            last_activity_at: now,
            metadata,
        };
        self.conversations
            .insert(id.clone(), Arc::new(RwLock::new(conv)));
        // P6 — index by tenant for `list_for_tenant` and isolation
        // enforcement.
        self.conversations_by_tenant
            .entry(tenant_id_for_index(&self.conversations, &id))
            .or_default()
            .insert(id.clone());
        self.emit(Event::ConversationOpened {
            conversation_id: id.clone(),
            at: now,
        });
        Ok(id)
    }

    /// P6 — install/replace per-tenant quotas. V2.B provisions the
    /// per-tenant admission semaphores from the quota config: each
    /// `max_concurrent_*` slot gets an `Arc<Semaphore>` with that
    /// capacity. Resize-up is supported (extra permits added via
    /// `Semaphore::add_permits`); resize-down with live permits would
    /// require revoking issued permits and is intentionally rejected
    /// — call sites that want to shrink a quota should drain the
    /// active sessions first.
    pub fn set_tenant_quotas(
        &self,
        tenant: TenantId,
        quotas: crate::config::TenantQuotas,
    ) -> Result<()> {
        // Provision / resize recording semaphore.
        if let Some(new_cap) = quotas.max_concurrent_recordings {
            match self.recording_sems.entry(tenant.clone()) {
                dashmap::mapref::entry::Entry::Vacant(v) => {
                    v.insert(Arc::new(Semaphore::new(new_cap)));
                }
                dashmap::mapref::entry::Entry::Occupied(o) => {
                    // Compare against an implicit "total issued" — we
                    // can't directly read total capacity from a tokio
                    // Semaphore, so we track resize-up by checking if
                    // new_cap exceeds current available + outstanding.
                    // Outstanding = total - available. We approximate
                    // by using the Semaphore's add_permits which always
                    // adds (no resize-down possible).
                    let sem = o.get();
                    let available = sem.available_permits();
                    // For resize-up: add (new - available) permits when
                    // new > available. This is conservative — if the
                    // existing cap was already higher than `available`,
                    // we may end up adding too few permits (loss of
                    // capacity that's currently held). Documented as
                    // a v2.B.1 caveat — call sites that mix shrink and
                    // expand on the same tenant need explicit drain
                    // semantics.
                    if new_cap > available {
                        sem.add_permits(new_cap - available);
                    } else if new_cap < available {
                        return Err(RvoipError::InvalidState(
                            "set_tenant_quotas: shrinking recording quota \
                             not supported while permits are held; drain first",
                        ));
                    }
                }
            }
        }
        if let Some(new_cap) = quotas.max_concurrent_ai_sessions {
            match self.ai_sems.entry(tenant.clone()) {
                dashmap::mapref::entry::Entry::Vacant(v) => {
                    v.insert(Arc::new(Semaphore::new(new_cap)));
                }
                dashmap::mapref::entry::Entry::Occupied(o) => {
                    let sem = o.get();
                    let available = sem.available_permits();
                    if new_cap > available {
                        sem.add_permits(new_cap - available);
                    } else if new_cap < available {
                        return Err(RvoipError::InvalidState(
                            "set_tenant_quotas: shrinking AI quota not \
                             supported while permits are held; drain first",
                        ));
                    }
                }
            }
        }
        self.tenant_quotas.insert(tenant, quotas);
        Ok(())
    }

    /// P6 — best-effort snapshot for the periodic capacity scheduler
    /// and on-demand inspection. P9 — also updates the global
    /// Prometheus gauges so a scraper sees current state without
    /// having to subscribe to the event bus.
    pub fn capacity_report(&self) -> Event {
        let active_connections = self.connections.len() as u64;
        let active_bridges = self.cross_bridges.len() as u64;
        let admission_in_use =
            (self.config.max_concurrent_setups - self.admission.available_permits()) as u64;
        let active_sessions = self.sessions.len() as u64;
        let active_conversations = self.conversations.len() as u64;
        let active_recordings = self.recordings.len() as u64;
        let active_ai = self.ai_attachments.len() as u64;

        metrics::gauge!("rvoip_active_connections").set(active_connections as f64);
        metrics::gauge!("rvoip_active_bridges").set(active_bridges as f64);
        metrics::gauge!("rvoip_admission_in_use").set(admission_in_use as f64);
        metrics::gauge!("rvoip_active_sessions").set(active_sessions as f64);
        metrics::gauge!("rvoip_active_conversations").set(active_conversations as f64);
        metrics::gauge!("rvoip_active_recordings").set(active_recordings as f64);
        metrics::gauge!("rvoip_active_ai_attachments").set(active_ai as f64);

        Event::CapacityReport {
            tenant_id: None,
            active_connections,
            active_bridges,
            admission_in_use,
            at: Utc::now(),
        }
    }

    /// P9 — sample current `QualitySnapshot` for every active
    /// Connection at the configured cadence and emit
    /// `Event::MediaQuality`. Spawns one task that ticks `every`.
    pub fn spawn_media_quality_sampler(self: &Arc<Self>, every: std::time::Duration) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.tick().await;
            loop {
                tick.tick().await;
                // Snapshot connections.
                let conns: Vec<(ConnectionId, Transport)> = me
                    .connections
                    .iter()
                    .map(|e| (e.key().clone(), e.value().transport))
                    .collect();
                for (cid, transport) in conns {
                    let Ok(adapter) = me.adapter(transport) else {
                        continue;
                    };
                    let Ok(streams) = adapter.streams(cid.clone()).await else {
                        continue;
                    };
                    let mut totaled = crate::stream::QualitySnapshot {
                        jitter_ms: 0.0,
                        packet_loss_pct: 0.0,
                        mos: None,
                    };
                    let mut n = 0usize;
                    for s in streams {
                        let snap = s.quality_snapshot();
                        totaled.jitter_ms += snap.jitter_ms;
                        totaled.packet_loss_pct += snap.packet_loss_pct;
                        if let Some(m) = snap.mos {
                            totaled.mos = Some(totaled.mos.map_or(m, |a| a + m));
                        }
                        n += 1;
                    }
                    if n == 0 {
                        continue;
                    }
                    totaled.jitter_ms /= n as f32;
                    totaled.packet_loss_pct /= n as f32;
                    totaled.mos = totaled.mos.map(|m| m / n as f32);
                    me.emit(Event::MediaQuality {
                        connection_id: cid,
                        snapshot: totaled,
                        at: Utc::now(),
                    });
                }
            }
        });
    }

    /// P10 — drive idle-close of `Ephemeral` Conversations. Spawns
    /// one task that ticks `every` and force-closes any Conversation
    /// whose `last_activity_at` is older than its policy's
    /// `idle_close_secs` AND has no `Active` Sessions.
    pub fn spawn_idle_closer(self: &Arc<Self>, every: std::time::Duration) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.tick().await;
            loop {
                tick.tick().await;
                let now = Utc::now();
                let mut to_close: Vec<ConversationId> = Vec::new();
                for entry in me.conversations.iter() {
                    let c = entry.value().read().expect("conv lock poisoned");
                    let ConversationPolicy::Ephemeral { idle_close_secs } = c.policy else {
                        continue;
                    };
                    if c.state != ConversationState::Open {
                        continue;
                    }
                    let idle = (now - c.last_activity_at).num_seconds().max(0) as u64;
                    if idle < idle_close_secs {
                        continue;
                    }
                    // Skip if any Session is Active.
                    let any_active = c.sessions.iter().any(|sid| {
                        me.sessions
                            .get(sid)
                            .map(|s| {
                                s.value().read().expect("sess lock poisoned").state
                                    == SessionState::Active
                            })
                            .unwrap_or(false)
                    });
                    if any_active {
                        continue;
                    }
                    to_close.push(entry.key().clone());
                }
                for cid in to_close {
                    let _ = me.close_conversation(cid, false).await;
                }
            }
        });
    }

    /// P6 — start the periodic capacity-report emitter using the
    /// cadence in `Config::capacity_report_interval`. Returns
    /// immediately; the scheduler task is owned by the Orchestrator
    /// and aborts when the Orchestrator is dropped (best-effort —
    /// real teardown semantics ship with P11 graceful-shutdown).
    pub fn spawn_capacity_scheduler(self: &Arc<Self>) {
        let Some(interval) = self.config.capacity_report_interval else {
            return;
        };
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            // Skip the immediate tick — first emit happens after one
            // interval.
            tick.tick().await;
            loop {
                tick.tick().await;
                me.emit(me.capacity_report());
            }
        });
    }

    fn check_session_quota(&self, conv_id: &ConversationId) -> Result<()> {
        let Some(tenant) = self.conversations.get(conv_id).map(|e| {
            e.value()
                .read()
                .expect("conv lock poisoned")
                .tenant_id
                .clone()
        }) else {
            return Ok(());
        };
        let Some(quotas) = self.tenant_quotas.get(&tenant).map(|e| *e.value()) else {
            return Ok(());
        };
        if let Some(max) = quotas.max_concurrent_sessions {
            // Count active sessions for this tenant.
            let mut active = 0usize;
            if let Some(convs) = self.conversations_by_tenant.get(&tenant) {
                for cid in convs.iter() {
                    if let Some(conv_arc) =
                        self.conversations.get(&*cid).map(|e| Arc::clone(e.value()))
                    {
                        for sid in &conv_arc.read().expect("conv lock poisoned").sessions {
                            if let Some(sess) = self.sessions.get(sid) {
                                if sess.value().read().expect("sess lock poisoned").state
                                    == SessionState::Active
                                {
                                    active += 1;
                                }
                            }
                        }
                    }
                }
            }
            if active >= max {
                return Err(RvoipError::AdmissionRejected(
                    "tenant max_concurrent_sessions exceeded",
                ));
            }
        }
        Ok(())
    }

    /// Close a Conversation. `force=false` rejects with `InvalidState`
    /// when any Session under the Conversation is still active;
    /// `force=true` first ends those Sessions (best-effort), then
    /// transitions the Conversation to Closed and emits
    /// `Event::ConversationClosed`. Closing an already-Closed
    /// Conversation is a no-op (idempotent).
    #[instrument(skip(self), fields(conversation_id = %id, force))]
    pub async fn close_conversation(&self, id: ConversationId, force: bool) -> Result<()> {
        let conv_arc = self
            .conversations
            .get(&id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::ConversationNotFound(id.clone()))?;

        let active_sessions: Vec<SessionId> = {
            let conv = conv_arc.read().expect("conversation lock poisoned");
            if conv.state == ConversationState::Closed {
                return Ok(());
            }
            conv.sessions
                .iter()
                .filter(|sid| {
                    self.sessions
                        .get(sid)
                        .map(|s| {
                            let st = s.value().read().expect("session lock poisoned").state;
                            !matches!(st, SessionState::Ended | SessionState::Failed)
                        })
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };

        if !active_sessions.is_empty() && !force {
            return Err(RvoipError::InvalidState(
                "close_conversation: active sessions exist; pass force=true to end them",
            ));
        }

        if force {
            for sid in active_sessions {
                let _ = self.end_session(sid, EndReason::Normal).await;
            }
        }

        let now = Utc::now();
        {
            let mut conv = conv_arc.write().expect("conversation lock poisoned");
            conv.state = ConversationState::Closed;
            conv.closed_at = Some(now);
            conv.last_activity_at = now;
        }
        self.emit(Event::ConversationClosed {
            conversation_id: id,
            at: now,
        });
        Ok(())
    }

    /// Start a new Session within an Open Conversation. Emits
    /// `Event::SessionStarted`. `invitees` populates the
    /// `Session::participants` set immediately; matching `Participant`
    /// entries are added to the Conversation when each invitee actually
    /// joins via `join_session` (so identity_ref / kind / role land
    /// from a real join, not from the invite).
    #[instrument(skip(self, invitees), fields(conversation_id = %conversation_id, medium = ?medium, session_id))]
    pub async fn start_session(
        &self,
        conversation_id: ConversationId,
        medium: SessionMedium,
        invitees: Vec<ParticipantId>,
    ) -> Result<SessionId> {
        let conv_arc = self
            .conversations
            .get(&conversation_id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::ConversationNotFound(conversation_id.clone()))?;

        {
            let conv = conv_arc.read().expect("conversation lock poisoned");
            if conv.state != ConversationState::Open {
                return Err(RvoipError::InvalidState(
                    "start_session: conversation is not Open",
                ));
            }
        }
        // P6 — quota check.
        self.check_session_quota(&conversation_id)?;

        let sid = SessionId::new();
        let now = Utc::now();
        let session = Session {
            id: sid.clone(),
            conversation_id: conversation_id.clone(),
            state: SessionState::Initiating,
            medium,
            participants: invitees.into_iter().collect(),
            connections: HashMap::new(),
            negotiated_capabilities: CapabilityIntersection::default(),
            started_at: now,
            ended_at: None,
            end_reason: None,
        };
        self.sessions
            .insert(sid.clone(), Arc::new(RwLock::new(session)));
        // P3 — every Session gets a vCon builder bound to it on start.
        self.session_vcons.insert(
            sid.clone(),
            Arc::new(crate::vcon::DefaultVconBuilder::new()),
        );

        {
            let mut conv = conv_arc.write().expect("conversation lock poisoned");
            conv.sessions.push(sid.clone());
            conv.last_activity_at = now;
        }

        self.emit(Event::SessionStarted {
            session_id: sid.clone(),
            conversation_id,
            at: now,
        });
        Ok(sid)
    }

    /// End a Session. Transitions state to `Ended`, drops multi-party
    /// subscriptions, clears the reverse Connection→Session index, and
    /// emits `Event::SessionEnded`. Idempotent: ending an already-
    /// Ended or Failed Session returns `Ok(())`.
    #[instrument(skip(self), fields(session_id = %session_id, reason = ?reason))]
    pub async fn end_session(&self, session_id: SessionId, reason: EndReason) -> Result<()> {
        let sess_arc = self
            .sessions
            .get(&session_id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;

        let now = Utc::now();
        let conv_id = {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            if matches!(sess.state, SessionState::Ended | SessionState::Failed) {
                return Ok(());
            }
            sess.state = SessionState::Ended;
            sess.ended_at = Some(now);
            sess.end_reason = Some(reason);
            sess.conversation_id.clone()
        };

        // Multi-party cleanup + reverse-index cleanup.
        self.drop_session_subscriptions(&session_id);
        self.sessions_by_connection
            .retain(|_, sid| sid != &session_id);

        // P3 — finalize the Session's vCon: snapshot, encode, persist,
        // emit VconReady. Best-effort — a store failure logs but does
        // not block SessionEnded emission.
        let tenant_id = self.conversations.get(&conv_id).map(|e| {
            e.value()
                .read()
                .expect("conv lock poisoned")
                .tenant_id
                .clone()
        });
        if let (Some((_, builder)), Some(tenant_id)) =
            (self.session_vcons.remove(&session_id), tenant_id)
        {
            let snap = builder.snapshot();
            let bytes = crate::vcon::encode_snapshot(&snap);
            let store = Arc::clone(&self.config.vcon_store);
            let sid_clone = session_id.clone();
            let events_tx = self.events.clone();
            let cross_crate_publisher = self.cross_crate_publisher.clone();
            tokio::spawn(async move {
                match store.put(&tenant_id, &sid_clone, bytes).await {
                    Ok(handle) => {
                        let ev = Event::VconReady {
                            session_id: sid_clone,
                            handle,
                            at: Utc::now(),
                        };
                        let _ = Self::emit_to_channels(
                            &events_tx,
                            cross_crate_publisher.as_deref(),
                            ev,
                        );
                    }
                    Err(e) => warn!(?e, "VconStore::put failed; VconReady not emitted"),
                }
            });
        }

        if let Some(conv_arc) = self
            .conversations
            .get(&conv_id)
            .map(|e| Arc::clone(e.value()))
        {
            conv_arc
                .write()
                .expect("conversation lock poisoned")
                .last_activity_at = now;
        }

        // P9 — snapshot the per-Session quality aggregator.
        let report = self
            .session_quality
            .remove(&session_id)
            .and_then(|(_, agg)| agg.finish());
        self.emit(Event::SessionEnded {
            report,
            session_id,
            at: now,
        });
        Ok(())
    }

    /// P3 — read access to a Session's vCon builder. Returns None if
    /// the Session is not active.
    pub fn session_vcon_handle(
        &self,
        session_id: &SessionId,
    ) -> Option<Arc<dyn crate::vcon::VconBuilderHandle>> {
        self.session_vcons
            .get(session_id)
            .map(|e| Arc::clone(e.value()) as Arc<dyn crate::vcon::VconBuilderHandle>)
    }

    /// Join a Participant to a Session. First join transitions the
    /// Session from `Initiating` to `Active`. Adds a matching
    /// `Participant` entry to the parent Conversation if one doesn't
    /// exist yet. Emits `Event::ParticipantJoined`. Rejects with
    /// `InvalidState` for Sessions in `Ending`, `Ended`, or `Failed`.
    pub async fn join_session(
        &self,
        session_id: SessionId,
        participant_id: ParticipantId,
        kind: ParticipantKind,
        role: ParticipantRole,
    ) -> Result<()> {
        let sess_arc = self
            .sessions
            .get(&session_id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;

        let now = Utc::now();
        let conv_id = {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            if matches!(
                sess.state,
                SessionState::Ending | SessionState::Ended | SessionState::Failed
            ) {
                return Err(RvoipError::InvalidState(
                    "join_session: session is ending or ended",
                ));
            }
            sess.participants.insert(participant_id.clone());
            if sess.state == SessionState::Initiating {
                sess.state = SessionState::Active;
            }
            sess.conversation_id.clone()
        };

        if let Some(conv_arc) = self
            .conversations
            .get(&conv_id)
            .map(|e| Arc::clone(e.value()))
        {
            let mut conv = conv_arc.write().expect("conversation lock poisoned");
            let exists = conv.participants.iter().any(|p| p.id == participant_id);
            if !exists {
                conv.participants.push(Participant {
                    id: participant_id.clone(),
                    conversation_id: conv_id.clone(),
                    identity_ref: None,
                    kind,
                    role,
                    display_name: None,
                    joined_at: now,
                    left_at: None,
                });
            }
            conv.last_activity_at = now;
        }

        // P3 — auto-collect the joining party into the Session's vCon.
        if let Some(builder) = self
            .session_vcons
            .get(&session_id)
            .map(|e| Arc::clone(e.value()))
        {
            builder.add_party(crate::vcon::VconParty {
                participant_id: participant_id.clone(),
                display_name: None,
                did_or_stir: None,
                validation: crate::identity::IdentityAssurance::Anonymous,
            });
        }

        self.emit(Event::ParticipantJoined {
            session_id,
            participant_id,
            at: now,
        });
        Ok(())
    }

    /// Remove a Participant from a Session. Sets `left_at` on the
    /// matching Conversation-level `Participant` entry if present.
    /// Emits `Event::ParticipantLeft`. Idempotent — leaving a Session
    /// the Participant isn't in is a no-op (still emits the event so
    /// downstream consumers see the intent).
    pub async fn leave_session(
        &self,
        session_id: SessionId,
        participant_id: ParticipantId,
    ) -> Result<()> {
        let sess_arc = self
            .sessions
            .get(&session_id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;

        let now = Utc::now();
        let conv_id = {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            sess.participants.remove(&participant_id);
            sess.conversation_id.clone()
        };

        if let Some(conv_arc) = self
            .conversations
            .get(&conv_id)
            .map(|e| Arc::clone(e.value()))
        {
            let mut conv = conv_arc.write().expect("conversation lock poisoned");
            if let Some(p) = conv
                .participants
                .iter_mut()
                .find(|p| p.id == participant_id)
            {
                p.left_at = Some(now);
            }
            conv.last_activity_at = now;
        }

        self.emit(Event::ParticipantLeft {
            session_id,
            participant_id,
            at: now,
        });
        Ok(())
    }

    /// P1.12 — reverse lookup `ConnectionId → SessionId`. Populated by
    /// `route_inbound_connection` on `InboundAction::Accept`; cleared
    /// by `forget_connection`.
    pub fn session_of(&self, connection_id: &ConnectionId) -> Option<SessionId> {
        self.sessions_by_connection
            .get(connection_id)
            .map(|e| e.value().clone())
    }

    /// Read-only handle to a live Conversation. Holds the inner Arc
    /// across the borrow; the caller manages the `RwLock`. Returns
    /// `None` if the Conversation was never opened or has already been
    /// purged.
    pub fn conversation(&self, id: &ConversationId) -> Option<Arc<RwLock<Conversation>>> {
        self.conversations.get(id).map(|e| Arc::clone(e.value()))
    }

    /// Read-only handle to a live Session. See [`Self::conversation`]
    /// for the locking contract.
    pub fn session(&self, id: &SessionId) -> Option<Arc<RwLock<Session>>> {
        self.sessions.get(id).map(|e| Arc::clone(e.value()))
    }

    /// P1.10 — Connection has gone away (adapter `Ended` / `Failed`).
    /// If it was bound to a Session, remove it from
    /// `Session.connections`. When the removal drops the last
    /// Connection from an `Active` Session, auto-transition to `Ended`
    /// and emit `SessionEnded`. Inline (no spawn) — the work is all
    /// synchronous lock acquisition + event emission.
    fn detach_connection_from_session(&self, conn: &ConnectionId) {
        let Some((_, sid)) = self.sessions_by_connection.remove(conn) else {
            return;
        };
        let Some(sess_arc) = self.sessions.get(&sid).map(|e| Arc::clone(e.value())) else {
            return;
        };
        let (auto_end, conv_id) = {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            sess.connections.remove(conn);
            let auto_end = sess.state == SessionState::Active && sess.connections.is_empty();
            (auto_end, sess.conversation_id.clone())
        };
        if !auto_end {
            return;
        }
        let now = Utc::now();
        {
            let mut sess = sess_arc.write().expect("session lock poisoned");
            sess.state = SessionState::Ended;
            sess.ended_at = Some(now);
            sess.end_reason = Some(EndReason::Normal);
        }
        self.drop_session_subscriptions(&sid);
        if let Some(conv_arc) = self
            .conversations
            .get(&conv_id)
            .map(|e| Arc::clone(e.value()))
        {
            conv_arc
                .write()
                .expect("conversation lock poisoned")
                .last_activity_at = now;
        }
        // P9 — snapshot the per-Session quality aggregator.
        let report = self
            .session_quality
            .remove(&sid)
            .and_then(|(_, agg)| agg.finish());
        self.emit(Event::SessionEnded {
            report,
            session_id: sid,
            at: now,
        });
    }

    // --- Multi-party subscription routing (v0.x MP1) -------------------
    //
    // Wire layer (`stream.subscribe` / `stream.unsubscribe` from the UCTP
    // coordinator) lands in MP2; media-path fanout that consults
    // `subscribers_for` lands in MP3. The methods below are the stable
    // surface those two PRs target.

    /// Add a subscription: `subscriber` will receive media datagrams
    /// from `publisher`'s `strm_id` Stream within `sid`. Idempotent.
    ///
    /// v0.x scope: stores the routing row only. The wire-side handler
    /// translating `stream.subscribe` envelopes into one or more
    /// `add_subscription` calls lands in MP2; the media-path fanout
    /// that drives this lookup lands in MP3.
    pub fn add_subscription(
        &self,
        sid: SessionId,
        subscriber: ConnectionId,
        publisher: ConnectionId,
        strm_id: StreamId,
    ) {
        let table = self.subscriptions.for_session(&sid);
        table.add(publisher, strm_id, subscriber);
    }

    /// Remove a single subscription. Idempotent — removing a
    /// subscription that doesn't exist is a no-op (returns `false`).
    pub fn remove_subscription(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        publisher: &ConnectionId,
        strm_id: &StreamId,
    ) -> bool {
        let table = self.subscriptions.for_session(sid);
        table.remove(publisher, strm_id, subscriber)
    }

    /// Snapshot the set of Connections subscribed to `(publisher,
    /// strm_id)` within `sid`. The media-path fanout (MP3) iterates
    /// the returned vec without holding any subscription-table lock.
    pub fn subscribers_for(
        &self,
        sid: &SessionId,
        publisher: &ConnectionId,
        strm_id: &StreamId,
    ) -> Vec<ConnectionId> {
        let table = self.subscriptions.for_session(sid);
        table.subscribers_for(publisher, strm_id)
    }

    /// Drop every subscription, publisher row, and subscriber-side media
    /// stream that names `conn`. This narrower synchronous cleanup hook is
    /// used by authenticated transport coordinators during abrupt teardown,
    /// before their asynchronous terminal adapter event is delivered.
    pub fn drop_connection_subscriptions(&self, conn: &ConnectionId) {
        self.subscriptions.drop_connection(conn);
        if let Some(registry) = self.publisher_registry.get() {
            registry.drop_publisher(conn);
        }
        self.subscriber_streams
            .retain(|(_, subscriber, publisher, _), _| subscriber != conn && publisher != conn);
    }

    /// Drop the entire subscription table for a Session. Called on
    /// `session.ended`. Idempotent.
    pub fn drop_session_subscriptions(&self, sid: &SessionId) {
        self.subscriptions.drop_session(sid);
        // Same mirror as `forget_connection`: clear publisher rows for
        // this Session so a `from_participant` subscribe issued after a
        // late peer joins on a recycled SessionId can't resolve to a
        // dead row from the previous tenant.
        if let Some(reg) = self.publisher_registry.get() {
            reg.drop_session(sid);
        }
        // MP3c: drop all per-subscription MediaStreams owned by this
        // Session.
        self.subscriber_streams.retain(|(s, _, _, _), _| s != sid);
    }

    /// Fan a publisher's `MediaFrame` out to every subscriber of
    /// `(sid, publisher, strm_id)`. v0.x MP3a primitive — adapter
    /// datagram-receive loops call this after unpacking a publisher's
    /// datagram (MP3b wires the publisher-side trigger).
    ///
    /// Per-subscriber stream resolution (plan §12 MP3c / G4):
    /// 1. Try the cached subscriber-side MediaStream for
    ///    `(sid, subscriber, publisher, strm_id)`. Reuses prior
    ///    allocation so each publisher's frames keep landing on the
    ///    same `stream_local_id`.
    /// 2. If absent, ask the subscriber's adapter to allocate a fresh
    ///    one via [`crate::adapter::ConnectionAdapter::allocate_subscriber_stream`].
    ///    The adapter picks the next free `stream_local_id`, registers
    ///    the MediaStream for inbound routing, and emits a
    ///    `stream.opened` envelope so the peer learns the new id.
    /// 3. If the adapter doesn't support allocation (returns
    ///    `NotImplemented` — e.g. SIP, WebRTC, or any adapter that
    ///    doesn't own the multi-party wire surface), fall back to the
    ///    legacy "first matching MediaStream by kind" path. Keeps
    ///    single-publisher rooms working unchanged.
    ///
    /// Returns the number of subscribers a frame was successfully
    /// delivered to. Best-effort: per-subscriber failures (channel
    /// full, adapter error) are logged at `debug` and do not block the
    /// remaining subscribers.
    ///
    /// Refinement still deferred: codec mismatch validation.
    /// `add_subscription` accepts any pair today; codec checking
    /// alongside `PublisherRegistry` codec metadata is plan B2.
    pub async fn fanout_frame(
        &self,
        sid: &SessionId,
        publisher: &ConnectionId,
        strm_id: &StreamId,
        frame: crate::stream::MediaFrame,
    ) -> usize {
        let subscribers = self.subscribers_for(sid, publisher, strm_id);
        let mut delivered = 0;
        for subscriber_connid in subscribers {
            let Ok(adapter) = self.adapter_for(&subscriber_connid) else {
                continue;
            };
            let key = (
                sid.clone(),
                subscriber_connid.clone(),
                publisher.clone(),
                strm_id.clone(),
            );
            // (1) Cached per-subscription stream — MP3c path.
            let target_opt: Option<Arc<dyn crate::stream::MediaStream>> = self
                .subscriber_streams
                .get(&key)
                .map(|entry| Arc::clone(entry.value()));
            let target = if let Some(s) = target_opt {
                Some(s)
            } else {
                // (2) Try to allocate a fresh per-subscription stream.
                // Adapters that don't carry multi-party responsibility
                // (SIP, WebRTC) return NotImplemented; we fall through
                // to (3) for them.
                let codec = self
                    .publisher_registry
                    .get()
                    .and_then(|reg| reg.entry(sid, &strm_id.to_string()))
                    .and_then(|entry| entry.codec.clone())
                    .unwrap_or_else(crate::capability::default_audio_codec);
                match adapter
                    .allocate_subscriber_stream(subscriber_connid.clone(), frame.kind, codec)
                    .await
                {
                    Ok(stream) => {
                        self.subscriber_streams
                            .insert(key.clone(), Arc::clone(&stream));
                        Some(stream)
                    }
                    Err(RvoipError::NotImplemented(_)) => {
                        // (3) Legacy fallback — pick first MediaStream
                        // by kind. Single-publisher rooms / non-UCTP
                        // substrates keep working unchanged.
                        adapter
                            .streams(subscriber_connid.clone())
                            .await
                            .ok()
                            .and_then(|streams| {
                                streams.into_iter().find(|s| s.kind() == frame.kind)
                            })
                    }
                    Err(e) => {
                        debug!(
                            error = %e,
                            ?subscriber_connid,
                            "fanout_frame: allocate_subscriber_stream failed"
                        );
                        None
                    }
                }
            };
            let Some(stream) = target else {
                continue;
            };
            let tx = stream.frames_out();
            match tx.try_send(frame.clone()) {
                Ok(()) => delivered += 1,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    metrics::counter!(
                        "rvoip_fanout_drops_total",
                        "reason" => "subscriber-queue-full"
                    )
                    .increment(1);
                    debug!(
                        ?subscriber_connid,
                        "fanout_frame: slow subscriber queue full"
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    metrics::counter!(
                        "rvoip_fanout_drops_total",
                        "reason" => "subscriber-closed"
                    )
                    .increment(1);
                    self.subscriber_streams.remove(&key);
                }
            }
        }
        delivered
    }

    /// Process-shared `PublisherRegistry` for the multi-party fanout
    /// path. Adapters build an `OrchestratorSubscriptionHandler` from
    /// this registry plus the orchestrator itself; the registry is
    /// the bridge from "publisher emitted `stream.opened`" (registered
    /// from the publishing coordinator) to "subscriber sent
    /// `stream.subscribe` with this strm_id" (resolved by the
    /// subscriber's coordinator's handler).
    pub fn publisher_registry(&self) -> Arc<crate::subscriptions::PublisherRegistry> {
        // Lazily ensure the registry exists. We don't pre-allocate it
        // in `new()` because Orchestrators that never run multi-party
        // routing shouldn't pay for the storage; but we want a single
        // shared instance once it's requested.
        Arc::clone(self.publisher_registry_inner())
    }

    fn publisher_registry_inner(&self) -> &Arc<crate::subscriptions::PublisherRegistry> {
        self.publisher_registry
            .get_or_init(|| Arc::new(crate::subscriptions::PublisherRegistry::new()))
    }

    fn emit_to_channels(
        events: &broadcast::Sender<Event>,
        cross_crate_publisher: Option<&CrossCrateEventPublisher>,
        event: Event,
    ) -> Option<CrossCrateEnqueueResult> {
        let cross_crate_result =
            cross_crate_publisher.map(|publisher| publisher.enqueue(event.to_cross_crate()));
        // Cross-crate backpressure must never suppress rich in-process events.
        let _ = events.send(event);
        cross_crate_result
    }

    /// Publish an event on the in-process broadcast channel and, if a
    /// coordinator is configured, enqueue it on the bounded cross-crate FIFO.
    fn emit(&self, event: Event) {
        let _ = Self::emit_to_channels(&self.events, self.cross_crate_publisher.as_deref(), event);
    }

    fn publish_inbound_if_current(&self, pending: &PendingInboundPublication) -> bool {
        if !self.adapter_connection_is_live(pending.transport, &pending.connection_id) {
            return false;
        }
        let Ok(_lifecycle) =
            self.lock_connection_lifecycles(std::slice::from_ref(&pending.lifecycle))
        else {
            return false;
        };
        let Some(mut entry) = self.connections.get_mut(&pending.connection_id) else {
            return false;
        };
        if entry.transport != pending.transport {
            return false;
        }
        let gated = self.inbound_admission_gate.get().is_some();
        let expected_state = if gated {
            InboundPublicationState::Pending(pending.lifecycle.generation)
        } else {
            InboundPublicationState::Unseen
        };
        if entry.inbound_publication != expected_state {
            return entry.inbound_publication == InboundPublicationState::Published;
        }
        if let Some(principal) = pending.principal.as_ref() {
            if entry
                .principal
                .as_ref()
                .is_none_or(|registered| !registered.has_same_owner(principal))
            {
                return false;
            }
        }
        if gated
            && entry
                .principal
                .as_ref()
                .is_some_and(|principal| principal.is_expired())
        {
            return false;
        }
        let deferred_principal = entry.deferred_principal_authentication.take();
        let deferred_authentication = entry.deferred_authentication.take();
        let retained_principal = entry.principal.clone();
        entry.inbound_publication = InboundPublicationState::Published;
        drop(entry);

        self.emit(Event::ConnectionInbound {
            connection_id: pending.connection_id.clone(),
            at: pending.observed_at,
        });
        if let Some(deferred) = deferred_principal {
            self.emit(Event::ConnectionAuthenticated {
                connection_id: pending.connection_id.clone(),
                identity_id: deferred.principal.subject.clone(),
                participant_id: deferred.participant_id.clone(),
                assurance: deferred.principal.assurance.clone(),
                at: deferred.at,
            });
            self.emit(Event::ConnectionPrincipalAuthenticated {
                connection_id: pending.connection_id.clone(),
                participant_id: deferred.participant_id,
                principal: deferred.principal,
                at: deferred.at,
            });
        } else if let (Some(participant_id), Some(principal)) =
            (pending.participant_id.as_ref(), retained_principal)
        {
            self.emit(Event::ConnectionAuthenticated {
                connection_id: pending.connection_id.clone(),
                identity_id: principal.subject.clone(),
                participant_id: participant_id.clone(),
                assurance: principal.assurance.clone(),
                at: pending.observed_at,
            });
            self.emit(Event::ConnectionPrincipalAuthenticated {
                connection_id: pending.connection_id.clone(),
                participant_id: participant_id.clone(),
                principal,
                at: pending.observed_at,
            });
        } else if let Some(deferred) = deferred_authentication {
            self.emit(Event::ConnectionAuthenticated {
                connection_id: pending.connection_id.clone(),
                identity_id: deferred.identity_id,
                participant_id: deferred.participant_id,
                assurance: deferred.assurance,
                at: deferred.at,
            });
        }
        true
    }

    async fn reject_claimed_unadmitted_connection(
        self: &Arc<Self>,
        claimed: ClaimedInboundRejection,
        reason: RejectReason,
        result: &'static str,
    ) -> bool {
        let transport = claimed.transport;
        let connection_id = claimed.connection_id.clone();
        let normalized_lifecycle_was_visible = claimed.normalized_lifecycle_was_visible;
        let adapter = self.adapter(transport).ok();
        let completed = tokio::time::timeout(INBOUND_ADMISSION_ADAPTER_CLEANUP_TIMEOUT, async {
            let Some(adapter) = adapter else {
                return false;
            };
            let stopped = if adapter.reject(connection_id.clone(), reason).await.is_ok() {
                true
            } else {
                adapter
                    .end(
                        connection_id.clone(),
                        EndReason::Failed {
                            detail: "inbound admission rejected".into(),
                        },
                    )
                    .await
                    .is_ok()
            };
            if !stopped {
                return false;
            }
            if let Some(forgotten) = self.begin_claimed_inbound_teardown(&claimed) {
                let forgotten = self
                    .finish_connection_teardown(&connection_id, forgotten)
                    .await;
                debug_assert!(forgotten.was_tracked);
                true
            } else {
                let lifecycle = claimed
                    .lifecycle
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                lifecycle.retired && !lifecycle.active
            }
        })
        .await;
        if !matches!(completed, Ok(true)) {
            metrics::counter!(
                "rvoip_core_inbound_admission_cleanup_total",
                "result" => "quarantined",
                "transport" => format!("{transport:?}")
            )
            .increment(1);
            return false;
        }
        metrics::counter!(
            "rvoip_core_inbound_admission_total",
            "result" => result,
            "transport" => format!("{transport:?}")
        )
        .increment(1);
        if normalized_lifecycle_was_visible {
            self.emit(Event::ConnectionFailed {
                connection_id,
                detail: "connection authentication policy rejected".into(),
                at: Utc::now(),
            });
        }
        true
    }

    async fn reject_expected_unadmitted_connection(
        self: &Arc<Self>,
        pending: &PendingInboundPublication,
        reason: RejectReason,
        result: &'static str,
    ) -> bool {
        let Some(claimed) = self.claim_pending_inbound_rejection(
            &pending.connection_id,
            pending.transport,
            Some(&pending.lifecycle),
        ) else {
            return false;
        };
        self.reject_claimed_unadmitted_connection(claimed, reason, result)
            .await
    }

    async fn wait_for_inbound_admission(
        self: Arc<Self>,
        pending: PendingInboundPublication,
        decision: tokio::sync::oneshot::Receiver<InboundAdmissionDecision>,
        decision_timeout: Duration,
        _permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        let decision = tokio::time::timeout(decision_timeout, decision).await;
        match decision {
            Ok(Ok(InboundAdmissionDecision {
                disposition: InboundAdmissionDisposition::Accept,
                completion,
            })) => {
                let published = self.publish_inbound_if_current(&pending);
                if published {
                    metrics::counter!(
                        "rvoip_core_inbound_admission_total",
                        "result" => "accepted",
                        "transport" => format!("{:?}", pending.transport)
                    )
                    .increment(1);
                } else {
                    self.reject_expected_unadmitted_connection(
                        &pending,
                        RejectReason::ServerError,
                        "stale_accept",
                    )
                    .await;
                }
                if let Some(completion) = completion {
                    let _ = completion.send(published);
                }
            }
            Ok(Ok(InboundAdmissionDecision {
                disposition: InboundAdmissionDisposition::Reject(reason),
                completion,
            })) => {
                let rejected = self
                    .reject_expected_unadmitted_connection(&pending, reason, "rejected")
                    .await;
                if let Some(completion) = completion {
                    let _ = completion.send(rejected);
                }
            }
            Ok(Err(_)) => {
                self.reject_expected_unadmitted_connection(
                    &pending,
                    RejectReason::ServerError,
                    "decision_closed",
                )
                .await;
            }
            Err(_) => {
                self.reject_expected_unadmitted_connection(
                    &pending,
                    RejectReason::ServerError,
                    "decision_timeout",
                )
                .await;
            }
        }
    }

    async fn gate_or_publish_inbound(self: &Arc<Self>, pending: PendingInboundPublication) {
        let Some(gate) = self.inbound_admission_gate.get() else {
            let _ = self.publish_inbound_if_current(&pending);
            return;
        };

        let pending_committed = {
            let _registry = self
                .connection_registry_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(current) = self.connection_lifecycles.get(&pending.connection_id) else {
                return;
            };
            if !Arc::ptr_eq(current.value(), &pending.lifecycle.state) {
                return;
            }
            let lifecycle = pending
                .lifecycle
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !lifecycle.active
                || lifecycle.retired
                || lifecycle.generation != pending.lifecycle.generation
            {
                return;
            }
            let Some(mut entry) = self.connections.get_mut(&pending.connection_id) else {
                return;
            };
            if entry.transport != pending.transport
                || entry.inbound_publication != InboundPublicationState::Unseen
            {
                false
            } else {
                entry.inbound_publication =
                    InboundPublicationState::Pending(pending.lifecycle.generation);
                true
            }
        };
        if !pending_committed {
            return;
        }

        let permit = match Arc::clone(&gate.permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.reject_expected_unadmitted_connection(
                    &pending,
                    RejectReason::ServerError,
                    "capacity",
                )
                .await;
                return;
            }
        };
        let (decision, resolved) = tokio::sync::oneshot::channel();
        let admission = InboundAdmission::new(
            pending.connection_id.clone(),
            pending.transport,
            pending.observed_at,
            pending.lifecycle.generation,
            Arc::downgrade(self),
            decision,
        );
        if let Err(error) = gate.sender.try_send(admission) {
            drop(resolved);
            drop(error.into_inner());
            self.reject_expected_unadmitted_connection(
                &pending,
                RejectReason::ServerError,
                "queue_unavailable",
            )
            .await;
            return;
        }

        let decision_timeout = gate.decision_timeout;
        let orchestrator = Arc::clone(self);
        tokio::spawn(async move {
            orchestrator
                .wait_for_inbound_admission(pending, resolved, decision_timeout, permit)
                .await;
        });
    }

    /// Atomically linearize a fail-closed decision against publication and
    /// capture the exact lifecycle identity the cleanup is allowed to erase.
    fn claim_pending_inbound_rejection(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
        expected: Option<&ConnectionLifecycleTicket>,
    ) -> Option<ClaimedInboundRejection> {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self.connection_lifecycles.get(connection_id)?;
        if let Some(expected) = expected {
            if expected.connection_id != *connection_id
                || !Arc::ptr_eq(current.value(), &expected.state)
            {
                return None;
            }
        }
        let lifecycle_state = Arc::clone(current.value());
        let lifecycle = lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active
            || lifecycle.retired
            || expected.is_some_and(|expected| expected.generation != lifecycle.generation)
        {
            return None;
        }
        let generation = lifecycle.generation;
        let mut entry = self.connections.get_mut(connection_id)?;
        if entry.transport != transport
            || entry.inbound_publication != InboundPublicationState::Pending(generation)
        {
            return None;
        }
        entry.inbound_publication = InboundPublicationState::Rejecting(generation);
        drop(entry);
        drop(lifecycle);
        drop(current);
        Some(ClaimedInboundRejection {
            connection_id: connection_id.clone(),
            transport,
            lifecycle: ConnectionLifecycleTicket {
                connection_id: connection_id.clone(),
                generation,
                state: lifecycle_state,
            },
            normalized_lifecycle_was_visible: false,
        })
    }

    fn decide_operational_adapter_event(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
    ) -> OperationalEventDecision {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = self.connection_lifecycles.get(connection_id) else {
            return OperationalEventDecision::Drop;
        };
        let lifecycle_state = Arc::clone(current.value());
        let lifecycle = lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active || lifecycle.retired {
            return OperationalEventDecision::Drop;
        }
        let generation = lifecycle.generation;
        let Some(mut entry) = self.connections.get_mut(connection_id) else {
            return OperationalEventDecision::Drop;
        };
        if entry.transport != transport {
            return OperationalEventDecision::Drop;
        }
        match entry.inbound_publication {
            InboundPublicationState::Published => OperationalEventDecision::Published,
            InboundPublicationState::Pending(pending_generation)
                if pending_generation == generation =>
            {
                entry.inbound_publication = InboundPublicationState::Rejecting(generation);
                OperationalEventDecision::Reject(ClaimedInboundRejection {
                    connection_id: connection_id.clone(),
                    transport,
                    lifecycle: ConnectionLifecycleTicket {
                        connection_id: connection_id.clone(),
                        generation,
                        state: Arc::clone(&lifecycle_state),
                    },
                    normalized_lifecycle_was_visible: false,
                })
            }
            InboundPublicationState::NotInbound
            | InboundPublicationState::Unseen
            | InboundPublicationState::Pending(_)
            | InboundPublicationState::Rejecting(_) => OperationalEventDecision::Drop,
        }
    }

    fn claim_route_policy_rejection(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
    ) -> Option<ClaimedInboundRejection> {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self.connection_lifecycles.get(connection_id)?;
        let lifecycle_state = Arc::clone(current.value());
        let lifecycle = lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active || lifecycle.retired {
            return None;
        }
        let generation = lifecycle.generation;
        let mut entry = self.connections.get_mut(connection_id)?;
        if entry.transport != transport {
            return None;
        }
        let normalized_lifecycle_was_visible = match entry.inbound_publication {
            InboundPublicationState::Pending(pending_generation)
                if pending_generation == generation =>
            {
                false
            }
            InboundPublicationState::Published => true,
            InboundPublicationState::NotInbound
            | InboundPublicationState::Unseen
            | InboundPublicationState::Pending(_)
            | InboundPublicationState::Rejecting(_) => return None,
        };
        entry.inbound_publication = InboundPublicationState::Rejecting(generation);
        Some(ClaimedInboundRejection {
            connection_id: connection_id.clone(),
            transport,
            lifecycle: ConnectionLifecycleTicket {
                connection_id: connection_id.clone(),
                generation,
                state: Arc::clone(&lifecycle_state),
            },
            normalized_lifecycle_was_visible,
        })
    }

    fn decide_principal_adapter_event(
        &self,
        connection_id: &ConnectionId,
        transport: Transport,
        participant_id: &str,
        principal: &AuthenticatedPrincipal,
    ) -> PrincipalEventDecision {
        let _registry = self
            .connection_registry_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = self.connection_lifecycles.get(connection_id) else {
            return PrincipalEventDecision::Drop;
        };
        let lifecycle_state = Arc::clone(current.value());
        let lifecycle = lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !lifecycle.active || lifecycle.retired {
            return PrincipalEventDecision::Drop;
        }
        let generation = lifecycle.generation;
        let Some(mut entry) = self.connections.get_mut(connection_id) else {
            return PrincipalEventDecision::Drop;
        };
        if entry.transport != transport {
            return PrincipalEventDecision::Drop;
        }
        let normalized_lifecycle_was_visible = match entry.inbound_publication {
            InboundPublicationState::Pending(pending_generation)
                if pending_generation == generation =>
            {
                false
            }
            InboundPublicationState::Published => true,
            InboundPublicationState::NotInbound
            | InboundPublicationState::Unseen
            | InboundPublicationState::Pending(_)
            | InboundPublicationState::Rejecting(_) => return PrincipalEventDecision::Drop,
        };
        let tenantless = principal.tenant.as_deref().is_none_or(str::is_empty);
        let owner_mismatch = entry
            .principal
            .as_ref()
            .is_some_and(|current| !current.has_same_owner(principal));
        if tenantless || owner_mismatch {
            entry.inbound_publication = InboundPublicationState::Rejecting(generation);
            return PrincipalEventDecision::Reject(ClaimedInboundRejection {
                connection_id: connection_id.clone(),
                transport,
                lifecycle: ConnectionLifecycleTicket {
                    connection_id: connection_id.clone(),
                    generation,
                    state: Arc::clone(&lifecycle_state),
                },
                normalized_lifecycle_was_visible,
            });
        }
        if entry.principal.is_some() {
            // Freeze the first complete principal for this lifecycle. A
            // same-owner refresh may change scopes, method, assurance, or
            // expiry and therefore cannot replace the authorization snapshot
            // inspected by admission policy.
            return PrincipalEventDecision::Handled;
        }
        entry.principal = Some(principal.clone());
        if !normalized_lifecycle_was_visible {
            entry.deferred_principal_authentication = Some(DeferredPrincipalAuthentication {
                participant_id: participant_id.to_owned(),
                principal: principal.clone(),
                at: Utc::now(),
            });
            PrincipalEventDecision::Handled
        } else {
            PrincipalEventDecision::Publish
        }
    }

    async fn handle_orchestrator_adapter_event(
        self: &Arc<Self>,
        transport: Transport,
        event: OrchestratorAdapterEvent,
    ) {
        match event {
            OrchestratorAdapterEvent::Public(event) => {
                self.handle_adapter_event(transport, event).await;
            }
            OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                connection,
                participant_id,
                principal,
            } => {
                if !self.adapter_connection_is_live(transport, &connection.id) {
                    return;
                }
                let malformed = connection.transport != transport
                    || connection.direction != Direction::Inbound
                    || principal.tenant.as_deref().is_none_or(str::is_empty);
                if malformed {
                    let _ = self
                        .adapter(transport)
                        .ok()
                        .and_then(|adapter| adapter.take_inbound_context(&connection.id));
                    if let Some(claimed) =
                        self.claim_route_policy_rejection(&connection.id, transport)
                    {
                        self.reject_claimed_unadmitted_connection(
                            claimed,
                            RejectReason::ServerError,
                            "malformed_inbound",
                        )
                        .await;
                    } else {
                        self.retire_untracked_connection_id(&connection.id);
                        self.reject_colliding_adapter_route(transport, connection.id)
                            .await;
                    }
                    return;
                }
                if self.connection_owned_by_other_transport(&connection.id, transport) {
                    self.reject_colliding_adapter_route(transport, connection.id)
                        .await;
                    return;
                }
                let existing_state = self
                    .connections
                    .get(&connection.id)
                    .map(|entry| entry.inbound_publication);
                let update_pending = existing_state
                    .is_some_and(|state| matches!(state, InboundPublicationState::Pending(_)));
                let incompatible_existing = existing_state.is_some_and(|state| {
                    matches!(
                        state,
                        InboundPublicationState::NotInbound | InboundPublicationState::Unseen
                    )
                });
                if incompatible_existing {
                    self.reject_colliding_adapter_route(transport, connection.id)
                        .await;
                    return;
                }
                let discard_duplicate = existing_state.is_some_and(|state| {
                    matches!(
                        state,
                        InboundPublicationState::Rejecting(_) | InboundPublicationState::Published
                    )
                });
                if discard_duplicate {
                    // A repeated atomic handoff must not retain a second
                    // adapter-owned attachment context.
                    let _ = self
                        .adapter(transport)
                        .ok()
                        .and_then(|adapter| adapter.take_inbound_context(&connection.id));
                    return;
                }
                let inbound_context = self
                    .adapter(transport)
                    .ok()
                    .and_then(|adapter| adapter.take_inbound_context(&connection.id))
                    .filter(|context| {
                        connection.transport == transport
                            && context.connection_id() == &connection.id
                            && context.transport() == transport
                    });
                if update_pending {
                    let update = {
                        let _registry = self
                            .connection_registry_lock
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        let Some(current) = self.connection_lifecycles.get(&connection.id) else {
                            return;
                        };
                        let lifecycle_state = Arc::clone(current.value());
                        let lifecycle = lifecycle_state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if !lifecycle.active || lifecycle.retired {
                            return;
                        }
                        let generation = lifecycle.generation;
                        let Some(mut entry) = self.connections.get_mut(&connection.id) else {
                            return;
                        };
                        if entry.inbound_publication != InboundPublicationState::Pending(generation)
                        {
                            return;
                        }
                        if entry.transport != transport {
                            AtomicPendingUpdate::TransportCollision
                        } else if entry
                            .principal
                            .as_ref()
                            .is_some_and(|current| !current.has_same_owner(&principal))
                        {
                            entry.inbound_publication =
                                InboundPublicationState::Rejecting(generation);
                            AtomicPendingUpdate::Reject(ClaimedInboundRejection {
                                connection_id: connection.id.clone(),
                                transport,
                                lifecycle: ConnectionLifecycleTicket {
                                    connection_id: connection.id.clone(),
                                    generation,
                                    state: Arc::clone(&lifecycle_state),
                                },
                                normalized_lifecycle_was_visible: false,
                            })
                        } else {
                            if !entry.inbound_context_retired && entry.inbound_context.is_none() {
                                let context_principal =
                                    entry.principal.clone().unwrap_or_else(|| principal.clone());
                                entry.inbound_context = match inbound_context {
                                    Some(context)
                                        if context.is_bound_to(
                                            &connection.id,
                                            transport,
                                            &context_principal,
                                        ) =>
                                    {
                                        Some(context)
                                    }
                                    Some(_) => {
                                        entry.inbound_context_retired = true;
                                        None
                                    }
                                    None => None,
                                };
                            }
                            if entry.principal.is_none() {
                                entry.principal = Some(principal.clone());
                                entry.deferred_principal_authentication =
                                    Some(DeferredPrincipalAuthentication {
                                        participant_id,
                                        principal,
                                        at: Utc::now(),
                                    });
                            }
                            AtomicPendingUpdate::Handled
                        }
                    };
                    match update {
                        AtomicPendingUpdate::Handled => {}
                        AtomicPendingUpdate::Reject(claimed) => {
                            self.reject_claimed_unadmitted_connection(
                                claimed,
                                RejectReason::ServerError,
                                "principal_changed",
                            )
                            .await;
                        }
                        AtomicPendingUpdate::TransportCollision => {
                            self.reject_colliding_adapter_route(transport, connection.id)
                                .await;
                        }
                    }
                    return;
                }
                if !self.track_connection(&connection.id, transport, inbound_context)
                    || !self.track_connection_principal(
                        &connection.id,
                        transport,
                        principal.clone(),
                    )
                {
                    self.reject_colliding_adapter_route(transport, connection.id)
                        .await;
                    return;
                }
                if self.mark_connection_inbound(&connection.id).is_err() {
                    return;
                }
                if !self.adapter_connection_is_live(transport, &connection.id) {
                    self.forget_connection(&connection.id).await;
                    return;
                }
                let at = Utc::now();
                let lifecycle = match self
                    .capture_connection_lifecycles(std::slice::from_ref(&connection.id))
                {
                    Ok(mut lifecycle) => lifecycle.remove(0),
                    Err(_) => return,
                };
                self.gate_or_publish_inbound(PendingInboundPublication {
                    connection_id: connection.id,
                    transport,
                    participant_id: Some(participant_id),
                    principal: Some(principal),
                    observed_at: at,
                    lifecycle,
                })
                .await;
            }
        }
    }

    async fn handle_adapter_event(self: &Arc<Self>, transport: Transport, event: AdapterEvent) {
        let scoped_connection_id = match &event {
            AdapterEvent::InboundConnection { connection } => Some(&connection.id),
            AdapterEvent::Connected { connection_id }
            | AdapterEvent::Authenticated { connection_id, .. }
            | AdapterEvent::PrincipalAuthenticated { connection_id, .. }
            | AdapterEvent::Dtmf { connection_id, .. }
            | AdapterEvent::Quality { connection_id, .. }
            | AdapterEvent::Message { connection_id, .. }
            | AdapterEvent::DataMessage { connection_id, .. }
            | AdapterEvent::StepUpResponse { connection_id, .. } => Some(connection_id),
            // Terminal events arrive after the adapter removes its route.
            // Native events are not connection-scoped.
            AdapterEvent::Ended { .. }
            | AdapterEvent::Failed { .. }
            | AdapterEvent::Native { .. } => None,
            _ => None,
        };
        if let Some(connection_id) = scoped_connection_id {
            if !self.adapter_connection_is_live(transport, connection_id) {
                debug!(
                    ?transport,
                    ?connection_id,
                    "ignoring stale adapter event for a route that has ended"
                );
                return;
            }
            let malformed_inbound = matches!(
                &event,
                AdapterEvent::InboundConnection { connection }
                    if connection.transport != transport
                        || connection.direction != Direction::Inbound
            );
            if malformed_inbound {
                if let Some(claimed) = self.claim_route_policy_rejection(connection_id, transport) {
                    self.reject_claimed_unadmitted_connection(
                        claimed,
                        RejectReason::ServerError,
                        "malformed_inbound",
                    )
                    .await;
                } else {
                    self.retire_untracked_connection_id(connection_id);
                    self.reject_colliding_adapter_route(transport, connection_id.clone())
                        .await;
                }
                return;
            }
            if self.connection_owned_by_other_transport(connection_id, transport) {
                self.reject_colliding_adapter_route(transport, connection_id.clone())
                    .await;
                return;
            }
            let inbound_collides_with_setup = matches!(
                &event,
                AdapterEvent::InboundConnection { .. }
                    if self.connections.get(connection_id).is_some_and(|entry| {
                        matches!(
                            entry.inbound_publication,
                            InboundPublicationState::NotInbound
                                | InboundPublicationState::Unseen
                        )
                    })
            );
            if inbound_collides_with_setup {
                self.reject_colliding_adapter_route(transport, connection_id.clone())
                    .await;
                return;
            }
            if self.connections.get(connection_id).is_some_and(|entry| {
                matches!(
                    entry.inbound_publication,
                    InboundPublicationState::Rejecting(_)
                )
            }) {
                return;
            }

            match &event {
                AdapterEvent::InboundConnection { .. } => {}
                AdapterEvent::Authenticated {
                    identity_id,
                    participant_id,
                    assurance,
                    ..
                } => {
                    let Some(mut entry) = self.connections.get_mut(connection_id) else {
                        return;
                    };
                    if matches!(
                        entry.inbound_publication,
                        InboundPublicationState::Pending(_)
                    ) {
                        if entry.deferred_authentication.is_none() {
                            entry.deferred_authentication = Some(DeferredAuthentication {
                                identity_id: identity_id.clone(),
                                participant_id: participant_id.clone(),
                                assurance: assurance.clone(),
                                at: Utc::now(),
                            });
                        }
                        return;
                    }
                    if entry.inbound_publication != InboundPublicationState::Published {
                        return;
                    }
                }
                AdapterEvent::PrincipalAuthenticated {
                    participant_id,
                    principal,
                    ..
                } => match self.decide_principal_adapter_event(
                    connection_id,
                    transport,
                    participant_id,
                    principal,
                ) {
                    PrincipalEventDecision::Handled | PrincipalEventDecision::Drop => return,
                    PrincipalEventDecision::Publish => {}
                    PrincipalEventDecision::Reject(claimed) => {
                        self.reject_claimed_unadmitted_connection(
                            claimed,
                            RejectReason::ServerError,
                            "principal_policy",
                        )
                        .await;
                        return;
                    }
                },
                AdapterEvent::Connected { .. }
                | AdapterEvent::Dtmf { .. }
                | AdapterEvent::Quality { .. }
                | AdapterEvent::Message { .. }
                | AdapterEvent::DataMessage { .. }
                | AdapterEvent::StepUpResponse { .. } => {
                    match self.decide_operational_adapter_event(connection_id, transport) {
                        OperationalEventDecision::Published => {}
                        OperationalEventDecision::Drop => return,
                        OperationalEventDecision::Reject(claimed) => {
                            self.reject_claimed_unadmitted_connection(
                                claimed,
                                RejectReason::ServerError,
                                "event_before_admission",
                            )
                            .await;
                            return;
                        }
                    }
                }
                AdapterEvent::Ended { .. }
                | AdapterEvent::Failed { .. }
                | AdapterEvent::Native { .. } => {}
                _ => {}
            }
        }

        match event {
            AdapterEvent::InboundConnection { connection } => {
                let inbound_context = self
                    .adapter(transport)
                    .ok()
                    .and_then(|adapter| adapter.take_inbound_context(&connection.id))
                    .filter(|context| {
                        connection.transport == transport
                            && context.connection_id() == &connection.id
                            && context.transport() == transport
                    });
                if !self.track_connection(&connection.id, transport, inbound_context) {
                    self.reject_colliding_adapter_route(transport, connection.id)
                        .await;
                    return;
                }
                if self.mark_connection_inbound(&connection.id).is_err() {
                    return;
                }
                if !self.adapter_connection_is_live(transport, &connection.id) {
                    self.forget_connection(&connection.id).await;
                    return;
                }
                let observed_at = Utc::now();
                let lifecycle = match self
                    .capture_connection_lifecycles(std::slice::from_ref(&connection.id))
                {
                    Ok(mut lifecycle) => lifecycle.remove(0),
                    Err(_) => return,
                };
                self.gate_or_publish_inbound(PendingInboundPublication {
                    connection_id: connection.id,
                    transport,
                    participant_id: None,
                    principal: None,
                    observed_at,
                    lifecycle,
                })
                .await;
            }
            AdapterEvent::Connected { connection_id } => {
                self.emit(Event::ConnectionConnected {
                    connection_id,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Authenticated {
                connection_id,
                identity_id,
                participant_id,
                assurance,
            } => {
                self.emit(Event::ConnectionAuthenticated {
                    connection_id,
                    identity_id,
                    participant_id,
                    assurance,
                    at: Utc::now(),
                });
            }
            AdapterEvent::PrincipalAuthenticated {
                connection_id,
                participant_id,
                principal,
            } => {
                let at = Utc::now();
                // Preserve the legacy normalized event for existing
                // subscribers, then publish the complete principal additively.
                self.emit(Event::ConnectionAuthenticated {
                    connection_id: connection_id.clone(),
                    identity_id: principal.subject.clone(),
                    participant_id: participant_id.clone(),
                    assurance: principal.assurance.clone(),
                    at,
                });
                self.emit(Event::ConnectionPrincipalAuthenticated {
                    connection_id,
                    participant_id,
                    principal,
                    at,
                });
            }
            AdapterEvent::Ended {
                connection_id,
                reason,
            } => {
                if self.connection_owned_by_other_transport(&connection_id, transport) {
                    return;
                }
                let forgotten = self.forget_connection(&connection_id).await;
                if forgotten.was_tracked && forgotten.normalized_lifecycle_was_visible {
                    self.emit(Event::ConnectionEnded {
                        connection_id,
                        reason,
                        at: Utc::now(),
                    });
                }
            }
            AdapterEvent::Failed {
                connection_id,
                detail,
            } => {
                if self.connection_owned_by_other_transport(&connection_id, transport) {
                    return;
                }
                let forgotten = self.forget_connection(&connection_id).await;
                if forgotten.was_tracked && forgotten.normalized_lifecycle_was_visible {
                    self.emit(Event::ConnectionFailed {
                        connection_id,
                        detail,
                        at: Utc::now(),
                    });
                }
            }
            AdapterEvent::Dtmf {
                connection_id,
                digits,
                duration_ms,
            } => {
                // `Event::DtmfReceived` carries digits + connection_id
                // only — duration_ms is dropped at the orchestrator
                // boundary (it's transport-detail). Consumers that need
                // per-digit timing subscribe to the adapter event
                // stream directly. Plan C2.
                self.emit(Event::DtmfReceived {
                    connection_id: connection_id.clone(),
                    digits: digits.clone(),
                    at: Utc::now(),
                });
                // Gap plan §4.3 / v1 punch list — cross-bridge DTMF
                // auto-route. When the connection is part of a
                // cross-transport bridge, forward the digits to the
                // peer leg via the adapter's `send_dtmf`. This is what
                // makes UCTP→SIP DTMF work end-to-end without app
                // code: a UCTP peer signals digits out-of-band via
                // `dtmf.send`, the SIP-side adapter synthesizes RFC
                // 4733 packets onto outbound RTP.
                //
                // `handle_adapter_event` is synchronous; spawn a task
                // so the forward doesn't block adapter-event ingest.
                if let Some(peer) = self.bridge_peer_of(&connection_id) {
                    metrics::counter!("uctp_bridge_dtmf_forwarded_total").increment(1);
                    let peer_for_task = peer.clone();
                    let digits_for_task = digits.clone();
                    let adapter = self.adapter_for(&peer);
                    match adapter {
                        Ok(adapter) => {
                            let src = connection_id.clone();
                            tokio::spawn(async move {
                                match adapter
                                    .send_dtmf(peer_for_task.clone(), &digits_for_task, duration_ms)
                                    .await
                                {
                                    Ok(()) => {
                                        debug!(
                                            ?src,
                                            ?peer_for_task,
                                            digits = %digits_for_task,
                                            "orchestrator: auto-forwarded DTMF across cross-transport bridge"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            ?src,
                                            ?peer_for_task,
                                            error = %e,
                                            "orchestrator: cross-bridge DTMF auto-forward failed"
                                        );
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            warn!(
                                ?connection_id,
                                ?peer,
                                error = %e,
                                "orchestrator: cross-bridge DTMF auto-forward — no adapter for peer transport"
                            );
                        }
                    }
                }
            }
            AdapterEvent::Quality {
                connection_id,
                snapshot,
            } => {
                // P9 — feed the per-Session aggregator so
                // `Event::SessionEnded.report` carries averaged
                // quality at session end.
                if let Some(sid) = self.session_of(&connection_id) {
                    let mut entry = self.session_quality.entry(sid).or_default();
                    entry.add(&snapshot, None);
                }
                metrics::gauge!("rvoip_media_jitter_ms").set(snapshot.jitter_ms as f64);
                metrics::gauge!("rvoip_media_packet_loss_pct").set(snapshot.packet_loss_pct as f64);
                self.emit(Event::MediaQuality {
                    connection_id,
                    snapshot,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Message {
                connection_id,
                text,
            } => {
                let Some((conversation_id, participant_id)) =
                    self.message_context_for_connection(&connection_id)
                else {
                    warn!(
                        ?transport,
                        ?connection_id,
                        "adapter message arrived before connection was accepted into a session"
                    );
                    return;
                };
                let now = Utc::now();
                let message = Message {
                    id: MessageId::new(),
                    conversation_id: conversation_id.clone(),
                    origin: MessageOrigin::Connection(connection_id.clone()),
                    from_participant: participant_id,
                    to: MessageRecipients::All,
                    direction: Direction::Inbound,
                    content_type: ContentType::Text,
                    body: Bytes::from(text),
                    attachments: vec![],
                    in_reply_to: None,
                    timestamp: now,
                };
                if let Err(error) = Self::validate_inline_body(&message) {
                    warn!(
                        ?connection_id,
                        error = %error,
                        "adapter message rejected by inline body policy"
                    );
                    return;
                }
                let message_id = message.id.clone();
                if let Err(error) = self.config.message_store.put(message).await {
                    warn!(
                        ?connection_id,
                        ?conversation_id,
                        error = %error,
                        "MessageStore::put failed for adapter message"
                    );
                    return;
                }
                if let Some(conv_arc) = self.conversation(&conversation_id) {
                    if let Ok(mut conv) = conv_arc.write() {
                        conv.messages.push(message_id.clone());
                        conv.last_activity_at = now;
                    }
                }
                self.emit(Event::MessageReceived {
                    message_id,
                    conversation_id,
                    at: now,
                });
            }
            AdapterEvent::DataMessage {
                connection_id,
                message,
            } => {
                if let Err(error) = message.validate() {
                    warn!(
                        ?connection_id,
                        error = %error,
                        "invalid adapter data message rejected"
                    );
                    return;
                }
                if !self.connections.contains_key(&connection_id) {
                    warn!(
                        ?transport,
                        ?connection_id,
                        "data message rejected for untracked connection"
                    );
                    return;
                }
                self.emit(Event::DataMessageReceived {
                    connection_id: connection_id.clone(),
                    message: message.clone(),
                    at: Utc::now(),
                });

                if !matches!(message.label.as_str(), "rvoip-chat" | "rvoip-messages") {
                    return;
                }
                let Some((conversation_id, participant_id)) =
                    self.message_context_for_connection(&connection_id)
                else {
                    warn!(
                        ?transport,
                        ?connection_id,
                        "legacy data-message projection arrived before session attachment"
                    );
                    return;
                };
                let now = Utc::now();
                let media_type = message
                    .content_type
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                let content_type =
                    if media_type == "application/json" || media_type.ends_with("+json") {
                        ContentType::Json
                    } else if media_type.starts_with("text/") {
                        ContentType::Text
                    } else {
                        ContentType::Binary
                    };
                let legacy = Message {
                    id: message.message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    origin: MessageOrigin::Connection(connection_id.clone()),
                    from_participant: participant_id,
                    to: MessageRecipients::All,
                    direction: Direction::Inbound,
                    content_type,
                    body: message.bytes,
                    attachments: vec![],
                    in_reply_to: None,
                    timestamp: now,
                };
                let message_id = legacy.id.clone();
                if let Err(error) = self.config.message_store.put(legacy).await {
                    warn!(
                        ?connection_id,
                        ?conversation_id,
                        error = %error,
                        "MessageStore::put failed for projected data message"
                    );
                    return;
                }
                if let Some(conv_arc) = self.conversation(&conversation_id) {
                    if let Ok(mut conv) = conv_arc.write() {
                        conv.messages.push(message_id.clone());
                        conv.last_activity_at = now;
                    }
                }
                self.emit(Event::MessageReceived {
                    message_id,
                    conversation_id,
                    at: now,
                });
            }
            AdapterEvent::StepUpResponse {
                connection_id,
                method,
                credential,
            } => {
                // P12.6 — re-emit as a public event so the consumer
                // can resolve `(method, credential)` to a real
                // `Credential` and call `complete_step_up`. The
                // orchestrator deliberately doesn't auto-call
                // `complete_step_up` because that requires an
                // `IdentityProvider`, which is consumer-owned per
                // INTERFACE_DESIGN §8.
                self.emit(Event::IdentityStepUpResponseReceived {
                    connection_id,
                    method,
                    credential,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Native { kind, detail } => {
                debug!(
                    ?transport,
                    ?kind,
                    ?detail,
                    "adapter native event (unmapped)"
                );
            }
            _ => {
                metrics::counter!("rvoip_core_unhandled_adapter_events_total").increment(1);
                debug!(
                    ?transport,
                    "adapter event variant has no orchestrator mapping; dropping"
                );
            }
        }
    }

    fn message_context_for_connection(
        &self,
        connection_id: &ConnectionId,
    ) -> Option<(ConversationId, ParticipantId)> {
        let session_id = self.session_of(connection_id)?;
        let session_arc = self.session(&session_id)?;
        let session = session_arc.read().ok()?;
        let participant_id = session
            .connections
            .get(connection_id)
            .map(|connection| connection.participant_id.clone())?;
        Some((session.conversation_id.clone(), participant_id))
    }

    // ------------------------------------------------------------------
    // Command surface — dispatched via ConnectionAdapter.
    // ------------------------------------------------------------------

    pub async fn route_inbound_connection(
        &self,
        connection_id: ConnectionId,
        action: InboundAction,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        match action {
            // P1.8 — bind the Connection to its target Session before
            // accepting, so the first `AdapterEvent::Connected` arrives
            // on a Session that already lists this connection. Auto-
            // transitions Initiating → Active on first attach.
            InboundAction::Accept {
                session_id,
                participant_id,
            } => {
                let sess_arc = self
                    .sessions
                    .get(&session_id)
                    .map(|e| Arc::clone(e.value()))
                    .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;
                {
                    let mut sess = sess_arc.write().expect("session lock poisoned");
                    if matches!(sess.state, SessionState::Ended | SessionState::Failed) {
                        return Err(RvoipError::InvalidState(
                            "route_inbound_connection: target session is ended",
                        ));
                    }
                    sess.connections.insert(
                        connection_id.clone(),
                        ConnectionRef {
                            id: connection_id.clone(),
                            participant_id,
                        },
                    );
                    if sess.state == SessionState::Initiating {
                        sess.state = SessionState::Active;
                    }
                }
                self.sessions_by_connection
                    .insert(connection_id.clone(), session_id);
                adapter.accept(connection_id).await
            }
            InboundAction::Reject { reason } => adapter.reject(connection_id, reason).await,
            // P2 — inbound gateway pattern: accept the inbound leg,
            // originate the outbound leg, bridge them. The outbound's
            // transport selection still uses the v0 "first adapter"
            // heuristic until P6 adds the `transport` field to
            // OriginateRequest; if the outbound and inbound share a
            // transport (common case: SIP↔SIP gateway), that's fine.
            InboundAction::BridgeTo {
                session_id,
                outbound,
            } => {
                // 1. Bind inbound to the named Session + accept it.
                let sess_arc = self
                    .sessions
                    .get(&session_id)
                    .map(|e| Arc::clone(e.value()))
                    .ok_or_else(|| RvoipError::SessionNotFound(session_id.clone()))?;
                {
                    let mut sess = sess_arc.write().expect("session lock poisoned");
                    if matches!(sess.state, SessionState::Ended | SessionState::Failed) {
                        return Err(RvoipError::InvalidState(
                            "BridgeTo: target session is ended",
                        ));
                    }
                    sess.connections.insert(
                        connection_id.clone(),
                        ConnectionRef {
                            id: connection_id.clone(),
                            // BridgeTo doesn't carry a participant_id;
                            // use the outbound's invitee as the
                            // gateway-side identity placeholder.
                            participant_id: outbound.participant_id.clone(),
                        },
                    );
                    if sess.state == SessionState::Initiating {
                        sess.state = SessionState::Active;
                    }
                }
                self.sessions_by_connection
                    .insert(connection_id.clone(), session_id.clone());
                adapter.accept(connection_id.clone()).await?;

                // 2. Originate the outbound.
                let out_handle = self.originate_connection(outbound).await?;
                let out_id = out_handle.connection.id.clone();
                // Bind the outbound to the same Session.
                {
                    let mut sess = sess_arc.write().expect("session lock poisoned");
                    sess.connections.insert(
                        out_id.clone(),
                        ConnectionRef {
                            id: out_id.clone(),
                            participant_id: out_handle.connection.participant_id.clone(),
                        },
                    );
                }
                self.sessions_by_connection
                    .insert(out_id.clone(), session_id);

                // 3. Bridge them. Errors here roll up; we leave the
                // legs attached to the Session so the caller can
                // observe + tear down explicitly.
                self.bridge_connections(connection_id, out_id).await?;
                Ok(())
            }
        }
    }

    #[instrument(skip(self, request), fields(target = %request.target, transport = ?request.transport, connection_id))]
    pub async fn originate_connection(
        &self,
        request: OriginateRequest,
    ) -> Result<ConnectionHandle> {
        let session_id = request.session_id.clone();
        let participant_id = request.participant_id.clone();
        self.bind_connection_to_session_probe(&session_id)?;
        // P6 — caller-selected transport takes precedence; fall back
        // to the v0 "first registered adapter" path when the request
        // doesn't specify (back-compat for single-adapter
        // deployments).
        let transport = match request.transport {
            Some(t) => t,
            None => self
                .adapters
                .iter()
                .next()
                .map(|entry| *entry.key())
                .ok_or(RvoipError::NotImplemented(
                    "no adapter registered — register one before originating",
                ))?,
        };
        let adapter = self.adapter(transport)?;
        let handle = adapter.originate(request).await?;
        if handle.connection.transport != transport
            || handle.connection.direction != Direction::Outbound
        {
            self.retire_untracked_connection_id(&handle.connection.id);
            self.reject_colliding_adapter_route(transport, handle.connection.id.clone())
                .await;
            return Err(RvoipError::AdmissionRejected(
                "originated connection transport or direction mismatch",
            ));
        }
        if !self.track_connection(&handle.connection.id, transport, None) {
            self.reject_colliding_adapter_route(transport, handle.connection.id.clone())
                .await;
            return Err(RvoipError::AdmissionRejected(
                "connection ID is owned by another transport",
            ));
        }
        let published = self
            .connections
            .get_mut(&handle.connection.id)
            .is_some_and(|mut entry| {
                if entry.transport != transport {
                    return false;
                }
                entry.inbound_publication = InboundPublicationState::Published;
                true
            });
        if !published {
            self.reject_colliding_adapter_route(transport, handle.connection.id.clone())
                .await;
            return Err(RvoipError::AdmissionRejected(
                "connection ID transport ownership changed during originate",
            ));
        }
        self.bind_connection_to_session(&handle.connection.id, &session_id, participant_id)?;
        self.emit(Event::ConnectionOutbound {
            connection_id: handle.connection.id.clone(),
            at: Utc::now(),
        });
        Ok(handle)
    }

    /// P6 — ergonomic wrapper that sets `request.transport = Some(transport)`
    /// before dispatch. Equivalent to mutating the field directly.
    pub async fn originate_connection_via(
        &self,
        transport: Transport,
        mut request: OriginateRequest,
    ) -> Result<ConnectionHandle> {
        request.transport = Some(transport);
        self.originate_connection(request).await
    }

    pub async fn end_connection(
        &self,
        connection_id: ConnectionId,
        reason: EndReason,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.end(connection_id, reason).await
    }

    pub async fn hold(&self, connection_id: ConnectionId) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.hold(connection_id).await
    }

    pub async fn resume(&self, connection_id: ConnectionId) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.resume(connection_id).await
    }

    #[instrument(skip(self), fields(connection_id = %connection_id, target = ?target))]
    pub async fn transfer_connection(
        &self,
        connection_id: ConnectionId,
        target: TransferTarget,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.transfer(connection_id, target).await
    }

    pub async fn send_dtmf(
        &self,
        connection_id: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.send_dtmf(connection_id, digits, duration_ms).await
    }

    /// Legacy name retained for compatibility — alias of
    /// [`Self::send_message_to_connection`].
    pub async fn send_message(&self, connection_id: ConnectionId, message: Message) -> Result<()> {
        self.send_message_to_connection(connection_id, message)
            .await
    }

    /// P4 — send a Message to a single Connection (single-substrate hop).
    /// Persists the Message in the configured `MessageStore`, dispatches
    /// to the adapter, emits `MessageSent` then `MessageDelivered`.
    pub async fn send_message_to_connection(
        &self,
        connection_id: ConnectionId,
        message: Message,
    ) -> Result<()> {
        Self::validate_inline_body(&message)?;
        let adapter = self.adapter_for(&connection_id)?;
        let msg_id = message.id.clone();
        let cid = message.conversation_id.clone();
        self.config.message_store.put(message.clone()).await?;
        adapter.send_message(connection_id, message).await?;
        self.emit(Event::MessageSent {
            message_id: msg_id.clone(),
            conversation_id: cid,
            at: Utc::now(),
        });
        self.emit(Event::MessageDelivered {
            message_id: msg_id,
            at: Utc::now(),
        });
        Ok(())
    }

    pub async fn send_data_message(
        &self,
        connection_id: ConnectionId,
        message: DataMessage,
    ) -> Result<()> {
        self.send_data_message_to_connection(connection_id, message)
            .await
    }

    pub async fn send_data_message_to_connection(
        &self,
        connection_id: ConnectionId,
        message: DataMessage,
    ) -> Result<()> {
        message
            .validate()
            .map_err(|error| RvoipError::Adapter(format!("invalid data message: {error}")))?;
        let adapter = self.adapter_for(&connection_id)?;
        adapter.send_data_message(connection_id, message).await
    }

    /// P4 — fan-out a Message to every active Connection across every
    /// active Session within a Conversation. Persists once; emits
    /// `MessageSent` once + `MessageDelivered` per successful per-leg
    /// dispatch. Per-leg adapter errors are logged at `warn` and do
    /// not abort the fan-out.
    pub async fn send_message_to_conversation(
        &self,
        conversation_id: ConversationId,
        message: Message,
    ) -> Result<MessageId> {
        Self::validate_inline_body(&message)?;
        let conv_arc = self
            .conversations
            .get(&conversation_id)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::ConversationNotFound(conversation_id.clone()))?;

        let session_ids: Vec<SessionId> = {
            let conv = conv_arc.read().expect("conv lock poisoned");
            if conv.state != ConversationState::Open {
                return Err(RvoipError::InvalidState(
                    "send_message_to_conversation: conversation not Open",
                ));
            }
            conv.sessions.clone()
        };

        // Collect (connection_id, transport) snapshots for active Sessions.
        let mut legs: Vec<ConnectionId> = Vec::new();
        for sid in &session_ids {
            if let Some(sess_arc) = self.sessions.get(sid).map(|e| Arc::clone(e.value())) {
                let sess = sess_arc.read().expect("sess lock poisoned");
                if sess.state == SessionState::Active {
                    for cref in sess.connections.keys() {
                        legs.push(cref.clone());
                    }
                }
            }
        }

        let msg_id = message.id.clone();
        self.config.message_store.put(message.clone()).await?;
        self.emit(Event::MessageSent {
            message_id: msg_id.clone(),
            conversation_id,
            at: Utc::now(),
        });

        for connection_id in legs {
            match self.adapter_for(&connection_id) {
                Ok(adapter) => {
                    if let Err(e) = adapter
                        .send_message(connection_id.clone(), message.clone())
                        .await
                    {
                        warn!(?connection_id, error=%e, "per-leg send_message failed");
                        continue;
                    }
                    self.emit(Event::MessageDelivered {
                        message_id: msg_id.clone(),
                        at: Utc::now(),
                    });
                }
                Err(e) => warn!(?connection_id, error=%e, "no adapter for leg"),
            }
        }
        Ok(msg_id)
    }

    /// P4 — paginated history.
    pub async fn list_messages(
        &self,
        conversation_id: ConversationId,
        filter: crate::store::MessageFilter,
        page: Option<crate::store::PageCursor>,
    ) -> Result<crate::store::MessagePage> {
        self.config
            .message_store
            .list(&conversation_id, filter, page)
            .await
    }

    /// P4 — record a read receipt + emit `MessageRead`.
    pub async fn mark_message_read(
        &self,
        message_id: crate::ids::MessageId,
        by_participant: ParticipantId,
    ) -> Result<()> {
        self.config
            .message_store
            .mark_read(&message_id, &by_participant)
            .await?;
        self.emit(Event::MessageRead {
            message_id,
            at: Utc::now(),
        });
        Ok(())
    }

    /// P9 — record a per-tenant usage unit. Emits `UsageRecord` on
    /// the bus so downstream billing pipelines can aggregate.
    pub fn record_usage(&self, tenant_id: TenantId, kind: crate::events::UsageKind, units: u64) {
        self.emit(Event::UsageRecord {
            tenant_id,
            kind,
            units,
            at: Utc::now(),
        });
    }

    /// P9 — registrar adapters call this once they observe a
    /// registration refresh.
    pub fn notify_registration_heartbeat(&self, aor: impl Into<String>) {
        self.emit(Event::RegistrationHeartbeat {
            aor: aor.into(),
            at: Utc::now(),
        });
    }

    /// P9 — registrar adapters call this when registration state
    /// changes (registered / expired / unregistered / contact-changed).
    pub fn notify_registration_changed(&self, aor: impl Into<String>) {
        self.emit(Event::RegistrationChanged {
            aor: aor.into(),
            at: Utc::now(),
        });
    }

    /// P8 — emit an `ActiveSpeakerChanged` advisory. Called by the
    /// UCTP adapter when audio-level extension data shows a new
    /// dominant speaker. The Orchestrator just forwards on the bus;
    /// there's no routing-side change because the multi-party fanout
    /// is publisher-driven (subscribers always receive their
    /// subscribed publishers regardless of who's loudest).
    pub fn notify_active_speaker(
        &self,
        session_id: SessionId,
        connection_id: ConnectionId,
        audio_level_dbov: i8,
    ) {
        self.emit(Event::ActiveSpeakerChanged {
            session_id,
            connection_id,
            audio_level_dbov,
            at: Utc::now(),
        });
    }

    // --- P7 step-up auth ------------------------------------------------

    /// Request a step-up to a higher IdentityAssurance level on an
    /// existing Connection. P12.6 wires the full round-trip:
    ///
    /// 1. Dispatches an `identity.step-up-request` envelope through the
    ///    Connection's adapter (`ConnectionAdapter::send_step_up_request`).
    ///    UCTP-family adapters serialize the envelope per
    ///    CONVERSATION_PROTOCOL.md §5.8; non-UCTP adapters
    ///    (SIP / WebRTC) return `NotImplemented`.
    /// 2. Emits [`Event::IdentityStepUpRequested`] so the consumer
    ///    sees the request reached the wire.
    /// 3. When the peer's `identity.step-up-response` arrives, the
    ///    adapter forwards it as `AdapterEvent::StepUpResponse`; the
    ///    orchestrator re-emits it as
    ///    [`Event::IdentityStepUpResponseReceived`]. The consumer
    ///    resolves the `(method, credential)` pair to a
    ///    [`crate::identity::Credential`] and calls
    ///    [`Self::complete_step_up`] to finalize the assurance change.
    pub async fn request_step_up(
        &self,
        connection_id: ConnectionId,
        required: crate::capability::IdentityAssuranceRequirement,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter
            .send_step_up_request(connection_id.clone(), required.clone(), Vec::new(), None)
            .await?;
        self.emit(Event::IdentityStepUpRequested {
            connection_id,
            required,
            at: Utc::now(),
        });
        Ok(())
    }

    /// P7 — accept a step-up credential and emit
    /// `IdentityAssuranceChanged`.
    pub async fn complete_step_up(
        &self,
        connection_id: ConnectionId,
        credential: crate::identity::Credential,
        provider: Arc<dyn crate::identity::IdentityProvider>,
    ) -> Result<crate::identity::IdentityAssurance> {
        let (identity_id, assurance) = provider.authenticate(credential).await?;
        self.emit(Event::IdentityAssuranceChanged {
            connection_id,
            identity_id: Some(identity_id),
            at: Utc::now(),
        });
        Ok(assurance)
    }

    // --- P5 provider registration ---------------------------------------

    pub fn register_asr_provider(
        &self,
        name: impl Into<String>,
        provider: Arc<dyn crate::harness::AsrProvider>,
    ) {
        self.asr_providers.insert(name.into(), provider);
    }
    pub fn register_tts_provider(
        &self,
        name: impl Into<String>,
        provider: Arc<dyn crate::harness::TtsProvider>,
    ) {
        self.tts_providers.insert(name.into(), provider);
    }
    pub fn register_dialog_manager(
        &self,
        name: impl Into<String>,
        manager: Arc<dyn crate::harness::DialogManager>,
    ) {
        self.dialog_managers.insert(name.into(), manager);
    }
    pub fn register_recording_sink(
        &self,
        name: impl Into<String>,
        sink: Arc<dyn crate::harness::RecordingSink>,
    ) {
        self.recording_sinks.insert(name.into(), sink);
    }

    // --- P5 recording / transcription -----------------------------------

    /// P5 — start recording the audio MediaStream of a Connection (or
    /// of every Connection in a Session) into a registered
    /// RecordingSink. Returns the `RecordingId` for stop/pause/resume.
    pub async fn start_recording(
        self: &Arc<Self>,
        target: crate::commands::RecordingTarget,
        sink_name: impl Into<String>,
    ) -> Result<crate::ids::RecordingId> {
        use crate::commands::RecordingTarget;
        let sink_name = sink_name.into();
        let sink = self
            .recording_sinks
            .get(&sink_name)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::AdmissionRejected("recording sink not registered"))?;

        // Resolve target → list of Connections to tap.
        let (conns, tenant_id) = match target {
            RecordingTarget::Connection(c) => {
                let tid = self
                    .session_of(&c)
                    .and_then(|sid| {
                        self.sessions.get(&sid).map(|e| {
                            self.conversations
                                .get(
                                    &e.value()
                                        .read()
                                        .expect("sess lock poisoned")
                                        .conversation_id,
                                )
                                .map(|c| {
                                    c.value()
                                        .read()
                                        .expect("conv lock poisoned")
                                        .tenant_id
                                        .clone()
                                })
                        })
                    })
                    .flatten();
                (vec![c], tid)
            }
            RecordingTarget::Session(sid) => {
                let (cs, tid) = self
                    .sessions
                    .get(&sid)
                    .map(|e| {
                        let s = e.value().read().expect("sess lock poisoned");
                        let conns = s.connections.keys().cloned().collect::<Vec<_>>();
                        let tid = self.conversations.get(&s.conversation_id).map(|c| {
                            c.value()
                                .read()
                                .expect("conv lock poisoned")
                                .tenant_id
                                .clone()
                        });
                        (conns, tid)
                    })
                    .ok_or_else(|| RvoipError::SessionNotFound(sid))?;
                (cs, tid)
            }
        };
        if conns.is_empty() {
            return Err(RvoipError::AdmissionRejected(
                "recording target has no Connections",
            ));
        }
        for connection_id in &conns {
            let _ = self.adapter_for(connection_id)?;
        }
        let lifecycle_tickets = self.capture_connection_lifecycles(&conns)?;

        // V2.B — per-tenant Semaphore admission. When the tenant has
        // a `max_concurrent_recordings` quota, the semaphore was
        // provisioned in `set_tenant_quotas`. `try_acquire_owned`
        // returns the permit directly (no shard contention); the
        // permit is stored in `RecordingHandle._permit` and released
        // by Drop when the handle is removed.
        let permit = if let Some(ref tid) = tenant_id {
            self.recording_sems
                .get(tid)
                .map(|s| Arc::clone(s.value()))
                .and_then(|sem| match sem.try_acquire_owned() {
                    Ok(p) => Some(Ok(p)),
                    Err(_) => Some(Err(RvoipError::AdmissionRejected(
                        "tenant max_concurrent_recordings exceeded",
                    ))),
                })
                .transpose()?
        } else {
            None
        };

        let rid = crate::ids::RecordingId::new();
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let connection_ids = conns.clone();
        let mut media = MediaTapHandle::default();
        for connection_id in conns {
            let (route, mut receiver) = match self.media_tap_for_connection(connection_id, 64).await
            {
                Ok(tap) => tap,
                // Preserve the pre-graph API contract: recording admission
                // can reserve a quota slot before a transport publishes its
                // first audio stream. Once a stream exists, callers can stop
                // and restart the recording to attach it.
                Err(RvoipError::AdmissionRejected("no audio stream")) => continue,
                Err(error) => return Err(error),
            };
            let sink_for_task = Arc::clone(&sink);
            let paused_for_task = Arc::clone(&paused);
            let task = tokio::spawn(async move {
                while let Some(frame) = receiver.recv().await {
                    if paused_for_task.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    if sink_for_task.write(frame).await.is_err() {
                        break;
                    }
                }
            });
            media.push(route, task.abort_handle());
        }

        let statuses = media.statuses();
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            media.stop_and_wait().await;
            let _ = sink.close().await;
            return Err(error);
        }
        let lifecycle_guards = self.lock_connection_lifecycles(&lifecycle_tickets)?;
        // V2.B — the permit (if any) is stored in the handle and drops
        // alongside it on stop_recording or terminal route cleanup.
        let _ = tenant_id;
        self.recordings.insert(
            rid.clone(),
            RecordingHandle {
                sink: Arc::clone(&sink),
                media,
                connection_ids: connection_ids.clone(),
                paused: Arc::clone(&paused),
                _permit: permit,
            },
        );
        self.emit(Event::RecordingStarted {
            recording_id: rid.clone(),
            at: Utc::now(),
        });
        drop(lifecycle_guards);
        self.supervise_recording_routes(rid.clone(), statuses);
        Ok(rid)
    }

    pub async fn stop_recording(
        &self,
        recording_id: crate::ids::RecordingId,
    ) -> Result<crate::harness::RecordingArtifact> {
        let (_, mut handle) = self
            .recordings
            .remove(&recording_id)
            .ok_or_else(|| RvoipError::AdmissionRejected("recording not found"))?;
        drop(handle._permit.take());
        handle.media.stop_and_wait().await;
        // V2.B — permit drops with the handle struct, releasing the
        // tenant's admission slot.
        let artifact = handle.sink.close().await?;
        self.emit(Event::RecordingStopped {
            recording_id: recording_id.clone(),
            at: Utc::now(),
        });
        self.emit(Event::RecordingComplete {
            recording_id,
            sink: artifact.url.clone(),
            vcon_ref: None,
            at: Utc::now(),
        });
        Ok(artifact)
    }

    /// P5 — set the pause flag on the recording's pump task. Frames
    /// arriving while the flag is set are dropped silently (the sink
    /// doesn't see them). `resume_recording` clears the flag.
    ///
    /// Concurrency note: the pause flag is `Relaxed`-ordered and
    /// checked per-frame in each per-stream pump task. Frames that are
    /// already in the per-stream mpsc buffer at the moment `pause` is
    /// called may still be drained and written before subsequent
    /// per-frame checks observe the flag — pause means "drop new
    /// frames", not "abandon frames already accepted". For strict
    /// drain-on-pause semantics, follow `pause_recording` with
    /// `stop_recording` (no resume) instead.
    pub async fn pause_recording(&self, id: crate::ids::RecordingId) -> Result<()> {
        let entry = self
            .recordings
            .get(&id)
            .ok_or_else(|| RvoipError::AdmissionRejected("recording not found"))?;
        entry
            .value()
            .paused
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    pub async fn resume_recording(&self, id: crate::ids::RecordingId) -> Result<()> {
        let entry = self
            .recordings
            .get(&id)
            .ok_or_else(|| RvoipError::AdmissionRejected("recording not found"))?;
        entry
            .value()
            .paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// P5 — start transcription. Pulls audio frames into the named
    /// AsrProvider; emits `TranscriptTurn` for each final result.
    pub async fn start_transcription(
        self: &Arc<Self>,
        target: crate::commands::RecordingTarget,
        provider_ref: impl Into<String>,
    ) -> Result<crate::ids::TranscriptionId> {
        use crate::commands::RecordingTarget;
        let provider_name = provider_ref.into();
        let provider = self
            .asr_providers
            .get(&provider_name)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| RvoipError::AdmissionRejected("ASR provider not registered"))?;
        let conn = match target {
            RecordingTarget::Connection(c) => c,
            RecordingTarget::Session(sid) => self
                .sessions
                .get(&sid)
                .and_then(|e| {
                    e.value()
                        .read()
                        .expect("sess lock poisoned")
                        .connections
                        .keys()
                        .next()
                        .cloned()
                })
                .ok_or_else(|| RvoipError::SessionNotFound(sid))?,
        };
        let _ = self.adapter_for(&conn)?;
        let lifecycle_tickets = self.capture_connection_lifecycles(std::slice::from_ref(&conn))?;

        let tid = crate::ids::TranscriptionId::new();
        let stream: Arc<dyn crate::harness::AsrStream> = Arc::from(
            provider
                .open_stream(conn.clone(), crate::harness::AsrConfig::default())
                .await?,
        );
        let (route, mut receiver) = self.media_tap_for_connection(conn.clone(), 64).await?;
        let me = Arc::clone(self);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    frame = receiver.recv() => {
                        let Some(frame) = frame else { break; };
                        if stream.push(frame).await.is_err() {
                            break;
                        }
                    }
                    result = stream.next() => {
                        let Some(result) = result else { break; };
                        me.emit(Event::TranscriptTurn {
                            stream_id: result.stream_id,
                            speaker: result.speaker,
                            text: result.text,
                            confidence: result.confidence,
                            is_final: result.is_final,
                            assigned_provider: Some(provider_name.clone()),
                            at: Utc::now(),
                        });
                    }
                }
            }
            let _ = stream.close().await;
        });
        let mut media = MediaTapHandle::default();
        media.push(route, task.abort_handle());
        let statuses = media.statuses();
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            media.stop_and_wait().await;
            return Err(error);
        }
        let lifecycle_guards = self.lock_connection_lifecycles(&lifecycle_tickets)?;
        self.transcriptions.insert(
            tid.clone(),
            TranscriptionHandle {
                media,
                connection_id: conn.clone(),
            },
        );
        drop(lifecycle_guards);
        self.supervise_transcription_routes(tid.clone(), statuses);
        Ok(tid)
    }

    pub async fn stop_transcription(&self, id: crate::ids::TranscriptionId) -> Result<()> {
        if let Some((_, mut handle)) = self.transcriptions.remove(&id) {
            handle.media.stop_and_wait().await;
            Ok(())
        } else {
            Err(RvoipError::AdmissionRejected("transcription not found"))
        }
    }

    // --- P5 AI harness --------------------------------------------------

    /// P5 — attach an AI runtime to a Connection. Uses registered
    /// AsrProvider + DialogManager + TtsProvider names looked up from
    /// `config`. Returns the AiAttachmentId for detach.
    ///
    /// `config["asr"]` / `config["tts"]` / `config["dialog"]` keys
    /// must point to registered providers.
    ///
    /// P5 barge-in: when ASR yields a partial / final result while a
    /// TTS playback is in flight, the orchestrator cancels the
    /// playback and emits `Event::BargeInDetected` before continuing
    /// the dialog loop.
    #[instrument(skip(self, provider_ref, config), fields(connection_id = %connection_id))]
    pub async fn attach_ai(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        provider_ref: impl Into<String>,
        config: std::collections::HashMap<String, String>,
    ) -> Result<crate::ids::AiAttachmentId> {
        let _ = self.adapter_for(&connection_id)?;
        let lifecycle_tickets =
            self.capture_connection_lifecycles(std::slice::from_ref(&connection_id))?;
        // P6 — tenant attribution + AI quota enforcement.
        let tenant_id = self
            .session_of(&connection_id)
            .and_then(|sid| {
                self.sessions.get(&sid).map(|e| {
                    self.conversations
                        .get(
                            &e.value()
                                .read()
                                .expect("sess lock poisoned")
                                .conversation_id,
                        )
                        .map(|c| {
                            c.value()
                                .read()
                                .expect("conv lock poisoned")
                                .tenant_id
                                .clone()
                        })
                })
            })
            .flatten();
        // V2.B — per-tenant Semaphore admission. Permit stored in the
        // AiAttachmentHandle and released by Drop on detach.
        let ai_permit = if let Some(ref tid) = tenant_id {
            self.ai_sems
                .get(tid)
                .map(|s| Arc::clone(s.value()))
                .and_then(|sem| match sem.try_acquire_owned() {
                    Ok(p) => Some(Ok(p)),
                    Err(_) => Some(Err(RvoipError::AdmissionRejected(
                        "tenant max_concurrent_ai_sessions exceeded",
                    ))),
                })
                .transpose()?
        } else {
            None
        };

        let provider_ref = provider_ref.into();
        let asr_name = config
            .get("asr")
            .cloned()
            .unwrap_or_else(|| provider_ref.clone());
        let tts_name = config
            .get("tts")
            .cloned()
            .unwrap_or_else(|| provider_ref.clone());
        let dialog_name = config
            .get("dialog")
            .cloned()
            .unwrap_or_else(|| provider_ref.clone());

        let asr = self
            .asr_providers
            .get(&asr_name)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| {
                RvoipError::AdmissionRejected("attach_ai: ASR provider not registered")
            })?;
        let tts = self
            .tts_providers
            .get(&tts_name)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| {
                RvoipError::AdmissionRejected("attach_ai: TTS provider not registered")
            })?;
        let dialog = self
            .dialog_managers
            .get(&dialog_name)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| {
                RvoipError::AdmissionRejected("attach_ai: DialogManager not registered")
            })?;

        let aid = crate::ids::AiAttachmentId::new();
        let speaking = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let speak_cancel: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let stream: Arc<dyn crate::harness::AsrStream> = Arc::from(
            asr.open_stream(connection_id.clone(), crate::harness::AsrConfig::default())
                .await?,
        );
        let (route, mut receiver) = self
            .media_tap_for_connection(connection_id.clone(), 64)
            .await?;

        let me = Arc::clone(self);
        let connection_id_for_task = connection_id.clone();
        let aid_for_task = aid.clone();
        let speaking_for_task = Arc::clone(&speaking);
        let speak_cancel_for_task = Arc::clone(&speak_cancel);
        let task = tokio::spawn(async move {
            let connection_id = connection_id_for_task;
            let stream_for_push = Arc::clone(&stream);
            let push_loop = async move {
                while let Some(frame) = receiver.recv().await {
                    if stream_for_push.push(frame).await.is_err() {
                        break;
                    }
                }
            };
            let dialog_loop = async {
                while let Some(asr_result) = stream.next().await {
                    // P5 barge-in: if user speech detected while we're
                    // speaking, cancel current playback + fire event.
                    if speaking_for_task.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Some(tx) = speak_cancel_for_task.lock().await.take() {
                            let _ = tx.send(());
                        }
                        speaking_for_task.store(false, std::sync::atomic::Ordering::Relaxed);
                        me.emit(Event::BargeInDetected {
                            connection_id: connection_id.clone(),
                            ai_attachment_id: aid_for_task.clone(),
                            at: Utc::now(),
                        });
                    }
                    if !asr_result.is_final {
                        continue;
                    }
                    let action = match dialog.turn(&asr_result).await {
                        Ok(a) => a,
                        Err(_) => break,
                    };
                    match action {
                        crate::harness::DialogAction::Listen => continue,
                        crate::harness::DialogAction::End => break,
                        crate::harness::DialogAction::Say { text, voice } => {
                            let playback = match tts
                                .synthesize(crate::harness::TtsRequest {
                                    voice,
                                    text,
                                    sample_rate_hz: None,
                                })
                                .await
                            {
                                Ok(p) => p,
                                Err(_) => continue,
                            };
                            let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
                            *speak_cancel_for_task.lock().await = Some(cancel_tx);
                            speaking_for_task.store(true, std::sync::atomic::Ordering::Relaxed);

                            if let Ok(adapter) = me.adapter_for(&connection_id) {
                                if let Ok(streams) = adapter.streams(connection_id.clone()).await {
                                    let out =
                                        streams.into_iter().find(|s| s.kind() == StreamKind::Audio);
                                    if let Some(audio) = out {
                                        let tx = audio.frames_out();
                                        loop {
                                            tokio::select! {
                                                _ = &mut cancel_rx => {
                                                    let _ = playback.cancel().await;
                                                    break;
                                                }
                                                frame_opt = playback.next_frame() => {
                                                    let Some(frame) = frame_opt else {
                                                        break;
                                                    };
                                                    let _ = tx.send(frame).await;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            speaking_for_task.store(false, std::sync::atomic::Ordering::Relaxed);
                            // Drain any stale cancel sender (defensive).
                            let _ = speak_cancel_for_task.lock().await.take();
                        }
                    }
                }
            };
            tokio::pin!(push_loop, dialog_loop);
            tokio::select! {
                _ = &mut push_loop => {}
                _ = &mut dialog_loop => {}
            }
            let _ = stream.close().await;
        });
        let mut media = MediaTapHandle::default();
        media.push(route, task.abort_handle());

        // V2.B — permit (if any) stored in the handle; releases on
        // Drop when detach removes the entry.
        let _ = tenant_id;
        let statuses = media.statuses();
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            media.stop_and_wait().await;
            return Err(error);
        }
        let lifecycle_guards = self.lock_connection_lifecycles(&lifecycle_tickets)?;
        self.ai_attachments.insert(
            aid.clone(),
            AiAttachmentHandle {
                media,
                connection_id: connection_id.clone(),
                speaking,
                speak_cancel,
                _permit: ai_permit,
            },
        );
        self.emit(Event::AiAttached {
            connection_id,
            attachment_id: aid.clone(),
            provider_ref,
            at: Utc::now(),
        });
        drop(lifecycle_guards);
        self.supervise_ai_routes(aid.clone(), statuses);
        Ok(aid)
    }

    /// P5 — attach a listener tap. Spawns a per-Connection task that
    /// forwards inbound audio frames to the chosen sink. Separated-
    /// streams default: each Connection's audio lands as its own
    /// stream into the sink (no mixing). The `ListenerSink::Channel`
    /// variant is consumed via [`Self::listener_channel`] which
    /// returns the receive end the consumer can pull from.
    pub fn attach_listener(
        self: &Arc<Self>,
        target: crate::commands::ListenerTarget,
        sink: crate::commands::ListenerSink,
    ) -> Result<crate::ids::ListenerId> {
        use crate::commands::{ListenerSink, ListenerTarget};
        let conns: Vec<ConnectionId> = match target {
            ListenerTarget::Connection(c) => vec![c],
            ListenerTarget::Session(sid) => self
                .sessions
                .get(&sid)
                .map(|e| {
                    e.value()
                        .read()
                        .expect("sess lock poisoned")
                        .connections
                        .keys()
                        .cloned()
                        .collect()
                })
                .ok_or_else(|| RvoipError::SessionNotFound(sid))?,
        };
        if conns.is_empty() {
            return Err(RvoipError::AdmissionRejected(
                "listener target has no Connections",
            ));
        }
        for connection_id in &conns {
            let _ = self.adapter_for(connection_id)?;
        }
        let lifecycle_tickets = self.capture_connection_lifecycles(&conns)?;

        let lid = crate::ids::ListenerId::new();
        let me = Arc::clone(self);

        // Build the per-sink frame consumer. Channel sinks expose a
        // receiver via `listener_channels`; File/Url sinks just log
        // the byte count (full file/HTTP implementations live in
        // consumer crates).
        let (tx_for_channel, rx_for_channel) = match sink {
            ListenerSink::Channel => {
                let (t, r) = tokio::sync::mpsc::channel::<crate::stream::MediaFrame>(256);
                (Some(t), Some(r))
            }
            _ => (None, None),
        };
        if let Some(rx) = rx_for_channel {
            self.listener_channels
                .insert(lid.clone(), Mutex::new(Some(rx)));
        }
        let lid_for_task = lid.clone();
        let connection_ids = conns.clone();
        let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let mut tasks = tokio::task::JoinSet::new();
            for cid in conns {
                let (route, mut receiver) = match me.media_tap_for_connection(cid, 64).await {
                    Ok(tap) => tap,
                    Err(_) => continue,
                };
                if let Some(status) = route.status() {
                    me.supervise_listener_route(lid_for_task.clone(), status);
                }
                let tx_clone = tx_for_channel.clone();
                let lid_clone = lid_for_task.clone();
                tasks.spawn(async move {
                    // Holding the route in this task couples graph membership
                    // to the task lifetime. JoinSet aborts children on drop.
                    let _route = route;
                    while let Some(frame) = receiver.recv().await {
                        if let Some(tx) = &tx_clone {
                            if tx.send(frame).await.is_err() {
                                break;
                            }
                        } else {
                            // File/URL — drop after counting. Full persistence
                            // is supplied by consumer crates.
                            let _ = (frame, &lid_clone);
                        }
                    }
                });
            }
            while tasks.join_next().await.is_some() {
                // Keep remaining source taps alive until each source closes or
                // detach aborts this parent (which drops and aborts JoinSet).
            }
            me.remove_listener_owner(&lid_for_task);
        });
        let lifecycle_guards = match self.lock_connection_lifecycles(&lifecycle_tickets) {
            Ok(guards) => guards,
            Err(error) => {
                task.abort();
                self.listener_channels.remove(&lid);
                return Err(error);
            }
        };
        self.listener_tasks.insert(
            lid.clone(),
            MediaTaskHandle {
                abort: task.abort_handle(),
                connection_ids: connection_ids.clone(),
            },
        );
        self.emit(Event::ListenerAttached {
            listener_id: lid.clone(),
            at: Utc::now(),
        });
        drop(lifecycle_guards);
        let _ = start_tx.send(());
        Ok(lid)
    }

    /// P5 — take the receiver for a `Channel` listener.
    /// Single-take per listener; subsequent calls return `None`.
    pub fn listener_channel(
        &self,
        id: &crate::ids::ListenerId,
    ) -> Option<tokio::sync::mpsc::Receiver<crate::stream::MediaFrame>> {
        self.listener_channels
            .get(id)
            .and_then(|e| e.value().lock().expect("listener lock poisoned").take())
    }

    pub async fn detach(&self, attachment: crate::commands::AttachmentRef) -> Result<()> {
        use crate::commands::AttachmentRef;
        match attachment {
            AttachmentRef::Ai(id) => {
                if let Some((_, mut handle)) = self.ai_attachments.remove(&id) {
                    let routes = handle.media.begin_stop();
                    // Release admission before waiting on graph acknowledgement.
                    drop(handle._permit.take());
                    for route in routes {
                        let _ = route.remove().await;
                    }
                    self.emit(Event::AiDetached {
                        attachment_id: id,
                        at: Utc::now(),
                    });
                    Ok(())
                } else {
                    Err(RvoipError::AdmissionRejected("ai attachment not found"))
                }
            }
            AttachmentRef::Listener(id) => {
                self.remove_listener_owner(&id);
                Ok(())
            }
            AttachmentRef::Recording(id) => self.stop_recording(id).await.map(|_| ()),
        }
    }

    /// P4 — enforce inline body cap. >64KB must use attachments[].
    fn validate_inline_body(message: &Message) -> Result<()> {
        const MAX_INLINE_BODY: usize = 64 * 1024;
        if message.body.len() > MAX_INLINE_BODY && message.attachments.is_empty() {
            return Err(RvoipError::AdmissionRejected(
                "message body exceeds 64KB inline cap; use attachments[] with an OOB URL",
            ));
        }
        Ok(())
    }

    pub async fn renegotiate_media(
        &self,
        connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<crate::capability::NegotiatedCodecs> {
        let adapter = self.adapter_for(&connection_id)?;
        let negotiated = adapter
            .renegotiate_media(connection_id.clone(), capabilities)
            .await?;

        // Gap plan §4.2 v1 punch list — if the connection is in a
        // cross-transport bridge, hot-swap its transcoders so the
        // pump's `from_pt`/`to_pt` reflect the post-renegotiation
        // codec on this leg. The other leg's codec is unchanged
        // (renegotiate_media is per-connection); the swap only
        // touches the direction whose PT actually moved.
        if let Some(peer) = self.bridge_peer_of(&connection_id) {
            if let Some(audio) = negotiated.audio.as_ref() {
                if let Some(new_pt) = codec_to_pt(&audio.name) {
                    // A2 — snapshot the bridge handle's relevant state
                    // (orientation + swap channel availability) WITHOUT
                    // holding the DashMap iterator guard across any
                    // .await. Extract bridge_id first, then re-fetch
                    // by id inside a tight non-async scope.
                    let bridge_id_opt: Option<BridgeId> = {
                        self.cross_bridges
                            .iter()
                            .find(|e| e.value().a == connection_id || e.value().b == connection_id)
                            .map(|e| e.key().clone())
                    };
                    if let Some(bridge_id) = bridge_id_opt {
                        // Snapshot orientation (no .await held).
                        let orientation_this_is_a = self
                            .cross_bridges
                            .get(&bridge_id)
                            .map(|e| e.value().a == connection_id);
                        let Some(orientation_this_is_a) = orientation_this_is_a else {
                            return Ok(negotiated);
                        };

                        // A2 — direct .await for the peer's stream
                        // lookup (was `block_in_place + block_on`).
                        let peer_pt = if let Ok(adp) = self.adapter_for(&peer) {
                            adp.streams(peer.clone())
                                .await
                                .ok()
                                .and_then(|streams| {
                                    streams
                                        .into_iter()
                                        .find(|s| s.kind() == StreamKind::Audio)
                                        .map(|s| s.codec().name)
                                })
                                .and_then(|n| codec_to_pt(&n))
                                .unwrap_or(new_pt)
                        } else {
                            new_pt
                        };

                        // Build per-direction swap messages.
                        let (a_swap, b_swap) = if orientation_this_is_a {
                            // a is "this" connection (new_pt), b is peer (peer_pt).
                            (make_swap(new_pt, peer_pt), make_swap(peer_pt, new_pt))
                        } else {
                            (make_swap(peer_pt, new_pt), make_swap(new_pt, peer_pt))
                        };
                        // Snapshot only cloneable swap state while the map
                        // entry is guarded. Channel backpressure, pump acks,
                        // and graph updates all happen after the guard drops.
                        let swap_controller = self
                            .cross_bridges
                            .get(&bridge_id)
                            .map(|entry| entry.value().swap_controller());
                        let swap_result = match swap_controller {
                            Some(Ok(controller)) => {
                                controller.swap_transcoders(a_swap, b_swap).await
                            }
                            Some(Err(error)) => Err(error),
                            None => Ok(()),
                        };
                        if let Err(e) = swap_result {
                            warn!(
                                ?connection_id,
                                error = %e,
                                "orchestrator: bridge transcoder hot-swap failed; bridge may carry stale codecs"
                            );
                        } else {
                            metrics::counter!(
                                "uctp_renegotiations_completed_total",
                                "outcome" => "hot-swapped",
                            )
                            .increment(1);
                        }
                    }
                }
            }
        }

        Ok(negotiated)
    }

    /// P2 — mute one direction (Send / Receive / Both) on a Connection.
    /// Dispatches through the registered adapter; adapters that don't
    /// implement mute return `RvoipError::NotImplemented`.
    pub async fn mute(&self, connection_id: ConnectionId, direction: MuteDirection) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.mute(connection_id, direction).await
    }

    pub async fn unmute(
        &self,
        connection_id: ConnectionId,
        direction: MuteDirection,
    ) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.unmute(connection_id, direction).await
    }

    /// P2 — start playback of `source` toward the peer on
    /// `connection_id`. The returned [`PlaybackHandle`] cancels
    /// playback on `.cancel()`.
    pub async fn play_audio(
        &self,
        connection_id: ConnectionId,
        source: AudioSource,
    ) -> Result<PlaybackHandle> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.play_audio(connection_id, source).await
    }

    /// Bridge two connections — wires a bidirectional frame pump between
    /// their audio streams, inserting a transcoder when the negotiated
    /// codecs differ. Per INTERFACE_DESIGN.md §10.2.
    ///
    /// Adapters populate audio streams lazily (typically on
    /// `connection.ready`), so a caller that calls
    /// `bridge_connections` immediately from `Event::ConnectionInbound`
    /// may race the stream registration. This method polls for both
    /// streams up to [`Config::bridge_stream_deadline`] before failing
    /// with `AdmissionRejected("no audio stream")`. Set the deadline to
    /// zero in `Config` for strict no-wait behavior.
    ///
    /// Errors:
    /// - `AdmissionRejected` if `a == b` or either is already bridged.
    /// - `ConnectionNotFound` if either connection is unknown.
    /// - `NoAdapterForTransport` if either connection's transport has no adapter.
    /// - `AdmissionRejected("no audio stream")` if either side still has no
    ///   audio `MediaStream` after the deadline.
    /// - `UnsupportedCodec(name)` if a negotiated codec has no PT mapping.
    #[instrument(skip(self), fields(a = %a, b = %b, bridge_id))]
    pub async fn bridge_connections(&self, a: ConnectionId, b: ConnectionId) -> Result<BridgeId> {
        if a == b {
            return Err(RvoipError::AdmissionRejected(
                "cannot bridge a connection to itself",
            ));
        }
        let a_adapter = self.adapter_for(&a)?;
        let b_adapter = self.adapter_for(&b)?;
        let lifecycle_tickets = self.capture_connection_lifecycles(&[a.clone(), b.clone()])?;
        let id = BridgeId::new();
        let reservation = self.reserve_cross_bridge(id.clone(), a.clone(), b.clone())?;

        // Poll both adapters for an audio stream up to the configured
        // deadline. Adapters create streams on connection.ready, so a
        // bridge requested from Event::ConnectionInbound usually has to
        // wait a handful of ms. 50ms polling interval is small enough
        // to be inaudible at the call setup level and large enough not
        // to spin.
        let deadline = self.config.bridge_stream_deadline;
        let poll_interval = std::time::Duration::from_millis(50);
        let start = std::time::Instant::now();
        let (a_audio, b_audio) = loop {
            let a_streams = a_adapter.streams(a.clone()).await?;
            let b_streams = b_adapter.streams(b.clone()).await?;
            let a_audio = a_streams
                .into_iter()
                .find(|s| s.kind() == StreamKind::Audio);
            let b_audio = b_streams
                .into_iter()
                .find(|s| s.kind() == StreamKind::Audio);
            match (a_audio, b_audio) {
                (Some(a_s), Some(b_s)) => break (a_s, b_s),
                _ if start.elapsed() >= deadline => {
                    return Err(RvoipError::AdmissionRejected(
                        "no audio stream on one or both connections within deadline",
                    ));
                }
                _ => {
                    tokio::time::sleep(poll_interval).await;
                }
            }
        };

        // Validate codecs before either graph consumes a single-take source.
        codec_to_pt(&a_audio.codec().name)
            .ok_or_else(|| RvoipError::UnsupportedCodec(a_audio.codec().name.clone()))?;
        codec_to_pt(&b_audio.codec().name)
            .ok_or_else(|| RvoipError::UnsupportedCodec(b_audio.codec().name.clone()))?;

        let a_graph = self
            .media_graph_for_stream(a.clone(), Arc::clone(&a_audio))
            .await?;
        let b_graph = self
            .media_graph_for_stream(b.clone(), Arc::clone(&b_audio))
            .await?;
        let a_to_b = a_graph.add_managed_sink(b_audio.codec(), b_audio.frames_out())?;
        if a_to_b.wait_active().await.is_err() {
            let _ = a_to_b.remove().await;
            return Err(RvoipError::InvalidState(
                "first bridge route terminated during setup",
            ));
        }
        let b_to_a = match b_graph.add_managed_sink(a_audio.codec(), a_audio.frames_out()) {
            Ok(route) => route,
            Err(error) => {
                let _ = a_to_b.remove().await;
                return Err(error);
            }
        };
        if b_to_a.wait_active().await.is_err() {
            let (a_result, b_result) = tokio::join!(a_to_b.remove(), b_to_a.remove());
            let _ = (a_result, b_result);
            return Err(RvoipError::InvalidState(
                "second bridge route terminated during setup",
            ));
        }

        let mut handle = CrossBridgeHandle::with_managed_media_graphs(
            id.clone(),
            a.clone(),
            b.clone(),
            a_graph,
            b_graph,
            a_to_b,
            b_to_a,
        );
        let statuses = handle
            .media_route_statuses()
            .expect("media-graph bridge exposes route statuses");
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            let _ = handle.stop().await;
            return Err(error);
        }
        let lifecycle_guards = self.lock_connection_lifecycles(&lifecycle_tickets)?;
        self.cross_bridges.insert(id.clone(), handle);
        reservation.commit();
        self.emit(Event::ConnectionsBridged {
            bridge_id: id.clone(),
            a,
            b,
            at: Utc::now(),
        });
        drop(lifecycle_guards);
        self.supervise_cross_bridge_routes(id.clone(), statuses);
        Ok(id)
    }

    /// Return the reusable media graph for a Connection, creating it from the
    /// Connection's audio stream on first use. Broadcast adapters call this
    /// method to attach a sink without stealing frames from an active bridge.
    pub async fn media_graph_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<MediaGraphHandle> {
        let adapter = self.adapter_for(&connection_id)?;
        if let Some(graph) = self.media_graphs.get(&connection_id) {
            return Ok(graph.value().clone());
        }
        let stream = adapter
            .streams(connection_id.clone())
            .await?
            .into_iter()
            .find(|stream| stream.kind() == StreamKind::Audio)
            .ok_or(RvoipError::AdmissionRejected("no audio stream"))?;
        self.media_graph_for_stream(connection_id, stream).await
    }

    async fn media_graph_for_stream(
        &self,
        connection_id: ConnectionId,
        stream: Arc<dyn crate::stream::MediaStream>,
    ) -> Result<MediaGraphHandle> {
        if let Some(graph) = self.media_graphs.get(&connection_id) {
            return Ok(graph.value().clone());
        }
        // Serialize first-use only for this Connection so concurrent
        // bridge/broadcast requests cannot both take its single receiver,
        // without coupling independent calls to one global mutex.
        let init_lock = self
            .media_graph_inits
            .entry(connection_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = init_lock.lock().await;
        if let Some(graph) = self.media_graphs.get(&connection_id) {
            return Ok(graph.value().clone());
        }
        let codec = stream.codec();
        // Validation must precede the destructive single-consumer take. Once
        // acquired, a receiver cannot be put back into a MediaStream.
        validate_media_graph_codec(&codec)?;
        let source = stream.try_frames_in()?;
        let graph = start_media_graph(source, codec, MediaGraphPolicy::default())?;
        self.media_graphs
            .insert(connection_id.clone(), graph.clone());
        // The adapter stream lookup above is asynchronous. A disconnect may
        // have removed the Connection while it was in flight; insert first,
        // then revalidate so either this path or `forget_connection` observes
        // and shuts down the graph in every interleaving.
        if !self.connections.contains_key(&connection_id) {
            if let Some((_, stale)) = self.media_graphs.remove(&connection_id) {
                stale.shutdown();
            }
            return Err(RvoipError::ConnectionNotFound(connection_id));
        }
        Ok(graph)
    }

    /// Attach a bounded observer channel to a Connection's reusable source
    /// graph. The returned route owns its graph membership and removes itself
    /// on drop, which makes attachment cancellation leak-free.
    async fn media_tap_for_connection(
        &self,
        connection_id: ConnectionId,
        channel_capacity: usize,
    ) -> Result<(
        MediaTapRoute,
        tokio::sync::mpsc::Receiver<crate::stream::MediaFrame>,
    )> {
        let graph = self.media_graph_for_connection(connection_id).await?;
        let source_codec = graph.latest_snapshot().source_codec;
        let (target, receiver) = tokio::sync::mpsc::channel(channel_capacity.max(1));
        let route = graph.add_managed_sink(source_codec, target)?;
        route
            .wait_active()
            .await
            .map_err(|_| RvoipError::InvalidState("media graph route terminated during setup"))?;
        Ok((MediaTapRoute::new(route), receiver))
    }

    /// Attach an arbitrary destination to a Connection's source graph.
    pub async fn attach_media_sink(
        &self,
        connection_id: ConnectionId,
        codec: crate::capability::CodecInfo,
        target: tokio::sync::mpsc::Sender<crate::stream::MediaFrame>,
    ) -> Result<crate::ids::MediaRouteId> {
        let _ = self.adapter_for(&connection_id)?;
        let lifecycle_tickets =
            self.capture_connection_lifecycles(std::slice::from_ref(&connection_id))?;
        let graph = self.media_graph_for_connection(connection_id).await?;
        let route_id = graph.add_sink(codec, target)?;
        if let Err(error) = await_media_route(&graph, &route_id).await {
            let _ = graph.remove_sink_and_wait(route_id).await;
            return Err(error);
        }
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            let _ = graph.remove_sink_and_wait(route_id).await;
            return Err(error);
        }
        let lifecycle_error = match self.lock_connection_lifecycles(&lifecycle_tickets) {
            Ok(guards) => {
                drop(guards);
                None
            }
            Err(error) => Some(error),
        };
        if let Some(error) = lifecycle_error {
            let _ = graph.remove_sink_and_wait(route_id).await;
            return Err(error);
        }
        Ok(route_id)
    }

    /// Publish a Connection's reusable audio source under a canonical
    /// Session/Stream identity for the existing subscription fanout path.
    ///
    /// The returned lease owns a bounded MediaGraph sink, the publisher
    /// registry generation, and its fanout task. It never acquires the source
    /// MediaStream receiver directly, so bridges, recorders, and other
    /// publishers can consume the same source graph concurrently.
    pub async fn register_virtual_publisher(
        self: &Arc<Self>,
        source_connection_id: ConnectionId,
        descriptor: crate::virtual_publisher::VirtualPublisherDescriptor,
    ) -> Result<crate::virtual_publisher::ManagedVirtualPublisher> {
        if descriptor.session_id.as_str().is_empty()
            || descriptor.stream_id.as_str().is_empty()
            || descriptor.participant.trim().is_empty()
        {
            return Err(RvoipError::InvalidState(
                "virtual publisher identity fields must be non-empty",
            ));
        }

        let _ = self.adapter_for(&source_connection_id)?;
        let lifecycle_tickets =
            self.capture_connection_lifecycles(std::slice::from_ref(&source_connection_id))?;
        let graph = self
            .media_graph_for_connection(source_connection_id.clone())
            .await?;
        let codec = graph.latest_snapshot().source_codec;
        let (target, frames) = tokio::sync::mpsc::channel(
            crate::virtual_publisher::DEFAULT_VIRTUAL_PUBLISHER_QUEUE_CAPACITY,
        );
        let route = graph.add_managed_sink(codec.clone(), target)?;
        if route.wait_active().await.is_err() {
            return Err(RvoipError::InvalidState(
                "virtual publisher media route terminated during setup",
            ));
        }
        if let Err(error) = self.validate_connection_lifecycles(&lifecycle_tickets) {
            let _ = route.remove().await;
            return Err(error);
        }

        let lifecycle_guards = match self.lock_connection_lifecycles(&lifecycle_tickets) {
            Ok(guards) => guards,
            Err(error) => {
                let _ = route.remove().await;
                return Err(error);
            }
        };
        let registry = self.publisher_registry();
        let registration = registry.register_managed(
            descriptor.session_id.clone(),
            descriptor.stream_id.to_string(),
            crate::subscriptions::PublisherEntry {
                connection: source_connection_id.clone(),
                participant: descriptor.participant.clone(),
                kind: "audio".to_string(),
                codec: Some(codec),
            },
        );
        let registration_id = match registration {
            Ok(registration_id) => registration_id,
            Err(_) => {
                drop(lifecycle_guards);
                let _ = route.remove().await;
                return Err(RvoipError::AdmissionRejected(
                    "virtual publisher stream is already registered",
                ));
            }
        };
        let publisher = crate::virtual_publisher::ManagedVirtualPublisher::start(
            Arc::downgrade(self),
            source_connection_id,
            descriptor,
            route,
            frames,
            registry,
            registration_id,
        );
        drop(lifecycle_guards);
        Ok(publisher)
    }

    pub fn detach_media_sink(
        &self,
        connection_id: &ConnectionId,
        route_id: crate::ids::MediaRouteId,
    ) -> bool {
        self.media_graphs
            .get(connection_id)
            .is_some_and(|graph| graph.remove_sink(route_id))
    }

    pub async fn unbridge_connections(&self, bridge_id: BridgeId) -> Result<()> {
        // Cross-transport bridges first. Do not publish success until both
        // media directions have acknowledged removal or pump termination.
        if self.remove_cross_bridge_internal(&bridge_id).await? {
            self.emit(Event::ConnectionsUnbridged {
                bridge_id,
                at: Utc::now(),
            });
            return Ok(());
        }
        // SIP-fast-path BridgeManager.
        match self.bridges.remove(&bridge_id) {
            Some(_handle) => {
                // Drop tears down the bridge synchronously.
                self.emit(Event::ConnectionsUnbridged {
                    bridge_id,
                    at: Utc::now(),
                });
                Ok(())
            }
            None => Err(RvoipError::BridgeNotFound(bridge_id)),
        }
    }
}

// Allow forwarding the `RejectReason` argument from older call sites that
// already had it imported. Re-exported for consumer convenience.
pub use crate::adapter::RejectReason as InboundRejectReason;

/// P6 — tenant-id lookup keyed on the freshly-inserted Conversation.
/// Cheap: one DashMap get + one RwLock read.
fn tenant_id_for_index(
    conversations: &Arc<DashMap<ConversationId, Arc<RwLock<Conversation>>>>,
    id: &ConversationId,
) -> TenantId {
    conversations
        .get(id)
        .map(|e| {
            e.value()
                .read()
                .expect("conv lock poisoned")
                .tenant_id
                .clone()
        })
        .unwrap_or_default()
}

/// `MediaGraphHandle::add_sink` is deliberately nonblocking. Orchestrator
/// operations have a stronger contract: once bridge/attach returns, the route
/// must already be active so the caller's first frame is not lost while the
/// graph actor is still processing its command queue.
async fn await_media_route(graph: &MediaGraphHandle, route_id: &MediaRouteId) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if graph
            .snapshot()
            .await
            .sinks
            .iter()
            .any(|sink| &sink.route_id == route_id)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RvoipError::InvalidState(
                "media graph route activation timed out",
            ));
        }
        tokio::task::yield_now().await;
    }
}

/// Gap plan §4.2 v1 punch list — construct a [`TranscoderSwap`] for
/// one direction of a hot-swap. Builds a fresh `Transcoder` (with a
/// new per-direction `FormatConverter`) when `from_pt != to_pt`;
/// otherwise leaves the transcoder slot empty (passthrough).
fn make_swap(from_pt: u8, to_pt: u8) -> frame_pump::TranscoderSwap {
    let transcoder = if from_pt != to_pt {
        Some(Transcoder::new(Arc::new(TokioRwLock::new(
            FormatConverter::new(),
        ))))
    } else {
        None
    };
    frame_pump::TranscoderSwap {
        new_transcoder: transcoder,
        new_from_pt: from_pt,
        new_to_pt: to_pt,
        // A3 — ack is wired by `swap_transcoders` itself when it
        // needs synchronization. `make_swap` leaves it None so the
        // caller decides.
        ack: None,
    }
}

#[cfg(test)]
mod cross_crate_publisher_tests {
    use super::*;
    use rvoip_infra_common::events::cross_crate::RvoipCoreCrossCrateEvent;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Semaphore as TokioSemaphore;

    struct RecordingSink {
        events: Mutex<Vec<String>>,
        block_first: AtomicBool,
        first_started: TokioSemaphore,
        release_first: TokioSemaphore,
        delivered: TokioSemaphore,
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl RecordingSink {
        fn new(block_first: bool) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                block_first: AtomicBool::new(block_first),
                first_started: TokioSemaphore::new(0),
                release_first: TokioSemaphore::new(0),
                delivered: TokioSemaphore::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
            }
        }

        fn event_ids(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl CrossCrateEventSink for RecordingSink {
        async fn publish(&self, event: RvoipCrossCrateEvent) -> std::result::Result<(), String> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);

            if self.block_first.swap(false, Ordering::SeqCst) {
                self.first_started.add_permits(1);
                let permit = self
                    .release_first
                    .acquire()
                    .await
                    .map_err(|error| error.to_string())?;
                permit.forget();
            }

            let id = match event {
                RvoipCrossCrateEvent::Core(RvoipCoreCrossCrateEvent::ConnectionInbound {
                    connection_id,
                }) => connection_id,
                _ => return Err("unexpected cross-crate test event".to_owned()),
            };
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(id);
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.delivered.add_permits(1);
            Ok(())
        }
    }

    fn inbound_event(id: &str) -> Event {
        Event::ConnectionInbound {
            connection_id: ConnectionId::from_string(id),
            at: Utc::now(),
        }
    }

    async fn consume_permits(semaphore: &TokioSemaphore, count: u32) {
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            semaphore.acquire_many(count),
        )
        .await
        .expect("timed out waiting for event publication")
        .expect("test semaphore closed");
        permit.forget();
    }

    #[tokio::test]
    async fn cross_crate_publisher_preserves_fifo_with_one_worker() {
        let sink = Arc::new(RecordingSink::new(false));
        let publisher = CrossCrateEventPublisher::with_capacity(sink.clone(), 8);

        for id in ["one", "two", "three"] {
            assert_eq!(
                publisher.enqueue(inbound_event(id).to_cross_crate()),
                CrossCrateEnqueueResult::Enqueued
            );
        }
        consume_permits(&sink.delivered, 3).await;

        assert_eq!(sink.event_ids(), ["one", "two", "three"]);
        assert_eq!(sink.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn saturation_drops_cross_crate_only_and_keeps_in_process_delivery() {
        let sink = Arc::new(RecordingSink::new(true));
        let publisher = CrossCrateEventPublisher::with_capacity(sink.clone(), 2);
        let (events, _) = broadcast::channel(8);
        let mut in_process = events.subscribe();

        let _ = Orchestrator::emit_to_channels(&events, Some(&publisher), inbound_event("one"));
        consume_permits(&sink.first_started, 1).await;
        let _ = Orchestrator::emit_to_channels(&events, Some(&publisher), inbound_event("two"));
        let _ = Orchestrator::emit_to_channels(&events, Some(&publisher), inbound_event("three"));
        assert_eq!(
            Orchestrator::emit_to_channels(&events, Some(&publisher), inbound_event("four")),
            Some(CrossCrateEnqueueResult::DroppedFull)
        );
        // Even an event rejected by the cross-crate queue must remain visible
        // on the rich, in-process bus.

        sink.release_first.add_permits(1);
        consume_permits(&sink.delivered, 3).await;
        assert_eq!(sink.event_ids(), ["one", "two", "three"]);

        let mut observed = Vec::new();
        for _ in 0..4 {
            let event = tokio::time::timeout(std::time::Duration::from_secs(1), in_process.recv())
                .await
                .expect("timed out waiting for in-process event")
                .expect("in-process event bus closed");
            let Event::ConnectionInbound { connection_id, .. } = event else {
                panic!("unexpected in-process event");
            };
            observed.push(connection_id.to_string());
        }
        assert_eq!(observed, ["one", "two", "three", "four"]);
    }
}
