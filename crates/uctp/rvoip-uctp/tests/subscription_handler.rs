//! v0.x MP2 — end-to-end test of SubscriptionHandler wired into the
//! UctpCoordinator with the concrete OrchestratorSubscriptionHandler.
//!
//! Validates:
//! - Default (no handler) → 503 multi-party-not-implemented (back-compat).
//! - With handler + registered publisher → `stream.subscribe` produces
//!   `ack` and the orchestrator's subscription registry holds the row.
//! - Unknown strm_id → 404.
//! - `stream.unsubscribe` is idempotent and removes the row.

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::config::Config;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::subscriptions::{PublisherEntry, PublisherRegistry};
use rvoip_core::Orchestrator;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{
        connection::{ConnectionOffer, StreamOffer},
        stream::{StreamSubscribe, StreamSubscription, StreamUnsubscribe},
    },
    state::{OrchestratorSubscriptionHandler, UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

mod common;
use common::drive_auth_handshake;

fn descriptor_with_opus() -> Arc<CapabilityDescriptor> {
    Arc::new(CapabilityDescriptor {
        audio_codecs: vec![rvoip_core::capability::CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }],
        ..Default::default()
    })
}

fn invite_env(sid: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: Some(format!("conv_{sid}")),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::json!({
            "from": "part_subscriber",
            "to": ["part_remote"],
            "medium": "voice",
            "intent": "synchronous-engagement",
            "capabilities_offer": {}
        }),
        signature: None,
    }
}

fn subscriber_offer_env(sid: &str, connid: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionOffer,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::to_value(ConnectionOffer {
            by_participant: "part_subscriber".into(),
            substrate: "quic".into(),
            capabilities: serde_json::Value::Object(Default::default()),
            streams_offered: vec![StreamOffer {
                id: format!("strm_fixture_{connid}"),
                kind: "audio".into(),
                direction: "recvonly".into(),
                codec_preferences: vec!["opus".into()],
            }],
            substrate_setup: serde_json::Value::Null,
        })
        .unwrap(),
        signature: None,
    }
}

fn ready_env(sid: &str, connid: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionReady,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::Value::Object(Default::default()),
        signature: None,
    }
}

async fn establish_subscriber_connection(
    in_tx: &mpsc::Sender<UctpEnvelope>,
    out_rx: &mut mpsc::Receiver<UctpEnvelope>,
    sid: &str,
    connid: &str,
) {
    in_tx
        .send(invite_env(sid))
        .await
        .expect("send session invite");
    in_tx
        .send(subscriber_offer_env(sid, connid))
        .await
        .expect("send subscriber connection offer");
    in_tx
        .send(ready_env(sid, connid))
        .await
        .expect("send subscriber connection ready");

    let opened = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
        .await
        .expect("subscriber stream.opened timeout")
        .expect("coordinator output closed");
    assert_eq!(opened.msg_type, MessageType::StreamOpened);
    assert_eq!(opened.sid.as_deref(), Some(sid));
    assert_eq!(opened.connid.as_deref(), Some(connid));
}

fn subscribe_env(sid: &str, connid: &str, strm_ids: &[&str]) -> UctpEnvelope {
    let payload = StreamSubscribe {
        by_participant: "part_subscriber".into(),
        subscriptions: strm_ids
            .iter()
            .map(|s| StreamSubscription {
                strm_id: Some((*s).to_string()),
                from_participant: None,
                kinds: Vec::new(),
            })
            .collect(),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::StreamSubscribe,
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

fn unsubscribe_env(sid: &str, connid: &str, strm_ids: &[&str]) -> UctpEnvelope {
    let payload = StreamUnsubscribe {
        strm_ids: strm_ids.iter().map(|s| (*s).to_string()).collect(),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::StreamUnsubscribe,
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

#[tokio::test]
async fn subscribe_with_registered_publisher_emits_ack_and_records_row() {
    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());

    // Pre-register the publisher of strm_x in sess_a as conn_publisher.
    let sid = SessionId::from_string("sess_a");
    let publisher_connid = ConnectionId::from_string("conn_publisher");
    publishers.register(
        sid.clone(),
        "strm_x".into(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "part_publisher".into(),
            kind: "audio".into(),
            codec: None,
        },
    );

    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), publishers);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_a", "conn_subscriber").await;

    in_tx
        .send(subscribe_env("sess_a", "conn_subscriber", &["strm_x"]))
        .await
        .unwrap();

    let reply = out_rx.recv().await.expect("expected ack");
    assert_eq!(reply.msg_type, MessageType::Ack);

    let subscriber_connid = ConnectionId::from_string("conn_subscriber");
    let strm = StreamId::from_string("strm_x");
    let subs = orch.subscribers_for(&sid, &publisher_connid, &strm);
    assert_eq!(subs, vec![subscriber_connid]);
}

#[tokio::test]
async fn subscribe_with_unknown_strm_id_emits_404() {
    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());
    let handler = OrchestratorSubscriptionHandler::new(orch, publishers);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_a", "conn_subscriber").await;

    in_tx
        .send(subscribe_env(
            "sess_a",
            "conn_subscriber",
            &["strm_does_not_exist"],
        ))
        .await
        .unwrap();

    let reply = out_rx.recv().await.expect("expected error");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 404);
    assert!(payload.reason.contains("strm_does_not_exist"));
}

