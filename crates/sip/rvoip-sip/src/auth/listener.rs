//! Enforceable listener-level SIP authentication policy.

use super::{
    SipAuthContext, SipAuthService, SipAuthSource, SipPrincipalAuthDecision,
    SipPrincipalAuthEvaluation, SipTransportSecurityContext,
};
use async_trait::async_trait;
use ipnet::IpNet;
use rvoip_core_traits::identity::{AuthenticatedPrincipal, AuthenticationMethod};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::{HeaderName, HeaderValue, StatusCode, TypedHeader};
use rvoip_sip_dialog::transaction::{
    SipRequestAuthorization, SipRequestIngressAuthorizer, SipRequestIngressContext,
    SipRequestRejection,
};
use rvoip_sip_transport::transport::TransportType;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_AUTH_RATE_LIMIT_RETRY_AFTER_SECS: u32 = 1;
const MAX_AUTH_RATE_LIMIT_RETRY_AFTER_SECS: u32 = 3_600;

fn bounded_auth_retry_after_secs(retry_after: Option<Duration>) -> u32 {
    let retry_after = retry_after
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_AUTH_RATE_LIMIT_RETRY_AFTER_SECS.into()));
    let rounded_seconds = retry_after
        .as_secs()
        .saturating_add(u64::from(retry_after.subsec_nanos() != 0));
    rounded_seconds
        .clamp(
            u64::from(DEFAULT_AUTH_RATE_LIMIT_RETRY_AFTER_SECS),
            u64::from(MAX_AUTH_RATE_LIMIT_RETRY_AFTER_SECS),
        )
        .try_into()
        .expect("bounded authentication Retry-After fits u32")
}

/// Disabled-by-default policy enforced before a new SIP request reaches the
/// dialog layer or application callbacks.
///
/// Enabled mechanisms are alternatives. A request is accepted when its
/// transport-verified certificate fingerprint has an explicit principal,
/// its source IP belongs to an explicitly trusted CIDR with a principal, or
/// its `Authorization` header is accepted by the configured
/// [`SipAuthService`].
#[derive(Clone, Default)]
pub struct SipListenerAuthPolicy {
    enabled: bool,
    /// Ownership namespace for every principal admitted by this listener.
    ///
    /// `None` is valid only while admission is disabled. It is intentionally
    /// retained in the representation so pre-tenant builder calls remain
    /// source compatible but fail closed during validation and admission.
    tenant: Option<String>,
    auth_service: Option<SipAuthService>,
    trusted_sources: Vec<(IpNet, AuthenticatedPrincipal)>,
    mtls_principals: HashMap<String, AuthenticatedPrincipal>,
}

impl fmt::Debug for SipListenerAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipListenerAuthPolicy")
            .field("enabled", &self.enabled)
            .field("tenant_configured", &self.tenant.is_some())
            .field("auth_service_configured", &self.auth_service.is_some())
            .field("trusted_source_count", &self.trusted_sources.len())
            .field("mtls_principal_count", &self.mtls_principals.len())
            .finish()
    }
}

impl SipListenerAuthPolicy {
    /// Preserve the historical unauthenticated listener behavior.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Legacy builder for credentials accepted by an existing Digest/Bearer
    /// [`SipAuthService`].
    ///
    /// An enabled tenantless policy fails closed. Complete migration with
    /// [`Self::with_tenant`] or prefer [`Self::authenticated_for_tenant`].
    pub fn authenticated(auth_service: SipAuthService) -> Self {
        Self {
            enabled: true,
            tenant: None,
            auth_service: Some(auth_service),
            trusted_sources: Vec::new(),
            mtls_principals: HashMap::new(),
        }
    }

    /// Legacy builder that enables this policy without header authentication.
    ///
    /// Add a tenant with [`Self::with_tenant`] plus at least one trusted CIDR
    /// or mTLS identity before coordinator startup, or prefer
    /// [`Self::enabled_for_tenant`]. A tenantless enabled policy fails closed.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Require credentials and bind every accepted identity to one tenant.
    ///
    /// This is the production replacement for [`Self::authenticated`]. The
    /// tenant is validated immediately: it must be 1–128 characters, already
    /// trimmed, and contain no control characters.
    pub fn authenticated_for_tenant(
        tenant: impl Into<String>,
        auth_service: SipAuthService,
    ) -> crate::errors::Result<Self> {
        Ok(Self {
            enabled: true,
            tenant: Some(validated_listener_tenant(tenant.into())?),
            auth_service: Some(auth_service),
            trusted_sources: Vec::new(),
            mtls_principals: HashMap::new(),
        })
    }

