//! SIP access authentication for UAC and UAS applications.
//!
//! This module is the cargo-doc entry point for SIP header authentication in
//! `rvoip-sip`. It re-exports the lower-level Digest and Bearer primitives from
//! `rvoip-auth-core`, then adds SIP-aware UAC retry configuration and UAS
//! challenge/validation helpers.
//!
//! # Header Flow
//!
//! SIP authentication is negotiated in SIP headers:
//!
//! | Challenge | Retry header | Common meaning |
//! | --- | --- | --- |
//! | `401 WWW-Authenticate` | `Authorization` | Origin-server authentication |
//! | `407 Proxy-Authenticate` | `Proxy-Authorization` | Proxy authentication |
//!
//! SDP is not an auth negotiation channel. It is only part of authentication
//! when Digest `qop=auth-int` hashes the SIP message body.
//!
//! # Which API Do I Use?
//!
//! | Role | Use | Notes |
//! | --- | --- | --- |
//! | PBX Digest account | [`SipAccount`](crate::SipAccount) and [`EndpointBuilder::sip_account`](crate::EndpointBuilder::sip_account) | Configures REGISTER plus challenged outbound requests from one account model. |
//! | UAC with one scheme | [`SipClientAuth::digest`], [`SipClientAuth::bearer_token`], [`SipClientAuth::basic`], or [`SipClientAuth::aka`] | Attach through [`Config::auth`](crate::Config::auth), peer builders, or per-request `.with_auth(...)`. |
//! | UAC with negotiation | [`SipClientAuth::any`] | Chooses the strongest configured compatible challenge. |
//! | UAS validation | [`SipAuthService`] | Validates inbound requests and returns [`AuthIdentity`]. |
//! | Digest-only UAS compatibility | [`SipDigestAuthService`] | Small wrapper for Digest challenge/validation only. |
//! | Enterprise UAS hooks | [`SipAuthService::with_audit_sink`], [`SipAuthService::with_rate_limiter`], [`SipAuthService::with_digest_replay_store`] | Redacted audit, rate-limit/lockout, and shared Digest replay state. |
//! | Raw auth-core primitives | [`DigestAuthenticator`], [`DigestAuth`], [`BearerValidator`], [`JwtValidator`], [`JwksJwtValidator`], [`OAuth2IntrospectionValidator`], [`AAuthValidator`] | Crypto/token primitives; retry orchestration stays in `rvoip-sip`. |
//!
//! # Scheme and Algorithm Support
//!
//! | Scheme | UAC | UAS | Algorithms / providers | Security notes |
//! | --- | --- | --- | --- | --- |
//! | Digest | `SipClientAuth::digest`, `SipAccount`, `Credentials` shorthand | `SipAuthService::digest`, `SipDigestAuthService` | MD5, MD5-sess, SHA-256, SHA-256-sess, SHA-512-256, SHA-512-256-sess; `qop=auth` and `qop=auth-int` | Omitted algorithm defaults to MD5 for legacy PBX compatibility; unknown algorithms fail. |
//! | Bearer | `SipClientAuth::bearer_token` | `SipAuthService::with_bearer_validator` | Validator-dependent JWT/JWKS/OAuth2 introspection/AAuth/opaque tokens through `rvoip-auth-core` | UAS exposes subject/scopes through `AuthIdentity`. |
//! | Basic | `SipClientAuth::basic` | `SipAuthService::with_basic_realm` / `with_basic_user` | None | Legacy compatibility only. Cleartext SIP is rejected unless explicitly allowed. Prefer TLS or stronger schemes. |
//! | AKA | `SipClientAuth::aka` with [`AkaClientProvider`] | `SipAuthService::with_aka_provider` with [`AkaVectorProvider`] | Provider-backed `AKAv1-MD5` / `AKAv2-MD5` | The crate supplies the API shape, not SIM/USIM infrastructure or carrier IMS certification. |
//!
//! When several challenges are offered and the UAC is configured with
//! [`SipClientAuth::any`], selection prefers AKA, then Bearer, then Digest,
//! then Basic among the configured compatible options. Basic is still subject
//! to the cleartext policy.
//!
//! # Enterprise UAS Hooks
//!
//! `SipAuthService` can call [`AuthAuditSink`], [`AuthRateLimiter`], and
//! [`DigestReplayStore`] providers from `rvoip-auth-core`. Rate-limit provider
//! errors fail closed. Audit sink errors fail open by default and fail closed
//! when [`AuditFailurePolicy::FailClosed`] is configured. Audit events are
//! redacted and must not contain raw credentials.
//!
//! The sync [`SipAuthService::challenges`] helper is for local/simple use. When
//! a shared Digest replay store is configured, use
//! [`SipAuthService::challenges_async`] or the async inbound authentication
//! helpers so issued nonces are atomically admitted into bounded shared
//! storage. The store must implement the additive `admit_nonce` and
//! `accept_client_nonce_count` methods; their legacy defaults fail closed.
//! Digest-only users can use
//! [`SipDigestAuthService::authenticate_authorization_with_replay_store`].
//!
//! # Examples
//!
//! PBX Digest account with Endpoint:
//!
//! ```rust,no_run
//! use rvoip_sip::{Endpoint, Result, SipAccount};
//!
//! # async fn example() -> Result<()> {
//! let account = SipAccount::new("sips:pbx.example.com:5061", "1001", "secret")
//!     .auth_username("auth1001");
//!
//! let mut endpoint = Endpoint::builder()
//!     .sip_account(account)
//!     .build()
//!     .await?;
//!
//! endpoint.register().await?;
//! endpoint.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Bearer UAC retry on an out-of-dialog request:
//!
//! ```rust,no_run
//! use rvoip_sip::{Config, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("alice", 5060)).await?;
//! coordinator
//!     .message("sip:bob@example.com")
//!     .with_bearer_token("access-token")
//!     .with_body("hello")
//!     .send()
//!     .await?;
//! coordinator.shutdown();
//! # Ok(())
//! # }
//! ```
//!
//! Bearer UAS validation:
//!
//! ```rust
//! use std::sync::Arc;
//! use rvoip_sip::{BearerValidator, SipAuthService};
//!
//! fn service(validator: Arc<dyn BearerValidator>) -> SipAuthService {
//!     SipAuthService::new()
//!         .with_bearer_validator("api", validator)
//!         .with_bearer_scope("calls:write")
//! }
//! ```
//!
//! Basic over TLS, and explicit cleartext opt-in for legacy peers:
//!
//! ```rust
//! use rvoip_sip::SipClientAuth;
//!
//! let tls_basic = SipClientAuth::basic("alice", "secret");
//! let legacy_cleartext_basic =
//!     SipClientAuth::basic("legacy", "secret").allow_basic_over_cleartext(true);
//! # let _ = (tls_basic, legacy_cleartext_basic);
//! ```
//!
//! Composite UAC negotiation:
//!
//! ```rust
//! use rvoip_sip::SipClientAuth;
//!
//! let auth = SipClientAuth::any([
//!     SipClientAuth::bearer_token("access-token"),
//!     SipClientAuth::digest("1001", "secret"),
//! ]);
//! # let _ = auth;
//! ```
//!
//! Provider-backed AKA shape:
//!
//! ```rust
//! use std::sync::Arc;
//! use rvoip_sip::{AkaClientConfig, AkaClientProvider, SipClientAuth};
//!
//! fn aka_auth(provider: Arc<dyn AkaClientProvider>) -> SipClientAuth {
//!     SipClientAuth::aka(AkaClientConfig::new(provider))
//! }
//! ```
//!
//! Users-core-backed UAS service shape:
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use rvoip_sip::{DigestAlgorithm, SipAuthService};
//! use users_core::{AuthenticationService, UsersCoreAuthProvider};
//!
//! fn users_core_auth(auth_service: Arc<AuthenticationService>) -> SipAuthService {
//!     let provider = UsersCoreAuthProvider::shared(auth_service);
//!     SipAuthService::new()
//!         .with_bearer_validator("users-core", provider.clone())
//!         .with_basic_verifier("users-core", provider.clone())
//!         .with_digest_provider("pbx.example.com", provider)
//!         .with_digest_provider_algorithm(DigestAlgorithm::SHA256)
//! }
//! ```
//!
//! External opaque-token introspection shape:
//!
//! ```rust,ignore
//! use rvoip_sip::{OAuth2IntrospectionValidator, SipAuthService};
//! use url::Url;
//!
//! fn introspection_auth(endpoint: Url) -> SipAuthService {
//!     let validator = OAuth2IntrospectionValidator::new(endpoint)
//!         .with_issuer(["https://idp.example.com"])
//!         .with_audience(["rvoip-sip"])
//!         .into_arc();
//!     SipAuthService::new().with_bearer_validator("idp", validator)
//! }
//! ```
//!
//! External JWT/JWKS shape:
//!
//! ```rust,ignore
//! use rvoip_sip::{JwksJwtValidator, SipAuthService};
//! use url::Url;
//!
//! fn jwks_auth(jwks_url: Url) -> SipAuthService {
//!     let validator = JwksJwtValidator::new(jwks_url)
//!         .with_issuer(["https://idp.example.com"])
//!         .with_audience(["rvoip-sip"])
//!         .into_arc();
//!     SipAuthService::new().with_bearer_validator("idp", validator)
//! }
//! ```
//!
//! Custom Digest provider shape:
//!
//! ```rust,ignore
//! use rvoip_sip::{DigestAlgorithm, DigestSecret, DigestSecretProvider};
//!
//! struct MyDigestProvider;
//!
//! #[async_trait::async_trait]
//! impl DigestSecretProvider for MyDigestProvider {
//!     async fn lookup_digest_secret(
//!         &self,
//!         username: &str,
//!         realm: &str,
//!         algorithm: DigestAlgorithm,
//!     ) -> Result<Option<DigestSecret>, rvoip_sip::CredentialAuthError> {
//!         let ha1 = load_ha1_from_my_database(username, realm, algorithm).await?;
//!         Ok(ha1.map(DigestSecret::Ha1))
//!     }
//! }
//! ```

mod listener;

pub use listener::SipListenerAuthPolicy;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::errors::{redacted_auth_failure, AuthFailureStage, Result, SessionError};
use crate::types::Credentials;

// Re-export digest authentication from auth-core.
pub use rvoip_auth_core::{
    AAuthValidator, ApiKeyVerifier, AuthAuditEvent, AuthAuditOutcome, AuthAuditScheme,
    AuthAuditSink, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind, AuthRateLimitVerdict,
    AuthRateLimiter, BearerAuthError, BearerValidator, CredentialAuthError, DigestAlgorithm,
    DigestAuthenticator, DigestChallenge, DigestChallengeDetails, DigestClient as DigestAuth,
    DigestComputed, DigestNonceStatus, DigestReplayStore, DigestResponse, DigestSecret,
    DigestSecretProvider, JwksJwtValidator, JwtValidator, OAuth2IntrospectionValidator,
    PasswordVerifier, TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};

const MAX_LOCAL_DIGEST_NONCES: usize = 4_096;
const MAX_LOCAL_DIGEST_NONCE_COUNTS: usize = 16_384;
const MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME: usize = 4_096;
const MAX_LOCAL_DIGEST_SEQUENCES_PER_NONCE: usize = 8_192;
const MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME_NONCE: usize = 4_096;

fn admit_local_digest_nonce(
    nonces: &RwLock<HashMap<String, Instant>>,
    nonce_counts: &RwLock<HashMap<(String, String, String), u32>>,
    requested_nonce: &str,
    nonce_ttl: Duration,
) -> String {
    let now = Instant::now();
    let expires_at = now.checked_add(nonce_ttl).unwrap_or(now);
    let mut nonces = nonces
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let expired = nonces
        .iter()
        .filter(|(_, expires_at)| **expires_at <= now)
        .map(|(nonce, _)| nonce.clone())
        .collect::<HashSet<_>>();
    if !expired.is_empty() {
        nonces.retain(|nonce, _| !expired.contains(nonce));
    }

    let admitted = if nonces.len() >= MAX_LOCAL_DIGEST_NONCES {
        // Never evict an active challenge. Reuse one until expiry so a peer
        // that already received it can still complete authentication under
        // unauthenticated challenge churn.
        nonces
            .iter()
            .max_by_key(|(_, expires_at)| **expires_at)
            .map(|(nonce, _)| nonce.clone())
            .unwrap_or_else(|| requested_nonce.to_string())
    } else {
        nonces.insert(requested_nonce.to_string(), expires_at);
        requested_nonce.to_string()
    };
    drop(nonces);

    if !expired.is_empty() {
        nonce_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(_, nonce, _), _| !expired.contains(nonce));
    }
    admitted
}

fn digest_nonce_count(response: &DigestResponse) -> Option<u32> {
    if !matches!(response.qop.as_deref(), Some("auth") | Some("auth-int")) {
        return None;
    }
    let value = response.nc.as_deref()?;
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let count = u32::from_str_radix(value, 16).ok()?;
    let cnonce = response.cnonce.as_deref()?;
    if count == 0 || cnonce.is_empty() || cnonce.len() > 256 {
        return None;
    }
    Some(count)
}

fn accept_local_digest_nonce_count(
    nonce_counts: &RwLock<HashMap<(String, String, String), u32>>,
    response: &DigestResponse,
) -> bool {
    let Some(count) = digest_nonce_count(response) else {
        return false;
    };
    let cnonce = response
        .cnonce
        .as_deref()
        .expect("validated by digest_nonce_count");
    let key = (
        response.username.clone(),
        response.nonce.clone(),
        cnonce.to_string(),
    );
    let mut nonce_counts = nonce_counts
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(previous) = nonce_counts.get_mut(&key) {
        if count <= *previous {
            return false;
        }
        *previous = count;
        return true;
    }

    let mut username_sequences = 0;
    let mut nonce_sequences = 0;
    let mut username_nonce_sequences = 0;
    for (username, nonce, _) in nonce_counts.keys() {
        if username == &response.username {
            username_sequences += 1;
        }
        if nonce == &response.nonce {
            nonce_sequences += 1;
        }
        if username == &response.username && nonce == &response.nonce {
            username_nonce_sequences += 1;
        }
    }

    if nonce_counts.len() >= MAX_LOCAL_DIGEST_NONCE_COUNTS
        || username_sequences >= MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME
        || nonce_sequences >= MAX_LOCAL_DIGEST_SEQUENCES_PER_NONCE
        || username_nonce_sequences >= MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME_NONCE
    {
        return false;
    }
    nonce_counts.insert(key, count);
    true
}

#[derive(Clone)]
struct DigestVerifierSet {
    md5_ha1: String,
    sha256_ha1: String,
    sha512256_ha1: String,
}

impl DigestVerifierSet {
    fn from_password(username: &str, realm: &str, mut password: String) -> Self {
        let verifiers = Self {
            md5_ha1: DigestAlgorithm::MD5.compute_ha1(username, realm, &password),
            sha256_ha1: DigestAlgorithm::SHA256.compute_ha1(username, realm, &password),
            sha512256_ha1: DigestAlgorithm::SHA512256.compute_ha1(username, realm, &password),
        };
        password.zeroize();
        verifiers
    }

    fn secret(&self, algorithm: DigestAlgorithm) -> DigestSecret {
        let ha1 = match algorithm {
            DigestAlgorithm::MD5 | DigestAlgorithm::MD5Sess => &self.md5_ha1,
            DigestAlgorithm::SHA256 | DigestAlgorithm::SHA256Sess => &self.sha256_ha1,
            DigestAlgorithm::SHA512256 | DigestAlgorithm::SHA512256Sess => &self.sha512256_ha1,
        };
        DigestSecret::Ha1(ha1.clone())
    }
}

/// SIP authentication scheme shared by UAC negotiation, UAS challenges, and
/// authenticated identity results.
///
/// SIP access authentication is carried in SIP headers:
/// `WWW-Authenticate`, `Proxy-Authenticate`, `Authorization`, and
/// `Proxy-Authorization`. SDP is only relevant to Digest when
/// `qop=auth-int` hashes the request body.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub enum SipAuthScheme {
    /// SIP Digest authentication.
    Digest,
    /// Bearer-token authentication.
    Bearer,
    /// Basic username/password authentication.
    Basic,
    /// IMS AKA authentication (`AKAv1-MD5` / `AKAv2-MD5`).
    Aka,
    /// Unknown or future auth scheme.
    Other(String),
}

/// Whether an authenticated identity came from origin or proxy auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipAuthSource {
    /// `Authorization` in response to `WWW-Authenticate`.
    Origin,
    /// `Proxy-Authorization` in response to `Proxy-Authenticate`.
    Proxy,
}

/// Authenticated identity returned by the UAS-side [`SipAuthService`].
///
/// Digest and Basic usually populate [`username`](Self::username). Bearer
/// validators commonly populate [`subject`](Self::subject) and
/// [`scopes`](Self::scopes). AKA providers decide which identity fields they
/// can assert from the vector infrastructure they integrate with.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthIdentity {
    /// Scheme that authenticated the peer.
    pub scheme: SipAuthScheme,
    /// SIP username, when the scheme authenticates a named SIP user.
    pub username: Option<String>,
    /// Token subject or external identity, when available.
    pub subject: Option<String>,
    /// Authentication realm, when the scheme carries one.
    pub realm: Option<String>,
    /// Token scopes mapped from Bearer/AAuth/JWT validators.
    pub scopes: Vec<String>,
    /// Whether the accepted credential came from origin or proxy auth.
    pub source: SipAuthSource,
}

/// Result of evaluating inbound UAS authentication across all enabled
/// schemes in [`SipAuthService`].
#[derive(Clone, PartialEq, Eq)]
pub enum SipAuthDecision {
    /// The inbound request carried acceptable credentials.
    Authorized(AuthIdentity),
    /// The inbound request should be challenged or rejected.
    Rejected {
        /// Challenge header values in priority order.
        challenges: Vec<SipAuthChallenge>,
    },
}

/// Listener-oriented authentication result retaining the complete canonical
/// principal alongside the compatibility [`AuthIdentity`] view.
#[derive(Clone)]
pub enum SipPrincipalAuthDecision {
    /// The request authenticated successfully.
    Authorized {
        /// Compatibility SIP identity view used by existing applications.
        identity: AuthIdentity,
        /// Complete canonical principal returned by the validator or derived
        /// from the accepted SIP Digest identity.
        principal: AuthenticatedPrincipal,
    },
    /// The request did not carry acceptable credentials.
    Rejected {
        /// Authentication challenges suitable for the response.
        challenges: Vec<SipAuthChallenge>,
    },
}

/// UAS challenge value generated by [`SipAuthService`].
///
/// Send [`value`](Self::value) in either `WWW-Authenticate` or
/// `Proxy-Authenticate`, depending on [`source`](Self::source).
#[derive(Clone, PartialEq, Eq)]
pub struct SipAuthChallenge {
    /// Scheme to advertise.
    pub scheme: SipAuthScheme,
    /// Header value, for example `Digest realm="...", nonce="..."`.
    pub value: String,
    /// Whether this is a proxy challenge (`407`) rather than origin (`401`).
    pub source: SipAuthSource,
}

/// Credential-free diagnostic view of an authentication scheme.
///
/// `SipAuthScheme::Other` is peer-controlled, so its spelling must never be
/// rendered by an authentication result container.
struct SipAuthSchemeDiagnostic<'a>(&'a SipAuthScheme);

impl fmt::Debug for SipAuthScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Digest => formatter.write_str("Digest"),
            Self::Bearer => formatter.write_str("Bearer"),
            Self::Basic => formatter.write_str("Basic"),
            Self::Aka => formatter.write_str("Aka"),
            Self::Other(value) => formatter
                .debug_struct("Other")
                .field("value_len", &value.len())
                .finish(),
        }
    }
}

impl fmt::Debug for SipAuthSchemeDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for SipAuthSchemeDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            SipAuthScheme::Digest => "digest",
            SipAuthScheme::Bearer => "bearer",
            SipAuthScheme::Basic => "basic",
            SipAuthScheme::Aka => "aka",
            SipAuthScheme::Other(_) => "other",
        })
    }
}

struct AuthIdentityDiagnostic<'a>(&'a AuthIdentity);

impl fmt::Debug for AuthIdentityDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthIdentity")
            .field("scheme", &SipAuthSchemeDiagnostic(&self.0.scheme))
            .field("source", &self.0.source)
            .field("username_present", &self.0.username.is_some())
            .field("subject_present", &self.0.subject.is_some())
            .field("realm_present", &self.0.realm.is_some())
            .field("scope_count", &self.0.scopes.len())
            .finish()
    }
}

impl fmt::Debug for AuthIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&AuthIdentityDiagnostic(self), formatter)
    }
}

impl fmt::Debug for SipAuthDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authorized(identity) => formatter
                .debug_tuple("Authorized")
                .field(&AuthIdentityDiagnostic(identity))
                .finish(),
            Self::Rejected { challenges } => formatter
                .debug_struct("Rejected")
                .field("challenge_count", &challenges.len())
                .finish(),
        }
    }
}

impl fmt::Debug for SipPrincipalAuthDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authorized {
                identity,
                principal,
            } => formatter
                .debug_struct("Authorized")
                .field("identity", &AuthIdentityDiagnostic(identity))
                .field("principal_method", &principal.method)
                .field("principal_assurance", &principal.assurance.kind())
                .field("principal_tenant_present", &principal.tenant.is_some())
                .field("principal_issuer_present", &principal.issuer.is_some())
                .field("principal_expiry_present", &principal.expires_at.is_some())
                .field("principal_scope_count", &principal.scopes.len())
                .finish(),
            Self::Rejected { challenges } => formatter
                .debug_struct("Rejected")
                .field("challenge_count", &challenges.len())
                .finish(),
        }
    }
}

