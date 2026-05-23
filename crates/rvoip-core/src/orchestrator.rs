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
    AdapterEvent, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest, RejectReason,
    TransferTarget,
};
use crate::bridge::{codec_to_pt, frame_pump, BridgeManager, CrossBridgeHandle};
use crate::capability::CapabilityDescriptor;
use crate::commands::{InboundAction, MuteDirection};
use crate::config::Config;
use crate::connection::Transport;
use crate::error::{Result, RvoipError};
use crate::events::Event;
use crate::ids::{BridgeId, ConnectionId, SessionId, StreamId};
use crate::message::Message;
use crate::stream::StreamKind;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_media_core::codec::transcoding::Transcoder;
use rvoip_media_core::processing::format::FormatConverter;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock as TokioRwLock, Semaphore};
use tracing::{debug, warn};

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

    fn forget_connection(&self, conn: &ConnectionId) {
        self.connections.remove(conn);
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
    }

    /// Fan a publisher's `MediaFrame` out to every subscriber of
    /// `(sid, publisher, strm_id)`. v0.x MP3a primitive — adapter
    /// datagram-receive loops will call this after unpacking a
    /// publisher's datagram, after MP3b wires the publisher-side
    /// trigger.
    ///
    /// For each subscriber: looks up its `ConnectionAdapter` (via the
    /// connections map populated at registration), queries the
    /// adapter's `streams(connid)`, finds the first `MediaStream`
    /// matching `frame.kind`, and pushes the frame into its
    /// `frames_out` channel. Returns the number of subscribers a
    /// frame was successfully delivered to (best-effort: a single
    /// subscriber's failed delivery does not block the others).
    ///
    /// Refinements deferred to MP3b/MP3c:
    /// - Per-subscriber `stream_local_id` allocation + datagram
    ///   header rewriting (CONVERSATION_PROTOCOL.md §10.1 multi-party
    ///   note). Today the subscriber's `frames_out` consumer (its
    ///   adapter pump) re-frames with whatever local id its
    ///   MediaStream owns.
    /// - Codec mismatch validation. Today `add_subscription` accepts
    ///   any pair; if codecs disagree, the subscriber receives frames
    ///   it can't decode. Validation should land alongside
    ///   storing the publisher's negotiated codec on the
    ///   `PublisherRegistry`.
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
            let Ok(streams) = adapter.streams(subscriber_connid).await else {
                continue;
            };
            let Some(target) = streams.into_iter().find(|s| s.kind() == frame.kind) else {
                continue;
            };
            let tx = target.frames_out();
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
            InboundAction::Accept { .. } => adapter.accept(connection_id).await,
            InboundAction::Reject { reason } => adapter.reject(connection_id, reason).await,
            InboundAction::BridgeTo { .. } => Err(RvoipError::NotImplemented(
                "InboundAction::BridgeTo — bridge dispatch lands with SipBridgeStrategy (step 9+)",
            )),
        }
    }

    pub async fn originate_connection(
        &self,
        request: OriginateRequest,
    ) -> Result<ConnectionHandle> {
        // The OriginateRequest's transport is implied by which adapter we
        // call. v1: caller picks the transport by registering only one
        // adapter at a time; once multi-adapter dispatch is needed (step 9+)
        // the request grows a `transport` field.
        let transport = self
            .adapters
            .iter()
            .next()
            .map(|entry| *entry.key())
            .ok_or(RvoipError::NotImplemented(
                "no adapter registered — register one before originating",
            ))?;
        let adapter = self.adapter(transport)?;
        let handle = adapter.originate(request).await?;
        self.track_connection(&handle.connection.id, transport);
        self.emit(Event::ConnectionOutbound {
            connection_id: handle.connection.id.clone(),
            at: Utc::now(),
        });
        Ok(handle)
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

    pub async fn send_message(&self, connection_id: ConnectionId, message: Message) -> Result<()> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.send_message(connection_id, message).await
    }

    pub async fn renegotiate_media(
        &self,
        connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<crate::capability::NegotiatedCodecs> {
        let adapter = self.adapter_for(&connection_id)?;
        adapter.renegotiate_media(connection_id, capabilities).await
    }

    // Mute/Unmute aren't on the ConnectionAdapter trait yet (per
    // INTERFACE_DESIGN §6 they're per-direction local controls). They will
    // land when the trait grows the corresponding methods.
    pub async fn mute(
        &self,
        _connection_id: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "mute — adapter trait method not defined yet",
        ))
    }

    pub async fn unmute(
        &self,
        _connection_id: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "unmute — adapter trait method not defined yet",
        ))
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

        let a_to_b = frame_pump::spawn_pump("a->b", a_in, b_out, transcoder_a_to_b, a_pt, b_pt);
        let b_to_a = frame_pump::spawn_pump("b->a", b_in, a_out, transcoder_b_to_a, b_pt, a_pt);

        let id = BridgeId::new();
        self.cross_bridges.insert(
            id.clone(),
            CrossBridgeHandle::new(id.clone(), a.clone(), b.clone(), a_to_b.abort_handle(), b_to_a.abort_handle()),
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