    /// Enable a tenant-bound policy for trusted-CIDR and/or mTLS mappings.
    ///
    /// At least one mechanism must still be added before coordinator startup.
    pub fn enabled_for_tenant(tenant: impl Into<String>) -> crate::errors::Result<Self> {
        Ok(Self {
            enabled: true,
            tenant: Some(validated_listener_tenant(tenant.into())?),
            ..Self::default()
        })
    }

    /// Migrate a legacy enabled policy to an explicit tenant namespace.
    ///
    /// This method is additive so existing builder chains can migrate without
    /// changing how authentication mechanisms are assembled. Static mappings
    /// are checked by [`Self::validate`] before a listener is opened.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> crate::errors::Result<Self> {
        self.tenant = Some(validated_listener_tenant(tenant.into())?);
        Ok(self)
    }

    /// The configured listener tenant, if admission is tenant-bound.
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }

    /// Whether this policy contains certificate-fingerprint mappings and
    /// therefore requires transport-level client-certificate verification.
    pub fn has_verified_mtls_peers(&self) -> bool {
        !self.mtls_principals.is_empty()
    }

    /// Add Digest/Bearer header authentication as an accepted mechanism.
    pub fn with_auth_service(mut self, auth_service: SipAuthService) -> Self {
        self.enabled = true;
        self.auth_service = Some(auth_service);
        self
    }

    /// Trust a source network as the explicitly supplied principal.
    ///
    /// Source IP is only a selector for configured identity; it is never
    /// promoted into a principal automatically.
    pub fn with_trusted_cidr(mut self, cidr: IpNet, principal: AuthenticatedPrincipal) -> Self {
        self.enabled = true;
        self.trusted_sources.push((cidr, principal));
        self
    }

    /// Map a rustls-verified client leaf-certificate SHA-256 fingerprint to
    /// an explicit principal.
    ///
    /// The transport must have produced the fingerprint after successful
    /// `WebPkiClientVerifier` validation. The returned principal's method is
    /// normalized to `mutual-tls`.
    pub fn with_verified_mtls_peer(
        mut self,
        leaf_certificate_sha256: impl Into<String>,
        mut principal: AuthenticatedPrincipal,
    ) -> Self {
        self.enabled = true;
        principal.method = AuthenticationMethod::MutualTls;
        self.mtls_principals.insert(
            leaf_certificate_sha256.into().to_ascii_lowercase(),
            principal,
        );
        self
    }

    /// Whether transaction ingress enforcement should be installed.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Validate static policy identity mappings before opening listeners.
    pub fn validate(&self) -> crate::errors::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let tenant = self.tenant.as_deref().ok_or_else(|| {
            crate::errors::SessionError::ConfigError(
                "enabled SIP listener auth policy requires an explicit tenant; migrate with SipListenerAuthPolicy::authenticated_for_tenant, enabled_for_tenant, or with_tenant"
                    .to_string(),
            )
        })?;
        validate_listener_tenant(tenant)?;
        if self.auth_service.is_none()
            && self.trusted_sources.is_empty()
            && self.mtls_principals.is_empty()
        {
            return Err(crate::errors::SessionError::ConfigError(
                "enabled SIP listener auth policy has no authentication mechanisms".to_string(),
            ));
        }
        for (_, principal) in &self.trusted_sources {
            validate_static_principal(principal, "trusted CIDR")?;
            validate_static_principal_tenant(principal, tenant, "trusted CIDR")?;
        }
        for (fingerprint, principal) in &self.mtls_principals {
            if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(crate::errors::SessionError::ConfigError(
                    "mTLS leaf certificate SHA-256 fingerprint must be 64 hexadecimal characters"
                        .to_string(),
                ));
            }
            validate_static_principal(principal, "mTLS fingerprint")?;
            validate_static_principal_tenant(principal, tenant, "mTLS fingerprint")?;
        }
        Ok(())
    }

    pub(crate) fn into_authorizer(self) -> Option<Arc<dyn SipRequestIngressAuthorizer>> {
        self.enabled
            .then(|| Arc::new(self) as Arc<dyn SipRequestIngressAuthorizer>)
    }

    fn mapped_transport_principal(
        &self,
        context: &SipRequestIngressContext,
    ) -> Option<AuthenticatedPrincipal> {
        if !matches!(
            context.transport_type,
            TransportType::Tls | TransportType::Wss
        ) {
            return None;
        }
        let fingerprint = context
            .connection_metadata
            .as_ref()?
            .tls_peer_identity
            .leaf_certificate_sha256
            .to_ascii_lowercase();
        self.mtls_principals.get(&fingerprint).cloned()
    }

    fn trusted_source_principal(
        &self,
        context: &SipRequestIngressContext,
    ) -> Option<AuthenticatedPrincipal> {
        self.trusted_sources
            .iter()
            .find(|(cidr, _)| cidr.contains(&context.source.ip()))
            .map(|(_, principal)| principal.clone())
    }

    fn rejection_from_challenges(
        challenges: Vec<super::SipAuthChallenge>,
    ) -> SipRequestAuthorization {
        let mut rejection = SipRequestRejection::new(StatusCode::Unauthorized)
            .with_reason("SIP listener credentials rejected");
        for challenge in challenges {
            let name = match challenge.source {
                SipAuthSource::Origin => HeaderName::WwwAuthenticate,
                SipAuthSource::Proxy => HeaderName::ProxyAuthenticate,
            };
            rejection = rejection.with_header(TypedHeader::Other(
                name,
                HeaderValue::Raw(challenge.value.into_bytes()),
            ));
        }
        SipRequestAuthorization::Rejected(rejection)
    }

    fn tenant_bound_static_principal(
        &self,
        principal: AuthenticatedPrincipal,
    ) -> Option<AuthenticatedPrincipal> {
        let tenant = self.tenant.as_deref()?;
        (principal.tenant.as_deref() == Some(tenant)).then_some(principal)
    }

    fn tenant_bound_header_principal(
        &self,
        mut principal: AuthenticatedPrincipal,
    ) -> Option<AuthenticatedPrincipal> {
        let tenant = self.tenant.as_deref()?;
        match principal.tenant.as_deref() {
            Some(principal_tenant) if principal_tenant == tenant => Some(principal),
            Some(_) => None,
            None if principal.method == AuthenticationMethod::SipDigest => {
                principal.tenant = Some(tenant.to_string());
                Some(principal)
            }
            None => None,
        }
    }
}

