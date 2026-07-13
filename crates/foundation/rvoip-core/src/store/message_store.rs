//! P4 — message persistence and pagination for Conversation history.
//!
//! `MessageStore` is the trait consumers plug their durable store
//! against (Postgres, Redis, etc.). The in-memory `MemoryMessageStore`
//! is the v1 default for dev/tests.

use crate::error::Result;
use crate::ids::{ConversationId, MessageId, ParticipantId};
use crate::message::{ContentType, Message};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Filter shape for `list_messages`. All fields optional; absent
/// means "don't constrain".
#[derive(Clone, Debug, Default)]
pub struct MessageFilter {
    pub from_participant: Option<ParticipantId>,
    pub content_types: Option<Vec<ContentType>>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub page_size: Option<usize>,
}

/// Opaque cursor. Today: byte offset into the per-Conversation log.
/// Wire form is a JSON-encoded number so clients can round-trip it
/// without parsing.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PageCursor {
    pub offset: usize,
}

pub struct MessagePage {
    pub messages: Vec<Message>,
    pub next: Option<PageCursor>,
}

#[async_trait::async_trait]
pub trait MessageStore: Send + Sync {
    async fn put(&self, message: Message) -> Result<()>;

    async fn get(&self, id: &MessageId) -> Result<Option<Message>>;

    async fn list(
        &self,
        conversation_id: &ConversationId,
        filter: MessageFilter,
        cursor: Option<PageCursor>,
    ) -> Result<MessagePage>;

    /// Record a read receipt; the next `list` call should reflect it
    /// (consumer-defined: by side-band map, or by amending the
    /// Message itself). For the in-memory impl we keep a side-band
    /// `read_by` map keyed by message_id → Vec<participant_id>.
    async fn mark_read(&self, id: &MessageId, by: &ParticipantId) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct MemoryMessageStore {
    /// Per-Conversation log, kept in insertion order.
    log: Arc<DashMap<ConversationId, Vec<Message>>>,
    /// Side-band read receipts.
    read_by: Arc<DashMap<MessageId, Vec<ParticipantId>>>,
}

impl fmt::Debug for MemoryMessageStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message_count = self
            .log
            .iter()
            .map(|entry| entry.value().len())
            .sum::<usize>();
        let receipt_count = self
            .read_by
            .iter()
            .map(|entry| entry.value().len())
            .sum::<usize>();
        formatter
            .debug_struct("MemoryMessageStore")
            .field("conversation_count", &self.log.len())
            .field("message_count", &message_count)
            .field("receipt_count", &receipt_count)
            .finish()
    }
}

impl MemoryMessageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_receipts(&self, id: &MessageId) -> Vec<ParticipantId> {
        self.read_by
            .get(id)
            .map(|e| e.value().clone())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl MessageStore for MemoryMessageStore {
    async fn put(&self, message: Message) -> Result<()> {
        let cid = message.conversation_id.clone();
        self.log.entry(cid).or_default().push(message);
        Ok(())
    }

    async fn get(&self, id: &MessageId) -> Result<Option<Message>> {
        for entry in self.log.iter() {
            if let Some(m) = entry.value().iter().find(|m| &m.id == id) {
                return Ok(Some(m.clone()));
            }
        }
        Ok(None)
    }

    async fn list(
        &self,
        conversation_id: &ConversationId,
        filter: MessageFilter,
        cursor: Option<PageCursor>,
    ) -> Result<MessagePage> {
        let log = self
            .log
            .get(conversation_id)
            .map(|e| e.value().clone())
            .unwrap_or_default();
        let start = cursor.map(|c| c.offset).unwrap_or(0);
        let limit = filter.page_size.unwrap_or(50);

        let filtered: Vec<Message> = log
            .into_iter()
            .skip(start)
            .filter(|m| {
                filter
                    .from_participant
                    .as_ref()
                    .map_or(true, |p| &m.from_participant == p)
                    && filter
                        .content_types
                        .as_ref()
                        .map_or(true, |cts| cts.contains(&m.content_type))
                    && filter.since.map_or(true, |t| m.timestamp >= t)
                    && filter.until.map_or(true, |t| m.timestamp <= t)
            })
            .take(limit)
            .collect();

        let next = if filtered.len() == limit {
            Some(PageCursor {
                offset: start + limit,
            })
        } else {
            None
        };

        Ok(MessagePage {
            messages: filtered,
            next,
        })
    }

    async fn mark_read(&self, id: &MessageId, by: &ParticipantId) -> Result<()> {
        let mut e = self.read_by.entry(id.clone()).or_default();
        if !e.contains(by) {
            e.push(by.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::connection::Direction;
    use crate::message::{MessageOrigin, MessageRecipients};

    #[tokio::test]
    async fn memory_message_store_debug_is_aggregate_only() {
        const CANARY: &str = "message-store-canary\r\nAuthorization: exposed";
        let store = MemoryMessageStore::new();
        store
            .put(Message {
                id: MessageId::from_string(CANARY),
                conversation_id: ConversationId::from_string(CANARY),
                origin: MessageOrigin::System,
                from_participant: ParticipantId::from_string(CANARY),
                to: MessageRecipients::All,
                direction: Direction::Inbound,
                content_type: ContentType::Attachment(CANARY.into()),
                body: bytes::Bytes::from_static(b"message-store-canary\r\nAuthorization: exposed"),
                attachments: Vec::new(),
                in_reply_to: None,
                timestamp: Utc::now(),
            })
            .await
            .unwrap();
        let debug = format!("{store:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("conversation_count: 1"));
        assert!(debug.contains("message_count: 1"));
    }
}
