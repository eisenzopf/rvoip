use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_core_traits::{AuthenticatedPrincipal, AuthenticationMethod, PrincipalOwnershipKey};
use serde::{Deserialize, Serialize};

use crate::{MoqSessionId, MoqTokenBinding, MoqTokenReplayStore};

const MAX_TRACK_NAME_BYTES: usize = 128;

/// Operation being authorized on an MOQT namespace or track.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoqAction {
    EstablishSession,
    PublishNamespace,
    PublishTrack,
    SubscribeNamespace,
    SubscribeTrack,
    FetchCatalog,
    Relay,
}

impl MoqAction {
    fn scope_requirement(self, broadcast_id: &str) -> ScopeRequirement {
        match self {
            Self::PublishNamespace | Self::PublishTrack => ScopeRequirement::Publish,
            Self::SubscribeNamespace | Self::SubscribeTrack | Self::FetchCatalog => {
                ScopeRequirement::Subscribe(format!("broadcast:subscribe:{broadcast_id}"))
            }
            Self::Relay => ScopeRequirement::Relay,
            Self::EstablishSession => {
                ScopeRequirement::Endpoint(format!("broadcast:subscribe:{broadcast_id}"))
            }
        }
    }
}

enum ScopeRequirement {
    Publish,
    Subscribe(String),
    Relay,
    Endpoint(String),
}

/// Validated authorization target within one tenant-owned broadcast.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MoqResource {
    namespace: crate::MoqNamespace,
    track: Option<String>,
}

impl MoqResource {
    pub fn broadcast(namespace: crate::MoqNamespace) -> Self {
        Self {
            namespace,
            track: None,
        }
    }

    pub fn track(
        namespace: crate::MoqNamespace,
        track: impl Into<String>,
    ) -> Result<Self, MoqAuthorizationError> {
        let track = track.into();
        validate_track_name(&track)?;
        Ok(Self {
            namespace,
            track: Some(track),
        })
    }

    pub fn namespace(&self) -> &crate::MoqNamespace {
        &self.namespace
    }

    pub fn track_name(&self) -> Option<&str> {
        self.track.as_deref()
    }
}

fn validate_track_name(track: &str) -> Result<(), MoqAuthorizationError> {
    if track.is_empty() {
        return Err(MoqAuthorizationError::InvalidResource(
            "track name is empty",
        ));
    }
    if track.len() > MAX_TRACK_NAME_BYTES {
        return Err(MoqAuthorizationError::InvalidResource(
            "track name exceeds the maximum encoded length",
        ));
    }
    if track.starts_with('/') || track.ends_with('/') || track.contains("//") {
        return Err(MoqAuthorizationError::InvalidResource(
            "track name contains an empty path component",
        ));
    }
    for component in track.split('/') {
        if matches!(component, "." | "..") {
            return Err(MoqAuthorizationError::InvalidResource(
                "track name contains a reserved path component",
            ));
        }
    }
    if !track.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/')
    }) {
        return Err(MoqAuthorizationError::InvalidResource(
            "track name contains a non-canonical character",
        ));
    }
    Ok(())
}

/// Safe, credential-free identity retained for an authenticated MOQT peer.
///
/// Only the ownership tuple and categorical authentication diagnostics are
/// copied from [`AuthenticatedPrincipal`]. The assurance payload, token,
/// certificate, key material, and scopes are deliberately not retained.
#[derive(Clone, Eq, PartialEq)]
pub struct MoqPeerIdentity {
    owner: PrincipalOwnershipKey,
    method: AuthenticationMethod,
    assurance_kind: String,
    authenticated_until: Option<DateTime<Utc>>,
}

impl MoqPeerIdentity {
    pub fn from_principal(principal: &AuthenticatedPrincipal) -> Self {
        Self {
            owner: principal.ownership_key(),
            method: principal.method,
            assurance_kind: principal.assurance.kind().into(),
            authenticated_until: principal.expires_at,
        }
    }