impl fmt::Debug for SipAuthChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipAuthChallenge")
            .field("scheme", &SipAuthSchemeDiagnostic(&self.scheme))
            .field("source", &self.source)
            .field("value_present", &!self.value.is_empty())
            .field("value_len", &self.value.len())
            .finish()
    }
}

#[cfg(test)]
mod auth_container_diagnostic_tests {
    use super::*;

    const IDENTITY_CANARY: &str = "identity\r\nX-Identity-Canary: exposed";
    const CHALLENGE_CANARY: &str = "Digest realm=\"secret\", nonce=\"nonce-canary\"";
    const SCHEME_CANARY: &str = "Scheme\r\nX-Scheme-Canary: exposed";

    fn malicious_identity() -> AuthIdentity {
        AuthIdentity {
            scheme: SipAuthScheme::Other(SCHEME_CANARY.to_string()),
            username: Some(IDENTITY_CANARY.to_string()),
            subject: Some(format!("subject-{IDENTITY_CANARY}")),
            realm: Some(format!("realm-{IDENTITY_CANARY}")),
            scopes: vec![format!("scope-{IDENTITY_CANARY}")],
            source: SipAuthSource::Proxy,
        }
    }

    fn malicious_challenge() -> SipAuthChallenge {
        SipAuthChallenge {
            scheme: SipAuthScheme::Other(SCHEME_CANARY.to_string()),
            value: CHALLENGE_CANARY.to_string(),
            source: SipAuthSource::Origin,
        }
    }

    fn assert_no_auth_canaries(rendered: &str) {
        for canary in [IDENTITY_CANARY, CHALLENGE_CANARY, SCHEME_CANARY] {
            assert!(
                !rendered.contains(canary),
                "authentication container leaked peer or credential data: {rendered}"
            );
        }
    }

    #[test]
    fn auth_challenge_debug_retains_shape_without_challenge_content() {
        let challenge = malicious_challenge();
        let rendered = format!("{challenge:?}");

        assert!(rendered.starts_with("SipAuthChallenge"));
        assert!(rendered.contains("scheme: other"));
        assert!(rendered.contains("source: Origin"));
        assert!(rendered.contains("value_present: true"));
        assert!(rendered.contains(&format!("value_len: {}", CHALLENGE_CANARY.len())));
        assert_no_auth_canaries(&rendered);

        // Diagnostic hardening must not change the live response value.
        assert_eq!(challenge.value, CHALLENGE_CANARY);
    }

    #[test]
    fn auth_decision_debug_retains_variant_and_metadata_only_identity_shape() {
        let authorized = SipAuthDecision::Authorized(malicious_identity());
        let rejected = SipAuthDecision::Rejected {
            challenges: vec![malicious_challenge()],
        };

        let authorized_debug = format!("{authorized:?}");
        assert!(authorized_debug.starts_with("Authorized(AuthIdentity"));
        assert!(authorized_debug.contains("scheme: other"));
        assert!(authorized_debug.contains("source: Proxy"));
        assert!(authorized_debug.contains("username_present: true"));
        assert!(authorized_debug.contains("scope_count: 1"));
        assert_no_auth_canaries(&authorized_debug);

        let rejected_debug = format!("{rejected:?}");
        assert!(rejected_debug.starts_with("Rejected"));
        assert!(rejected_debug.contains("challenge_count: 1"));
        assert_no_auth_canaries(&rejected_debug);
    }

    #[test]
    fn principal_decision_debug_omits_ownership_and_scope_values() {
        let principal = AuthenticatedPrincipal {
            subject: format!("principal-{IDENTITY_CANARY}"),
            tenant: Some(format!("tenant-{IDENTITY_CANARY}")),
            scopes: vec![format!("principal-scope-{IDENTITY_CANARY}")],
            issuer: Some(format!("issuer-{IDENTITY_CANARY}")),
            expires_at: Some(chrono::Utc::now()),
            method: AuthenticationMethod::SipDigest,
            assurance: IdentityAssurance::Anonymous,
        };
        let decision = SipPrincipalAuthDecision::Authorized {
            identity: malicious_identity(),
            principal,
        };

        let rendered = format!("{decision:?}");
        assert!(rendered.starts_with("Authorized"));
        assert!(rendered.contains("principal_method: SipDigest"));
        assert!(rendered.contains("principal_assurance: \"anonymous\""));
        assert!(rendered.contains("principal_tenant_present: true"));
        assert!(rendered.contains("principal_scope_count: 1"));
        assert_no_auth_canaries(&rendered);
    }

    #[test]
    fn direct_scheme_and_identity_debug_are_metadata_only() {
        let scheme = SipAuthScheme::Other(SCHEME_CANARY.to_string());
        let scheme_debug = format!("{scheme:?}");
        assert_eq!(
            scheme_debug,
            format!("Other {{ value_len: {} }}", SCHEME_CANARY.len())
        );
        assert_no_auth_canaries(&scheme_debug);

        let identity = malicious_identity();
        let identity_debug = format!("{identity:?}");
        assert!(identity_debug.starts_with("AuthIdentity"));
        assert!(identity_debug.contains("scheme: other"));
        assert!(identity_debug.contains("realm_present: true"));
        assert!(identity_debug.contains("scope_count: 1"));
        assert_no_auth_canaries(&identity_debug);

        assert_eq!(identity.username.as_deref(), Some(IDENTITY_CANARY));
        assert_eq!(identity.scheme, SipAuthScheme::Other(SCHEME_CANARY.into()));
    }

    #[test]
    fn auth_context_policy_and_transport_debug_expose_only_shape() {
        let context = SipAuthContext::new()
            .with_peer(IDENTITY_CANARY)
            .with_metadata("identity-key", IDENTITY_CANARY);
        let context_debug = format!("{context:?}");
        assert_eq!(
            context_debug,
            "SipAuthContext { peer_present: true, metadata_entry_count: 1 }"
        );
        assert_no_auth_canaries(&context_debug);

        let policy = SipAuthPolicy::new().allow_only([
            SipAuthScheme::Digest,
            SipAuthScheme::Other(SCHEME_CANARY.to_string()),
        ]);
        let policy_debug = format!("{policy:?}");
        assert!(policy_debug.starts_with("SipAuthPolicy"));
        assert!(policy_debug.contains("enabled_scheme_count: 2"));
        assert_no_auth_canaries(&policy_debug);

        let transport = SipTransportSecurityContext::from_transport_name(IDENTITY_CANARY)
            .with_addrs(IDENTITY_CANARY, IDENTITY_CANARY);
        let transport_debug = format!("{transport:?}");
        assert_eq!(
            transport_debug,
            "SipTransportSecurityContext { transport_present: true, local_addr_present: true, remote_addr_present: true, secure: false }"
        );
        assert_no_auth_canaries(&transport_debug);
    }

    #[test]
    fn client_header_and_service_debug_omit_auth_material_and_configuration_values() {
        let header = ClientAuthHeader {
            value: CHALLENGE_CANARY.to_string(),
            scheme: SipAuthScheme::Other(SCHEME_CANARY.to_string()),
            digest_challenge: None,
            stale: true,
        };
        let header_debug = format!("{header:?}");
        assert!(header_debug.starts_with("ClientAuthHeader"));
        assert!(header_debug.contains("value_present: true"));
        assert!(header_debug.contains(&format!("value_len: {}", CHALLENGE_CANARY.len())));
        assert!(header_debug.contains("scheme: other"));
        assert_no_auth_canaries(&header_debug);
        assert_eq!(header.value, CHALLENGE_CANARY);

        let policy =
            SipAuthPolicy::new().allow_only([SipAuthScheme::Other(SCHEME_CANARY.to_string())]);
        let service = SipAuthService::digest(IDENTITY_CANARY)
            .with_policy(policy)
            .with_bearer_scope(format!("scope-{IDENTITY_CANARY}"))
            .with_required_bearer_scope(format!("required-{IDENTITY_CANARY}"))
            .with_basic_realm(format!("basic-{IDENTITY_CANARY}"));
        let service_debug = format!("{service:?}");
        assert!(service_debug.starts_with("SipAuthService"));
        assert!(service_debug.contains("bearer_scope_present: true"));
        assert!(service_debug.contains("required_bearer_scope_present: true"));
        assert!(service_debug.contains("basic: true"));
        assert_no_auth_canaries(&service_debug);

        let digest_service = SipDigestAuthService::new(IDENTITY_CANARY);
        digest_service.add_user(IDENTITY_CANARY, CHALLENGE_CANARY);
        let digest_debug = format!("{digest_service:?}");
        assert!(digest_debug.starts_with("SipDigestAuthService"));
        assert!(digest_debug.contains("realm_present: true"));
        assert!(digest_debug.contains("user_count: 1"));
        assert_no_auth_canaries(&digest_debug);
    }

    #[test]
    fn digest_compatibility_decision_debug_omits_identity_realm_and_challenge() {
        let authorized = AuthDecision::Authorized {
            username: IDENTITY_CANARY.to_string(),
            realm: CHALLENGE_CANARY.to_string(),
        };
        let authorized_debug = format!("{authorized:?}");
        assert_eq!(
            authorized_debug,
            "Authorized { username_present: true, realm_present: true }"
        );
        assert_no_auth_canaries(&authorized_debug);

        let service = SipDigestAuthService::new(CHALLENGE_CANARY);
        let challenge = service.challenge();
        let rejected = AuthDecision::Rejected {
            www_authenticate: service.www_authenticate(&challenge),
            challenge,
        };
        let rejected_debug = format!("{rejected:?}");
        assert!(rejected_debug.starts_with("Rejected"));
        assert!(rejected_debug.contains("challenge_present: true"));
        assert_no_auth_canaries(&rejected_debug);

        match authorized {
            AuthDecision::Authorized { username, realm } => {
                assert_eq!(username, IDENTITY_CANARY);
                assert_eq!(realm, CHALLENGE_CANARY);
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
}

/// Non-secret context supplied to UAS-side authentication.
///
/// Context values are used for rate-limit keys and redacted audit events. Do
/// not put passwords, bearer tokens, API keys, HA1 values, full JWTs, or raw
/// Authorization headers in [`metadata`](Self::metadata).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SipAuthContext {
    /// Source peer, IP, connection id, or deployment-specific peer handle.
    pub peer: Option<String>,
    /// Additional non-secret metadata to attach to audit/rate-limit events.
    pub metadata: BTreeMap<String, String>,
}

impl fmt::Debug for SipAuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipAuthContext")
            .field("peer_present", &self.peer.is_some())
            .field("metadata_entry_count", &self.metadata.len())
            .finish()
    }
}

impl SipAuthContext {
    /// Create an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a peer identifier.
    pub fn with_peer(mut self, peer: impl Into<String>) -> Self {
        self.peer = Some(peer.into());
        self
    }

    /// Attach non-secret metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// How `SipAuthService` handles audit-sink failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditFailurePolicy {
    /// Ignore audit sink failures after credential handling completes.
    FailOpen,
    /// Return an auth error when the audit sink cannot record an event.
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthAttemptScheme {
    Digest,
    Bearer,
    Basic,
    Aka,
    Unknown,
    Missing,
}

impl AuthAttemptScheme {
    fn audit_scheme(self) -> AuthAuditScheme {
        match self {
            Self::Digest => AuthAuditScheme::Digest,
            Self::Bearer => AuthAuditScheme::Bearer,
            Self::Basic => AuthAuditScheme::Basic,
            Self::Aka => AuthAuditScheme::Aka,
            Self::Unknown | Self::Missing => AuthAuditScheme::Other("sip".to_string()),
        }
    }

    fn rate_limit_kind(self) -> AuthRateLimitKind {
        match self {
            Self::Digest => AuthRateLimitKind::Digest,
            Self::Bearer => AuthRateLimitKind::BearerToken,
            Self::Basic => AuthRateLimitKind::BasicPassword,
            Self::Aka => AuthRateLimitKind::SipRequest,
            Self::Unknown | Self::Missing => AuthRateLimitKind::SipRequest,
        }
    }
}

/// UAS-side authentication policy for [`SipAuthService`].
///
/// This policy is additive to provider configuration. Providers answer
/// credentials; policy decides which schemes and transport/security posture are
/// acceptable before provider validation is trusted.
#[derive(Clone)]
pub struct SipAuthPolicy {
    enabled_schemes: Option<Vec<SipAuthScheme>>,
    minimum_digest_algorithm: Option<DigestAlgorithm>,
    allow_basic_over_cleartext: bool,
    allow_bearer_over_cleartext: bool,
    require_digest_replay_store: bool,
    audit_failure_policy: AuditFailurePolicy,
}

impl fmt::Debug for SipAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipAuthPolicy")
            .field(
                "enabled_scheme_count",
                &self.enabled_schemes.as_ref().map_or(0, Vec::len),
            )
            .field(
                "minimum_digest_algorithm_present",
                &self.minimum_digest_algorithm.is_some(),
            )
            .field(
                "allow_basic_over_cleartext",
                &self.allow_basic_over_cleartext,
            )
            .field(
                "allow_bearer_over_cleartext",
                &self.allow_bearer_over_cleartext,
            )
            .field(
                "require_digest_replay_store",
                &self.require_digest_replay_store,
            )
            .field("audit_failure_policy", &self.audit_failure_policy)
            .finish()
    }
}

impl SipAuthPolicy {
    /// Create the default policy.
    ///
    /// Defaults permit any configured scheme, reject Basic and Bearer over
    /// cleartext, allow MD5 Digest for PBX compatibility, do not require shared
    /// replay storage, and fail open on audit sink errors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict UAS auth to the listed schemes.
    pub fn allow_only(mut self, schemes: impl IntoIterator<Item = SipAuthScheme>) -> Self {
        self.enabled_schemes = Some(schemes.into_iter().collect());
        self
    }

    /// Require Digest responses to use at least this algorithm strength.
    ///
    /// Strength order is MD5, MD5-sess, SHA-256, SHA-256-sess, SHA-512-256,
    /// SHA-512-256-sess.
    pub fn with_minimum_digest_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        self.minimum_digest_algorithm = Some(algorithm);
        self
    }

    /// Permit Basic credentials on cleartext SIP transports.
    pub fn allow_basic_over_cleartext(mut self, allow: bool) -> Self {
        self.allow_basic_over_cleartext = allow;
        self
    }

    /// Permit Bearer tokens on cleartext SIP transports.
    pub fn allow_bearer_over_cleartext(mut self, allow: bool) -> Self {
        self.allow_bearer_over_cleartext = allow;
        self
    }

    /// Require a configured [`DigestReplayStore`] before accepting Digest.
    pub fn require_digest_replay_store(mut self, require: bool) -> Self {
        self.require_digest_replay_store = require;
        self
    }

    /// Select whether audit sink failures fail open or fail closed.
    pub fn with_audit_failure_policy(mut self, policy: AuditFailurePolicy) -> Self {
        self.audit_failure_policy = policy;
        self
    }

    fn scheme_allowed(&self, scheme: SipAuthScheme) -> bool {
        self.enabled_schemes
            .as_ref()
            .is_none_or(|schemes| schemes.iter().any(|candidate| *candidate == scheme))
    }

    fn digest_algorithm_allowed(&self, algorithm: DigestAlgorithm) -> bool {
        self.minimum_digest_algorithm.is_none_or(|minimum| {
            digest_algorithm_strength(algorithm) >= digest_algorithm_strength(minimum)
        })
    }
}

impl Default for SipAuthPolicy {
    fn default() -> Self {
        Self {
            enabled_schemes: None,
            minimum_digest_algorithm: None,
            allow_basic_over_cleartext: false,
            allow_bearer_over_cleartext: false,
            require_digest_replay_store: false,
            audit_failure_policy: AuditFailurePolicy::FailOpen,
        }
    }
}

/// Transport security context for auth policy decisions.
///
/// Prefer values derived from the actual receiving/sending transport. The
/// `from_request_uri_hint` constructor exists only as a compatibility fallback
/// until every event path carries transport metadata.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SipTransportSecurityContext {
    /// Transport flavor, for example `UDP`, `TCP`, `TLS`, `WS`, or `WSS`.
    pub transport: Option<String>,
    /// Local socket/address, if known.
    pub local_addr: Option<String>,
    /// Remote socket/address, if known.
    pub remote_addr: Option<String>,
    /// Whether the transport is protected for sending credentials such as
    /// Basic passwords or Bearer tokens.
    pub secure: bool,
}

impl fmt::Debug for SipTransportSecurityContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipTransportSecurityContext")
            .field("transport_present", &self.transport.is_some())
            .field("local_addr_present", &self.local_addr.is_some())
            .field("remote_addr_present", &self.remote_addr.is_some())
            .field("secure", &self.secure)
            .finish()
    }
}

impl SipTransportSecurityContext {
    /// Unknown or cleartext transport. This is intentionally fail-closed for
    /// Basic/Bearer cleartext policy.
    pub fn unknown() -> Self {
        Self::default()
    }

    /// Compatibility constructor from the legacy boolean parameter.
    pub fn from_is_tls(is_tls: bool) -> Self {
        if is_tls {
            Self::secure("TLS")
        } else {
            Self::unknown()
        }
    }

    /// Build from a concrete transport flavor.
    pub fn from_transport_name(transport: impl Into<String>) -> Self {
        let transport = transport.into();
        let secure = transport_name_is_secure(&transport);
        Self {
            transport: Some(transport),
            local_addr: None,
            remote_addr: None,
            secure,
        }
    }

    /// Build from cross-crate SIP transport metadata.
    pub fn from_transport_context(
        context: &rvoip_infra_common::events::cross_crate::SipTransportContext,
    ) -> Self {
        Self {
            transport: Some(context.transport.clone()),
            local_addr: Some(context.local_addr.clone()),
            remote_addr: Some(context.remote_addr.clone()),
            secure: context.secure,
        }
    }

    /// Compatibility fallback from request URI scheme.
    ///
    /// This is weaker than transport-truth metadata because a `sips:` URI is
    /// not itself proof that this hop arrived or will be sent over TLS/WSS.
    pub fn from_request_uri_hint(request_uri: &str) -> Self {
        if request_uri.to_ascii_lowercase().starts_with("sips:") {
            Self::secure("SIPS-URI")
        } else {
            Self::unknown()
        }
    }

    /// Compatibility fallback from SIP URI syntax, including `;transport=`.
    ///
    /// This handles URI-selected transports such as
    /// `sip:bob@example.com;transport=tls` and `;transport=wss`. It is still a
    /// syntactic hint, not proof of the actual selected outbound transport.
    pub fn from_request_uri_transport_hint(request_uri: &str) -> Self {
        let Ok(uri) = rvoip_sip_core::Uri::from_str(request_uri) else {
            return Self::from_request_uri_hint(request_uri);
        };
        let transport = rvoip_sip_transport::resolver::select_transport_for_uri(&uri);
        Self::from_transport_name(transport.to_string())
    }

    /// Secure transport context with a named transport.
    pub fn secure(transport: impl Into<String>) -> Self {
        Self {
            transport: Some(transport.into()),
            local_addr: None,
            remote_addr: None,
            secure: true,
        }
    }

    /// Attach local and remote non-secret address metadata.
    pub fn with_addrs(
        mut self,
        local_addr: impl Into<String>,
        remote_addr: impl Into<String>,
    ) -> Self {
        self.local_addr = Some(local_addr.into());
        self.remote_addr = Some(remote_addr.into());
        self
    }

    /// Whether this context is secure enough for credential-bearing schemes.
    pub fn is_secure(&self) -> bool {
        self.secure
    }
}

fn transport_name_is_secure(transport: &str) -> bool {
    matches!(
        transport.trim().to_ascii_uppercase().as_str(),
        "TLS" | "WSS" | "SIPS" | "SIPS-URI"
    )
}

/// UAC-side authentication configuration for challenged outbound requests.
///
/// Attach this to default configuration with [`Config::auth`](crate::Config::auth)
/// or to individual request builders with `.with_auth(...)`. The Digest-only
/// `Credentials` shorthand still works and converts into
/// [`SipClientAuth::Digest`].
#[non_exhaustive]
#[derive(Clone)]
pub enum SipClientAuth {
    /// Digest username/password credentials.
    Digest(Credentials),
    /// Bearer token. Sent after a Bearer challenge unless the request builder
    /// explicitly authors a preemptive Authorization header.
    BearerToken(String),
    /// Bearer token with explicit cleartext SIP opt-in.
    ///
    /// Prefer [`SipClientAuth::bearer_token`] on TLS/WSS transports. This
    /// variant exists for controlled legacy test/deployment environments where
    /// sending a bearer token over cleartext SIP has been explicitly accepted.
    BearerTokenCleartextAllowed(String),
    /// Basic credentials. Disabled over cleartext unless the instance opts in.
    Basic {
        /// Basic username.
        username: String,
        /// Basic password.
        password: String,
        /// Permit sending Basic on non-TLS SIP transports.
        allow_cleartext: bool,
    },
    /// IMS AKA client provider configuration.
    Aka(AkaClientConfig),
    /// Multiple configured auth options. Selection prefers AKA, then Bearer,
    /// then Digest, then Basic when the peer offers several compatible
    /// challenges.
    Composite(Vec<SipClientAuth>),
}