#[async_trait]
impl SipRequestIngressAuthorizer for SipListenerAuthPolicy {
    async fn authorize(
        &self,
        request: &rvoip_sip_core::Request,
        context: &SipRequestIngressContext,
    ) -> SipRequestAuthorization {
        if !self.enabled {
            return SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::ServerInternalError)
                    .with_reason("disabled SIP listener policy was installed"),
            );
        }

        if self.tenant.is_none() {
            return SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::ServerInternalError)
                    .with_reason("SIP listener tenant is not configured"),
            );
        }

        if let Some(principal) = self
            .mapped_transport_principal(context)
            .or_else(|| self.trusted_source_principal(context))
        {
            if principal.is_expired() {
                return SipRequestAuthorization::Rejected(
                    SipRequestRejection::new(StatusCode::Forbidden)
                        .with_reason("configured SIP listener principal is expired"),
                );
            }
            return match self.tenant_bound_static_principal(principal) {
                Some(principal) => SipRequestAuthorization::Authorized { principal },
                None => SipRequestAuthorization::Rejected(
                    SipRequestRejection::new(StatusCode::Forbidden)
                        .with_reason("SIP listener principal tenant is not authorized"),
                ),
            };
        }

        let Some(auth_service) = &self.auth_service else {
            return SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::Forbidden)
                    .with_reason("SIP listener peer is not trusted"),
            );
        };

        let authorization = request.raw_header_value(&HeaderName::Authorization);
        let transport = SipTransportSecurityContext {
            transport: Some(context.transport_type.to_string()),
            local_addr: Some(context.destination.to_string()),
            remote_addr: Some(context.source.to_string()),
            secure: matches!(
                context.transport_type,
                rvoip_sip_transport::transport::TransportType::Tls
                    | rvoip_sip_transport::transport::TransportType::Wss
            ),
        };
        let auth_context = SipAuthContext::new()
            // Source ports are ephemeral and attacker-controlled. Aggregate
            // auth attempts by source IP so reconnecting or rotating ports
            // cannot bypass the listener's peer rate limit.
            .with_peer(rate_limit_peer(context.source))
            .with_metadata("transport", context.transport_type.to_string());
        let body = (!request.body().is_empty()).then(|| request.body());

        match auth_service
            .evaluate_principal_with_context_and_transport(
                authorization.as_deref(),
                request.method().as_str(),
                &request.uri().to_string(),
                body,
                SipAuthSource::Origin,
                &transport,
                &auth_context,
            )
            .await
        {
            Ok(SipPrincipalAuthEvaluation::Decision(SipPrincipalAuthDecision::Authorized {
                principal,
                ..
            })) if !principal.is_expired() => match self.tenant_bound_header_principal(principal) {
                Some(principal) => SipRequestAuthorization::Authorized { principal },
                None => SipRequestAuthorization::Rejected(
                    SipRequestRejection::new(StatusCode::Forbidden)
                        .with_reason("SIP listener principal tenant is not authorized"),
                ),
            },
            Ok(SipPrincipalAuthEvaluation::Decision(SipPrincipalAuthDecision::Authorized {
                ..
            })) => SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::Unauthorized)
                    .with_reason("SIP listener principal is expired"),
            ),
            Ok(SipPrincipalAuthEvaluation::Decision(SipPrincipalAuthDecision::Rejected {
                challenges,
            })) => Self::rejection_from_challenges(challenges),
            Ok(SipPrincipalAuthEvaluation::RateLimited { retry_after }) => {
                let seconds = bounded_auth_retry_after_secs(retry_after);
                SipRequestAuthorization::Rejected(
                    SipRequestRejection::new(StatusCode::ServiceUnavailable)
                        .with_header(TypedHeader::RetryAfter(
                            rvoip_sip_core::types::retry_after::RetryAfter::new(seconds),
                        ))
                        .with_reason("SIP listener authentication rate limited"),
                )
            }
            Err(error) => SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::ServiceUnavailable)
                    .with_reason(format!("SIP listener authentication unavailable: {error}")),
            ),
        }
    }
}

