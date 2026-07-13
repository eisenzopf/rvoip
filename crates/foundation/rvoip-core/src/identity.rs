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
use std::fmt;
use tokio::sync::mpsc;

// V2.A — pure-data types now live in rvoip-core-traits. Re-export so
// downstream `use rvoip_core::identity::IdentityAssurance` etc. keep
// working unchanged.
pub use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, Credential, CredentialKind,
    DeviceKind, IdentityAssurance, IdentityKind, Jwk, PrincipalOwnershipKey,
};

#[derive(Clone)]
pub struct Identity {
    pub id: IdentityId,
    pub display_name: Option<String>,
    pub kind: IdentityKind,
    pub external_refs: HashMap<String, String>,
    pub signing_keys: Vec<Jwk>,
    pub assurance: IdentityAssurance,
}

impl fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("id_present", &!self.id.as_str().is_empty())
            .field("display_name_present", &self.display_name.is_some())
            .field("kind", &self.kind)
            .field("external_reference_count", &self.external_refs.len())
            .field("signing_key_count", &self.signing_keys.len())
            .field("assurance_kind", &self.assurance.kind())
            .finish()
    }
}

#[derive(Clone)]
pub struct Device {
    pub id: DeviceId,
    pub identity_id: IdentityId,
    pub kind: DeviceKind,
    pub platform: String,
    pub registered_at: DateTime<Utc>,
    pub device_signing_key: Option<Jwk>,
}

impl fmt::Debug for Device {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Device")
            .field("id_present", &!self.id.as_str().is_empty())
            .field("identity_present", &!self.identity_id.as_str().is_empty())
            .field("kind", &self.kind)
            .field("platform_present", &!self.platform.is_empty())
            .field("signing_key_present", &self.device_signing_key.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct ReachabilityHint {
    pub transport: crate::connection::Transport,
    pub address: String,
    pub device_id: DeviceId,
    pub priority: u16,
    pub expires_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for ReachabilityHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReachabilityHint")
            .field("transport", &self.transport)
            .field("address_present", &!self.address.is_empty())
            .field("device_present", &!self.device_id.as_str().is_empty())
            .field("priority", &self.priority)
            .field("expires_at_present", &self.expires_at.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct ReachabilityChange {
    pub identity_id: IdentityId,
    pub kind: ReachabilityChangeKind,
    pub hint: ReachabilityHint,
}

impl fmt::Debug for ReachabilityChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReachabilityChange")
            .field("identity_present", &!self.identity_id.as_str().is_empty())
            .field("kind", &self.kind)
            .field("hint", &self.hint)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReachabilityChangeKind {
    Added,
    Removed,
    Updated,
    Expired,
}

/// DTLS-SRTP fingerprint binding payload (RFC 8122 §5).
#[derive(Clone)]
pub struct DtlsFingerprint {
    pub algorithm: String,
    pub value: String,
}

impl fmt::Debug for DtlsFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DtlsFingerprint")
            .field("algorithm_present", &!self.algorithm.is_empty())
            .field("fingerprint_bytes", &self.value.len())
            .finish()
    }
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    const CANARY: &str = "identity-diagnostic-canary\r\nAuthorization: exposed";

    #[test]
    fn identity_device_reachability_and_fingerprint_debug_are_metadata_only() {
        let identity = Identity {
            id: IdentityId::from_string(CANARY),
            display_name: Some(CANARY.into()),
            kind: IdentityKind::Service,
            external_refs: HashMap::from([(CANARY.into(), CANARY.into())]),
            signing_keys: vec![Jwk(serde_json::json!({"private": CANARY}))],
            assurance: IdentityAssurance::DtlsFingerprint {
                algorithm: CANARY.into(),
                value: CANARY.into(),
            },
        };
        let device = Device {
            id: DeviceId::from_string(CANARY),
            identity_id: IdentityId::from_string(CANARY),
            kind: DeviceKind::Server,
            platform: CANARY.into(),
            registered_at: Utc::now(),
            device_signing_key: Some(Jwk(serde_json::json!({"private": CANARY}))),
        };
        let hint = ReachabilityHint {
            transport: crate::connection::Transport::Quic,
            address: CANARY.into(),
            device_id: DeviceId::from_string(CANARY),
            priority: 1,
            expires_at: None,
        };
        let fingerprint = DtlsFingerprint {
            algorithm: CANARY.into(),
            value: CANARY.into(),
        };

        for rendered in [
            format!("{identity:?}"),
            format!("{device:?}"),
            format!("{hint:?}"),
            format!("{fingerprint:?}"),
        ] {
            assert!(!rendered.contains(CANARY), "identity leaked: {rendered}");
        }
        assert_eq!(identity.id.as_str(), CANARY);
        assert_eq!(device.platform, CANARY);
        assert_eq!(hint.address, CANARY);
        assert_eq!(fingerprint.value, CANARY);
    }
}
