use crate::ids::{ConversationId, IdentityId, ParticipantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParticipantKind {
    Human,
    Ai,
    System,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParticipantRole {
    Customer,
    Agent,
    Supervisor,
    Observer,
    Custom(String),
}

#[derive(Clone, Debug)]
pub struct Participant {
    pub id: ParticipantId,
    pub conversation_id: ConversationId,
    pub identity_ref: Option<IdentityId>,
    pub kind: ParticipantKind,
    pub role: ParticipantRole,
    pub display_name: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
}