impl std::fmt::Debug for SipClientAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Digest(_) => formatter
                .debug_struct("SipClientAuth::Digest")
                .field("credentials", &"[redacted]")
                .finish(),
            Self::BearerToken(_) => formatter
                .debug_tuple("SipClientAuth::BearerToken")
                .field(&"[redacted]")
                .finish(),
            Self::BearerTokenCleartextAllowed(_) => formatter
                .debug_tuple("SipClientAuth::BearerTokenCleartextAllowed")
                .field(&"[redacted]")
                .finish(),
            Self::Basic {
                allow_cleartext, ..
            } => formatter
                .debug_struct("SipClientAuth::Basic")
                .field("username", &"[redacted]")
                .field("password", &"[redacted]")
                .field("allow_cleartext", allow_cleartext)
                .finish(),
            Self::Aka(_) => formatter.write_str("SipClientAuth::Aka([redacted])"),
            Self::Composite(auth) => formatter
                .debug_struct("SipClientAuth::Composite")
                .field("option_count", &auth.len())
                .finish(),
        }
    }
}

impl SipClientAuth {
    /// Build Digest username/password credentials.
    pub fn digest(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Digest(Credentials::new(username, password))
    }

    /// Build a Bearer token response option.
    ///
    /// The token is sent only when the peer offers a Bearer challenge, unless
    /// a lower-level caller explicitly authors a preemptive header.
    pub fn bearer_token(token: impl Into<String>) -> Self {
        Self::BearerToken(token.into())
    }

    /// Permit Bearer tokens on non-TLS SIP transports.
    ///
    /// This is an explicit legacy interoperability opt-in. Leave it disabled
    /// unless the application has a deployment-specific reason to accept
    /// cleartext bearer tokens on the wire.
    pub fn allow_bearer_over_cleartext(self, allow: bool) -> Self {
        match self {
            Self::BearerToken(token) if allow => Self::BearerTokenCleartextAllowed(token),
            Self::BearerTokenCleartextAllowed(token) if !allow => Self::BearerToken(token),
            Self::Composite(auths) => Self::Composite(
                auths
                    .into_iter()
                    .map(|auth| auth.allow_bearer_over_cleartext(allow))
                    .collect(),
            ),
            _ => self,
        }
    }

    /// Build Basic username/password credentials.
    ///
    /// Cleartext SIP is rejected unless
    /// [`Self::allow_basic_over_cleartext`] is called on the returned value.
    /// Use Basic only for legacy compatibility; prefer TLS plus Digest,
    /// Bearer, or AKA where available.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Basic {
            username: username.into(),
            password: password.into(),
            allow_cleartext: false,
        }
    }

    /// Build IMS AKA client configuration from an application-supplied
    /// provider.
    pub fn aka(config: AkaClientConfig) -> Self {
        Self::Aka(config)
    }

    /// Configure multiple UAC auth options and negotiate the strongest
    /// compatible scheme offered by the peer.
    ///
    /// Selection prefers AKA, then Bearer, then Digest, then Basic among the
    /// configured options. Basic still obeys the cleartext policy.
    pub fn any(auth: impl IntoIterator<Item = SipClientAuth>) -> Self {
        Self::Composite(auth.into_iter().collect())
    }

    /// Permit Basic credentials on non-TLS SIP transports.
    ///
    /// This is an explicit legacy interoperability opt-in. Leave it disabled
    /// unless the application has a deployment-specific reason to accept
    /// cleartext Basic credentials on the wire.
    pub fn allow_basic_over_cleartext(mut self, allow: bool) -> Self {
        if let Self::Basic {
            allow_cleartext, ..
        } = &mut self
        {
            *allow_cleartext = allow;
        }
        self
    }

    /// Build an Authorization / Proxy-Authorization header value for the
    /// selected challenge.
    pub fn authorization_for_challenge(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
        body: Option<&[u8]>,
        is_tls: bool,
    ) -> Result<ClientAuthHeader> {
        self.authorization_for_challenge_with_transport_context(
            challenge_header,
            method,
            request_uri,
            nonce_count,
            body,
            &SipTransportSecurityContext::from_is_tls(is_tls),
        )
    }

    /// Build an Authorization / Proxy-Authorization header value for the
    /// selected challenge using transport-truth security context.
    pub fn authorization_for_challenge_with_transport_context(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
        body: Option<&[u8]>,
        transport: &SipTransportSecurityContext,
    ) -> Result<ClientAuthHeader> {
        let selected = match self {
            SipClientAuth::Digest(credentials) => {
                let challenge = extract_digest_challenge(challenge_header).ok_or_else(|| {
                    SessionError::AuthError(
                        "Digest credentials cannot answer a non-Digest challenge".to_string(),
                    )
                })?;
                let challenge = rvoip_auth_core::DigestAuthenticator::parse_challenge(&challenge)?;
                let computed = rvoip_auth_core::DigestClient::compute_response_with_state(
                    &credentials.username,
                    &credentials.password,
                    &challenge,
                    method,
                    request_uri,
                    nonce_count,
                    body,
                )?;
                let value = rvoip_auth_core::DigestClient::format_authorization_with_state(
                    &credentials.username,
                    &challenge,
                    request_uri,
                    &computed,
                );
                Ok(ClientAuthHeader {
                    value,
                    scheme: SipAuthScheme::Digest,
                    digest_challenge: Some(challenge),
                    stale: parse_digest_stale(challenge_header),
                })
            }
            SipClientAuth::BearerToken(token) => {
                if !contains_auth_scheme(challenge_header, "Bearer") {
                    return Err(SessionError::AuthError(
                        "Bearer token cannot answer a non-Bearer challenge".to_string(),
                    ));
                }
                if !transport.is_secure() {
                    return Err(SessionError::AuthError(
                        "Bearer authentication over cleartext SIP is disabled".to_string(),
                    ));
                }
                if token.is_empty() {
                    return Err(SessionError::AuthError(
                        "Bearer token cannot be empty".to_string(),
                    ));
                }
                Ok(ClientAuthHeader {
                    value: format!("Bearer {token}"),
                    scheme: SipAuthScheme::Bearer,
                    digest_challenge: None,
                    stale: false,
                })
            }
            SipClientAuth::BearerTokenCleartextAllowed(token) => {
                if !contains_auth_scheme(challenge_header, "Bearer") {
                    return Err(SessionError::AuthError(
                        "Bearer token cannot answer a non-Bearer challenge".to_string(),
                    ));
                }
                if token.is_empty() {
                    return Err(SessionError::AuthError(
                        "Bearer token cannot be empty".to_string(),
                    ));
                }
                Ok(ClientAuthHeader {
                    value: format!("Bearer {token}"),
                    scheme: SipAuthScheme::Bearer,
                    digest_challenge: None,
                    stale: false,
                })
            }
            SipClientAuth::Basic {
                username,
                password,
                allow_cleartext,
            } => {
                if !contains_auth_scheme(challenge_header, "Basic") {
                    return Err(SessionError::AuthError(
                        "Basic credentials cannot answer a non-Basic challenge".to_string(),
                    ));
                }
                if !transport.is_secure() && !*allow_cleartext {
                    return Err(SessionError::AuthError(
                        "Basic authentication over cleartext SIP is disabled".to_string(),
                    ));
                }
                let token = BASE64_STANDARD.encode(format!("{username}:{password}"));
                Ok(ClientAuthHeader {
                    value: format!("Basic {token}"),
                    scheme: SipAuthScheme::Basic,
                    digest_challenge: None,
                    stale: false,
                })
            }
            SipClientAuth::Aka(config) => {
                if !contains_aka_challenge(challenge_header) {
                    return Err(SessionError::AuthError(
                        "AKA credentials cannot answer a non-AKA challenge".to_string(),
                    ));
                }
                let response = config
                    .respond(challenge_header, method, request_uri, nonce_count)
                    .map_err(|error| {
                        redacted_auth_failure(AuthFailureStage::AkaClientProvider, error)
                    })?;
                Ok(ClientAuthHeader {
                    value: response,
                    scheme: SipAuthScheme::Aka,
                    digest_challenge: None,
                    stale: false,
                })
            }
            SipClientAuth::Composite(auths) => select_composite_client_auth(
                auths,
                challenge_header,
                method,
                request_uri,
                nonce_count,
                body,
                transport,
            ),
        }?;
        rvoip_sip_core::validation::validate_authorization_header_value(&selected.value).map_err(
            |_| {
                SessionError::AuthError(
                    "generated SIP authorization header failed wire-safety validation".to_string(),
                )
            },
        )?;
        Ok(selected)
    }
}

impl From<Credentials> for SipClientAuth {
    fn from(value: Credentials) -> Self {
        Self::Digest(value)
    }
}

/// UAC auth header value selected by [`SipClientAuth`] for a challenge.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientAuthHeader {
    /// Authorization header body.
    pub value: String,
    /// Scheme selected for the challenge.
    pub scheme: SipAuthScheme,
    /// Parsed Digest challenge when the selected scheme is Digest.
    pub digest_challenge: Option<DigestChallenge>,
    /// Whether the selected Digest challenge carried `stale=true`.
    pub stale: bool,
}

impl std::fmt::Debug for ClientAuthHeader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientAuthHeader")
            .field("value_present", &!self.value.is_empty())
            .field("value_len", &self.value.len())
            .field("scheme", &SipAuthSchemeDiagnostic(&self.scheme))
            .field("has_digest_challenge", &self.digest_challenge.is_some())
            .field("stale", &self.stale)
            .finish()
    }
}

/// UAC-side IMS AKA client configuration.
///
/// Production IMS integrations should provide a [`AkaClientProvider`].
/// The in-tree API keeps AKA provider-backed because RES/CK/IK material is
/// issued by SIM/USIM or IMS infrastructure, not by static passwords.
#[derive(Clone)]
pub struct AkaClientConfig {
    provider: Arc<dyn AkaClientProvider>,
}

impl AkaClientConfig {
    /// Create an AKA config from a provider.
    pub fn new(provider: Arc<dyn AkaClientProvider>) -> Self {
        Self { provider }
    }

    fn respond(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
    ) -> Result<String> {
        self.provider
            .authorization(challenge_header, method, request_uri, nonce_count)
    }
}

impl std::fmt::Debug for AkaClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AkaClientConfig")
            .field("provider", &"<AkaClientProvider>")
            .finish()
    }
}

impl PartialEq for AkaClientConfig {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.provider, &other.provider)
    }
}

impl Eq for AkaClientConfig {}

/// UAC-side AKA response provider.
///
/// Applications implement this trait to calculate the AKA `Authorization`
/// header body for a selected `AKAv1-MD5` or `AKAv2-MD5` challenge. The
/// library does not embed SIM/USIM, Milenage, or carrier IMS infrastructure.
pub trait AkaClientProvider: Send + Sync {
    /// Build an AKA Authorization header body for the selected challenge.
    fn authorization(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
    ) -> Result<String>;
}

/// UAS-side AKA challenge and validation provider.
///
/// Applications implement this trait to issue IMS AKA vectors and validate
/// inbound AKA credentials. The provider owns vector storage, resync policy,
/// and any SIM/USIM or HSS/AuC integration.
#[async_trait]
pub trait AkaVectorProvider: Send + Sync {
    /// Validate an inbound AKA Authorization header and return the AKA user.
    async fn validate(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<Option<AuthIdentity>>;

    /// Build an AKA challenge value, usually a Digest-family challenge with
    /// `algorithm=AKAv1-MD5` or `algorithm=AKAv2-MD5`.
    fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge;
}

/// General UAS-side SIP authentication facade.
///
/// Enable one or more schemes, then pass the service to
/// `IncomingCall::authenticate_with`, `IncomingRequest::authenticate_with`, or
/// `IncomingRegister::authenticate_with`. Missing or rejected credentials
/// return challenges for the enabled schemes; accepted credentials return an
/// [`AuthIdentity`].
///
/// Bearer validators are responsible for token trust policy: issuer, audience
/// or resource indicators, expiry, accepted algorithms, `kid` behavior,
/// revocation/introspection strategy, and application-required scopes. A SIP
/// auth realm only labels the challenge; it is not a substitute for validator
/// policy.
#[derive(Clone)]
pub struct SipAuthService {
    policy: SipAuthPolicy,
    digest: Option<SipDigestAuthService>,
    digest_provider: Option<DigestProviderAuthStore>,
    bearer: Option<Arc<dyn BearerValidator>>,
    bearer_realm: Option<String>,
    bearer_scope: Option<String>,
    required_bearer_scope: Option<String>,
    basic: Option<BasicAuthStore>,
    aka: Option<Arc<dyn AkaVectorProvider>>,
    allow_bearer_over_cleartext: bool,
    allow_basic_over_cleartext: bool,
    audit_sink: Option<Arc<dyn AuthAuditSink>>,
    audit_failure_policy: AuditFailurePolicy,
    rate_limiter: Option<Arc<dyn AuthRateLimiter>>,
    digest_replay_store: Option<Arc<dyn DigestReplayStore>>,
}

impl SipAuthService {
    /// Create an empty UAS auth service. Add schemes with the `with_*`
    /// methods.
    pub fn new() -> Self {
        Self {
            policy: SipAuthPolicy::default(),
            digest: None,
            digest_provider: None,
            bearer: None,
            bearer_realm: None,
            bearer_scope: None,
            required_bearer_scope: None,
            basic: None,
            aka: None,
            allow_bearer_over_cleartext: false,
            allow_basic_over_cleartext: false,
            audit_sink: None,
            audit_failure_policy: AuditFailurePolicy::FailOpen,
            rate_limiter: None,
            digest_replay_store: None,
        }
    }

    /// Create a UAS service with a Digest realm.
    pub fn digest(realm: impl Into<String>) -> Self {
        Self::new().with_digest_service(SipDigestAuthService::new(realm))
    }

    /// Apply UAS authentication policy.
    pub fn with_policy(mut self, policy: SipAuthPolicy) -> Self {
        self.allow_basic_over_cleartext = policy.allow_basic_over_cleartext;
        self.allow_bearer_over_cleartext = policy.allow_bearer_over_cleartext;
        self.audit_failure_policy = policy.audit_failure_policy;
        self.policy = policy;
        self
    }

    /// Add a Digest service.
    pub fn with_digest_service(mut self, service: SipDigestAuthService) -> Self {
        self.digest = Some(service);
        self
    }

    /// Add provider-backed Digest validation for UAS requests.
    ///
    /// The provider supplies SIP Digest secret material, usually HA1 values
    /// from a user service. Nonce generation, stale handling, and nonce-count
    /// replay checks remain in `rvoip-sip`.
    pub fn with_digest_provider(
        mut self,
        realm: impl Into<String>,
        provider: Arc<dyn DigestSecretProvider>,
    ) -> Self {
        let mut digest = DigestProviderAuthStore::new(realm, provider);
        if let Some(replay_store) = self.digest_replay_store.clone() {
            digest = digest.with_replay_store(replay_store);
        }
        self.digest_provider = Some(digest);
        self
    }

    /// Select the Digest algorithm used for provider-backed challenges.
    ///
    /// Omitted Digest algorithms remain MD5 for legacy PBX compatibility, but
    /// first-party services should prefer SHA-256 or SHA-512-256 when peers
    /// support them.
    pub fn with_digest_provider_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        if let Some(digest) = self.digest_provider.take() {
            self.digest_provider = Some(digest.with_algorithm(algorithm));
        }
        self
    }

    /// Add shared Digest nonce/replay storage for provider-backed Digest.
    ///
    /// The existing sync [`Self::challenges`] helper remains in-memory only.
    /// Use [`Self::challenges_async`] or
    /// [`Self::authenticate_authorization_with_context`] when shared replay
    /// state must record newly issued nonces.
    pub fn with_digest_replay_store(mut self, replay_store: Arc<dyn DigestReplayStore>) -> Self {
        if let Some(digest) = self.digest_provider.take() {
            self.digest_provider = Some(digest.with_replay_store(replay_store.clone()));
        }
        self.digest_replay_store = Some(replay_store);
        self
    }

    /// Add a redacted audit sink for authentication events.
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuthAuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    /// Select how audit sink failures are handled.
    ///
    /// Default is [`AuditFailurePolicy::FailOpen`].
    pub fn with_audit_failure_policy(mut self, policy: AuditFailurePolicy) -> Self {
        self.audit_failure_policy = policy;
        self
    }

    /// Add rate-limit/lockout policy for inbound auth attempts.
    ///
    /// Rate-limiter provider errors fail closed. Authentication fails before
    /// credential validation if the limiter cannot answer.
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<dyn AuthRateLimiter>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Add a Bearer validator for UAS token validation.
    ///
    /// The validator comes from `rvoip-auth-core`; this facade maps successful
    /// validation into [`AuthIdentity`]. The validator must enforce issuer,
    /// audience/resource, expiry, allowed algorithms, `kid` handling,
    /// revocation/introspection requirements, and application scopes.
    pub fn with_bearer_validator(
        mut self,
        realm: impl Into<String>,
        validator: Arc<dyn BearerValidator>,
    ) -> Self {
        self.bearer = Some(validator);
        self.bearer_realm = Some(realm.into());
        self
    }

    /// Set Bearer challenge scope.
    pub fn with_bearer_scope(mut self, scope: impl Into<String>) -> Self {
        self.bearer_scope = Some(scope.into());
        self
    }

    /// Require one application scope on every accepted Bearer principal.
    ///
    /// [`Self::with_bearer_scope`] only advertises a requested OAuth scope in
    /// the SIP challenge for compatibility. This method is the enforceable
    /// listener policy boundary and rejects otherwise valid credentials that
    /// lack the configured scope (or wildcard scope).
    pub fn with_required_bearer_scope(mut self, scope: impl Into<String>) -> Self {
        self.required_bearer_scope = Some(scope.into());
        self
    }

    /// Add Basic validation with the given realm.
    ///
    /// Basic is rejected on cleartext SIP unless
    /// [`Self::allow_basic_over_cleartext`] is enabled.
    pub fn with_basic_realm(mut self, realm: impl Into<String>) -> Self {
        self.basic = Some(BasicAuthStore {
            realm: realm.into(),
            users: Arc::new(RwLock::new(HashMap::new())),
            verifier: None,
        });
        self
    }

    /// Add provider-backed Basic password validation.
    ///
    /// Basic remains rejected on cleartext SIP unless
    /// [`Self::allow_basic_over_cleartext`] is enabled.
    pub fn with_basic_verifier(
        mut self,
        realm: impl Into<String>,
        verifier: Arc<dyn PasswordVerifier>,
    ) -> Self {
        self.basic = Some(BasicAuthStore {
            realm: realm.into(),
            users: Arc::new(RwLock::new(HashMap::new())),
            verifier: Some(verifier),
        });
        self
    }

    /// Add or replace a Basic user. If Basic has not been enabled yet, it is
    /// enabled with the default `sip` realm.
    pub fn add_basic_user(&mut self, username: impl Into<String>, password: impl Into<String>) {
        if self.basic.is_none() {
            self.basic = Some(BasicAuthStore {
                realm: "sip".to_string(),
                users: Arc::new(RwLock::new(HashMap::new())),
                verifier: None,
            });
        }
        if let Some(basic) = &self.basic {
            let mut users = basic
                .users
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            users.insert(username.into(), password.into());
        }
    }

    /// Fluent form of [`Self::add_basic_user`].
    pub fn with_basic_user(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.add_basic_user(username, password);
        self
    }

    /// Permit Bearer validation on non-TLS SIP transports.
    ///
    /// This is an explicit legacy interoperability opt-in. Leave it disabled
    /// unless the application has a deployment-specific reason to accept
    /// cleartext bearer tokens on the wire.
    pub fn allow_bearer_over_cleartext(mut self, allow: bool) -> Self {
        self.allow_bearer_over_cleartext = allow;
        self
    }

    /// Permit Basic validation on non-TLS SIP transports.
    pub fn allow_basic_over_cleartext(mut self, allow: bool) -> Self {
        self.allow_basic_over_cleartext = allow;
        self
    }

    /// Add a provider-backed AKA validation provider.
    pub fn with_aka_provider(mut self, provider: Arc<dyn AkaVectorProvider>) -> Self {
        self.aka = Some(provider);
        self
    }

    /// Add a Digest user. If Digest has not been enabled yet, it is enabled
    /// with the default `sip` realm.
    pub fn add_digest_user(&mut self, username: impl Into<String>, password: impl Into<String>) {
        if self.digest.is_none() {
            self.digest = Some(SipDigestAuthService::new("sip"));
        }
        if let Some(digest) = &self.digest {
            digest.add_user(username, password);
        }
    }

    /// Fluent form of [`Self::add_digest_user`].
    pub fn with_digest_user(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.add_digest_user(username, password);
        self
    }

    /// Validate an optional inbound `Authorization` or `Proxy-Authorization`
    /// value.
    pub async fn authenticate_authorization(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        is_tls: bool,
    ) -> Result<SipAuthDecision> {
        self.authenticate_authorization_with_transport_context(
            authorization,
            method,
            request_uri,
            body,
            source,
            &SipTransportSecurityContext::from_is_tls(is_tls),
        )
        .await
    }

    /// Validate an optional inbound auth header using transport-truth security
    /// context for Basic/Bearer cleartext policy decisions.
    pub async fn authenticate_authorization_with_transport_context(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
    ) -> Result<SipAuthDecision> {
        self.authenticate_authorization_with_context_and_transport(
            authorization,
            method,
            request_uri,
            body,
            source,
            transport,
            &SipAuthContext::default(),
        )
        .await
    }

    /// Validate an optional inbound auth header with non-secret context for
    /// audit and rate-limit providers.
    pub async fn authenticate_authorization_with_context(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        is_tls: bool,
        context: &SipAuthContext,
    ) -> Result<SipAuthDecision> {
        self.authenticate_authorization_with_context_and_transport(
            authorization,
            method,
            request_uri,
            body,
            source,
            &SipTransportSecurityContext::from_is_tls(is_tls),
            context,
        )
        .await
    }

