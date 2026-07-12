//! Production rvoip authentication and durable-lease adapter for moq-rs.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moq_relay_ietf::{
    AdmissionClaims, AdmissionCloseContext, AdmissionCloseError, AdmissionCloseReason,
    AdmissionDecision, AdmissionError, AdmissionLease, AdmissionPrincipal, AdmissionRequest,
    AdmittedSession, AuthenticationMethod as RelayAuthenticationMethod, SessionAdmission,
};
use moq_transport::session::Transport;
use rvoip_auth_core::{BearerValidator, ValidatedBearer};
use rvoip_core_traits::AuthenticationMethod;
use sha2::{Digest, Sha256};

use crate::{
    MoqAction, MoqAuthorizationGrant, MoqAuthorizationRequest, MoqAuthorizer, MoqNamespace,
    MoqResource, MoqSessionId, MoqSessionLease, MoqSessionLeaseBinding, MoqSessionLeaseClose,
    MoqSessionLeaseError, MoqSessionLeaseStore, MoqTokenBinding, MOQT_NEGOTIATED_PROTOCOL,
};

const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Public-listener transport accepted by an rvoip MOQT subscriber admission
/// policy.
///
/// A policy accepts exactly one substrate. Run separate listeners when native
/// raw-QUIC clients and browser WebTransport clients must both be supported.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MoqRelayAdmissionSubstrate {
    /// Browser-compatible WebTransport listener.
    #[default]
    WebTransport,
    /// Native draft-19 MOQT-over-QUIC listener.
    RawQuic,
}

impl MoqRelayAdmissionSubstrate {
    fn accepts(self, substrate: Transport) -> bool {
        matches!(
            (self, substrate),
            (Self::WebTransport, Transport::WebTransport) | (Self::RawQuic, Transport::RawQuic)
        )
    }
}

/// Internal I/O bound for validation, revocation, and durable lease actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqRelayAdmissionConfig {
    /// Maximum duration of one external validation or lease-store operation.
    pub operation_timeout: Duration,
    /// Exact public-listener transport accepted by this policy.
    pub subscriber_substrate: MoqRelayAdmissionSubstrate,
}

impl MoqRelayAdmissionConfig {
    /// Creates a WebTransport subscriber policy configuration.
    pub fn new(operation_timeout: Duration) -> Result<Self, MoqSessionLeaseError> {
        Self::for_substrate(operation_timeout, MoqRelayAdmissionSubstrate::WebTransport)
    }

    /// Creates a subscriber policy configuration for one exact substrate.
    pub fn for_substrate(
        operation_timeout: Duration,
        subscriber_substrate: MoqRelayAdmissionSubstrate,
    ) -> Result<Self, MoqSessionLeaseError> {
        if operation_timeout.is_zero() {
            return Err(MoqSessionLeaseError::InvalidConfig(
                "relay admission operation timeout must be greater than zero",
            ));
        }
        Ok(Self {
            operation_timeout,
            subscriber_substrate,
        })
    }
}

impl Default for MoqRelayAdmissionConfig {
    fn default() -> Self {
        Self {
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            subscriber_substrate: MoqRelayAdmissionSubstrate::WebTransport,
        }
    }
}

/// moq-rs admission policy backed by rvoip bearer validation, revocation, and
/// an atomic [`MoqSessionLeaseStore`].
pub struct RvoipMoqRelayAdmission {
    validator: Arc<dyn BearerValidator>,
    authorizer: Arc<dyn MoqAuthorizer>,
    leases: Arc<dyn MoqSessionLeaseStore>,
    config: MoqRelayAdmissionConfig,
}

