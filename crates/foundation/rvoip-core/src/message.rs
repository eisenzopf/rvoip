use crate::connection::Direction;
use crate::ids::{ConnectionId, ConversationId, MessageId, ParticipantId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone)]
pub enum MessageOrigin {
    Connection(ConnectionId),
    System,
    Ai(ParticipantId),
}

impl fmt::Debug for MessageOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connection(_) => "MessageOrigin::Connection",
            Self::System => "MessageOrigin::System",
            Self::Ai(_) => "MessageOrigin::Ai",
        })
    }
}

#[derive(Clone)]
pub enum MessageRecipients {
    All,
    Participants(Vec<ParticipantId>),
}

impl fmt::Debug for MessageRecipients {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => formatter.write_str("MessageRecipients::All"),
            Self::Participants(participants) => formatter
                .debug_struct("MessageRecipients::Participants")
                .field("count", &participants.len())
                .finish(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum ContentType {
    Text,
    Json,
    Binary,
    Image,
    Audio,
    Attachment(String),
}

impl fmt::Debug for ContentType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Text => "ContentType::Text",
            Self::Json => "ContentType::Json",
            Self::Binary => "ContentType::Binary",
            Self::Image => "ContentType::Image",
            Self::Audio => "ContentType::Audio",
            Self::Attachment(_) => "ContentType::Attachment",
        })
    }
}

#[derive(Clone)]
pub struct Attachment {
    pub url: String,
    pub content_type: ContentType,
    pub size_bytes: u64,
}

impl fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attachment")
            .field("url_present", &!self.url.is_empty())
            .field("content_type", &self.content_type)
            .field("size_bytes", &self.size_bytes)
            .finish()
    }
}

#[derive(Clone)]
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

impl fmt::Debug for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Message")
            .field("origin", &self.origin)
            .field("recipients", &self.to)
            .field("direction", &self.direction)
            .field("content_type", &self.content_type)
            .field("body_bytes", &self.body.len())
            .field("attachment_count", &self.attachments.len())
            .field("in_reply_to_present", &self.in_reply_to.is_some())
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn message_debug_exposes_shape_without_routing_or_payload_values() {
        let secret = "credential-canary-value";
        let message = Message {
            id: MessageId::new(),
            conversation_id: ConversationId::new(),
            origin: MessageOrigin::Connection(ConnectionId::new()),
            from_participant: ParticipantId::new(),
            to: MessageRecipients::Participants(vec![ParticipantId::new()]),
            direction: Direction::Inbound,
            content_type: ContentType::Attachment(secret.into()),
            body: Bytes::from(secret),
            attachments: vec![Attachment {
                url: format!("https://{secret}.invalid/object"),
                content_type: ContentType::Attachment(secret.into()),
                size_bytes: 23,
            }],
            in_reply_to: Some(MessageId::new()),
            timestamp: Utc::now(),
        };

        let debug = format!("{message:?}");
        assert!(!debug.contains(secret));
        assert!(debug.contains("body_bytes: 23"));
        assert!(debug.contains("attachment_count: 1"));
        assert!(debug.contains("count: 1"));
    }
}
