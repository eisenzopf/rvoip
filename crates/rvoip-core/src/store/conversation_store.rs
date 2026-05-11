use crate::conversation::Conversation;
use crate::error::Result;
use crate::ids::{ConversationId, TenantId};
use dashmap::DashMap;
use std::sync::Arc;

/// Plug-in persistence for [`Conversation`]. Default impl is in-memory.
#[async_trait::async_trait]
pub trait ConversationStore: Send + Sync {
    async fn put(&self, conversation: Conversation) -> Result<()>;
    async fn get(&self, id: &ConversationId) -> Result<Option<Conversation>>;
    async fn delete(&self, id: &ConversationId) -> Result<()>;
    async fn list_for_tenant(&self, tenant: &TenantId) -> Result<Vec<Conversation>>;
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
