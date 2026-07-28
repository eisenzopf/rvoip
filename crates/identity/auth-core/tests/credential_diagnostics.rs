use rvoip_auth_core::types::UserContext;
use rvoip_auth_core::{
    ActorClaims, AuthAuditEvent, AuthAuditOutcome, AuthAuditScheme, AuthFailureReason,
    AuthRateLimitKey, AuthRateLimitKind, CredentialAuthError, DigestAlgorithm, DigestChallenge,
    DigestComputed, DigestResponse, DigestSecret, DpopError, DpopProof, EnvelopeSignature,
    Sig9421Error, TokenRevocationContext, ValidatedDpop,
};
use rvoip_core_traits::ids::IdentityId;

const CANARY: &str = "credential-boundary-malicious-canary\r\nAuthorization: exposed";

fn assert_redacted(value: &impl std::fmt::Debug) {
    let rendered = format!("{value:?}");
    assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
}

#[test]
fn digest_and_provider_containers_keep_values_but_redact_debug() {
    let challenge = DigestChallenge {
        realm: CANARY.into(),
        nonce: CANARY.into(),
        algorithm: DigestAlgorithm::SHA256,
        qop: Some(vec![CANARY.into()]),
        opaque: Some(CANARY.into()),
    };
    let response = DigestResponse {
        username: CANARY.into(),
        realm: CANARY.into(),
        nonce: CANARY.into(),
        uri: CANARY.into(),
        response: CANARY.into(),
        algorithm: DigestAlgorithm::SHA256,
        cnonce: Some(CANARY.into()),
        qop: Some(CANARY.into()),
        nc: Some(CANARY.into()),
        opaque: Some(CANARY.into()),
    };
    let computed = DigestComputed {
        response: CANARY.into(),
        cnonce: Some(CANARY.into()),
        nc: Some(CANARY.into()),
        qop: Some(CANARY.into()),
    };
    let secret = DigestSecret::PlaintextPassword(CANARY.into());
    let revocation = TokenRevocationContext::new(CANARY)
        .with_subject(CANARY)
        .with_issuer(CANARY);

    for rendered in [
        format!("{challenge:?}"),
        format!("{response:?}"),
        format!("{computed:?}"),
        format!("{secret:?}"),
        format!("{revocation:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }

    assert_eq!(challenge.nonce, CANARY);
    assert_eq!(response.response, CANARY);
    assert_eq!(computed.response, CANARY);
    assert_eq!(secret, DigestSecret::PlaintextPassword(CANARY.into()));
    assert_eq!(revocation.token_id, CANARY);
}

#[test]
fn audit_rate_limit_and_provider_errors_redact_arbitrary_values() {
    let event = AuthAuditEvent::new(
        AuthAuditScheme::Other(CANARY.into()),
        AuthAuditOutcome::Failure(AuthFailureReason::Other(CANARY.into())),
    )
    .with_subject(CANARY)
    .with_realm(CANARY)
    .with_peer(CANARY)
    .with_metadata(CANARY, CANARY);
    let key = AuthRateLimitKey::new(AuthRateLimitKind::Other(CANARY.into()))
        .with_subject(CANARY)
        .with_realm(CANARY)
        .with_peer(CANARY);
    let provider = CredentialAuthError::Unavailable(CANARY.into());

    assert_redacted(&event);
    assert_redacted(&key);
    assert_redacted(&provider);
    assert!(!provider.to_string().contains(CANARY));

    let wire = serde_json::to_string(&event).unwrap();
    let restored: AuthAuditEvent = serde_json::from_str(&wire).unwrap();
    assert_eq!(restored, event);
    assert_eq!(restored.subject.as_deref(), Some(CANARY));
    match provider {
        CredentialAuthError::Unavailable(value) => assert_eq!(value, CANARY),
        other => panic!("unexpected provider error: {other:?}"),
    }
}

#[test]
fn credential_containers_do_not_regain_derived_debug() {
    let digest = include_str!("../src/sip_digest.rs");
    let providers = include_str!("../src/providers.rs");
    for declaration in [
        "pub struct DigestChallenge",
        "pub struct DigestResponse",
        "pub struct DigestComputed",
        "pub enum DigestSecret",
        "pub struct TokenRevocationContext",
        "pub struct AuthAuditEvent",
        "pub struct AuthRateLimitKey",
    ] {
        let source = if digest.contains(declaration) {
            digest
        } else {
            providers
        };
        let prefix = &source[..source.find(declaration).unwrap()];
        let attributes = prefix.rsplit("\n\n").next().unwrap_or_default();
        assert!(
            !attributes.contains("Debug"),
            "{declaration} regained derived Debug"
        );
    }
}

#[test]
fn decoded_claim_containers_do_not_regain_derived_debug() {
    for (source, declaration) in [
        (include_str!("../src/jwt.rs"), "struct Claims"),
        (include_str!("../src/jwt.rs"), "struct RoleAccess"),
        (include_str!("../src/jwks.rs"), "struct TokenClaims"),
        (include_str!("../src/jwks.rs"), "struct RoleAccess"),
        (include_str!("../src/jwks.rs"), "struct JwksDocument"),
        (include_str!("../src/jwks.rs"), "struct JwksKey"),
        (
            include_str!("../src/introspection.rs"),
            "struct IntrospectionResponse",
        ),
        (
            include_str!("../src/introspection.rs"),
            "enum IntrospectionAudience",
        ),
    ] {
        let declaration_offset = source
            .find(declaration)
            .unwrap_or_else(|| panic!("missing declaration: {declaration}"));
        let prefix = &source[..declaration_offset];
        let derive_offset = prefix
            .rfind("#[derive(")
            .unwrap_or_else(|| panic!("missing derive for {declaration}"));
        assert!(
            !prefix[derive_offset..].contains("Debug"),
            "{declaration} regained derived Debug"
        );
    }
}

#[test]
fn principal_proofs_signatures_and_errors_are_metadata_only() {
    let context = UserContext {
        user_id: CANARY.into(),
        username: CANARY.into(),
        roles: vec![CANARY.into()],
        claims: std::collections::HashMap::from([(CANARY.into(), serde_json::json!(CANARY))]),
        expires_at: Some(1),
        scopes: vec![CANARY.into()],
    };
    let actor = ActorClaims {
        identity: IdentityId::from_string(CANARY),
        scopes: vec![CANARY.into()],
    };
    let proof = DpopProof {
        jti: CANARY.into(),
        htm: CANARY.into(),
        htu: CANARY.into(),
        iat: 1,
        ath: Some(CANARY.into()),
    };
    let validated = ValidatedDpop {
        jkt: CANARY.into(),
        proof,
    };
    let signature = EnvelopeSignature {
        keyid: CANARY.into(),
        alg: CANARY.into(),
        sig: CANARY.into(),
    };
    let errors: Vec<Box<dyn std::fmt::Debug>> = vec![
        Box::new(DpopError::Signature(CANARY.into())),
        Box::new(Sig9421Error::UnknownKeyid(CANARY.into())),
    ];

    for rendered in [
        format!("{context:?}"),
        format!("{actor:?}"),
        format!("{validated:?}"),
        format!("{signature:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    for error in errors {
        assert_redacted(&error);
    }
    assert_eq!(context.user_id, CANARY);
    assert_eq!(actor.identity.as_str(), CANARY);
    assert_eq!(validated.jkt, CANARY);
    assert_eq!(signature.sig, CANARY);
}
