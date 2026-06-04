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
//! helpers so issued nonces are recorded in shared storage. Digest-only users
//! can use
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

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rvoip_core_traits::identity::IdentityAssurance;

use crate::errors::{Result, SessionError};
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

/// SIP authentication scheme shared by UAC negotiation, UAS challenges, and
/// authenticated identity results.
///
/// SIP access authentication is carried in SIP headers:
/// `WWW-Authenticate`, `Proxy-Authenticate`, `Authorization`, and
/// `Proxy-Authorization`. SDP is only relevant to Digest when
/// `qop=auth-int` hashes the request body.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipAuthDecision {
    /// The inbound request carried acceptable credentials.
    Authorized(AuthIdentity),
    /// The inbound request should be challenged or rejected.
    Rejected {
        /// Challenge header values in priority order.
        challenges: Vec<SipAuthChallenge>,
    },
}

/// UAS challenge value generated by [`SipAuthService`].
///
/// Send [`value`](Self::value) in either `WWW-Authenticate` or
/// `Proxy-Authenticate`, depending on [`source`](Self::source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipAuthChallenge {
    /// Scheme to advertise.
    pub scheme: SipAuthScheme,
    /// Header value, for example `Digest realm="...", nonce="..."`.
    pub value: String,
    /// Whether this is a proxy challenge (`407`) rather than origin (`401`).
    pub source: SipAuthSource,
}

/// Non-secret context supplied to UAS-side authentication.
///
/// Context values are used for rate-limit keys and redacted audit events. Do
/// not put passwords, bearer tokens, API keys, HA1 values, full JWTs, or raw
/// Authorization headers in [`metadata`](Self::metadata).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SipAuthContext {
    /// Source peer, IP, connection id, or deployment-specific peer handle.
    pub peer: Option<String>,
    /// Additional non-secret metadata to attach to audit/rate-limit events.
    pub metadata: BTreeMap<String, String>,
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

/// UAC-side authentication configuration for challenged outbound requests.
///
/// Attach this to default configuration with [`Config::auth`](crate::Config::auth)
/// or to individual request builders with `.with_auth(...)`. The Digest-only
/// `Credentials` shorthand still works and converts into
/// [`SipClientAuth::Digest`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum SipClientAuth {
    /// Digest username/password credentials.
    Digest(Credentials),
    /// Bearer token. Sent after a Bearer challenge unless the request builder
    /// explicitly authors a preemptive Authorization header.
    BearerToken(String),
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
        match self {
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
                if !is_tls && !*allow_cleartext {
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
                let response =
                    config.respond(challenge_header, method, request_uri, nonce_count)?;
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
                is_tls,
            ),
        }
    }
}

impl From<Credentials> for SipClientAuth {
    fn from(value: Credentials) -> Self {
        Self::Digest(value)
    }
}

