//! Auth provider contracts for RVoIP services.
//!
//! Protocol crates use these traits to authenticate credentials without
//! depending on a specific user database or identity provider. `users-core`
//! implements these traits behind its `auth-core` feature, while applications
//! can implement them for external services such as LDAP, OIDC, IMS, or a
//! custom database.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use rvoip_core_traits::identity::IdentityAssurance;
use serde::{Deserialize, Serialize};

use crate::sip_digest::DigestAlgorithm;

/// Error returned by provider-backed credential checks.
pub enum CredentialAuthError {
    /// Credentials were present but did not authenticate.
    Invalid,

    /// The backing provider could not answer the request.
    Unavailable(String),

    /// A configured security policy rejected the credential or request.
    PolicyRejected(String),
}

impl CredentialAuthError {
    fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Unavailable(_) => "provider-unavailable",
            Self::PolicyRejected(_) => "policy-rejected",
        }
    }
}

impl fmt::Display for CredentialAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "credential authentication failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for CredentialAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialAuthError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for CredentialAuthError {}

/// Password verifier for Basic-style username/password authentication.
///
/// This trait intentionally verifies credentials without issuing access or
/// refresh tokens. Token issuance remains a user-service concern.
#[async_trait]
pub trait PasswordVerifier: Send + Sync {
    /// Verify a username/password pair and return the authenticated identity.
    async fn verify_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<IdentityAssurance, CredentialAuthError>;
}

/// Secret material usable for SIP Digest validation.
#[derive(Clone, Eq, PartialEq)]
pub enum DigestSecret {
    /// Plaintext SIP Digest password.
    PlaintextPassword(String),

    /// Precomputed HA1 value for `username:realm:password`.
    ///
    /// For `-sess` algorithms this is the base HA1 before nonce/cnonce folding.
    Ha1(String),
}

impl fmt::Debug for DigestSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlaintextPassword(value) => formatter
                .debug_struct("PlaintextPassword")
                .field("secret_bytes", &value.len())
                .finish(),
            Self::Ha1(value) => formatter
                .debug_struct("Ha1")
                .field("secret_bytes", &value.len())
                .finish(),
        }
    }
}

/// Provider for SIP Digest credential material.
///
/// Implementations should prefer returning [`DigestSecret::Ha1`] so the
/// backing store does not retain plaintext SIP secrets. This is separate from
/// login password storage; Argon2 login hashes are not valid SIP Digest
/// secrets.
#[async_trait]
pub trait DigestSecretProvider: Send + Sync {
    /// Look up SIP Digest credential material for a username and realm.
    async fn lookup_digest_secret(
        &self,
        username: &str,
        realm: &str,
        algorithm: DigestAlgorithm,
    ) -> Result<Option<DigestSecret>, CredentialAuthError>;
}

/// API key verifier for services that accept first-party API keys directly.
#[async_trait]
pub trait ApiKeyVerifier: Send + Sync {
    /// Verify an API key and return the authenticated identity.
    async fn verify_api_key(&self, api_key: &str)
        -> Result<IdentityAssurance, CredentialAuthError>;
}

/// Revocation state for an access token identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRevocationStatus {
    /// Token identifier is not revoked.
    Active,
    /// Token identifier has been revoked and must be rejected.
    Revoked,
}

/// Redacted context supplied to a token revocation checker.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenRevocationContext {
    /// Token identifier, usually the JWT `jti` claim.
    pub token_id: String,
    /// Token subject, usually the JWT `sub` claim.
    pub subject: Option<String>,
    /// Token issuer, usually the JWT `iss` claim.
    pub issuer: Option<String>,
    /// Token issued-at time when present.
    pub issued_at: Option<SystemTime>,
    /// Token expiry time when present.
    pub expires_at: Option<SystemTime>,
}

