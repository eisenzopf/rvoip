use rvoip_auth_core::EnvelopeSignature;
use rvoip_uctp::adapter_helpers::ConnectionBindingError;
use rvoip_uctp::payloads::auth::{AuthChallenge, AuthRefresh, AuthResponse};
use rvoip_uctp::payloads::capability::CapabilityAdvertise;
use rvoip_uctp::payloads::connection::{ConnectionOffer, WebRtcSubstrateSetup};
use rvoip_uctp::payloads::control::IdentityStepUpResponse;
use rvoip_uctp::payloads::conversation::{ConversationCreate, ConversationPolicy};
use rvoip_uctp::payloads::message::{Attachment, BodyEncoding, MessageSend};
use rvoip_uctp::payloads::session::SessionInvite;
use rvoip_uctp::payloads::stream::StreamInfo;
use rvoip_uctp::state::{ResourceBindingError, SubscriptionOutcome, UctpScopePolicy};
use rvoip_uctp::substrate::{
    MediaDatagram, PeerMediaRouteKey, PeerMediaRouterError, RtpDatagram, RtpMediaPayload,
};
use rvoip_uctp::{CorrelationIdDiagnostic, MessageType, SubstrateError, UctpEnvelope, UctpError};

const CANARY: &str = "uctp-credential-malicious-canary\r\nAuthorization: exposed";

