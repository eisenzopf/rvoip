use crate::error::Result;
use crate::ids::{SessionId, TenantId};
use bytes::Bytes;
use dashmap::DashMap;
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;

/// Reference to a stored vCon.
#[derive(Clone)]
pub struct VconHandle {
    pub url: String,
    pub content_hash: String,
}

impl fmt::Debug for VconHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconHandle")
            .field("url_present", &!self.url.is_empty())
            .field("url_bytes", &self.url.len())
            .field("content_hash_present", &!self.content_hash.is_empty())
            .field("content_hash_bytes", &self.content_hash.len())
            .finish()
    }
}

/// Plug-in persistence for finalized vCons. Production signing /
/// encryption lives in `rvoip-vcon`; the v1 in-process pathway accepts
/// already-built bytes here.
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

/// In-memory store keyed by `memory:vcon/<session_id>/<seq>`. P3
/// promotes this from a mock (returning `len=N` for content_hash) to a
/// real `sha256:<hex>` content hash so callers can verify the stored
/// bytes match the returned handle.
#[derive(Clone, Default)]
pub struct MemoryVconStore {
    inner: Arc<DashMap<String, Bytes>>,
    /// Per-session counter so multiple `put`s for the same Session
    /// generate distinct handles (vs the old design that overwrote).
    seq: Arc<DashMap<String, u64>>,
}

impl fmt::Debug for MemoryVconStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryVconStore")
            .field("object_count", &self.inner.len())
            .field("session_count", &self.seq.len())
            .finish()
    }
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
        let sid = session_id.to_string();
        let n = {
            let mut entry = self.seq.entry(sid.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        let url = format!("memory:vcon/{}/{}", sid, n);
        let mut hasher = Sha256::new();
        hasher.update(&vcon_jws);
        let digest = hasher.finalize();
        let content_hash = format!("sha256:{}", hex::encode(digest));
        self.inner.insert(url.clone(), vcon_jws);
        Ok(VconHandle { url, content_hash })
    }

    async fn get(&self, handle: &VconHandle) -> Result<Option<Bytes>> {
        Ok(self.inner.get(&handle.url).map(|e| e.value().clone()))
    }

    async fn list_for_session(&self, session_id: &SessionId) -> Result<Vec<VconHandle>> {
        // P3 — return all `memory:vcon/<sid>/*` entries, freshly
        // hashing for the content_hash field. Cheap because the
        // in-memory store is dev-only.
        let prefix = format!("memory:vcon/{}/", session_id);
        let mut out = Vec::new();
        for entry in self.inner.iter() {
            if entry.key().starts_with(&prefix) {
                let mut hasher = Sha256::new();
                hasher.update(entry.value());
                let digest = hasher.finalize();
                out.push(VconHandle {
                    url: entry.key().clone(),
                    content_hash: format!("sha256:{}", hex::encode(digest)),
                });
            }
        }
        Ok(out)
    }
}

// Local hex helper so we don't pull a tiny extra dep just for one
// encode. Lower-case, fixed-width per byte.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let bytes = bytes.as_ref();
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
        s
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[tokio::test]
    async fn vcon_store_debug_never_renders_urls_hashes_keys_or_payloads() {
        const CANARY: &str = "vcon-store-canary\r\nAuthorization: exposed";
        let handle = VconHandle {
            url: CANARY.into(),
            content_hash: CANARY.into(),
        };
        assert!(!format!("{handle:?}").contains(CANARY));

        let store = MemoryVconStore::new();
        store
            .put(
                &TenantId::from_string(CANARY),
                &SessionId::from_string(CANARY),
                Bytes::from_static(b"vcon-store-canary\r\nAuthorization: exposed"),
            )
            .await
            .unwrap();
        let debug = format!("{store:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("object_count: 1"));
    }
}
