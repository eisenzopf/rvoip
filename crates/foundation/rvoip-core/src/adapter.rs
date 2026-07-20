use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::commands::{AudioSource, MuteDirection};
use crate::connection::Transport;
use crate::error::{Result, RvoipError};
use crate::identity::IdentityAssurance;
use crate::ids::ConnectionId;
use crate::message::Message;
use crate::stream::MediaStream;
use crate::DataMessage;
use std::fmt;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

pub use rvoip_core_traits::adapter::{
    AdapterEvent, AdapterKind, ConnectionHandle, EndReason, ExternalConnectionReference,
    ExternalConnectionReferenceError, InboundConnectionContext, InboundContextError,
    InboundRoutingHint, InboundSignalingMetadata, OriginateContext, OriginateRequest,
    OutboundActivation, PlaybackHandle, RejectReason, SignatureHeaders, TransferStatus,
    TransferTarget, MAX_EXTERNAL_CONNECTION_REFERENCES, MAX_EXTERNAL_REFERENCE_KIND_BYTES,
    MAX_EXTERNAL_REFERENCE_VALUE_BYTES, MAX_INBOUND_ROUTING_HINT_BYTES,
};
pub use rvoip_core_traits::ids::TransferAttemptId;

/// Core-private adapter-to-Orchestrator event envelope.
///
/// This type is public only because [`ConnectionAdapter`] is implemented by
/// transport crates. Application-facing subscriptions continue to expose
/// [`AdapterEvent`] and therefore retain its existing source surface.
#[doc(hidden)]
#[derive(Clone)]
#[non_exhaustive]
pub enum OrchestratorAdapterEvent {
    Public(AdapterEvent),
    AuthenticatedInboundConnection {
        connection: crate::connection::Connection,
        participant_id: String,
        principal: crate::identity::AuthenticatedPrincipal,
    },
}

impl fmt::Debug for OrchestratorAdapterEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public(event) => formatter.debug_tuple("Public").field(event).finish(),
            Self::AuthenticatedInboundConnection { connection, .. } => formatter
                .debug_struct("AuthenticatedInboundConnection")
                .field("transport", &connection.transport)
                .field("direction", &connection.direction)
                .field("state", &connection.state)
                .field("stream_count", &connection.streams.len())
                .field("participant_present", &true)
                .field("principal_present", &true)
                .finish(),
        }
    }
}

impl From<AdapterEvent> for OrchestratorAdapterEvent {
    fn from(event: AdapterEvent) -> Self {
        Self::Public(event)
    }
}

/// Direct fallback for terminal adapter events when the adapter's bounded
/// event queue is saturated or closed.
///
/// The Orchestrator implementation invalidates/removes the connection before
/// awaiting the remaining media cleanup. Adapters invoke this only after
/// removing their own route and stream state; the peer task retains its
/// bounded admission permit until cleanup converges.
#[async_trait::async_trait]
pub trait AdapterLifecycleSink: Send + Sync {
    async fn deliver_terminal(&self, event: AdapterEvent);
}

/// Shareable, late-bound lifecycle sink used by adapters whose server loops
/// start before the adapter is registered with an Orchestrator.
#[derive(Clone, Default)]
pub struct AdapterLifecycleSinkSlot {
    inner: Arc<OnceLock<Arc<dyn AdapterLifecycleSink>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalDelivery {
    Queued,
    Fallback,
    Undeliverable,
}

/// Lifecycle guarantees an adapter can provide to the Orchestrator.
///
/// All fields default to `false`. This keeps third-party adapter
/// implementations source compatible while allowing security-sensitive core
/// features to reject an adapter that cannot satisfy their fail-closed
/// contract.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterLifecycleCapabilities {
    /// [`ConnectionAdapter::is_connection_live`] is authoritative for every
    /// route owned by this adapter.
    pub authoritative_liveness: bool,
    /// Authenticated inbound identity and connection creation are delivered
    /// as one [`OrchestratorAdapterEvent::AuthenticatedInboundConnection`]
    /// before any operational event for that connection.
    pub atomic_inbound_handoff: bool,
    /// The adapter installs and uses [`AdapterLifecycleSink`] when its normal
    /// bounded event path cannot deliver a terminal event.
    pub terminal_fallback: bool,
    /// Outbound routes remain externally dormant after `originate` returns:
    /// they publish no peer-visible call signaling and release no lifecycle
    /// event until core calls the outbound activation hook. Local allocation,
    /// SDP construction, and ICE gathering are allowed during preparation.
    pub staged_outbound_activation: bool,
}