/// UAC auth header value selected by [`SipClientAuth`] for a challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Clone)]
pub struct SipAuthService {
    digest: Option<SipDigestAuthService>,
    digest_provider: Option<DigestProviderAuthStore>,
    bearer: Option<Arc<dyn BearerValidator>>,
    bearer_realm: Option<String>,
    bearer_scope: Option<String>,
    basic: Option<BasicAuthStore>,
    aka: Option<Arc<dyn AkaVectorProvider>>,
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
            digest: None,
            digest_provider: None,
            bearer: None,
            bearer_realm: None,
            bearer_scope: None,
            basic: None,
            aka: None,
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
    /// Rate-limiter provider errors fail closed.
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<dyn AuthRateLimiter>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Add a Bearer validator for UAS token validation.
    ///
    /// The validator comes from `rvoip-auth-core`; this facade maps successful
    /// validation into [`AuthIdentity`].
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
        self.authenticate_authorization_with_context(
            authorization,
            method,
            request_uri,
            body,
            source,
            is_tls,
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
        let attempt = auth_attempt_scheme(authorization);
        let rate_key = self.rate_limit_key(attempt, authorization, method, context);

        let verdict = match self.check_rate_limit(&rate_key).await {
            Ok(verdict) => verdict,
            Err(err) => {
                let outcome = AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable);
                self.audit_attempt(attempt, outcome, authorization, source, method, context)
                    .await?;
                return Err(SessionError::AuthError(format!(
                    "auth rate limiter unavailable: {err}"
                )));
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
                        self.authenticate_digest_with_reason(
                            trimmed,
                            method,
                            request_uri,
                            body,
                            source,
                        )
                        .await
                    }
                    AuthAttemptScheme::Bearer => {
                        self.authenticate_bearer_with_reason(trimmed, source).await
                    }
                    AuthAttemptScheme::Basic => {
                        if !is_tls && !self.allow_basic_over_cleartext {
                            Ok((
                                self.rejected_async(source).await?,
                                Some(AuthFailureReason::PolicyRejected),
                            ))
                        } else {
                            self.authenticate_basic_with_reason(trimmed, source, is_tls)
                                .await
                        }
                    }
                    AuthAttemptScheme::Aka => {
                        self.authenticate_aka_with_reason(
                            trimmed,
                            method,
                            request_uri,
                            body,
                            source,
                        )
                        .await
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
        if let Err(err) = rate_limiter.record_auth_result(key, outcome).await {
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
            return Err(SessionError::AuthError(format!(
                "auth rate limiter unavailable: {err}"
            )));
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
            Err(err) if self.audit_failure_policy == AuditFailurePolicy::FailOpen => {
                let _ = err;
                Ok(())
            }
            Err(err) => Err(SessionError::AuthError(format!(
                "auth audit sink unavailable: {err}"
            ))),
        }
    }

    fn challenges_with_digest_value(
        &self,
        source: SipAuthSource,
        digest_value: String,
    ) -> Vec<SipAuthChallenge> {
        let mut challenges = Vec::new();
        if let Some(aka) = &self.aka {
            challenges.push(aka.challenge(source));
        }
        if let Some(bearer) = &self.bearer_realm {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Bearer,
                value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                source,
            });
        }
        challenges.push(SipAuthChallenge {
            scheme: SipAuthScheme::Digest,
            value: digest_value,
            source,
        });
        if let Some(basic) = &self.basic {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Basic,
                value: format!("Basic realm=\"{}\"", basic.realm),
                source,
            });
        }
        challenges
    }

    /// Build challenge header values for the enabled schemes.
    ///
    /// Use [`SipAuthSource::Origin`] for `WWW-Authenticate` / `401` and
    /// [`SipAuthSource::Proxy`] for `Proxy-Authenticate` / `407`.
    pub fn challenges(&self, source: SipAuthSource) -> Vec<SipAuthChallenge> {
        let mut challenges = Vec::new();
        if let Some(aka) = &self.aka {
            challenges.push(aka.challenge(source));
        }
        if let Some(bearer) = &self.bearer_realm {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Bearer,
                value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                source,
            });
        }
        if let Some(digest) = &self.digest_provider {
            let challenge = digest.challenge();
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Digest,
                value: digest.www_authenticate(&challenge),
                source,
            });
        } else if let Some(digest) = &self.digest {
            let challenge = digest.challenge();
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Digest,
                value: digest.www_authenticate(&challenge),
                source,
            });
        }
        if let Some(basic) = &self.basic {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Basic,
                value: format!("Basic realm=\"{}\"", basic.realm),
                source,
            });
        }
        challenges
    }

    /// Build challenge values and record issued Digest nonces in configured
    /// shared replay storage.
    pub async fn challenges_async(&self, source: SipAuthSource) -> Result<Vec<SipAuthChallenge>> {
        let mut challenges = Vec::new();
        if let Some(aka) = &self.aka {
            challenges.push(aka.challenge(source));
        }
        if let Some(bearer) = &self.bearer_realm {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Bearer,
                value: bearer_challenge_value(bearer, self.bearer_scope.as_deref(), None, None),
                source,
            });
        }
        if let Some(digest) = &self.digest_provider {
            let challenge = digest.challenge_async().await?;
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Digest,
                value: digest.www_authenticate(&challenge),
                source,
            });
        } else if let Some(digest) = &self.digest {
            let challenge = if let Some(replay_store) = &self.digest_replay_store {
                digest
                    .challenge_with_replay_store(replay_store.clone())
                    .await?
            } else {
                digest.challenge()
            };
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Digest,
                value: digest.www_authenticate(&challenge),
                source,
            });
        }
        if let Some(basic) = &self.basic {
            challenges.push(SipAuthChallenge {
                scheme: SipAuthScheme::Basic,
                value: format!("Basic realm=\"{}\"", basic.realm),
                source,
            });
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
    ) -> Result<(SipAuthDecision, Option<AuthFailureReason>)> {
        let Some(validator) = &self.bearer else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
            ));
        };
        let token = authorization
            .split_once(char::is_whitespace)
            .map(|(_, value)| value.trim())
            .unwrap_or_default();
        match validator.validate(token).await {
            Ok(assurance) => Ok((
                SipAuthDecision::Authorized(identity_from_bearer_assurance(
                    assurance,
                    self.bearer_realm.clone(),
                    source,
                )),
                None,
            )),
            Err(BearerAuthError::Empty) | Err(BearerAuthError::Invalid(_)) => Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::InvalidCredential),
            )),
            Err(BearerAuthError::Unavailable(err)) => Err(SessionError::AuthError(format!(
                "Bearer validator unavailable: {err}"
            ))),
        }
    }

    async fn authenticate_basic_with_reason(
        &self,
        authorization: &str,
        source: SipAuthSource,
        is_tls: bool,
    ) -> Result<(SipAuthDecision, Option<AuthFailureReason>)> {
        let Some(basic) = &self.basic else {
            return Ok((
                self.rejected_async(source).await?,
                Some(AuthFailureReason::UnsupportedScheme),
            ));
        };
        if !is_tls && !self.allow_basic_over_cleartext {
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
                Err(err) => Err(SessionError::AuthError(err.to_string())),
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
            .await?
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
}

impl std::fmt::Debug for SipAuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SipAuthService")
            .field("digest", &self.digest.is_some())
            .field("digest_provider", &self.digest_provider.is_some())
            .field("bearer", &self.bearer.is_some())
            .field("bearer_realm", &self.bearer_realm)
            .field("bearer_scope", &self.bearer_scope)
            .field("basic", &self.basic.as_ref().map(|b| &b.realm))
            .field("aka", &self.aka.is_some())
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
    realm: String,
    provider: Arc<dyn DigestSecretProvider>,
    nonces: Arc<RwLock<HashMap<String, Instant>>>,
    nonce_counts: Arc<RwLock<HashMap<(String, String), u32>>>,
    nonce_ttl: Duration,
    replay_store: Option<Arc<dyn DigestReplayStore>>,
}

