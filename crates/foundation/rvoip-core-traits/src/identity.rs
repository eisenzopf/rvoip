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
use std::fmt;
use thiserror::Error;

/// Opaque JWK placeholder. Real shape lives in `rvoip-identity`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Jwk(pub serde_json::Value);

impl fmt::Debug for Jwk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, member_count) = match &self.0 {
            serde_json::Value::Null => ("null", 0),
            serde_json::Value::Bool(_) => ("bool", 1),
            serde_json::Value::Number(_) => ("number", 1),
            serde_json::Value::String(_) => ("string", 1),
            serde_json::Value::Array(values) => ("array", values.len()),
            serde_json::Value::Object(values) => ("object", values.len()),
        };
        formatter
            .debug_struct("Jwk")
            .field("kind", &kind)
            .field("member_count", &member_count)
            .finish()
    }
}

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
#[derive(Clone)]
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

impl fmt::Debug for IdentityAssurance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anonymous => formatter.write_str("Anonymous"),
            Self::Pseudonymous { ephemeral_key } => formatter
                .debug_struct("Pseudonymous")
                .field("ephemeral_key_present", &!ephemeral_key.0.is_null())
                .finish(),
            Self::Identified { credential_kind } => formatter
                .debug_struct("Identified")
                .field("credential_kind", credential_kind)
                .finish(),
            Self::TaskScoped { scopes, .. } => formatter
                .debug_struct("TaskScoped")
                .field("scope_count", &scopes.len())
                .field("expires_at_present", &true)
                .finish(),
            Self::UserAuthorized { scopes, .. } => formatter
                .debug_struct("UserAuthorized")
                .field("scope_count", &scopes.len())
                .finish(),
            Self::DtlsFingerprint {
                algorithm, value, ..
            } => formatter
                .debug_struct("DtlsFingerprint")
                .field("algorithm_present", &!algorithm.is_empty())
                .field("fingerprint_bytes", &value.len())
                .finish(),
        }
    }
}

impl IdentityAssurance {
    /// Stable, credential-free assurance class for policy diagnostics and
    /// cross-crate events. This deliberately does not format embedded keys,
    /// fingerprints, user IDs, or scopes.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Pseudonymous { .. } => "pseudonymous",
            Self::Identified { .. } => "identified",
            Self::TaskScoped { .. } => "task-scoped",
            Self::UserAuthorized { .. } => "user-authorized",
            Self::DtlsFingerprint { .. } => "dtls-fingerprint",
        }
    }
}

/// Authentication mechanism that established an [`AuthenticatedPrincipal`].
///
/// This describes the credential family rather than a concrete provider so
/// signaling and media adapters can apply policy without depending on an
/// authentication implementation crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthenticationMethod {
    Anonymous,
    Bearer,
    Jwt,
    Oidc,
    OAuth2Introspection,
    Dpop,
    SipDigest,
    MutualTls,
    AAuth,
    ApiKey,
}

impl AuthenticationMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Bearer => "bearer",
            Self::Jwt => "jwt",
            Self::Oidc => "oidc",
            Self::OAuth2Introspection => "oauth2-introspection",
            Self::Dpop => "dpop",
            Self::SipDigest => "sip-digest",
            Self::MutualTls => "mutual-tls",
            Self::AAuth => "aauth",
            Self::ApiKey => "api-key",
        }
    }
}

impl fmt::Display for AuthenticationMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable authorization ownership boundary for a principal-owned resource.
///
/// Subject alone is not sufficient: two issuers can use the same `sub`, and
/// the same issuer can reuse a subject in different tenants. All three values
/// therefore participate in equality and hashing. Missing issuer or tenant
/// values remain explicit rather than acting as wildcards.
#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PrincipalOwnershipKey {
    pub issuer: Option<String>,
    pub tenant: Option<String>,
    pub subject: String,
}

impl fmt::Debug for PrincipalOwnershipKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrincipalOwnershipKey")
            .field("issuer_present", &self.issuer.is_some())
            .field("tenant_present", &self.tenant.is_some())
            .field("subject_present", &!self.subject.is_empty())
            .finish()
    }
}