#[tokio::test]
async fn unsubscribe_is_idempotent_and_removes_row() {
    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());

    let sid = SessionId::from_string("sess_a");
    let publisher_connid = ConnectionId::from_string("conn_publisher");
    publishers.register(
        sid.clone(),
        "strm_x".into(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "part_publisher".into(),
            kind: "audio".into(),
            codec: None,
        },
    );

    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), publishers);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_a", "conn_subscriber").await;

    // Subscribe first.
    in_tx
        .send(subscribe_env("sess_a", "conn_subscriber", &["strm_x"]))
        .await
        .unwrap();
    let ack = out_rx.recv().await.expect("subscribe ack");
    assert_eq!(ack.msg_type, MessageType::Ack);

    // Unsubscribe — expect ack and row gone.
    in_tx
        .send(unsubscribe_env("sess_a", "conn_subscriber", &["strm_x"]))
        .await
        .unwrap();
    let ack = out_rx.recv().await.expect("unsubscribe ack");
    assert_eq!(ack.msg_type, MessageType::Ack);

    let strm = StreamId::from_string("strm_x");
    let subs = orch.subscribers_for(&sid, &publisher_connid, &strm);
    assert!(subs.is_empty(), "subscriber should be removed");

    // Repeat unsubscribe — idempotent, still ack.
    in_tx
        .send(unsubscribe_env("sess_a", "conn_subscriber", &["strm_x"]))
        .await
        .unwrap();
    let ack = out_rx.recv().await.expect("second unsubscribe ack");
    assert_eq!(ack.msg_type, MessageType::Ack);
}

// Helper: spin up a coordinator + handler with a pre-populated
// publisher registry covering Alice's audio and video streams. The
// coordinator is already past the auth handshake when this returns —
// callers can dispatch session/stream envelopes directly.
async fn setup_with_alice_streams() -> (
    Arc<Orchestrator>,
    Arc<PublisherRegistry>,
    tokio::sync::mpsc::Sender<UctpEnvelope>,
    tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());

    let sid = SessionId::from_string("sess_x");
    let alice_conn = ConnectionId::from_string("conn_alice");
    publishers.register(
        sid.clone(),
        "strm_alice_audio".into(),
        PublisherEntry {
            connection: alice_conn.clone(),
            participant: "part_alice".into(),
            kind: "audio".into(),
            codec: None,
        },
    );
    publishers.register(
        sid.clone(),
        "strm_alice_video".into(),
        PublisherEntry {
            connection: alice_conn.clone(),
            participant: "part_alice".into(),
            kind: "video".into(),
            codec: None,
        },
    );

    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), Arc::clone(&publishers));
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );
    std::mem::forget(_coord); // keep driver task alive for the test duration

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_x", "conn_bob").await;

    (orch, publishers, in_tx, out_rx)
}

