use crate::adapter::EndReason;
use crate::capability::CapabilityIntersection;
use crate::ids::{ConnectionId, ConversationId, ParticipantId, SessionId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionState {
    Initiating,
    Active,
    Ending,
    Ended,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionMedium {
    Voice,
    Video,
    VoiceVideo,
    ScreenShare,
    TextChat,
    Mixed,
}

#[derive(Clone)]
pub struct ConnectionRef {
    pub id: ConnectionId,
    pub participant_id: ParticipantId,
}

impl fmt::Debug for ConnectionRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectionRef")
    }
}

#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    pub conversation_id: ConversationId,
    pub state: SessionState,
    pub medium: SessionMedium,
    pub participants: HashSet<ParticipantId>,
    pub connections: HashMap<ConnectionId, ConnectionRef>,
    pub negotiated_capabilities: CapabilityIntersection,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<EndReason>,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("state", &self.state)
            .field("medium", &self.medium)
            .field("participant_count", &self.participants.len())
            .field("connection_count", &self.connections.len())
            .field("started_at", &self.started_at)
            .field("ended_at", &self.ended_at)
            .field("end_reason_present", &self.end_reason.is_some())
            .finish()
    }
}
