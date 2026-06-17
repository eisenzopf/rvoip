//! # rvoip-identity
//!
//! Identity-backend implementations for [`rvoip_core::IdentityProvider`].
//! Per `rvoip-core/INTERFACE_DESIGN.md` §2.1 the trait surface lives
//! in `rvoip-core::identity`; this crate provides the runtime
//! backends.
//!
//! v1 ships only `BearerProvider` (an in-memory bearer token table).
//! Production OAuth 2.1 + DPoP, OIDC, SIP Digest, Passkey/WebAuthn,
//! SCIM/SAML/LDAP, and AAuth pieces live in `rvoip-auth-core`,
//! `rvoip-users-core`, and dedicated extension crates.

use async_trait::async_trait;
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::identity::{
    Credential, Device, Identity, IdentityAssurance, IdentityKind, IdentityProvider,
    ReachabilityChange, ReachabilityHint,
};
use rvoip_core::ids::IdentityId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// v1 minimal provider: a static `bearer -> identity` table. Intended
/// for dev/test; production backends replace it.
pub struct BearerProvider {
    table: Arc<Mutex<HashMap<String, (IdentityId, IdentityAssurance)>>>,
}

impl BearerProvider {
    pub fn new() -> Self {
        Self {
            table: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn insert(
        &self,
        bearer: impl Into<String>,
        id: IdentityId,
        assurance: IdentityAssurance,
    ) {
        self.table
            .lock()
            .await
            .insert(bearer.into(), (id, assurance));
    }
}

impl Default for BearerProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdentityProvider for BearerProvider {
    async fn resolve(&self, identity_ref: &str) -> Result<Identity> {
        Ok(Identity {
            id: IdentityId::from_string(identity_ref.to_string()),
            display_name: None,
            kind: IdentityKind::Human,
            external_refs: HashMap::new(),
            signing_keys: Vec::new(),
            assurance: IdentityAssurance::Anonymous,
        })
    }
    async fn devices(&self, _id: IdentityId) -> Result<Vec<Device>> {
        Ok(Vec::new())
    }
    async fn reachable_via(&self, _id: IdentityId) -> Result<Vec<ReachabilityHint>> {
        Ok(Vec::new())
    }
    async fn authenticate(
        &self,
        credential: Credential,
    ) -> Result<(IdentityId, IdentityAssurance)> {
        let Credential::Bearer(token) = credential else {
            return Err(RvoipError::AdmissionRejected(
                "BearerProvider only supports Credential::Bearer",
            ));
        };
        self.table
            .lock()
            .await
            .get(&token)
            .cloned()
            .ok_or(RvoipError::AdmissionRejected("unknown bearer token"))
    }
    async fn assurance_level(&self, _id: IdentityId) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
    fn subscribe_reachability(&self) -> mpsc::Receiver<ReachabilityChange> {
        // No reachability source in the v1 in-memory bearer backend.
        let (_tx, rx) = mpsc::channel(1);
        rx
    }
}
