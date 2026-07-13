use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::ids::{ConnectionId, ParticipantId, SessionId};
use crate::stream::MediaStreamHandle;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Transport {
    Quic,
    WebTransport,
    WebSocket,
    Sip,
    WebRtc,
    /// Amazon Connect (WebRTC interop via the `StartWebRTCContact` API +
    /// Amazon Chime SDK media). Like [`Transport::WebRtc`] this is an
    /// `AdapterKind::Interop` gateway to a foreign protocol.
    AmazonConnect,
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
#[derive(Clone)]
pub struct TransportHandle(pub Arc<dyn std::any::Any + Send + Sync>);

impl fmt::Debug for TransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransportHandle { present: true }")
    }
}

#[derive(Clone)]
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

impl fmt::Debug for Connection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("id", &self.id)
            .field("session_id", &self.session_id)
            .field("participant_id", &self.participant_id)
            .field("transport", &self.transport)
            .field("direction", &self.direction)
            .field("state", &self.state)
            .field("capabilities", &self.capabilities)
            .field("negotiated_codecs", &self.negotiated_codecs)
            .field("stream_count", &self.streams.len())
            .field("messaging_enabled", &self.messaging_enabled)
            .field("opened_at", &self.opened_at)
            .field("closed_at", &self.closed_at)
            .finish()
    }
}
