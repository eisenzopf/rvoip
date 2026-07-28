//! Gap plan §4.2A v1 punch list — `Pending::deliver` gate in
//! `UctpCoordinator::dispatch_inner`.
//!
//! Two assertions:
//!
//! 1. **Authenticated matched-waiter delivery.** A waiter registers on
//!    `Pending`, then an authenticated envelope arrives whose
//!    `in_reply_to` matches the waiter's id. The gate delivers it only after
//!    the normal security checks.
//!
//! 2. **No-match fallthrough.** An envelope with `in_reply_to` set
//!    to an id NOT registered on `Pending` falls through to the
//!    normal handler. The waiter (if any) does not resolve from
//!    this unrelated traffic.

mod common;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::ids::EnvelopeId;
use rvoip_uctp::payloads::{auth, connection};
use rvoip_uctp::state::{
    default_v0_descriptor, rejecting_handler, UctpCoordinator, UctpCoordinatorCaps,
    ENVELOPE_CHANNEL_CAP,
};
use rvoip_uctp::types::MessageType;
use tokio::sync::mpsc;
use uuid::Uuid;

fn build_coordinator() -> (
    Arc<UctpCoordinator>,
    mpsc::Sender<UctpEnvelope>,
    mpsc::Receiver<UctpEnvelope>,
) {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        Arc::new(default_v0_descriptor()),
        rejecting_handler(),
        UctpCoordinatorCaps::default(),
    );
    (coord, in_tx, out_rx)
}

fn unsolicited_connection_update(in_reply_to: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionUpdate,
        id: format!("env_{}", Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_1".into()),
        connid: Some("conn_1".into()),
        in_reply_to: Some(in_reply_to.into()),
        payload: serde_json::to_value(connection::ConnectionUpdate {
            action: "renegotiate-media".into(),
            streams: vec![],
            codec_preferences: vec!["opus".into()],
            details: serde_json::Value::Null,
        })
        .unwrap(),
        signature: None,
    }
}

#[tokio::test]
async fn matched_in_reply_to_routes_to_pending_waiter_and_skips_handler() {
    let (coord, in_tx, mut out_rx) = build_coordinator();
    common::drive_auth_handshake(&in_tx, &mut out_rx).await;
    let pending = coord.pending();

    // Register a waiter for envelope id `env_my_request`.
    let waiter_id = EnvelopeId::from_string("env_my_request".to_string());
    let waiter_handle = {
        let pending = Arc::clone(&pending);
        let id = waiter_id.clone();
        tokio::spawn(async move { pending.wait_for(id, Duration::from_secs(5)).await })
    };
    // Tiny yield so wait_for registers before we deliver.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let reply_env = unsolicited_connection_update("env_my_request");
    let reply_env_id = reply_env.id.clone();
    in_tx.send(reply_env).await.unwrap();

    // The waiter resolves with the envelope. The handler never ran:
    // if it had, the coordinator would have emitted *something* on
    // out_rx (an error or a reply); assert nothing arrives.
    let got = tokio::time::timeout(Duration::from_secs(2), waiter_handle)
        .await
        .expect("waiter resolves within 2s")
        .expect("task join")
        .expect("waiter ok");
    assert_eq!(got.id, reply_env_id);
    assert_eq!(got.msg_type, MessageType::ConnectionUpdate);

    // No coordinator-side reply landed on out_rx.
    let no_out = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(
        no_out.is_err(),
        "delivered reply must NOT trigger the regular handler; got {:?}",
        no_out.ok().flatten().map(|e| e.msg_type)
    );
}

#[tokio::test]
async fn unauthenticated_correlated_reply_is_rejected_before_delivery() {
    let (coord, in_tx, mut out_rx) = build_coordinator();
    let pending = coord.pending();
    let waiter = {
        let pending = Arc::clone(&pending);
        tokio::spawn(async move {
            pending
                .wait_for(
                    EnvelopeId::from_string("env_protected_request"),
                    Duration::from_millis(250),
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;

    in_tx
        .send(unsolicited_connection_update("env_protected_request"))
        .await
        .unwrap();

    let error = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
        .await
        .expect("security rejection")
        .expect("output channel open");
    assert_eq!(error.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 401);

    assert!(
        waiter.await.unwrap().is_err(),
        "the rejected reply must not consume or resolve the waiter"
    );
}

#[tokio::test]
async fn unmatched_in_reply_to_falls_through_to_regular_handler() {
    let (coord, in_tx, mut out_rx) = build_coordinator();
    let _pending = coord.pending();

    // Auth the peer first, so the regular handler can actually run.
    common::drive_auth_handshake(&in_tx, &mut out_rx).await;

    // Send a `connection.update` whose `in_reply_to` doesn't match
    // any registered waiter. The deliver gate's `Err(env)` path
    // hands the envelope back; the regular `handle_connection_update`
    // runs. Because the connid is unknown, the handler emits a
    // 404 error. The exact code doesn't matter — what matters is
    // *something* lands on out_rx, proving the handler ran.
    let env = unsolicited_connection_update("env_no_waiter");
    in_tx.send(env).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .expect("handler emits something within 2s")
        .expect("channel open");
    // Should be an `error` envelope — the connid `conn_1` doesn't
    // exist in the connection registry.
    assert_eq!(
        reply.msg_type,
        MessageType::Error,
        "expected handler to emit error for unknown connid; got {:?}",
        reply.msg_type
    );
}

#[allow(dead_code)]
fn _silence() {
    let _: Option<auth::AuthHello> = None;
}
