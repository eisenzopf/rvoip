#![cfg(feature = "relay-admission")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use moq_native_ietf::tls::PeerIdentity;
use moq_relay_ietf::{
    AdmissionCloseContext, AdmissionCloseReason, AdmissionError, AdmissionRequest,
    AdmissionSessionId, SessionAdmission,
};
use moq_transport::session::{SessionTarget, SetupAuthorization, Transport};
use rvoip_auth_core::{BearerAuthError, BearerValidator, ValidatedBearer};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::{AuthenticatedPrincipal, AuthenticationMethod};
use rvoip_moq::{
    BoundedMemoryMoqReplayStore, BoundedMemoryMoqSessionLeaseStore, MoqAction,
    MoqAuthorizationError, MoqAuthorizationGrant, MoqAuthorizationRequest, MoqAuthorizer,
    MoqPeerIdentity, MoqRelayAdmissionConfig, MoqRelayAdmissionSubstrate, MoqResource,
    MoqRevocationChecker, MoqRevocationError, MoqRevocationStatus, MoqSessionLease,
    MoqSessionLeaseBinding, MoqSessionLeaseClose, MoqSessionLeaseError, MoqSessionLeaseLimits,
    MoqSessionLeaseSnapshot, MoqSessionLeaseStore, MoqTokenBinding, MoqTokenReplayStore,
    RvoipMoqRelayAdmission, SecureMoqAuthorizer, MOQT_NEGOTIATED_PROTOCOL,
};

#[derive(Clone)]
struct TemplateBearerValidator {
    principal: AuthenticatedPrincipal,
    include_token_id: bool,
    delay: StdDuration,
}

#[async_trait]
impl BearerValidator for TemplateBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(self.principal.assurance.clone())
    }

    async fn validate_credential(&self, token: &str) -> Result<ValidatedBearer, BearerAuthError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(ValidatedBearer {
            principal: self.principal.clone(),
            token_id: self
                .include_token_id
                .then(|| format!("test-token-id-{}", token.as_bytes()[0])),
            issued_at: None,
        })
    }
}

#[derive(Default)]
struct ToggleRevocation {
    revoked: AtomicBool,
}

#[async_trait]
impl MoqRevocationChecker for ToggleRevocation {
    async fn check(
        &self,
        _peer: &MoqPeerIdentity,
        _action: MoqAction,
        _resource: &MoqResource,
        _binding: &MoqTokenBinding,
        _now: DateTime<Utc>,
    ) -> Result<MoqRevocationStatus, MoqRevocationError> {
        Ok(if self.revoked.load(Ordering::SeqCst) {
            MoqRevocationStatus::Revoked
        } else {
            MoqRevocationStatus::Active
        })
    }
}

struct RequestFixture {
    session_id: AdmissionSessionId,
    peer_identity: PeerIdentity,
    target: SessionTarget,
    authorization: Option<SetupAuthorization>,
}

impl RequestFixture {
    fn new(session_id: &str, target: &str, token: Option<&str>) -> Self {
        Self {
            session_id: AdmissionSessionId::new(session_id).expect("valid session ID"),
            peer_identity: PeerIdentity::Anonymous,
            target: SessionTarget::parse(target).expect("valid session target"),
            authorization: token.map(|token| {
                SetupAuthorization::new(token.as_bytes()).expect("valid test authorization")
            }),
        }
    }

    fn request(&self, substrate: Transport) -> AdmissionRequest<'_> {
        AdmissionRequest {
            session_id: &self.session_id,
            peer_identity: &self.peer_identity,
            target: &self.target,
            substrate,
            negotiated_protocol: MOQT_NEGOTIATED_PROTOCOL,
            setup_authorization: self.authorization.as_ref(),
        }
    }

    fn with_authorization(mut self, authorization: SetupAuthorization) -> Self {
        self.authorization = Some(authorization);
        self
    }
}

struct Harness {
    policy: RvoipMoqRelayAdmission,
    revocation: Arc<ToggleRevocation>,
    leases: Arc<BoundedMemoryMoqSessionLeaseStore>,
}

fn principal(
    tenant: &str,
    scopes: &[&str],
    expires_at: Option<DateTime<Utc>>,
) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "listener".to_owned(),
        tenant: Some(tenant.to_owned()),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        issuer: Some("https://issuer.test".to_owned()),
        expires_at,
        method: AuthenticationMethod::Jwt,
        assurance: IdentityAssurance::Anonymous,
    }
}

