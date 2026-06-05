//! Identity surface — the trait + the rich structs that reference
//! rvoip-core's `Result` type live here; the *pure data types*
//! (`Jwk`, `IdentityKind`, `DeviceKind`, `IdentityAssurance`,
//! `CredentialKind`, `Credential`) moved to `rvoip-core-traits` in
//! V2.A.1 and are re-exported below so `use rvoip_core::identity::*`
//! call sites work unchanged.

use crate::error::Result;
use crate::ids::{DeviceId, IdentityId};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio::sync::mpsc;

// V2.A — pure-data types now live in rvoip-core-traits. Re-export so
// downstream `use rvoip_core::identity::IdentityAssurance` etc. keep
// working unchanged.
pub use rvoip_core_traits::identity::{
    Credential, CredentialKind, DeviceKind, IdentityAssurance, IdentityKind, Jwk,
};

#[derive(Clone, Debug)]
pub struct Identity {
    pub id: IdentityId,
    pub display_name: Option<String>,
    pub kind: IdentityKind,
    pub external_refs: HashMap<String, String>,
    pub signing_keys: Vec<Jwk>,
    pub assurance: IdentityAssurance,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: DeviceId,
    pub identity_id: IdentityId,
    pub kind: DeviceKind,
    pub platform: String,
    pub registered_at: DateTime<Utc>,
    pub device_signing_key: Option<Jwk>,
}

#[derive(Clone, Debug)]
pub struct ReachabilityHint {
    pub transport: crate::connection::Transport,
    pub address: String,
    pub device_id: DeviceId,
    pub priority: u16,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct ReachabilityChange {
    pub identity_id: IdentityId,
    pub kind: ReachabilityChangeKind,
    pub hint: ReachabilityHint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReachabilityChangeKind {
    Added,
    Removed,
    Updated,
    Expired,
}

/// DTLS-SRTP fingerprint binding payload (RFC 8122 §5).
#[derive(Clone, Debug)]
pub struct DtlsFingerprint {
    pub algorithm: String,
    pub value: String,
}

/// SignatureHeaders re-exported here for the trait surface — actual
/// shape lives in [`crate::adapter::SignatureHeaders`].
pub use crate::adapter::SignatureHeaders;

/// Plug-in identity backend. P7 completes the trait surface per
/// INTERFACE_DESIGN.md §8 (9 methods). Production impls live in
/// `rvoip-identity`; the 3 default-`NotImplemented` methods let
/// existing in-tree no-op impls compile unchanged.
#[async_trait::async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn resolve(&self, identity_ref: &str) -> Result<Identity>;
    async fn devices(&self, identity_id: IdentityId) -> Result<Vec<Device>>;
    async fn reachable_via(&self, identity_id: IdentityId) -> Result<Vec<ReachabilityHint>>;
    async fn authenticate(&self, credential: Credential)
        -> Result<(IdentityId, IdentityAssurance)>;
    async fn assurance_level(&self, id: IdentityId) -> Result<IdentityAssurance>;
    fn subscribe_reachability(&self) -> mpsc::Receiver<ReachabilityChange>;

    /// P7 — register an agent's public signing key. Default
    /// `NotImplemented`.
    async fn register_agent_key(&self, _id: IdentityId, _key: Jwk) -> Result<()> {
        Err(crate::error::RvoipError::NotImplemented(
            "IdentityProvider::register_agent_key",
        ))
    }

    /// P7 — verify an RFC 9421 signature against the identity's
    /// registered keys.
    async fn verify_signature(
        &self,
        _id: IdentityId,
        _sig: SignatureHeaders,
        _body: &[u8],
    ) -> Result<IdentityAssurance> {
        Err(crate::error::RvoipError::NotImplemented(
            "IdentityProvider::verify_signature",
        ))
    }

    /// P7 — derive (or look up) the DTLS-SRTP fingerprint bound to an
    /// Identity. None when the identity has no fingerprint binding.
    async fn derive_dtls_fingerprint(&self, _id: IdentityId) -> Result<Option<DtlsFingerprint>> {
        Err(crate::error::RvoipError::NotImplemented(
            "IdentityProvider::derive_dtls_fingerprint",
        ))
    }
}
