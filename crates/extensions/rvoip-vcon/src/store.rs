//! [`VconStore`] trait + [`MemoryVconStore`].
//!
//! Pluggable persistence keyed by `uuid::Uuid` (the same uuid that
//! `rvoip_core::vcon::VconRef::Local { uuid }` carries). Production
//! deployments swap in disk / S3 / database stores by implementing
//! the trait; tests and the v0 demo use the in-memory one.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use thiserror::Error;
use uuid::Uuid;

use crate::{content_hash, types::Vcon};

#[derive(Debug, Error)]
pub enum VconStoreError {
    #[error("vcon not found: {0}")]
    NotFound(Uuid),

    #[error("vcon store backend error: {0}")]
    Backend(String),
}

/// Pluggable vCon persistence. Producers (such as core Session finalization)
/// call `put`; consumers (transcription, analytics,
/// recording-fetch envelopes) call `get`.
///
/// Implementations are expected to be cheap to clone (Arc-wrapped
/// internal state) so the orchestrator can hand a clone to every
/// adapter that needs to emit vCons.
#[async_trait]
pub trait VconStore: Send + Sync {
    /// Persist `vcon`, return its uuid (== `vcon.uuid`). Implementations
    /// MAY return an error if the uuid is already present (replacement
    /// is opt-in via [`Self::put_overwrite`]).
    async fn put(&self, vcon: Vcon) -> Result<Uuid, VconStoreError>;

    /// Variant that explicitly allows overwriting an existing entry.
    /// Default impl just calls `put` — backends that distinguish
    /// override.
    async fn put_overwrite(&self, vcon: Vcon) -> Result<Uuid, VconStoreError> {
        self.put(vcon).await
    }

    /// Resolve a uuid to its vCon. Returns
    /// [`VconStoreError::NotFound`] if the uuid isn't present.
    async fn get(&self, uuid: &Uuid) -> Result<Vcon, VconStoreError>;

    /// Return the conformant SHA-512 content hash calculated from the
    /// serialized bytes accepted by the store.
    async fn content_hash(&self, uuid: &Uuid) -> Result<String, VconStoreError>;

    /// Delete an entry. Returns Ok even if the uuid wasn't present
    /// (idempotent delete).
    async fn delete(&self, uuid: &Uuid) -> Result<(), VconStoreError>;

    /// Number of stored vCons. Diagnostic / metrics path; backends
    /// without efficient size may return `None`.
    async fn len(&self) -> Option<usize> {
        None
    }
}

/// In-memory, DashMap-backed [`VconStore`]. The v0 default — production
/// deployments swap in disk / S3 / RDBMS variants.
#[derive(Clone, Default)]
pub struct MemoryVconStore {
    inner: Arc<DashMap<Uuid, StoredVcon>>,
}

#[derive(Clone)]
struct StoredVcon {
    document: Vcon,
    content_hash: String,
}

impl MemoryVconStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn prepare(vcon: Vcon) -> Result<(Uuid, StoredVcon), VconStoreError> {
        vcon.validate()
            .map_err(|error| VconStoreError::Backend(error.to_string()))?;
        let uuid = vcon.uuid;
        let bytes = serde_json::to_vec(&vcon)
            .map_err(|error| VconStoreError::Backend(format!("serialize vcon: {error}")))?;
        Ok((
            uuid,
            StoredVcon {
                document: vcon,
                content_hash: content_hash(&bytes),
            },
        ))
    }
}

#[async_trait]
impl VconStore for MemoryVconStore {
    async fn put(&self, vcon: Vcon) -> Result<Uuid, VconStoreError> {
        let (uuid, stored) = Self::prepare(vcon)?;
        // Refuse silent overwrite — callers wanting it use put_overwrite.
        match self.inner.entry(uuid) {
            Entry::Occupied(_) => Err(VconStoreError::Backend(format!(
                "uuid already exists: {uuid} (use put_overwrite to replace)"
            ))),
            Entry::Vacant(entry) => {
                entry.insert(stored);
                Ok(uuid)
            }
        }
    }

    async fn put_overwrite(&self, vcon: Vcon) -> Result<Uuid, VconStoreError> {
        let (uuid, stored) = Self::prepare(vcon)?;
        self.inner.insert(uuid, stored);
        Ok(uuid)
    }

    async fn get(&self, uuid: &Uuid) -> Result<Vcon, VconStoreError> {
        self.inner
            .get(uuid)
            .map(|entry| entry.document.clone())
            .ok_or(VconStoreError::NotFound(*uuid))
    }

    async fn content_hash(&self, uuid: &Uuid) -> Result<String, VconStoreError> {
        self.inner
            .get(uuid)
            .map(|entry| entry.content_hash.clone())
            .ok_or(VconStoreError::NotFound(*uuid))
    }

    async fn delete(&self, uuid: &Uuid) -> Result<(), VconStoreError> {
        self.inner.remove(uuid);
        Ok(())
    }

    async fn len(&self) -> Option<usize> {
        Some(self.inner.len())
    }
}
