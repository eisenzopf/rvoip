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

mod common;
use common::drive_auth_handshake;

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
        signature: None,
    }
}

fn auth_response_env(token: &str, in_reply_to: &str) -> UctpEnvelope {
    let payload = auth::AuthResponse {
        method: "bearer".into(),
        credential: token.into(),
        actor_token: None,
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
        signature: None,
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
        signature: None,
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
    in_tx
        .send(auth_response_env("", &challenge.id))
        .await
        .unwrap();

    let reply = out_rx.recv().await.expect("expected error");
    assert_eq!(reply.msg_type, MessageType::Error);
}

#[tokio::test]
async fn inbound_invite_emits_event() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    // A1: session.invite from an un-authed peer is refused with 401, so
    // every test that drives the wire-level lifecycle must complete the
    // auth handshake first.
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    // Drain the Authenticated event so it doesn't shadow the InboundInvite
    // we're about to assert.
    let _ = events_rx.recv().await;

    in_tx
        .send(session_invite_env("sess_x", "conv_y"))
        .await
        .unwrap();

    let event = events_rx.recv().await.expect("expected InboundInvite");
    match event {
        UctpSessionEvent::InboundInvite {
            from, to, medium, ..
        } => {
            assert_eq!(from, "part_alice");
            assert_eq!(to, vec!["part_bob".to_string()]);
            assert_eq!(medium, "voice");
        }
        other => panic!("expected InboundInvite, got {:?}", other),
    }
}

#[tokio::test]
async fn multi_party_stream_subscribe_rejected_with_501() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;

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
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected error envelope");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 501);
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
        signature: None,
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

    drive_auth_handshake(&in_tx, &mut out_rx).await;

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

    drive_auth_handshake(&in_tx, &mut out_rx).await;

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

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    // Drain the Authenticated event so it doesn't shadow the InboundInvite.
    let _ = events_rx.recv().await;

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
        envs.iter().any(
            |e| e.msg_type == MessageType::SessionEnd && e.sid.as_deref() == Some("sess_alpha")
        ),
        "expected synthesized session.end for sess_alpha, got {:?}",
        envs.iter()
            .map(|e| (&e.msg_type, &e.sid))
            .collect::<Vec<_>>()
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

    drive_auth_handshake(&in_tx, &mut out_rx).await;

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
        signature: None,
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
async fn session_invite_before_auth_emits_401() {
    // A1 regression: an un-authed peer sending session.invite must be
    // refused with `error 401 auth/unauthenticated`. No machine state
    // should be created.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let env = session_invite_env("sess_unauth", "conv_unauth");
    let env_id = env.id.clone();
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 401 error");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(env_id.as_str()));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
    assert_eq!(payload.category, "auth");
    assert_eq!(payload.reason, "unauthenticated");
}

#[tokio::test]
async fn connection_offer_before_auth_emits_401() {
    // Same gate covers the connection-level envelope catalog.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let env = connection_offer_env("sess_x", "conn_y", &["opus"]);
    let env_id = env.id.clone();
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 401 error");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(env_id.as_str()));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
    assert_eq!(payload.reason, "unauthenticated");
    // The 401 must carry the sid/connid so a misbehaving peer can map
    // it back to the offending envelope without a separate correlator.
    assert_eq!(reply.connid.as_deref(), Some("conn_y"));
}

#[tokio::test]
async fn auth_handshake_unlocks_subsequent_envelopes() {
    // After auth completes, the same peer can drive session/connection
    // envelopes without hitting the gate. This is the inverse of
    // `session_invite_before_auth_emits_401`.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    // Drain the Authenticated event so we can assert InboundInvite cleanly.
    let _ = events_rx.recv().await;

    in_tx
        .send(session_invite_env("sess_unlocked", "conv_unlocked"))
        .await
        .unwrap();

    let event = events_rx
        .recv()
        .await
        .expect("expected InboundInvite after auth");
    matches!(event, UctpSessionEvent::InboundInvite { .. });

    // No 401 should land on out_rx.
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    if let Ok(envelope) = out_rx.try_recv() {
        if envelope.msg_type == MessageType::Error {
            let payload: rvoip_uctp::payloads::control::Error = envelope.decode_payload().unwrap();
            assert_ne!(payload.code, 401, "post-auth envelope must not produce 401");
        }
    }
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
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    // Give the coordinator a moment to process.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(out_rx.try_recv().is_err());
}

#[tokio::test]
async fn session_invite_over_cap_emits_429() {
    // D1: per-peer session cap. With max_sessions_per_peer = 2, a third
    // distinct session.invite must be refused with `error 429
    // rate-limit/too-many-sessions`. The two existing sessions stay
    // open.
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let caps = UctpCoordinatorCaps {
        max_sessions_per_peer: 2,
        ..Default::default()
    };
    let _coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        Arc::new(rvoip_uctp::state::default_v0_descriptor()),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // drain Authenticated

    // First two invites: accepted.
    in_tx
        .send(session_invite_env("sess_one", "conv_one"))
        .await
        .unwrap();
    let _ = events_rx.recv().await; // InboundInvite
    in_tx
        .send(session_invite_env("sess_two", "conv_two"))
        .await
        .unwrap();
    let _ = events_rx.recv().await; // InboundInvite

    // Third invite: over cap → 429.
    let third = session_invite_env("sess_three", "conv_three");
    let third_id = third.id.clone();
    in_tx.send(third).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 429 error");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(third_id.as_str()));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 429);
    assert_eq!(payload.category, "rate-limit");
    assert_eq!(payload.reason, "too-many-sessions");
    // sid is echoed so the peer can pin the failure to the right
    // invite without parsing the in_reply_to id.
    assert_eq!(reply.sid.as_deref(), Some("sess_three"));
}

