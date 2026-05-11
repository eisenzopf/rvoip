use crate::connection::Direction;
use crate::ids::{ConnectionId, ConversationId, MessageId, ParticipantId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub enum MessageOrigin {
    Connection(ConnectionId),
    System,
    Ai(ParticipantId),
}

#[derive(Clone, Debug)]
pub enum MessageRecipients {
    All,
    Participants(Vec<ParticipantId>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ContentType {
    Text,
    Json,
    Binary,
    Image,
    Audio,
    Attachment(String),
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub url: String,
    pub content_type: ContentType,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub origin: MessageOrigin,
    pub from_participant: ParticipantId,
    pub to: MessageRecipients,
    pub direction: Direction,
    pub content_type: ContentType,
    pub body: Bytes,
    pub attachments: Vec<Attachment>,
    pub in_reply_to: Option<MessageId>,
    pub timestamp: DateTime<Utc>,
}
