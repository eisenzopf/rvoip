//! Shared test helpers for coordinator-driven integration tests.

use chrono::Utc;
use rvoip_uctp::{envelope::UctpEnvelope, payloads::auth, types::MessageType};
use tokio::sync::mpsc;

/// Drive the coordinator through the four-envelope auth handshake
/// (`auth.hello → auth.challenge → auth.response → auth.session`) so
/// subsequent session/connection/stream envelopes from `in_tx` pass the
/// per-peer auth gate (plan §7 G1 / `coordinator.rs::require_authenticated`).
///
/// Drains the inbound `auth.challenge` and `auth.session` envelopes from
/// `out_rx`. Does not touch the events channel; callers that observe the
/// `UctpSessionEvent::Authenticated` event should keep their `events_rx`
/// handle and drain it after this returns.
pub async fn drive_auth_handshake(
    in_tx: &mpsc::Sender<UctpEnvelope>,
    out_rx: &mut mpsc::Receiver<UctpEnvelope>,
) {
    let hello = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_test".into(),
                kind: "desktop".into(),
                platform: "test-platform".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    };
    in_tx.send(hello).await.expect("send auth.hello");

    let challenge = out_rx.recv().await.expect("receive auth.challenge");
    assert_eq!(
        challenge.msg_type,
        MessageType::AuthChallenge,
        "expected auth.challenge from coordinator"
    );

    let response = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(challenge.id),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
        })
        .unwrap(),
    };
    in_tx.send(response).await.expect("send auth.response");

    let session = out_rx.recv().await.expect("receive auth.session");
    assert_eq!(
        session.msg_type,
        MessageType::AuthSession,
        "expected auth.session from coordinator"
    );
}
