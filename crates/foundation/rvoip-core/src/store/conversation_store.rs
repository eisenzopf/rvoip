use crate::conversation::{Conversation, ConversationState};
use crate::error::Result;
use crate::ids::{ConversationId, IdentityId, ParticipantId, TenantId};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;

/// P10 — richer filter shape per PRD §10. Every field optional;
/// absent means "don't constrain".
#[derive(Clone, Debug, Default)]
pub struct ConversationFilter {
    pub tenant: Option<TenantId>,
    pub participant: Option<ParticipantId>,
    pub identity: Option<IdentityId>,
    pub state: Option<ConversationState>,
    pub opened_since: Option<DateTime<Utc>>,
    pub opened_until: Option<DateTime<Utc>>,
}

/// Plug-in persistence for [`Conversation`]. Default impl is in-memory.
#[async_trait::async_trait]
pub trait ConversationStore: Send + Sync {
    async fn put(&self, conversation: Conversation) -> Result<()>;
    async fn get(&self, id: &ConversationId) -> Result<Option<Conversation>>;
    async fn delete(&self, id: &ConversationId) -> Result<()>;
    async fn list_for_tenant(&self, tenant: &TenantId) -> Result<Vec<Conversation>>;

    /// P10 — widened query interface. Default implementation falls
    /// back to `list_for_tenant` so existing impls compile unchanged;
    /// override for index-aware backends.
    async fn list(&self, filter: ConversationFilter) -> Result<Vec<Conversation>> {
        let Some(tenant) = filter.tenant.as_ref() else {
            return Ok(Vec::new());
        };
        let all = self.list_for_tenant(tenant).await?;
        Ok(all
            .into_iter()
            .filter(|c| {
                filter.state.map_or(true, |s| c.state == s)
                    && filter.opened_since.map_or(true, |t| c.opened_at >= t)
                    && filter.opened_until.map_or(true, |t| c.opened_at <= t)
                    && filter
                        .participant
                        .as_ref()
                        .map_or(true, |p| c.participants.iter().any(|pp| &pp.id == p))
                    && filter.identity.as_ref().map_or(true, |i| {
                        c.participants
                            .iter()
                            .any(|p| p.identity_ref.as_ref() == Some(i))
                    })
            })
            .collect())
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryConversationStore {
    inner: Arc<DashMap<ConversationId, Conversation>>,
}

impl MemoryConversationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl ConversationStore for MemoryConversationStore {
    async fn put(&self, conversation: Conversation) -> Result<()> {
        self.inner.insert(conversation.id.clone(), conversation);
        Ok(())
    }

    async fn get(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        Ok(self.inner.get(id).map(|e| e.value().clone()))
    }

    async fn delete(&self, id: &ConversationId) -> Result<()> {
        self.inner.remove(id);
        Ok(())
    }

    async fn list_for_tenant(&self, tenant: &TenantId) -> Result<Vec<Conversation>> {
        Ok(self
            .inner
            .iter()
            .filter(|e| &e.value().tenant_id == tenant)
            .map(|e| e.value().clone())
            .collect())
    }
}
