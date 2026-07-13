//! Enforceable listener-level SIP authentication policy.

use super::{
    SipAuthContext, SipAuthService, SipAuthSource, SipPrincipalAuthDecision,
    SipTransportSecurityContext,
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
    auth_service: Option<SipAuthService>,
    trusted_sources: Vec<(IpNet, AuthenticatedPrincipal)>,
    mtls_principals: HashMap<String, AuthenticatedPrincipal>,
}

impl fmt::Debug for SipListenerAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipListenerAuthPolicy")
            .field("enabled", &self.enabled)
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

    /// Require credentials accepted by an existing Digest/Bearer
    /// [`SipAuthService`].
    pub fn authenticated(auth_service: SipAuthService) -> Self {
        Self {
            enabled: true,
            auth_service: Some(auth_service),
            trusted_sources: Vec::new(),
            mtls_principals: HashMap::new(),
        }
    }

    /// Enable this policy without header authentication. Add at least one
    /// trusted CIDR or mTLS identity before coordinator startup.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
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
            return SipRequestAuthorization::Authorized { principal };
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
            .with_peer(context.source.to_string())
            .with_metadata("transport", context.transport_type.to_string());
        let body = (!request.body().is_empty()).then(|| request.body());

        match auth_service
            .authenticate_principal_with_context_and_transport(
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
            Ok(SipPrincipalAuthDecision::Authorized { principal, .. })
                if !principal.is_expired() =>
            {
                SipRequestAuthorization::Authorized { principal }
            }
            Ok(SipPrincipalAuthDecision::Authorized { .. }) => SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::Unauthorized)
                    .with_reason("SIP listener principal is expired"),
            ),
            Ok(SipPrincipalAuthDecision::Rejected { challenges }) => {
                Self::rejection_from_challenges(challenges)
            }
            Err(error) => SipRequestAuthorization::Rejected(
                SipRequestRejection::new(StatusCode::ServiceUnavailable)
                    .with_reason(format!("SIP listener authentication unavailable: {error}")),
            ),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{BearerAuthError, BearerValidator, DigestAuth, DigestAuthenticator};
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

    #[test]
    fn disabled_policy_does_not_install_an_authorizer() {
        assert!(SipListenerAuthPolicy::disabled()
            .into_authorizer()
            .is_none());
    }

    #[test]
    fn listener_policy_debug_omits_mapped_principals_selectors_and_auth_config() {
        const PRINCIPAL_CANARY: &str = "mapped-principal-secret-canary";
        const FINGERPRINT_CANARY: &str = "fingerprint-secret-canary";
        let mut mapped = principal(PRINCIPAL_CANARY, AuthenticationMethod::ApiKey);
        mapped.tenant = Some(format!("tenant-{PRINCIPAL_CANARY}"));
        mapped.issuer = Some(format!("issuer-{PRINCIPAL_CANARY}"));
        mapped.scopes = vec![format!("scope-{PRINCIPAL_CANARY}")];

        let policy = SipListenerAuthPolicy::authenticated(SipAuthService::digest(PRINCIPAL_CANARY))
            .with_trusted_cidr("192.0.2.0/24".parse().expect("CIDR"), mapped.clone())
            .with_verified_mtls_peer(FINGERPRINT_CANARY, mapped.clone());

        let rendered = format!("{policy:?}");
        assert_eq!(
            rendered,
            "SipListenerAuthPolicy { enabled: true, auth_service_configured: true, trusted_source_count: 1, mtls_principal_count: 1 }"
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
        let policy = SipListenerAuthPolicy::enabled()
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
    async fn mtls_mapping_only_accepts_transport_verified_fingerprint() {
        let fingerprint = "ab".repeat(32);
        let expected = principal("mtls-gateway", AuthenticationMethod::Bearer);
        let policy = SipListenerAuthPolicy::enabled()
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
        let policy = SipListenerAuthPolicy::authenticated(service);

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
        let policy = SipListenerAuthPolicy::authenticated(service);

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
    async fn missing_header_is_challenged_and_never_authorized() {
        let policy = SipListenerAuthPolicy::authenticated(
            SipAuthService::digest("listener").with_digest_user("alice", "secret"),
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
                if rejection.status == StatusCode::Unauthorized
                    && rejection.headers.iter().any(|header| header.name() == HeaderName::WwwAuthenticate)
        ));
    }
}