impl DigestProviderAuthStore {
    fn new(realm: impl Into<String>, provider: Arc<dyn DigestSecretProvider>) -> Self {
        let realm = realm.into();
        Self {
            authenticator: DigestAuthenticator::new(realm.clone()),
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
        self
    }

    fn with_replay_store(mut self, replay_store: Arc<dyn DigestReplayStore>) -> Self {
        self.replay_store = Some(replay_store);
        self
    }

    fn challenge(&self) -> DigestChallenge {
        let challenge = self.authenticator.generate_challenge();
        self.record_nonce_local(&challenge.nonce);
        challenge
    }

    async fn challenge_async(&self) -> Result<DigestChallenge> {
        let challenge = self.authenticator.generate_challenge();
        if let Some(replay_store) = &self.replay_store {
            replay_store
                .record_nonce(&challenge.nonce, system_time_after(self.nonce_ttl))
                .await
                .map_err(|err| SessionError::AuthError(err.to_string()))?;
        } else {
            self.record_nonce_local(&challenge.nonce);
        }
        Ok(challenge)
    }

    fn record_nonce_local(&self, nonce: &str) {
        let expires_at = Instant::now()
            .checked_add(self.nonce_ttl)
            .unwrap_or_else(Instant::now);
        let mut nonces = self
            .nonces
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        nonces.insert(nonce.to_string(), expires_at);
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

        if response.uri != request_uri || response.realm != self.realm {
            return self
                .rejected_with_reason(AuthFailureReason::InvalidCredential)
                .await;
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
            Err(err) => return Err(SessionError::AuthError(err.to_string())),
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
                NonceStatus::Expired
            }
            None => NonceStatus::Unknown,
        }
    }