    /// Validate an optional inbound auth header with audit/rate-limit context
    /// and transport-truth security context.
    pub async fn authenticate_authorization_with_context_and_transport(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
        context: &SipAuthContext,
    ) -> Result<SipAuthDecision> {
        let mut principal = None;
        self.authenticate_authorization_with_context_and_transport_internal(
            authorization,
            method,
            request_uri,
            body,
            source,
            transport,
            context,
            &mut principal,
        )
        .await
    }

    /// Validate listener credentials while retaining the provider's complete
    /// canonical principal. Digest identities are promoted into a canonical
    /// SIP-Digest principal; Bearer validators retain issuer, tenant, expiry,
    /// method, assurance, and scopes from `validate_principal`.
    pub async fn authenticate_principal_with_context_and_transport(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
        context: &SipAuthContext,
    ) -> Result<SipPrincipalAuthDecision> {
        let mut principal = None;
        let decision = self
            .authenticate_authorization_with_context_and_transport_internal(
                authorization,
                method,
                request_uri,
                body,
                source,
                transport,
                context,
                &mut principal,
            )
            .await?;

        match decision {
            SipAuthDecision::Authorized(identity) => {
                let principal = principal
                    .or_else(|| principal_from_sip_auth_identity(&identity))
                    .ok_or_else(|| {
                        redacted_auth_failure(AuthFailureStage::PrincipalProjection, ())
                    })?;
                Ok(SipPrincipalAuthDecision::Authorized {
                    identity,
                    principal,
                })
            }
            SipAuthDecision::Rejected { challenges } => {
                Ok(SipPrincipalAuthDecision::Rejected { challenges })
            }
        }
    }

    async fn authenticate_authorization_with_context_and_transport_internal(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
        context: &SipAuthContext,
        principal_out: &mut Option<AuthenticatedPrincipal>,
    ) -> Result<SipAuthDecision> {
        let attempt = auth_attempt_scheme(authorization);
        let rate_key = self.rate_limit_key(attempt, authorization, method, context);

        let verdict = match self.check_rate_limit(&rate_key).await {
            Ok(verdict) => verdict,
            Err(error) => {
                let outcome = AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable);
                self.audit_attempt(attempt, outcome, authorization, source, method, context)
                    .await?;
                return Err(redacted_auth_failure(
                    AuthFailureStage::RateLimitCheck,
                    error,
                ));
            }
        };

        match verdict {
            AuthRateLimitVerdict::Allowed => {}
            AuthRateLimitVerdict::Denied { .. } => {
                let outcome = AuthAuditOutcome::Failure(AuthFailureReason::PolicyRejected);
                self.record_rate_result_or_audit_unavailable(
                    &rate_key,
                    &outcome,
                    attempt,
                    authorization,
                    source,
                    method,
                    context,
                )
                .await?;
                self.audit_attempt(attempt, outcome, authorization, source, method, context)
                    .await?;
                return self.rejected_async(source).await;
            }
        }

        let auth_result = match authorization {
            None => Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::MissingCredential),
            )),
            Some(authorization) => {
                let trimmed = authorization.trim();
                match attempt {
                    AuthAttemptScheme::Digest => {
                        if !self.policy.scheme_allowed(SipAuthScheme::Digest) {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else if self.policy.require_digest_replay_store
                            && self.digest_replay_store.is_none()
                        {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else if let Ok(response) =
                            DigestAuthenticator::parse_authorization(trimmed)
                        {
                            if !self.policy.digest_algorithm_allowed(response.algorithm) {
                                Ok((
                                    self.rejected_async(source).await?,
                                    Some(AuthFailureReason::PolicyRejected),
                                ))
                            } else {
                                self.authenticate_digest_with_reason(
                                    trimmed,
                                    method,
                                    request_uri,
                                    body,
                                    source,
                                )
                                .await
                            }
                        } else {
                            self.authenticate_digest_with_reason(
                                trimmed,
                                method,
                                request_uri,
                                body,
                                source,
                            )
                            .await
                        }
                    }
                    AuthAttemptScheme::Bearer => {
                        if !self.policy.scheme_allowed(SipAuthScheme::Bearer) {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else if !transport.is_secure() && !self.allow_bearer_over_cleartext {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else {
                            match self.authenticate_bearer_with_reason(trimmed, source).await {
                                Ok((decision, reason, principal)) => {
                                    *principal_out = principal;
                                    Ok((decision, reason))
                                }
                                Err(error) => Err(error),
                            }
                        }
                    }
                    AuthAttemptScheme::Basic => {
                        if !self.policy.scheme_allowed(SipAuthScheme::Basic) {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else if !transport.is_secure() && !self.allow_basic_over_cleartext {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else {
                            self.authenticate_basic_with_reason(trimmed, source, transport)
                                .await
                        }
                    }
                    AuthAttemptScheme::Aka => {
                        if !self.policy.scheme_allowed(SipAuthScheme::Aka) {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else {
                            self.authenticate_aka_with_reason(
                                trimmed,
                                method,
                                request_uri,
                                body,
                                source,
                            )
                            .await
                        }
                    }
                    AuthAttemptScheme::Unknown | AuthAttemptScheme::Missing => Ok((
                        self.rejected_async(source).await?,
                        Some(AuthFailureReason::UnsupportedScheme),
                    )),
                }
            }
        };

        let (result, failure_reason) = match auth_result {
            Ok(result) => result,
            Err(err) => {
                let outcome = AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable);
                self.record_rate_result_or_audit_unavailable(
                    &rate_key,
                    &outcome,
                    attempt,
                    authorization,
                    source,
                    method,
                    context,
                )
                .await?;
                self.audit_attempt(attempt, outcome, authorization, source, method, context)
                    .await?;
                return Err(err);
            }
        };

        let outcome = auth_outcome_for_decision(&result, failure_reason);
        self.record_rate_result_or_audit_unavailable(
            &rate_key,
            &outcome,
            attempt,
            authorization,
            source,
            method,
            context,
        )
        .await?;
        self.audit_attempt(attempt, outcome, authorization, source, method, context)
            .await?;
        Ok(result)
    }

    fn rate_limit_key(
        &self,
        attempt: AuthAttemptScheme,
        authorization: Option<&str>,
        method: &str,
        context: &SipAuthContext,
    ) -> AuthRateLimitKey {
        let (subject, realm) = subject_realm_from_authorization(authorization);
        let kind = if method.eq_ignore_ascii_case("REGISTER") {
            AuthRateLimitKind::SipRegister
        } else {
            attempt.rate_limit_kind()
        };
        let mut key = AuthRateLimitKey::new(kind);
        if let Some(subject) = subject {
            key = key.with_subject(subject);
        }
        if let Some(realm) = realm {
            key = key.with_realm(realm);
        }
        if let Some(peer) = context.peer.as_ref() {
            key = key.with_peer(peer.clone());
        }
        key
    }

    async fn check_rate_limit(
        &self,
        key: &AuthRateLimitKey,
    ) -> std::result::Result<AuthRateLimitVerdict, CredentialAuthError> {
        let Some(rate_limiter) = &self.rate_limiter else {
            return Ok(AuthRateLimitVerdict::Allowed);
        };
        rate_limiter.check_auth_attempt(key).await
    }

    async fn record_rate_result_or_audit_unavailable(
        &self,
        key: &AuthRateLimitKey,
        outcome: &AuthAuditOutcome,
        attempt: AuthAttemptScheme,
        authorization: Option<&str>,
        source: SipAuthSource,
        method: &str,
        context: &SipAuthContext,
    ) -> Result<()> {
        let Some(rate_limiter) = &self.rate_limiter else {
            return Ok(());
        };
        if let Err(error) = rate_limiter.record_auth_result(key, outcome).await {
            let provider_outcome =
                AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable);
            self.audit_attempt(
                attempt,
                provider_outcome,
                authorization,
                source,
                method,
                context,
            )
            .await?;
            return Err(redacted_auth_failure(
                AuthFailureStage::RateLimitRecord,
                error,
            ));
        }
        Ok(())
    }

    async fn audit_attempt(
        &self,
        attempt: AuthAttemptScheme,
        outcome: AuthAuditOutcome,
        authorization: Option<&str>,
        source: SipAuthSource,
        method: &str,
        context: &SipAuthContext,
    ) -> Result<()> {
        let Some(sink) = &self.audit_sink else {
            return Ok(());
        };
        let (subject, realm) = subject_realm_from_authorization(authorization);
        let mut event = AuthAuditEvent::new(attempt.audit_scheme(), outcome);
        if let Some(subject) = subject {
            event = event.with_subject(subject);
        }
        if let Some(realm) = realm {
            event = event.with_realm(realm);
        }
        if let Some(peer) = context.peer.as_ref() {
            event = event.with_peer(peer.clone());
        }
        event = event
            .with_metadata("method", method.to_ascii_uppercase())
            .with_metadata(
                "source",
                match source {
                    SipAuthSource::Origin => "origin",
                    SipAuthSource::Proxy => "proxy",
                },
            );
        for (key, value) in &context.metadata {
            event = event.with_metadata(key.clone(), value.clone());
        }

        match sink.record_auth_event(event).await {
            Ok(()) => Ok(()),
            Err(error) if self.audit_failure_policy == AuditFailurePolicy::FailOpen => {
                let _ = error;
                Ok(())
            }
            Err(error) => Err(redacted_auth_failure(AuthFailureStage::AuditSink, error)),
        }
    }

    fn challenges_with_digest_value(
        &self,
        source: SipAuthSource,
        digest_value: String,
    ) -> Vec<SipAuthChallenge> {
        let mut challenges = Vec::new();
        if self.policy.scheme_allowed(SipAuthScheme::Aka) {
            if let Some(aka) = &self.aka {
                challenges.push(aka.challenge(source));
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Bearer) {
            if let Some(bearer) = &self.bearer_realm {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Bearer,
                    value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                    source,
                });
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Digest) {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Digest,
                value: digest_value,
                source,
            });
        }
        if self.policy.scheme_allowed(SipAuthScheme::Basic) {
            if let Some(basic) = &self.basic {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Basic,
                    value: format!("Basic realm=\"{}\"", basic.realm),
                    source,
                });
            }
        }
        challenges
    }

    /// Build challenge header values for the enabled schemes.
    ///
    /// Use [`SipAuthSource::Origin`] for `WWW-Authenticate` / `401` and
    /// [`SipAuthSource::Proxy`] for `Proxy-Authenticate` / `407`.
    pub fn challenges(&self, source: SipAuthSource) -> Vec<SipAuthChallenge> {
        let mut challenges = Vec::new();
        if self.policy.scheme_allowed(SipAuthScheme::Aka) {
            if let Some(aka) = &self.aka {
                challenges.push(aka.challenge(source));
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Bearer) {
            if let Some(bearer) = &self.bearer_realm {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Bearer,
                    value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                    source,
                });
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Digest) {
            if let Some(digest) = &self.digest_provider {
                let challenge = digest.challenge();
                if self.policy.digest_algorithm_allowed(challenge.algorithm) {
                    challenges.push(SipAuthChallenge {
                        scheme: SipAuthScheme::Digest,
                        value: digest.www_authenticate(&challenge),
                        source,
                    });
                }
            } else if let Some(digest) = &self.digest {
                let challenge = digest.challenge();
                if self.policy.digest_algorithm_allowed(challenge.algorithm) {
                    challenges.push(SipAuthChallenge {
                        scheme: SipAuthScheme::Digest,
                        value: digest.www_authenticate(&challenge),
                        source,
                    });
                }
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Basic) {
            if let Some(basic) = &self.basic {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Basic,
                    value: format!("Basic realm=\"{}\"", basic.realm),
                    source,
                });
            }
        }
        challenges
    }

    /// Build challenge values and record issued Digest nonces in configured
    /// shared replay storage.
    pub async fn challenges_async(&self, source: SipAuthSource) -> Result<Vec<SipAuthChallenge>> {
        let mut challenges = Vec::new();
        if self.policy.scheme_allowed(SipAuthScheme::Aka) {
            if let Some(aka) = &self.aka {
                challenges.push(aka.challenge(source));
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Bearer) {
            if let Some(bearer) = &self.bearer_realm {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Bearer,
                    value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                    source,
                });
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Digest) {
            if let Some(digest) = &self.digest_provider {
                let challenge = digest.challenge_async().await?;
                if self.policy.digest_algorithm_allowed(challenge.algorithm) {
                    challenges.push(SipAuthChallenge {
                        scheme: SipAuthScheme::Digest,
                        value: digest.www_authenticate(&challenge),
                        source,
                    });
                }
            } else if let Some(digest) = &self.digest {
                let challenge = if let Some(replay_store) = &self.digest_replay_store {
                    digest
                        .challenge_with_replay_store(replay_store.clone())
                        .await?
                } else {
                    digest.challenge()
                };
                if self.policy.digest_algorithm_allowed(challenge.algorithm) {
                    challenges.push(SipAuthChallenge {
                        scheme: SipAuthScheme::Digest,
                        value: digest.www_authenticate(&challenge),
                        source,
                    });
                }
            }
        }
        if self.policy.scheme_allowed(SipAuthScheme::Basic) {
            if let Some(basic) = &self.basic {
                challenges.push(SipAuthChallenge {
                    scheme: SipAuthScheme::Basic,
                    value: format!("Basic realm=\"{}\"", basic.realm),
                    source,
                });
            }
        }
        Ok(challenges)
    }

    async fn rejected_async(&self, source: SipAuthSource) -> Result<SipAuthDecision> {
        Ok(SipAuthDecision::Rejected {
            challenges: self.challenges_async(source).await?,
        })
    }

    async fn authenticate_digest_with_reason(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
    ) -> Result<(SipAuthDecision, Option<AuthFailureReason>)> {
        if let Some(digest) = &self.digest_provider {
            return match digest
                .validate_authorization_detailed(authorization, method, request_uri, body)
                .await?
            {
                (AuthDecision::Authorized { username, realm }, failure_reason) => Ok((
                    SipAuthDecision::Authorized(AuthIdentity {
                        scheme: SipAuthScheme::Digest,
                        username: Some(username),
                        subject: None,
                        realm: Some(realm),
                        scopes: Vec::new(),
                        source,
                    }),
                    failure_reason,
                )),
                (
                    AuthDecision::Rejected {
                        www_authenticate, ..
                    },
                    failure_reason,
                ) => Ok((
                    SipAuthDecision::Rejected {
                        challenges: self.challenges_with_digest_value(source, www_authenticate),
                    },
                    failure_reason,
                )),
            };
        }
        let Some(digest) = &self.digest else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
            ));
        };
        let digest_decision = if let Some(replay_store) = &self.digest_replay_store {
            digest
                .validate_authorization_with_replay_store(
                    authorization,
                    method,
                    request_uri,
                    body,
                    replay_store.clone(),
                )
                .await?
        } else {
            digest.validate_authorization(authorization, method, request_uri, body)?
        };
        match digest_decision {
            AuthDecision::Authorized { username, realm } => Ok((
                SipAuthDecision::Authorized(AuthIdentity {
                    scheme: SipAuthScheme::Digest,
                    username: Some(username),
                    subject: None,
                    realm: Some(realm),
                    scopes: Vec::new(),
                    source,
                }),
                None,
            )),
            AuthDecision::Rejected {
                www_authenticate, ..
            } => {
                let reason = if www_authenticate.contains("stale=true") {
                    AuthFailureReason::StaleNonce
                } else {
                    AuthFailureReason::InvalidCredential
                };
                Ok((
                    SipAuthDecision::Rejected {
                        challenges: self.challenges_with_digest_value(source, www_authenticate),
                    },
                    Some(reason),
                ))
            }
        }
    }

    async fn authenticate_bearer_with_reason(
        &self,
        authorization: &str,
        source: SipAuthSource,
    ) -> Result<(
        SipAuthDecision,
        Option<AuthFailureReason>,
        Option<AuthenticatedPrincipal>,
    )> {
        let Some(validator) = &self.bearer else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
                None,
            ));
        };
        let token = authorization
            .split_once(char::is_whitespace)
            .map(|(_, value)| value.trim())
            .unwrap_or_default();
        match validator.validate_principal(token).await {
            Ok(principal) if principal.is_expired() => Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::InvalidCredential),
                None,
            )),
            Ok(principal)
                if self
                    .required_bearer_scope
                    .as_deref()
                    .is_some_and(|scope| !principal.has_scope(scope)) =>
            {
                Ok((
                    self.rejected_async(source).await?,
                    Some(AuthFailureReason::PolicyRejected),
                    None,
                ))
            }
            Ok(principal) => Ok((
                SipAuthDecision::Authorized(identity_from_bearer_principal(
                    &principal,
                    self.bearer_realm.clone(),
                    source,
                )),
                None,
                Some(principal),
            )),
            Err(BearerAuthError::Empty) | Err(BearerAuthError::Invalid(_)) => Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::InvalidCredential),
                None,
            )),
            Err(BearerAuthError::Unavailable(error)) => Err(redacted_auth_failure(
                AuthFailureStage::BearerValidator,
                error,
            )),
        }
    }

    async fn authenticate_basic_with_reason(
        &self,
        authorization: &str,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
    ) -> Result<(SipAuthDecision, Option<AuthFailureReason>)> {
        let Some(basic) = &self.basic else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
            ));
        };
        if !transport.is_secure() && !self.allow_basic_over_cleartext {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::PolicyRejected),
            ));
        }
        let token = authorization
            .split_once(char::is_whitespace)
            .map(|(_, value)| value.trim())
            .unwrap_or_default();
        let decoded = match BASE64_STANDARD.decode(token) {
            Ok(decoded) => decoded,
            Err(_) => {
                return Ok((
                    self.rejected_async(source).await?,
                    Some(AuthFailureReason::MalformedCredential),
                ))
            }
        };
        let decoded = match String::from_utf8(decoded) {
            Ok(decoded) => decoded,
            Err(_) => {
                return Ok((
                    self.rejected_async(source).await?,
                    Some(AuthFailureReason::MalformedCredential),
                ))
            }
        };
        let Some((username, password)) = decoded.split_once(':') else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::MalformedCredential),
            ));
        };
        if let Some(verifier) = &basic.verifier {
            return match verifier.verify_password(username, password).await {
                Ok(assurance) => {
                    let mut identity = identity_from_bearer_assurance(
                        assurance,
                        Some(basic.realm.clone()),
                        source,
                    );
                    identity.scheme = SipAuthScheme::Basic;
                    identity.username = Some(username.to_string());
                    Ok((SipAuthDecision::Authorized(identity), None))
                }
                Err(CredentialAuthError::Invalid) => Ok((
                    self.rejected_async(source).await?,
                    Some(AuthFailureReason::InvalidCredential),
                )),
                Err(CredentialAuthError::PolicyRejected(_)) => Ok((
                    self.rejected_async(source).await?,
                    Some(AuthFailureReason::PolicyRejected),
                )),
                Err(error) => Err(redacted_auth_failure(
                    AuthFailureStage::BasicVerifier,
                    error,
                )),
            };
        }
        let valid = {
            let users = basic
                .users
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            users.get(username).is_some_and(|stored| stored == password)
        };
        if !valid {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::InvalidCredential),
            ));
        }
        Ok((
            SipAuthDecision::Authorized(AuthIdentity {
                scheme: SipAuthScheme::Basic,
                username: Some(username.to_string()),
                subject: None,
                realm: Some(basic.realm.clone()),
                scopes: Vec::new(),
                source,
            }),
            None,
        ))
    }

    async fn authenticate_aka_with_reason(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
    ) -> Result<(SipAuthDecision, Option<AuthFailureReason>)> {
        let Some(aka) = &self.aka else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
            ));
        };
        match aka
            .validate(authorization, method, request_uri, body)
            .await
            .map_err(|error| redacted_auth_failure(AuthFailureStage::AkaVectorProvider, error))?
        {
            Some(mut identity) => {
                identity.scheme = SipAuthScheme::Aka;
                identity.source = source;
                Ok((SipAuthDecision::Authorized(identity), None))
            }
            None => Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::InvalidCredential),
            )),
        }
    }
}

impl Default for SipAuthService {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait implemented by UAS auth services that can validate inbound SIP
/// request credentials from typed incoming API surfaces.
///
/// Application code normally uses [`SipAuthService`] or
/// [`SipDigestAuthService`], both of which implement this trait.
#[async_trait]
pub trait SipIncomingAuthenticator {
    /// Decision type returned by the service.
    type Decision: Send;

    /// Validate the selected inbound auth header.
    async fn authenticate_incoming(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        is_tls: bool,
    ) -> Result<Self::Decision>;

    /// Validate the selected inbound auth header with transport-truth security
    /// context.
    ///
    /// The default bridges to the legacy boolean TLS method. Auth services
    /// that enforce scheme policy should override this method so callers can
    /// pass concrete TLS/WSS metadata without losing detail.
    async fn authenticate_incoming_with_transport_context(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
    ) -> Result<Self::Decision> {
        self.authenticate_incoming(
            authorization,
            method,
            request_uri,
            body,
            source,
            transport.is_secure(),
        )
        .await
    }
}

#[async_trait]
impl SipIncomingAuthenticator for SipDigestAuthService {
    type Decision = AuthDecision;

    async fn authenticate_incoming(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        _source: SipAuthSource,
        _is_tls: bool,
    ) -> Result<Self::Decision> {
        self.authenticate_authorization(authorization, method, request_uri, body)
    }
}

#[async_trait]
impl SipIncomingAuthenticator for SipAuthService {
    type Decision = SipAuthDecision;

    async fn authenticate_incoming(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        is_tls: bool,
    ) -> Result<Self::Decision> {
        self.authenticate_authorization(authorization, method, request_uri, body, source, is_tls)
            .await
    }

