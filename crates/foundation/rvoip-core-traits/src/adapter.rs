use crate::capability::CapabilityDescriptor;
use crate::connection::{Connection, Direction, Transport};
use crate::data::DataMessage;
use crate::identity::{AuthenticatedPrincipal, IdentityAssurance, Jwk};
use crate::ids::{ConnectionId, ParticipantId, PlaybackId, SessionId};
use crate::stream::QualitySnapshot;
use tokio::sync::oneshot;

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

/// Handle returned by adapter playback paths that lets callers stop an
/// in-flight playback.
#[derive(Debug)]
pub struct PlaybackHandle {
    id: PlaybackId,
    cancel_tx: oneshot::Sender<()>,
}

impl PlaybackHandle {
    /// Adapter helper: build a handle + the matching cancel receiver.
    pub fn new(id: PlaybackId) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self { id, cancel_tx: tx }, rx)
    }

    pub fn id(&self) -> &PlaybackId {
        &self.id
    }

    /// Best-effort cancellation. Returns `Err` only when the adapter's
    /// playback task already exited.
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

/// Adapter-native event surface. `rvoip-core` normalizes these into the
/// orchestration event vocabulary; consumers wanting protocol-native
/// access can subscribe directly to the adapter.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AdapterEvent {
    InboundConnection {
        connection: Connection,
    },
    Connected {
        connection_id: ConnectionId,
    },
    Authenticated {
        connection_id: ConnectionId,
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
    },
    /// Additive full-principal authentication event. The legacy
    /// `Authenticated` variant remains unchanged for source compatibility.
    PrincipalAuthenticated {
        connection_id: ConnectionId,
        participant_id: String,
        principal: AuthenticatedPrincipal,
    },
    Ended {
        connection_id: ConnectionId,
        reason: EndReason,
    },
    Failed {
        connection_id: ConnectionId,
        detail: String,
    },
    Dtmf {
        connection_id: ConnectionId,
        digits: String,
        duration_ms: u32,
    },
    Quality {
        connection_id: ConnectionId,
        snapshot: QualitySnapshot,
    },
    Message {
        connection_id: ConnectionId,
        text: String,
    },
    DataMessage {
        connection_id: ConnectionId,
        message: DataMessage,
    },
    StepUpResponse {
        connection_id: ConnectionId,
        method: String,
        credential: String,
    },
    Native {
        kind: &'static str,
        detail: String,
    },
}
