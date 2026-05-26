//! Identity *data* types — moved here (V2.A.1) so consumer crates
//! like `rvoip-auth-core` and `rvoip-vcon` can depend on
//! `rvoip-core-traits` instead of `rvoip-core`, breaking the dep
//! cycle.
//!
//! The `IdentityProvider` trait and the structs that reference
//! rvoip-core's `Result` type (`Identity`, `Device`, `ReachabilityHint`,
//! `ReachabilityChange`, `DtlsFingerprint`) stay in
//! `rvoip-core::identity` — that's the broader move scope listed in
//! GAP_PLAN.md V2.A.4–6 and isn't required for the v2.A cycle break.

use crate::ids::IdentityId;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Opaque JWK placeholder. Real shape lives in `rvoip-identity`.
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

/// IdentityAssurance gradient per CONVERSATION_PROTOCOL.md §5.6.
///
/// The `DtlsFingerprint` variant is always compiled (downstream
/// crates like rvoip-auth-core match on it); the
/// `identity-fingerprint-binding` feature flag in rvoip-core controls
/// whether production fingerprint *verification* is wired by
/// default. See INTERFACE_DESIGN.md §8.4.
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
    /// D2 — DTLS-SRTP fingerprint binding (RFC 8122 §5).
    /// `algorithm` is the IANA hash name (e.g. `"sha-256"`);
    /// `value` is the colon-separated hex digest as it appears in
    /// the SDP `a=fingerprint:` attribute.
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
