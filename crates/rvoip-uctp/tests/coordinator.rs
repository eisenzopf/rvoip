//! End-to-end coordinator tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8 / §3.5.
//!
//! Drives the coordinator with synthetic inbound envelopes and asserts
//! both outbound envelopes and emitted [`UctpSessionEvent`]s.

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{auth, connection, session},
    state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use std::sync::Arc;
use tokio::sync::mpsc;

fn auth_hello_env() -> UctpEnvelope {
    let payload = auth::AuthHello {
        device: auth::Device {
            id: "dev_x".into(),
            kind: "desktop".into(),
            platform: "linux-x86_64".into(),
            sdk_version: "rvoip-test/0.1".into(),
        },
        auth_methods: vec!["bearer".into()],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
    }
}

fn auth_response_env(token: &str, in_reply_to: &str) -> UctpEnvelope {
    let payload = auth::AuthResponse {
        method: "bearer".into(),
        credential: token.into(),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(in_reply_to.into()),
        payload: serde_json::to_value(payload).unwrap(),
    }
}

fn session_invite_env(sid: &str, cid: &str) -> UctpEnvelope {
    let payload = session::SessionInvite {
        from: "part_alice".into(),
        to: vec!["part_bob".into()],
        medium: "voice".into(),
        intent: "synchronous-engagement".into(),
        capabilities_offer: serde_json::Value::Object(Default::default()),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: Some(cid.into()),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
    }
}

#[tokio::test]
async fn auth_hello_produces_challenge() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    in_tx.send(auth_hello_env()).await.unwrap();

    let reply = out_rx.recv().await.expect("expected challenge");
    assert_eq!(reply.msg_type, MessageType::AuthChallenge);
    assert!(reply.in_reply_to.is_some());
}

#[tokio::test]
async fn auth_response_with_nonempty_token_yields_auth_session() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.unwrap();

    in_tx
        .send(auth_response_env("any-non-empty", &challenge.id))
        .await
        .unwrap();

    let reply = out_rx.recv().await.expect("expected auth.session");
    assert_eq!(reply.msg_type, MessageType::AuthSession);

    let event = events_rx.recv().await.expect("expected Authenticated");
    matches!(event, UctpSessionEvent::Authenticated { .. });
}

#[tokio::test]
async fn auth_response_with_empty_token_yields_401_error() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.unwrap();
    in_tx.send(auth_response_env("", &challenge.id)).await.unwrap();

    let reply = out_rx.recv().await.expect("expected error");
    assert_eq!(reply.msg_type, MessageType::Error);
}

#[tokio::test]
async fn inbound_invite_emits_event() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, _out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    in_tx
        .send(session_invite_env("sess_x", "conv_y"))
        .await
        .unwrap();

    let event = events_rx.recv().await.expect("expected InboundInvite");
    match event {
        UctpSessionEvent::InboundInvite { from, to, medium, .. } => {
            assert_eq!(from, "part_alice");
            assert_eq!(to, vec!["part_bob".to_string()]);
            assert_eq!(medium, "voice");
        }
        other => panic!("expected InboundInvite, got {:?}", other),
    }
}

#[tokio::test]
async fn multi_party_stream_subscribe_rejected_with_503() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::StreamSubscribe,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_x".into()),
        connid: Some("conn_y".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "by_participant": "part_alice",
            "subscriptions": [{"strm_id": "strm_z"}]
        }),
    };
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected error envelope");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 503);
    assert_eq!(payload.reason, "multi-party-routing-not-implemented");
}

fn connection_offer_env(sid: &str, connid: &str, prefs: &[&str]) -> UctpEnvelope {
    let payload = connection::ConnectionOffer {
        by_participant: "part_alice".into(),
        substrate: "quic".into(),
        capabilities: serde_json::Value::Object(Default::default()),
        streams_offered: vec![connection::StreamOffer {
            id: "strm_1".into(),
            kind: "audio".into(),
            direction: "sendrecv".into(),
            codec_preferences: prefs.iter().map(|s| (*s).to_string()).collect(),
        }],
        substrate_setup: serde_json::Value::Null,
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionOffer,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
    }
}