fn from_participant_env(
    sid: &str,
    connid: &str,
    participant: &str,
    kinds: &[&str],
) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::StreamSubscribe,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some(sid.into()),
        connid: Some(connid.into()),
        in_reply_to: None,
        payload: serde_json::to_value(StreamSubscribe {
            by_participant: "part_subscriber".into(),
            subscriptions: vec![StreamSubscription {
                strm_id: None,
                from_participant: Some(participant.into()),
                kinds: kinds.iter().map(|s| (*s).to_string()).collect(),
            }],
        })
        .unwrap(),
        signature: None,
    }
}

#[tokio::test]
async fn from_participant_subscribes_to_all_streams_when_no_kinds_filter() {
    let (orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams().await;

    in_tx
        .send(from_participant_env(
            "sess_x",
            "conn_bob",
            "part_alice",
            &[],
        ))
        .await
        .unwrap();
    let ack = out_rx.recv().await.expect("expected ack");
    assert_eq!(ack.msg_type, MessageType::Ack);

    // Both Alice's streams (audio + video) should now be subscribed.
    let sid = SessionId::from_string("sess_x");
    let alice = ConnectionId::from_string("conn_alice");
    let audio_subs = orch.subscribers_for(&sid, &alice, &StreamId::from_string("strm_alice_audio"));
    let video_subs = orch.subscribers_for(&sid, &alice, &StreamId::from_string("strm_alice_video"));
    assert_eq!(audio_subs.len(), 1);
    assert_eq!(video_subs.len(), 1);
    assert_eq!(audio_subs[0].to_string(), "conn_bob");
    assert_eq!(video_subs[0].to_string(), "conn_bob");
}

#[tokio::test]
async fn from_participant_with_kinds_filter_only_subscribes_matching_streams() {
    let (orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams().await;

    // Subscribe only to Alice's audio.
    in_tx
        .send(from_participant_env(
            "sess_x",
            "conn_bob",
            "part_alice",
            &["audio"],
        ))
        .await
        .unwrap();
    let ack = out_rx.recv().await.expect("expected ack");
    assert_eq!(ack.msg_type, MessageType::Ack);

    let sid = SessionId::from_string("sess_x");
    let alice = ConnectionId::from_string("conn_alice");
    let audio_subs = orch.subscribers_for(&sid, &alice, &StreamId::from_string("strm_alice_audio"));
    let video_subs = orch.subscribers_for(&sid, &alice, &StreamId::from_string("strm_alice_video"));
    assert_eq!(audio_subs.len(), 1, "audio subscription should be created");
    assert_eq!(
        video_subs.len(),
        0,
        "video stream excluded by kinds filter must not be subscribed"
    );
}

#[tokio::test]
async fn from_participant_with_unknown_participant_yields_404() {
    let (_orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams().await;

    in_tx
        .send(from_participant_env(
            "sess_x",
            "conn_bob",
            "part_ghost",
            &[],
        ))
        .await
        .unwrap();
    let reply = out_rx.recv().await.expect("expected error");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 404);
    assert!(payload.reason.contains("part_ghost"));
}

#[tokio::test]
async fn from_participant_with_kinds_that_match_nothing_yields_488() {
    let (_orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams().await;

    // Alice publishes audio + video; subscribe with kinds=["data"] →
    // participant exists but no streams match → 488 incompatible.
    in_tx
        .send(from_participant_env(
            "sess_x",
            "conn_bob",
            "part_alice",
            &["data"],
        ))
        .await
        .unwrap();
    let reply = out_rx.recv().await.expect("expected error");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 488);
    assert!(payload.reason.contains("part_alice"));
}

#[tokio::test]
async fn subscribe_refuses_unsupported_codec_with_488() {
    // B2: a publisher with a codec outside DEFAULT_ACCEPTED_CODECS
    // (e.g. "exotic-codec") triggers `error 488` on subscribe so the
    // subscriber doesn't receive frames it can't decode.
    use rvoip_core::capability::CodecInfo;

    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());

    let sid = SessionId::from_string("sess_b2");
    let publisher_connid = ConnectionId::from_string("conn_exotic");
    publishers.register(
        sid.clone(),
        "strm_exotic".into(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "part_publisher".into(),
            kind: "audio".into(),
            codec: Some(CodecInfo {
                name: "exotic-codec".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            }),
        },
    );
    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), publishers);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_b2", "conn_subscriber").await;

    in_tx
        .send(subscribe_env(
            "sess_b2",
            "conn_subscriber",
            &["strm_exotic"],
        ))
        .await
        .unwrap();
    let reply = out_rx.recv().await.expect("expected 488");
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 488);
    assert!(
        payload.reason.contains("unsupported codec") && payload.reason.contains("exotic-codec"),
        "488 reason must identify the offending codec; got {:?}",
        payload.reason
    );

    let subs = orch.subscribers_for(
        &sid,
        &publisher_connid,
        &StreamId::from_string("strm_exotic"),
    );
    assert!(
        subs.is_empty(),
        "refused subscription must not record a row"
    );
}