fn rate_limit_peer(source: std::net::SocketAddr) -> String {
    source.ip().to_string()
}

fn validated_listener_tenant(tenant: String) -> crate::errors::Result<String> {
    validate_listener_tenant(&tenant)?;
    Ok(tenant)
}

fn validate_listener_tenant(tenant: &str) -> crate::errors::Result<()> {
    let character_count = tenant.chars().count();
    if !(1..=128).contains(&character_count)
        || tenant.trim() != tenant
        || tenant.chars().any(char::is_control)
    {
        return Err(crate::errors::SessionError::ConfigError(
            "SIP listener tenant must be 1-128 characters, already trimmed, and contain no control characters"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_static_principal(
    principal: &AuthenticatedPrincipal,
    mechanism: &str,
) -> crate::errors::Result<()> {
    if principal.subject.trim().is_empty() || principal.subject.chars().any(char::is_control) {
        return Err(crate::errors::SessionError::ConfigError(format!(
            "{mechanism} principal subject must be non-empty and contain no control characters"
        )));
    }
    if principal.is_expired() {
        return Err(crate::errors::SessionError::ConfigError(format!(
            "{mechanism} principal is already expired"
        )));
    }
    Ok(())
}

fn validate_static_principal_tenant(
    principal: &AuthenticatedPrincipal,
    tenant: &str,
    mechanism: &str,
) -> crate::errors::Result<()> {
    if principal.tenant.as_deref() != Some(tenant) {
        return Err(crate::errors::SessionError::ConfigError(format!(
            "{mechanism} principal tenant must exactly match the SIP listener tenant"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{
        AuthAttemptAdmission, AuthAttemptReservation, AuthAuditOutcome, AuthRateLimitKey,
        AuthRateLimitVerdict, AuthRateLimiter, BearerAuthError, BearerValidator,
        CredentialAuthError, DigestAuth, DigestAuthenticator,
    };
    use rvoip_core_traits::identity::{CredentialKind, IdentityAssurance};
    use rvoip_core_traits::ids::IdentityId;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_transport::transport::{
        TlsPeerIdentity, TransportConnectionMetadata, TransportType,
    };
    use std::str::FromStr;

    fn principal(subject: &str, method: AuthenticationMethod) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: subject.to_string(),
            tenant: Some("tenant-a".to_string()),
            scopes: vec!["sip:call".to_string()],
            issuer: Some("listener-test".to_string()),
            expires_at: None,
            method,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::SipDigest,
            },
        }
    }

    fn invite(authorization: Option<&str>) -> Request {
        let mut request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.test")
            .unwrap()
            .from("Alice", "sip:alice@example.test", Some("alice-tag"))
            .to("Bob", "sip:bob@example.test", None)
            .call_id("listener-policy-call")
            .cseq(1)
            .via("192.0.2.10:5060", "UDP", Some("z9hG4bK.listener-policy"))
            .max_forwards(70)
            .build();
        if let Some(authorization) = authorization {
            request.headers.push(TypedHeader::Other(
                HeaderName::Authorization,
                HeaderValue::Raw(authorization.as_bytes().to_vec()),
            ));
        }
        request
    }

    fn context(source: &str, transport_type: TransportType) -> SipRequestIngressContext {
        SipRequestIngressContext::new(
            source.parse().unwrap(),
            "192.0.2.20:5060".parse().unwrap(),
            transport_type,
        )
    }

    #[derive(Clone)]
    struct DenyingRateLimiter {
        retry_after: Option<Duration>,
    }

    #[async_trait]
    impl AuthRateLimiter for DenyingRateLimiter {
        async fn check_auth_attempt(
            &self,
            _key: &AuthRateLimitKey,
        ) -> std::result::Result<AuthRateLimitVerdict, CredentialAuthError> {
            Ok(AuthRateLimitVerdict::Denied {
                retry_after: self.retry_after,
            })
        }

        async fn record_auth_result(
            &self,
            _key: &AuthRateLimitKey,
            _outcome: &AuthAuditOutcome,
        ) -> std::result::Result<(), CredentialAuthError> {
            panic!("denied listener admission must not record an auth result")
        }

        async fn reserve_auth_attempt(
            &self,
            _key: &AuthRateLimitKey,
        ) -> std::result::Result<AuthAttemptAdmission, CredentialAuthError> {
            Ok(AuthAttemptAdmission::Denied {
                retry_after: self.retry_after,
            })
        }

        async fn complete_auth_attempt(
            &self,
            _reservation: &AuthAttemptReservation,
            _outcome: &AuthAuditOutcome,
        ) -> std::result::Result<(), CredentialAuthError> {
            panic!("denied listener admission must not complete a reservation")
        }
    }

    #[test]
    fn rate_limit_peer_ignores_attacker_controlled_source_ports() {
        let first: std::net::SocketAddr = "192.0.2.42:5060".parse().unwrap();
        let rotated: std::net::SocketAddr = "192.0.2.42:65000".parse().unwrap();
        assert_eq!(rate_limit_peer(first), "192.0.2.42");
        assert_eq!(rate_limit_peer(first), rate_limit_peer(rotated));
    }

    #[test]
    fn authentication_retry_after_is_rounded_and_bounded() {
        for (retry_after, expected) in [
            (None, 1),
            (Some(Duration::ZERO), 1),
            (Some(Duration::from_nanos(1)), 1),
            (Some(Duration::from_millis(1_500)), 2),
            (Some(Duration::from_secs(3_600)), 3_600),
            (Some(Duration::from_secs(3_601)), 3_600),
            (Some(Duration::from_secs(u64::MAX)), 3_600),
        ] {
            assert_eq!(bounded_auth_retry_after_secs(retry_after), expected);
        }
    }

    #[tokio::test]
    async fn rate_limited_listener_rejects_with_bounded_retry_after_without_challenge() {
        for (retry_after, expected) in [
            (None, 1),
            (Some(Duration::from_millis(1_500)), 2),
            (Some(Duration::from_secs(86_400)), 3_600),
        ] {
            let service = SipAuthService::digest("listener")
                .with_digest_user("alice", "secret")
                .with_rate_limiter(Arc::new(DenyingRateLimiter { retry_after }));
            let policy = SipListenerAuthPolicy::authenticated_for_tenant("tenant-a", service)
                .expect("tenant-bound policy");
            let decision = policy
                .authorize(
                    &invite(None),
                    &context("192.0.2.42:5060", TransportType::Udp),
                )
                .await;
            let SipRequestAuthorization::Rejected(rejection) = decision else {
                panic!("rate-limited request was unexpectedly authorized");
            };
            assert_eq!(rejection.status, StatusCode::ServiceUnavailable);
            assert!(!rejection.headers.iter().any(|header| {
                matches!(
                    header.name(),
                    HeaderName::WwwAuthenticate | HeaderName::ProxyAuthenticate
                )
            }));
            assert!(rejection.headers.iter().any(|header| {
                matches!(header, TypedHeader::RetryAfter(value) if value.delay == expected)
            }));
        }
    }

    #[test]
    fn disabled_policy_does_not_install_an_authorizer() {
        let policy = SipListenerAuthPolicy::disabled();
        policy
            .validate()
            .expect("disabled policy remains compatible");
        assert!(policy.into_authorizer().is_none());
    }

    #[test]
    fn enabled_legacy_policy_requires_explicit_tenant_migration() {
        let legacy = SipListenerAuthPolicy::authenticated(
            SipAuthService::digest("listener").with_digest_user("alice", "secret"),
        );
        let error = legacy
            .validate()
            .expect_err("tenantless enabled policy must fail closed");
        assert!(matches!(
            error,
            crate::errors::SessionError::ConfigError(ref detail)
                if detail.contains("explicit tenant")
        ));

        let migrated = legacy
            .with_tenant("tenant-a")
            .expect("valid migration tenant");
        migrated.validate().expect("migrated policy");
    }

    #[test]
    fn tenant_construction_rejects_ambiguous_or_malformed_values() {
        for tenant in ["", " tenant-a", "tenant-a ", "tenant\na", "tenant\u{0000}a"] {
            assert!(SipListenerAuthPolicy::enabled_for_tenant(tenant).is_err());
        }
        assert!(SipListenerAuthPolicy::enabled_for_tenant("x".repeat(129)).is_err());

        let max = "é".repeat(128);
        let policy = SipListenerAuthPolicy::enabled_for_tenant(max.clone())
            .expect("limit is measured in characters");
        assert_eq!(policy.tenant(), Some(max.as_str()));
    }

    #[tokio::test]
    async fn tenantless_enabled_policy_also_fails_closed_without_startup_validation() {
        let policy = SipListenerAuthPolicy::enabled().with_trusted_cidr(
            IpNet::from_str("192.0.2.0/24").unwrap(),
            principal("trusted-gateway", AuthenticationMethod::ApiKey),
        );
        let decision = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5060", TransportType::Udp),
            )
            .await;
        assert!(matches!(
            decision,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::ServerInternalError
        ));
    }

    #[test]
    fn listener_policy_debug_omits_mapped_principals_selectors_and_auth_config() {
        const PRINCIPAL_CANARY: &str = "mapped-principal-secret-canary";
        const FINGERPRINT_CANARY: &str = "fingerprint-secret-canary";
        let mut mapped = principal(PRINCIPAL_CANARY, AuthenticationMethod::ApiKey);
        let mapped_tenant = format!("tenant-{PRINCIPAL_CANARY}");
        mapped.tenant = Some(mapped_tenant.clone());
        mapped.issuer = Some(format!("issuer-{PRINCIPAL_CANARY}"));
        mapped.scopes = vec![format!("scope-{PRINCIPAL_CANARY}")];

        let policy = SipListenerAuthPolicy::authenticated_for_tenant(
            mapped_tenant,
            SipAuthService::digest(PRINCIPAL_CANARY),
        )
        .expect("tenant-bound policy")
        .with_trusted_cidr("192.0.2.0/24".parse().expect("CIDR"), mapped.clone())
        .with_verified_mtls_peer(FINGERPRINT_CANARY, mapped.clone());

        let rendered = format!("{policy:?}");
        assert_eq!(
            rendered,
            "SipListenerAuthPolicy { enabled: true, tenant_configured: true, auth_service_configured: true, trusted_source_count: 1, mtls_principal_count: 1 }"
        );
        for canary in [PRINCIPAL_CANARY, FINGERPRINT_CANARY, "192.0.2.0/24"] {
            assert!(
                !rendered.contains(canary),
                "listener auth policy leaked mapped identity data: {rendered}"
            );
        }

        // Diagnostic hardening must not alter selector or principal behavior.
        assert_eq!(policy.trusted_sources[0].1.subject, PRINCIPAL_CANARY);
        assert_eq!(
            policy
                .mtls_principals
                .get(&FINGERPRINT_CANARY.to_ascii_lowercase())
                .expect("mapped mTLS principal")
                .subject,
            PRINCIPAL_CANARY
        );
    }

    #[tokio::test]
    async fn trusted_cidr_requires_match_and_uses_explicit_principal() {
        let expected = principal("trusted-gateway", AuthenticationMethod::ApiKey);
        let policy = SipListenerAuthPolicy::enabled_for_tenant("tenant-a")
            .expect("tenant-bound policy")
            .with_trusted_cidr(IpNet::from_str("192.0.2.0/24").unwrap(), expected.clone());

        let accepted = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5060", TransportType::Udp),
            )
            .await;
        assert!(matches!(
            accepted,
            SipRequestAuthorization::Authorized { principal }
                if principal.ownership_key() == expected.ownership_key()
        ));

        let rejected = policy
            .authorize(
                &invite(None),
                &context("198.51.100.42:5060", TransportType::Udp),
            )
            .await;
        assert!(matches!(
            rejected,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::Forbidden
        ));
    }

    #[tokio::test]
    async fn trusted_cidr_principal_must_exactly_match_listener_tenant() {
        for tenant in [None, Some("tenant-b".to_string())] {
            let mut mapped = principal("trusted-gateway", AuthenticationMethod::ApiKey);
            mapped.tenant = tenant;
            let policy = SipListenerAuthPolicy::enabled_for_tenant("tenant-a")
                .expect("tenant-bound policy")
                .with_trusted_cidr(IpNet::from_str("192.0.2.0/24").unwrap(), mapped);
            assert!(policy.validate().is_err());
            let decision = policy
                .authorize(
                    &invite(None),
                    &context("192.0.2.42:5060", TransportType::Udp),
                )
                .await;
            assert!(matches!(
                decision,
                SipRequestAuthorization::Rejected(ref rejection)
                    if rejection.status == StatusCode::Forbidden
            ));
        }
    }

    #[tokio::test]
    async fn mtls_mapping_only_accepts_transport_verified_fingerprint() {
        let fingerprint = "ab".repeat(32);
        let expected = principal("mtls-gateway", AuthenticationMethod::Bearer);
        let policy = SipListenerAuthPolicy::enabled_for_tenant("tenant-a")
            .expect("tenant-bound policy")
            .with_verified_mtls_peer(fingerprint.clone(), expected.clone());
        let metadata = TransportConnectionMetadata {
            tls_peer_identity: TlsPeerIdentity {
                leaf_certificate_sha256: fingerprint,
                presented_chain_len: 2,
            },
        };

        let accepted = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5061", TransportType::Tls).with_connection_metadata(metadata),
            )
            .await;
        assert!(matches!(
            accepted,
            SipRequestAuthorization::Authorized { principal }
                if principal.ownership_key() == expected.ownership_key()
                    && principal.method == AuthenticationMethod::MutualTls
        ));

        let rejected = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5061", TransportType::Tls),
            )
            .await;
        assert!(matches!(
            rejected,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::Forbidden
        ));

        let forged_metadata = TransportConnectionMetadata {
            tls_peer_identity: TlsPeerIdentity {
                leaf_certificate_sha256: "ab".repeat(32),
                presented_chain_len: 2,
            },
        };
        let rejected = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5060", TransportType::Udp)
                    .with_connection_metadata(forged_metadata),
            )
            .await;
        assert!(matches!(
            rejected,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::Forbidden
        ));
    }

    #[tokio::test]
    async fn mtls_principal_must_exactly_match_listener_tenant() {
        let fingerprint = "cd".repeat(32);
        let mut mapped = principal("mtls-gateway", AuthenticationMethod::Bearer);
        mapped.tenant = Some("tenant-b".to_string());
        let policy = SipListenerAuthPolicy::enabled_for_tenant("tenant-a")
            .expect("tenant-bound policy")
            .with_verified_mtls_peer(fingerprint.clone(), mapped);
        assert!(policy.validate().is_err());

        let metadata = TransportConnectionMetadata {
            tls_peer_identity: TlsPeerIdentity {
                leaf_certificate_sha256: fingerprint,
                presented_chain_len: 1,
            },
        };
        let decision = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5061", TransportType::Tls).with_connection_metadata(metadata),
            )
            .await;
        assert!(matches!(
            decision,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::Forbidden
        ));
    }

    #[tokio::test]
    async fn digest_header_auth_produces_canonical_principal() {
        let service = SipAuthService::digest("listener")
            .with_digest_user("alice", "correct horse battery staple");
        let challenge = service
            .challenges_async(SipAuthSource::Origin)
            .await
            .unwrap()
            .into_iter()
            .find(|challenge| challenge.scheme == super::super::SipAuthScheme::Digest)
            .unwrap();
        let challenge = DigestAuthenticator::parse_challenge(&challenge.value).unwrap();
        let computed = DigestAuth::compute_response_with_state(
            "alice",
            "correct horse battery staple",
            &challenge,
            "INVITE",
            "sip:bob@example.test",
            1,
            None,
        )
        .unwrap();
        let authorization = DigestAuth::format_authorization_with_state(
            "alice",
            &challenge,
            "sip:bob@example.test",
            &computed,
        );
        let policy = SipListenerAuthPolicy::authenticated_for_tenant("tenant-a", service)
            .expect("tenant-bound policy");

        let decision = policy
            .authorize(
                &invite(Some(&authorization)),
                &context("192.0.2.42:5060", TransportType::Udp),
            )
            .await;
        assert!(matches!(
            decision,
            SipRequestAuthorization::Authorized { principal }
                if principal.subject == "alice"
                    && principal.tenant.as_deref() == Some("tenant-a")
                    && principal.method == AuthenticationMethod::SipDigest
                    && principal.issuer.as_deref() == Some("sip-digest:listener")
        ));
    }

    struct FullPrincipalBearerValidator {
        principal: AuthenticatedPrincipal,
    }

    impl fmt::Debug for FullPrincipalBearerValidator {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("FullPrincipalBearerValidator")
                .field("method", &self.principal.method)
                .field("assurance", &self.principal.assurance.kind())
                .field("tenant_present", &self.principal.tenant.is_some())
                .field("issuer_present", &self.principal.issuer.is_some())
                .field("scope_count", &self.principal.scopes.len())
                .finish()
        }
    }

    #[async_trait]
    impl BearerValidator for FullPrincipalBearerValidator {
        async fn validate(
            &self,
            token: &str,
        ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
            if token != "valid-token" {
                return Err(BearerAuthError::Invalid("bad token".to_string()));
            }
            Ok(self.principal.assurance.clone())
        }

        async fn validate_principal(
            &self,
            token: &str,
        ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
            if token != "valid-token" {
                return Err(BearerAuthError::Invalid("bad token".to_string()));
            }
            Ok(self.principal.clone())
        }
    }

    #[tokio::test]
    async fn bearer_header_auth_retains_full_validator_principal() {
        let expected = AuthenticatedPrincipal {
            subject: "bearer-user".to_string(),
            tenant: Some("tenant-42".to_string()),
            scopes: vec!["sip:call".to_string(), "calls:transfer".to_string()],
            issuer: Some("https://issuer.example".to_string()),
            expires_at: None,
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::UserAuthorized {
                identity: IdentityId::from_string("identity-42"),
                user_id: IdentityId::from_string("bearer-user"),
                scopes: vec!["sip:call".to_string(), "calls:transfer".to_string()],
            },
        };
        let service = SipAuthService::new().with_bearer_validator(
            "listener",
            Arc::new(FullPrincipalBearerValidator {
                principal: expected.clone(),
            }),
        );
        let policy = SipListenerAuthPolicy::authenticated_for_tenant("tenant-42", service)
            .expect("tenant-bound policy");

        let decision = policy
            .authorize(
                &invite(Some("Bearer valid-token")),
                &context("192.0.2.42:5061", TransportType::Tls),
            )
            .await;
        assert!(matches!(
            decision,
            SipRequestAuthorization::Authorized { principal }
                if principal.ownership_key() == expected.ownership_key()
                    && principal.scopes == expected.scopes
                    && principal.method == AuthenticationMethod::Jwt
        ));
    }

    #[tokio::test]
    async fn bearer_principal_requires_exact_listener_tenant() {
        for tenant in [None, Some("tenant-b".to_string())] {
            let mut candidate = principal("bearer-user", AuthenticationMethod::Jwt);
            candidate.tenant = tenant;
            let service = SipAuthService::new().with_bearer_validator(
                "listener",
                Arc::new(FullPrincipalBearerValidator {
                    principal: candidate,
                }),
            );
            let policy = SipListenerAuthPolicy::authenticated_for_tenant("tenant-a", service)
                .expect("tenant-bound policy");
            let decision = policy
                .authorize(
                    &invite(Some("Bearer valid-token")),
                    &context("192.0.2.42:5061", TransportType::Tls),
                )
                .await;
            assert!(matches!(
                decision,
                SipRequestAuthorization::Rejected(ref rejection)
                    if rejection.status == StatusCode::Forbidden
            ));
        }
    }

    #[tokio::test]
    async fn missing_header_is_challenged_and_never_authorized() {
        let policy = SipListenerAuthPolicy::authenticated_for_tenant(
            "tenant-a",
            SipAuthService::digest("listener").with_digest_user("alice", "secret"),
        )
        .expect("tenant-bound policy");
        let decision = policy
            .authorize(
                &invite(None),
                &context("192.0.2.42:5060", TransportType::Udp),
            )
            .await;
        assert!(matches!(
            decision,
            SipRequestAuthorization::Rejected(ref rejection)
                if rejection.status == StatusCode::Unauthorized
                    && rejection.headers.iter().any(|header| header.name() == HeaderName::WwwAuthenticate)
        ));
    }
}