impl std::fmt::Debug for RvoipMoqRelayAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RvoipMoqRelayAdmission")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RvoipMoqRelayAdmission {
    pub fn new(
        validator: Arc<dyn BearerValidator>,
        authorizer: Arc<dyn MoqAuthorizer>,
        leases: Arc<dyn MoqSessionLeaseStore>,
    ) -> Self {
        Self {
            validator,
            authorizer,
            leases,
            config: MoqRelayAdmissionConfig::default(),
        }
    }

    pub fn with_config(
        validator: Arc<dyn BearerValidator>,
        authorizer: Arc<dyn MoqAuthorizer>,
        leases: Arc<dyn MoqSessionLeaseStore>,
        config: MoqRelayAdmissionConfig,
    ) -> Result<Self, MoqSessionLeaseError> {
        let config = MoqRelayAdmissionConfig::for_substrate(
            config.operation_timeout,
            config.subscriber_substrate,
        )?;
        Ok(Self {
            validator,
            authorizer,
            leases,
            config,
        })
    }

    pub const fn config(&self) -> MoqRelayAdmissionConfig {
        self.config
    }

    async fn prepare(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<PreparedAdmission, AdmissionError> {
        if request.negotiated_protocol != MOQT_NEGOTIATED_PROTOCOL
            || !self.config.subscriber_substrate.accepts(request.substrate)
            || request.peer_identity.presented_certificate().is_some()
        {
            return Err(AdmissionError::PolicyDenied);
        }
        let authorization = request
            .setup_authorization
            .ok_or(AdmissionError::PolicyDenied)?;
        // Bridgefu 1.0 uses the draft-19 out-of-band bearer token type. Other
        // token types require a dedicated validator and must never be silently
        // interpreted as UTF-8 bearer credentials.
        if authorization.token_type() != 0 || authorization.is_empty() {
            return Err(AdmissionError::PolicyDenied);
        }
        let token = std::str::from_utf8(authorization.as_bytes())
            .map_err(|_| AdmissionError::PolicyDenied)?;
        let fingerprint: [u8; 32] = Sha256::digest(authorization.as_bytes()).into();
        // Validation precedes all state mutation, so cancelling this bounded
        // operation cannot strand replay or quota state.
        let validated = tokio::time::timeout(
            self.config.operation_timeout,
            self.validator.validate_credential(token),
        )
        .await
        .map_err(|_| AdmissionError::PolicyDenied)?
        .map_err(|_| AdmissionError::PolicyDenied)?;
        let now = Utc::now();
        let namespace = namespace_from_request(&request)?;
        let prepared = prepare_validated(
            request.session_id.as_str(),
            namespace,
            fingerprint,
            validated,
            now,
        )?;
        // The existing rvoip authorizer remains authoritative for resource
        // scopes, revocation, and its replay policy. The session lease is a
        // second durable boundary for exact binding and tenant quota. If that
        // second step fails, close_session retains the authorizer tombstone.
        let authorization_grant = match self
            .authorize_bounded(&prepared.principal, &prepared.authorization_request, now)
            .await
        {
            Ok(grant) => grant,
            Err(_) => {
                self.close_authorization_bounded(
                    &prepared.principal,
                    &prepared.authorization_request,
                    now,
                )
                .await;
                return Err(AdmissionError::PolicyDenied);
            }
        };
        let provisional_lease = MoqSessionLease::from_binding(prepared.binding.clone());
        let lease = match self.acquire_bounded(&prepared.binding, now).await {
            Ok(lease) => lease,
            Err(error) => {
                self.compensate_bounded(
                    &prepared.principal,
                    &prepared.authorization_request,
                    &provisional_lease,
                    now,
                )
                .await;
                return Err(match error {
                    BoundedOperationError::Operation(error) => map_acquire_error(error),
                    BoundedOperationError::TimeoutOrTaskFailure => AdmissionError::PolicyDenied,
                });
            }
        };
        Ok(PreparedAdmission {
            decision: prepared.decision,
            principal: prepared.principal,
            authorization_request: prepared.authorization_request,
            authorization_grant,
            lease,
        })
    }

    async fn authorize_bounded(
        &self,
        principal: &rvoip_core_traits::AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<MoqAuthorizationGrant, BoundedOperationError<crate::MoqAuthorizationError>> {
        let authorizer = self.authorizer.clone();
        let principal = principal.clone();
        let request = request.clone();
        let mut task =
            tokio::spawn(async move { authorizer.authorize(&principal, &request, now).await });
        match tokio::time::timeout(self.config.operation_timeout, &mut task).await {
            Ok(Ok(result)) => result.map_err(BoundedOperationError::Operation),
            Ok(Err(_)) | Err(_) => Err(BoundedOperationError::TimeoutOrTaskFailure),
        }
    }

    async fn acquire_bounded(
        &self,
        binding: &MoqSessionLeaseBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqSessionLease, BoundedOperationError<MoqSessionLeaseError>> {
        let store = self.leases.clone();
        let binding = binding.clone();
        let mut task = tokio::spawn(async move { store.acquire(&binding, now).await });
        match tokio::time::timeout(self.config.operation_timeout, &mut task).await {
            Ok(Ok(result)) => result.map_err(BoundedOperationError::Operation),
            Ok(Err(_)) | Err(_) => Err(BoundedOperationError::TimeoutOrTaskFailure),
        }
    }

    async fn close_authorization_bounded(
        &self,
        principal: &rvoip_core_traits::AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) {
        let authorizer = self.authorizer.clone();
        let principal = principal.clone();
        let request = request.clone();
        let mut task =
            tokio::spawn(async move { authorizer.close_session(&principal, &request, now).await });
        // A timed-out JoinHandle is detached, not cancelled.
        let _ = tokio::time::timeout(self.config.operation_timeout, &mut task).await;
    }

    async fn compensate_bounded(
        &self,
        principal: &rvoip_core_traits::AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        lease: &MoqSessionLease,
        now: DateTime<Utc>,
    ) {
        let authorizer = self.authorizer.clone();
        let principal = principal.clone();
        let request = request.clone();
        let mut authorization =
            tokio::spawn(async move { authorizer.close_session(&principal, &request, now).await });
        let store = self.leases.clone();
        let lease = lease.clone();
        let mut quota = tokio::spawn(async move {
            store
                .close(&lease, MoqSessionLeaseClose::ActivationFailed, now)
                .await
        });
        let _ = tokio::join!(
            tokio::time::timeout(self.config.operation_timeout, &mut authorization),
            tokio::time::timeout(self.config.operation_timeout, &mut quota)
        );
    }
}

#[async_trait]
impl SessionAdmission for RvoipMoqRelayAdmission {
    async fn admit(
        &self,
        _request: AdmissionRequest<'_>,
    ) -> Result<AdmissionDecision, AdmissionError> {
        // Production token admission is intentionally available only through
        // the atomic decision-plus-lease path below.
        Err(AdmissionError::PolicyDenied)
    }

    async fn admit_session(
        &self,
        request: AdmissionRequest<'_>,
    ) -> Result<AdmittedSession, AdmissionError> {
        // Do not wrap this transaction in a cancelling outer timeout. Once a
        // replay claim can exist, each mutation runs in an owned bounded task
        // and prepare performs close-before-acquire-safe compensation.
        let prepared = self.prepare(request).await?;
        let lease = RvoipMoqAdmissionLease {
            lease: prepared.lease,
            store: self.leases.clone(),
            authorizer: self.authorizer.clone(),
            principal: prepared.principal,
            authorization_request: prepared.authorization_request,
            authorization_grant: prepared.authorization_grant,
            operation_timeout: self.config.operation_timeout,
        };
        Ok(AdmittedSession::new(prepared.decision, Box::new(lease)))
    }

    fn supports_production_token_leases(&self) -> bool {
        true
    }

    fn supports_bounded_session_leases(&self) -> bool {
        true
    }

    fn supports_atomic_token_admission(&self) -> bool {
        true
    }

    fn supports_awaited_session_close(&self) -> bool {
        true
    }
}

struct PreparedAdmission {
    decision: AdmissionDecision,
    principal: rvoip_core_traits::AuthenticatedPrincipal,
    authorization_request: MoqAuthorizationRequest,
    authorization_grant: MoqAuthorizationGrant,
    lease: MoqSessionLease,
}

enum BoundedOperationError<E> {
    Operation(E),
    TimeoutOrTaskFailure,
}

struct RvoipMoqAdmissionLease {
    lease: MoqSessionLease,
    store: Arc<dyn MoqSessionLeaseStore>,
    authorizer: Arc<dyn MoqAuthorizer>,
    principal: rvoip_core_traits::AuthenticatedPrincipal,
    authorization_request: MoqAuthorizationRequest,
    authorization_grant: MoqAuthorizationGrant,
    operation_timeout: Duration,
}

#[async_trait]
impl AdmissionLease for RvoipMoqAdmissionLease {
    async fn revalidate(&self, now_unix_seconds: u64) -> Result<(), AdmissionError> {
        let now =
            datetime_from_unix_seconds(now_unix_seconds).ok_or(AdmissionError::PolicyDenied)?;
        if self.lease.binding().expires_at() <= now {
            return Err(AdmissionError::PolicyDenied);
        }
        let authorizer = self.authorizer.clone();
        let principal = self.principal.clone();
        let grant = self.authorization_grant.clone();
        let mut authorization =
            tokio::spawn(async move { authorizer.recheck(&principal, &grant, now).await });
        match tokio::time::timeout(self.operation_timeout, &mut authorization).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => return Err(AdmissionError::PolicyDenied),
        }

        let store = self.store.clone();
        let lease = self.lease.clone();
        let mut verification = tokio::spawn(async move { store.verify(&lease, now).await });
        match tokio::time::timeout(self.operation_timeout, &mut verification).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => Err(AdmissionError::PolicyDenied),
        }
    }

    async fn close(&mut self, context: AdmissionCloseContext) -> Result<(), AdmissionCloseError> {
        let now = datetime_from_unix_seconds(context.ended_at_unix_seconds)
            .ok_or(AdmissionCloseError::InvalidState)?;
        // Both finalizers run in owned tasks. A timeout detaches rather than
        // cancels cleanup, preserving fail-closed tombstones.
        let authorizer = self.authorizer.clone();
        let principal = self.principal.clone();
        let request = self.authorization_request.clone();
        let mut authorization =
            tokio::spawn(async move { authorizer.close_session(&principal, &request, now).await });
        let store = self.store.clone();
        let lease = self.lease.clone();
        let reason = map_close_reason(context.reason);
        let mut quota = tokio::spawn(async move { store.close(&lease, reason, now).await });
        let (authorization, quota) = tokio::join!(
            tokio::time::timeout(self.operation_timeout, &mut authorization),
            tokio::time::timeout(self.operation_timeout, &mut quota)
        );
        let authorization = match authorization {
            Ok(Ok(result)) => result.map_err(map_authorization_close_error),
            Ok(Err(_)) | Err(_) => Err(AdmissionCloseError::ReplayFinalizeUnavailable),
        };
        let quota = match quota {
            Ok(Ok(result)) => result.map_err(map_close_error),
            Ok(Err(_)) | Err(_) => Err(AdmissionCloseError::LeaseReleaseUnavailable),
        };
        authorization?;
        quota
    }
}

