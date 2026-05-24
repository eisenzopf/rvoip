//! End-to-end test: UCTP coordinator's auth gate (plan A1) backed by a
//! real [`rvoip_auth_core::JwtValidator`] (plan C4 prelude) refuses
//! invalid tokens and accepts valid ones. Proves the BearerValidator
//! trait integration works for non-stub validators.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use rvoip_auth_core::JwtValidator;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::auth,
    state::{UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use serde::Serialize;
use tokio::sync::mpsc;

const SECRET: &[u8] = b"integration-test-secret";

#[derive(Serialize)]
struct Claims {
    sub: String,
    exp: i64,
}

fn mint(sub: &str, expires_in_secs: i64) -> String {
    let claims = Claims {
        sub: sub.into(),
        exp: (Utc::now().timestamp() + expires_in_secs),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(SECRET),
    )
    .expect("encode")
}

#[tokio::test]
async fn coordinator_accepts_valid_jwt_and_unlocks_envelopes() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let validator = Arc::new(JwtValidator::from_hmac_secret(SECRET));
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, validator);

    // Drive `auth.hello → auth.challenge → auth.response (token) → auth.session`.
    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_t".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    );
    in_tx.send(hello).await.unwrap();
    let challenge = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .expect("challenge")
        .unwrap();
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);

    let token = mint("id_alice", 3600);
    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: token,
        })
        .unwrap(),
    )
    .with_in_reply_to(challenge.id);
    in_tx.send(response).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .expect("session reply")
        .unwrap();
    assert_eq!(
        reply.msg_type,
        MessageType::AuthSession,
        "valid JWT must yield auth.session"
    );

    // The auth.session payload echoes the assurance label. JwtValidator
    // produces `UserAuthorized`, which maps to "user-authorized" per
    // the coordinator's assurance_label translation.
    let payload: auth::AuthSession = reply.decode_payload().unwrap();
    assert_eq!(
        payload.assurance, "user-authorized",
        "JWT-authenticated peer must receive UserAuthorized assurance"
    );
}

#[tokio::test]
async fn coordinator_rejects_invalid_jwt_with_401() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let validator = Arc::new(JwtValidator::from_hmac_secret(SECRET));
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, validator);

    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_t".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    );
    in_tx.send(hello).await.unwrap();
    let challenge = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .unwrap()
        .unwrap();

    // Send a malformed token — the JwtValidator must refuse and the
    // coordinator must emit `error 401 auth/bearer-validation-failed`.
    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "not.a.valid.jwt".into(),
        })
        .unwrap(),
    )
    .with_in_reply_to(challenge.id);
    in_tx.send(response).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .expect("401 expected")
        .unwrap();
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error =
        reply.decode_payload().expect("decode error");
    assert_eq!(payload.code, 401);
    assert_eq!(payload.category, "auth");
}

#[tokio::test]
async fn coordinator_rejects_expired_jwt_with_401() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let validator = Arc::new(JwtValidator::from_hmac_secret(SECRET));
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, validator);

    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_t".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    );
    in_tx.send(hello).await.unwrap();
    let challenge = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .unwrap()
        .unwrap();

    // Expired JWT (10 minutes ago, well past the default 60s leeway).
    let expired = mint("id_alice", -600);
    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: expired,
        })
        .unwrap(),
    )
    .with_in_reply_to(challenge.id);
    in_tx.send(response).await.unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
        .await
        .expect("401 for expired token")
        .unwrap();
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
}