    async fn authenticate_incoming_with_transport_context(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        source: SipAuthSource,
        transport: &SipTransportSecurityContext,
    ) -> Result<Self::Decision> {
        self.authenticate_authorization_with_transport_context(
            authorization,
            method,
            request_uri,
            body,
            source,
            transport,
        )
        .await
    }
}

impl std::fmt::Debug for SipAuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SipAuthService")
            .field("policy", &self.policy)
            .field("digest", &self.digest.is_some())
            .field("digest_provider", &self.digest_provider.is_some())
            .field("bearer", &self.bearer.is_some())
            .field("bearer_realm_present", &self.bearer_realm.is_some())
            .field("bearer_scope_present", &self.bearer_scope.is_some())
            .field(
                "required_bearer_scope_present",
                &self.required_bearer_scope.is_some(),
            )
            .field("basic", &self.basic.is_some())
            .field("aka", &self.aka.is_some())
            .field(
                "allow_bearer_over_cleartext",
                &self.allow_bearer_over_cleartext,
            )
            .field(
                "allow_basic_over_cleartext",
                &self.allow_basic_over_cleartext,
            )
            .field("audit_sink", &self.audit_sink.is_some())
            .field("audit_failure_policy", &self.audit_failure_policy)
            .field("rate_limiter", &self.rate_limiter.is_some())
            .field("digest_replay_store", &self.digest_replay_store.is_some())
            .finish()
    }
}

#[derive(Clone)]
struct DigestProviderAuthStore {
    authenticator: DigestAuthenticator,
    algorithm: DigestAlgorithm,
    realm: String,
    provider: Arc<dyn DigestSecretProvider>,
    nonces: Arc<RwLock<HashMap<String, Instant>>>,
    nonce_counts: Arc<RwLock<HashMap<(String, String, String), u32>>>,
    nonce_ttl: Duration,
    replay_store: Option<Arc<dyn DigestReplayStore>>,
}

impl DigestProviderAuthStore {
    fn new(realm: impl Into<String>, provider: Arc<dyn DigestSecretProvider>) -> Self {
        let realm = realm.into();
        Self {
            authenticator: DigestAuthenticator::new(realm.clone()),
            algorithm: DigestAlgorithm::MD5,
            realm,
            provider,
            nonces: Arc::new(RwLock::new(HashMap::new())),
            nonce_counts: Arc::new(RwLock::new(HashMap::new())),
            nonce_ttl: Duration::from_secs(300),
            replay_store: None,
        }
    }

    fn with_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        self.authenticator = self.authenticator.with_algorithm(algorithm);
        self.algorithm = algorithm;
        self
    }

    fn with_replay_store(mut self, replay_store: Arc<dyn DigestReplayStore>) -> Self {
        self.replay_store = Some(replay_store);
        self
    }

    fn challenge(&self) -> DigestChallenge {
        let mut challenge = self.authenticator.generate_challenge();
        challenge.nonce = admit_local_digest_nonce(
            self.nonces.as_ref(),
            self.nonce_counts.as_ref(),
            &challenge.nonce,
            self.nonce_ttl,
        );
        challenge
    }

    async fn challenge_async(&self) -> Result<DigestChallenge> {
        let mut challenge = self.authenticator.generate_challenge();
        if let Some(replay_store) = &self.replay_store {
            challenge.nonce = replay_store
                .admit_nonce(&challenge.nonce, system_time_after(self.nonce_ttl))
                .await
                .map_err(|error| {
                    redacted_auth_failure(AuthFailureStage::ReplayNonceRecord, error)
                })?;
        } else {
            challenge.nonce = admit_local_digest_nonce(
                self.nonces.as_ref(),
                self.nonce_counts.as_ref(),
                &challenge.nonce,
                self.nonce_ttl,
            );
        }
        Ok(challenge)
    }

    fn www_authenticate(&self, challenge: &DigestChallenge) -> String {
        self.authenticator.format_www_authenticate(challenge)
    }

    fn www_authenticate_with_stale(&self, challenge: &DigestChallenge, stale: bool) -> String {
        self.authenticator
            .format_www_authenticate_with_stale(challenge, stale)
    }

    async fn validate_authorization_detailed(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<(AuthDecision, Option<AuthFailureReason>)> {
        let response = match DigestAuthenticator::parse_authorization(authorization) {
            Ok(response) => response,
            Err(_) => {
                return self
                    .rejected_with_reason(AuthFailureReason::MalformedCredential)
                    .await
            }
        };

        if response.uri != request_uri
            || response.realm != self.realm
            || response.algorithm != self.algorithm
            || digest_nonce_count(&response).is_none()
        {
            return self
                .rejected_with_reason(AuthFailureReason::InvalidCredential)
                .await;
        }

        match self.nonce_status_async(&response.nonce).await? {
            NonceStatus::Active => {}
            NonceStatus::Expired => {
                return self
                    .rejected_stale_with_reason(AuthFailureReason::StaleNonce)
                    .await
            }
            NonceStatus::Unknown => {
                return self
                    .rejected_with_reason(AuthFailureReason::InvalidCredential)
                    .await
            }
        }

        let secret = match self
            .provider
            .lookup_digest_secret(&response.username, &response.realm, response.algorithm)
            .await
        {
            Ok(Some(secret)) => secret,
            Ok(None) | Err(CredentialAuthError::Invalid) => {
                return self
                    .rejected_with_reason(AuthFailureReason::InvalidCredential)
                    .await
            }
            Err(CredentialAuthError::PolicyRejected(_)) => {
                return self
                    .rejected_with_reason(AuthFailureReason::PolicyRejected)
                    .await
            }
            Err(error) => {
                return Err(redacted_auth_failure(
                    AuthFailureStage::DigestSecretProvider,
                    error,
                ))
            }
        };

        let valid = match self
            .authenticator
            .validate_response_with_secret_and_body(&response, method, &secret, body)
        {
            Ok(valid) => valid,
            Err(_) => {
                return self
                    .rejected_with_reason(AuthFailureReason::UnsupportedScheme)
                    .await
            }
        };
        if !valid {
            return self
                .rejected_with_reason(AuthFailureReason::InvalidCredential)
                .await;
        }

        if let Some(reason) = self.accept_nonce_count_async(&response).await? {
            return self.rejected_with_reason(reason).await;
        }

        Ok((
            AuthDecision::Authorized {
                username: response.username,
                realm: response.realm,
            },
            None,
        ))
    }

    fn nonce_status(&self, nonce: &str) -> NonceStatus {
        let now = Instant::now();
        let mut nonces = self
            .nonces
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match nonces.get(nonce).copied() {
            Some(expires_at) if expires_at > now => NonceStatus::Active,
            Some(_) => {
                nonces.remove(nonce);
                drop(nonces);
                self.nonce_counts
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .retain(|(_, recorded_nonce, _), _| recorded_nonce != nonce);
                NonceStatus::Expired
            }
            None => NonceStatus::Unknown,
        }
    }

    fn accept_nonce_count(&self, response: &DigestResponse) -> bool {
        accept_local_digest_nonce_count(self.nonce_counts.as_ref(), response)
    }

    async fn nonce_status_async(&self, nonce: &str) -> Result<NonceStatus> {
        let Some(replay_store) = &self.replay_store else {
            return Ok(self.nonce_status(nonce));
        };
        match replay_store
            .nonce_status(nonce, SystemTime::now())
            .await
            .map_err(|error| redacted_auth_failure(AuthFailureStage::ReplayNonceStatus, error))?
        {
            DigestNonceStatus::Active => Ok(NonceStatus::Active),
            DigestNonceStatus::Expired => Ok(NonceStatus::Expired),
            DigestNonceStatus::Unknown => Ok(NonceStatus::Unknown),
        }
    }

    async fn accept_nonce_count_async(
        &self,
        response: &DigestResponse,
    ) -> Result<Option<AuthFailureReason>> {
        let Some(nc) = digest_nonce_count(response) else {
            return Ok(Some(AuthFailureReason::MalformedCredential));
        };
        let accepted = if let Some(replay_store) = &self.replay_store {
            replay_store
                .accept_client_nonce_count(
                    &response.username,
                    &response.nonce,
                    response
                        .cnonce
                        .as_deref()
                        .expect("validated by digest_nonce_count"),
                    nc,
                    SystemTime::now(),
                )
                .await
                .map_err(|error| redacted_auth_failure(AuthFailureStage::ReplayNonceCount, error))?
        } else {
            self.accept_nonce_count(response)
        };
        if accepted {
            Ok(None)
        } else {
            Ok(Some(AuthFailureReason::ReplayRejected))
        }
    }

    async fn rejected_async(&self) -> Result<AuthDecision> {
        let challenge = self.challenge_async().await?;
        let www_authenticate = self.www_authenticate(&challenge);
        Ok(AuthDecision::Rejected {
            challenge,
            www_authenticate,
        })
    }

    async fn rejected_stale_async(&self) -> Result<AuthDecision> {
        let challenge = self.challenge_async().await?;
        let www_authenticate = self.www_authenticate_with_stale(&challenge, true);
        Ok(AuthDecision::Rejected {
            challenge,
            www_authenticate,
        })
    }

    async fn rejected_with_reason(
        &self,
        reason: AuthFailureReason,
    ) -> Result<(AuthDecision, Option<AuthFailureReason>)> {
        Ok((self.rejected_async().await?, Some(reason)))
    }

    async fn rejected_stale_with_reason(
        &self,
        reason: AuthFailureReason,
    ) -> Result<(AuthDecision, Option<AuthFailureReason>)> {
        Ok((self.rejected_stale_async().await?, Some(reason)))
    }
}

#[derive(Clone)]
struct BasicAuthStore {
    realm: String,
    users: Arc<RwLock<HashMap<String, String>>>,
    verifier: Option<Arc<dyn PasswordVerifier>>,
}

fn identity_from_bearer_assurance(
    assurance: IdentityAssurance,
    realm: Option<String>,
    source: SipAuthSource,
) -> AuthIdentity {
    match assurance {
        IdentityAssurance::UserAuthorized {
            user_id, scopes, ..
        } => AuthIdentity {
            scheme: SipAuthScheme::Bearer,
            username: None,
            subject: Some(user_id.to_string()),
            realm,
            scopes,
            source,
        },
        IdentityAssurance::TaskScoped {
            identity,
            task_id,
            scopes,
            ..
        } => AuthIdentity {
            scheme: SipAuthScheme::Bearer,
            username: None,
            subject: Some(format!("{}:{}", identity, task_id)),
            realm,
            scopes,
            source,
        },
        other => AuthIdentity {
            scheme: SipAuthScheme::Bearer,
            username: None,
            subject: Some(fallback_bearer_assurance_subject(&other)),
            realm,
            scopes: Vec::new(),
            source,
        },
    }
}

/// Derive a stable, credential-free subject when a legacy Bearer validator
/// returns assurance without a first-class principal subject.
///
/// The assurance variant is an explicit domain separator and every retained
/// field is length-prefixed before SHA-256. This keeps subjects distinct
/// without copying JWKs or DTLS fingerprints into application-visible state.
/// Diagnostic formatting is intentionally not involved: `Debug` is redacted
/// and does not have protocol stability guarantees.
fn fallback_bearer_assurance_subject(assurance: &IdentityAssurance) -> String {
    let (kind, fields): (&'static str, Vec<Vec<u8>>) = match assurance {
        IdentityAssurance::Anonymous => ("anonymous", Vec::new()),
        IdentityAssurance::Pseudonymous { ephemeral_key } => (
            "pseudonymous",
            vec![serde_json::to_vec(&ephemeral_key.0)
                .expect("serde_json::Value is always serializable")],
        ),
        IdentityAssurance::Identified { credential_kind } => (
            "identified",
            vec![credential_kind_label(*credential_kind).as_bytes().to_vec()],
        ),
        IdentityAssurance::DtlsFingerprint { algorithm, value } => (
            "dtls-fingerprint",
            vec![algorithm.as_bytes().to_vec(), value.as_bytes().to_vec()],
        ),
        // These variants are handled above because they carry a meaningful
        // subject and scopes. Keep this branch total if the matching logic is
        // refactored later, without ever falling back to `Debug`.
        IdentityAssurance::TaskScoped {
            identity, task_id, ..
        } => (
            "task-scoped",
            vec![
                identity.as_str().as_bytes().to_vec(),
                task_id.as_bytes().to_vec(),
            ],
        ),
        IdentityAssurance::UserAuthorized { user_id, .. } => (
            "user-authorized",
            vec![user_id.as_str().as_bytes().to_vec()],
        ),
    };

    let mut digest = Sha256::new();
    digest.update(b"rvoip:sip:bearer-assurance-subject:v1\0");
    update_length_prefixed(&mut digest, kind.as_bytes());
    for field in fields {
        update_length_prefixed(&mut digest, &field);
    }
    let digest = digest.finalize();
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut fingerprint, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("urn:rvoip:sip:bearer-assurance:{kind}:sha256:{fingerprint}")
}

fn update_length_prefixed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

const fn credential_kind_label(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::OAuth2Dpop => "oauth2-dpop",
        CredentialKind::Oidc => "oidc",
        CredentialKind::SipDigest => "sip-digest",
        CredentialKind::Passkey => "passkey",
        CredentialKind::AAuth => "aauth",
    }
}

fn identity_from_bearer_principal(
    principal: &AuthenticatedPrincipal,
    realm: Option<String>,
    source: SipAuthSource,
) -> AuthIdentity {
    AuthIdentity {
        scheme: SipAuthScheme::Bearer,
        username: None,
        subject: Some(principal.subject.clone()),
        realm,
        scopes: principal.scopes.clone(),
        source,
    }
}

fn principal_from_sip_auth_identity(identity: &AuthIdentity) -> Option<AuthenticatedPrincipal> {
    if identity.scheme != SipAuthScheme::Digest {
        return None;
    }
    let subject = identity
        .username
        .clone()
        .or_else(|| identity.subject.clone())?;
    Some(AuthenticatedPrincipal {
        subject,
        tenant: None,
        scopes: identity.scopes.clone(),
        issuer: identity
            .realm
            .as_ref()
            .map(|realm| format!("sip-digest:{realm}")),
        expires_at: None,
        method: AuthenticationMethod::SipDigest,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::SipDigest,
        },
    })
}

fn auth_attempt_scheme(authorization: Option<&str>) -> AuthAttemptScheme {
    let Some(authorization) = authorization
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return AuthAttemptScheme::Missing;
    };
    let lower = authorization.to_ascii_lowercase();
    if lower.starts_with("bearer ") {
        AuthAttemptScheme::Bearer
    } else if lower.starts_with("basic ") {
        AuthAttemptScheme::Basic
    } else if lower.starts_with("digest ") {
        if contains_aka_challenge(authorization) {
            AuthAttemptScheme::Aka
        } else {
            AuthAttemptScheme::Digest
        }
    } else {
        AuthAttemptScheme::Unknown
    }
}

fn auth_outcome_for_decision(
    decision: &SipAuthDecision,
    failure_reason: Option<AuthFailureReason>,
) -> AuthAuditOutcome {
    match decision {
        SipAuthDecision::Authorized(_) => AuthAuditOutcome::Success,
        SipAuthDecision::Rejected { .. } => AuthAuditOutcome::Failure(
            failure_reason.unwrap_or(AuthFailureReason::InvalidCredential),
        ),
    }
}

fn subject_realm_from_authorization(
    authorization: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(authorization) = authorization
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return (None, None);
    };
    let lower = authorization.to_ascii_lowercase();
    if lower.starts_with("digest ") {
        return DigestAuthenticator::parse_authorization(authorization)
            .map(|response| (Some(response.username), Some(response.realm)))
            .unwrap_or((None, None));
    }
    if lower.starts_with("basic ") {
        let token = authorization
            .split_once(char::is_whitespace)
            .map(|(_, value)| value.trim())
            .unwrap_or_default();
        return BASE64_STANDARD
            .decode(token)
            .ok()
            .and_then(|decoded| String::from_utf8(decoded).ok())
            .and_then(|decoded| {
                decoded
                    .split_once(':')
                    .map(|(username, _)| username.to_string())
            })
            .map(|username| (Some(username), None))
            .unwrap_or((None, None));
    }
    (None, None)
}

fn system_time_after(duration: Duration) -> SystemTime {
    SystemTime::now()
        .checked_add(duration)
        .unwrap_or_else(SystemTime::now)
}

async fn accept_nonce_count_with_replay_store(
    response: &DigestResponse,
    replay_store: &dyn DigestReplayStore,
) -> Result<bool> {
    let Some(nc) = digest_nonce_count(response) else {
        return Ok(false);
    };
    replay_store
        .accept_client_nonce_count(
            &response.username,
            &response.nonce,
            response
                .cnonce
                .as_deref()
                .expect("validated by digest_nonce_count"),
            nc,
            SystemTime::now(),
        )
        .await
        .map_err(|error| redacted_auth_failure(AuthFailureStage::ReplayNonceCount, error))
}

fn bearer_challenge_value(
    realm: &str,
    scope: Option<&str>,
    error: Option<&str>,
    error_description: Option<&str>,
) -> String {
    let mut value = format!("Bearer realm=\"{realm}\"");
    if let Some(scope) = scope {
        value.push_str(&format!(", scope=\"{scope}\""));
    }
    if let Some(error) = error {
        value.push_str(&format!(", error=\"{error}\""));
    }
    if let Some(error_description) = error_description {
        value.push_str(&format!(", error_description=\"{error_description}\""));
    }
    value
}

fn contains_auth_scheme(value: &str, scheme: &str) -> bool {
    split_auth_challenges(value).into_iter().any(|challenge| {
        let trimmed = challenge.trim_start();
        let token = trimmed
            .split_once(char::is_whitespace)
            .map(|(token, _)| token)
            .unwrap_or(trimmed);
        token.eq_ignore_ascii_case(scheme)
    })
}

fn contains_aka_challenge(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    upper.contains("AKAV1-MD5") || upper.contains("AKAV2-MD5")
}

fn extract_digest_challenge(value: &str) -> Option<String> {
    let mut best = None;
    for challenge in split_auth_challenges(value)
        .into_iter()
        .filter(|challenge| {
            challenge
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("digest ")
        })
    {
        let Ok(parsed) = rvoip_auth_core::DigestAuthenticator::parse_challenge(&challenge) else {
            if best.is_none() {
                best = Some((0, challenge));
            }
            continue;
        };
        let strength = digest_algorithm_strength(parsed.algorithm);
        if best
            .as_ref()
            .map_or(true, |(best_strength, _)| strength > *best_strength)
        {
            best = Some((strength, challenge));
        }
    }
    best.map(|(_, challenge)| challenge)
}

fn parse_digest_stale(value: &str) -> bool {
    let Some(challenge) = extract_digest_challenge(value) else {
        return false;
    };
    rvoip_auth_core::DigestAuthenticator::parse_challenge_details(&challenge)
        .map(|details| details.stale)
        .unwrap_or(false)
}

fn digest_algorithm_strength(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::SHA512256Sess => 60,
        DigestAlgorithm::SHA512256 => 50,
        DigestAlgorithm::SHA256Sess => 40,
        DigestAlgorithm::SHA256 => 30,
        DigestAlgorithm::MD5Sess => 20,
        DigestAlgorithm::MD5 => 10,
    }
}

fn split_auth_challenges(value: &str) -> Vec<String> {
    let mut challenges = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let chars: Vec<(usize, char)> = value.char_indices().collect();

    for (position, (idx, ch)) in chars.iter().copied().enumerate() {
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if ch != ',' || in_quotes {
            continue;
        }
        let next_idx = idx + ch.len_utf8();
        let rest = &value[next_idx..];
        let trimmed = rest.trim_start();
        if looks_like_auth_challenge_start(trimmed) {
            let current = value[start..idx].trim();
            if !current.is_empty() {
                challenges.push(current.to_string());
            }
            let whitespace = rest.len() - trimmed.len();
            start = next_idx + whitespace;
        }

        if position + 1 == chars.len() {
            break;
        }
    }

    let current = value[start..].trim();
    if !current.is_empty() {
        challenges.push(current.to_string());
    }
    challenges
}

fn looks_like_auth_challenge_start(value: &str) -> bool {
    let mut token_len = 0;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            token_len += ch.len_utf8();
        } else {
            break;
        }
    }
    if token_len == 0 {
        return false;
    }
    let rest = &value[token_len..];
    rest.starts_with(char::is_whitespace)
}

fn select_composite_client_auth(
    auths: &[SipClientAuth],
    challenge_header: &str,
    method: &str,
    request_uri: &str,
    nonce_count: u32,
    body: Option<&[u8]>,
    transport: &SipTransportSecurityContext,
) -> Result<ClientAuthHeader> {
    let priorities: &[fn(&SipClientAuth) -> bool] = &[
        |auth| matches!(auth, SipClientAuth::Aka(_)),
        |auth| {
            matches!(
                auth,
                SipClientAuth::BearerToken(_) | SipClientAuth::BearerTokenCleartextAllowed(_)
            )
        },
        |auth| matches!(auth, SipClientAuth::Digest(_)),
        |auth| matches!(auth, SipClientAuth::Basic { .. }),
    ];

    for matches_priority in priorities {
        for auth in auths.iter().filter(|auth| matches_priority(auth)) {
            if let Ok(header) = auth.authorization_for_challenge_with_transport_context(
                challenge_header,
                method,
                request_uri,
                nonce_count,
                body,
                transport,
            ) {
                return Ok(header);
            }
        }
    }

    Err(SessionError::AuthError(
        "no configured auth option can answer the challenge".to_string(),
    ))
}