struct ValidatedPreparation {
    decision: AdmissionDecision,
    binding: MoqSessionLeaseBinding,
    principal: rvoip_core_traits::AuthenticatedPrincipal,
    authorization_request: MoqAuthorizationRequest,
}

fn prepare_validated(
    relay_session_id: &str,
    namespace: MoqNamespace,
    credential_fingerprint_sha256: [u8; 32],
    validated: ValidatedBearer,
    now: DateTime<Utc>,
) -> Result<ValidatedPreparation, AdmissionError> {
    let principal = validated.principal;
    if principal.method == AuthenticationMethod::Anonymous || principal.is_expired_at(now) {
        return Err(AdmissionError::PolicyDenied);
    }
    let expires_at = principal.expires_at.ok_or(AdmissionError::PolicyDenied)?;
    let token_id = validated.token_id.ok_or(AdmissionError::PolicyDenied)?;
    // This public token-subscriber policy is receive-only and bound to one
    // exact broadcast. Wildcard, publisher, and relay scopes are deliberately
    // not substitutes; those ingress roles use a separate mTLS policy.
    let canonical_scope = exact_subscriber_scope(&principal, &namespace)?;
    let session_id =
        MoqSessionId::new(relay_session_id).map_err(|_| AdmissionError::PolicyDenied)?;
    let binding = MoqSessionLeaseBinding::new(
        session_id.clone(),
        principal.ownership_key(),
        token_id.clone(),
        credential_fingerprint_sha256,
        namespace.clone(),
        canonical_scope.clone(),
        expires_at,
    )
    .map_err(|_| AdmissionError::PolicyDenied)?;
    let expires_at_unix_seconds =
        u64::try_from(expires_at.timestamp()).map_err(|_| AdmissionError::PolicyDenied)?;
    let decision = AdmissionDecision::new(
        AdmissionPrincipal::new(
            principal.subject.clone(),
            RelayAuthenticationMethod::SetupToken,
        )
        .map_err(|_| AdmissionError::PolicyDenied)?,
        AdmissionClaims {
            scope: Some(canonical_scope),
            publish: false,
            subscribe: true,
            expires_at_unix_seconds: Some(expires_at_unix_seconds),
            token_id: Some(token_id.clone()),
        },
    )
    .map_err(|_| AdmissionError::PolicyDenied)?;
    let authorization_request = MoqAuthorizationRequest::new(
        MoqAction::EstablishSession,
        MoqResource::broadcast(namespace),
        MoqTokenBinding::from_sha256(session_id, credential_fingerprint_sha256)
            .map_err(|_| AdmissionError::PolicyDenied)?,
        expires_at,
    );
    Ok(ValidatedPreparation {
        decision,
        binding,
        principal,
        authorization_request,
    })
}