impl AdapterLifecycleCapabilities {
    /// Capabilities required by the Orchestrator's fail-closed inbound
    /// admission gate.
    pub const FAIL_CLOSED_INBOUND: Self = Self {
        authoritative_liveness: true,
        atomic_inbound_handoff: true,
        terminal_fallback: true,
        staged_outbound_activation: false,
    };

    pub const fn supports_fail_closed_inbound(self) -> bool {
        self.authoritative_liveness && self.atomic_inbound_handoff && self.terminal_fallback
    }
}

impl AdapterLifecycleSinkSlot {
    pub fn install(
        &self,
        sink: Arc<dyn AdapterLifecycleSink>,
    ) -> std::result::Result<(), Arc<dyn AdapterLifecycleSink>> {
        self.inner.set(sink)
    }

    /// Deliver a terminal event through the installed fallback. Returns
    /// `false` when the adapter has not been registered with an Orchestrator.
    pub async fn deliver_terminal(&self, event: AdapterEvent) -> bool {
        let Some(sink) = self.inner.get().cloned() else {
            return false;
        };
        sink.deliver_terminal(event).await;
        true
    }

    /// Prefer the adapter's normal bounded event queue so terminal events
    /// retain FIFO ordering. If that queue is full or closed, invoke the
    /// direct lifecycle sink instead of waiting indefinitely or allocating an
    /// unbounded overflow queue.
    pub async fn queue_or_deliver_terminal(
        &self,
        events: &mpsc::Sender<AdapterEvent>,
        event: AdapterEvent,
    ) -> TerminalDelivery {
        match events.try_send(event) {
            Ok(()) => TerminalDelivery::Queued,
            Err(mpsc::error::TrySendError::Full(event))
            | Err(mpsc::error::TrySendError::Closed(event)) => {
                if self.deliver_terminal(event).await {
                    TerminalDelivery::Fallback
                } else {
                    TerminalDelivery::Undeliverable
                }
            }
        }
    }

    /// Atomic-stream counterpart to [`Self::queue_or_deliver_terminal`].
    pub async fn queue_or_deliver_orchestrator_terminal(
        &self,
        events: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: AdapterEvent,
    ) -> TerminalDelivery {
        match events.try_send(OrchestratorAdapterEvent::Public(event)) {
            Ok(()) => TerminalDelivery::Queued,
            Err(mpsc::error::TrySendError::Full(OrchestratorAdapterEvent::Public(event)))
            | Err(mpsc::error::TrySendError::Closed(OrchestratorAdapterEvent::Public(event))) => {
                if self.deliver_terminal(event).await {
                    TerminalDelivery::Fallback
                } else {
                    TerminalDelivery::Undeliverable
                }
            }
            Err(mpsc::error::TrySendError::Full(_)) | Err(mpsc::error::TrySendError::Closed(_)) => {
                debug_assert!(false, "terminal event wrapper changed unexpectedly");
                TerminalDelivery::Undeliverable
            }
        }
    }
}

