//! Dynamic, application-authorized MOQT publisher admission.
//!
//! The certificate ceiling is intentionally evaluated before the application
//! authority. A backend grant can narrow a certificate's rights, but can never
//! expand them into another tenant or an unrelated exact namespace.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moq_relay_ietf::{
    AdmissionClaims, AdmissionCloseContext, AdmissionCloseError, AdmissionDecision, AdmissionError,
    AdmissionLease, AdmissionPrincipal, AdmissionRequest, AdmittedSession,
    AuthenticationMethod as RelayAuthenticationMethod, SessionAdmission,
};
use moq_transport::session::Transport;

use crate::{MoqNamespace, MOQT_NEGOTIATED_PROTOCOL};

const MAX_AUTHORITY_FENCE_BYTES: usize = 128;

/// Maximum namespace set one verified client certificate may publish.
///
/// `TenantPrefix` means the canonical path prefix `/{tenant_id}/`; matching is
/// performed on parsed namespace components, never with string prefix logic.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MoqPublisherNamespaceCeiling {
    Exact(MoqNamespace),
    TenantPrefix { tenant_id: String },
}

impl MoqPublisherNamespaceCeiling {
    pub fn exact(namespace: MoqNamespace) -> Self {
        Self::Exact(namespace)
    }

    pub fn tenant_prefix(tenant_id: impl Into<String>) -> Result<Self, MoqPublisherAdmissionError> {
        let tenant_id = tenant_id.into();
        validate_tenant_component(&tenant_id)?;
        Ok(Self::TenantPrefix { tenant_id })
    }

    fn validate(&self) -> Result<(), MoqPublisherAdmissionError> {
        match self {
            Self::Exact(_) => Ok(()),
            Self::TenantPrefix { tenant_id } => validate_tenant_component(tenant_id),
        }
    }

    fn admits(&self, namespace: &MoqNamespace) -> bool {
        match self {
            Self::Exact(expected) => expected == namespace,
            Self::TenantPrefix { tenant_id } => namespace.tenant_id() == tenant_id,
        }
    }
}

/// One verified leaf-certificate fingerprint and its maximum namespace set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqPublisherCertificateBinding {
    pub certificate_sha256: String,
    pub namespace_ceiling: MoqPublisherNamespaceCeiling,
}

/// Credential-free request sent to an application publication authority.
#[derive(Clone, Eq, PartialEq)]
pub struct MoqPublisherPublicationRequest {
    certificate_sha256: [u8; 32],
    namespace: MoqNamespace,
}

impl MoqPublisherPublicationRequest {
    pub fn new(certificate_sha256: [u8; 32], namespace: MoqNamespace) -> Self {
        Self {
            certificate_sha256,
            namespace,
        }
    }

    pub fn certificate_sha256_hex(&self) -> String {
        encode_hex(&self.certificate_sha256)
    }

    pub fn namespace(&self) -> &MoqNamespace {
        &self.namespace
    }
}

impl std::fmt::Debug for MoqPublisherPublicationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqPublisherPublicationRequest")
            .field("certificate_sha256", &"<redacted>")
            .field("namespace", &self.namespace)
            .finish()
    }
}

