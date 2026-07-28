use crate::ids::{ConversationId, MessageId, SessionId, TenantId};
use crate::participant::Participant;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

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
        ConversationPolicy::Ephemeral {
            idle_close_secs: 30,
        }
    }
}

#[derive(Clone)]
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
    /// Bumped on every lifecycle event the Conversation owns (Session
    /// start/end, Participant join/leave, Message send). Drives the
    /// Ephemeral-policy idle-close timer landing in P10; populated now
    /// so call sites don't churn when the driver is wired up.
    pub last_activity_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

impl fmt::Debug for Conversation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Conversation")
            .field("state", &self.state)
            .field("policy", &self.policy)
            .field("participant_count", &self.participants.len())
            .field("session_count", &self.sessions.len())
            .field("message_count", &self.messages.len())
            .field("opened_at", &self.opened_at)
            .field("closed_at", &self.closed_at)
            .field("last_activity_at", &self.last_activity_at)
            .field("metadata_count", &self.metadata.len())
            .finish()
    }
}