fn namespace_from_request(request: &AdmissionRequest<'_>) -> Result<MoqNamespace, AdmissionError> {
    if request.target.query().is_some() || request.target.fragment().is_some() {
        return Err(AdmissionError::PolicyDenied);
    }
    let path = request
        .target
        .path()
        .strip_prefix('/')
        .ok_or(AdmissionError::PolicyDenied)?;
    MoqNamespace::parse(path).map_err(|_| AdmissionError::PolicyDenied)
}

fn exact_subscriber_scope(
    principal: &rvoip_core_traits::AuthenticatedPrincipal,
    namespace: &MoqNamespace,
) -> Result<String, AdmissionError> {
    let subscribe = format!("broadcast:subscribe:{}", namespace.broadcast_id());
    let has_escalated_scope = principal.scopes.iter().any(|scope| {
        matches!(
            scope.as_str(),
            "*" | "broadcast:publish" | "broadcast:relay"
        )
    });
    if !has_escalated_scope && principal.scopes.iter().any(|scope| scope == &subscribe) {
        Ok(subscribe)
    } else {
        Err(AdmissionError::PolicyDenied)
    }
}

fn datetime_from_unix_seconds(seconds: u64) -> Option<DateTime<Utc>> {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
}

fn map_acquire_error(error: MoqSessionLeaseError) -> AdmissionError {
    match error {
        MoqSessionLeaseError::CapacityExceeded | MoqSessionLeaseError::TenantQuotaExceeded => {
            AdmissionError::CapacityExhausted
        }
        _ => AdmissionError::PolicyDenied,
    }
}