/// Generation-fenced application grant for one exact publication.
#[derive(Clone, Eq, PartialEq)]
pub struct MoqPublisherPublicationGrant {
    fence: String,
    expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for MoqPublisherPublicationGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqPublisherPublicationGrant")
            .field("fence", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl MoqPublisherPublicationGrant {
    pub fn new(
        fence: impl Into<String>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, MoqPublisherAdmissionError> {
        let fence = fence.into();
        if fence.is_empty()
            || fence.len() > MAX_AUTHORITY_FENCE_BYTES
            || !fence.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
        {
            return Err(MoqPublisherAdmissionError::InvalidConfig(
                "publisher authority fence is invalid",
            ));
        }
        Ok(Self { fence, expires_at })
    }

    pub fn fence(&self) -> &str {
        &self.fence
    }

    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Application authority for exact, currently active publications.
///
/// Implementations must return a different fence when a deleted publication
/// is recreated. Lookups are read-only and must be cancellation-safe because
/// rvoip cancels them at the configured timeout. Backend failures return
/// `Unavailable`; callers fail closed.
#[async_trait]
pub trait MoqPublisherPublicationAuthority: Send + Sync {
    async fn active_publication(
        &self,
        request: &MoqPublisherPublicationRequest,
        now: DateTime<Utc>,
    ) -> Result<Option<MoqPublisherPublicationGrant>, MoqPublisherPublicationAuthorityError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MoqPublisherPublicationAuthorityError {
    #[error("MOQT publisher publication authority is unavailable")]
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqPublisherAdmissionConfig {
    pub operation_timeout: Duration,
    pub max_active_sessions_per_certificate: usize,
}

impl MoqPublisherAdmissionConfig {
    pub fn new(
        operation_timeout: Duration,
        max_active_sessions_per_certificate: usize,
    ) -> Result<Self, MoqPublisherAdmissionError> {
        if operation_timeout.is_zero() {
            return Err(MoqPublisherAdmissionError::InvalidConfig(
                "publisher authority timeout must be greater than zero",
            ));
        }
        if max_active_sessions_per_certificate == 0
            || max_active_sessions_per_certificate > tokio::sync::Semaphore::MAX_PERMITS
        {
            return Err(MoqPublisherAdmissionError::InvalidConfig(
                "publisher per-certificate session limit is invalid",
            ));
        }
        Ok(Self {
            operation_timeout,
            max_active_sessions_per_certificate,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MoqPublisherAdmissionError {
    #[error("invalid MOQT publisher admission configuration: {0}")]
    InvalidConfig(&'static str),
}

/// mTLS publisher policy combining a certificate ceiling with a live grant.
pub struct RvoipMoqPublisherAdmission {
    ceilings: HashMap<[u8; 32], Vec<MoqPublisherNamespaceCeiling>>,
    capacity: HashMap<[u8; 32], Arc<tokio::sync::Semaphore>>,
    authority: Arc<dyn MoqPublisherPublicationAuthority>,
    config: MoqPublisherAdmissionConfig,
}

impl std::fmt::Debug for RvoipMoqPublisherAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RvoipMoqPublisherAdmission")
            .field("certificate_count", &self.ceilings.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RvoipMoqPublisherAdmission {
    pub fn new(
        bindings: impl IntoIterator<Item = MoqPublisherCertificateBinding>,
        authority: Arc<dyn MoqPublisherPublicationAuthority>,
        config: MoqPublisherAdmissionConfig,
    ) -> Result<Self, MoqPublisherAdmissionError> {
        let config = MoqPublisherAdmissionConfig::new(
            config.operation_timeout,
            config.max_active_sessions_per_certificate,
        )?;
        let mut ceilings = HashMap::<[u8; 32], Vec<MoqPublisherNamespaceCeiling>>::new();
        for binding in bindings {
            binding.namespace_ceiling.validate()?;
            let fingerprint = decode_fingerprint(&binding.certificate_sha256)?;
            ceilings
                .entry(fingerprint)
                .or_default()
                .push(binding.namespace_ceiling);
        }
        if ceilings.is_empty() {
            return Err(MoqPublisherAdmissionError::InvalidConfig(
                "at least one publisher certificate ceiling is required",
            ));
        }
        let capacity = ceilings
            .keys()
            .map(|fingerprint| {
                (
                    *fingerprint,
                    Arc::new(tokio::sync::Semaphore::new(
                        config.max_active_sessions_per_certificate,
                    )),
                )
            })
            .collect();
        Ok(Self {
            ceilings,
            capacity,
            authority,
            config,
        })
    }

    pub const fn config(&self) -> MoqPublisherAdmissionConfig {
        self.config
    }

    async fn prepare_fingerprint(
        &self,
        certificate_sha256: [u8; 32],
        namespace: MoqNamespace,
        now: DateTime<Utc>,
    ) -> Result<PreparedPublisherAdmission, AdmissionError> {
        let ceilings = self
            .ceilings
            .get(&certificate_sha256)
            .ok_or(AdmissionError::IdentityNotAllowed)?;
        if !ceilings.iter().any(|ceiling| ceiling.admits(&namespace)) {
            return Err(AdmissionError::PolicyDenied);
        }
        let request = MoqPublisherPublicationRequest::new(certificate_sha256, namespace);
        let grant = self.lookup_bounded(request.clone(), now).await?;
        if grant.expires_at <= now {
            return Err(AdmissionError::PolicyDenied);
        }
        let permit = self
            .capacity
            .get(&certificate_sha256)
            .ok_or(AdmissionError::PolicyDenied)?
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::CapacityExhausted)?;
        let expires_at_unix_seconds = u64::try_from(grant.expires_at.timestamp())
            .map_err(|_| AdmissionError::PolicyDenied)?;
        let exact_scope = format!("/{}", request.namespace);
        let decision = AdmissionDecision::new(
            AdmissionPrincipal::new(
                format!("certificate-sha256:{}", encode_hex(&certificate_sha256)),
                RelayAuthenticationMethod::MutualTls,
            )
            .map_err(|_| AdmissionError::PolicyDenied)?,
            AdmissionClaims {
                scope: Some(exact_scope),
                publish: true,
                subscribe: false,
                expires_at_unix_seconds: Some(expires_at_unix_seconds),
                token_id: None,
            },
        )
        .map_err(|_| AdmissionError::PolicyDenied)?;
        Ok(PreparedPublisherAdmission {
            decision,
            lease: DynamicPublisherLease {
                permit: Some(permit),
                authority: self.authority.clone(),
                request,
                grant,
                operation_timeout: self.config.operation_timeout,
            },
        })
    }

    async fn lookup_bounded(
        &self,
        request: MoqPublisherPublicationRequest,
        now: DateTime<Utc>,
    ) -> Result<MoqPublisherPublicationGrant, AdmissionError> {
        match tokio::time::timeout(
            self.config.operation_timeout,
            self.authority.active_publication(&request, now),
        )
        .await
        {
            Ok(Ok(Some(grant))) => Ok(grant),
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => Err(AdmissionError::PolicyDenied),
        }
    }
}

#[async_trait]
impl SessionAdmission for RvoipMoqPublisherAdmission {
    async fn admit(
        &self,
        _request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        // Capacity and the authority fence must be retained atomically with
        // the decision, so the legacy decision-only path is unavailable.
        Err(AdmissionError::PolicyDenied)
    }

    async fn admit_session(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmittedSession, AdmissionError> {
        if request.substrate != Transport::RawQuic
            || request.negotiated_protocol != MOQT_NEGOTIATED_PROTOCOL
            || request.setup_authorization.is_some()
        {
            return Err(AdmissionError::PolicyDenied);
        }
        let certificate = request
            .peer_identity
            .certificate()
            .ok_or(AdmissionError::MissingPeerCertificate)?;
        let namespace = namespace_from_target(request.target)?;
        let prepared = self
            .prepare_fingerprint(*certificate.leaf_sha256(), namespace, Utc::now())
            .await?;
        Ok(AdmittedSession::new(
            prepared.decision,
            Box::new(prepared.lease),
        ))
    }

    fn supports_bounded_session_leases(&self) -> bool {
        true
    }
}

struct PreparedPublisherAdmission {
    decision: AdmissionDecision,
    lease: DynamicPublisherLease,
}

struct DynamicPublisherLease {
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    authority: Arc<dyn MoqPublisherPublicationAuthority>,
    request: MoqPublisherPublicationRequest,
    grant: MoqPublisherPublicationGrant,
    operation_timeout: Duration,
}

#[async_trait]
impl AdmissionLease for DynamicPublisherLease {
    async fn revalidate(&self, now_unix_seconds: u64) -> Result<(), AdmissionError> {
        if self.permit.is_none() {
            return Err(AdmissionError::PolicyDenied);
        }
        let now =
            datetime_from_unix_seconds(now_unix_seconds).ok_or(AdmissionError::PolicyDenied)?;
        if self.grant.expires_at <= now {
            return Err(AdmissionError::PolicyDenied);
        }
        match tokio::time::timeout(
            self.operation_timeout,
            self.authority.active_publication(&self.request, now),
        )
        .await
        {
            Ok(Ok(Some(active))) if active.fence == self.grant.fence && active.expires_at > now => {
                Ok(())
            }
            Ok(Ok(Some(_))) | Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                Err(AdmissionError::PolicyDenied)
            }
        }
    }

    async fn close(&mut self, _context: AdmissionCloseContext) -> Result<(), AdmissionCloseError> {
        self.permit.take();
        Ok(())
    }
}

fn namespace_from_target(
    target: &moq_transport::session::SessionTarget,
) -> Result<MoqNamespace, AdmissionError> {
    if target.query().is_some() || target.fragment().is_some() {
        return Err(AdmissionError::PolicyDenied);
    }
    let path = target
        .path()
        .strip_prefix('/')
        .ok_or(AdmissionError::PolicyDenied)?;
    MoqNamespace::parse(path).map_err(|_| AdmissionError::PolicyDenied)
}

fn validate_tenant_component(tenant_id: &str) -> Result<(), MoqPublisherAdmissionError> {
    MoqNamespace::new(tenant_id, "publisher-ceiling-validation")
        .map(|_| ())
        .map_err(|_| {
            MoqPublisherAdmissionError::InvalidConfig("publisher tenant-prefix ceiling is invalid")
        })
}

fn decode_fingerprint(value: &str) -> Result<[u8; 32], MoqPublisherAdmissionError> {
    if value.len() != 64 {
        return Err(MoqPublisherAdmissionError::InvalidConfig(
            "publisher certificate fingerprint must contain 64 hexadecimal characters",
        ));
    }
    let mut decoded = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (decode_nibble(chunk[0])? << 4) | decode_nibble(chunk[1])?;
    }
    Ok(decoded)
}

fn decode_nibble(value: u8) -> Result<u8, MoqPublisherAdmissionError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(MoqPublisherAdmissionError::InvalidConfig(
            "publisher certificate fingerprint must be hexadecimal",
        )),
    }
}

fn encode_hex(value: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in value {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn datetime_from_unix_seconds(seconds: u64) -> Option<DateTime<Utc>> {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    #[derive(Clone)]
    enum AuthorityState {
        Active(MoqPublisherPublicationGrant),
        Missing,
        Unavailable,
    }

    struct TestAuthority {
        state: Mutex<AuthorityState>,
        calls: AtomicUsize,
    }

    impl TestAuthority {
        fn new(state: AuthorityState) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(state),
                calls: AtomicUsize::new(0),
            })
        }

        fn set(&self, state: AuthorityState) {
            *self.state.lock().expect("authority state lock") = state;
        }
    }

    #[async_trait]
    impl MoqPublisherPublicationAuthority for TestAuthority {
        async fn active_publication(
            &self,
            _request: &MoqPublisherPublicationRequest,
            _now: DateTime<Utc>,
        ) -> Result<Option<MoqPublisherPublicationGrant>, MoqPublisherPublicationAuthorityError>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.state.lock().expect("authority state lock").clone() {
                AuthorityState::Active(grant) => Ok(Some(grant)),
                AuthorityState::Missing => Ok(None),
                AuthorityState::Unavailable => {
                    Err(MoqPublisherPublicationAuthorityError::Unavailable)
                }
            }
        }
    }

    fn active_grant(fence: &str) -> MoqPublisherPublicationGrant {
        MoqPublisherPublicationGrant::new(fence, Utc::now() + chrono::Duration::minutes(2))
            .expect("valid active grant")
    }

    fn policy(
        certificate: [u8; 32],
        ceiling: MoqPublisherNamespaceCeiling,
        authority: Arc<TestAuthority>,
    ) -> RvoipMoqPublisherAdmission {
        RvoipMoqPublisherAdmission::new(
            [MoqPublisherCertificateBinding {
                certificate_sha256: encode_hex(&certificate),
                namespace_ceiling: ceiling,
            }],
            authority,
            MoqPublisherAdmissionConfig::new(Duration::from_secs(1), 2).unwrap(),
        )
        .expect("valid publisher policy")
    }

    #[tokio::test]
    async fn wrong_certificate_and_cross_tenant_prefix_stop_before_authority() {
        let allowed = [7_u8; 32];
        let authority = TestAuthority::new(AuthorityState::Active(active_grant("generation-a")));
        let prefix_policy = policy(
            allowed,
            MoqPublisherNamespaceCeiling::tenant_prefix("tenant-a").unwrap(),
            authority.clone(),
        );
        assert_eq!(
            prefix_policy
                .prepare_fingerprint(
                    [8_u8; 32],
                    MoqNamespace::new("tenant-a", "broadcast-a").unwrap(),
                    Utc::now(),
                )
                .await
                .err()
                .expect("wrong certificate must be rejected"),
            AdmissionError::IdentityNotAllowed
        );
        assert_eq!(
            prefix_policy
                .prepare_fingerprint(
                    allowed,
                    MoqNamespace::new("tenant-a-evil", "broadcast-a").unwrap(),
                    Utc::now(),
                )
                .await
                .err()
                .expect("cross-tenant namespace must be rejected"),
            AdmissionError::PolicyDenied
        );
        let exact_policy = policy(
            allowed,
            MoqPublisherNamespaceCeiling::exact(
                MoqNamespace::new("tenant-a", "broadcast-a").unwrap(),
            ),
            authority.clone(),
        );
        assert_eq!(
            exact_policy
                .prepare_fingerprint(
                    allowed,
                    MoqNamespace::new("tenant-a", "broadcast-b").unwrap(),
                    Utc::now(),
                )
                .await
                .err()
                .expect("exact ceiling must reject another broadcast"),
            AdmissionError::PolicyDenied
        );
        assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn absent_expired_and_unavailable_authority_fail_closed() {
        let certificate = [9_u8; 32];
        let authority = TestAuthority::new(AuthorityState::Missing);
        let policy = policy(
            certificate,
            MoqPublisherNamespaceCeiling::tenant_prefix("tenant-a").unwrap(),
            authority.clone(),
        );
        let namespace = MoqNamespace::new("tenant-a", "broadcast-a").unwrap();
        assert!(policy
            .prepare_fingerprint(certificate, namespace.clone(), Utc::now())
            .await
            .is_err());

        authority.set(AuthorityState::Unavailable);
        assert!(policy
            .prepare_fingerprint(certificate, namespace.clone(), Utc::now())
            .await
            .is_err());

        authority.set(AuthorityState::Active(
            MoqPublisherPublicationGrant::new(
                "expired-generation",
                Utc::now() - chrono::Duration::seconds(1),
            )
            .unwrap(),
        ));
        assert!(policy
            .prepare_fingerprint(certificate, namespace, Utc::now())
            .await
            .is_err());
    }

    #[test]
    fn path_confusion_and_wildcard_like_ceilings_are_rejected() {
        for target in [
            "moqt://relay.test/tenant-a/broadcast-a/extra",
            "moqt://relay.test/tenant-a%2Fother/broadcast-a",
            "moqt://relay.test/tenant-a/../broadcast-a",
            "moqt://relay.test/tenant-a/broadcast-a?tenant=other",
            "moqt://relay.test/tenant-a/broadcast-a#track:other",
        ] {
            let target = moq_transport::session::SessionTarget::parse(target).unwrap();
            assert_eq!(
                namespace_from_target(&target),
                Err(AdmissionError::PolicyDenied),
                "target should fail closed: {target:?}"
            );
        }
        for tenant in ["*", "tenant/", "tenant%2Fother", ".private"] {
            assert!(MoqPublisherNamespaceCeiling::tenant_prefix(tenant).is_err());
        }
    }

    #[tokio::test]
    async fn revocation_or_replacement_between_admission_and_activation_is_rejected() {
        let certificate = [10_u8; 32];
        let authority = TestAuthority::new(AuthorityState::Active(active_grant("generation-a")));
        let policy = policy(
            certificate,
            MoqPublisherNamespaceCeiling::exact(
                MoqNamespace::new("tenant-a", "broadcast-a").unwrap(),
            ),
            authority.clone(),
        );
        let prepared = policy
            .prepare_fingerprint(
                certificate,
                MoqNamespace::new("tenant-a", "broadcast-a").unwrap(),
                Utc::now(),
            )
            .await
            .expect("initial active publication should be admitted");

        authority.set(AuthorityState::Missing);
        assert_eq!(
            prepared.lease.revalidate(unix_now()).await,
            Err(AdmissionError::PolicyDenied)
        );
        authority.set(AuthorityState::Active(active_grant("generation-b")));
        assert_eq!(
            prepared.lease.revalidate(unix_now()).await,
            Err(AdmissionError::PolicyDenied)
        );
    }

    #[tokio::test]
    async fn certificate_capacity_is_bounded_and_released_by_close() {
        let certificate = [11_u8; 32];
        let authority = TestAuthority::new(AuthorityState::Active(active_grant("generation-a")));
        let policy = policy(
            certificate,
            MoqPublisherNamespaceCeiling::tenant_prefix("tenant-a").unwrap(),
            authority,
        );
        let namespace = MoqNamespace::new("tenant-a", "broadcast-a").unwrap();
        let mut first = policy
            .prepare_fingerprint(certificate, namespace.clone(), Utc::now())
            .await
            .unwrap();
        assert_eq!(
            first.decision.claims.scope.as_deref(),
            Some("/tenant-a/broadcast-a")
        );
        assert!(first.decision.claims.publish);
        assert!(!first.decision.claims.subscribe);
        let _second = policy
            .prepare_fingerprint(certificate, namespace.clone(), Utc::now())
            .await
            .unwrap();
        assert_eq!(
            policy
                .prepare_fingerprint(certificate, namespace.clone(), Utc::now())
                .await
                .err(),
            Some(AdmissionError::CapacityExhausted)
        );

        first
            .lease
            .close(AdmissionCloseContext {
                reason: moq_relay_ietf::AdmissionCloseReason::PeerClosed,
                ended_at_unix_seconds: unix_now(),
            })
            .await
            .unwrap();
        policy
            .prepare_fingerprint(certificate, namespace, Utc::now())
            .await
            .expect("closed publisher lease must release certificate capacity");
    }

    fn unix_now() -> u64 {
        u64::try_from(Utc::now().timestamp()).unwrap()
    }
}
