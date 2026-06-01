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
    AdapterEvent, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest, PlaybackHandle, TransferTarget,
};
use crate::bridge::{codec_to_pt, frame_pump, BridgeManager, CrossBridgeHandle};
use crate::capability::{CapabilityDescriptor, CapabilityIntersection};
use crate::commands::{AudioSource, InboundAction, MuteDirection};
use crate::config::Config;
use crate::connection::Transport;
use crate::conversation::{Conversation, ConversationPolicy, ConversationState};
use crate::error::{Result, RvoipError};
use crate::events::Event;
use crate::ids::{
    BridgeId, ConnectionId, ConversationId, MessageId, ParticipantId, SessionId, StreamId, TenantId,
};
use crate::message::Message;
use crate::participant::{Participant, ParticipantKind, ParticipantRole};
use crate::session::{ConnectionRef, Session, SessionMedium, SessionState};
use crate::stream::StreamKind;
use crate::vcon::VconBuilderHandle;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_media_core::codec::transcoding::Transcoder;
use rvoip_media_core::processing::format::FormatConverter;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, RwLock as TokioRwLock, Semaphore};
use tracing::{debug, instrument, warn};

/// Per-connection registration tracked by the orchestrator so subsequent
/// commands (`end`, `hold`, `transfer`, `send_dtmf`, ...) can route to the
/// right adapter without the caller re-stating the transport.
#[derive(Clone, Debug)]
struct ConnectionEntry {
    transport: Transport,
}

pub struct Orchestrator {
    pub config: Config,
    pub bridges: BridgeManager,
    /// Cross-transport bridges — siblings of `bridges` (which holds the
    /// SIP-fast-path `BridgeHandle`s from media-core). Dropping a handle
    /// from this map aborts its two pump tasks.
    cross_bridges: Arc<DashMap<BridgeId, CrossBridgeHandle>>,
    pub admission: Arc<Semaphore>,
    adapters: Arc<DashMap<Transport, Arc<dyn ConnectionAdapter>>>,
    connections: Arc<DashMap<ConnectionId, ConnectionEntry>>,
    events: broadcast::Sender<Event>,
    /// Optional cross-crate publication. When `Some`, every emitted event is
    /// also published through `infra-common::GlobalEventCoordinator` as the
    /// `RvoipCrossCrateEvent::Core(...)` variant.
    coordinator: Option<Arc<GlobalEventCoordinator>>,
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
    /// P5 — provider registry (name → Arc<dyn Provider>). Populated
    /// by `register_asr_provider` etc. before `attach_ai` /
    /// `start_recording` / `start_transcription` resolve the name.
    asr_providers: Arc<DashMap<String, Arc<dyn crate::harness::AsrProvider>>>,
    tts_providers: Arc<DashMap<String, Arc<dyn crate::harness::TtsProvider>>>,
    dialog_managers: Arc<DashMap<String, Arc<dyn crate::harness::DialogManager>>>,
    recording_sinks:
        Arc<DashMap<String, Arc<dyn crate::harness::RecordingSink>>>,
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
    listener_tasks: Arc<DashMap<crate::ids::ListenerId, tokio::task::AbortHandle>>,
    /// P9 — per-Session quality accumulator. Each `AdapterEvent::Quality`
    /// updates the aggregator for the Session that owns the
    /// Connection; `end_session` snapshots + fills
    /// `SessionEnded.report`.
    session_quality: Arc<DashMap<SessionId, QualityAggregator>>,
    /// P6 — per-tenant quotas. Empty map = unlimited everywhere.
    tenant_quotas: Arc<DashMap<TenantId, crate::config::TenantQuotas>>,
    /// P6 — per-tenant Conversation index.
    conversations_by_tenant:
        Arc<DashMap<TenantId, dashmap::DashSet<ConversationId>>>,
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
    pub abort: tokio::task::AbortHandle,
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
    pub abort: tokio::task::AbortHandle,
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
    pub abort: tokio::task::AbortHandle,
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
            admission,
            adapters: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            events,
            coordinator: None,
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
            admission,
            adapters: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            events,
            coordinator: Some(coordinator),
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
        if self.adapters.contains_key(&transport) {
            return Err(RvoipError::AdapterAlreadyRegistered(transport));
        }
        let mut events = adapter.subscribe_events();
        self.adapters.insert(transport, adapter);