/// Result of evaluating inbound SIP Digest authentication with
/// [`SipDigestAuthService`].
#[derive(Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// The inbound request carried a valid digest response.
    Authorized {
        /// Authenticated digest username.
        username: String,
        /// Realm from the digest response.
        realm: String,
    },
    /// The inbound request should be challenged or rejected.
    Rejected {
        /// Fresh challenge to send to the peer.
        challenge: DigestChallenge,
        /// Formatted `WWW-Authenticate` header value.
        www_authenticate: String,
    },
}

impl fmt::Debug for AuthDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authorized { username, realm } => formatter
                .debug_struct("Authorized")
                .field("username_present", &!username.is_empty())
                .field("realm_present", &!realm.is_empty())
                .finish(),
            Self::Rejected {
                challenge,
                www_authenticate,
            } => formatter
                .debug_struct("Rejected")
                .field("challenge_present", &!challenge.nonce.is_empty())
                .field("www_authenticate_present", &!www_authenticate.is_empty())
                .field("www_authenticate_len", &www_authenticate.len())
                .finish(),
        }
    }
}

/// UAS-side SIP Digest authentication facade.
///
/// The service stores a simple in-memory username/password map, generates typed
/// challenges, formats `WWW-Authenticate`, and validates inbound
/// `Authorization` values through `rvoip-auth-core`.
///
/// Prefer [`SipAuthService`] for new applications that need Bearer, Basic, AKA,
/// or multi-challenge UAS behavior. This type remains the small Digest-only
/// compatibility wrapper.
#[derive(Clone)]
pub struct SipDigestAuthService {
    authenticator: DigestAuthenticator,
    algorithm: DigestAlgorithm,
    realm: String,
    users: Arc<RwLock<HashMap<String, DigestVerifierSet>>>,
    nonces: Arc<RwLock<HashMap<String, Instant>>>,
    nonce_counts: Arc<RwLock<HashMap<(String, String, String), u32>>>,
    nonce_ttl: Duration,
}

impl SipDigestAuthService {
    /// Create a digest service for the given realm.
    pub fn new(realm: impl Into<String>) -> Self {
        let realm = realm.into();
        Self {
            authenticator: DigestAuthenticator::new(realm.clone()),
            algorithm: DigestAlgorithm::MD5,
            realm,
            users: Arc::new(RwLock::new(HashMap::new())),
            nonces: Arc::new(RwLock::new(HashMap::new())),
            nonce_counts: Arc::new(RwLock::new(HashMap::new())),
            nonce_ttl: Duration::from_secs(300),
        }
    }