/// Transport-neutral result of successful authentication.
///
/// This type lives in `rvoip-core-traits` so auth implementations and protocol
/// adapters can carry the same complete identity without creating a dependency
/// cycle. Resource authorization should compare [`Self::ownership_key`], not
/// `subject` by itself.
#[derive(Clone)]
pub struct AuthenticatedPrincipal {
    pub subject: String,
    pub tenant: Option<String>,
    pub scopes: Vec<String>,
    pub issuer: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub method: AuthenticationMethod,
    pub assurance: IdentityAssurance,
}

impl fmt::Debug for AuthenticatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedPrincipal")
            .field("subject_present", &!self.subject.is_empty())
            .field("tenant_present", &self.tenant.is_some())
            .field("scope_count", &self.scopes.len())
            .field("issuer_present", &self.issuer.is_some())
            .field("expires_at_present", &self.expires_at.is_some())
            .field("method", &self.method)
            .field("assurance_kind", &self.assurance.kind())
            .finish()
    }
}

/// Bearer validation failure retained in the dependency-cycle-free trait
/// surface so [`AuthenticatedPrincipal::require_scope`] remains source
/// compatible when the principal type is re-exported by auth-core.
#[derive(Debug, Error)]
pub enum BearerAuthError {
    #[error("empty bearer token")]
    Empty,

    #[error("invalid bearer token: {0}")]
    Invalid(String),

    #[error("validator unavailable: {0}")]
    Unavailable(String),
}