        // Spawn the per-adapter event-normalize loop. Each AdapterEvent is
        // translated into one or more rvoip-core Events and republished.
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                me.handle_adapter_event(transport, event);
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

    /// Look up which adapter owns a given connection. Returns
    /// [`RvoipError::ConnectionNotFound`] if the connection isn't registered.
    fn adapter_for(&self, conn: &ConnectionId) -> Result<Arc<dyn ConnectionAdapter>> {
        let entry = self
            .connections
            .get(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let transport = entry.transport;
        drop(entry);
        self.adapter(transport)
    }

    fn track_connection(&self, conn: &ConnectionId, transport: Transport) {
        self.connections
            .insert(conn.clone(), ConnectionEntry { transport });
    }

    /// If `conn` is currently in a cross-transport bridge, return the
    /// peer `ConnectionId` on the other leg. Gap plan §4.3 / v1 punch
    /// list — used by the DTMF auto-route in the `AdapterEvent::Dtmf`
    /// handler to forward digits across the bridge when one side
    /// signals DTMF out-of-band (e.g. UCTP `dtmf.send` envelope) and
    /// the bridged peer needs to inject the corresponding RFC 4733
    /// telephone-event packets onto its outbound RTP.
    fn bridge_peer_of(&self, conn: &ConnectionId) -> Option<ConnectionId> {
        for entry in self.cross_bridges.iter() {
            let h = entry.value();
            if &h.a == conn {
                return Some(h.b.clone());
            }
            if &h.b == conn {
                return Some(h.a.clone());
            }
        }
        None
    }

    fn forget_connection(&self, conn: &ConnectionId) {
        self.connections.remove(conn);
        // P1.10 — if this Connection was bound to a Session, detach it
        // and auto-end the Session when it loses its last Connection.
        // Must run before subscription cleanup so the Session lookup
        // sees a stable connection set.
        self.detach_connection_from_session(conn);
        // Eagerly clean up any subscriptions that name this Connection
        // (either as publisher or subscriber). Idempotent — see
        // `SessionSubscriptions::drop_connection` for the contract.
        self.subscriptions.drop_connection(conn);
        // Mirror the cleanup into the publisher registry so a publisher
        // that hangs up doesn't leave stale `(sid, strm_id) -> connid`
        // and `(sid, participant) -> [strm_id]` rows that a subsequent
        // `from_participant` subscribe would resolve to a dead Connection.
        // Skip if the registry was never lazily initialized.
        if let Some(reg) = self.publisher_registry.get() {
            reg.drop_publisher(conn);
        }
        // MP3c subscriber-stream map: drop rows that name this
        // Connection as subscriber OR publisher so the per-subscription
        // MediaStream goes out of scope along with the substrate-level
        // Connection. Without this, the per-publisher MediaStreams keep
        // a strong reference to the dead Connection's quinn handle.
        self.subscriber_streams
            .retain(|(_, sub, pubr, _), _| sub != conn && pubr != conn);
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
        let admission_in_use = (self.config.max_concurrent_setups
            - self.admission.available_permits()) as u64;
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
        let Some(tenant) = self
            .conversations
            .get(conv_id)
            .map(|e| e.value().read().expect("conv lock poisoned").tenant_id.clone())
        else {
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
    pub async fn close_conversation(
        &self,
        id: ConversationId,
        force: bool,
    ) -> Result<()> {
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
    /// joins via [`join_session`] (so identity_ref / kind / role land
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
    pub async fn end_session(
        &self,
        session_id: SessionId,
        reason: EndReason,
    ) -> Result<()> {
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
        let tenant_id = self
            .conversations
            .get(&conv_id)
            .map(|e| e.value().read().expect("conv lock poisoned").tenant_id.clone());
        if let (Some((_, builder)), Some(tenant_id)) =
            (self.session_vcons.remove(&session_id), tenant_id)
        {
            let snap = builder.snapshot();
            let bytes = crate::vcon::encode_snapshot(&snap);
            let store = Arc::clone(&self.config.vcon_store);
            let sid_clone = session_id.clone();
            let events_tx = self.events.clone();
            let coordinator = self.coordinator.clone();
            tokio::spawn(async move {
                match store.put(&tenant_id, &sid_clone, bytes).await {
                    Ok(handle) => {
                        let ev = Event::VconReady {
                            session_id: sid_clone,
                            handle,
                            at: Utc::now(),
                        };
                        if let Some(coord) = coordinator {
                            let cross = ev.to_cross_crate();
                            let _ = coord.publish(Arc::new(cross)).await;
                        }
                        let _ = events_tx.send(ev);
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
        if let Some(builder) = self.session_vcons.get(&session_id).map(|e| Arc::clone(e.value())) {
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
            if let Some(p) = conv.participants.iter_mut().find(|p| p.id == participant_id) {
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
    pub fn conversation(
        &self,
        id: &ConversationId,
    ) -> Option<Arc<RwLock<Conversation>>> {
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
            let auto_end =
                sess.state == SessionState::Active && sess.connections.is_empty();
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
        self.subscriber_streams
            .retain(|(s, _, _, _), _| s != sid);
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
                    .allocate_subscriber_stream(
                        subscriber_connid.clone(),
                        frame.kind,
                        codec,
                    )
                    .await
                {
                    Ok(stream) => {
                        self.subscriber_streams.insert(key.clone(), Arc::clone(&stream));
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
            if tx.send(frame.clone()).await.is_ok() {
                delivered += 1;
            }
        }
        delivered
    }

    /// Process-shared [`PublisherRegistry`] for the multi-party fanout
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

    /// Publish an event on the in-process broadcast channel and, if a
    /// `GlobalEventCoordinator` is configured, on the cross-crate bus too.
    fn emit(&self, event: Event) {
        if let Some(coordinator) = &self.coordinator {
            let cross = event.to_cross_crate();
            let coord = Arc::clone(coordinator);
            tokio::spawn(async move {
                if let Err(err) = coord.publish(Arc::new(cross)).await {
                    warn!(?err, "rvoip-core cross-crate event publish failed");
                }
            });
        }
        let _ = self.events.send(event);
    }

    fn handle_adapter_event(&self, transport: Transport, event: AdapterEvent) {
        match event {
            AdapterEvent::InboundConnection { connection } => {
                self.track_connection(&connection.id, transport);
                self.emit(Event::ConnectionInbound {
                    connection_id: connection.id.clone(),
                    at: Utc::now(),
                });
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
            AdapterEvent::Ended {
                connection_id,
                reason,
            } => {
                self.forget_connection(&connection_id);
                self.emit(Event::ConnectionEnded {
                    connection_id,
                    reason,
                    at: Utc::now(),
                });
            }
            AdapterEvent::Failed {
                connection_id,
                detail,
            } => {
                self.forget_connection(&connection_id);
                self.emit(Event::ConnectionFailed {
                    connection_id,
                    detail,
                    at: Utc::now(),
                });
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
                metrics::gauge!("rvoip_media_packet_loss_pct")
                    .set(snapshot.packet_loss_pct as f64);
                self.emit(Event::MediaQuality {
                    connection_id,
                    snapshot,
                    at: Utc::now(),
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
        }
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
        self.track_connection(&handle.connection.id, transport);
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
        self.send_message_to_connection(connection_id, message).await
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
    pub fn record_usage(
        &self,
        tenant_id: TenantId,
        kind: crate::events::UsageKind,
        units: u64,
    ) {
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
            .send_step_up_request(
                connection_id.clone(),
                required.clone(),
                Vec::new(),
                None,
            )
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
            .ok_or_else(|| {
                RvoipError::AdmissionRejected("recording sink not registered")
            })?;

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
                            c.value().read().expect("conv lock poisoned").tenant_id.clone()
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

        let me = Arc::clone(self);
        let sink_for_task = Arc::clone(&sink);
        let conns_for_task = conns.clone();
        let paused_for_task = Arc::clone(&paused);
        let task = tokio::spawn(async move {
            for cid in conns_for_task {
                if let Ok(adapter) = me.adapter_for(&cid) {
                    if let Ok(streams) = adapter.streams(cid.clone()).await {
                        for stream in streams
                            .into_iter()
                            .filter(|s| s.kind() == StreamKind::Audio)
                        {
                            let sink_clone = Arc::clone(&sink_for_task);
                            let paused_clone = Arc::clone(&paused_for_task);
                            tokio::spawn(async move {
                                let mut rx = stream.frames_in();
                                while let Some(frame) = rx.recv().await {
                                    if paused_clone
                                        .load(std::sync::atomic::Ordering::Relaxed)
                                    {
                                        // Drop frame silently while paused.
                                        continue;
                                    }
                                    if sink_clone.write(frame).await.is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                    }
                }
            }
            futures_alive().await;
        });

        // V2.B — the permit (if any) is stored in the handle and
        // drops alongside it on stop_recording.
        let _ = tenant_id;
        self.recordings.insert(
            rid.clone(),
            RecordingHandle {
                sink: Arc::clone(&sink),
                abort: task.abort_handle(),
                paused: Arc::clone(&paused),
                _permit: permit,
            },
        );
        self.emit(Event::RecordingStarted {
            recording_id: rid.clone(),
            at: Utc::now(),
        });
        Ok(rid)
    }

    pub async fn stop_recording(
        &self,
        recording_id: crate::ids::RecordingId,
    ) -> Result<crate::harness::RecordingArtifact> {
        let (_, handle) = self
            .recordings
            .remove(&recording_id)
            .ok_or_else(|| RvoipError::AdmissionRejected("recording not found"))?;
        handle.abort.abort();
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
    pub async fn pause_recording(
        &self,
        id: crate::ids::RecordingId,
    ) -> Result<()> {
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
    pub async fn resume_recording(
        &self,
        id: crate::ids::RecordingId,
    ) -> Result<()> {
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
            .ok_or_else(|| {
                RvoipError::AdmissionRejected("ASR provider not registered")
            })?;
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

        let tid = crate::ids::TranscriptionId::new();
        let me = Arc::clone(self);
        let task = tokio::spawn(async move {
            let stream = match provider
                .open_stream(conn.clone(), crate::harness::AsrConfig::default())
                .await
            {
                Ok(s) => s,
                Err(_) => return,
            };
            // Producer: frames → stream.push.
            let stream_arc: Arc<dyn crate::harness::AsrStream> = Arc::from(stream);
            let stream_for_push = Arc::clone(&stream_arc);
            let conn_for_push = conn.clone();
            let push_task = {
                let me = Arc::clone(&me);
                tokio::spawn(async move {
                    if let Ok(adapter) = me.adapter_for(&conn_for_push) {
                        if let Ok(streams) =
                            adapter.streams(conn_for_push).await
                        {
                            for s in streams
                                .into_iter()
                                .filter(|s| s.kind() == StreamKind::Audio)
                            {
                                let stream_clone = Arc::clone(&stream_for_push);
                                tokio::spawn(async move {
                                    let mut rx = s.frames_in();
                                    while let Some(f) = rx.recv().await {
                                        if stream_clone.push(f).await.is_err() {
                                            break;
                                        }
                                    }
                                });
                            }
                        }
                    }
                })
            };
            // Consumer: stream.next → TranscriptTurn event.
            while let Some(result) = stream_arc.next().await {
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
            let _ = push_task;
        });
        self.transcriptions.insert(
            tid.clone(),
            TranscriptionHandle {
                abort: task.abort_handle(),
            },
        );
        Ok(tid)
    }

    pub async fn stop_transcription(
        &self,
        id: crate::ids::TranscriptionId,
    ) -> Result<()> {
        if let Some((_, h)) = self.transcriptions.remove(&id) {
            h.abort.abort();
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
        let speak_cancel: Arc<
            tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        > = Arc::new(tokio::sync::Mutex::new(None));

        let me = Arc::clone(self);
        let connection_id_for_task = connection_id.clone();
        let aid_for_task = aid.clone();
        let speaking_for_task = Arc::clone(&speaking);
        let speak_cancel_for_task = Arc::clone(&speak_cancel);
        let task = tokio::spawn(async move {
            let connection_id = connection_id_for_task;
            let stream: Arc<dyn crate::harness::AsrStream> = match asr
                .open_stream(
                    connection_id.clone(),
                    crate::harness::AsrConfig::default(),
                )
                .await
            {
                Ok(s) => Arc::from(s),
                Err(_) => return,
            };
            // Push loop.
            let conn_for_push = connection_id.clone();
            let stream_for_push = Arc::clone(&stream);
            let me_for_push = Arc::clone(&me);
            let _push = tokio::spawn(async move {
                if let Ok(adapter) = me_for_push.adapter_for(&conn_for_push) {
                    if let Ok(streams) = adapter.streams(conn_for_push).await {
                        for s in streams
                            .into_iter()
                            .filter(|s| s.kind() == StreamKind::Audio)
                        {
                            let sc = Arc::clone(&stream_for_push);
                            tokio::spawn(async move {
                                let mut rx = s.frames_in();
                                while let Some(f) = rx.recv().await {
                                    if sc.push(f).await.is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                    }
                }
            });
            // Dialog loop with barge-in.
            while let Some(asr_result) = stream.next().await {
                // P5 barge-in: if user speech detected while we're
                // speaking, cancel current playback + fire event.
                if speaking_for_task.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(tx) =
                        speak_cancel_for_task.lock().await.take()
                    {
                        let _ = tx.send(());
                    }
                    speaking_for_task
                        .store(false, std::sync::atomic::Ordering::Relaxed);
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
                        let (cancel_tx, mut cancel_rx) =
                            tokio::sync::oneshot::channel::<()>();
                        *speak_cancel_for_task.lock().await = Some(cancel_tx);
                        speaking_for_task
                            .store(true, std::sync::atomic::Ordering::Relaxed);

                        if let Ok(adapter) = me.adapter_for(&connection_id) {
                            if let Ok(streams) =
                                adapter.streams(connection_id.clone()).await
                            {
                                let out = streams
                                    .into_iter()
                                    .find(|s| s.kind() == StreamKind::Audio);
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
                        speaking_for_task
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        // Drain any stale cancel sender (defensive).
                        let _ = speak_cancel_for_task.lock().await.take();
                    }
                }
            }
        });

        // V2.B — permit (if any) stored in the handle; releases on
        // Drop when detach removes the entry.
        let _ = tenant_id;
        self.ai_attachments.insert(
            aid.clone(),
            AiAttachmentHandle {
                abort: task.abort_handle(),
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

        let lid = crate::ids::ListenerId::new();
        let me = Arc::clone(self);

        // Build the per-sink frame consumer. Channel sinks expose a
        // receiver via `listener_channels`; File/Url sinks just log
        // the byte count (full file/HTTP implementations live in
        // consumer crates).
        let sink_kind = match sink {
            ListenerSink::Channel => "channel",
            ListenerSink::File { .. } => "file",
            ListenerSink::Url(_) => "url",
        };
        let (tx_for_channel, rx_for_channel) = match sink {
            ListenerSink::Channel => {
                let (t, r) = tokio::sync::mpsc::channel::<crate::stream::MediaFrame>(256);
                (Some(t), Some(r))
            }
            _ => (None, None),
        };
        if let Some(rx) = rx_for_channel {
            self.listener_channels.insert(lid.clone(), Mutex::new(Some(rx)));
        }
        let lid_for_task = lid.clone();
        let task = tokio::spawn(async move {
            for cid in conns {
                let Ok(adapter) = me.adapter_for(&cid) else {
                    continue;
                };
                let streams = match adapter.streams(cid.clone()).await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                for s in streams.into_iter().filter(|s| s.kind() == StreamKind::Audio) {
                    let tx_clone = tx_for_channel.clone();
                    let lid_clone = lid_for_task.clone();
                    tokio::spawn(async move {
                        let mut rx = s.frames_in();
                        while let Some(frame) = rx.recv().await {
                            if let Some(tx) = &tx_clone {
                                if tx.send(frame).await.is_err() {
                                    break;
                                }
                            } else {
                                // File/URL — drop after counting.
                                let _ = (frame, &lid_clone);
                            }
                        }
                    });
                }
            }
            let _ = sink_kind;
            // Hold the parent task alive so its abort_handle remains
            // meaningful for the lifetime of the listener.
            futures_alive().await;
        });
        // Bug-fix sweep — register the abort handle so `detach` can
        // tear down the listener cleanly (was leaking before).
        self.listener_tasks.insert(lid.clone(), task.abort_handle());

        self.emit(Event::ListenerAttached {
            listener_id: lid.clone(),
            at: Utc::now(),
        });
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

    pub async fn detach(
        &self,
        attachment: crate::commands::AttachmentRef,
    ) -> Result<()> {
        use crate::commands::AttachmentRef;
        match attachment {
            AttachmentRef::Ai(id) => {
                if let Some((_, h)) = self.ai_attachments.remove(&id) {
                    h.abort.abort();
                    // V2.B — permit drops with the handle struct.
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
                if let Some((_, abort)) = self.listener_tasks.remove(&id) {
                    abort.abort();
                }
                // Drop any cached channel receiver so a re-attach with
                // the same ID (unlikely, but defensive) starts clean.
                self.listener_channels.remove(&id);
                self.emit(Event::ListenerDetached {
                    listener_id: id,
                    at: Utc::now(),
                });
                Ok(())
            }
            AttachmentRef::Recording(id) => {
                self.stop_recording(id).await.map(|_| ())
            }
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
                            .find(|e| {
                                e.value().a == connection_id || e.value().b == connection_id
                            })
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
                        // Re-fetch the bridge entry just to call
                        // swap_transcoders. The entry guard is held
                        // only across this single .await — swap_transcoders
                        // itself sends on the swap channels and (with
                        // A3) awaits the pumps' acks. The guard never
                        // covers a media-path operation.
                        let swap_result = {
                            if let Some(entry) = self.cross_bridges.get(&bridge_id) {
                                let bridge = entry.value();
                                bridge.swap_transcoders(a_swap, b_swap).await
                            } else {
                                Ok(())
                            }
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
    pub async fn mute(
        &self,
        connection_id: ConnectionId,
        direction: MuteDirection,
    ) -> Result<()> {
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
        // Reject if either ConnectionId is already in a cross-transport bridge.
        for entry in self.cross_bridges.iter() {
            let h = entry.value();
            if h.a == a || h.b == a || h.a == b || h.b == b {
                return Err(RvoipError::AdmissionRejected(
                    "connection already bridged",
                ));
            }
        }

        let a_transport = self
            .connections
            .get(&a)
            .ok_or_else(|| RvoipError::ConnectionNotFound(a.clone()))?
            .transport;
        let b_transport = self
            .connections
            .get(&b)
            .ok_or_else(|| RvoipError::ConnectionNotFound(b.clone()))?
            .transport;
        let a_adapter = self.adapter(a_transport)?;
        let b_adapter = self.adapter(b_transport)?;

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
            let a_audio = a_streams.into_iter().find(|s| s.kind() == StreamKind::Audio);
            let b_audio = b_streams.into_iter().find(|s| s.kind() == StreamKind::Audio);
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

        let a_pt = codec_to_pt(&a_audio.codec().name)
            .ok_or_else(|| RvoipError::UnsupportedCodec(a_audio.codec().name.clone()))?;
        let b_pt = codec_to_pt(&b_audio.codec().name)
            .ok_or_else(|| RvoipError::UnsupportedCodec(b_audio.codec().name.clone()))?;

        // One transcoder per direction with its own FormatConverter.
        //
        // FormatConverter caches a Resampler keyed by the *input* sample
        // rate, so sharing across directions would thrash the cache (and
        // could cross-contaminate state) on every flip — e.g. G.711-mu
        // (8 kHz) <-> Opus (48 kHz) would tear down and rebuild the
        // resampler on every frame. Per-direction also removes the
        // RwLock contention point under bidirectional traffic.
        let (transcoder_a_to_b, transcoder_b_to_a) = if a_pt != b_pt {
            (
                Some(Transcoder::new(Arc::new(TokioRwLock::new(
                    FormatConverter::new(),
                )))),
                Some(Transcoder::new(Arc::new(TokioRwLock::new(
                    FormatConverter::new(),
                )))),
            )
        } else {
            (None, None)
        };

        // Single-take channels per MediaStream contract.
        let a_in = a_audio.frames_in();
        let a_out = a_audio.frames_out();
        let b_in = b_audio.frames_in();
        let b_out = b_audio.frames_out();

        // Gap plan §4.2 v1 punch list — wire each pump with a swap
        // channel so `Orchestrator::renegotiate_media` can hot-swap
        // the transcoders after a successful codec renegotiation.
        let (swap_a_to_b_tx, swap_a_to_b_rx) =
            tokio::sync::mpsc::channel::<frame_pump::TranscoderSwap>(4);
        let (swap_b_to_a_tx, swap_b_to_a_rx) =
            tokio::sync::mpsc::channel::<frame_pump::TranscoderSwap>(4);
        let a_to_b = frame_pump::spawn_pump_with_swap(
            "a->b",
            a_in,
            b_out,
            transcoder_a_to_b,
            a_pt,
            b_pt,
            swap_a_to_b_rx,
        );
        let b_to_a = frame_pump::spawn_pump_with_swap(
            "b->a",
            b_in,
            a_out,
            transcoder_b_to_a,
            b_pt,
            a_pt,
            swap_b_to_a_rx,
        );

        let id = BridgeId::new();
        self.cross_bridges.insert(
            id.clone(),
            CrossBridgeHandle::with_swap_channels(
                id.clone(),
                a.clone(),
                b.clone(),
                a_to_b.abort_handle(),
                b_to_a.abort_handle(),
                swap_a_to_b_tx,
                swap_b_to_a_tx,
            ),
        );
        self.emit(Event::ConnectionsBridged {
            bridge_id: id.clone(),
            a,
            b,
            at: Utc::now(),
        });
        Ok(id)
    }

    pub async fn unbridge_connections(&self, bridge_id: BridgeId) -> Result<()> {
        // Cross-transport bridges first (new path). Drop aborts both pumps.
        if let Some((_, _handle)) = self.cross_bridges.remove(&bridge_id) {
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
        .map(|e| e.value().read().expect("conv lock poisoned").tenant_id.clone())
        .unwrap_or_default()
}

/// Helper that blocks until the holding task is aborted. Used by
/// `start_recording` to keep the per-connection spawn task alive so
/// its abort handle remains meaningful.
async fn futures_alive() {
    std::future::pending::<()>().await;
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
