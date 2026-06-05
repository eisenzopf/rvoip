//! Envelope-level tests for `rvoip-uctp` PR 2.
//!
//! Per `UCTP_IMPLEMENTATION_PLAN.md` §3.8:
//! - Round-trip a typed payload through JSON and back, preserving
//!   unknown extension fields.
//! - `{"type":"future.feature",...}` decodes to `MessageType::Unknown`.

use chrono::Utc;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{auth::AuthHello, session::SessionInvite},
    types::MessageType,
};
use serde_json::json;

#[test]
fn typed_payload_roundtrips_through_json() {
    let payload = SessionInvite {
        from: "part_alice".into(),
        to: vec!["part_bob".into()],
        medium: "voice".into(),
        intent: "synchronous-engagement".into(),
        capabilities_offer: json!({"audio_codecs": [{"name": "opus"}]}),
    };

    let env = UctpEnvelope::new(MessageType::SessionInvite, payload)
        .with_cid("conv_abc")
        .with_sid("sess_xyz");

    // Encode -> decode generic, then decode payload to typed struct.
    let wire = serde_json::to_string(&env).expect("encode");
    let decoded: UctpEnvelope<serde_json::Value> = serde_json::from_str(&wire).expect("decode");

    assert_eq!(decoded.v, 1);
    assert_eq!(decoded.msg_type, MessageType::SessionInvite);
    assert_eq!(decoded.cid.as_deref(), Some("conv_abc"));
    assert_eq!(decoded.sid.as_deref(), Some("sess_xyz"));
    assert!(decoded.connid.is_none());

    let typed: SessionInvite = decoded.decode_payload().expect("typed decode");
    assert_eq!(typed.from, "part_alice");
    assert_eq!(typed.to, vec!["part_bob"]);
}

#[test]
fn unknown_envelope_extension_fields_are_preserved() {
    // A future server might add a new top-level field (e.g., `trace_id`).
    // The envelope's typed `serde_json::Value` payload preserves
    // unknown payload fields automatically; we also verify that the
    // SessionInvite decoder tolerates new payload fields.
    let wire = json!({
        "v": 1,
        "type": "session.invite",
        "id": "env_abc",
        "ts": Utc::now(),
        "cid": "conv_x",
        "sid": "sess_y",
        "payload": {
            "from": "part_alice",
            "to": ["part_bob"],
            "medium": "voice",
            "intent": "synchronous-engagement",
            "capabilities_offer": {},
            "future_extension_field": {"new": "stuff"}
        }
    });

    let env: UctpEnvelope<serde_json::Value> = serde_json::from_value(wire).expect("decode");

    // Payload still contains the unknown extension field.
    assert!(env.payload.get("future_extension_field").is_some());

    // Typed decode succeeds; unknown fields are dropped silently.
    let typed: SessionInvite = env.decode_payload().expect("typed decode");
    assert_eq!(typed.from, "part_alice");
}

#[test]
fn unknown_message_type_decodes_to_unknown_variant() {
    let wire = json!({
        "v": 1,
        "type": "future.feature",
        "id": "env_abc",
        "ts": Utc::now(),
        "payload": {}
    });

    let env: UctpEnvelope<serde_json::Value> = serde_json::from_value(wire).expect("decode");

    match env.msg_type {
        MessageType::Unknown(s) => assert_eq!(s, "future.feature"),
        other => panic!("expected Unknown, got {:?}", other),
    }
}

#[test]
fn message_type_roundtrip_through_string() {
    // Every known variant must round-trip through its wire string.
    for mt in [
        MessageType::AuthHello,
        MessageType::SessionInvite,
        MessageType::ConnectionOffer,
        MessageType::StreamSubscribe,
        MessageType::IdentityStepUpRequest,
        MessageType::Error,
        MessageType::Ack,
    ] {
        let s = mt.as_wire_str().to_string();
        let parsed = MessageType::from_wire_str(&s);
        assert_eq!(parsed, mt, "round-trip failed for {}", s);
    }
}

#[test]
fn webrtc_substrate_setup_roundtrips() {
    use rvoip_uctp::payloads::connection::WebRtcSubstrateSetup;
    let setup = WebRtcSubstrateSetup::new(
        "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
    );
    assert_eq!(setup.kind, "websocket+webrtc");
    let json = serde_json::to_value(&setup).unwrap();
    assert_eq!(json["kind"], "websocket+webrtc");
    let back: WebRtcSubstrateSetup = serde_json::from_value(json).unwrap();
    assert_eq!(back.sdp, setup.sdp);
}

#[test]
fn auth_hello_payload_decodes() {
    let wire = json!({
        "v": 1,
        "type": "auth.hello",
        "id": "env_abc",
        "ts": Utc::now(),
        "payload": {
            "device": {
                "id": "dev_xyz",
                "kind": "desktop",
                "platform": "linux-x86_64",
                "sdk_version": "rvoip-client/0.1.0"
            },
            "auth_methods": ["bearer"],
            "capabilities": {}
        }
    });

    let env: UctpEnvelope<serde_json::Value> = serde_json::from_value(wire).expect("decode");
    let payload: AuthHello = env.decode_payload().expect("typed decode");

    assert_eq!(payload.device.kind, "desktop");
    assert_eq!(payload.auth_methods, vec!["bearer"]);
}
