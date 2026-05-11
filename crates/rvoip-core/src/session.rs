use crate::adapter::EndReason;
use crate::capability::CapabilityIntersection;
use crate::ids::{ConnectionId, ConversationId, ParticipantId, SessionId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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

#[derive(Clone, Debug)]
pub struct ConnectionRef {
    pub id: ConnectionId,
    pub participant_id: ParticipantId,
}

#[derive(Clone, Debug)]
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