fn harness(
    principal: AuthenticatedPrincipal,
    include_token_id: bool,
    validator_delay: StdDuration,
    tenant_limit: usize,
    operation_timeout: StdDuration,
) -> Harness {
    harness_for_substrate(
        principal,
        include_token_id,
        validator_delay,
        tenant_limit,
        operation_timeout,
        MoqRelayAdmissionSubstrate::WebTransport,
    )
}

fn harness_for_substrate(
    principal: AuthenticatedPrincipal,
    include_token_id: bool,
    validator_delay: StdDuration,
    tenant_limit: usize,
    operation_timeout: StdDuration,
    substrate: MoqRelayAdmissionSubstrate,
) -> Harness {
    let revocation = Arc::new(ToggleRevocation::default());
    let replay = Arc::new(BoundedMemoryMoqReplayStore::new(64).expect("valid replay capacity"));
    let authorizer = Arc::new(SecureMoqAuthorizer::new(replay.clone(), revocation.clone()));
    let leases = Arc::new(
        BoundedMemoryMoqSessionLeaseStore::new(
            MoqSessionLeaseLimits::new(tenant_limit, tenant_limit).expect("valid limits"),
        )
        .expect("valid store"),
    );
    let validator = Arc::new(TemplateBearerValidator {
        principal,
        include_token_id,
        delay: validator_delay,
    });
    let policy = RvoipMoqRelayAdmission::with_config(
        validator,
        authorizer,
        leases.clone(),
        MoqRelayAdmissionConfig::for_substrate(operation_timeout, substrate)
            .expect("valid timeout and substrate"),
    )
    .expect("valid policy");
    Harness {
        policy,
        revocation,
        leases,
    }
}

#[tokio::test]
async fn raw_quic_subscriber_uses_the_same_secure_lifecycle_and_rejects_webtransport() {
    let expiry = Utc::now() + Duration::minutes(2);
    let harness = harness_for_substrate(
        principal(
            "tenant-a",
            &["broadcast:subscribe:broadcast-a"],
            Some(expiry),
        ),
        true,
        StdDuration::ZERO,
        4,
        StdDuration::from_secs(1),
        MoqRelayAdmissionSubstrate::RawQuic,
    );
    assert_eq!(
        harness.policy.config().subscriber_substrate,
        MoqRelayAdmissionSubstrate::RawQuic
    );

    let fixture = RequestFixture::new(
        "raw-session",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("raw-token"),
    );
    assert_eq!(
        require_denied(&harness.policy, &fixture, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );

    let mut admitted = harness
        .policy
        .admit_session(fixture.request(Transport::RawQuic))
        .await
        .expect("raw-QUIC token subscriber must be admitted");
    assert!(admitted.decision().claims.subscribe);
    assert!(!admitted.decision().claims.publish);
    assert_eq!(
        admitted.decision().claims.scope.as_deref(),
        Some("broadcast:subscribe:broadcast-a")
    );
    admitted
        .revalidate(unix_now())
        .await
        .expect("active raw-QUIC lease must revalidate");
    harness.revocation.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        admitted.revalidate(unix_now()).await,
        Err(AdmissionError::PolicyDenied)
    );
    harness.revocation.revoked.store(false, Ordering::SeqCst);
    let close = AdmissionCloseContext {
        reason: AdmissionCloseReason::PeerClosed,
        ended_at_unix_seconds: unix_now(),
    };
    admitted
        .close(close)
        .await
        .expect("first raw-QUIC close must succeed");
    admitted
        .close(close)
        .await
        .expect("raw-QUIC close retry must be idempotent");
    assert_eq!(
        harness
            .leases
            .snapshot(Utc::now())
            .await
            .unwrap()
            .active_sessions,
        0
    );

    let escalated = harness_for_substrate(
        principal(
            "tenant-a",
            &["broadcast:subscribe:broadcast-a", "broadcast:publish"],
            Some(expiry),
        ),
        true,
        StdDuration::ZERO,
        4,
        StdDuration::from_secs(1),
        MoqRelayAdmissionSubstrate::RawQuic,
    );
    let escalated_fixture = RequestFixture::new(
        "raw-escalated",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("raw-escalated-token"),
    );
    assert_eq!(
        require_denied(&escalated.policy, &escalated_fixture, Transport::RawQuic).await,
        AdmissionError::PolicyDenied
    );
}

async fn require_denied(
    policy: &RvoipMoqRelayAdmission,
    fixture: &RequestFixture,
    substrate: Transport,
) -> AdmissionError {
    match policy.admit_session(fixture.request(substrate)).await {
        Ok(_) => panic!("request unexpectedly admitted"),
        Err(error) => error,
    }
}