    fn accept_nonce_count(&self, response: &DigestResponse) -> bool {
        let Some(qop) = response.qop.as_deref() else {
            return true;
        };
        if qop != "auth" && qop != "auth-int" {
            return false;
        }
        let Some(nc) = response
            .nc
            .as_deref()
            .and_then(|value| u32::from_str_radix(value, 16).ok())
        else {
            return false;
        };
        let Some(cnonce) = response.cnonce.clone() else {
            return false;
        };
        if cnonce.is_empty() {
            return false;
        }
        let key = (response.username.clone(), response.nonce.clone());
        let mut nonce_counts = self
            .nonce_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if nonce_counts.get(&key).is_some_and(|last| nc <= *last) {
            return false;
        }
        nonce_counts.insert(key, nc);
        true
    }

    async fn nonce_status_async(&self, nonce: &str) -> Result<NonceStatus> {
        let Some(replay_store) = &self.replay_store else {
            return Ok(self.nonce_status(nonce));
        };
        match replay_store
            .nonce_status(nonce, SystemTime::now())
            .await
            .map_err(|err| SessionError::AuthError(err.to_string()))?
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
        let Some(qop) = response.qop.as_deref() else {
            return Ok(None);
        };
        if qop != "auth" && qop != "auth-int" {
            return Ok(Some(AuthFailureReason::UnsupportedScheme));
        }
        let Some(nc) = response
            .nc
            .as_deref()
            .and_then(|value| u32::from_str_radix(value, 16).ok())
        else {
            return Ok(Some(AuthFailureReason::MalformedCredential));
        };
        let Some(cnonce) = response.cnonce.as_ref() else {
            return Ok(Some(AuthFailureReason::MalformedCredential));
        };
        if cnonce.is_empty() {
            return Ok(Some(AuthFailureReason::MalformedCredential));
        }
        let accepted = if let Some(replay_store) = &self.replay_store {
            replay_store
                .accept_nonce_count(&response.username, &response.nonce, nc)
                .await
                .map_err(|err| SessionError::AuthError(err.to_string()))?
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
            subject: Some(format!("{other:?}")),
            realm,
            scopes: Vec::new(),
            source,
        },
    }
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
    let Some(qop) = response.qop.as_deref() else {
        return Ok(true);
    };
    if qop != "auth" && qop != "auth-int" {
        return Ok(false);
    }
    let Some(nc) = response
        .nc
        .as_deref()
        .and_then(|value| u32::from_str_radix(value, 16).ok())
    else {
        return Ok(false);
    };
    let Some(cnonce) = response.cnonce.as_ref() else {
        return Ok(false);
    };
    if cnonce.is_empty() {
        return Ok(false);
    }
    replay_store
        .accept_nonce_count(&response.username, &response.nonce, nc)
        .await
        .map_err(|err| SessionError::AuthError(err.to_string()))
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
    value
        .split(',')
        .any(|part| part.trim_start().starts_with(scheme))
        || value.trim_start().starts_with(scheme)
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
    is_tls: bool,
) -> Result<ClientAuthHeader> {
    let priorities: &[fn(&SipClientAuth) -> bool] = &[
        |auth| matches!(auth, SipClientAuth::Aka(_)),
        |auth| matches!(auth, SipClientAuth::BearerToken(_)),
        |auth| matches!(auth, SipClientAuth::Digest(_)),
        |auth| matches!(auth, SipClientAuth::Basic { .. }),
    ];

    for matches_priority in priorities {
        for auth in auths.iter().filter(|auth| matches_priority(auth)) {
            if let Ok(header) = auth.authorization_for_challenge(
                challenge_header,
                method,
                request_uri,
                nonce_count,
                body,
                is_tls,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    realm: String,
    users: Arc<RwLock<HashMap<String, String>>>,
    nonces: Arc<RwLock<HashMap<String, Instant>>>,
    nonce_counts: Arc<RwLock<HashMap<(String, String), u32>>>,
    nonce_ttl: Duration,
}

impl SipDigestAuthService {
    /// Create a digest service for the given realm.
    pub fn new(realm: impl Into<String>) -> Self {
        let realm = realm.into();
        Self {
            authenticator: DigestAuthenticator::new(realm.clone()),
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
        self
    }

    /// Set how long generated nonces remain valid.
    pub fn with_nonce_ttl(mut self, ttl: Duration) -> Self {
        self.nonce_ttl = ttl;
        self
    }

    /// Add or replace a digest user.
    pub fn add_user(&self, username: impl Into<String>, password: impl Into<String>) {
        let mut users = self
            .users
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        users.insert(username.into(), password.into());
    }

    /// Generate a fresh digest challenge.
    pub fn challenge(&self) -> DigestChallenge {
        let challenge = self.authenticator.generate_challenge();
        let expires_at = Instant::now() + self.nonce_ttl;
        let mut nonces = self
            .nonces
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        nonces.insert(challenge.nonce.clone(), expires_at);
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
        replay_store
            .record_nonce(&challenge.nonce, system_time_after(self.nonce_ttl))
            .await
            .map_err(|err| SessionError::AuthError(err.to_string()))?;
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

        if response.uri != request_uri {
            return Ok(self.rejected());
        }
        if response.realm != self.realm {
            return Ok(self.rejected());
        }

        let password = {
            let users = self
                .users
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match users.get(&response.username) {
                Some(password) => password.clone(),
                None => return Ok(self.rejected()),
            }
        };

        let valid = match self
            .authenticator
            .validate_response_with_body(&response, method, &password, body)
        {
            Ok(valid) => valid,
            Err(_) => return Ok(self.rejected()),
        };
        if !valid {
            return Ok(self.rejected());
        }

        match self.nonce_status(&response.nonce) {
            NonceStatus::Active => {}
            NonceStatus::Expired => return Ok(self.rejected_stale()),
            NonceStatus::Unknown => return Ok(self.rejected()),
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

        if response.uri != request_uri || response.realm != self.realm {
            return self.rejected_with_replay_store(replay_store).await;
        }

        let password = {
            let users = self
                .users
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            users.get(&response.username).cloned()
        };
        let Some(password) = password else {
            return self.rejected_with_replay_store(replay_store).await;
        };

        let valid = match self
            .authenticator
            .validate_response_with_body(&response, method, &password, body)
        {
            Ok(valid) => valid,
            Err(_) => return self.rejected_with_replay_store(replay_store).await,
        };
        if !valid {
            return self.rejected_with_replay_store(replay_store).await;
        }

        match replay_store
            .nonce_status(&response.nonce, SystemTime::now())
            .await
            .map_err(|err| SessionError::AuthError(err.to_string()))?
        {
            DigestNonceStatus::Active => {}
            DigestNonceStatus::Expired => {
                return self.rejected_stale_with_replay_store(replay_store).await
            }
            DigestNonceStatus::Unknown => {
                return self.rejected_with_replay_store(replay_store).await
            }
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
                NonceStatus::Expired
            }
            None => NonceStatus::Unknown,
        }
    }

    fn accept_nonce_count(&self, response: &DigestResponse) -> bool {
        let Some(qop) = response.qop.as_deref() else {
            return true;
        };
        if qop != "auth" && qop != "auth-int" {
            return false;
        }
        let Some(nc) = response
            .nc
            .as_deref()
            .and_then(|value| u32::from_str_radix(value, 16).ok())
        else {
            return false;
        };
        let Some(cnonce) = response.cnonce.clone() else {
            return false;
        };
        if cnonce.is_empty() {
            return false;
        }
        let key = (response.username.clone(), response.nonce.clone());
        let mut nonce_counts = self
            .nonce_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if nonce_counts.get(&key).is_some_and(|last| nc <= *last) {
            return false;
        }
        nonce_counts.insert(key, nc);
        true
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
            .field("authenticator", &self.authenticator)
            .field("user_count", &user_count)
            .field("nonce_count", &nonce_count)
            .field("nonce_ttl", &self.nonce_ttl)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct StaticPasswordVerifier;

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

    struct StaticDigestProvider;

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
                return Err(CredentialAuthError::Unavailable("audit down".to_string()));
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
                    "rate limiter down".to_string(),
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
                    "rate limiter record down".to_string(),
                ));
            }
            self.results.lock().unwrap().push(outcome.clone());
            Ok(())
        }
    }

    struct UnavailableBearer;

    #[async_trait::async_trait]
    impl BearerValidator for UnavailableBearer {
        async fn validate(
            &self,
            _token: &str,
        ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
            Err(BearerAuthError::Unavailable("idp down".to_string()))
        }
    }

    #[derive(Default)]
    struct MemoryDigestReplayStore {
        nonces: Mutex<HashMap<String, SystemTime>>,
        nonce_counts: Mutex<HashMap<(String, String), u32>>,
        force_expired: Mutex<bool>,
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
            nonce_count: u32,
        ) -> std::result::Result<bool, CredentialAuthError> {
            let key = (username.to_string(), nonce.to_string());
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
    fn sip_digest_auth_service_rejects_unknown_nonce_and_replay() {
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

        let next_nonce_same_count = authorization_for_nc(
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
                    &next_nonce_same_count,
                    "OPTIONS",
                    "sip:bob@example.test",
                    None
                )
                .expect("same nonce-count with new cnonce decision"),
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
            .authenticate_authorization(
                Some("Bearer token-123"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Proxy,
                false,
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

        assert!(matches!(err, SessionError::AuthError(_)));
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
            .authenticate_authorization(
                Some("Bearer token"),
                "MESSAGE",
                "sip:bob@example.test",
                None,
                SipAuthSource::Origin,
                false,
            )
            .await
            .expect_err("provider failure should return error");

        assert!(matches!(err, SessionError::AuthError(_)));
        assert_eq!(
            sink.events()[0].outcome,
            AuthAuditOutcome::Failure(AuthFailureReason::ProviderUnavailable)
        );
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
    fn sip_client_auth_composite_selects_strongest_compatible_scheme() {
        let auth = SipClientAuth::any([
            SipClientAuth::digest("alice", "secret"),
            SipClientAuth::bearer_token("token-123"),
            SipClientAuth::basic("alice", "secret").allow_basic_over_cleartext(true),
        ]);
        let header = auth
            .authorization_for_challenge(
                r#"Digest realm="pbx", nonce="n1", algorithm=MD5, Bearer realm="api", Basic realm="legacy""#,
                "OPTIONS",
                "sip:bob@example.test",
                1,
                None,
                false,
            )
            .expect("composite auth");

        assert_eq!(header.scheme, SipAuthScheme::Bearer);
        assert_eq!(header.value, "Bearer token-123");
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
}
