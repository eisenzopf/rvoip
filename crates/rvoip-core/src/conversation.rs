use crate::ids::{ConversationId, MessageId, SessionId, TenantId};
use crate::participant::Participant;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConversationState {
    Open,
    Closed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ConversationPolicy {
    Ephemeral { idle_close_secs: u64 },
    Persistent,
}

impl Default for ConversationPolicy {
    fn default() -> Self {
        ConversationPolicy::Ephemeral { idle_close_secs: 30 }
    }
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: ConversationId,
    pub tenant_id: TenantId,
    pub state: ConversationState,
    pub policy: ConversationPolicy,
    pub participants: Vec<Participant>,
    pub sessions: Vec<SessionId>,
    pub messages: Vec<MessageId>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub metadata: HashMap<String, String>,
}
