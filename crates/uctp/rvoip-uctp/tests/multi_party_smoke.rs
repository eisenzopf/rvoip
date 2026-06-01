//! v0.x MP2.6 — publisher registration + adapter wiring smoke test.
//!
//! Exercises the end-to-end flow where the coordinator (configured with
//! a real `OrchestratorSubscriptionHandler`):
//!
//! 1. Receives `connection.offer` and stores the accepted streams.
//! 2. Receives `connection.ready` and emits `stream.opened` for each
//!    accepted stream, auto-registering the publisher in the
//!    `PublisherRegistry`.
//! 3. A subsequent `stream.subscribe` against the registered `strm_id`
//!    resolves successfully (no test pre-population required).
//!
//! Validates the MP2 → MP2.6 hand-off: tests for MP2 had to manually
//! call `publishers.register(...)` because nothing populated it. With
//! MP2.6 in place, the wire flow auto-registers.

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_core::config::Config;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::Orchestrator;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{
        connection::{ConnectionOffer, StreamOffer},
        stream::{StreamOpened, StreamSubscribe, StreamSubscription},
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
        audio_codecs: vec![CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }],
        ..Default::default()
    })
}

fn offer_env(sid: &str, connid: &str, strm_id: &str) -> UctpEnvelope {
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
            by_participant: "part_publisher".into(),
            substrate: "quic".into(),
            capabilities: serde_json::Value::Object(Default::default()),
            streams_offered: vec![StreamOffer {
                id: strm_id.into(),
                kind: "audio".into(),
                direction: "sendrecv".into(),
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

fn subscribe_env(sid: &str, connid: &str, strm_id: &str) -> UctpEnvelope {
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
                strm_id: Some(strm_id.into()),
                from_participant: None,
                kinds: Vec::new(),
            }],
        })
        .unwrap(),
    signature: None,
    }
}

async fn next_envelope_of(
    rx: &mut mpsc::Receiver<UctpEnvelope>,
    msg_type: MessageType,
) -> Option<UctpEnvelope> {
    // Skip up to a handful of envelopes looking for the one we want.
    // The coordinator can emit multiple things before our target arrives
    // (e.g. acks). Bounded so the test fails-fast on a missing emission.
    for _ in 0..8 {
        let env = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .ok()
            .flatten()?;
        if env.msg_type == msg_type {
            return Some(env);
        }
    }
    None
}

#[tokio::test]
async fn connection_ready_emits_stream_opened_and_registers_publisher() {
    let orch = Orchestrator::new(Config::default());
    let publishers = orch.publisher_registry();
    let handler =
        OrchestratorSubscriptionHandler::new(Arc::clone(&orch), Arc::clone(&publishers));

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

    // 1. Send connection.offer — coordinator runs negotiation (passes;
    //    descriptor has opus, offer has opus) and stores accepted streams.
    in_tx
        .send(offer_env("sess_a", "conn_publisher", "strm_audio_1"))
        .await
        .unwrap();
    // Brief yield so the coordinator processes the offer.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 2. Send connection.ready — coordinator should emit stream.opened
    //    and auto-register the publisher.
    in_tx
        .send(ready_env("sess_a", "conn_publisher"))
        .await
        .unwrap();

    let opened = next_envelope_of(&mut out_rx, MessageType::StreamOpened)
        .await
        .expect("expected stream.opened envelope");
    assert_eq!(opened.sid.as_deref(), Some("sess_a"));
    assert_eq!(opened.connid.as_deref(), Some("conn_publisher"));
    let payload: StreamOpened = opened.decode_payload().unwrap();
    assert_eq!(payload.stream.strm_id, "strm_audio_1");
    assert_eq!(payload.stream.kind, "audio");
    assert_eq!(payload.stream.direction, "sendrecv");
    assert!(
        payload.stream.stream_local_id > 0,
        "stream_local_id must be non-zero — got {}",
        payload.stream.stream_local_id
    );
    // Codec field carries the negotiated codec name.
    assert_eq!(payload.stream.codec["name"], "opus");

    // 3. PublisherRegistry should now resolve the strm_id.
    let publisher = publishers.publisher(
        &SessionId::from_string("sess_a"),
        "strm_audio_1",
    );
    assert_eq!(
        publisher.as_ref().map(|c| c.to_string()),
        Some("conn_publisher".to_string()),
        "publisher registry should have auto-registered the publisher"
    );

    // 4. A subscriber sending stream.subscribe by strm_id should now
    //    succeed without any test pre-population.
    in_tx
        .send(subscribe_env("sess_a", "conn_subscriber", "strm_audio_1"))
        .await
        .unwrap();
    let ack = next_envelope_of(&mut out_rx, MessageType::Ack)
        .await
        .expect("expected ack on subscribe");
    assert!(ack.in_reply_to.is_some());

    // 5. The orchestrator's subscription registry should hold the row.
    let subs = orch.subscribers_for(
        &SessionId::from_string("sess_a"),
        &ConnectionId::from_string("conn_publisher"),
        &StreamId::from_string("strm_audio_1"),
    );
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].to_string(), "conn_subscriber");
}

#[tokio::test]
async fn duplicate_connection_ready_does_not_re_emit_stream_opened() {
    let orch = Orchestrator::new(Config::default());
    let publishers = orch.publisher_registry();
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

    in_tx
        .send(offer_env("sess_b", "conn_b", "strm_x"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    in_tx.send(ready_env("sess_b", "conn_b")).await.unwrap();

    // First ready → one stream.opened.
    let _opened = next_envelope_of(&mut out_rx, MessageType::StreamOpened)
        .await
        .expect("first stream.opened");

    // Drain anything else.
    while tokio::time::timeout(Duration::from_millis(30), out_rx.recv())
        .await
        .ok()
        .flatten()
        .is_some()
    {}

    // Second ready → no further stream.opened.
    in_tx.send(ready_env("sess_b", "conn_b")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    while let Ok(Some(env)) = tokio::time::timeout(Duration::from_millis(30), out_rx.recv()).await {
        assert_ne!(
            env.msg_type,
            MessageType::StreamOpened,
            "duplicate connection.ready must not re-emit stream.opened"
        );
    }
}