fn unix_now() -> u64 {
    u64::try_from(Utc::now().timestamp()).expect("test time follows Unix epoch")
}

#[test]
fn admission_configuration_defaults_to_webtransport_and_rejects_zero_timeout() {
    assert_eq!(
        MoqRelayAdmissionConfig::default().subscriber_substrate,
        MoqRelayAdmissionSubstrate::WebTransport
    );
    assert_eq!(
        MoqRelayAdmissionConfig::new(StdDuration::from_secs(1))
            .unwrap()
            .subscriber_substrate,
        MoqRelayAdmissionSubstrate::WebTransport
    );
    assert!(MoqRelayAdmissionConfig::for_substrate(
        StdDuration::ZERO,
        MoqRelayAdmissionSubstrate::RawQuic
    )
    .is_err());
}

#[tokio::test]
async fn valid_subscriber_revalidates_and_closes_idempotently() {
    let expiry = Utc::now() + Duration::minutes(2);
    let harness = harness(
        principal(
            "tenant-a",
            &["broadcast:subscribe:broadcast-a"],
            Some(expiry),
        ),
        true,
        StdDuration::ZERO,
        4,
        StdDuration::from_secs(1),
    );
    let fixture = RequestFixture::new(
        "session-a",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("token-a"),
    );
    let mut admitted = harness
        .policy
        .admit_session(fixture.request(Transport::WebTransport))
        .await
        .expect("valid subscriber must be admitted");
    assert!(admitted.decision().claims.subscribe);
    assert!(!admitted.decision().claims.publish);
    assert_eq!(
        admitted.decision().claims.scope.as_deref(),
        Some("broadcast:subscribe:broadcast-a")
    );
    admitted
        .revalidate(unix_now())
        .await
        .expect("active lease must revalidate");
    harness.revocation.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        admitted.revalidate(unix_now()).await,
        Err(AdmissionError::PolicyDenied)
    );
    harness.revocation.revoked.store(false, Ordering::SeqCst);
    let close = AdmissionCloseContext {
        reason: AdmissionCloseReason::PeerClosed,
        ended_at_unix_seconds: unix_now(),
    };
    admitted
        .close(close)
        .await
        .expect("first close must succeed");
    admitted
        .close(close)
        .await
        .expect("close retry must succeed");
    assert_eq!(
        harness
            .leases
            .snapshot(Utc::now())
            .await
            .unwrap()
            .active_sessions,
        0
    );
}

