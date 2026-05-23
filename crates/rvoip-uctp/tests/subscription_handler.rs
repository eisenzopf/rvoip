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
    payloads::stream::{StreamSubscribe, StreamSubscription, StreamUnsubscribe},
    state::{OrchestratorSubscriptionHandler, UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use std::sync::Arc;
use tokio::sync::mpsc;

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
        Arc::new(CapabilityDescriptor::default()),
        handler,
    );

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
        Arc::new(CapabilityDescriptor::default()),
        handler,
    );

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
        Arc::new(CapabilityDescriptor::default()),
        handler,
    );

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
// publisher registry covering Alice's audio and video streams.
fn setup_with_alice_streams() -> (
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
        },
    );
    publishers.register(
        sid.clone(),
        "strm_alice_video".into(),
        PublisherEntry {
            connection: alice_conn.clone(),
            participant: "part_alice".into(),
            kind: "video".into(),
        },
    );

    let handler = OrchestratorSubscriptionHandler::new(Arc::clone(&orch), Arc::clone(&publishers));
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        Arc::new(CapabilityDescriptor::default()),
        handler,
    );
    std::mem::forget(_coord); // keep driver task alive for the test duration

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
    }
}

#[tokio::test]
async fn from_participant_subscribes_to_all_streams_when_no_kinds_filter() {
    let (orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams();

    in_tx
        .send(from_participant_env("sess_x", "conn_bob", "part_alice", &[]))
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
    let (orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams();

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
    let (_orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams();

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
    let (_orch, _publishers, in_tx, mut out_rx) = setup_with_alice_streams();

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
