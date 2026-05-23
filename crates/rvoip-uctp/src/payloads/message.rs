//! Message envelope payloads per CONVERSATION_PROTOCOL.md §9.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `message.send` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageSend {
    pub msg_id: String,
    pub from: String,
    /// `["part_..."]` for specific recipients, or `"all"`.
    pub to: serde_json::Value,
    pub content_type: String,
    /// String for `text/*` and `application/json`; base64 for binary;
    /// URL reference for large attachments.
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Threading: reply to a prior message in the same Conversation.
    #[serde(default)]
    pub in_reply_to_msg: Option<String>,
}

/// `message.delivered` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageDelivered {
    pub msg_id: String,
    pub to_participant: String,
    pub delivered_at: DateTime<Utc>,
    #[serde(default)]
    pub via_connection: Option<String>,
}

/// `message.read` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageRead {
    pub msg_id: String,
    pub by_participant: String,
    pub read_at: DateTime<Utc>,
}

/// `message.history` (C→S) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageHistory {
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub since_msg_id: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_attachments: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    #[serde(default)]
    pub url: Option<String>,
    pub size_bytes: u64,
}