#[tokio::test]
async fn subscribe_accepts_when_codec_in_default_set() {
    // B2 inverse: opus is in DEFAULT_ACCEPTED_CODECS → subscribe passes.
    use rvoip_core::capability::CodecInfo;

    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());

    let sid = SessionId::from_string("sess_b2_ok");
    let publisher_connid = ConnectionId::from_string("conn_opus");
    publishers.register(
        sid.clone(),
        "strm_opus".into(),
        PublisherEntry {
            connection: publisher_connid.clone(),
            participant: "part_publisher".into(),
            kind: "audio".into(),
            codec: Some(CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            }),
        },
    );
    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), publishers);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_b2_ok", "conn_sub").await;

    in_tx
        .send(subscribe_env("sess_b2_ok", "conn_sub", &["strm_opus"]))
        .await
        .unwrap();
    let reply = out_rx.recv().await.expect("expected ack");
    assert_eq!(reply.msg_type, MessageType::Ack);

    let subs = orch.subscribers_for(&sid, &publisher_connid, &StreamId::from_string("strm_opus"));
    assert_eq!(subs.len(), 1, "opus subscription must succeed");
}

#[tokio::test]
async fn from_participant_skips_unsupported_codec_streams() {
    // B2 from_participant variant: best-effort enumeration. A
    // participant publishing one opus stream and one exotic-codec
    // stream — subscribing by from_participant subscribes only to
    // the opus stream; the exotic one is silently skipped.
    use rvoip_core::capability::CodecInfo;
    use rvoip_uctp::payloads::stream::{StreamSubscribe, StreamSubscription};

    let orch = Orchestrator::new(Config::default());
    let publishers = Arc::new(PublisherRegistry::new());
    let sid = SessionId::from_string("sess_b2_mix");
    let alice_conn = ConnectionId::from_string("conn_alice");
    publishers.register(
        sid.clone(),
        "strm_alice_opus".into(),
        PublisherEntry {
            connection: alice_conn.clone(),
            participant: "part_alice".into(),
            kind: "audio".into(),
            codec: Some(CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            }),
        },
    );
    publishers.register(
        sid.clone(),
        "strm_alice_exotic".into(),
        PublisherEntry {
            connection: alice_conn.clone(),
            participant: "part_alice".into(),
            kind: "audio".into(),
            codec: Some(CodecInfo {
                name: "exotic-codec".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            }),
        },
    );
    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), Arc::clone(&publishers));

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        descriptor_with_opus(),
        handler,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    establish_subscriber_connection(&in_tx, &mut out_rx, "sess_b2_mix", "conn_bob").await;

    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::StreamSubscribe,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_b2_mix".into()),
        connid: Some("conn_bob".into()),
        in_reply_to: None,
        payload: serde_json::to_value(StreamSubscribe {
            by_participant: "part_bob".into(),
            subscriptions: vec![StreamSubscription {
                strm_id: None,
                from_participant: Some("part_alice".into()),
                kinds: Vec::new(),
            }],
        })
        .unwrap(),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected ack");
    assert_eq!(reply.msg_type, MessageType::Ack);

    let opus_subs =
        orch.subscribers_for(&sid, &alice_conn, &StreamId::from_string("strm_alice_opus"));
    assert_eq!(opus_subs.len(), 1, "opus stream must be subscribed");

    let exotic_subs = orch.subscribers_for(
        &sid,
        &alice_conn,
        &StreamId::from_string("strm_alice_exotic"),
    );
    assert!(
        exotic_subs.is_empty(),
        "exotic-codec stream must be silently skipped, not subscribed"
    );
}
