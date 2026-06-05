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

#[derive(Clone, Debug, Default)]
pub struct MemoryMessageStore {
    /// Per-Conversation log, kept in insertion order.
    log: Arc<DashMap<ConversationId, Vec<Message>>>,
    /// Side-band read receipts.
    read_by: Arc<DashMap<MessageId, Vec<ParticipantId>>>,
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