    pub fn owner(&self) -> &PrincipalOwnershipKey {
        &self.owner
    }

    pub fn subject(&self) -> &str {
        &self.owner.subject
    }

    pub fn tenant(&self) -> Option<&str> {
        self.owner.tenant.as_deref()
    }

    pub fn issuer(&self) -> Option<&str> {
        self.owner.issuer.as_deref()
    }

    pub const fn method(&self) -> AuthenticationMethod {
        self.method
    }

    pub fn assurance_kind(&self) -> &str {
        &self.assurance_kind
    }

    pub const fn authenticated_until(&self) -> Option<DateTime<Utc>> {
        self.authenticated_until
    }
}

impl std::fmt::Debug for MoqPeerIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqPeerIdentity")
            .field("owner", &self.owner)
            .field("method", &self.method)
            .field("assurance_kind", &self.assurance_kind)
            .field("authenticated_until", &self.authenticated_until)
            .finish()
    }
}

/// Authorization input produced only after transport authentication succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqAuthorizationRequest {
    action: MoqAction,
    resource: MoqResource,
    token_binding: MoqTokenBinding,
    expires_at: DateTime<Utc>,
}

impl MoqAuthorizationRequest {
    pub fn new(
        action: MoqAction,
        resource: MoqResource,
        token_binding: MoqTokenBinding,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            action,
            resource,
            token_binding,
            expires_at,
        }
    }

    pub const fn action(&self) -> MoqAction {
        self.action
    }

    pub fn resource(&self) -> &MoqResource {
        &self.resource
    }

    pub fn token_binding(&self) -> &MoqTokenBinding {
        &self.token_binding
    }

    pub fn session_id(&self) -> &MoqSessionId {
        self.token_binding.session_id()
    }

    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Session-scoped authorization lease returned to a MOQT transport adapter.