impl fmt::Debug for TokenRevocationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenRevocationContext")
            .field("token_id_present", &!self.token_id.is_empty())
            .field("token_id_bytes", &self.token_id.len())
            .field("subject_present", &self.subject.is_some())
            .field("issuer_present", &self.issuer.is_some())
            .field("issued_at_present", &self.issued_at.is_some())
            .field("expires_at_present", &self.expires_at.is_some())
            .finish()
    }
}

impl TokenRevocationContext {
    /// Create a revocation context for a token identifier.
    pub fn new(token_id: impl Into<String>) -> Self {
        Self {
            token_id: token_id.into(),
            subject: None,
            issuer: None,
            issued_at: None,
            expires_at: None,
        }
    }

    /// Attach a token subject.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Attach a token issuer.
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    /// Attach token issued-at and expiry times.
    pub fn with_times(
        mut self,
        issued_at: Option<SystemTime>,
        expires_at: Option<SystemTime>,
    ) -> Self {
        self.issued_at = issued_at;
        self.expires_at = expires_at;
        self
    }
}

/// Checks whether an access token identifier has been revoked.
///
/// JWT validators call this with the token's `jti` claim when a revocation
/// checker is configured. Opaque-token validators can use the same contract
/// with provider-specific token identifiers.
#[async_trait]
pub trait TokenRevocationChecker: Send + Sync {
    /// Return revocation state for a token.
    async fn check_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<TokenRevocationStatus, CredentialAuthError>;
}

/// SIP Digest nonce state from a replay store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestNonceStatus {
    /// The nonce is known and not expired.
    Active,
    /// The nonce was issued by this service but has expired.
    Expired,
    /// The nonce is unknown to this service.
    Unknown,
}

/// Shared replay store for clustered SIP Digest UAS deployments.
///
/// The original methods remain for source compatibility with pre-0.3 stores.
/// Secure clustered SIP listeners use [`DigestReplayStore::admit_nonce`] and
/// [`DigestReplayStore::accept_client_nonce_count`], whose defaults fail closed
/// until a store explicitly implements bounded, client-aware replay state.
#[async_trait]
pub trait DigestReplayStore: Send + Sync {
    /// Record an issued nonce with its expiry time (legacy compatibility).
    ///
    /// This method cannot return a reused nonce when storage is saturated.
    /// New challenge issuers must call [`Self::admit_nonce`] instead.
    async fn record_nonce(
        &self,
        nonce: &str,
        expires_at: SystemTime,
    ) -> Result<(), CredentialAuthError>;

    /// Return current nonce state.
    async fn nonce_status(
        &self,
        nonce: &str,
        now: SystemTime,
    ) -> Result<DigestNonceStatus, CredentialAuthError>;

    /// Atomically accept a nonce-count for the legacy `(username, nonce)` key.
    ///
    /// New validators must call [`Self::accept_client_nonce_count`] so clients
    /// sharing an admitted nonce retain independent monotonic sequences.
    async fn accept_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        nonce_count: u32,
    ) -> Result<bool, CredentialAuthError>;

    /// Atomically admit a proposed nonce or return an already-active nonce.
    ///
    /// Implementations must bound retained nonce state for their tenant/store
    /// namespace and make concurrent admission atomic. Returning an active
    /// nonce under pressure prevents unauthenticated challenge churn from
    /// allocating unbounded shared state.
    async fn admit_nonce(
        &self,
        _proposed_nonce: &str,
        _expires_at: SystemTime,
    ) -> Result<String, CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "bounded Digest nonce admission is not implemented".to_string(),
        ))
    }

    /// Atomically accept a count only when the issued nonce is still active
    /// and the value is greater than the last accepted value for
    /// `(username, nonce, cnonce)`.
    ///
    /// Implementations must retain replay state for at least the nonce's
    /// remaining validity and stale-retention interval, and must apply fair
    /// tenant, principal, and nonce cardinality bounds. The default fails
    /// closed so legacy stores remain source compatible without silently
    /// weakening clustered replay protection.
    async fn accept_client_nonce_count(
        &self,
        _username: &str,
        _nonce: &str,
        _cnonce: &str,
        _nonce_count: u32,
        _now: SystemTime,
    ) -> Result<bool, CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "client-aware Digest replay protection is not implemented".to_string(),
        ))
    }
}

