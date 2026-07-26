// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use moq_native_ietf::tls::PeerIdentity;
use moq_transport::session::{SessionTarget, SetupAuthorization, Transport};
use ring::rand::SecureRandom;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "runtime", derive(clap::ValueEnum))]
pub enum ListenerSecurityPolicy {
    /// Relay/origin ingress: verified client certificate and publish claim.
    MutualTlsPublisher,
    /// Relay-to-relay upstream ingress: verified client certificate and an
    /// exact subscribe-only claim. This is deliberately distinct from the
    /// origin publisher role so a relay pulling media cannot publish into the
    /// upstream namespace.
    MutualTlsRelaySubscriber,
    /// Browser listener ingress: mandatory SETUP token and subscribe-only
    /// claim over WebTransport.
    TokenSubscriber,
    /// Native listener ingress: mandatory SETUP token and subscribe-only
    /// claim over raw QUIC with the draft-19 ALPN.
    RawQuicTokenSubscriber,
    /// Explicitly insecure local development listener.
    Development,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AuthenticationMethod {
    MutualTls,
    SetupToken,
    Development,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionPrincipal {
    subject: String,
    pub method: AuthenticationMethod,
}

impl AdmissionPrincipal {
    pub const MAX_SUBJECT_BYTES: usize = 256;

    pub fn new(subject: impl Into<String>, method: AuthenticationMethod) -> anyhow::Result<Self> {
        let subject = subject.into();
        anyhow::ensure!(
            !subject.is_empty(),
            "admission principal subject cannot be empty"
        );
        anyhow::ensure!(
            subject.len() <= Self::MAX_SUBJECT_BYTES,
            "admission principal subject exceeds 256 bytes"
        );
        Ok(Self { subject, method })
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionClaims {
    pub scope: Option<String>,
    pub publish: bool,
    pub subscribe: bool,
    /// Required for production token listeners.
    pub expires_at_unix_seconds: Option<u64>,
    /// Replay-protected token identifier supplied by a production validator.
    pub token_id: Option<String>,
}

impl AdmissionClaims {
    pub const MAX_SCOPE_BYTES: usize = 512;

    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(scope) = &self.scope {
            anyhow::ensure!(
                !scope.is_empty() && scope.len() <= Self::MAX_SCOPE_BYTES,
                "admission scope claim must contain 1 to 512 bytes"
            );
        }
        if let Some(token_id) = &self.token_id {
            anyhow::ensure!(
                !token_id.is_empty() && token_id.len() <= 256,
                "admission token ID must contain 1 to 256 bytes"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionDecision {
    pub principal: AdmissionPrincipal,
    pub claims: AdmissionClaims,
}

impl AdmissionDecision {
    pub fn new(principal: AdmissionPrincipal, claims: AdmissionClaims) -> anyhow::Result<Self> {
        claims.validate()?;
        Ok(Self { principal, claims })
    }
}

/// Server-generated identifier for one accepted transport session.
///
/// This value is deliberately independent from QUIC connection IDs, request
/// paths, and peer credentials. In particular, a QUIC original destination
/// connection ID is selected by the client and is therefore not a safe replay
/// binding. Relay-generated IDs contain 128 bits from the operating system
/// CSPRNG and use a bounded canonical ASCII representation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AdmissionSessionId(Arc<str>);

impl AdmissionSessionId {
    pub const MAX_BYTES: usize = 128;

    pub fn generate() -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        ring::rand::SystemRandom::new()
            .fill(&mut random)
            .map_err(|_| anyhow::anyhow!("operating system CSPRNG unavailable"))?;
        Self::new(hex::encode(random))
    }

    pub fn new(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        anyhow::ensure!(!value.is_empty(), "admission session ID cannot be empty");
        anyhow::ensure!(
            value.len() <= Self::MAX_BYTES,
            "admission session ID exceeds 128 bytes"
        );
        anyhow::ensure!(
            value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'_' | b'.' | b'~')),
            "admission session ID contains a non-canonical character"
        );
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AdmissionSessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Immutable inputs available before a relay mutates coordinator or media
/// state for an inbound session.
#[derive(Clone, Copy)]
pub struct AdmissionRequest<'a> {
    pub session_id: &'a AdmissionSessionId,
    pub peer_identity: &'a PeerIdentity,
    pub target: &'a SessionTarget,
    pub substrate: Transport,
    pub negotiated_protocol: &'static str,
    pub setup_authorization: Option<&'a SetupAuthorization>,
}

impl std::fmt::Debug for AdmissionRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmissionRequest")
            .field("session_id", self.session_id)
            .field("peer_identity", self.peer_identity)
            .field("target", &self.target.redacted_for_logging())
            .field("substrate", &self.substrate)
            .field("negotiated_protocol", &self.negotiated_protocol)
            .field("setup_authorization", &self.setup_authorization)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AdmissionError {
    #[error("authenticated client certificate required")]
    MissingPeerCertificate,
    #[error("peer certificate identity is not admitted")]
    IdentityNotAllowed,
    #[error("session denied by admission policy")]
    PolicyDenied,
    #[error("session capacity exhausted")]
    CapacityExhausted,
}

/// Why an admitted session is being finalized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AdmissionCloseReason {
    PeerClosed,
    LocalClosed,
    ActivationFailed,
    AdmissionRevalidationFailed,
    ProtocolError,
    RelayShutdown,
}

impl AdmissionCloseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PeerClosed => "peer_closed",
            Self::LocalClosed => "local_closed",
            Self::ActivationFailed => "activation_failed",
            Self::AdmissionRevalidationFailed => "admission_revalidation_failed",
            Self::ProtocolError => "protocol_error",
            Self::RelayShutdown => "relay_shutdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionCloseContext {
    pub reason: AdmissionCloseReason,
    pub ended_at_unix_seconds: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AdmissionCloseError {
    #[error("admission replay state could not be finalized")]
    ReplayFinalizeUnavailable,
    #[error("admission quota lease could not be released")]
    LeaseReleaseUnavailable,
    #[error("admission lease ownership changed")]
    OwnershipMismatch,
    #[error("admission lease is in an invalid lifecycle state")]
    InvalidState,
}

/// An application-owned authorization and capacity grant held for the complete
/// admitted session lifetime.
///
/// `close` implementations must be idempotent and cancellation-safe. A store
/// timeout or cancellation must leave replay and quota state fail-closed; it
/// must never make the credential reusable. The relay awaits `close` with a
/// configured bound while both its global permit and this lease remain held.
#[async_trait]
pub trait AdmissionLease: Send + Sync {
    /// Revalidate expiry, revocation, replay ownership, and resource scopes.
    /// The default denies so a production token policy cannot accidentally
    /// retain the historical policy-only revalidation behavior.
    async fn revalidate(&self, _now_unix_seconds: u64) -> Result<(), AdmissionError> {
        Err(AdmissionError::PolicyDenied)
    }

    /// Atomically tombstone replay state and release any distributed lease.
    /// Non-token and development leases may use the no-op default.
    async fn close(&mut self, _context: AdmissionCloseContext) -> Result<(), AdmissionCloseError> {
        Ok(())
    }
}

/// Atomic admission result. The decision and its lifecycle lease are created
/// together while the raw SETUP authorization material is still available.
pub struct AdmittedSession {
    decision: AdmissionDecision,
    lease: Box<dyn AdmissionLease>,
}

impl AdmittedSession {
    pub fn new(decision: AdmissionDecision, lease: Box<dyn AdmissionLease>) -> Self {
        Self { decision, lease }
    }

    pub fn decision(&self) -> &AdmissionDecision {
        &self.decision
    }

    pub async fn revalidate(&self, now_unix_seconds: u64) -> Result<(), AdmissionError> {
        self.lease.revalidate(now_unix_seconds).await
    }

    pub async fn close(
        &mut self,
        context: AdmissionCloseContext,
    ) -> Result<(), AdmissionCloseError> {
        self.lease.close(context).await
    }
}

#[derive(Debug)]
struct UnmeteredAdmissionLease;

impl AdmissionLease for UnmeteredAdmissionLease {}

#[async_trait]
pub trait SessionAdmission: Send + Sync {
    async fn admit(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError>;

    /// Authenticate, authorize, claim replay state, and reserve policy capacity
    /// as one externally visible transaction. Legacy non-token policies use the
    /// default composition. Production token policies must override this method
    /// and advertise both lifecycle capability flags below.
    ///
    /// The relay supervises this future in an owned task and deliberately does
    /// not cancel it when the client-facing admission deadline expires. A late
    /// grant is immediately finalized through its lease. Implementations must
    /// therefore use internally bounded I/O and eventually settle; they must
    /// not depend on future cancellation to roll back claimed replay or quota
    /// state.
    async fn admit_session(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmittedSession, AdmissionError> {
        let decision = self.admit(request).await?;
        let lease = self.acquire_session_lease(&decision).await?;
        Ok(AdmittedSession::new(decision, lease))
    }

    /// Marks a built-in policy that may only be installed when the relay is
    /// explicitly running in development mode.
    fn development_only(&self) -> bool {
        false
    }

    fn allow_all(&self) -> bool {
        false
    }

    /// Production token policies must bind expiry/JTI/replay state and support
    /// periodic lease revalidation (for example through rvoip auth stores).
    fn supports_production_token_leases(&self) -> bool {
        false
    }

    fn supports_bounded_session_leases(&self) -> bool {
        false
    }

    /// The policy overrides `admit_session` so a token claim and its capacity
    /// lease cannot be separated by a cancellation window.
    fn supports_atomic_token_admission(&self) -> bool {
        false
    }

    /// The lease implements bounded, awaited replay tombstoning and release.
    fn supports_awaited_session_close(&self) -> bool {
        false
    }

    async fn revalidate(&self, _decision: &AdmissionDecision) -> Result<(), AdmissionError> {
        Err(AdmissionError::PolicyDenied)
    }

    /// Acquire policy-specific capacity after authentication and before any
    /// coordinator or media mutation. Production integrations should return
    /// a tenant/account-scoped RAII guard whose `Drop` releases the capacity.
    async fn acquire_session_lease(
        &self,
        _decision: &AdmissionDecision,
    ) -> Result<Box<dyn AdmissionLease>, AdmissionError> {
        Ok(Box::new(UnmeteredAdmissionLease))
    }
}

/// Fail-closed policy useful as a safe placeholder during configuration.
#[derive(Debug, Default)]
pub struct DenyAllAdmission;

#[async_trait]
impl SessionAdmission for DenyAllAdmission {
    async fn admit(
        &self,
        _request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        Err(AdmissionError::PolicyDenied)
    }

    fn supports_bounded_session_leases(&self) -> bool {
        true
    }
}

/// Explicitly development-only permissive policy.
#[derive(Debug)]
pub struct DevelopmentAllowAllAdmission {
    _private: (),
}

impl DevelopmentAllowAllAdmission {
    pub fn explicitly_enabled() -> Arc<Self> {
        Arc::new(Self { _private: () })
    }
}

#[async_trait]
impl SessionAdmission for DevelopmentAllowAllAdmission {
    async fn admit(
        &self,
        _request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        AdmissionDecision::new(
            AdmissionPrincipal::new("development-anonymous", AuthenticationMethod::Development)
                .expect("static development principal is valid"),
            AdmissionClaims {
                scope: None,
                publish: true,
                subscribe: true,
                expires_at_unix_seconds: None,
                token_id: None,
            },
        )
        .map_err(|_| AdmissionError::PolicyDenied)
    }

    fn development_only(&self) -> bool {
        true
    }

    fn allow_all(&self) -> bool {
        true
    }
}

/// Admit only verified TLS peers whose leaf certificate fingerprint appears
/// in an explicit SHA-256 allowlist.
#[derive(Debug)]
pub struct CertificateFingerprintAdmission {
    bindings: HashMap<[u8; 32], HashSet<String>>,
    capacity: HashMap<[u8; 32], Arc<tokio::sync::Semaphore>>,
    role: CertificateAdmissionRole,
}

/// Least-privilege capability profile attached to a certificate allowlist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CertificateAdmissionRole {
    /// Origin or gateway may publish, but cannot subscribe.
    Publisher,
    /// Downstream relay may subscribe or fetch, but cannot publish.
    RelaySubscriber,
}

impl CertificateAdmissionRole {
    const fn claims(self, scope: String) -> AdmissionClaims {
        AdmissionClaims {
            scope: Some(scope),
            publish: matches!(self, Self::Publisher),
            subscribe: matches!(self, Self::RelaySubscriber),
            expires_at_unix_seconds: None,
            token_id: None,
        }
    }

    fn matches(self, decision: &AdmissionDecision) -> bool {
        decision.principal.method == AuthenticationMethod::MutualTls
            && decision.claims.scope.is_some()
            && decision.claims.publish == matches!(self, Self::Publisher)
            && decision.claims.subscribe == matches!(self, Self::RelaySubscriber)
            && decision.claims.expires_at_unix_seconds.is_none()
            && decision.claims.token_id.is_none()
    }
}

struct FingerprintAdmissionLease {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl AdmissionLease for FingerprintAdmissionLease {}

impl CertificateFingerprintAdmission {
    /// Legacy constructor retained so callers fail with a clear migration
    /// error instead of accidentally creating an unscoped production policy.
    #[deprecated(note = "use new_scoped with explicit publisher scopes")]
    pub fn new(fingerprints: impl IntoIterator<Item = String>) -> anyhow::Result<Arc<Self>> {
        let _ = fingerprints.into_iter().count();
        anyhow::bail!("certificate admission requires at least one explicit publisher scope")
    }

    pub fn new_scoped(
        fingerprints: impl IntoIterator<Item = String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<Arc<Self>> {
        let fingerprints = fingerprints.into_iter().collect::<Vec<_>>();
        anyhow::ensure!(
            fingerprints.len() == 1,
            "new_scoped supports one certificate security domain; use new_bindings for multiple principals"
        );
        let fingerprint = fingerprints.into_iter().next().unwrap();
        Self::new_bindings(
            scopes
                .into_iter()
                .map(|scope| format!("{fingerprint}={scope}")),
        )
    }

    /// Create explicit `sha256=path-scope` bindings. Repeat the same
    /// fingerprint to authorize multiple scopes without creating a Cartesian
    /// product between principals and tenants.
    pub fn new_bindings(bindings: impl IntoIterator<Item = String>) -> anyhow::Result<Arc<Self>> {
        Self::new_bindings_with_limit(bindings, 100)
    }

    pub fn new_bindings_with_limit(
        bindings: impl IntoIterator<Item = String>,
        max_active_sessions_per_fingerprint: usize,
    ) -> anyhow::Result<Arc<Self>> {
        Self::new_bindings_for_role_with_limit(
            bindings,
            CertificateAdmissionRole::Publisher,
            max_active_sessions_per_fingerprint,
        )
    }

    /// Create subscribe-only relay/upstream certificate bindings.
    pub fn new_relay_subscriber_bindings(
        bindings: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<Arc<Self>> {
        Self::new_relay_subscriber_bindings_with_limit(bindings, 100)
    }

    /// Create subscribe-only relay/upstream certificate bindings with an
    /// explicit per-fingerprint active-session bound.
    pub fn new_relay_subscriber_bindings_with_limit(
        bindings: impl IntoIterator<Item = String>,
        max_active_sessions_per_fingerprint: usize,
    ) -> anyhow::Result<Arc<Self>> {
        Self::new_bindings_for_role_with_limit(
            bindings,
            CertificateAdmissionRole::RelaySubscriber,
            max_active_sessions_per_fingerprint,
        )
    }

    /// Create bindings for one explicit certificate capability profile.
    ///
    /// A single policy instance cannot mix publisher and relay-subscriber
    /// certificates. Deploy them on role-separated listeners so TLS posture,
    /// request routing, and capacity remain independently auditable.
    pub fn new_bindings_for_role_with_limit(
        bindings: impl IntoIterator<Item = String>,
        role: CertificateAdmissionRole,
        max_active_sessions_per_fingerprint: usize,
    ) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            max_active_sessions_per_fingerprint > 0,
            "per-fingerprint active-session limit must be positive"
        );
        let mut allowed = HashMap::<[u8; 32], HashSet<String>>::new();
        for binding in bindings {
            let (fingerprint, scope) = binding.split_once('=').ok_or_else(|| {
                anyhow::anyhow!("certificate bindings must use SHA256=/path/scope")
            })?;
            let normalized = fingerprint.trim().to_ascii_lowercase();
            anyhow::ensure!(
                normalized.len() == 64,
                "admitted client certificate fingerprints must be 64 hexadecimal characters"
            );
            let decoded = hex::decode(&normalized)
                .map_err(|_| anyhow::anyhow!("invalid client certificate SHA-256 fingerprint"))?;
            let fingerprint: [u8; 32] = decoded
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid client certificate SHA-256 fingerprint"))?;
            let scope = scope.to_string();
            anyhow::ensure!(
                scope.starts_with('/')
                    && !scope.contains(['?', '#'])
                    && scope.len() <= AdmissionClaims::MAX_SCOPE_BYTES,
                "certificate scopes must be bounded path-only values beginning with '/'"
            );
            allowed.entry(fingerprint).or_default().insert(scope);
        }
        anyhow::ensure!(
            !allowed.is_empty(),
            "at least one certificate binding is required"
        );
        let capacity = allowed
            .keys()
            .map(|fingerprint| {
                (
                    *fingerprint,
                    Arc::new(tokio::sync::Semaphore::new(
                        max_active_sessions_per_fingerprint,
                    )),
                )
            })
            .collect();
        Ok(Arc::new(Self {
            bindings: allowed,
            capacity,
            role,
        }))
    }

    fn admit_verified_fingerprint(
        &self,
        fingerprint: &[u8; 32],
        target: &SessionTarget,
    ) -> Result<AdmissionDecision, AdmissionError> {
        let scopes = self
            .bindings
            .get(fingerprint)
            .ok_or(AdmissionError::IdentityNotAllowed)?;
        // Built-in certificate admission deliberately does not accept query
        // credentials. Applications needing richer tenancy must supply an
        // external SessionAdmission implementation.
        if target.query().is_some() || !scopes.contains(target.path()) {
            return Err(AdmissionError::PolicyDenied);
        }
        AdmissionDecision::new(
            AdmissionPrincipal::new(
                format!("certificate-sha256:{}", hex::encode(fingerprint)),
                AuthenticationMethod::MutualTls,
            )
            .map_err(|_| AdmissionError::PolicyDenied)?,
            self.role.claims(target.path().to_string()),
        )
        .map_err(|_| AdmissionError::PolicyDenied)
    }
}

#[async_trait]
impl SessionAdmission for CertificateFingerprintAdmission {
    async fn admit(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        let identity = request
            .peer_identity
            .certificate()
            .ok_or(AdmissionError::MissingPeerCertificate)?;
        self.admit_verified_fingerprint(identity.leaf_sha256(), request.target)
    }

    fn supports_bounded_session_leases(&self) -> bool {
        true
    }

    async fn acquire_session_lease(
        &self,
        decision: &AdmissionDecision,
    ) -> Result<Box<dyn AdmissionLease>, AdmissionError> {
        if !self.role.matches(decision) {
            return Err(AdmissionError::PolicyDenied);
        }
        let fingerprint: [u8; 32] = decision
            .principal
            .subject()
            .strip_prefix("certificate-sha256:")
            .and_then(|value| hex::decode(value).ok())
            .and_then(|value| value.try_into().ok())
            .ok_or(AdmissionError::PolicyDenied)?;
        let scope = decision
            .claims
            .scope
            .as_deref()
            .ok_or(AdmissionError::PolicyDenied)?;
        if !self
            .bindings
            .get(&fingerprint)
            .is_some_and(|scopes| scopes.contains(scope))
        {
            return Err(AdmissionError::PolicyDenied);
        }
        let permit = self
            .capacity
            .get(&fingerprint)
            .ok_or(AdmissionError::PolicyDenied)?
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::CapacityExhausted)?;
        Ok(Box::new(FingerprintAdmissionLease { _permit: permit }))
    }
}

/// Development/static-operator subscribe-only token admission.
///
/// This intentionally cannot be used as a production browser admission
/// policy: static digests provide no tenant/scope, expiry, replay, or JTI
/// guarantees. Production integrations must implement [`SessionAdmission`]
/// with atomic [`SessionAdmission::admit_session`], lease-owned revalidation
/// and close hooks, and all production token lifecycle capability flags.
#[derive(Debug)]
pub struct SetupTokenAdmission {
    allowed: HashSet<[u8; 32]>,
}

impl SetupTokenAdmission {
    pub fn new(token_sha256: impl IntoIterator<Item = String>) -> anyhow::Result<Arc<Self>> {
        let mut allowed = HashSet::new();
        for digest in token_sha256 {
            let normalized = digest.trim().to_ascii_lowercase();
            anyhow::ensure!(
                normalized.len() == 64,
                "admitted SETUP token digests must be 64 hexadecimal characters"
            );
            let decoded = hex::decode(normalized)
                .map_err(|_| anyhow::anyhow!("invalid SETUP token SHA-256 digest"))?;
            allowed.insert(
                decoded
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid SETUP token SHA-256 digest"))?,
            );
        }
        anyhow::ensure!(
            !allowed.is_empty(),
            "at least one admitted SETUP token digest is required"
        );
        Ok(Arc::new(Self { allowed }))
    }
}

#[async_trait]
impl SessionAdmission for SetupTokenAdmission {
    async fn admit(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        let token = request
            .setup_authorization
            .ok_or(AdmissionError::PolicyDenied)?;
        if token.is_empty() {
            return Err(AdmissionError::PolicyDenied);
        }
        let digest = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
        let digest: [u8; 32] = digest
            .as_ref()
            .try_into()
            .map_err(|_| AdmissionError::PolicyDenied)?;
        if !self.allowed.contains(&digest) {
            return Err(AdmissionError::PolicyDenied);
        }
        AdmissionDecision::new(
            AdmissionPrincipal::new(
                format!("setup-token-sha256:{}", hex::encode(digest)),
                AuthenticationMethod::SetupToken,
            )
            .map_err(|_| AdmissionError::PolicyDenied)?,
            AdmissionClaims {
                scope: None,
                publish: false,
                subscribe: true,
                expires_at_unix_seconds: None,
                token_id: Some(hex::encode(digest)),
            },
        )
        .map_err(|_| AdmissionError::PolicyDenied)
    }

    fn development_only(&self) -> bool {
        true
    }

    async fn revalidate(&self, _decision: &AdmissionDecision) -> Result<(), AdmissionError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        session_id: &'a AdmissionSessionId,
        peer_identity: &'a PeerIdentity,
        target: &'a SessionTarget,
        authorization: Option<&'a SetupAuthorization>,
    ) -> AdmissionRequest<'a> {
        AdmissionRequest {
            session_id,
            peer_identity,
            target,
            substrate: Transport::RawQuic,
            negotiated_protocol: "moqt-19",
            setup_authorization: authorization,
        }
    }

    #[tokio::test]
    async fn static_token_policy_is_subscribe_only_and_development_only() {
        let session_id = AdmissionSessionId::new("test-session").unwrap();
        let token = SetupAuthorization::new(b"listener-token").unwrap();
        let digest = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
        let policy = SetupTokenAdmission::new([hex::encode(digest.as_ref())]).unwrap();
        let target: SessionTarget = "moqt://relay.example/listen".parse().unwrap();
        let peer = PeerIdentity::Anonymous;
        let decision = policy
            .admit(request(&session_id, &peer, &target, Some(&token)))
            .await
            .unwrap();

        assert!(policy.development_only());
        assert!(!policy.supports_production_token_leases());
        assert!(decision.claims.subscribe);
        assert!(!decision.claims.publish);
        assert_eq!(decision.principal.method, AuthenticationMethod::SetupToken);
        assert!(policy
            .admit(request(
                &session_id,
                &peer,
                &target,
                Some(&SetupAuthorization::new(b"wrong").unwrap()),
            ))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn deny_and_development_allow_all_are_explicit() {
        let session_id = AdmissionSessionId::new("test-session").unwrap();
        let target: SessionTarget = "moqt://relay.example/".parse().unwrap();
        let peer = PeerIdentity::Anonymous;
        assert!(DenyAllAdmission
            .admit(request(&session_id, &peer, &target, None))
            .await
            .is_err());
        let allow = DevelopmentAllowAllAdmission::explicitly_enabled();
        assert!(allow.allow_all());
        assert!(allow.development_only());
        assert!(allow
            .admit(request(&session_id, &peer, &target, None))
            .await
            .is_ok());
    }

    #[test]
    fn generated_session_ids_are_canonical_and_ignore_reused_client_odcid() {
        let reused_client_odcid = "aabbccddeeff0011";
        let first = AdmissionSessionId::generate().unwrap();
        let second = AdmissionSessionId::generate().unwrap();
        assert_ne!(first, second);
        assert_ne!(first.as_str(), reused_client_odcid);
        assert_ne!(second.as_str(), reused_client_odcid);
        assert_eq!(first.as_str().len(), 32);
        assert!(first.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(AdmissionSessionId::new("client/cid").is_err());
    }

    #[test]
    fn admission_request_debug_redacts_target_query_and_setup_token() {
        let session_id = AdmissionSessionId::new("test-session").unwrap();
        let target: SessionTarget = "moqt://relay.example/live?token=query-secret"
            .parse()
            .unwrap();
        let peer = PeerIdentity::Anonymous;
        let token = SetupAuthorization::new(b"setup-secret").unwrap();
        let debug = format!("{:?}", request(&session_id, &peer, &target, Some(&token)));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("query-secret"));
        assert!(!debug.contains("setup-secret"));
    }

    #[test]
    fn certificate_policy_is_scope_bound_and_publisher_only() {
        let fingerprint = [0x42; 32];
        let policy = CertificateFingerprintAdmission::new_scoped(
            [hex::encode(fingerprint)],
            ["/tenant-a/live".to_string()],
        )
        .unwrap();
        let allowed: SessionTarget = "moqt://relay.example/tenant-a/live".parse().unwrap();
        let decision = policy
            .admit_verified_fingerprint(&fingerprint, &allowed)
            .unwrap();
        assert_eq!(decision.claims.scope.as_deref(), Some("/tenant-a/live"));
        assert!(decision.claims.publish);
        assert!(!decision.claims.subscribe);

        let cross_scope: SessionTarget = "moqt://relay.example/tenant-b/live".parse().unwrap();
        assert_eq!(
            policy.admit_verified_fingerprint(&fingerprint, &cross_scope),
            Err(AdmissionError::PolicyDenied)
        );
        let query: SessionTarget = "moqt://relay.example/tenant-a/live?token=secret"
            .parse()
            .unwrap();
        assert_eq!(
            policy.admit_verified_fingerprint(&fingerprint, &query),
            Err(AdmissionError::PolicyDenied)
        );
    }

    #[test]
    fn relay_subscriber_certificate_policy_is_scope_bound_and_subscribe_only() {
        let fingerprint = [0x24; 32];
        let policy = CertificateFingerprintAdmission::new_relay_subscriber_bindings([format!(
            "{}=/tenant-a/live",
            hex::encode(fingerprint)
        )])
        .unwrap();
        let allowed: SessionTarget = "moqt://relay.example/tenant-a/live".parse().unwrap();
        let decision = policy
            .admit_verified_fingerprint(&fingerprint, &allowed)
            .unwrap();
        assert_eq!(decision.claims.scope.as_deref(), Some("/tenant-a/live"));
        assert!(!decision.claims.publish);
        assert!(decision.claims.subscribe);
        assert_eq!(decision.principal.method, AuthenticationMethod::MutualTls);

        let cross_scope: SessionTarget = "moqt://relay.example/tenant-b/live".parse().unwrap();
        assert_eq!(
            policy.admit_verified_fingerprint(&fingerprint, &cross_scope),
            Err(AdmissionError::PolicyDenied)
        );
    }

    #[tokio::test]
    async fn certificate_lease_rejects_capability_or_scope_escalation() {
        let fingerprint = [0x24; 32];
        let policy = CertificateFingerprintAdmission::new_relay_subscriber_bindings([format!(
            "{}=/tenant-a/live",
            hex::encode(fingerprint)
        )])
        .unwrap();
        let allowed: SessionTarget = "moqt://relay.example/tenant-a/live".parse().unwrap();
        let decision = policy
            .admit_verified_fingerprint(&fingerprint, &allowed)
            .unwrap();
        assert!(policy.acquire_session_lease(&decision).await.is_ok());

        let publish_escalation = AdmissionDecision::new(
            decision.principal.clone(),
            AdmissionClaims {
                publish: true,
                subscribe: false,
                ..decision.claims.clone()
            },
        )
        .unwrap();
        assert_eq!(
            policy
                .acquire_session_lease(&publish_escalation)
                .await
                .err(),
            Some(AdmissionError::PolicyDenied)
        );

        let scope_escalation = AdmissionDecision::new(
            decision.principal,
            AdmissionClaims {
                scope: Some("/tenant-b/live".to_owned()),
                ..decision.claims
            },
        )
        .unwrap();
        assert_eq!(
            policy.acquire_session_lease(&scope_escalation).await.err(),
            Some(AdmissionError::PolicyDenied)
        );
    }

    #[test]
    fn certificate_bindings_do_not_form_a_cross_tenant_product() {
        let tenant_a = [0xAA; 32];
        let tenant_b = [0xBB; 32];
        let policy = CertificateFingerprintAdmission::new_bindings([
            format!("{}=/tenant-a/live", hex::encode(tenant_a)),
            format!("{}=/tenant-b/live", hex::encode(tenant_b)),
        ])
        .unwrap();
        let scope_a: SessionTarget = "moqt://relay.example/tenant-a/live".parse().unwrap();
        let scope_b: SessionTarget = "moqt://relay.example/tenant-b/live".parse().unwrap();

        assert!(policy
            .admit_verified_fingerprint(&tenant_a, &scope_a)
            .is_ok());
        assert!(policy
            .admit_verified_fingerprint(&tenant_b, &scope_b)
            .is_ok());
        assert_eq!(
            policy.admit_verified_fingerprint(&tenant_a, &scope_b),
            Err(AdmissionError::PolicyDenied)
        );
        assert_eq!(
            policy.admit_verified_fingerprint(&tenant_b, &scope_a),
            Err(AdmissionError::PolicyDenied)
        );
    }

    #[tokio::test]
    async fn certificate_capacity_is_fair_per_principal_and_releases() {
        let tenant_a = [0xAA; 32];
        let tenant_b = [0xBB; 32];
        let policy = CertificateFingerprintAdmission::new_bindings_with_limit(
            [
                format!("{}=/tenant-a/live", hex::encode(tenant_a)),
                format!("{}=/tenant-b/live", hex::encode(tenant_b)),
            ],
            1,
        )
        .unwrap();
        let scope_a: SessionTarget = "moqt://relay.example/tenant-a/live".parse().unwrap();
        let scope_b: SessionTarget = "moqt://relay.example/tenant-b/live".parse().unwrap();
        let decision_a = policy
            .admit_verified_fingerprint(&tenant_a, &scope_a)
            .unwrap();
        let decision_b = policy
            .admit_verified_fingerprint(&tenant_b, &scope_b)
            .unwrap();

        let lease_a = policy.acquire_session_lease(&decision_a).await.unwrap();
        assert_eq!(
            policy.acquire_session_lease(&decision_a).await.err(),
            Some(AdmissionError::CapacityExhausted)
        );
        let lease_b = policy.acquire_session_lease(&decision_b).await.unwrap();
        drop(lease_a);
        assert!(policy.acquire_session_lease(&decision_a).await.is_ok());
        drop(lease_b);
    }
}