///
/// Network code must call [`MoqAuthorizer::recheck`] periodically and before
/// every resource-changing operation. A grant is not a permanent permission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqAuthorizationGrant {
    peer: MoqPeerIdentity,
    action: MoqAction,
    resource: MoqResource,
    token_binding: MoqTokenBinding,
    authorized_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl MoqAuthorizationGrant {
    pub fn peer(&self) -> &MoqPeerIdentity {
        &self.peer
    }

    pub const fn action(&self) -> MoqAction {
        self.action
    }

    pub fn resource(&self) -> &MoqResource {
        &self.resource
    }

    pub fn token_binding(&self) -> &MoqTokenBinding {
        &self.token_binding
    }

    pub fn session_id(&self) -> &MoqSessionId {
        self.token_binding.session_id()
    }

    pub const fn authorized_at(&self) -> DateTime<Utc> {
        self.authorized_at
    }

    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoqRevocationStatus {
    Active,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqRevocationError {
    #[error("MOQT revocation service unavailable: {0}")]
    Unavailable(String),
}

/// Application hook for token, peer, namespace, or broadcast revocation.
///
/// The binding contains only a SHA-256 fingerprint and its `Debug` output is
/// redacted. Implementations must not resolve or return a raw credential.
#[async_trait]
pub trait MoqRevocationChecker: Send + Sync {
    async fn check(
        &self,
        peer: &MoqPeerIdentity,
        action: MoqAction,
        resource: &MoqResource,
        binding: &MoqTokenBinding,
        now: DateTime<Utc>,
    ) -> Result<MoqRevocationStatus, MoqRevocationError>;
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqAuthorizationError {
    #[error("MOQT peer is not authenticated")]
    Unauthenticated,
    #[error("MOQT principal has expired")]
    PrincipalExpired,
    #[error("MOQT authorization request has expired")]
    RequestExpired,
    #[error("MOQT authorization grant has expired")]
    GrantExpired,
    #[error("MOQT principal is missing a tenant")]
    MissingTenant,
    #[error("MOQT principal tenant does not own the requested namespace")]
    TenantMismatch,
    #[error("MOQT principal is missing required scope {required}")]
    MissingScope { required: String },
    #[error("MOQT subscriber token is not bound to broadcast {broadcast_id}")]
    BroadcastScopeMismatch { broadcast_id: String },
    #[error("MOQT principal ownership changed during the session")]
    OwnerChanged,
    #[error("MOQT authorization has been revoked")]
    Revoked,
    #[error("MOQT revocation check failed closed: {0}")]
    RevocationUnavailable(String),
    #[error("invalid MOQT authorization resource: {0}")]
    InvalidResource(&'static str),
    #[error(transparent)]
    Replay(#[from] crate::MoqReplayError),
}

/// MOQT authorization boundary used by origins, relays, and subscribers.
#[async_trait]
pub trait MoqAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<MoqAuthorizationGrant, MoqAuthorizationError>;

    async fn recheck(
        &self,
        principal: &AuthenticatedPrincipal,
        grant: &MoqAuthorizationGrant,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError>;

    /// Atomically close a pending or authorized request while retaining replay
    /// tombstones until token expiry. Calling this method must never make a
    /// token reusable, even when it races the first authorization.
    async fn close_session(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError>;
}

/// Fail-closed MOQT policy over an authenticated rvoip principal.
///
/// Construction requires both a replay store and a revocation checker. There
/// is intentionally no permissive `Default` implementation that could skip
/// either security control.
#[derive(Clone)]
pub struct SecureMoqAuthorizer {
    replay: Arc<dyn MoqTokenReplayStore>,
    revocation: Arc<dyn MoqRevocationChecker>,
}

impl SecureMoqAuthorizer {
    pub fn new(
        replay: Arc<dyn MoqTokenReplayStore>,
        revocation: Arc<dyn MoqRevocationChecker>,
    ) -> Self {
        Self { replay, revocation }
    }

    fn validate_principal(
        principal: &AuthenticatedPrincipal,
        action: MoqAction,
        resource: &MoqResource,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        if principal.method == AuthenticationMethod::Anonymous || principal.subject.is_empty() {
            return Err(MoqAuthorizationError::Unauthenticated);
        }
        if principal.is_expired_at(now) {
            return Err(MoqAuthorizationError::PrincipalExpired);
        }
        let tenant = principal
            .tenant
            .as_deref()
            .ok_or(MoqAuthorizationError::MissingTenant)?;
        if tenant != resource.namespace().tenant_id() {
            return Err(MoqAuthorizationError::TenantMismatch);
        }

        Self::validate_action_resource(action, resource)?;
        let requirement = action.scope_requirement(resource.namespace().broadcast_id());
        let authorized = match &requirement {
            ScopeRequirement::Publish => principal.has_scope("broadcast:publish"),
            ScopeRequirement::Subscribe(scope) => principal.has_scope(scope),
            ScopeRequirement::Relay => principal.has_scope("broadcast:relay"),
            ScopeRequirement::Endpoint(scope) => {
                principal.has_scope("broadcast:publish")
                    || principal.has_scope(scope)
                    || principal.has_scope("broadcast:relay")
            }
        };
        if !authorized {
            if matches!(
                requirement,
                ScopeRequirement::Subscribe(_) | ScopeRequirement::Endpoint(_)
            ) && principal
                .scopes
                .iter()
                .any(|scope| scope.starts_with("broadcast:subscribe:"))
            {
                return Err(MoqAuthorizationError::BroadcastScopeMismatch {
                    broadcast_id: resource.namespace().broadcast_id().into(),
                });
            }
            let required = match requirement {
                ScopeRequirement::Publish => "broadcast:publish".into(),
                ScopeRequirement::Subscribe(scope) => scope,
                ScopeRequirement::Relay => "broadcast:relay".into(),
                ScopeRequirement::Endpoint(scope) => {
                    format!("broadcast:publish or {scope} or broadcast:relay")
                }
            };
            return Err(MoqAuthorizationError::MissingScope { required });
        }
        Ok(())
    }

    fn validate_action_resource(
        action: MoqAction,
        resource: &MoqResource,
    ) -> Result<(), MoqAuthorizationError> {
        let requires_track = matches!(action, MoqAction::PublishTrack | MoqAction::SubscribeTrack);
        if requires_track != resource.track_name().is_some() {
            return Err(MoqAuthorizationError::InvalidResource(if requires_track {
                "track action requires a track resource"
            } else {
                "namespace action cannot target a track resource"
            }));
        }
        Ok(())
    }

    fn map_replay_error(error: crate::MoqReplayError) -> MoqAuthorizationError {
        match error {
            crate::MoqReplayError::SessionOwnerChanged => MoqAuthorizationError::OwnerChanged,
            other => MoqAuthorizationError::Replay(other),
        }
    }

    async fn check_revocation(
        &self,
        peer: &MoqPeerIdentity,
        action: MoqAction,
        resource: &MoqResource,
        binding: &MoqTokenBinding,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        let status = self
            .revocation
            .check(peer, action, resource, binding, now)
            .await
            .map_err(|error| MoqAuthorizationError::RevocationUnavailable(error.to_string()))?;
        match status {
            MoqRevocationStatus::Active => Ok(()),
            MoqRevocationStatus::Revoked => Err(MoqAuthorizationError::Revoked),
        }
    }
}

#[async_trait]
impl MoqAuthorizer for SecureMoqAuthorizer {
    async fn authorize(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<MoqAuthorizationGrant, MoqAuthorizationError> {
        if request.expires_at <= now {
            return Err(MoqAuthorizationError::RequestExpired);
        }
        Self::validate_principal(principal, request.action, &request.resource, now)?;

        let peer = MoqPeerIdentity::from_principal(principal);
        self.check_revocation(
            &peer,
            request.action,
            &request.resource,
            &request.token_binding,
            now,
        )
        .await?;

        let expires_at = principal
            .expires_at
            .map_or(request.expires_at, |expiry| expiry.min(request.expires_at));
        self.replay
            .claim(&request.token_binding, peer.owner(), expires_at, now)
            .await
            .map_err(Self::map_replay_error)?;

        Ok(MoqAuthorizationGrant {
            peer,
            action: request.action,
            resource: request.resource.clone(),
            token_binding: request.token_binding.clone(),
            authorized_at: now,
            expires_at,
        })
    }

    async fn recheck(
        &self,
        principal: &AuthenticatedPrincipal,
        grant: &MoqAuthorizationGrant,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        if grant.expires_at <= now {
            return Err(MoqAuthorizationError::GrantExpired);
        }
        if principal.ownership_key() != *grant.peer.owner() {
            return Err(MoqAuthorizationError::OwnerChanged);
        }
        Self::validate_principal(principal, grant.action, &grant.resource, now)?;
        self.check_revocation(
            &grant.peer,
            grant.action,
            &grant.resource,
            &grant.token_binding,
            now,
        )
        .await?;
        self.replay
            .verify(&grant.token_binding, grant.peer.owner(), now)
            .await
            .map_err(Self::map_replay_error)?;
        Ok(())
    }

    async fn close_session(
        &self,
        principal: &AuthenticatedPrincipal,
        request: &MoqAuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<(), MoqAuthorizationError> {
        let expires_at = principal
            .expires_at
            .map_or(request.expires_at, |expiry| expiry.min(request.expires_at));
        self.replay
            .close(
                &request.token_binding,
                &principal.ownership_key(),
                expires_at,
                now,
            )
            .await
            .map_err(Self::map_replay_error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::Duration;
    use rvoip_core_traits::identity::IdentityAssurance;

    use super::*;
    use crate::BoundedMemoryMoqReplayStore;

    #[derive(Default)]
    struct ToggleRevocation {
        revoked: AtomicBool,
        unavailable: AtomicBool,
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
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(MoqRevocationError::Unavailable("test outage".into()));
            }
            Ok(if self.revoked.load(Ordering::SeqCst) {
                MoqRevocationStatus::Revoked
            } else {
                MoqRevocationStatus::Active
            })
        }
    }

    fn principal(
        tenant: &str,
        scopes: &[&str],
        expires_at: DateTime<Utc>,
    ) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "listener-1".into(),
            tenant: Some(tenant.into()),
            scopes: scopes.iter().map(|scope| (*scope).into()).collect(),
            issuer: Some("https://issuer.example".into()),
            expires_at: Some(expires_at),
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    fn request(
        action: MoqAction,
        tenant: &str,
        broadcast: &str,
        session: &str,
        token_byte: u8,
        expires_at: DateTime<Utc>,
    ) -> MoqAuthorizationRequest {
        MoqAuthorizationRequest::new(
            action,
            MoqResource::broadcast(crate::MoqNamespace::new(tenant, broadcast).unwrap()),
            MoqTokenBinding::from_sha256(MoqSessionId::new(session).unwrap(), [token_byte; 32])
                .unwrap(),
            expires_at,
        )
    }

    fn authorizer(
        capacity: usize,
    ) -> (
        SecureMoqAuthorizer,
        Arc<ToggleRevocation>,
        Arc<BoundedMemoryMoqReplayStore>,
    ) {
        let revocation = Arc::new(ToggleRevocation::default());
        let replay = Arc::new(BoundedMemoryMoqReplayStore::new(capacity).unwrap());
        (
            SecureMoqAuthorizer::new(replay.clone(), revocation.clone()),
            revocation,
            replay,
        )
    }

    #[tokio::test]
    async fn exact_tenant_scope_and_broadcast_are_required() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, _, _) = authorizer(8);
        let valid = request(
            MoqAction::SubscribeNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            expires,
        );

        let wrong_tenant = principal("tenant-b", &["broadcast:subscribe:broadcast-a"], expires);
        assert_eq!(
            authorizer.authorize(&wrong_tenant, &valid, now).await,
            Err(MoqAuthorizationError::TenantMismatch)
        );

        let wrong_scope = principal("tenant-a", &["calls:read"], expires);
        assert_eq!(
            authorizer.authorize(&wrong_scope, &valid, now).await,
            Err(MoqAuthorizationError::MissingScope {
                required: "broadcast:subscribe:broadcast-a".into()
            })
        );

        let wrong_broadcast = principal("tenant-a", &["broadcast:subscribe:broadcast-b"], expires);
        assert_eq!(
            authorizer.authorize(&wrong_broadcast, &valid, now).await,
            Err(MoqAuthorizationError::BroadcastScopeMismatch {
                broadcast_id: "broadcast-a".into()
            })
        );
    }

    #[tokio::test]
    async fn expired_principals_and_requests_fail_before_replay_claim() {
        let now = Utc::now();
        let (authorizer, _, replay) = authorizer(8);
        let expired_principal = principal(
            "tenant-a",
            &["broadcast:publish"],
            now - Duration::seconds(1),
        );
        let live_request = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            now + Duration::minutes(1),
        );
        assert_eq!(
            authorizer
                .authorize(&expired_principal, &live_request, now)
                .await,
            Err(MoqAuthorizationError::PrincipalExpired)
        );

        let live_principal = principal(
            "tenant-a",
            &["broadcast:publish"],
            now + Duration::minutes(1),
        );
        let expired_request = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            now,
        );
        assert_eq!(
            authorizer
                .authorize(&live_principal, &expired_request, now)
                .await,
            Err(MoqAuthorizationError::RequestExpired)
        );
        assert_eq!(replay.retained_claims(now).await, 0);
    }

    #[tokio::test]
    async fn token_is_bound_to_the_first_session() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, _, _) = authorizer(8);
        let principal = principal("tenant-a", &["broadcast:publish"], expires);
        let first = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            7,
            expires,
        );
        let replay = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-b",
            7,
            expires,
        );
        authorizer.authorize(&principal, &first, now).await.unwrap();
        assert_eq!(
            authorizer.authorize(&principal, &replay, now).await,
            Err(MoqAuthorizationError::Replay(
                crate::MoqReplayError::CrossSessionReplay
            ))
        );
    }

    #[tokio::test]
    async fn session_is_bound_to_the_first_owner_across_authorizations() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, _, _) = authorizer(8);
        let first_principal = principal("tenant-a", &["broadcast:publish"], expires);
        let first = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            expires,
        );
        authorizer
            .authorize(&first_principal, &first, now)
            .await
            .unwrap();

        let mut different_owner = principal("tenant-b", &["broadcast:publish"], expires);
        different_owner.subject = "other-subject".into();
        let same_session = request(
            MoqAction::PublishNamespace,
            "tenant-b",
            "broadcast-b",
            "session-a",
            2,
            expires,
        );
        assert_eq!(
            authorizer
                .authorize(&different_owner, &same_session, now)
                .await,
            Err(MoqAuthorizationError::OwnerChanged)
        );
    }

    #[tokio::test]
    async fn action_specific_resources_and_scopes_are_enforced() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, _, _) = authorizer(16);
        let namespace = crate::MoqNamespace::new("tenant-a", "broadcast-a").unwrap();
        let publish = principal("tenant-a", &["broadcast:publish"], expires);
        let subscribe = principal("tenant-a", &["broadcast:subscribe:broadcast-a"], expires);
        let relay = principal("tenant-a", &["broadcast:relay"], expires);

        let cases = [
            (
                MoqAction::EstablishSession,
                MoqResource::broadcast(namespace.clone()),
                &subscribe,
            ),
            (
                MoqAction::PublishNamespace,
                MoqResource::broadcast(namespace.clone()),
                &publish,
            ),
            (
                MoqAction::PublishTrack,
                MoqResource::track(namespace.clone(), "audio/main").unwrap(),
                &publish,
            ),
            (
                MoqAction::SubscribeNamespace,
                MoqResource::broadcast(namespace.clone()),
                &subscribe,
            ),
            (
                MoqAction::SubscribeTrack,
                MoqResource::track(namespace.clone(), "audio/main").unwrap(),
                &subscribe,
            ),
            (
                MoqAction::FetchCatalog,
                MoqResource::broadcast(namespace.clone()),
                &subscribe,
            ),
            (
                MoqAction::Relay,
                MoqResource::broadcast(namespace.clone()),
                &relay,
            ),
        ];
        for (index, (action, resource, principal)) in cases.into_iter().enumerate() {
            let request = MoqAuthorizationRequest::new(
                action,
                resource,
                MoqTokenBinding::from_sha256(
                    MoqSessionId::new(format!("session-{index}")).unwrap(),
                    [u8::try_from(index + 1).unwrap(); 32],
                )
                .unwrap(),
                expires,
            );
            authorizer
                .authorize(principal, &request, now)
                .await
                .unwrap();
        }

        let invalid = MoqAuthorizationRequest::new(
            MoqAction::PublishTrack,
            MoqResource::broadcast(namespace),
            MoqTokenBinding::from_sha256(MoqSessionId::new("invalid").unwrap(), [99; 32]).unwrap(),
            expires,
        );
        assert!(matches!(
            authorizer.authorize(&publish, &invalid, now).await,
            Err(MoqAuthorizationError::InvalidResource(_))
        ));
    }

    #[tokio::test]
    async fn recheck_detects_revocation_expiry_owner_change_and_store_loss() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, revocation, _) = authorizer(8);
        let principal = principal("tenant-a", &["broadcast:publish"], expires);
        let auth_request = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            now + Duration::minutes(1),
        );
        let grant = authorizer
            .authorize(&principal, &auth_request, now)
            .await
            .unwrap();

        revocation.revoked.store(true, Ordering::SeqCst);
        assert_eq!(
            authorizer.recheck(&principal, &grant, now).await,
            Err(MoqAuthorizationError::Revoked)
        );
        revocation.revoked.store(false, Ordering::SeqCst);

        let mut other_owner = principal.clone();
        other_owner.subject = "listener-2".into();
        assert_eq!(
            authorizer.recheck(&other_owner, &grant, now).await,
            Err(MoqAuthorizationError::OwnerChanged)
        );
        assert_eq!(
            authorizer
                .recheck(&principal, &grant, grant.expires_at())
                .await,
            Err(MoqAuthorizationError::GrantExpired)
        );

        authorizer
            .close_session(&principal, &auth_request, now)
            .await
            .unwrap();
        assert_eq!(
            authorizer.recheck(&principal, &grant, now).await,
            Err(MoqAuthorizationError::Replay(
                crate::MoqReplayError::TokenConsumed
            ))
        );

        let replay_after_close = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-b",
            1,
            now + Duration::minutes(1),
        );
        assert_eq!(
            authorizer
                .authorize(&principal, &replay_after_close, now)
                .await,
            Err(MoqAuthorizationError::Replay(
                crate::MoqReplayError::CrossSessionReplay
            ))
        );
    }

    #[tokio::test]
    async fn revocation_outage_fails_closed_on_authorize_and_recheck() {
        let now = Utc::now();
        let expires = now + Duration::minutes(2);
        let (authorizer, revocation, replay) = authorizer(8);
        let principal = principal("tenant-a", &["broadcast:publish"], expires);
        let request = request(
            MoqAction::PublishNamespace,
            "tenant-a",
            "broadcast-a",
            "session-a",
            1,
            expires,
        );
        revocation.unavailable.store(true, Ordering::SeqCst);
        assert!(matches!(
            authorizer.authorize(&principal, &request, now).await,
            Err(MoqAuthorizationError::RevocationUnavailable(_))
        ));
        assert_eq!(replay.retained_claims(now).await, 0);
    }

    #[test]
    fn peer_identity_and_token_diagnostics_never_include_credentials() {
        let certificate_fingerprint = "AA:BB:CC:SECRET-CERT";
        let principal = AuthenticatedPrincipal {
            subject: "origin-1".into(),
            tenant: Some("tenant-a".into()),
            scopes: vec!["broadcast:publish".into()],
            issuer: Some("internal-ca".into()),
            expires_at: None,
            method: AuthenticationMethod::MutualTls,
            assurance: IdentityAssurance::DtlsFingerprint {
                algorithm: "sha-256".into(),
                value: certificate_fingerprint.into(),
            },
        };
        let peer = MoqPeerIdentity::from_principal(&principal);
        let diagnostic = format!("{peer:?}");
        assert_eq!(peer.assurance_kind(), "dtls-fingerprint");
        assert!(!diagnostic.contains(certificate_fingerprint));
        assert!(!diagnostic.contains("broadcast:publish"));

        let binding =
            MoqTokenBinding::from_sha256(MoqSessionId::new("session-a").unwrap(), [0xab; 32])
                .unwrap();
        assert!(!format!("{binding:?}").contains("abab"));
    }

    #[test]
    fn resource_track_names_are_canonical() {
        let namespace = crate::MoqNamespace::new("tenant-a", "broadcast-a").unwrap();
        assert!(MoqResource::track(namespace.clone(), "audio/main").is_ok());
        for invalid in ["", "/audio", "audio/", "audio//main", "../events", "évents"] {
            assert!(MoqResource::track(namespace.clone(), invalid).is_err());
        }
    }
}
