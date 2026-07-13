use rvoip_auth_core::EnvelopeSignature;
use rvoip_uctp::payloads::auth::{AuthChallenge, AuthRefresh, AuthResponse};
use rvoip_uctp::payloads::control::IdentityStepUpResponse;
use rvoip_uctp::{MessageType, UctpEnvelope};

const CANARY: &str = "uctp-credential-malicious-canary\r\nAuthorization: exposed";

#[test]
fn direct_auth_and_control_payloads_redact_but_serialize_exactly() {
    let response = AuthResponse {
        method: "bearer".into(),
        credential: CANARY.into(),
        actor_token: Some(CANARY.into()),
    };
    let refresh = AuthRefresh {
        method: "oauth2-dpop".into(),
        credential: CANARY.into(),
        actor_token: Some(CANARY.into()),
    };
    let challenge = AuthChallenge {
        nonce: CANARY.into(),
        accepted_methods: vec!["bearer".into()],
        server_capabilities: serde_json::json!({"proof": CANARY}),
    };
    let step_up = IdentityStepUpResponse {
        method: "bearer".into(),
        credential: CANARY.into(),
    };

    for rendered in [
        format!("{response:?}"),
        format!("{refresh:?}"),
        format!("{challenge:?}"),
        format!("{step_up:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    let response: AuthResponse =
        serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
    let refresh: AuthRefresh =
        serde_json::from_str(&serde_json::to_string(&refresh).unwrap()).unwrap();
    let challenge: AuthChallenge =
        serde_json::from_str(&serde_json::to_string(&challenge).unwrap()).unwrap();
    let step_up: IdentityStepUpResponse =
        serde_json::from_str(&serde_json::to_string(&step_up).unwrap()).unwrap();
    assert_eq!(response.credential, CANARY);
    assert_eq!(refresh.credential, CANARY);
    assert_eq!(challenge.nonce, CANARY);
    assert_eq!(step_up.credential, CANARY);
}

#[test]
fn enclosing_envelope_never_delegates_to_payload_or_signature_debug() {
    let envelope = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::json!({"credential": CANARY}),
    )
    .with_signature(EnvelopeSignature {
        keyid: CANARY.into(),
        alg: "EdDSA".into(),
        sig: CANARY.into(),
    });

    let rendered = format!("{envelope:?}");
    assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    let wire = serde_json::to_string(&envelope).unwrap();
    let restored: UctpEnvelope = serde_json::from_str(&wire).unwrap();
    assert_eq!(restored.payload["credential"], CANARY);
    assert_eq!(envelope.payload["credential"], CANARY);
}

#[test]
fn credential_payloads_do_not_regain_derived_debug() {
    for (source, declaration) in [
        (
            include_str!("../src/payloads/auth.rs"),
            "pub struct AuthResponse",
        ),
        (
            include_str!("../src/payloads/auth.rs"),
            "pub struct AuthRefresh",
        ),
        (
            include_str!("../src/payloads/control.rs"),
            "pub struct IdentityStepUpResponse",
        ),
        (
            include_str!("../src/envelope.rs"),
            "pub struct UctpEnvelope",
        ),
    ] {
        let prefix = &source[..source.find(declaration).unwrap()];
        let attributes = prefix.rsplit("\n\n").next().unwrap_or_default();
        assert!(
            !attributes.contains("Debug"),
            "{declaration} regained derived Debug"
        );
    }
}

#[test]
fn coordinator_never_logs_raw_auth_provider_errors() {
    let source = include_str!("../src/state/coordinator.rs");
    assert!(!source.contains("warn!(error = %e, \"auth.bearer"));
    assert!(!source.contains("warn!(error = %e, \"auth.refresh"));
    assert!(!source.contains("warn!(%error, \"auth.bearer"));
    assert!(!source.contains("warn!(%error, \"auth.refresh"));
    assert!(source.contains("error_class = \"credential-validation\""));
    assert!(source.contains("error_class = \"resource-binding-authorization\""));
}