/// Auth scheme associated with an audit event.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthAuditScheme {
    /// SIP Digest authentication.
    Digest,
    /// Bearer token authentication.
    Bearer,
    /// Basic username/password authentication.
    Basic,
    /// IMS AKA authentication.
    Aka,
    /// API key authentication.
    ApiKey,
    /// Direct password verification.
    Password,
    /// Token issuance, refresh, or revocation.
    Token,
    /// External or future scheme.
    Other(String),
}

impl fmt::Debug for AuthAuditScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Digest => formatter.write_str("Digest"),
            Self::Bearer => formatter.write_str("Bearer"),
            Self::Basic => formatter.write_str("Basic"),
            Self::Aka => formatter.write_str("Aka"),
            Self::ApiKey => formatter.write_str("ApiKey"),
            Self::Password => formatter.write_str("Password"),
            Self::Token => formatter.write_str("Token"),
            Self::Other(value) => formatter
                .debug_struct("Other")
                .field("value_len", &value.len())
                .finish(),
        }
    }
}

/// Security-relevant reason for an authentication failure.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFailureReason {
    /// No credential was supplied.
    MissingCredential,
    /// Credential was malformed.
    MalformedCredential,
    /// Credential was present but invalid.
    InvalidCredential,
    /// Credential used an unsupported auth scheme or algorithm.
    UnsupportedScheme,
    /// Credential was rejected by transport or deployment policy.
    PolicyRejected,
    /// Token was expired.
    TokenExpired,
    /// Token identifier was revoked.
    TokenRevoked,
    /// Digest nonce was stale and should be re-challenged.
    StaleNonce,
    /// Digest nonce-count or proof replay was rejected.
    ReplayRejected,
    /// Backing provider was unavailable.
    ProviderUnavailable,
    /// External or future failure reason.
    Other(String),
}

impl fmt::Debug for AuthFailureReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredential => formatter.write_str("MissingCredential"),
            Self::MalformedCredential => formatter.write_str("MalformedCredential"),
            Self::InvalidCredential => formatter.write_str("InvalidCredential"),
            Self::UnsupportedScheme => formatter.write_str("UnsupportedScheme"),
            Self::PolicyRejected => formatter.write_str("PolicyRejected"),
            Self::TokenExpired => formatter.write_str("TokenExpired"),
            Self::TokenRevoked => formatter.write_str("TokenRevoked"),
            Self::StaleNonce => formatter.write_str("StaleNonce"),
            Self::ReplayRejected => formatter.write_str("ReplayRejected"),
            Self::ProviderUnavailable => formatter.write_str("ProviderUnavailable"),
            Self::Other(value) => formatter
                .debug_struct("Other")
                .field("value_len", &value.len())
                .finish(),
        }
    }
}

/// Result captured by an auth audit event.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthAuditOutcome {
    /// Authentication succeeded.
    Success,
    /// Authentication failed with a categorized reason.
    Failure(AuthFailureReason),
}

impl fmt::Debug for AuthAuditOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success => formatter.write_str("Success"),
            Self::Failure(reason) => formatter.debug_tuple("Failure").field(reason).finish(),
        }
    }
}

/// Redacted audit event for auth/security logging.
///
/// Events intentionally carry identifiers and metadata, not credential values.
/// Do not put passwords, HA1 values, bearer tokens, API keys, full
/// Authorization headers, or full JWTs into `metadata`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthAuditEvent {
    /// Scheme or auth subsystem involved.
    pub scheme: AuthAuditScheme,
    /// Success/failure result.
    pub outcome: AuthAuditOutcome,
    /// User, subject, token id, SIP username, or API key id when known.
    pub subject: Option<String>,
    /// Auth realm, issuer, tenant, or provider name when known.
    pub realm: Option<String>,
    /// Source peer, IP, connection id, or SIP source when known.
    pub peer: Option<String>,
    /// Additional non-secret attributes.
    pub metadata: BTreeMap<String, String>,
}