#[test]
fn direct_auth_and_control_payloads_redact_but_serialize_exactly() {
    let response = AuthResponse {
        method: CANARY.into(),
        credential: CANARY.into(),
        actor_token: Some(CANARY.into()),
    };
    let refresh = AuthRefresh {
        method: CANARY.into(),
        credential: CANARY.into(),
        actor_token: Some(CANARY.into()),
    };
    let challenge = AuthChallenge {
        nonce: CANARY.into(),
        accepted_methods: vec![CANARY.into()],
        server_capabilities: serde_json::json!({"proof": CANARY}),
    };
    let step_up = IdentityStepUpResponse {
        method: CANARY.into(),
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
    assert_eq!(response.method, CANARY);
    assert_eq!(refresh.credential, CANARY);
    assert_eq!(refresh.method, CANARY);
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
    assert!(!source.contains("asserted_participant = %"));
    assert!(!source.contains("authenticated_participant = %"));
    assert!(!source.contains("reason = %error.reason"));
    assert!(!source.contains("method = %payload.method"));
    assert_eq!(
        source
            .matches("method_class = auth_method_diagnostic_class(&payload.method)")
            .count(),
        2,
        "both auth.response and auth.refresh spans must classify method values"
    );
    assert_eq!(
        source
            .matches("method_bytes = payload.method.len()")
            .count(),
        2
    );
    assert!(source.contains("error_class = \"credential-validation\""));
    assert!(source.contains("error_class = \"resource-binding-authorization\""));
}

#[test]
fn every_uctp_substrate_uses_metadata_only_correlation_diagnostics() {
    let rendered = format!("{:?}", CorrelationIdDiagnostic::new(CANARY));
    assert!(!rendered.contains(CANARY));
    assert!(!rendered.contains("Authorization: exposed"));
    assert!(rendered.contains("present: true"));
    assert!(rendered.contains(&format!("bytes: {}", CANARY.len())));

    let sources = [
        (
            "rvoip-uctp coordinator",
            include_str!("../src/state/coordinator.rs"),
        ),
        (
            "rvoip-quic server",
            include_str!("../../rvoip-quic/src/server.rs"),
        ),
        (
            "rvoip-quic adapter",
            include_str!("../../rvoip-quic/src/adapter.rs"),
        ),
        (
            "rvoip-quic media stream",
            include_str!("../../rvoip-quic/src/media_stream.rs"),
        ),
        (
            "rvoip-webtransport server",
            include_str!("../../rvoip-webtransport/src/server.rs"),
        ),
        (
            "rvoip-webtransport adapter",
            include_str!("../../rvoip-webtransport/src/adapter.rs"),
        ),
        (
            "rvoip-webtransport media stream",
            include_str!("../../rvoip-webtransport/src/media_stream.rs"),
        ),
        (
            "rvoip-websocket server",
            include_str!("../../rvoip-websocket/src/server.rs"),
        ),
        (
            "rvoip-websocket adapter",
            include_str!("../../rvoip-websocket/src/adapter.rs"),
        ),
    ];
    let forbidden = [
        "%sid",
        "?sid",
        "%conn",
        "?conn",
        "%core_connection_id",
        "?core_connection_id",
        "%existing",
        "?existing",
        "%stream_id",
        "?stream_id",
        "?env.sid",
        "?env.connid",
    ];

    for (name, source) in sources {
        for fragment in forbidden {
            assert!(
                !source.contains(fragment),
                "{name} regained raw correlation diagnostic fragment {fragment:?}"
            );
        }
        assert!(
            source.contains("CorrelationIdDiagnostic::"),
            "{name} must keep correlation fields behind the metadata-only wrapper"
        );
    }
}

#[test]
fn outer_payload_and_resource_error_diagnostics_are_metadata_only() {
    let session = SessionInvite {
        from: CANARY.into(),
        to: vec![CANARY.into()],
        medium: CANARY.into(),
        intent: CANARY.into(),
        capabilities_offer: serde_json::json!({"credential": CANARY}),
    };
    let connection = ConnectionOffer {
        by_participant: CANARY.into(),
        substrate: CANARY.into(),
        capabilities: serde_json::json!({"credential": CANARY}),
        streams_offered: Vec::new(),
        substrate_setup: serde_json::json!({"sdp": CANARY}),
    };
    let webrtc = WebRtcSubstrateSetup::new(CANARY);
    let conversation = ConversationCreate {
        tenant_id: CANARY.into(),
        policy: ConversationPolicy::Ephemeral,
        idle_close_secs: Some(30),
        metadata: serde_json::json!({"credential": CANARY}),
        initial_participants: Vec::new(),
    };
    let message = MessageSend {
        msg_id: CANARY.into(),
        from: CANARY.into(),
        to: serde_json::json!([CANARY]),
        content_type: CANARY.into(),
        label: CANARY.into(),
        reliability: rvoip_core::DataReliability::ReliableOrdered,
        body: CANARY.into(),
        body_encoding: BodyEncoding::Utf8,
        attachments: vec![Attachment {
            id: CANARY.into(),
            content_type: CANARY.into(),
            url: Some(CANARY.into()),
            size_bytes: 1,
        }],
        in_reply_to_msg: Some(CANARY.into()),
    };
    let stream = StreamInfo {
        strm_id: CANARY.into(),
        kind: CANARY.into(),
        codec: serde_json::json!({"credential": CANARY}),
        direction: CANARY.into(),
        stream_local_id: 1,
        opened_at: chrono::Utc::now(),
    };
    let capability = CapabilityAdvertise {
        by_participant: CANARY.into(),
        capabilities: serde_json::json!({"credential": CANARY}),
        trigger: CANARY.into(),
    };
    let binding = ResourceBindingError::forbidden(CANARY);
    let outcome = SubscriptionOutcome::reject(403, CANARY);

    for rendered in [
        format!("{session:?}"),
        format!("{connection:?}"),
        format!("{webrtc:?}"),
        format!("{conversation:?}"),
        format!("{message:?}"),
        format!("{stream:?}"),
        format!("{capability:?}"),
        format!("{binding:?} {binding}"),
        format!("{outcome:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
    }

    assert_eq!(session.from, CANARY);
    assert_eq!(webrtc.sdp, CANARY);
    assert_eq!(message.body, CANARY);
    assert_eq!(binding.reason, CANARY);
}

#[test]
fn public_payload_structs_do_not_regain_derived_debug() {
    for source in [
        include_str!("../src/payloads/capability.rs"),
        include_str!("../src/payloads/connection.rs"),
        include_str!("../src/payloads/conversation.rs"),
        include_str!("../src/payloads/message.rs"),
        include_str!("../src/payloads/session.rs"),
        include_str!("../src/payloads/stream.rs"),
    ] {
        let lines = source.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("pub struct ") {
                continue;
            }
            let attributes = lines[index.saturating_sub(4)..index].join("\n");
            assert!(
                !attributes.contains("Debug"),
                "public payload struct regained derived Debug: {line}"
            );
        }
    }
}

#[test]
fn identifiers_media_routes_and_datagrams_do_not_expose_retained_values() {
    let connection = rvoip_uctp::ConnectionId::from_string(CANARY);
    let attempted = rvoip_uctp::ConnectionId::from_string(format!("attempted-{CANARY}"));
    let binding_error = ConnectionBindingError::AlreadyBound {
        existing: connection.clone(),
        attempted,
    };
    let route = PeerMediaRouteKey::new(
        rvoip_uctp::SessionId::from_string(CANARY),
        connection,
        rvoip_core::StreamId::from_string(CANARY),
    );
    let route_error = PeerMediaRouterError::DuplicateRoute {
        route: route.clone(),
        existing_local_id: std::num::NonZeroU16::new(7).unwrap(),
    };
    let payload = bytes::Bytes::copy_from_slice(CANARY.as_bytes());
    let rtp = RtpMediaPayload {
        payload: payload.clone(),
        payload_type: 111,
        sequence_number: 9,
        timestamp: 10,
        ssrc: 11,
    };
    let datagram = RtpDatagram {
        flags: 0,
        stream_local_id: 7,
        seq: 12,
        rtp,
    };
    let raw = MediaDatagram {
        flags: 0,
        stream_local_id: 7,
        seq: 12,
        payload,
    };
    let envelope_id = rvoip_uctp::EnvelopeId::from_string(CANARY);
    let unknown = MessageType::Unknown(CANARY.into());

    for rendered in [
        format!("{binding_error:?} {binding_error}"),
        format!("{route:?}"),
        format!("{route_error:?} {route_error}"),
        format!("{datagram:?}"),
        format!("{raw:?}"),
        format!("{envelope_id:?}"),
        format!("{unknown:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "state leaked: {rendered}");
    }
}

#[test]
fn policy_and_protocol_errors_redact_arbitrary_boundary_values() {
    let policy = UctpScopePolicy::secure_defaults();
    let unknown = UctpError::UnknownEnvelopeType(CANARY.into());
    let auth = UctpError::Auth(rvoip_auth_core::BearerAuthError::Invalid(CANARY.into()));
    let transport = SubstrateError::Io(std::io::Error::other(CANARY));

    for rendered in [
        format!("{policy:?}"),
        format!("{unknown:?} {unknown}"),
        format!("{auth:?} {auth}"),
        format!("{transport:?} {transport}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    match unknown {
        UctpError::UnknownEnvelopeType(value) => assert_eq!(value, CANARY),
        other => panic!("unexpected error variant: {other:?}"),
    }
}