fn answerer_with(codecs: &[&str]) -> Arc<CapabilityDescriptor> {
    Arc::new(CapabilityDescriptor {
        audio_codecs: codecs
            .iter()
            .map(|s| CodecInfo {
                name: (*s).to_string(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            })
            .collect(),
        ..Default::default()
    })
}

#[tokio::test]
async fn connection_offer_with_disjoint_codecs_emits_488() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    // Local descriptor supports only opus; peer offers only g.722 → 488.
    let descriptor = answerer_with(&["opus"]);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor,
    );

    let env = connection_offer_env("sess_x", "conn_y", &["g.722", "g.711-mu"]);
    let offer_id = env.id.clone();
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected error envelope");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(offer_id.as_str()));
    assert_eq!(reply.connid.as_deref(), Some("conn_y"));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 488);
    assert_eq!(payload.category, "capability");
    assert_eq!(payload.reason, "incompatible-capabilities");
}

#[tokio::test]
async fn connection_offer_with_overlapping_codecs_is_accepted() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let descriptor = answerer_with(&["opus", "g.711-mu"]);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor,
    );

    let env = connection_offer_env("sess_x", "conn_y", &["opus"]);
    in_tx.send(env).await.unwrap();

    // No outbound envelope should arrive — accepting is silent in v0
    // (the spec doesn't mandate an immediate ack for connection.offer).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(out_rx.try_recv().is_err());
}

#[tokio::test]
async fn shutdown_emits_session_end_for_inflight_sessions() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    // Bring a session into the Inviting state.
    in_tx
        .send(session_invite_env("sess_alpha", "conv_alpha"))
        .await
        .unwrap();
    // Drain the InboundInvite event so the queue is clean.
    let _ = events_rx.recv().await;

    coord.shutdown().await;

    // Expect a synthesized session.end on the outbound channel.
    let envs: Vec<UctpEnvelope> = std::iter::from_fn(|| out_rx.try_recv().ok())
        .take(8)
        .collect();
    assert!(
        envs.iter()
            .any(|e| e.msg_type == MessageType::SessionEnd
                && e.sid.as_deref() == Some("sess_alpha")),
        "expected synthesized session.end for sess_alpha, got {:?}",
        envs.iter().map(|e| (&e.msg_type, &e.sid)).collect::<Vec<_>>()
    );

    // And a terminal UctpSessionEvent::SessionEnded.
    let mut saw_ended = false;
    while let Ok(ev) = events_rx.try_recv() {
        if let UctpSessionEvent::SessionEnded { sid, reason } = ev {
            assert_eq!(sid.to_string(), "sess_alpha");
            assert_eq!(reason, "shutdown");
            saw_ended = true;
        }
    }
    assert!(saw_ended, "expected SessionEnded event on shutdown");
}

#[tokio::test]
async fn envelope_for_unknown_connid_emits_404() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    // connection.answer for a connid the coordinator never saw.
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionAnswer,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: Some("conn_ghost".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "by_participant": "part_alice",
            "substrate": "quic",
            "capabilities": {},
            "streams_answered": [],
            "substrate_setup": null
        }),
    };
    let env_id = env.id.clone();
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 404 error");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(env_id.as_str()));
    assert_eq!(reply.connid.as_deref(), Some("conn_ghost"));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 404);
    assert_eq!(payload.category, "not-found");
    assert_eq!(payload.reason, "unknown-connid");
}

#[tokio::test]
async fn unknown_envelope_types_are_silently_ignored() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::Unknown("future.feature".into()),
        id: "env_x".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::Value::Object(Default::default()),
    };
    in_tx.send(env).await.unwrap();

    // Give the coordinator a moment to process.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(out_rx.try_recv().is_err());
}
