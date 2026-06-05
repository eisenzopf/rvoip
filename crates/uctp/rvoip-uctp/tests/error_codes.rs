//! Gap plan §3.1 — error codes 501 / 505.
//!
//! Covers the two codes added to CONVERSATION_PROTOCOL.md §11.2 in this
//! pass:
//!
//! - **501 not-implemented** — the receiver recognized the envelope type
//!   but cannot service it on this connection. The default
//!   `RejectingHandler` returns this for `stream.subscribe` /
//!   `stream.unsubscribe`; the prior behavior was 503.
//!
//! - **505 version-not-supported** — the envelope's `v` field is not in
//!   the set this server understands. Pre-v0.x silently dropped these;
//!   now the server replies with 505 and a `details.supported` array.

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    state::{UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use tokio::sync::mpsc;

mod common;
use common::drive_auth_handshake;

#[tokio::test]
async fn default_subscription_handler_rejects_with_501() {
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
    assert_eq!(
        payload.code, 501,
        "RejectingHandler must return 501 not-implemented (was 503 pre-§3.1)"
    );
    assert_eq!(payload.reason, "multi-party-routing-not-implemented");
}

#[tokio::test]
async fn unsupported_protocol_version_replies_505() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    // The version gate runs before the auth gate, so we don't need to
    // drive the auth handshake — a `v=2` `auth.hello` should still be
    // rejected with 505.
    let env = UctpEnvelope {
        v: 2,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::json!({}),
        signature: None,
    };
    let env_id = env.id.clone();
    in_tx.send(env).await.unwrap();

    let reply = out_rx.recv().await.expect("expected error envelope");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(
        reply.in_reply_to.as_deref(),
        Some(env_id.as_str()),
        "505 reply must correlate to the rejected envelope"
    );

    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 505);
    assert_eq!(payload.category, "protocol");
    assert_eq!(payload.reason, "version-not-supported");

    let supported = payload
        .details
        .get("supported")
        .and_then(|v| v.as_array())
        .expect("details.supported array");
    assert!(
        supported.iter().any(|v| v.as_u64() == Some(1)),
        "details.supported must list v=1; got {:?}",
        supported
    );
}