    /// Select the algorithm used for generated challenges.
    pub fn with_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        self.authenticator = self.authenticator.with_algorithm(algorithm);
        self.algorithm = algorithm;
        self
    }

    /// Set how long generated nonces remain valid.
    pub fn with_nonce_ttl(mut self, ttl: Duration) -> Self {
        self.nonce_ttl = ttl;
        self
    }

    /// Add or replace a digest user.
    pub fn add_user(&self, username: impl Into<String>, password: impl Into<String>) {
        let username = username.into();
        let verifier = DigestVerifierSet::from_password(&username, &self.realm, password.into());
        let mut users = self
            .users
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        users.insert(username, verifier);
    }

    /// Generate a fresh digest challenge.
    pub fn challenge(&self) -> DigestChallenge {
        let mut challenge = self.authenticator.generate_challenge();
        challenge.nonce = admit_local_digest_nonce(
            self.nonces.as_ref(),
            self.nonce_counts.as_ref(),
            &challenge.nonce,
            self.nonce_ttl,
        );
        challenge
    }

    /// Generate a challenge and record its nonce in a shared replay store.
    ///
    /// Use this in clustered deployments that keep using the Digest-only
    /// compatibility service. Single-process deployments can continue using
    /// [`Self::challenge`].
    pub async fn challenge_with_replay_store(
        &self,
        replay_store: Arc<dyn DigestReplayStore>,
    ) -> Result<DigestChallenge> {
        let challenge = self.authenticator.generate_challenge();
        let mut challenge = challenge;
        challenge.nonce = replay_store
            .admit_nonce(&challenge.nonce, system_time_after(self.nonce_ttl))
            .await
            .map_err(|error| redacted_auth_failure(AuthFailureStage::ReplayNonceRecord, error))?;
        Ok(challenge)
    }

    /// Format a challenge as a `WWW-Authenticate` header value.
    pub fn www_authenticate(&self, challenge: &DigestChallenge) -> String {
        self.authenticator.format_www_authenticate(challenge)
    }

    /// Format a challenge as a `WWW-Authenticate` header value with
    /// `stale=true` when the peer can retry using a fresh nonce.
    pub fn www_authenticate_with_stale(&self, challenge: &DigestChallenge, stale: bool) -> String {
        self.authenticator
            .format_www_authenticate_with_stale(challenge, stale)
    }

    /// Validate an optional inbound `Authorization` value.
    ///
    /// Missing, malformed, unknown-user, or invalid digest values return
    /// [`AuthDecision::Rejected`] with a fresh challenge.
    pub fn authenticate_authorization(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<AuthDecision> {
        let Some(authorization) = authorization else {
            return Ok(self.rejected());
        };

        self.validate_authorization(authorization, method, request_uri, body)
    }

    /// Validate an optional inbound `Authorization` value using shared
    /// nonce/replay storage.
    ///
    /// This is the async compatibility path for deployments that need
    /// cluster-safe Digest replay protection but are not using
    /// [`SipAuthService`].
    pub async fn authenticate_authorization_with_replay_store(
        &self,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        replay_store: Arc<dyn DigestReplayStore>,
    ) -> Result<AuthDecision> {
        let Some(authorization) = authorization else {
            return self.rejected_with_replay_store(replay_store).await;
        };

        self.validate_authorization_with_replay_store(
            authorization,
            method,
            request_uri,
            body,
            replay_store,
        )
        .await
    }

    /// Validate a present inbound `Authorization` value.
    ///
    /// Malformed, unknown-user, wrong-password, realm mismatch, nonce mismatch,
    /// or request-URI mismatch return [`AuthDecision::Rejected`] with a fresh
    /// challenge so callers can answer 401/407 without raw string assembly.
    pub fn validate_authorization(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> Result<AuthDecision> {
        let response = match DigestAuthenticator::parse_authorization(authorization) {
            Ok(response) => response,
            Err(_) => return Ok(self.rejected()),
        };

        if response.uri != request_uri
            || response.realm != self.realm
            || response.algorithm != self.algorithm
            || digest_nonce_count(&response).is_none()
        {
            return Ok(self.rejected());
        }

        match self.nonce_status(&response.nonce) {
            NonceStatus::Active => {}
            NonceStatus::Expired => return Ok(self.rejected_stale()),
            NonceStatus::Unknown => return Ok(self.rejected()),
        }

        let secret = {
            let users = self
                .users
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match users.get(&response.username) {
                Some(verifiers) => verifiers.secret(response.algorithm),
                None => return Ok(self.rejected()),
            }
        };

        let valid = match self
            .authenticator
            .validate_response_with_secret_and_body(&response, method, &secret, body)
        {
            Ok(valid) => valid,
            Err(_) => return Ok(self.rejected()),
        };
        if !valid {
            return Ok(self.rejected());
        }

        if !self.accept_nonce_count(&response) {
            return Ok(self.rejected());
        }

        Ok(AuthDecision::Authorized {
            username: response.username,
            realm: response.realm,
        })
    }

    /// Validate a present inbound `Authorization` value using shared
    /// nonce/replay storage.
    pub async fn validate_authorization_with_replay_store(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
        replay_store: Arc<dyn DigestReplayStore>,
    ) -> Result<AuthDecision> {
        let response = match DigestAuthenticator::parse_authorization(authorization) {
            Ok(response) => response,
            Err(_) => return self.rejected_with_replay_store(replay_store).await,
        };

        if response.uri != request_uri
            || response.realm != self.realm
            || response.algorithm != self.algorithm
            || digest_nonce_count(&response).is_none()
        {
            return self.rejected_with_replay_store(replay_store).await;
        }

        match replay_store
            .nonce_status(&response.nonce, SystemTime::now())
            .await
            .map_err(|error| redacted_auth_failure(AuthFailureStage::ReplayNonceStatus, error))?
        {
            DigestNonceStatus::Active => {}
            DigestNonceStatus::Expired => {
                return self.rejected_stale_with_replay_store(replay_store).await
            }
            DigestNonceStatus::Unknown => {
                return self.rejected_with_replay_store(replay_store).await
            }
        }

        let secret = {
            let users = self
                .users
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            users
                .get(&response.username)
                .map(|verifiers| verifiers.secret(response.algorithm))
        };
        let Some(secret) = secret else {
            return self.rejected_with_replay_store(replay_store).await;
        };

        let valid = match self
            .authenticator
            .validate_response_with_secret_and_body(&response, method, &secret, body)
        {
            Ok(valid) => valid,
            Err(_) => return self.rejected_with_replay_store(replay_store).await,
        };
        if !valid {
            return self.rejected_with_replay_store(replay_store).await;
        }

        if !accept_nonce_count_with_replay_store(&response, replay_store.as_ref()).await? {
            return self.rejected_with_replay_store(replay_store).await;
        }

        Ok(AuthDecision::Authorized {
            username: response.username,
            realm: response.realm,
        })
    }

    fn nonce_status(&self, nonce: &str) -> NonceStatus {
        let now = Instant::now();
        let mut nonces = self
            .nonces
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match nonces.get(nonce).copied() {
            Some(expires_at) if expires_at > now => NonceStatus::Active,
            Some(_) => {
                nonces.remove(nonce);
                drop(nonces);
                self.nonce_counts
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .retain(|(_, recorded_nonce, _), _| recorded_nonce != nonce);
                NonceStatus::Expired
            }
            None => NonceStatus::Unknown,
        }
    }

    fn accept_nonce_count(&self, response: &DigestResponse) -> bool {
        accept_local_digest_nonce_count(self.nonce_counts.as_ref(), response)
    }

    fn rejected(&self) -> AuthDecision {
        let challenge = self.challenge();
        let www_authenticate = self.www_authenticate(&challenge);
        AuthDecision::Rejected {
            challenge,
            www_authenticate,
        }
    }

    fn rejected_stale(&self) -> AuthDecision {
        let challenge = self.challenge();
        let www_authenticate = self.www_authenticate_with_stale(&challenge, true);
        AuthDecision::Rejected {
            challenge,
            www_authenticate,
        }
    }

    async fn rejected_with_replay_store(
        &self,
        replay_store: Arc<dyn DigestReplayStore>,
    ) -> Result<AuthDecision> {
        let challenge = self.challenge_with_replay_store(replay_store).await?;
        let www_authenticate = self.www_authenticate(&challenge);
        Ok(AuthDecision::Rejected {
            challenge,
            www_authenticate,
        })
    }

    async fn rejected_stale_with_replay_store(
        &self,
        replay_store: Arc<dyn DigestReplayStore>,
    ) -> Result<AuthDecision> {
        let challenge = self.challenge_with_replay_store(replay_store).await?;
        let www_authenticate = self.www_authenticate_with_stale(&challenge, true);
        Ok(AuthDecision::Rejected {
            challenge,
            www_authenticate,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonceStatus {
    Active,
    Expired,
    Unknown,
}

impl std::fmt::Debug for SipDigestAuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let user_count = self
            .users
            .read()
            .map(|users| users.len())
            .unwrap_or_default();
        let nonce_count = self
            .nonces
            .read()
            .map(|nonces| nonces.len())
            .unwrap_or_default();
        f.debug_struct("SipDigestAuthService")
            .field("realm_present", &!self.realm.is_empty())
            .field("user_count", &user_count)
            .field("nonce_count", &nonce_count)
            .field("nonce_ttl", &self.nonce_ttl)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Mutex;

    const LOWER_ERROR_CANARY: &str = "lower-auth-secret\r\nX-Auth-Canary: exposed";

    fn assert_auth_stage(error: &SessionError, stage: AuthFailureStage) {
        assert_eq!(
            error.to_string(),
            format!("Authentication error: {}", stage.message())
        );
        assert!(!error.to_string().contains(LOWER_ERROR_CANARY));
        assert!(!format!("{error:?}").contains(LOWER_ERROR_CANARY));
        assert!(matches!(
            error,
            SessionError::AuthError(value) if value == stage.message()
        ));
    }

    struct StaticAkaClientResponse(String);

    impl AkaClientProvider for StaticAkaClientResponse {
        fn authorization(
            &self,
            _challenge_header: &str,
            _method: &str,
            _request_uri: &str,
            _nonce_count: u32,
        ) -> Result<String> {
            Ok(self.0.clone())
        }
    }

    struct StaticPasswordVerifier;

    struct FailingPasswordVerifier;

    #[async_trait::async_trait]
    impl PasswordVerifier for StaticPasswordVerifier {
        async fn verify_password(
            &self,
            username: &str,
            password: &str,
        ) -> std::result::Result<IdentityAssurance, CredentialAuthError> {
            if username == "alice" && password == "secret" {
                Ok(test_assurance())
            } else {
                Err(CredentialAuthError::Invalid)
            }
        }
    }

    #[async_trait::async_trait]
    impl PasswordVerifier for FailingPasswordVerifier {
        async fn verify_password(
            &self,
            _username: &str,
            _password: &str,
        ) -> std::result::Result<IdentityAssurance, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }
    }

    struct StaticDigestProvider;

    struct FailingDigestProvider;

    #[async_trait::async_trait]
    impl DigestSecretProvider for StaticDigestProvider {
        async fn lookup_digest_secret(
            &self,
            username: &str,
            _realm: &str,
            _algorithm: DigestAlgorithm,
        ) -> std::result::Result<Option<DigestSecret>, CredentialAuthError> {
            if username == "alice" {
                Ok(Some(DigestSecret::PlaintextPassword("secret".to_string())))
            } else {
                Ok(None)
            }
        }
    }

    #[async_trait::async_trait]
    impl DigestSecretProvider for FailingDigestProvider {
        async fn lookup_digest_secret(
            &self,
            _username: &str,
            _realm: &str,
            _algorithm: DigestAlgorithm,
        ) -> std::result::Result<Option<DigestSecret>, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingAuditSink {
        events: Arc<Mutex<Vec<AuthAuditEvent>>>,
        fail: bool,
    }

    impl RecordingAuditSink {
        fn into_arc(self) -> Arc<dyn AuthAuditSink> {
            Arc::new(self)
        }

        fn events(&self) -> Vec<AuthAuditEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl AuthAuditSink for RecordingAuditSink {
        async fn record_auth_event(
            &self,
            event: AuthAuditEvent,
        ) -> std::result::Result<(), CredentialAuthError> {
            if self.fail {
                return Err(CredentialAuthError::Unavailable(
                    LOWER_ERROR_CANARY.to_string(),
                ));
            }
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct TestRateLimiter {
        verdict: AuthRateLimitVerdict,
        checked: Arc<Mutex<Vec<AuthRateLimitKey>>>,
        results: Arc<Mutex<Vec<AuthAuditOutcome>>>,
        fail_check: bool,
        fail_record: bool,
    }

    impl TestRateLimiter {
        fn allow() -> Self {
            Self {
                verdict: AuthRateLimitVerdict::Allowed,
                checked: Arc::new(Mutex::new(Vec::new())),
                results: Arc::new(Mutex::new(Vec::new())),
                fail_check: false,
                fail_record: false,
            }
        }

        fn deny() -> Self {
            Self {
                verdict: AuthRateLimitVerdict::Denied {
                    retry_after: Some(Duration::from_secs(1)),
                },
                checked: Arc::new(Mutex::new(Vec::new())),
                results: Arc::new(Mutex::new(Vec::new())),
                fail_check: false,
                fail_record: false,
            }
        }

        fn fail_check() -> Self {
            Self {
                fail_check: true,
                ..Self::allow()
            }
        }

        fn fail_record() -> Self {
            Self {
                fail_record: true,
                ..Self::allow()
            }
        }

        fn into_arc(self) -> Arc<dyn AuthRateLimiter> {
            Arc::new(self)
        }

        fn results(&self) -> Vec<AuthAuditOutcome> {
            self.results.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl AuthRateLimiter for TestRateLimiter {
        async fn check_auth_attempt(
            &self,
            key: &AuthRateLimitKey,
        ) -> std::result::Result<AuthRateLimitVerdict, CredentialAuthError> {
            if self.fail_check {
                return Err(CredentialAuthError::Unavailable(
                    LOWER_ERROR_CANARY.to_string(),
                ));
            }
            self.checked.lock().unwrap().push(key.clone());
            Ok(self.verdict.clone())
        }

        async fn record_auth_result(
            &self,
            _key: &AuthRateLimitKey,
            outcome: &AuthAuditOutcome,
        ) -> std::result::Result<(), CredentialAuthError> {
            if self.fail_record {
                return Err(CredentialAuthError::Unavailable(
                    LOWER_ERROR_CANARY.to_string(),
                ));
            }
            self.results.lock().unwrap().push(outcome.clone());
            Ok(())
        }
    }

    struct UnavailableBearer;

    struct ScopedBearer(Vec<String>);

    #[async_trait::async_trait]
    impl BearerValidator for ScopedBearer {
        async fn validate(
            &self,
            _token: &str,
        ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
            Ok(IdentityAssurance::Anonymous)
        }

        async fn validate_principal(
            &self,
            _token: &str,
        ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
            Ok(AuthenticatedPrincipal {
                subject: "scoped-test".into(),
                tenant: Some("tenant-a".into()),
                scopes: self.0.clone(),
                issuer: Some("test".into()),
                expires_at: None,
                method: AuthenticationMethod::Bearer,
                assurance: IdentityAssurance::Anonymous,
            })
        }
    }

    #[async_trait::async_trait]
    impl BearerValidator for UnavailableBearer {
        async fn validate(
            &self,
            _token: &str,
        ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
            Err(BearerAuthError::Unavailable(LOWER_ERROR_CANARY.to_string()))
        }
    }

    struct FailingAkaVectorProvider;

    #[async_trait::async_trait]
    impl AkaVectorProvider for FailingAkaVectorProvider {
        async fn validate(
            &self,
            _authorization: &str,
            _method: &str,
            _request_uri: &str,
            _body: Option<&[u8]>,
        ) -> Result<Option<AuthIdentity>> {
            Err(SessionError::AuthError(LOWER_ERROR_CANARY.to_string()))
        }

        fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge {
            SipAuthChallenge {
                scheme: SipAuthScheme::Aka,
                value: "Digest algorithm=AKAv1-MD5".to_string(),
                source,
            }
        }
    }

    #[derive(Default)]
    struct MemoryDigestReplayStore {
        nonces: Mutex<HashMap<String, SystemTime>>,
        nonce_counts: Mutex<HashMap<(String, String, String), u32>>,
        force_expired: Mutex<bool>,
    }

    struct FailingDigestReplayStore;

    #[async_trait::async_trait]
    impl DigestReplayStore for FailingDigestReplayStore {
        async fn record_nonce(
            &self,
            _nonce: &str,
            _expires_at: SystemTime,
        ) -> std::result::Result<(), CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }

        async fn nonce_status(
            &self,
            _nonce: &str,
            _now: SystemTime,
        ) -> std::result::Result<DigestNonceStatus, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }

        async fn accept_nonce_count(
            &self,
            _username: &str,
            _nonce: &str,
            _cnonce: &str,
            _nonce_count: u32,
        ) -> std::result::Result<bool, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }

        async fn admit_nonce(
            &self,
            _proposed_nonce: &str,
            _expires_at: SystemTime,
        ) -> std::result::Result<String, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }

        async fn accept_client_nonce_count(
            &self,
            _username: &str,
            _nonce: &str,
            _cnonce: &str,
            _nonce_count: u32,
            _now: SystemTime,
        ) -> std::result::Result<bool, CredentialAuthError> {
            Err(CredentialAuthError::Unavailable(
                LOWER_ERROR_CANARY.to_string(),
            ))
        }
    }

    impl MemoryDigestReplayStore {
        fn set_force_expired(&self, expired: bool) {
            *self.force_expired.lock().unwrap() = expired;
        }
    }

    #[async_trait::async_trait]
    impl DigestReplayStore for MemoryDigestReplayStore {
        async fn record_nonce(
            &self,
            nonce: &str,
            expires_at: SystemTime,
        ) -> std::result::Result<(), CredentialAuthError> {
            self.nonces
                .lock()
                .unwrap()
                .insert(nonce.to_string(), expires_at);
            Ok(())
        }

        async fn nonce_status(
            &self,
            nonce: &str,
            now: SystemTime,
        ) -> std::result::Result<DigestNonceStatus, CredentialAuthError> {
            let nonces = self.nonces.lock().unwrap();
            let Some(expires_at) = nonces.get(nonce).copied() else {
                return Ok(DigestNonceStatus::Unknown);
            };
            if *self.force_expired.lock().unwrap() || expires_at <= now {
                Ok(DigestNonceStatus::Expired)
            } else {
                Ok(DigestNonceStatus::Active)
            }
        }

        async fn accept_nonce_count(
            &self,
            username: &str,
            nonce: &str,
            cnonce: &str,
            nonce_count: u32,
        ) -> std::result::Result<bool, CredentialAuthError> {
            self.accept_client_nonce_count(username, nonce, cnonce, nonce_count, SystemTime::now())
                .await
        }

        async fn admit_nonce(
            &self,
            proposed_nonce: &str,
            expires_at: SystemTime,
        ) -> std::result::Result<String, CredentialAuthError> {
            self.record_nonce(proposed_nonce, expires_at).await?;
            Ok(proposed_nonce.to_string())
        }

        async fn accept_client_nonce_count(
            &self,
            username: &str,
            nonce: &str,
            cnonce: &str,
            nonce_count: u32,
            now: SystemTime,
        ) -> std::result::Result<bool, CredentialAuthError> {
            if self.nonce_status(nonce, now).await? != DigestNonceStatus::Active {
                return Ok(false);
            }
            let key = (username.to_string(), nonce.to_string(), cnonce.to_string());
            let mut counts = self.nonce_counts.lock().unwrap();
            if counts.get(&key).is_some_and(|last| nonce_count <= *last) {
                return Ok(false);
            }
            counts.insert(key, nonce_count);
            Ok(true)
        }
    }

    fn test_assurance() -> IdentityAssurance {
        let identity = rvoip_core_traits::ids::IdentityId::from_string("user_alice");
        IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: vec!["sip.register".to_string()],
        }
    }

    #[test]
    fn fallback_bearer_subjects_are_stable_typed_and_credential_free() {
        const KEY_CANARY: &str = "jwk-private-material-canary";
        const FINGERPRINT_CANARY: &str = "AA:BB:CC:credential-canary";

        let assurances = [
            IdentityAssurance::Anonymous,
            IdentityAssurance::Identified {
                credential_kind: CredentialKind::Oidc,
            },
            IdentityAssurance::Pseudonymous {
                ephemeral_key: rvoip_core_traits::identity::Jwk(serde_json::json!({
                    "kty": "oct",
                    "k": KEY_CANARY,
                })),
            },
            IdentityAssurance::DtlsFingerprint {
                algorithm: "sha-256".into(),
                value: FINGERPRINT_CANARY.into(),
            },
        ];

        let subjects = assurances
            .iter()
            .map(fallback_bearer_assurance_subject)
            .collect::<Vec<_>>();
        for (assurance, subject) in assurances.iter().zip(&subjects) {
            assert_eq!(subject, &fallback_bearer_assurance_subject(assurance));
            assert!(subject.starts_with("urn:rvoip:sip:bearer-assurance:"));
            assert!(subject.contains(assurance.kind()));
            assert!(!subject.contains(KEY_CANARY));
            assert!(!subject.contains(FINGERPRINT_CANARY));
            assert!(!subject.contains("ephemeral_key_present"));
            assert!(!subject.contains("fingerprint_bytes"));
        }

        let unique = subjects.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), subjects.len());

        let changed_key = IdentityAssurance::Pseudonymous {
            ephemeral_key: rvoip_core_traits::identity::Jwk(serde_json::json!({
                "kty": "oct",
                "k": "different-private-material",
            })),
        };
        assert_ne!(
            subjects[2],
            fallback_bearer_assurance_subject(&changed_key),
            "different pseudonymous key bindings must not collapse to one subject"
        );
    }

    fn auth_token_strategy() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[A-Za-z0-9._~-]{1,16}").unwrap()
    }

    fn quoted_auth_value_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            proptest::string::string_regex("[A-Za-z0-9._~-]{1,12}").unwrap(),
            1..4,
        )
        .prop_map(|parts| parts.join(","))
    }

    fn authorization_for(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        uri: &str,
        body: Option<&[u8]>,
    ) -> String {
        authorization_for_nc(username, password, challenge, method, uri, body, 1)
    }

    fn authorization_for_nc(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        uri: &str,
        body: Option<&[u8]>,
        nc: u32,
    ) -> String {
        let computed = DigestAuth::compute_response_with_state(
            username, password, challenge, method, uri, nc, body,
        )
        .expect("digest computation");
        DigestAuth::format_authorization_with_state(username, challenge, uri, &computed)
    }

    #[test]
    fn sip_digest_auth_service_accepts_valid_authorization() {
        let service =
            SipDigestAuthService::new("example.test").with_algorithm(DigestAlgorithm::SHA512256);
        service.add_user("alice", "secret");
        let challenge = service.challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let decision = service
            .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
            .expect("validation succeeds");

        assert_eq!(
            decision,
            AuthDecision::Authorized {
                username: "alice".to_string(),
                realm: "example.test".to_string(),
            }
        );
    }

    #[test]
    fn sip_digest_auth_service_rejects_missing_or_invalid_authorization() {
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");
        let challenge = service.challenge();
        let wrong_password = authorization_for(
            "alice",
            "wrong",
            &challenge,
            "MESSAGE",
            "sip:bob@example.test",
            None,
        );

        assert!(matches!(
            service
                .authenticate_authorization(None, "MESSAGE", "sip:bob@example.test", None)
                .expect("missing auth decision"),
            AuthDecision::Rejected { .. }
        ));
        assert!(matches!(
            service
                .validate_authorization(&wrong_password, "MESSAGE", "sip:bob@example.test", None)
                .expect("invalid auth decision"),
            AuthDecision::Rejected { .. }
        ));
    }

    #[test]
    fn sip_digest_auth_service_rejects_realm_mismatch() {
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");
        let mut challenge = service.challenge();
        challenge.realm = "wrong.realm".to_string();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("realm mismatch decision"),
            AuthDecision::Rejected { .. }
        ));
    }

    #[test]
    fn sip_digest_auth_service_rejects_qop_omission_and_algorithm_downgrade() {
        let service =
            SipDigestAuthService::new("example.test").with_algorithm(DigestAlgorithm::SHA256);
        service.add_user("alice", "secret");
        let challenge = service.challenge();

        let mut no_qop = challenge.clone();
        no_qop.qop = None;
        let authorization = authorization_for(
            "alice",
            "secret",
            &no_qop,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("qop omission decision"),
            AuthDecision::Rejected { .. }
        ));

        let mut downgraded = challenge;
        downgraded.algorithm = DigestAlgorithm::MD5;
        let authorization = authorization_for(
            "alice",
            "secret",
            &downgraded,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("algorithm downgrade decision"),
            AuthDecision::Rejected { .. }
        ));
    }

    #[test]
    fn sip_digest_auth_service_bounds_local_nonce_state_without_active_eviction() {
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");
        let legitimate = service.challenge();
        for _ in 0..(MAX_LOCAL_DIGEST_NONCES + 32) {
            service.challenge();
        }
        let nonces = service
            .nonces
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(nonces.len(), MAX_LOCAL_DIGEST_NONCES);
        assert!(nonces.contains_key(&legitimate.nonce));
        drop(nonces);

        let authorization = authorization_for(
            "alice",
            "secret",
            &legitimate,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("legitimate proof after challenge churn"),
            AuthDecision::Authorized { .. }
        ));

        let shared = service.challenge();
        let first_client = authorization_for(
            "alice",
            "secret",
            &shared,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        let second_client = authorization_for(
            "alice",
            "secret",
            &shared,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        let first_cnonce = DigestAuthenticator::parse_authorization(&first_client)
            .unwrap()
            .cnonce;
        let second_cnonce = DigestAuthenticator::parse_authorization(&second_client)
            .unwrap()
            .cnonce;
        assert_ne!(first_cnonce, second_cnonce);
        assert!(matches!(
            service
                .validate_authorization(&first_client, "OPTIONS", "sip:bob@example.test", None)
                .expect("first shared-nonce client"),
            AuthDecision::Authorized { .. }
        ));
        assert!(matches!(
            service
                .validate_authorization(&second_client, "OPTIONS", "sip:bob@example.test", None)
                .expect("second shared-nonce client"),
            AuthDecision::Authorized { .. }
        ));
        assert!(matches!(
            service
                .validate_authorization(&second_client, "OPTIONS", "sip:bob@example.test", None)
                .expect("same-client replay decision"),
            AuthDecision::Rejected { .. }
        ));
    }

    #[test]
    fn local_digest_replay_quota_is_fair_between_usernames() {
        let counts = RwLock::new(HashMap::new());
        {
            let mut retained = counts.write().unwrap();
            for index in 0..MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME {
                retained.insert(
                    (
                        "noisy-user".to_string(),
                        format!("nonce-{index}"),
                        format!("client-{index}"),
                    ),
                    1,
                );
            }
        }

        let response = |username: &str| DigestResponse {
            username: username.to_string(),
            realm: "example.test".to_string(),
            nonce: "shared-active-nonce".to_string(),
            uri: "sip:bob@example.test".to_string(),
            response: "proof".to_string(),
            algorithm: DigestAlgorithm::MD5,
            cnonce: Some("new-client".to_string()),
            qop: Some("auth".to_string()),
            nc: Some("00000001".to_string()),
            opaque: None,
        };

        assert!(!accept_local_digest_nonce_count(
            &counts,
            &response("noisy-user")
        ));
        assert!(accept_local_digest_nonce_count(
            &counts,
            &response("unrelated-user")
        ));
    }

    #[test]
    fn local_digest_shared_nonce_quota_preserves_other_users() {
        let counts = RwLock::new(HashMap::new());
        {
            let mut retained = counts.write().unwrap();
            for index in 0..MAX_LOCAL_DIGEST_SEQUENCES_PER_USERNAME_NONCE {
                retained.insert(
                    (
                        "noisy-user".to_string(),
                        "shared-active-nonce".to_string(),
                        format!("client-{index}"),
                    ),
                    1,
                );
            }
        }

        let mut response = DigestResponse {
            username: "noisy-user".to_string(),
            realm: "example.test".to_string(),
            nonce: "shared-active-nonce".to_string(),
            uri: "sip:bob@example.test".to_string(),
            response: "proof".to_string(),
            algorithm: DigestAlgorithm::MD5,
            cnonce: Some("new-client".to_string()),
            qop: Some("auth".to_string()),
            nc: Some("00000001".to_string()),
            opaque: None,
        };
        assert!(!accept_local_digest_nonce_count(&counts, &response));
        response.username = "unrelated-user".to_string();
        assert!(accept_local_digest_nonce_count(&counts, &response));
    }

    #[test]
    fn sip_digest_auth_service_marks_expired_issued_nonce_stale() {
        let service =
            SipDigestAuthService::new("example.test").with_nonce_ttl(Duration::from_millis(1));
        service.add_user("alice", "secret");
        let challenge = service.challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        std::thread::sleep(Duration::from_millis(5));

        let decision = service
            .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
            .expect("expired nonce decision");

        match decision {
            AuthDecision::Rejected {
                www_authenticate, ..
            } => assert!(
                www_authenticate.contains("stale=true"),
                "expired nonce should produce stale challenge: {www_authenticate}"
            ),
            other => panic!("expected stale rejection, got {other:?}"),
        }
    }

    #[test]
    fn sip_digest_auth_service_rejects_unknown_and_same_client_replay() {
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");

        let unknown_challenge = DigestChallenge {
            realm: "example.test".to_string(),
            nonce: "not-issued".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };
        let unknown_nonce_auth = authorization_for(
            "alice",
            "secret",
            &unknown_challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        assert!(matches!(
            service
                .validate_authorization(
                    &unknown_nonce_auth,
                    "OPTIONS",
                    "sip:bob@example.test",
                    None
                )
                .expect("unknown nonce decision"),
            AuthDecision::Rejected { .. }
        ));

        let challenge = service.challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("first nonce-count decision"),
            AuthDecision::Authorized { .. }
        ));
        assert!(matches!(
            service
                .validate_authorization(&authorization, "OPTIONS", "sip:bob@example.test", None)
                .expect("replayed nonce-count decision"),
            AuthDecision::Rejected { .. }
        ));

        let independent_client_same_count = authorization_for_nc(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
            1,
        );
        assert!(matches!(
            service
                .validate_authorization(
                    &independent_client_same_count,
                    "OPTIONS",
                    "sip:bob@example.test",
                    None
                )
                .expect("same nonce-count with new cnonce decision"),
            AuthDecision::Authorized { .. }
        ));
        assert!(matches!(
            service
                .validate_authorization(
                    &independent_client_same_count,
                    "OPTIONS",
                    "sip:bob@example.test",
                    None
                )
                .expect("same-client nonce-count replay decision"),
            AuthDecision::Rejected { .. }
        ));

        let next_nonce_count = authorization_for_nc(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
            2,
        );
        assert!(matches!(
            service
                .validate_authorization(&next_nonce_count, "OPTIONS", "sip:bob@example.test", None)
                .expect("higher nonce-count decision"),
            AuthDecision::Authorized { .. }
        ));
    }

    #[test]
    fn sip_digest_auth_service_validates_auth_int_body() {
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");
        let mut challenge = service.challenge();
        challenge.qop = Some(vec!["auth-int".to_string()]);
        let body = b"hello";
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "MESSAGE",
            "sip:bob@example.test",
            Some(body),
        );

        assert!(matches!(
            service
                .validate_authorization(
                    &authorization,
                    "MESSAGE",
                    "sip:bob@example.test",
                    Some(body)
                )
                .expect("auth-int decision"),
            AuthDecision::Authorized { .. }
        ));
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_basic_when_cleartext_explicitly_allowed() {
        let mut service = SipAuthService::new()
            .with_basic_realm("legacy")
            .allow_basic_over_cleartext(true);
        service.add_basic_user("alice", "secret");
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("basic validation");

        assert_eq!(
            decision,
            SipAuthDecision::Authorized(AuthIdentity {
                scheme: SipAuthScheme::Basic,
                username: Some("alice".to_string()),
                subject: None,
                realm: Some("legacy".to_string()),
                scopes: Vec::new(),
                source: SipAuthSource::Origin,
            })
        );
    }

    #[tokio::test]
    async fn sip_auth_service_rejects_basic_over_cleartext_by_default() {
        let mut service = SipAuthService::new().with_basic_realm("legacy");
        service.add_basic_user("alice", "secret");
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("basic validation");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_basic_with_secure_transport_context() {
        let mut service = SipAuthService::new().with_basic_realm("legacy");
        service.add_basic_user("alice", "secret");
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization_with_transport_context(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                &SipTransportSecurityContext::from_transport_name("WSS"),
            )
            .await
            .expect("basic validation with transport context");

        assert!(matches!(
            decision,
            SipAuthDecision::Authorized(AuthIdentity {
                scheme: SipAuthScheme::Basic,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_basic_password_verifier() {
        let service = SipAuthService::new()
            .with_basic_verifier("legacy", Arc::new(StaticPasswordVerifier))
            .allow_basic_over_cleartext(true);
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("provider-backed basic validation");

        match decision {
            SipAuthDecision::Authorized(identity) => {
                assert_eq!(identity.scheme, SipAuthScheme::Basic);
                assert_eq!(identity.username.as_deref(), Some("alice"));
                assert_eq!(identity.realm.as_deref(), Some("legacy"));
                assert_eq!(identity.scopes, vec!["sip.register".to_string()]);
            }
            other => panic!("expected provider-backed Basic authorization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_bearer_validator_identity() {
        let service =
            SipAuthService::new().with_bearer_validator("api", rvoip_auth_core::bearer_stub());

        let decision = service
            .authenticate_authorization_with_transport_context(
                Some("Bearer token-123"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Proxy,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .await
            .expect("bearer validation");

        match decision {
            SipAuthDecision::Authorized(identity) => {
                assert_eq!(identity.scheme, SipAuthScheme::Bearer);
                assert_eq!(identity.realm.as_deref(), Some("api"));
                assert_eq!(identity.source, SipAuthSource::Proxy);
            }
            other => panic!("expected bearer authorization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sip_auth_service_rejects_bearer_over_cleartext_by_default() {
        let service =
            SipAuthService::new().with_bearer_validator("api", rvoip_auth_core::bearer_stub());

        let decision = service
            .authenticate_authorization(
                Some("Bearer token-123"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("bearer cleartext policy");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_bearer_cleartext_when_explicitly_allowed() {
        let service = SipAuthService::new()
            .with_bearer_validator("api", rvoip_auth_core::bearer_stub())
            .allow_bearer_over_cleartext(true);

        let decision = service
            .authenticate_authorization(
                Some("Bearer token-123"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("bearer cleartext opt-in");

        assert!(matches!(
            decision,
            SipAuthDecision::Authorized(AuthIdentity {
                scheme: SipAuthScheme::Bearer,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn sip_auth_service_enforces_required_bearer_scope() {
        for (scopes, authorized) in [
            (vec!["sip:connect".to_string()], true),
            (vec!["*".to_string()], true),
            (vec!["calls:read".to_string()], false),
            (Vec::new(), false),
        ] {
            let service = SipAuthService::new()
                .with_bearer_validator("bridgefu", Arc::new(ScopedBearer(scopes)))
                .with_bearer_scope("sip:connect")
                .with_required_bearer_scope("sip:connect")
                .allow_bearer_over_cleartext(true);
            let decision = service
                .authenticate_authorization(
                    Some("Bearer token"),
                    "INVITE",
                    "sip:attachment@example.test",
                    None,
                    SipAuthSource::Origin,
                    false,
                )
                .await
                .expect("required scope policy");
            assert_eq!(
                matches!(decision, SipAuthDecision::Authorized(_)),
                authorized
            );
        }
    }

    #[tokio::test]
    async fn sip_auth_policy_filters_challenges_and_rejects_disabled_scheme() {
        let mut service = SipAuthService::digest("example.test")
            .with_bearer_validator("api", rvoip_auth_core::bearer_stub())
            .with_basic_realm("legacy")
            .with_policy(SipAuthPolicy::new().allow_only([SipAuthScheme::Bearer]));
        service.add_digest_user("alice", "secret");
        service.add_basic_user("alice", "secret");

        let challenges = service.challenges(SipAuthSource::Origin);
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].scheme, SipAuthScheme::Bearer);

        let token = BASE64_STANDARD.encode("alice:secret");
        let decision = service
            .authenticate_authorization_with_transport_context(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .await
            .expect("policy rejection");
        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn sip_auth_policy_rejects_digest_below_minimum_algorithm() {
        let mut service = SipAuthService::digest("example.test").with_policy(
            SipAuthPolicy::new().with_minimum_digest_algorithm(DigestAlgorithm::SHA256),
        );
        service.add_digest_user("alice", "secret");
        let challenge = SipDigestAuthService::new("example.test").challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let decision = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("minimum digest policy");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn sip_auth_policy_can_require_digest_replay_store() {
        let mut service = SipAuthService::digest("example.test")
            .with_policy(SipAuthPolicy::new().require_digest_replay_store(true));
        service.add_digest_user("alice", "secret");
        let challenge = SipDigestAuthService::new("example.test").challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let decision = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("required replay-store policy");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn sip_auth_service_accepts_digest_secret_provider() {
        let service = SipAuthService::new()
            .with_digest_provider("example.test", Arc::new(StaticDigestProvider))
            .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
        let challenge = service
            .challenges(SipAuthSource::Origin)
            .into_iter()
            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
            .expect("digest challenge");
        let digest_challenge =
            DigestAuthenticator::parse_challenge(&challenge.value).expect("parse challenge");
        assert_eq!(digest_challenge.algorithm, DigestAlgorithm::SHA256);
        let authorization = authorization_for(
            "alice",
            "secret",
            &digest_challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let decision = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("provider-backed digest validation");

        assert_eq!(
            decision,
            SipAuthDecision::Authorized(AuthIdentity {
                scheme: SipAuthScheme::Digest,
                username: Some("alice".to_string()),
                subject: None,
                realm: Some("example.test".to_string()),
                scopes: Vec::new(),
                source: SipAuthSource::Origin,
            })
        );
    }

    #[tokio::test]
    async fn sip_auth_service_emits_redacted_audit_and_rate_results() {
        let sink = RecordingAuditSink::default();
        let limiter = TestRateLimiter::allow();
        let mut service = SipAuthService::new()
            .with_basic_realm("legacy")
            .allow_basic_over_cleartext(true)
            .with_audit_sink(sink.clone().into_arc())
            .with_rate_limiter(limiter.clone().into_arc());
        service.add_basic_user("alice", "secret");
        let context = SipAuthContext::new()
            .with_peer("192.0.2.10")
            .with_metadata("tenant", "acme");

        let valid = BASE64_STANDARD.encode("alice:secret");
        let wrong = BASE64_STANDARD.encode("alice:wrong");

        service
            .authenticate_authorization_with_context(
                Some(&format!("Basic {valid}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
                &context,
            )
            .await
            .expect("valid Basic auth");
        service
            .authenticate_authorization_with_context(
                Some(&format!("Basic {wrong}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
                &context,
            )
            .await
            .expect("invalid Basic auth");
        service
            .authenticate_authorization_with_context(
                None,
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
                &context,
            )
            .await
            .expect("missing auth");

        let events = sink.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].scheme, AuthAuditScheme::Basic);
        assert_eq!(events[0].outcome, AuthAuditOutcome::Success);
        assert_eq!(events[0].subject.as_deref(), Some("alice"));
        assert_eq!(events[0].peer.as_deref(), Some("192.0.2.10"));
        assert_eq!(
            events[0].metadata.get("tenant").map(String::as_str),
            Some("acme")
        );
        assert_eq!(
            events[1].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential)
        );
        assert_eq!(
            events[2].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::MissingCredential)
        );
        assert_eq!(
            limiter.results(),
            vec![
                AuthAuditOutcome::Success,
                AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
                AuthAuditOutcome::Failure(AuthFailureReason::MissingCredential)
            ]
        );
        for event in events {
            assert!(
                !event
                    .metadata
                    .values()
                    .any(|value| value.contains("secret")),
                "audit metadata must not contain credentials: {event:?}"
            );
        }
    }

    #[tokio::test]
    async fn sip_auth_service_audits_basic_cleartext_rejection() {
        let sink = RecordingAuditSink::default();
        let mut service = SipAuthService::new()
            .with_basic_realm("legacy")
            .with_audit_sink(sink.clone().into_arc());
        service.add_basic_user("alice", "secret");
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("Basic cleartext rejection");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
        assert_eq!(
            sink.events()[0].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::PolicyRejected)
        );
    }

    #[tokio::test]
    async fn sip_auth_service_rate_limiter_denies_before_validation() {
        let sink = RecordingAuditSink::default();
        let limiter = TestRateLimiter::deny();
        let mut service = SipAuthService::new()
            .with_basic_realm("legacy")
            .allow_basic_over_cleartext(true)
            .with_audit_sink(sink.clone().into_arc())
            .with_rate_limiter(limiter.clone().into_arc());
        service.add_basic_user("alice", "secret");
        let token = BASE64_STANDARD.encode("alice:secret");

        let decision = service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("rate-limit denial");

        assert!(matches!(decision, SipAuthDecision::Rejected { .. }));
        assert_eq!(
            sink.events()[0].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::PolicyRejected)
        );
        assert_eq!(
            limiter.results(),
            vec![AuthAuditOutcome::Failure(AuthFailureReason::PolicyRejected)]
        );
    }

    #[tokio::test]
    async fn sip_auth_service_rate_limiter_failure_fails_closed() {
        let sink = RecordingAuditSink::default();
        let limiter = TestRateLimiter::fail_check();
        let service = SipAuthService::new()
            .with_bearer_validator("api", rvoip_auth_core::bearer_stub())
            .with_audit_sink(sink.clone().into_arc())
            .with_rate_limiter(limiter.into_arc());

        let err = service
            .authenticate_authorization(
                Some("Bearer token"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("rate limiter failure should fail closed");

        assert_auth_stage(&err, AuthFailureStage::RateLimitCheck);
        assert_eq!(
            sink.events()[0].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable)
        );
    }

    #[tokio::test]
    async fn sip_auth_service_audits_bearer_provider_unavailable() {
        let sink = RecordingAuditSink::default();
        let service = SipAuthService::new()
            .with_bearer_validator("api", Arc::new(UnavailableBearer))
            .with_audit_sink(sink.clone().into_arc());

        let err = service
            .authenticate_authorization_with_transport_context(
                Some("Bearer token"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .await
            .expect_err("provider failure should return error");

        assert_auth_stage(&err, AuthFailureStage::BearerValidator);
        assert_eq!(
            sink.events()[0].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable)
        );
    }

    #[tokio::test]
    async fn audit_and_rate_result_failures_expose_only_fixed_stages() {
        let token = BASE64_STANDARD.encode("alice:secret");
        let mut rate_service = SipAuthService::new()
            .with_basic_realm("legacy")
            .allow_basic_over_cleartext(true)
            .with_rate_limiter(TestRateLimiter::fail_record().into_arc());
        rate_service.add_basic_user("alice", "secret");
        let rate_error = rate_service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("rate result failure");
        assert_auth_stage(&rate_error, AuthFailureStage::RateLimitRecord);

        let sink = RecordingAuditSink {
            fail: true,
            ..RecordingAuditSink::default()
        };
        let mut audit_service = SipAuthService::new()
            .with_basic_realm("legacy")
            .allow_basic_over_cleartext(true)
            .with_audit_sink(sink.into_arc())
            .with_audit_failure_policy(AuditFailurePolicy::FailClosed);
        audit_service.add_basic_user("alice", "secret");
        let audit_error = audit_service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("audit sink failure");
        assert_auth_stage(&audit_error, AuthFailureStage::AuditSink);
    }

    #[tokio::test]
    async fn verifier_digest_aka_and_replay_failures_expose_only_fixed_stages() {
        let token = BASE64_STANDARD.encode("alice:secret");
        let basic_service = SipAuthService::new()
            .with_basic_verifier("legacy", Arc::new(FailingPasswordVerifier))
            .allow_basic_over_cleartext(true);
        let basic_error = basic_service
            .authenticate_authorization(
                Some(&format!("Basic {token}")),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("password verifier failure");
        assert_auth_stage(&basic_error, AuthFailureStage::BasicVerifier);

        let digest_service = SipAuthService::new()
            .with_digest_provider("example.test", Arc::new(FailingDigestProvider));
        let digest_challenge = digest_service
            .challenges(SipAuthSource::Origin)
            .into_iter()
            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
            .and_then(|challenge| DigestAuthenticator::parse_challenge(&challenge.value).ok())
            .expect("Digest challenge");
        let digest_authorization = authorization_for(
            "alice",
            "secret",
            &digest_challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        let digest_error = digest_service
            .authenticate_authorization(
                Some(&digest_authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("Digest provider failure");
        assert_auth_stage(&digest_error, AuthFailureStage::DigestSecretProvider);

        let aka_service =
            SipAuthService::new().with_aka_provider(Arc::new(FailingAkaVectorProvider));
        let aka_error = aka_service
            .authenticate_aka_with_reason(
                "Digest algorithm=AKAv1-MD5",
                "INVITE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
            )
            .await
            .expect_err("AKA vector provider failure");
        assert_auth_stage(&aka_error, AuthFailureStage::AkaVectorProvider);

        let replay_store: Arc<dyn DigestReplayStore> = Arc::new(FailingDigestReplayStore);
        let compatibility_service = SipDigestAuthService::new("example.test");
        let record_error = compatibility_service
            .challenge_with_replay_store(replay_store.clone())
            .await
            .expect_err("replay nonce record failure");
        assert_auth_stage(&record_error, AuthFailureStage::ReplayNonceRecord);

        compatibility_service.add_user("alice", "secret");
        let challenge = compatibility_service.challenge();
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );
        let status_error = compatibility_service
            .validate_authorization_with_replay_store(
                &authorization,
                "OPTIONS",
                "sip:bob@example.test",
                None,
                replay_store.clone(),
            )
            .await
            .expect_err("replay nonce status failure");
        assert_auth_stage(&status_error, AuthFailureStage::ReplayNonceStatus);

        let response = DigestAuthenticator::parse_authorization(&authorization)
            .expect("parse Digest authorization");
        let count_error = accept_nonce_count_with_replay_store(&response, replay_store.as_ref())
            .await
            .expect_err("replay nonce-count failure");
        assert_auth_stage(&count_error, AuthFailureStage::ReplayNonceCount);
    }

    #[tokio::test]
    async fn sip_auth_service_uses_digest_replay_store() {
        let replay_store = Arc::new(MemoryDigestReplayStore::default());
        let sink = RecordingAuditSink::default();
        let service = SipAuthService::new()
            .with_digest_provider("example.test", Arc::new(StaticDigestProvider))
            .with_digest_replay_store(replay_store.clone())
            .with_audit_sink(sink.clone().into_arc());
        let challenge = service
            .challenges_async(SipAuthSource::Origin)
            .await
            .expect("async challenges")
            .into_iter()
            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
            .expect("Digest challenge");
        let digest_challenge =
            DigestAuthenticator::parse_challenge(&challenge.value).expect("parse challenge");
        assert_eq!(
            replay_store
                .nonce_status(&digest_challenge.nonce, SystemTime::now())
                .await
                .unwrap(),
            DigestNonceStatus::Active
        );
        let authorization = authorization_for(
            "alice",
            "secret",
            &digest_challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let first = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("first Digest auth");
        assert!(matches!(first, SipAuthDecision::Authorized(_)));

        let replay = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("replayed Digest auth");
        assert!(matches!(replay, SipAuthDecision::Rejected { .. }));
        assert_eq!(
            sink.events().last().unwrap().outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::ReplayRejected)
        );
    }

    #[tokio::test]
    async fn sip_auth_service_preserves_digest_stale_challenge() {
        let replay_store = Arc::new(MemoryDigestReplayStore::default());
        let sink = RecordingAuditSink::default();
        let service = SipAuthService::new()
            .with_digest_provider("example.test", Arc::new(StaticDigestProvider))
            .with_digest_replay_store(replay_store.clone())
            .with_audit_sink(sink.clone().into_arc());
        let challenge = service
            .challenges_async(SipAuthSource::Origin)
            .await
            .expect("async challenges")
            .into_iter()
            .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
            .expect("Digest challenge");
        let digest_challenge =
            DigestAuthenticator::parse_challenge(&challenge.value).expect("parse challenge");
        replay_store.set_force_expired(true);
        let authorization = authorization_for(
            "alice",
            "secret",
            &digest_challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let decision = service
            .authenticate_authorization(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect("stale Digest auth");

        match decision {
            SipAuthDecision::Rejected { challenges } => {
                let digest = challenges
                    .into_iter()
                    .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
                    .expect("Digest challenge");
                assert!(
                    digest.value.contains("stale=true"),
                    "stale challenge must be preserved: {}",
                    digest.value
                );
            }
            other => panic!("expected stale rejection, got {other:?}"),
        }
        assert_eq!(
            sink.events().last().unwrap().outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::StaleNonce)
        );
    }

    #[tokio::test]
    async fn sip_digest_auth_service_supports_replay_store_helper() {
        let replay_store = Arc::new(MemoryDigestReplayStore::default());
        let service = SipDigestAuthService::new("example.test");
        service.add_user("alice", "secret");
        let challenge = service
            .challenge_with_replay_store(replay_store.clone())
            .await
            .expect("challenge with replay store");
        let authorization = authorization_for(
            "alice",
            "secret",
            &challenge,
            "OPTIONS",
            "sip:bob@example.test",
            None,
        );

        let first = service
            .authenticate_authorization_with_replay_store(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                replay_store.clone(),
            )
            .await
            .expect("first Digest auth");
        assert!(matches!(first, AuthDecision::Authorized { .. }));

        let replay = service
            .authenticate_authorization_with_replay_store(
                Some(&authorization),
                "OPTIONS",
                "sip:bob@example.test",
                None,
                replay_store,
            )
            .await
            .expect("replayed Digest auth");
        assert!(matches!(replay, AuthDecision::Rejected { .. }));
    }

    #[test]
    fn sip_transport_security_context_classifies_secure_transports() {
        assert!(SipTransportSecurityContext::from_transport_name("TLS").is_secure());
        assert!(SipTransportSecurityContext::from_transport_name("wss").is_secure());
        assert!(
            SipTransportSecurityContext::from_request_uri_hint("sips:bob@example.test").is_secure()
        );
        assert!(
            SipTransportSecurityContext::from_request_uri_transport_hint(
                "sip:bob@example.test;transport=tls"
            )
            .is_secure()
        );
        assert!(
            SipTransportSecurityContext::from_request_uri_transport_hint(
                "sip:bob@example.test;transport=wss"
            )
            .is_secure()
        );
        assert!(!SipTransportSecurityContext::from_transport_name("UDP").is_secure());
        assert!(
            !SipTransportSecurityContext::from_request_uri_hint("sip:bob@example.test").is_secure()
        );
    }

    #[test]
    fn sip_client_auth_basic_uses_transport_context_policy() {
        let auth = SipClientAuth::basic("alice", "secret");
        let cleartext = auth.authorization_for_challenge_with_transport_context(
            r#"Basic realm="legacy""#,
            "OPTIONS",
            "sip:bob@example.test",
            1,
            None,
            &SipTransportSecurityContext::unknown(),
        );
        assert!(
            format!("{:?}", cleartext.expect_err("cleartext Basic must fail"))
                .contains("cleartext")
        );

        let secure = auth
            .authorization_for_challenge_with_transport_context(
                r#"Basic realm="legacy""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .expect("secure transport permits Basic");
        assert_eq!(secure.scheme, SipAuthScheme::Basic);
        assert!(secure.value.starts_with("Basic "));
    }

    #[test]
    fn sip_client_auth_bearer_uses_transport_context_policy() {
        let auth = SipClientAuth::bearer_token("token-123");
        let cleartext = auth.authorization_for_challenge_with_transport_context(
            r#"Bearer realm="api""#,
            "OPTIONS",
            "sip:bob@example.test",
            1,
            None,
            &SipTransportSecurityContext::unknown(),
        );
        assert!(
            format!("{:?}", cleartext.expect_err("cleartext Bearer must fail"))
                .contains("cleartext")
        );

        let secure = auth
            .authorization_for_challenge_with_transport_context(
                r#"Bearer realm="api""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .expect("secure transport permits Bearer");
        assert_eq!(secure.scheme, SipAuthScheme::Bearer);
        assert_eq!(secure.value, "Bearer token-123");

        let cleartext_allowed = SipClientAuth::bearer_token("token-123")
            .allow_bearer_over_cleartext(true)
            .authorization_for_challenge_with_transport_context(
                r#"Bearer realm="api""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::unknown(),
            )
            .expect("explicit cleartext opt-in permits Bearer");
        assert_eq!(cleartext_allowed.value, "Bearer token-123");
    }

    #[test]
    fn generated_client_auth_values_cannot_smuggle_header_lines() {
        let secure = SipTransportSecurityContext::from_transport_name("TLS");
        let bearer_secret = "bearer-canary\r\nX-Injected: bearer";
        let bearer_error = SipClientAuth::bearer_token(bearer_secret)
            .authorization_for_challenge_with_transport_context(
                r#"Bearer realm="api""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                None,
                &secure,
            )
            .expect_err("Bearer controls must fail before raw-header insertion");
        assert!(!bearer_error.to_string().contains(bearer_secret));

        let digest_secret = "alice\r\nX-Injected: digest";
        let digest_error = SipClientAuth::digest(digest_secret, "secret")
            .authorization_for_challenge_with_transport_context(
                r#"Digest realm="pbx", nonce="n1", algorithm=MD5, qop="auth""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                None,
                &secure,
            )
            .expect_err("Digest identity controls must fail before insertion");
        assert!(!digest_error.to_string().contains(digest_secret));

        let aka_secret = "Digest username=\"aka\"\r\nX-Injected: aka";
        let aka_error = SipClientAuth::aka(AkaClientConfig::new(Arc::new(
            StaticAkaClientResponse(aka_secret.to_string()),
        )))
        .authorization_for_challenge_with_transport_context(
            r#"Digest realm="ims", nonce="n1", algorithm=AKAv1-MD5"#,
            "INVITE",
            "sip:bob@example.test",
            1,
            None,
            &secure,
        )
        .expect_err("AKA provider controls must fail before insertion");
        assert!(!aka_error.to_string().contains(aka_secret));

        // Basic encodes caller material before validation. Even hostile input
        // remains confined to the base64 token and cannot create a new line.
        let basic = SipClientAuth::basic("alice\r\nX-Injected", "basic\0secret")
            .authorization_for_challenge_with_transport_context(
                r#"Basic realm="legacy""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                None,
                &secure,
            )
            .expect("base64-encoded Basic output is wire-safe");
        assert!(basic.value.starts_with("Basic "));
        assert!(!basic.value.chars().any(char::is_control));
        assert!(!basic.value.contains("X-Injected"));
    }

    #[test]
    fn generated_bearer_value_obeys_the_final_wire_size_boundary() {
        let secure = SipTransportSecurityContext::from_transport_name("TLS");
        let maximum_token = "a".repeat(
            rvoip_sip_core::validation::MAX_AUTHORIZATION_HEADER_VALUE_BYTES - "Bearer ".len(),
        );
        let maximum = SipClientAuth::bearer_token(maximum_token)
            .authorization_for_challenge_with_transport_context(
                r#"Bearer realm="api""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                None,
                &secure,
            )
            .expect("exact final header boundary remains valid");
        assert_eq!(
            maximum.value.len(),
            rvoip_sip_core::validation::MAX_AUTHORIZATION_HEADER_VALUE_BYTES
        );

        let oversized_token = "a".repeat(
            rvoip_sip_core::validation::MAX_AUTHORIZATION_HEADER_VALUE_BYTES - "Bearer ".len() + 1,
        );
        let error = SipClientAuth::bearer_token(oversized_token)
            .authorization_for_challenge_with_transport_context(
                r#"Bearer realm="api""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                None,
                &secure,
            )
            .expect_err("oversized final header must fail");
        assert!(error.to_string().contains("wire-safety validation"));
    }

    #[test]
    fn sip_client_auth_matches_schemes_case_insensitively() {
        let bearer = SipClientAuth::bearer_token("token-123")
            .authorization_for_challenge_with_transport_context(
                r#"bearer realm="api""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .expect("lowercase bearer challenge must match");
        assert_eq!(bearer.scheme, SipAuthScheme::Bearer);

        let basic = SipClientAuth::basic("alice", "secret")
            .allow_basic_over_cleartext(true)
            .authorization_for_challenge(
                r#"bAsIc realm="legacy""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect("mixed-case basic challenge must match");
        assert_eq!(basic.scheme, SipAuthScheme::Basic);
    }

    #[test]
    fn sip_client_auth_composite_selects_strongest_compatible_scheme() {
        let auth = SipClientAuth::any([
            SipClientAuth::digest("alice", "secret"),
            SipClientAuth::bearer_token("token-123"),
            SipClientAuth::basic("alice", "secret").allow_basic_over_cleartext(true),
        ]);
        let header = auth
            .authorization_for_challenge_with_transport_context(
                r#"Digest realm="pbx", nonce="n1", algorithm=MD5, Bearer realm="api", Basic realm="legacy""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .expect("composite auth");

        assert_eq!(header.scheme, SipAuthScheme::Bearer);
        assert_eq!(header.value, "Bearer token-123");
    }

    #[test]
    fn sip_client_auth_composite_handles_quoted_commas_in_challenge_lists() {
        let auth = SipClientAuth::any([
            SipClientAuth::digest("alice", "secret"),
            SipClientAuth::bearer_token("token-123"),
            SipClientAuth::basic("alice", "secret").allow_basic_over_cleartext(true),
        ]);
        let header = auth
            .authorization_for_challenge_with_transport_context(
                r#"Basic realm="legacy,with,commas", Digest realm="pbx", nonce="n1", algorithm=MD5, qop="auth,auth-int", Bearer realm="api", scope="sip.invite,sip.message""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
            .expect("composite auth with quoted commas");

        assert_eq!(header.scheme, SipAuthScheme::Bearer);
        assert_eq!(header.value, "Bearer token-123");
    }

    proptest! {
        #[test]
        fn auth_challenge_splitter_preserves_quoted_commas(
            basic_realm in quoted_auth_value_strategy(),
            digest_realm in quoted_auth_value_strategy(),
            nonce in auth_token_strategy(),
            bearer_scope in quoted_auth_value_strategy(),
        ) {
            let header = format!(
                r#"Basic realm="{basic_realm}", Digest realm="{digest_realm}", nonce="{nonce}", algorithm=SHA-256, qop="auth,auth-int", Bearer realm="api", scope="{bearer_scope}""#
            );

            let challenges = split_auth_challenges(&header);
            prop_assert_eq!(
                challenges.len(),
                3,
                "challenge splitter must not split quoted commas: {:?}",
                challenges
            );
            prop_assert!(challenges[0].starts_with("Basic "));
            prop_assert!(challenges[1].starts_with("Digest "));
            prop_assert!(challenges[2].starts_with("Bearer "));

            let digest = extract_digest_challenge(&header).expect("Digest challenge");
            let parsed = DigestAuthenticator::parse_challenge(&digest).expect("parse Digest challenge");
            prop_assert_eq!(parsed.realm, digest_realm);
            prop_assert_eq!(parsed.nonce, nonce);
            prop_assert_eq!(parsed.algorithm, DigestAlgorithm::SHA256);
        }
    }

    #[test]
    fn sip_client_auth_composite_rejects_basic_downgrade_when_digest_is_offered() {
        let auth = SipClientAuth::any([
            SipClientAuth::basic("alice", "secret").allow_basic_over_cleartext(true),
            SipClientAuth::digest("alice", "secret"),
        ]);
        let header = auth
            .authorization_for_challenge(
                r#"Basic realm="legacy", Digest realm="pbx", nonce="n1", algorithm=SHA-256"#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect("composite auth");

        assert_eq!(header.scheme, SipAuthScheme::Digest);
        assert!(header.value.starts_with("Digest "));
    }

    #[test]
    fn sip_client_auth_digest_selects_strongest_supported_challenge() {
        let auth = SipClientAuth::digest("alice", "secret");
        let header = auth
            .authorization_for_challenge(
                r#"Digest realm="pbx", nonce="weak", algorithm=MD5, Digest realm="pbx", nonce="strong", algorithm=SHA-512-256, qop="auth""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect("digest auth");
        let response =
            DigestAuthenticator::parse_authorization(&header.value).expect("parse authorization");

        assert_eq!(header.scheme, SipAuthScheme::Digest);
        assert_eq!(response.algorithm, DigestAlgorithm::SHA512256);
        assert_eq!(response.nonce, "strong");
    }

    #[test]
    fn sip_client_auth_digest_skips_malformed_challenge_when_valid_digest_exists() {
        let auth = SipClientAuth::digest("alice", "secret");
        let header = auth
            .authorization_for_challenge(
                r#"Digest realm="pbx", algorithm=SHA-512-256, Digest realm="pbx", nonce="valid", algorithm=SHA-256, qop="auth""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect("valid Digest alternative should be selected");
        let response =
            DigestAuthenticator::parse_authorization(&header.value).expect("parse authorization");

        assert_eq!(header.scheme, SipAuthScheme::Digest);
        assert_eq!(response.algorithm, DigestAlgorithm::SHA256);
        assert_eq!(response.nonce, "valid");
    }

    #[test]
    fn sip_client_auth_digest_rejects_malformed_only_challenge() {
        let err = SipClientAuth::digest("alice", "secret")
            .authorization_for_challenge(
                r#"Digest realm="pbx", algorithm=SHA-512-256"#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect_err("malformed-only Digest challenge must fail");

        assert_auth_stage(&err, AuthFailureStage::CorePrimitive);
    }
}