#[tokio::test]
async fn transport_target_tenant_and_exact_receive_only_scope_are_enforced() {
    let expiry = Utc::now() + Duration::minutes(2);
    let valid_principal = || {
        principal(
            "tenant-a",
            &["broadcast:subscribe:broadcast-a"],
            Some(expiry),
        )
    };

    for (index, (candidate, target, substrate)) in [
        (
            valid_principal(),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::RawQuic,
        ),
        (
            valid_principal(),
            "moqt://relay.test/tenant-a/broadcast-a?credential=x",
            Transport::WebTransport,
        ),
        (
            valid_principal(),
            "moqt://relay.test/tenant-a/broadcast-a#track:audio",
            Transport::WebTransport,
        ),
        (
            principal(
                "tenant-b",
                &["broadcast:subscribe:broadcast-a"],
                Some(expiry),
            ),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
        (
            principal(
                "tenant-a",
                &["broadcast:subscribe:broadcast-b"],
                Some(expiry),
            ),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
        (
            principal("tenant-a", &["*"], Some(expiry)),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
        (
            principal("tenant-a", &["broadcast:publish"], Some(expiry)),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
        (
            principal("tenant-a", &["broadcast:relay"], Some(expiry)),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
        (
            principal(
                "tenant-a",
                &["broadcast:subscribe:broadcast-a", "broadcast:publish"],
                Some(expiry),
            ),
            "moqt://relay.test/tenant-a/broadcast-a",
            Transport::WebTransport,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let harness = harness(
            candidate,
            true,
            StdDuration::ZERO,
            4,
            StdDuration::from_secs(1),
        );
        let fixture = RequestFixture::new(
            &format!("denied-{index}"),
            target,
            Some(&format!("token-{index}")),
        );
        assert_eq!(
            require_denied(&harness.policy, &fixture, substrate).await,
            AdmissionError::PolicyDenied
        );
        assert_eq!(
            harness
                .leases
                .snapshot(Utc::now())
                .await
                .unwrap()
                .active_sessions,
            0
        );
    }

    let harness = harness(
        valid_principal(),
        true,
        StdDuration::ZERO,
        4,
        StdDuration::from_secs(1),
    );
    let missing = RequestFixture::new(
        "missing-setup",
        "moqt://relay.test/tenant-a/broadcast-a",
        None,
    );
    assert_eq!(
        require_denied(&harness.policy, &missing, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
    let wrong_protocol = RequestFixture::new(
        "wrong-protocol",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("wrong-protocol-token"),
    );
    let request = AdmissionRequest {
        negotiated_protocol: "moqt-18",
        ..wrong_protocol.request(Transport::WebTransport)
    };
    assert!(matches!(
        harness.policy.admit_session(request).await,
        Err(AdmissionError::PolicyDenied)
    ));

    let unsupported_token_type = RequestFixture::new(
        "unsupported-token-type",
        "moqt://relay.test/tenant-a/broadcast-a",
        None,
    )
    .with_authorization(
        SetupAuthorization::new_typed(7, b"provider-specific-token")
            .expect("bounded typed authorization"),
    );
    assert_eq!(
        require_denied(
            &harness.policy,
            &unsupported_token_type,
            Transport::WebTransport,
        )
        .await,
        AdmissionError::PolicyDenied
    );
}

#[tokio::test]
async fn missing_lifecycle_metadata_revocation_and_validation_timeout_fail_closed() {
    let expiry = Utc::now() + Duration::minutes(2);
    let exact = &["broadcast:subscribe:broadcast-a"];
    let cases = [
        (
            principal("tenant-a", exact, Some(expiry)),
            false,
            StdDuration::ZERO,
        ),
        (principal("tenant-a", exact, None), true, StdDuration::ZERO),
        (
            principal("tenant-a", exact, Some(expiry)),
            true,
            StdDuration::from_millis(75),
        ),
    ];
    for (index, (principal, include_token_id, delay)) in cases.into_iter().enumerate() {
        let harness = harness(
            principal,
            include_token_id,
            delay,
            4,
            StdDuration::from_millis(10),
        );
        let fixture = RequestFixture::new(
            &format!("metadata-{index}"),
            "moqt://relay.test/tenant-a/broadcast-a",
            Some("token-metadata"),
        );
        assert_eq!(
            require_denied(&harness.policy, &fixture, Transport::WebTransport).await,
            AdmissionError::PolicyDenied
        );
        assert_eq!(
            harness
                .leases
                .snapshot(Utc::now())
                .await
                .unwrap()
                .active_sessions,
            0
        );
    }

    let revoked = harness(
        principal("tenant-a", exact, Some(expiry)),
        true,
        StdDuration::ZERO,
        4,
        StdDuration::from_secs(1),
    );
    revoked.revocation.revoked.store(true, Ordering::SeqCst);
    let fixture = RequestFixture::new(
        "revoked",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("token-revoked"),
    );
    assert_eq!(
        require_denied(&revoked.policy, &fixture, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
}

#[tokio::test]
async fn replay_and_quota_failure_are_tombstoned() {
    let expiry = Utc::now() + Duration::minutes(2);
    let harness = harness(
        principal(
            "tenant-a",
            &["broadcast:subscribe:broadcast-a"],
            Some(expiry),
        ),
        true,
        StdDuration::ZERO,
        1,
        StdDuration::from_secs(1),
    );
    let first = RequestFixture::new(
        "session-one",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("token-a"),
    );
    let mut admitted = harness
        .policy
        .admit_session(first.request(Transport::WebTransport))
        .await
        .expect("first lease must acquire");
    let replay = RequestFixture::new(
        "session-replay",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("token-a"),
    );
    assert_eq!(
        require_denied(&harness.policy, &replay, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
    let quota = RequestFixture::new(
        "session-quota",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("different-token"),
    );
    assert_eq!(
        require_denied(&harness.policy, &quota, Transport::WebTransport).await,
        AdmissionError::CapacityExhausted
    );
    assert_eq!(
        harness
            .leases
            .snapshot(Utc::now())
            .await
            .unwrap()
            .active_sessions,
        1
    );
    admitted
        .close(AdmissionCloseContext {
            reason: AdmissionCloseReason::LocalClosed,
            ended_at_unix_seconds: unix_now(),
        })
        .await
        .expect("active session must close");
    assert_eq!(
        require_denied(&harness.policy, &quota, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
}

struct DelayedAuthorizer {
    inner: Arc<dyn MoqAuthorizer>,
    delay_after_authorize: StdDuration,
}

#[async_trait]
impl MoqAuthorizer for DelayedAuthorizer {
    async fn authorize(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<MoqAuthorizationGrant, MoqAuthorizationError> {
        let result = self.inner.authorize(principal, request, now).await;
        tokio::time::sleep(self.delay_after_authorize).await;
        result
    }

    async fn recheck(
        &self,
        principal: &AuthenticatedPrincipal,
        grant: &MoqAuthorizationGrant,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        self.inner.recheck(principal, grant, now).await
    }

    async fn close_session(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        self.inner.close_session(principal, request, now).await
    }
}

struct DelayedLeaseStore {
    inner: Arc<BoundedMemoryMoqSessionLeaseStore>,
    delay_after_acquire: StdDuration,
}

#[async_trait]
impl MoqSessionLeaseStore for DelayedLeaseStore {
    async fn acquire(
        &self,
        binding: &MoqSessionLeaseBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLease, MoqSessionLeaseError> {
        let result = self.inner.acquire(binding, now).await;
        tokio::time::sleep(self.delay_after_acquire).await;
        result
    }

    async fn verify(
        &self,
        lease: &MoqSessionLease,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        self.inner.verify(lease, now).await
    }

    async fn close(
        &self,
        lease: &MoqSessionLease,
        close: MoqSessionLeaseClose,
        now: DateTime<Utc>,
    ) -> Result<(), MoqSessionLeaseError> {
        self.inner.close(lease, close, now).await
    }

    async fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLeaseSnapshot, MoqSessionLeaseError> {
        self.inner.snapshot(now).await
    }
}

#[tokio::test]
async fn response_timeouts_detach_then_compensate_authorizer_and_quota_mutations() {
    let expiry = Utc::now() + Duration::minutes(2);
    let principal = principal(
        "tenant-a",
        &["broadcast:subscribe:broadcast-a"],
        Some(expiry),
    );
    let validator = Arc::new(TemplateBearerValidator {
        principal: principal.clone(),
        include_token_id: true,
        delay: StdDuration::ZERO,
    });
    let revocation = Arc::new(ToggleRevocation::default());
    let replay = Arc::new(BoundedMemoryMoqReplayStore::new(16).unwrap());
    let secure = Arc::new(SecureMoqAuthorizer::new(replay.clone(), revocation.clone()));
    let memory = Arc::new(
        BoundedMemoryMoqSessionLeaseStore::new(MoqSessionLeaseLimits::new(4, 4).unwrap()).unwrap(),
    );
    let delayed_store = Arc::new(DelayedLeaseStore {
        inner: memory.clone(),
        delay_after_acquire: StdDuration::from_millis(75),
    });
    let store_timeout = RvoipMoqRelayAdmission::with_config(
        validator.clone(),
        secure.clone(),
        delayed_store,
        MoqRelayAdmissionConfig::new(StdDuration::from_millis(10)).unwrap(),
    )
    .unwrap();
    let fixture = RequestFixture::new(
        "store-timeout",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("store-timeout-token"),
    );
    assert_eq!(
        require_denied(&store_timeout, &fixture, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
    tokio::time::sleep(StdDuration::from_millis(100)).await;
    let snapshot = memory.snapshot(Utc::now()).await.unwrap();
    assert_eq!(snapshot.active_sessions, 0);
    assert_eq!(snapshot.retained_sessions, 1);

    let delayed_authorizer = Arc::new(DelayedAuthorizer {
        inner: secure,
        delay_after_authorize: StdDuration::from_millis(75),
    });
    let authorizer_memory = Arc::new(
        BoundedMemoryMoqSessionLeaseStore::new(MoqSessionLeaseLimits::new(4, 4).unwrap()).unwrap(),
    );
    let authorizer_timeout = RvoipMoqRelayAdmission::with_config(
        validator,
        delayed_authorizer,
        authorizer_memory.clone(),
        MoqRelayAdmissionConfig::new(StdDuration::from_millis(10)).unwrap(),
    )
    .unwrap();
    let fixture = RequestFixture::new(
        "authorizer-timeout",
        "moqt://relay.test/tenant-a/broadcast-a",
        Some("authorizer-timeout-token"),
    );
    assert_eq!(
        require_denied(&authorizer_timeout, &fixture, Transport::WebTransport).await,
        AdmissionError::PolicyDenied
    );
    tokio::time::sleep(StdDuration::from_millis(100)).await;
    assert_eq!(
        authorizer_memory
            .snapshot(Utc::now())
            .await
            .unwrap()
            .active_sessions,
        0
    );
    assert_eq!(replay.retained_claims(Utc::now()).await, 2);
}
