use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::commands::{AudioSource, MuteDirection};
use crate::connection::{Connection, Direction, Transport};
use crate::error::{Result, RvoipError};
use crate::identity::{IdentityAssurance, Jwk};
use crate::ids::{ConnectionId, ParticipantId, PlaybackId, SessionId};
use crate::message::Message;
use crate::stream::MediaStream;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterKind {
    /// UCTP-native (QUIC, WebTransport, WebSocket).
    Substrate,
    /// Gateway to a foreign protocol (SIP, WebRTC).
    Interop,
}

#[derive(Clone, Debug)]
pub struct OriginateRequest {
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub target: String,
    pub direction: Direction,
    pub capabilities: CapabilityDescriptor,
    /// P6 — transport selector. When `Some`, the Orchestrator
    /// dispatches the originate through the adapter registered for
    /// this transport. When `None`, the "first registered adapter"
    /// fallback applies (single-adapter deployments).
    pub transport: Option<Transport>,
}

#[derive(Clone, Debug)]
pub struct ConnectionHandle {
    pub connection: Connection,
}

#[derive(Clone, Debug)]
pub enum RejectReason {
    Busy,
    Decline,
    NotFound,
    Forbidden,
    NotAcceptable,
    ServerError,
    Custom { code: u16, phrase: String },
}

#[derive(Clone, Debug)]
pub enum EndReason {
    Normal,
    Cancelled,
    Failed { detail: String },
    Timeout,
    BridgeTorn,
}

#[derive(Clone, Debug)]
pub enum TransferTarget {
    Uri(String),
    Connection(ConnectionId),
    Session(SessionId),
}

/// P2 — handle returned by [`ConnectionAdapter::play_audio`] (and
/// surfaced by [`crate::Orchestrator::play_audio`]) that lets the
/// caller stop an in-flight playback. The cancel channel is fired
/// at-most-once by [`Self::cancel`]; subsequent calls compile-error
/// because cancel takes `self`.
#[derive(Debug)]
pub struct PlaybackHandle {
    id: PlaybackId,
    cancel_tx: oneshot::Sender<()>,
}

impl PlaybackHandle {
    /// Adapter helper: build a handle + the matching cancel receiver.
    /// The adapter spawns its playback task watching `cancel_rx`; when
    /// the consumer calls [`Self::cancel`] the task gets `Ok(())` on
    /// its receiver and tears down.
    pub fn new(id: PlaybackId) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self { id, cancel_tx: tx }, rx)
    }

    pub fn id(&self) -> &PlaybackId {
        &self.id
    }

    /// Best-effort cancellation. Returns `Err` only when the adapter's
    /// playback task already exited (receiver dropped) — which means
    /// playback is already over and cancel is moot.
    pub fn cancel(self) -> std::result::Result<(), &'static str> {
        self.cancel_tx
            .send(())
            .map_err(|_| "playback already ended")
    }
}

#[derive(Clone, Debug)]
pub struct SignatureHeaders {
    pub signature: String,
    pub signature_input: String,
    pub signature_key: Option<Jwk>,
    pub signature_agent: Option<Jwk>,
}

/// Adapter-native event surface. rvoip-core normalizes these into the
/// `events::Event` vocabulary; consumers wanting protocol-native access can
/// subscribe directly to the adapter.
#[derive(Clone, Debug)]
pub enum AdapterEvent {
    InboundConnection {
        connection: Connection,
    },
    Connected {
        connection_id: ConnectionId,
    },
    /// Per-Connection auth completion. UCTP-family adapters emit this
    /// immediately after `InboundConnection` once they've matched the
    /// peer's auth handshake (CONVERSATION_PROTOCOL.md §5.1
    /// `auth.hello → auth.response → auth.session`) to the just-created
    /// Connection. Carries the server-issued `identity_id` and the
    /// peer's `participant_id`, plus the negotiated assurance gradient
    /// (plan §7 G1 / A1 + A3). SIP / WebRTC adapters that don't run a
    /// UCTP-style handshake never emit this variant; consumers should
    /// treat its absence as "auth model not applicable" rather than
    /// "auth failed".
    Authenticated {
        connection_id: ConnectionId,
        identity_id: String,
        participant_id: String,
        assurance: crate::identity::IdentityAssurance,
    },
    Ended {
        connection_id: ConnectionId,
        reason: EndReason,
    },
    Failed {
        connection_id: ConnectionId,
        detail: String,
    },
    /// DTMF digits decoded from an inbound `connection.dtmf` envelope
    /// (UCTP-family adapters) or RTP RFC 2833 event (SIP). The
    /// orchestrator translates this into [`crate::events::Event::DtmfReceived`].
    /// Plan C2.
    Dtmf {
        connection_id: ConnectionId,
        digits: String,
        duration_ms: u32,
    },
    /// Per-Stream media-quality snapshot the peer or adapter reported
    /// (UCTP-family: from a `connection.quality` envelope; SIP: from
    /// RTCP receiver reports). The orchestrator translates this into
    /// [`crate::events::Event::MediaQuality`]. Plan C2.
    Quality {
        connection_id: ConnectionId,
        snapshot: crate::stream::QualitySnapshot,
    },
    Native {
        kind: &'static str,
        detail: String,
    },
}

/// The cross-transport adapter contract. Every transport-specific crate
/// (rvoip-sip, rvoip-webrtc, rvoip-quic, rvoip-webtransport, rvoip-websocket)
/// implements this so the [`crate::Orchestrator`] can dispatch generically.
#[async_trait::async_trait]
pub trait ConnectionAdapter: Send + Sync {
    fn transport(&self) -> Transport;
    fn kind(&self) -> AdapterKind;

    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle>;
    async fn accept(&self, conn: ConnectionId) -> Result<()>;
    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> Result<()>;
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> Result<()>;
    async fn hold(&self, conn: ConnectionId) -> Result<()>;
    async fn resume(&self, conn: ConnectionId) -> Result<()>;
    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> Result<()>;

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
    async fn mute(
        &self,
        _conn: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::mute"))
    }
    async fn unmute(
        &self,
        _conn: ConnectionId,
        _direction: MuteDirection,
    ) -> Result<()> {
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

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent>;
    fn capabilities(&self) -> CapabilityDescriptor;

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        signature: SignatureHeaders,
    ) -> Result<IdentityAssurance>;
}
