//! Identity-provider integration seams for registrar provisioning.

use crate::error::{RegistrarError, Result};
use crate::types::AddressOfRecord;
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub aor: AddressOfRecord,
    pub external_id: String,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub enabled: bool,
    pub attributes: HashMap<String, String>,
}

impl ExternalIdentity {
    pub fn enabled(aor: AddressOfRecord, external_id: impl Into<String>) -> Self {
        Self {
            aor,
            external_id: external_id.into(),
            display_name: None,
            groups: Vec::new(),
            enabled: true,
            attributes: HashMap::new(),
        }
    }
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn resolve_identity(&self, aor: &AddressOfRecord) -> Result<Option<ExternalIdentity>>;

    async fn list_identities(&self) -> Result<Vec<ExternalIdentity>> {
        Ok(Vec::new())
    }
}

#[async_trait]
pub trait CredentialProvider: Send + Sync {
    async fn sip_digest_secret(&self, aor: &AddressOfRecord) -> Result<Option<String>>;
}

#[derive(Clone)]
pub struct IdentitySyncService {
    provider: Arc<dyn IdentityProvider>,
}

impl IdentitySyncService {
    pub fn new(provider: Arc<dyn IdentityProvider>) -> Self {
        Self { provider }
    }

    pub async fn fetch_identities(&self) -> Result<Vec<ExternalIdentity>> {
        self.provider.list_identities().await
    }
}

#[derive(Default)]
pub struct InMemoryIdentityProvider {
    identities: DashMap<String, ExternalIdentity>,
    credentials: DashMap<String, String>,
}

impl InMemoryIdentityProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_identity(&self, identity: ExternalIdentity) {
        self.identities.insert(identity.aor.to_string(), identity);
    }

    pub fn disable_identity(&self, aor: &AddressOfRecord) -> Result<()> {
        let Some(mut identity) = self.identities.get_mut(aor.as_str()) else {
            return Err(RegistrarError::UserNotFound(aor.to_string()));
        };
        identity.enabled = false;
        Ok(())
    }

    pub fn set_digest_secret(&self, aor: &AddressOfRecord, secret: impl Into<String>) {
        self.credentials.insert(aor.to_string(), secret.into());
    }
}

#[async_trait]
impl IdentityProvider for InMemoryIdentityProvider {
    async fn resolve_identity(&self, aor: &AddressOfRecord) -> Result<Option<ExternalIdentity>> {
        Ok(self
            .identities
            .get(aor.as_str())
            .map(|identity| identity.clone()))
    }

    async fn list_identities(&self) -> Result<Vec<ExternalIdentity>> {
        Ok(self
            .identities
            .iter()
            .map(|identity| identity.value().clone())
            .collect())
    }
}

#[async_trait]
impl CredentialProvider for InMemoryIdentityProvider {
    async fn sip_digest_secret(&self, aor: &AddressOfRecord) -> Result<Option<String>> {
        Ok(self
            .credentials
            .get(aor.as_str())
            .map(|secret| secret.value().clone()))
    }
}