impl AuthenticatedPrincipal {
    pub fn anonymous() -> Self {
        Self {
            subject: "anonymous".into(),
            tenant: None,
            scopes: Vec::new(),
            issuer: None,
            expires_at: None,
            method: AuthenticationMethod::Anonymous,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes
            .iter()
            .any(|candidate| candidate == scope || candidate == "*")
    }

    pub fn require_scope(&self, scope: &str) -> Result<(), BearerAuthError> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(BearerAuthError::Invalid(format!(
                "principal is missing required scope {scope}"
            )))
        }
    }

    pub fn ownership_key(&self) -> PrincipalOwnershipKey {
        PrincipalOwnershipKey {
            issuer: self.issuer.clone(),
            tenant: self.tenant.clone(),
            subject: self.subject.clone(),
        }
    }

    pub fn has_same_owner(&self, other: &Self) -> bool {
        self.ownership_key() == other.ownership_key()
    }

    /// Whether this principal is expired at `now`.
    ///
    /// An expiry equal to `now` is already expired. Principals without an
    /// expiry remain active until their backing authentication policy says
    /// otherwise.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Utc::now())
    }

    /// Compatibility mapping for authentication providers that currently
    /// produce only an [`IdentityAssurance`]. Validators with issuer, tenant,
    /// or credential-expiry information should construct a full principal.
    pub fn from_assurance(assurance: IdentityAssurance) -> Self {
        Self::from_assurance_with_method(assurance, AuthenticationMethod::Bearer)
    }

    pub fn from_assurance_with_method(
        assurance: IdentityAssurance,
        method: AuthenticationMethod,
    ) -> Self {
        let (subject, scopes, expires_at) = match &assurance {
            IdentityAssurance::Anonymous => ("anonymous".into(), Vec::new(), None),
            IdentityAssurance::Pseudonymous { ephemeral_key } => {
                let subject = ephemeral_key
                    .0
                    .get("kid")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pseudonymous")
                    .to_string();
                (subject, Vec::new(), None)
            }
            IdentityAssurance::Identified { credential_kind } => {
                (format!("identified:{credential_kind:?}"), Vec::new(), None)
            }
            IdentityAssurance::TaskScoped {
                identity,
                scopes,
                expires_at,
                ..
            } => (identity.to_string(), scopes.clone(), Some(*expires_at)),
            IdentityAssurance::UserAuthorized {
                identity, scopes, ..
            } => (identity.to_string(), scopes.clone(), None),
            IdentityAssurance::DtlsFingerprint { value, .. } => {
                (format!("dtls:{value}"), Vec::new(), None)
            }
        };
        Self {
            subject,
            tenant: None,
            scopes,
            issuer: None,
            expires_at,
            method,
            assurance,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CredentialKind {
    OAuth2Dpop,
    Oidc,
    SipDigest,
    Passkey,
    AAuth,
}

#[derive(Clone)]
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

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer(token) => formatter
                .debug_struct("Bearer")
                .field("token_bytes", &token.len())
                .finish(),
            Self::OAuth2Dpop {
                access_token,
                dpop_proof,
            } => formatter
                .debug_struct("OAuth2Dpop")
                .field("access_token_bytes", &access_token.len())
                .field("dpop_proof_bytes", &dpop_proof.len())
                .finish(),
            Self::Oidc {
                id_token,
                key_binding,
            } => formatter
                .debug_struct("Oidc")
                .field("id_token_bytes", &id_token.len())
                .field("key_binding_present", &key_binding.is_some())
                .finish(),
            Self::Passkey {
                challenge_response,
                attestation,
            } => formatter
                .debug_struct("Passkey")
                .field("challenge_response_bytes", &challenge_response.len())
                .field("attestation_present", &attestation.is_some())
                .field(
                    "attestation_bytes",
                    &attestation.as_ref().map_or(0, Bytes::len),
                )
                .finish(),
            Self::SipDigest {
                username,
                response,
                nonce,
            } => formatter
                .debug_struct("SipDigest")
                .field("username_bytes", &username.len())
                .field("response_bytes", &response.len())
                .field("nonce_bytes", &nonce.len())
                .finish(),
            Self::AAuth {
                signed_request,
                signature_key,
                signature_agent,
            } => formatter
                .debug_struct("AAuth")
                .field("signed_request_bytes", &signed_request.len())
                .field("signature_key_present", &!signature_key.0.is_null())
                .field("signature_agent_present", &signature_agent.is_some())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(issuer: &str, tenant: &str, expires_at: DateTime<Utc>) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "shared-subject".into(),
            tenant: Some(tenant.into()),
            scopes: vec!["calls:read".into()],
            issuer: Some(issuer.into()),
            expires_at: Some(expires_at),
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    #[test]
    fn ownership_includes_issuer_tenant_and_subject() {
        let expiry = Utc::now() + chrono::Duration::minutes(5);
        let original = principal("https://issuer-a.example", "tenant-a", expiry);
        let same = principal("https://issuer-a.example", "tenant-a", expiry);
        let other_tenant = principal("https://issuer-a.example", "tenant-b", expiry);
        let other_issuer = principal("https://issuer-b.example", "tenant-a", expiry);

        assert!(original.has_same_owner(&same));
        assert!(!original.has_same_owner(&other_tenant));
        assert!(!original.has_same_owner(&other_issuer));
    }

    #[test]
    fn expiry_boundary_is_inactive() {
        let now = Utc::now();
        let expired = principal("issuer", "tenant", now);
        let active = principal("issuer", "tenant", now + chrono::Duration::milliseconds(1));

        assert!(expired.is_expired_at(now));
        assert!(!active.is_expired_at(now));
    }

    #[test]
    fn diagnostic_names_are_stable_and_do_not_expose_assurance_payloads() {
        let assurance = IdentityAssurance::DtlsFingerprint {
            algorithm: "sha-256".into(),
            value: "secret-fingerprint-value".into(),
        };
        assert_eq!(assurance.kind(), "dtls-fingerprint");
        assert!(!assurance.kind().contains("secret-fingerprint-value"));
        assert_eq!(
            AuthenticationMethod::OAuth2Introspection.as_str(),
            "oauth2-introspection"
        );
        assert_eq!(AuthenticationMethod::MutualTls.to_string(), "mutual-tls");
    }

    #[test]
    fn principal_and_assurance_debug_are_metadata_only() {
        const SUBJECT: &str = "principal-subject-secret-canary";
        const TENANT: &str = "principal-tenant-secret-canary";
        const ISSUER: &str = "principal-issuer-secret-canary";
        const SCOPE: &str = "principal-scope-secret-canary";
        const FINGERPRINT: &str = "principal-fingerprint-secret-canary";

        let principal = AuthenticatedPrincipal {
            subject: SUBJECT.into(),
            tenant: Some(TENANT.into()),
            scopes: vec![SCOPE.into()],
            issuer: Some(ISSUER.into()),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            method: AuthenticationMethod::MutualTls,
            assurance: IdentityAssurance::DtlsFingerprint {
                algorithm: "sha-256".into(),
                value: FINGERPRINT.into(),
            },
        };

        let ownership = principal.ownership_key();
        let jwk = Jwk(serde_json::json!({"secret": SUBJECT}));
        let rendered = format!(
            "{principal:?} {:?} {ownership:?} {jwk:?}",
            principal.assurance
        );
        assert!(rendered.contains("AuthenticatedPrincipal"));
        assert!(rendered.contains("DtlsFingerprint"));
        assert!(rendered.contains("PrincipalOwnershipKey"));
        assert!(rendered.contains("Jwk"));
        assert!(rendered.contains("scope_count: 1"));
        for secret in [SUBJECT, TENANT, ISSUER, SCOPE, FINGERPRINT] {
            assert!(
                !rendered.contains(secret),
                "debug leaked {secret}: {rendered}"
            );
        }
    }

    #[test]
    fn credential_debug_reports_only_variant_metadata() {
        const TOKEN: &str = "bearer-token-secret-canary";
        const PROOF: &str = "dpop-proof-secret-canary";
        const USERNAME: &str = "digest-user-secret-canary";
        const RESPONSE: &str = "digest-response-secret-canary";
        const NONCE: &str = "digest-nonce-secret-canary";
        const SIGNED: &[u8] = b"aauth-signed-request-secret-canary";

        let credentials = [
            Credential::Bearer(TOKEN.into()),
            Credential::OAuth2Dpop {
                access_token: TOKEN.into(),
                dpop_proof: PROOF.into(),
            },
            Credential::Oidc {
                id_token: TOKEN.into(),
                key_binding: Some(Jwk(serde_json::json!({"secret": TOKEN}))),
            },
            Credential::Passkey {
                challenge_response: Bytes::from_static(SIGNED),
                attestation: Some(Bytes::from_static(SIGNED)),
            },
            Credential::SipDigest {
                username: USERNAME.into(),
                response: RESPONSE.into(),
                nonce: NONCE.into(),
            },
            Credential::AAuth {
                signed_request: Bytes::from_static(SIGNED),
                signature_key: Jwk(serde_json::json!({"secret": TOKEN})),
                signature_agent: Some(Jwk(serde_json::json!({"secret": PROOF}))),
            },
        ];

        let rendered = credentials
            .iter()
            .map(|credential| format!("{credential:?}"))
            .collect::<Vec<_>>()
            .join(" ");
        for variant in [
            "Bearer",
            "OAuth2Dpop",
            "Oidc",
            "Passkey",
            "SipDigest",
            "AAuth",
        ] {
            assert!(rendered.contains(variant));
        }
        for secret in [
            TOKEN,
            PROOF,
            USERNAME,
            RESPONSE,
            NONCE,
            "aauth-signed-request",
        ] {
            assert!(
                !rendered.contains(secret),
                "debug leaked {secret}: {rendered}"
            );
        }
    }

    #[test]
    fn identity_source_keeps_payload_containers_on_manual_debug() {
        let source = include_str!("identity.rs");
        for declaration in [
            "pub struct Jwk",
            "pub enum IdentityAssurance",
            "pub struct PrincipalOwnershipKey",
            "pub struct AuthenticatedPrincipal",
            "pub enum Credential {",
        ] {
            let declaration_offset = source
                .find(declaration)
                .unwrap_or_else(|| panic!("missing declaration {declaration}"));
            let prefix = &source[..declaration_offset];
            let derive_offset = prefix
                .rfind("#[derive(")
                .unwrap_or_else(|| panic!("missing derive for {declaration}"));
            assert!(
                !prefix[derive_offset..].contains("Debug"),
                "{declaration} regained derived Debug"
            );
        }
    }
}
