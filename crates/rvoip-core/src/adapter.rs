use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::connection::{Connection, Direction, Transport};
use crate::error::Result;
use crate::identity::{IdentityAssurance, Jwk};
use crate::ids::{ConnectionId, ParticipantId, SessionId};
use crate::message::Message;
use crate::stream::MediaStream;
use std::sync::Arc;
use tokio::sync::mpsc;

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
    Ended {
        connection_id: ConnectionId,
        reason: EndReason,
    },
    Failed {
        connection_id: ConnectionId,
        detail: String,
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
    async fn send_message(&self, conn: ConnectionId, message: Message) -> Result<()>;
    async fn send_dtmf(&self, conn: ConnectionId, digits: &str, duration_ms: u32) -> Result<()>;
    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs>;

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent>;
    fn capabilities(&self) -> CapabilityDescriptor;

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        signature: SignatureHeaders,
    ) -> Result<IdentityAssurance>;
}
