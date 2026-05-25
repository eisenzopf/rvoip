use crate::error::Result;
use crate::ids::{DeviceId, IdentityId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Opaque JWK placeholder. Real shape comes in step 2 / rvoip-identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Jwk(pub serde_json::Value);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IdentityKind {
    Human,
    Ai,
    Service,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeviceKind {
    Mobile,
    Web,
    Desktop,
    Embedded,
    Server,
}

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
pub enum IdentityAssurance {
    Anonymous,
    Pseudonymous {
        ephemeral_key: Jwk,
    },
    Identified {
        credential_kind: CredentialKind,
    },
    TaskScoped {
        identity: IdentityId,
        task_id: String,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
    },
    UserAuthorized {
        identity: IdentityId,
        user_id: IdentityId,
        scopes: Vec<String>,
    },
    /// D2 — the remote peer is bound to a specific DTLS certificate by
    /// hash. `algorithm` is the IANA hash name (e.g. `"sha-256"`) per RFC
    /// 8122 §5; `value` is the colon-separated hex digest as it appears
    /// in the SDP `a=fingerprint:` attribute. This is a key-binding form
    /// of pseudonymous identity — the peer has proven control of the
    /// private key matching the fingerprint, but no real-world identity
    /// is asserted.
    DtlsFingerprint {
        algorithm: String,
        value: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CredentialKind {
    OAuth2Dpop,
    Oidc,
    SipDigest,
    Passkey,
    AAuth,
}

#[derive(Clone, Debug)]
pub enum Credential {
    Bearer(String),
    OAuth2Dpop {
        access_token: String,
        dpop_proof: String,
    },
    Oidc {
        id_token: String,
        key_binding: Option<Jwk>,
    },
    Passkey {
        challenge_response: Bytes,
        attestation: Option<Bytes>,
    },
    SipDigest {
        username: String,
        response: String,
        nonce: String,
    },
    AAuth {
        signed_request: Bytes,
        signature_key: Jwk,
        signature_agent: Option<Jwk>,
    },
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

/// Plug-in identity backend. Step-1 skeleton: trait surface only — production
/// impls land in `rvoip-identity` (PRD §13.3 followup).
#[async_trait::async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn resolve(&self, identity_ref: &str) -> Result<Identity>;
    async fn devices(&self, identity_id: IdentityId) -> Result<Vec<Device>>;
    async fn reachable_via(&self, identity_id: IdentityId) -> Result<Vec<ReachabilityHint>>;
    async fn authenticate(&self, credential: Credential)
        -> Result<(IdentityId, IdentityAssurance)>;
    async fn assurance_level(&self, id: IdentityId) -> Result<IdentityAssurance>;
    fn subscribe_reachability(&self) -> mpsc::Receiver<ReachabilityChange>;
}