impl fmt::Debug for AuthAuditEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthAuditEvent")
            .field("scheme", &self.scheme)
            .field("outcome", &self.outcome)
            .field("subject_present", &self.subject.is_some())
            .field("realm_present", &self.realm.is_some())
            .field("peer_present", &self.peer.is_some())
            .field("metadata_entry_count", &self.metadata.len())
            .finish()
    }
}

impl AuthAuditEvent {
    /// Create an audit event without optional identifiers.
    pub fn new(scheme: AuthAuditScheme, outcome: AuthAuditOutcome) -> Self {
        Self {
            scheme,
            outcome,
            subject: None,
            realm: None,
            peer: None,
            metadata: BTreeMap::new(),
        }
    }

    /// Attach a non-secret subject identifier.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Attach a non-secret realm, issuer, tenant, or provider name.
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
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

/// Sink for security audit events.
///
/// Production implementations usually write to structured logging, SIEM, an
/// audit database, or a message bus. Applications decide whether an unavailable
/// sink is fail-open or fail-closed at the call site.
#[async_trait]
pub trait AuthAuditSink: Send + Sync {
    /// Record a redacted auth audit event.
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError>;
}

/// Authentication operation subject to rate limits or lockout policy.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub enum AuthRateLimitKind {
    /// Protocol-normal initial SIP authentication challenge issuance.
    ///
    /// This is deliberately separate from credential validation so providers
    /// can apply a bounded per-peer challenge budget without consuming a
    /// subject's invalid-credential budget before a subject is known.
    SipChallenge,
    /// SIP REGISTER attempts.
    SipRegister,
    /// SIP request authentication outside REGISTER.
    SipRequest,
    /// Basic username/password verification.
    BasicPassword,
    /// Direct login password verification.
    Password,
    /// API key verification.
    ApiKey,
    /// Bearer token validation.
    BearerToken,
    /// Token issuance or refresh.
    TokenIssuance,
    /// SIP Digest validation.
    Digest,
    /// External or future operation.
    Other(String),
}

impl fmt::Debug for AuthRateLimitKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SipChallenge => formatter.write_str("SipChallenge"),
            Self::SipRegister => formatter.write_str("SipRegister"),
            Self::SipRequest => formatter.write_str("SipRequest"),
            Self::BasicPassword => formatter.write_str("BasicPassword"),
            Self::Password => formatter.write_str("Password"),
            Self::ApiKey => formatter.write_str("ApiKey"),
            Self::BearerToken => formatter.write_str("BearerToken"),
            Self::TokenIssuance => formatter.write_str("TokenIssuance"),
            Self::Digest => formatter.write_str("Digest"),
            Self::Other(value) => formatter
                .debug_struct("Other")
                .field("value_len", &value.len())
                .finish(),
        }
    }
}

/// Rate-limit key. Fields are optional so applications can key by peer, realm,
/// subject, or any combination their deployment supports.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthRateLimitKey {
    /// Operation category.
    pub kind: AuthRateLimitKind,
    /// Subject or username when known.
    pub subject: Option<String>,
    /// Realm, issuer, tenant, or provider name when known.
    pub realm: Option<String>,
    /// Source peer, IP, connection id, or SIP source when known.
    pub peer: Option<String>,
}

impl fmt::Debug for AuthRateLimitKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthRateLimitKey")
            .field("kind", &self.kind)
            .field("subject_present", &self.subject.is_some())
            .field("realm_present", &self.realm.is_some())
            .field("peer_present", &self.peer.is_some())
            .finish()
    }
}

