//! Conversation envelope payloads per CONVERSATION_PROTOCOL.md §7.1.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! metadata_only_debug {
    ($($type:ty),+ $(,)?) => {
        $(
            impl fmt::Debug for $type {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(stringify!($type))
                }
            }
        )+
    };
}

/// `conversation.create` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationCreate {
    pub tenant_id: String,
    pub policy: ConversationPolicy,
    #[serde(default)]
    pub idle_close_secs: Option<u32>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub initial_participants: Vec<InitialParticipant>,
}

/// `conversation.opened` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationOpened {
    pub tenant_id: String,
    pub policy: ConversationPolicy,
    #[serde(default)]
    pub idle_close_secs: Option<u32>,
    pub participants: Vec<Participant>,
    pub opened_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// `conversation.closed` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationClosed {
    pub reason_code: u16,
    pub reason: String,
    pub closed_at: DateTime<Utc>,
}

/// `conversation.list` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationList {
    #[serde(default)]
    pub filter: serde_json::Value,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationPolicy {
    Ephemeral,
    Persistent,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InitialParticipant {
    pub identity_id: String,
    pub role: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Participant {
    pub participant_id: String,
    pub identity_id: String,
    pub kind: String,
    pub role: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

metadata_only_debug!(
    ConversationCreate,
    ConversationOpened,
    ConversationClosed,
    ConversationList,
    InitialParticipant,
    Participant,
);
