use crate::error::Result;
use crate::ids::{ConversationId, SessionId, TenantId};
use bytes::Bytes;
use dashmap::DashMap;
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

/// Plug-in persistence for finalized vCons. Core's automatic pathway
/// writes unsigned canonical JSON; callers may also persist explicitly
/// signed bytes produced by `rvoip-vcon`.
#[async_trait::async_trait]
pub trait VconStore: Send + Sync {
    async fn put(
        &self,
        tenant_id: &TenantId,
        conversation_id: &ConversationId,
        session_id: &SessionId,
        vcon_body: Bytes,
    ) -> Result<VconHandle>;

    async fn get(&self, handle: &VconHandle) -> Result<Option<Bytes>>;

    async fn list_for_session(&self, session_id: &SessionId) -> Result<Vec<VconHandle>>;

    /// List vCons for every Session belonging to one Conversation.
    ///
    /// This store/index relationship replaces the reserved vCon `group`
    /// parameter; it is deliberately not serialized into the vCon itself.
    async fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<VconHandle>>;
}

#[derive(Clone)]
struct VconIndexEntry {
    conversation_id: ConversationId,
    session_id: SessionId,
}

/// In-memory store keyed by `memory:vcon/<session_id>/<seq>`. P3
/// promotes this from a mock (returning `len=N` for content_hash) to a
/// vCon `sha512-<base64url>` content hash so callers can verify the stored
/// bytes match the returned handle.
#[derive(Clone, Default)]
pub struct MemoryVconStore {
    inner: Arc<DashMap<String, Bytes>>,
    index: Arc<DashMap<String, VconIndexEntry>>,
    /// Per-session counter so multiple `put`s for the same Session
    /// generate distinct handles (vs the old design that overwrote).
    seq: Arc<DashMap<String, u64>>,
}

impl fmt::Debug for MemoryVconStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryVconStore")
            .field("object_count", &self.inner.len())
            .field("index_count", &self.index.len())
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
        conversation_id: &ConversationId,
        session_id: &SessionId,
        vcon_body: Bytes,
    ) -> Result<VconHandle> {
        let sid = session_id.to_string();
        let n = {
            let mut entry = self.seq.entry(sid.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        let url = format!("memory:vcon/{}/{}", sid, n);
        let content_hash = stored_content_hash(&vcon_body);
        self.inner.insert(url.clone(), vcon_body);
        self.index.insert(
            url.clone(),
            VconIndexEntry {
                conversation_id: conversation_id.clone(),
                session_id: session_id.clone(),
            },
        );
        Ok(VconHandle { url, content_hash })
    }

    async fn get(&self, handle: &VconHandle) -> Result<Option<Bytes>> {
        Ok(self.inner.get(&handle.url).map(|e| e.value().clone()))
    }

    async fn list_for_session(&self, session_id: &SessionId) -> Result<Vec<VconHandle>> {
        self.list_matching(|entry| &entry.session_id == session_id)
    }

    async fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<VconHandle>> {
        self.list_matching(|entry| &entry.conversation_id == conversation_id)
    }
}

impl MemoryVconStore {
    fn list_matching(&self, matches: impl Fn(&VconIndexEntry) -> bool) -> Result<Vec<VconHandle>> {
        let mut out = Vec::new();
        for indexed in self.index.iter() {
            if matches(indexed.value()) {
                let Some(body) = self.inner.get(indexed.key()) else {
                    continue;
                };
                out.push(VconHandle {
                    url: indexed.key().clone(),
                    content_hash: stored_content_hash(body.value()),
                });
            }
        }
        out.sort_by(|left, right| left.url.cmp(&right.url));
        Ok(out)
    }
}

fn stored_content_hash(bytes: impl AsRef<[u8]>) -> String {
    #[cfg(feature = "vcon")]
    {
        rvoip_vcon::content_hash(bytes)
    }

    #[cfg(not(feature = "vcon"))]
    {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use sha2::{Digest, Sha512};

        let digest = Sha512::digest(bytes.as_ref());
        format!("sha512-{}", URL_SAFE_NO_PAD.encode(digest))
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn stored_hash_uses_vcon_sha512_base64url_format() {
        assert_eq!(
            stored_content_hash(b"abc"),
            "sha512-3a81oZNherrMQXNJriBBMRLm-k6JqX6iCp7u5ktV05ohkpkqJ0_BqDa6PCOj_uu9RU1EI2Q86A4qmslPpUyknw"
        );
    }

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
                &ConversationId::from_string(CANARY),
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