impl AuthRateLimitKey {
    /// Create a key for an auth operation.
    pub fn new(kind: AuthRateLimitKind) -> Self {
        Self {
            kind,
            subject: None,
            realm: None,
            peer: None,
        }
    }

    /// Attach a subject or username.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Attach a realm, issuer, tenant, or provider name.
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }

    /// Attach a peer identifier.
    pub fn with_peer(mut self, peer: impl Into<String>) -> Self {
        self.peer = Some(peer.into());
        self
    }
}

/// Rate-limit decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRateLimitVerdict {
    /// Request is allowed.
    Allowed,
    /// Request is denied by rate-limit or lockout policy.
    Denied {
        /// Suggested retry delay when known.
        retry_after: Option<Duration>,
    },
}

/// Opaque handle for one atomically admitted authentication attempt.
///
/// Callers must return this handle exactly once through
/// [`AuthRateLimiter::complete_auth_attempt`]. Providers use it to avoid
/// double-counting the same attempt and to release successful reservations
/// without releasing unrelated peer failures. Providers must issue an
/// unpredictable identifier that is unique among their live reservations.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthAttemptReservation {
    opaque_id: String,
}

impl AuthAttemptReservation {
    /// Construct a provider-owned opaque reservation handle.
    pub fn new(opaque_id: impl Into<String>) -> Result<Self, CredentialAuthError> {
        let opaque_id = opaque_id.into();
        if opaque_id.is_empty()
            || opaque_id.len() > 128
            || opaque_id.trim() != opaque_id
            || opaque_id.chars().any(char::is_control)
        {
            return Err(CredentialAuthError::PolicyRejected(
                "invalid auth-attempt reservation identifier".to_string(),
            ));
        }
        Ok(Self { opaque_id })
    }

    /// Return the provider-owned identifier for completion.
    pub fn opaque_id(&self) -> &str {
        &self.opaque_id
    }
}

impl fmt::Debug for AuthAttemptReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthAttemptReservation")
            .field("opaque_id_len", &self.opaque_id.len())
            .finish()
    }
}

/// Atomic authentication-attempt admission result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthAttemptAdmission {
    /// Capacity was reserved and credential validation may proceed.
    Reserved(AuthAttemptReservation),
    /// The aggregate peer or subject cohort is at capacity.
    Denied {
        /// Suggested retry delay when known.
        retry_after: Option<Duration>,
    },
}

/// Provider contract for rate-limit and lockout policy.
#[async_trait]
pub trait AuthRateLimiter: Send + Sync {
    /// Check whether an auth attempt is allowed before validating credentials.
    async fn check_auth_attempt(
        &self,
        key: &AuthRateLimitKey,
    ) -> Result<AuthRateLimitVerdict, CredentialAuthError>;

    /// Record the outcome after an auth attempt is evaluated.
    async fn record_auth_result(
        &self,
        key: &AuthRateLimitKey,
        outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError>;

    /// Atomically reserve capacity before credential validation.
    ///
    /// The default fails closed so legacy providers remain source compatible
    /// without preserving the check-then-record race. Secure callers use this
    /// method instead of [`Self::check_auth_attempt`].
    async fn reserve_auth_attempt(
        &self,
        _key: &AuthRateLimitKey,
    ) -> Result<AuthAttemptAdmission, CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "atomic auth-attempt admission is not implemented".to_string(),
        ))
    }

    /// Complete a previously reserved attempt exactly once.
    ///
    /// Successful attempts release their own reserved capacity; failed
    /// attempts retain one count through the provider's fixed window. The
    /// default fails closed for legacy implementations.
    async fn complete_auth_attempt(
        &self,
        _reservation: &AuthAttemptReservation,
        _outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError> {
        Err(CredentialAuthError::PolicyRejected(
            "atomic auth-attempt completion is not implemented".to_string(),
        ))
    }
}