#[tokio::test]
async fn session_invite_retransmit_does_not_count_against_cap() {
    // Idempotency check for the D1 cap. A retransmit of an *existing*
    // session.invite must still be accepted even if the cap is full —
    // otherwise legitimate §7.2 retransmits during normal lifecycle
    // would fail in caps-saturated coordinators.
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let caps = UctpCoordinatorCaps {
        max_sessions_per_peer: 1,
        ..Default::default()
    };
    let _coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        Arc::new(rvoip_uctp::state::default_v0_descriptor()),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;

    in_tx
        .send(session_invite_env("sess_dup", "conv_dup"))
        .await
        .unwrap();
    let _ = events_rx.recv().await;

    // Retransmit of the same sid — must pass the cap check.
    in_tx
        .send(session_invite_env("sess_dup", "conv_dup"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // No 429 should appear on out_rx.
    if let Ok(env) = out_rx.try_recv() {
        if env.msg_type == MessageType::Error {
            let payload: rvoip_uctp::payloads::control::Error = env.decode_payload().unwrap();
            assert_ne!(
                payload.code, 429,
                "retransmit of an existing sid must not be refused with 429"
            );
        }
    }
}

#[tokio::test]
async fn inbound_dtmf_send_emits_session_event() {
    // C2: a peer sending `dtmf.send` on an established Connection
    // produces `UctpSessionEvent::Dtmf` so the adapter can translate
    // to `AdapterEvent::Dtmf` and the orchestrator surfaces it as
    // `Event::DtmfReceived`.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let descriptor = answerer_with(&["opus"]);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // Authenticated

    // Bring up a Connection via a successful offer.
    in_tx
        .send(connection_offer_env("sess_dtmf", "conn_dtmf", &["opus"]))
        .await
        .unwrap();
    // Brief settle so the ConnectionMachine is created.
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    // Send dtmf.send.
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::DtmfSend,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_dtmf".into()),
        connid: Some("conn_dtmf".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "digits": "9*",
            "duration_ms": 120,
            "method": "rfc4733"
        }),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    // Drain the events channel looking for Dtmf.
    let mut saw_dtmf = false;
    for _ in 0..10 {
        if let Ok(Some(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(100), events_rx.recv()).await
        {
            if let UctpSessionEvent::Dtmf {
                digits,
                duration_ms,
                method,
                ..
            } = ev
            {
                assert_eq!(digits, "9*");
                assert_eq!(duration_ms, 120);
                assert_eq!(method, "rfc4733");
                saw_dtmf = true;
                break;
            }
        }
    }
    assert!(saw_dtmf, "expected UctpSessionEvent::Dtmf after dtmf.send");
}

#[tokio::test]
async fn inbound_connection_quality_emits_per_stream_events() {
    // C2: a peer sending `connection.quality` with multiple streams
    // produces one `UctpSessionEvent::Quality` per stream entry, so
    // the orchestrator's `Event::MediaQuality` consumer sees per-
    // Stream data even though the orchestrator-level event is
    // per-Connection.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let descriptor = answerer_with(&["opus"]);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // Authenticated

    in_tx
        .send(connection_offer_env("sess_q", "conn_q", &["opus"]))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    // connection.quality envelope reporting two streams.
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionQuality,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_q".into()),
        connid: Some("conn_q".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "interval_ms": 5000,
            "streams": [
                {
                    "strm_id": "strm_audio",
                    "loss_pct": 0.5,
                    "jitter_ms": 12,
                    "rtt_ms": 80,
                    "mos": 4.2,
                    "bitrate_bps": 32_000,
                    "packets_sent": 250,
                    "packets_received": 248
                },
                {
                    "strm_id": "strm_video",
                    "loss_pct": 1.2,
                    "jitter_ms": 35,
                    "rtt_ms": 80,
                    "mos": 3.6,
                    "bitrate_bps": 512_000,
                    "packets_sent": 1000,
                    "packets_received": 988
                }
            ]
        }),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let mut quality_events: Vec<(String, f32, f32)> = Vec::new();
    for _ in 0..10 {
        if let Ok(Some(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(100), events_rx.recv()).await
        {
            if let UctpSessionEvent::Quality {
                strm_id, snapshot, ..
            } = ev
            {
                quality_events.push((
                    strm_id,
                    snapshot.packet_loss_pct,
                    snapshot.mos.unwrap_or(-1.0),
                ));
                if quality_events.len() == 2 {
                    break;
                }
            }
        }
    }
    assert_eq!(quality_events.len(), 2, "expected one event per stream");

    let audio = quality_events
        .iter()
        .find(|(s, _, _)| s == "strm_audio")
        .expect("audio quality");
    assert!((audio.1 - 0.5).abs() < 0.001, "loss_pct passthrough");
    assert!((audio.2 - 4.2).abs() < 0.001, "mos passthrough");

    let video = quality_events
        .iter()
        .find(|(s, _, _)| s == "strm_video")
        .expect("video quality");
    assert!((video.2 - 3.6).abs() < 0.001, "video mos passthrough");
}

#[tokio::test]
async fn auth_refresh_updates_session_with_fresh_token() {
    // D4: a peer that already authenticated sends `auth.refresh` with
    // a new (valid, non-empty stub) token. Coordinator validates,
    // updates `PeerAuthState`, and replies with a fresh `auth.session`
    // envelope. The original identity_id / participant_id are
    // preserved across the refresh (continuity of logical session).
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    // Initial auth handshake.
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    // Capture identity_id from the initial Authenticated event.
    let initial_identity = match events_rx.recv().await.expect("Authenticated") {
        UctpSessionEvent::Authenticated {
            identity_id,
            participant_id,
            ..
        } => (identity_id, participant_id),
        other => panic!("expected Authenticated, got {:?}", other),
    };

    // Send auth.refresh with a different (still valid stub) token.
    let refresh_env = UctpEnvelope::new(
        MessageType::AuthRefresh,
        serde_json::to_value(auth::AuthRefresh {
            method: "bearer".into(),
            credential: "refreshed-token-xyz".into(),
        })
        .unwrap(),
    );
    let refresh_id = refresh_env.id.clone();
    in_tx.send(refresh_env).await.unwrap();

    // Expect a fresh auth.session reply.
    let reply = out_rx.recv().await.expect("expected auth.session reply");
    assert_eq!(reply.msg_type, MessageType::AuthSession);
    assert_eq!(reply.in_reply_to.as_deref(), Some(refresh_id.as_str()));
    let payload: auth::AuthSession = reply.decode_payload().unwrap();
    // Continuity: identity_id / participant_id preserved across the
    // refresh — clients can keep using their existing logical
    // bindings without rebinding consumers.
    assert_eq!(payload.identity_id, initial_identity.0);
    assert_eq!(payload.participant_id, initial_identity.1);
    // But the session token is fresh.
    assert!(payload.session_token.starts_with("tok_"));

    // Authenticated event re-emitted with the same identity ids.
    let post_refresh = events_rx.recv().await.expect("post-refresh Authenticated");
    match post_refresh {
        UctpSessionEvent::Authenticated {
            identity_id,
            participant_id,
            ..
        } => {
            assert_eq!(identity_id, initial_identity.0);
            assert_eq!(participant_id, initial_identity.1);
        }
        other => panic!("expected Authenticated, got {:?}", other),
    }
}

#[tokio::test]
async fn auth_refresh_with_empty_token_emits_401_and_preserves_session() {
    // D4: a refresh with an invalid (empty) token must not revoke the
    // existing session. The peer gets a 401 with reason
    // `refresh-failed` so it can distinguish from an initial-auth
    // 401, but its current PeerAuthState stays valid — sending a
    // session/connection envelope right after still succeeds.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // initial Authenticated

    // Bad refresh — bearer_stub rejects empty token with Empty.
    let refresh_env = UctpEnvelope::new(
        MessageType::AuthRefresh,
        serde_json::to_value(auth::AuthRefresh {
            method: "bearer".into(),
            credential: String::new(),
        })
        .unwrap(),
    );
    let refresh_id = refresh_env.id.clone();
    in_tx.send(refresh_env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 401");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(refresh_id.as_str()));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
    assert_eq!(payload.category, "auth");
    assert_eq!(
        payload.reason, "refresh-failed",
        "must distinguish from initial-auth bearer-validation-failed"
    );

    // Session continuity: a session.invite after the failed refresh
    // should still get past the auth gate (it would 401 with reason
    // `unauthenticated` if the refresh had revoked auth).
    in_tx
        .send(session_invite_env("sess_post_refresh", "conv_post_refresh"))
        .await
        .unwrap();
    let event = events_rx.recv().await.expect("post-refresh InboundInvite");
    assert!(
        matches!(event, UctpSessionEvent::InboundInvite { .. }),
        "failed refresh must not have revoked the existing session"
    );
}

#[tokio::test]
async fn dtmf_send_for_unknown_connid_emits_404() {
    // Symmetric with the other connection-scoped handlers: a `dtmf.send`
    // pointing at a connid the coordinator never saw produces
    // `error 404 not-found/unknown-connid`.
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    drive_auth_handshake(&in_tx, &mut out_rx).await;

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::DtmfSend,
        id: "env_dtmf_orphan".into(),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_ghost".into()),
        connid: Some("conn_ghost".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "digits": "1",
            "duration_ms": 100,
            "method": "rfc4733"
        }),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected 404");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 404);
    assert_eq!(payload.reason, "unknown-connid");
}
