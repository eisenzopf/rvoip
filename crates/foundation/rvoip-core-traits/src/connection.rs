use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::ids::{ConnectionId, ParticipantId, SessionId};
use crate::stream::MediaStreamHandle;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Transport {
    Quic,
    WebTransport,
    WebSocket,
    Sip,
    WebRtc,
    InProcessAi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Held,
    Ending,
    Ended,
    Failed,
}

/// Opaque transport-specific handle. Resolved by the owning adapter.
#[derive(Clone, Debug)]
pub struct TransportHandle(pub Arc<dyn std::any::Any + Send + Sync>);

#[derive(Clone, Debug)]
pub struct Connection {
    pub id: ConnectionId,
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub transport: Transport,
    pub direction: Direction,
    pub state: ConnectionState,
    pub capabilities: CapabilityDescriptor,
    pub negotiated_codecs: NegotiatedCodecs,
    pub streams: Vec<MediaStreamHandle>,
    pub messaging_enabled: bool,
    pub transport_handle: TransportHandle,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}