fn map_close_reason(reason: AdmissionCloseReason) -> MoqSessionLeaseClose {
    match reason {
        AdmissionCloseReason::PeerClosed => MoqSessionLeaseClose::PeerClosed,
        AdmissionCloseReason::LocalClosed => MoqSessionLeaseClose::LocalClosed,
        AdmissionCloseReason::ActivationFailed => MoqSessionLeaseClose::ActivationFailed,
        AdmissionCloseReason::AdmissionRevalidationFailed => {
            MoqSessionLeaseClose::AdmissionRevalidationFailed
        }
        AdmissionCloseReason::ProtocolError => MoqSessionLeaseClose::ProtocolError,
        AdmissionCloseReason::RelayShutdown => MoqSessionLeaseClose::RelayShutdown,
    }
}

fn map_close_error(error: MoqSessionLeaseError) -> AdmissionCloseError {
    match error {
        MoqSessionLeaseError::OwnerMismatch
        | MoqSessionLeaseError::BindingMismatch
        | MoqSessionLeaseError::CrossSessionReplay => AdmissionCloseError::OwnershipMismatch,
        MoqSessionLeaseError::BackendUnavailable(_) => AdmissionCloseError::LeaseReleaseUnavailable,
        _ => AdmissionCloseError::InvalidState,
    }
}

fn map_authorization_close_error(error: crate::MoqAuthorizationError) -> AdmissionCloseError {
    match error {
        crate::MoqAuthorizationError::OwnerChanged => AdmissionCloseError::OwnershipMismatch,
        _ => AdmissionCloseError::ReplayFinalizeUnavailable,
    }
}
