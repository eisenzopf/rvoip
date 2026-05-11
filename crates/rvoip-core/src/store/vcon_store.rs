use crate::error::Result;
use crate::ids::{SessionId, TenantId};
use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;

/// Reference to a stored vCon.
#[derive(Clone, Debug)]
pub struct VconHandle {
    pub url: String,
    pub content_hash: String,
}

/// Plug-in persistence for finalized vCons. Step-1 skeleton — production
/// signing/encryption lands in the future `rvoip-vcon` crate.
#[async_trait::async_trait]
pub trait VconStore: Send + Sync {
    async fn put(
        &self,
        tenant_id: &TenantId,
        session_id: &SessionId,
        vcon_jws: Bytes,
    ) -> Result<VconHandle>;

    async fn get(&self, handle: &VconHandle) -> Result<Option<Bytes>>;

    async fn list_for_session(&self, session_id: &SessionId) -> Result<Vec<VconHandle>>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryVconStore {
    inner: Arc<DashMap<String, Bytes>>,
}

impl MemoryVconStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl VconStore for MemoryVconStore {
    async fn put(
        &self,
        _tenant_id: &TenantId,
        session_id: &SessionId,
        vcon_jws: Bytes,
    ) -> Result<VconHandle> {
        let url = format!("memory:vcon/{}", session_id);
        self.inner.insert(url.clone(), vcon_jws.clone());
        Ok(VconHandle {
            url,
            content_hash: format!("len={}", vcon_jws.len()),
        })
    }

    async fn get(&self, handle: &VconHandle) -> Result<Option<Bytes>> {
        Ok(self.inner.get(&handle.url).map(|e| e.value().clone()))
    }

    async fn list_for_session(&self, _session_id: &SessionId) -> Result<Vec<VconHandle>> {
        Ok(Vec::new())
    }
}