/// Expand atomic authenticated-inbound events into the historical direct
/// adapter sequence without changing the Orchestrator's source queue.
///
/// The input receiver has already accepted the connection and its principal
/// as one bounded item. This forwarding task preserves event order and awaits
/// both compatibility events before reading the next source item. It is used
/// only for explicit direct adapter subscriptions; Orchestrator registration
/// consumes the unexpanded receiver through
/// [`ConnectionAdapter::subscribe_orchestrator_events`].
pub fn legacy_normalized_event_receiver(
    mut source: mpsc::Receiver<OrchestratorAdapterEvent>,
    capacity: usize,
) -> mpsc::Receiver<AdapterEvent> {
    let (events, receiver) = mpsc::channel(capacity.max(2));
    tokio::spawn(async move {
        while let Some(event) = source.recv().await {
            match event {
                OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                    connection,
                    participant_id,
                    principal,
                } => {
                    let connection_id = connection.id.clone();
                    if events
                        .send(AdapterEvent::InboundConnection { connection })
                        .await
                        .is_err()
                    {
                        break;
                    }
                    if events
                        .send(AdapterEvent::PrincipalAuthenticated {
                            connection_id,
                            participant_id,
                            principal,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                OrchestratorAdapterEvent::Public(event) => {
                    if events.send(event).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

/// The cross-transport adapter contract. Every transport-specific crate
/// (rvoip-sip, rvoip-webrtc, rvoip-quic, rvoip-webtransport, rvoip-websocket)
/// implements this so the [`crate::Orchestrator`] can dispatch generically.
#[async_trait::async_trait]
pub trait ConnectionAdapter: Send + Sync {
    fn transport(&self) -> Transport;
    fn kind(&self) -> AdapterKind;

    /// Explicit lifecycle guarantees implemented by this adapter.
    ///
    /// The default advertises no guarantees. In particular, overriding only
    /// one of `install_lifecycle_sink`, `is_connection_live`, or
    /// `subscribe_orchestrator_events` is not enough to opt into fail-closed
    /// inbound admission: the adapter must advertise the complete contract.
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities::default()
    }

    /// Whether this adapter consumes final inbound-admission confirmations.
    ///
    /// This is intentionally a separate, source-compatible capability rather
    /// than a field on [`AdapterLifecycleCapabilities`]. Existing adapters
    /// therefore remain build-compatible and default to the historical
    /// behavior.
    fn supports_inbound_admission_confirmation(&self) -> bool {
        false
    }

    /// Report the final policy outcome for one exact inbound lifecycle.
    ///
    /// Core calls this synchronously, at most once for a
    /// `(transport, connection_id, lifecycle_generation)` tuple, and only
    /// when an inbound admission gate is installed. `accepted` becomes true
    /// only after publication has committed; every fail-closed disposition is
    /// false. The callback deliberately carries no principal, credentials,
    /// attachment context, or signaling metadata.
    ///
    /// Implementations must return promptly and be idempotent. An adapter may
    /// use the generation to reject a delayed notification for a superseded
    /// local waiter. Waiting for this callback belongs inside the adapter's
    /// protocol task; it must not block the Orchestrator event loop.
    /// `accepted=true` is a policy/publication result, not a continuing
    /// liveness guarantee: a terminal event may follow immediately and must
    /// independently cancel or close the adapter's protocol waiter/response.
    fn notify_inbound_admission_outcome(
        &self,
        _connection_id: &ConnectionId,
        _lifecycle_generation: u64,
        _accepted: bool,
    ) {
    }

    /// Install the Orchestrator's terminal-event fallback. The default is a
    /// no-op for adapters that cannot overrun their lifecycle event path.
    fn install_lifecycle_sink(&self, _sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        Ok(())
    }

    /// Whether the adapter still owns a live route for `conn`. The
    /// Orchestrator consults this before accepting queued inbound/principal
    /// events, preventing an event that was queued before abrupt teardown
    /// from resurrecting a cleaned connection.
    fn is_connection_live(&self, _conn: &ConnectionId) -> bool {
        true
    }

    /// Take adapter-owned context captured for one inbound connection.
    ///
    /// Implementations must bind the value to the exact connection,
    /// transport, and authenticated principal that produced it and return it
    /// at most once. The default keeps existing adapters source compatible.
    fn take_inbound_context(&self, _conn: &ConnectionId) -> Option<InboundConnectionContext> {
        None
    }

    /// Subscribe to the adapter's atomic lifecycle stream for Orchestrator use.
    ///
    /// The default preserves source and behavioral compatibility for adapters
    /// that do not distinguish their public event stream. SIP and WebRTC
    /// override this method so an authenticated inbound handoff
    /// remains one bounded queue item on the security-sensitive path while
    /// their legacy public subscription continues to expand that item into
    /// `InboundConnection` followed by `PrincipalAuthenticated`.
    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        let mut public = self.subscribe_events();
        let (events, receiver) = mpsc::channel(256);
        tokio::spawn(async move {
            while let Some(event) = public.recv().await {
                if events
                    .send(OrchestratorAdapterEvent::Public(event))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        receiver
    }

    /// Create an outbound route.
    ///
    /// Adapters advertising
    /// [`AdapterLifecycleCapabilities::staged_outbound_activation`] must
    /// return a live but externally dormant route. `originate` may allocate
    /// local protocol state, construct SDP, and gather ICE, but it must not
    /// send a peer-visible call command (for example SIP INVITE or a provider
    /// originate request). The adapter also retains operational, principal,
    /// and terminal events in one bounded FIFO until activation. This ordering
    /// lets the Orchestrator claim and durably bind the returned ID before an
    /// external call exists or an event can refer to it. Core deliberately
    /// does not stage events for unknown IDs.
    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle>;

    /// Publish peer-visible signaling and release events for a successfully
    /// claimed outbound route.
    ///
    /// The default is a no-op for legacy adapters. Implementations that
    /// advertise staged outbound activation must make this operation
    /// idempotent and release retained events in FIFO order. Adapters that do
    /// not advertise it are compatibility-only for outbound origination:
    /// core cannot recover an operational, principal, or terminal event that
    /// such an adapter emits before `originate` returns its previously
    /// unknown connection ID.
    async fn activate_outbound(&self, _conn: ConnectionId) -> Result<()> {
        Ok(())
    }

    /// Activate a claimed outbound route and return its opaque activation
    /// receipt.
    ///
    /// Core calls this hook for both prepared and legacy outbound paths. The
    /// default preserves existing adapters by delegating to
    /// [`Self::activate_outbound`] and returning an empty receipt. Adapters
    /// that own stable external identifiers should override this method,
    /// perform the activation exactly once, and include those identifiers in
    /// the returned [`OutboundActivation`]. The override must be idempotent
    /// and return the same identifiers after a repeated activation. Core
    /// exposes the receipt only after activation, route liveness, lifecycle,
    /// and event-stream checks all succeed.
    async fn activate_outbound_with_receipt(
        &self,
        conn: ConnectionId,
    ) -> Result<OutboundActivation> {
        self.activate_outbound(conn).await?;
        Ok(OutboundActivation::default())
    }

    /// Start provisional inbound media without finally accepting the route.
    ///
    /// The default is unsupported. SIP adapters use this to negotiate and
    /// send a 183 response with SDP while an [`InboundAdmission`](crate::InboundAdmission)
    /// remains unresolved. Implementations must not emit a final answer or a
    /// transport-neutral `Connected` event from this operation.
    async fn start_inbound_early_media(&self, _conn: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "adapter does not support provisional inbound early media",
        ))
    }

    async fn accept(&self, conn: ConnectionId) -> Result<()>;
    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> Result<()>;
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> Result<()>;
    async fn hold(&self, conn: ConnectionId) -> Result<()>;
    async fn resume(&self, conn: ConnectionId) -> Result<()>;
    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> Result<()>;

    /// Submit a transfer with an application-owned correlation identifier.
    ///
    /// The default preserves compatibility by delegating to [`Self::transfer`].
    /// Adapters that can bind protocol status to the exact submitted transfer
    /// override this method and echo `attempt_id` in
    /// [`AdapterEvent::TransferStatus`].
    async fn transfer_with_attempt(
        &self,
        conn: ConnectionId,
        _attempt_id: TransferAttemptId,
        target: TransferTarget,
    ) -> Result<()> {
        self.transfer(conn, target).await
    }

    async fn streams(&self, conn: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>>;

    /// Allocate a fresh per-`(subscriber, publisher_strm)` MediaStream for
    /// the multi-party fanout path (plan §12 MP3c / G4). Required so a
    /// subscriber in an N-party room can demultiplex datagrams from
    /// multiple upstream publishers via distinct `stream_local_id`s on
    /// the wire — without this, all publishers land on the subscriber's
    /// default stream and the audio mixes at the jitter buffer.
    ///
    /// The default implementation returns
    /// [`RvoipError::NotImplemented`] so non-UCTP adapters (SIP,
    /// WebRTC) — which don't carry multi-party fanout responsibility —
    /// can stay unchanged. UCTP-family adapters override this to:
    /// 1. Allocate a fresh `stream_local_id` on the subscriber's
    ///    substrate connection.
    /// 2. Construct a directional `MediaStream` with that id.
    /// 3. Register it in the per-peer streams map so subsequent
    ///    [`Self::streams`] calls return it and inbound datagrams on
    ///    that id route correctly (subscribers may publish back).
    /// 4. Emit a `stream.opened` envelope to the peer announcing the
    ///    new id per CONVERSATION_PROTOCOL.md §10.1 multi-party note.
    ///
    /// `Orchestrator::fanout_frame` falls back to the legacy
    /// pick-by-kind behavior when this returns `NotImplemented`, so
    /// single-publisher rooms keep working everywhere.
    async fn allocate_subscriber_stream(
        &self,
        _subscriber: ConnectionId,
        _kind: crate::stream::StreamKind,
        _codec: crate::capability::CodecInfo,
    ) -> Result<Arc<dyn MediaStream>> {
        Err(RvoipError::NotImplemented(
            "ConnectionAdapter::allocate_subscriber_stream",
        ))
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> Result<()>;
    async fn send_data_message(&self, _conn: ConnectionId, _message: DataMessage) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "ConnectionAdapter::send_data_message",
        ))
    }
    async fn send_dtmf(&self, conn: ConnectionId, digits: &str, duration_ms: u32) -> Result<()>;
    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs>;

    /// P2 — local mute/unmute on a per-direction basis. Default
    /// `NotImplemented` so adapters opt in; the Orchestrator surfaces
    /// the error verbatim when a caller invokes mute against a
    /// transport that hasn't wired it.
    async fn mute(&self, _conn: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::mute"))
    }
    async fn unmute(&self, _conn: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::unmute"))
    }

    /// P2 — play `source` toward the peer on `conn`. Adapters that
    /// implement this construct a [`PlaybackHandle`] via
    /// [`PlaybackHandle::new`], spawn the playback task watching the
    /// returned `cancel_rx`, and return the handle. Default
    /// `NotImplemented`.
    async fn play_audio(
        &self,
        _conn: ConnectionId,
        _source: AudioSource,
    ) -> Result<PlaybackHandle> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::play_audio"))
    }

    /// P12.6 — send an `identity.step-up-request` envelope to the peer
    /// asking them to present higher-assurance credentials. The peer's
    /// `identity.step-up-response` arrives as
    /// [`AdapterEvent::StepUpResponse`] which the orchestrator
    /// re-emits as [`crate::events::Event::IdentityStepUpResponseReceived`].
    /// UCTP-family adapters override this; SIP / WebRTC default to
    /// `NotImplemented` since step-up is a UCTP-native flow per
    /// CONVERSATION_PROTOCOL.md §5.8.
    async fn send_step_up_request(
        &self,
        _conn: ConnectionId,
        _required: crate::capability::IdentityAssuranceRequirement,
        _allowed_methods: Vec<String>,
        _reason: Option<String>,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "ConnectionAdapter::send_step_up_request",
        ))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent>;
    fn capabilities(&self) -> CapabilityDescriptor;

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        signature: SignatureHeaders,
    ) -> Result<IdentityAssurance>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecordingSink {
        delivered: AtomicBool,
    }

    #[async_trait::async_trait]
    impl AdapterLifecycleSink for RecordingSink {
        async fn deliver_terminal(&self, event: AdapterEvent) {
            assert!(matches!(event, AdapterEvent::Ended { .. }));
            self.delivered.store(true, Ordering::Release);
        }
    }

    fn terminal_event() -> AdapterEvent {
        AdapterEvent::Ended {
            connection_id: ConnectionId::new(),
            reason: EndReason::Normal,
        }
    }

    #[tokio::test]
    async fn saturated_event_queue_uses_direct_terminal_fallback() {
        let (events_tx, _events_rx) = mpsc::channel(1);
        events_tx
            .try_send(AdapterEvent::Native {
                kind: "occupied",
                detail: "queue full".into(),
            })
            .expect("fill event queue");

        let sink = Arc::new(RecordingSink {
            delivered: AtomicBool::new(false),
        });
        let slot = AdapterLifecycleSinkSlot::default();
        assert!(slot.install(sink.clone()).is_ok());

        assert_eq!(
            slot.queue_or_deliver_terminal(&events_tx, terminal_event())
                .await,
            TerminalDelivery::Fallback
        );
        assert!(sink.delivered.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn closed_event_queue_uses_direct_terminal_fallback() {
        let (events_tx, events_rx) = mpsc::channel(1);
        drop(events_rx);
        let sink = Arc::new(RecordingSink {
            delivered: AtomicBool::new(false),
        });
        let slot = AdapterLifecycleSinkSlot::default();
        assert!(slot.install(sink.clone()).is_ok());

        assert_eq!(
            slot.queue_or_deliver_terminal(&events_tx, terminal_event())
                .await,
            TerminalDelivery::Fallback
        );
        assert!(sink.delivered.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn available_event_queue_preserves_normal_terminal_ordering() {
        let (events_tx, mut events_rx) = mpsc::channel(1);
        let sink = Arc::new(RecordingSink {
            delivered: AtomicBool::new(false),
        });
        let slot = AdapterLifecycleSinkSlot::default();
        assert!(slot.install(sink.clone()).is_ok());

        assert_eq!(
            slot.queue_or_deliver_terminal(&events_tx, terminal_event())
                .await,
            TerminalDelivery::Queued
        );
        assert!(!sink.delivered.load(Ordering::Acquire));
        assert!(matches!(
            events_rx.try_recv().expect("queued terminal event"),
            AdapterEvent::Ended { .. }
        ));
    }

    #[tokio::test]
    async fn unregistered_saturated_queue_reports_undeliverable_terminal() {
        let (events_tx, _events_rx) = mpsc::channel(1);
        events_tx
            .try_send(AdapterEvent::Native {
                kind: "occupied",
                detail: "queue full".into(),
            })
            .expect("fill event queue");

        assert_eq!(
            AdapterLifecycleSinkSlot::default()
                .queue_or_deliver_terminal(&events_tx, terminal_event())
                .await,
            TerminalDelivery::Undeliverable
        );
    }

    #[tokio::test]
    async fn second_sink_install_is_rejected_and_first_sink_is_retained() {
        let first = Arc::new(RecordingSink {
            delivered: AtomicBool::new(false),
        });
        let second = Arc::new(RecordingSink {
            delivered: AtomicBool::new(false),
        });
        let slot = AdapterLifecycleSinkSlot::default();
        assert!(slot.install(first.clone()).is_ok());
        assert!(slot.install(second.clone()).is_err());

        assert!(slot.deliver_terminal(terminal_event()).await);
        assert!(first.delivered.load(Ordering::Acquire));
        assert!(!second.delivered.load(Ordering::Acquire));
    }
}
