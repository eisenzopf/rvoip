//! End-to-end coordinator tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8 / §3.5.
//!
//! Drives the coordinator with synthetic inbound envelopes and asserts
//! both outbound envelopes and emitted [`UctpSessionEvent`]s.

use async_trait::async_trait;
use chrono::Utc;
use rvoip_auth_core::{
    bearer_stub, AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, BearerValidator,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::IdentityId;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{auth, connection, session},
    state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};

mod common;
use common::drive_auth_handshake;

struct StablePrincipalValidator;

#[async_trait]
impl BearerValidator for StablePrincipalValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        let subject = if token == "other-owner" {
            "user:bob"
        } else {
            "user:alice"
        };
        let identity = IdentityId::from_string(subject);
        let scopes = if token == "no-scopes" {
            Vec::new()
        } else {
            vec!["*".into()]
        };
        let assurance = IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: scopes.clone(),
        };
        Ok(AuthenticatedPrincipal {
            subject: subject.into(),
            tenant: Some("tenant-a".into()),
            scopes,
            issuer: Some("https://issuer.example".into()),
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance,
        })
    }
}

struct BlockingPrincipalValidator {
    entered: Semaphore,
    release: Semaphore,
}

impl BlockingPrincipalValidator {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        })
    }
}

#[async_trait]
impl BearerValidator for BlockingPrincipalValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        self.entered.add_permits(1);
        self.release
            .acquire()
            .await
            .expect("release semaphore remains open")
            .forget();
        StablePrincipalValidator.validate_principal(token).await
    }
}

struct PanickingPrincipalValidator;

#[async_trait]
impl BearerValidator for PanickingPrincipalValidator {
    async fn validate(&self, _token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        panic!("intentional validator panic")
    }

    async fn validate_principal(
        &self,
        _token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        panic!("intentional validator panic")
    }
}

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
    let payload: auth::AuthChallenge = reply.decode_payload().unwrap();
    assert_eq!(
        payload.server_capabilities["media_profile"],
        rvoip_uctp::UCTP_RTP_DATAGRAM_PROFILE
    );
    assert_eq!(
        payload.server_capabilities["envelope_versions"],
        serde_json::json!([1])
    );
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
async fn shutdown_waits_for_inflight_auth_and_clears_post_dispatch_state() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let validator = BlockingPrincipalValidator::new();
    let coordinator = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, validator.clone());

    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.expect("challenge");
    in_tx
        .send(auth_response_env("token", &challenge.id))
        .await
        .unwrap();
    validator
        .entered
        .acquire()
        .await
        .expect("validator entry signal")
        .forget();

    let coordinator_for_shutdown = Arc::clone(&coordinator);
    let shutdown = tokio::spawn(async move {
        coordinator_for_shutdown.shutdown().await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !shutdown.is_finished(),
        "shutdown must fence an in-flight validator before clearing state"
    );

    validator.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(1), shutdown)
        .await
        .expect("shutdown should complete after validator returns")
        .expect("shutdown task should not panic");
    let snapshot = coordinator.resource_snapshot();
    assert_eq!(snapshot.sessions, 0);
    assert_eq!(snapshot.connections, 0);
    assert_eq!(snapshot.pending_replies, 0);
    assert!(!snapshot.authenticated);
}

#[tokio::test]
async fn driver_panic_cancels_peer_and_clears_resources() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coordinator = UctpCoordinator::start(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        Arc::new(PanickingPrincipalValidator),
    );
    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.expect("challenge");
    in_tx
        .send(auth_response_env("panic", &challenge.id))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(1), coordinator.cancelled())
        .await
        .expect("driver panic must cancel the peer");
    tokio::time::timeout(Duration::from_secs(1), coordinator.shutdown())
        .await
        .expect("panic cleanup must complete");
    let snapshot = coordinator.resource_snapshot();
    assert_eq!(snapshot.sessions, 0);
    assert_eq!(snapshot.connections, 0);
    assert_eq!(snapshot.pending_replies, 0);
    assert!(!snapshot.authenticated);
}

#[tokio::test]
async fn malformed_envelope_and_correlation_ids_are_rejected_before_auth_handshake() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let oversized = format!("env_{}", "a".repeat(rvoip_uctp::MAX_ENVELOPE_ID_BYTES));
    for invalid_id in [String::new(), "env_has space".into(), oversized] {
        let mut hello = auth_hello_env();
        hello.id = invalid_id;
        in_tx.send(hello).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        out_rx.try_recv().is_err(),
        "malformed IDs must not reach the auth handler"
    );

    let hello = auth_hello_env();
    in_tx.send(hello).await.unwrap();
    let challenge = out_rx.recv().await.expect("valid auth challenge");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);

    let mut response = auth_response_env("token", "invalid correlation id");
    let response_id = response.id.clone();
    in_tx.send(response.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        out_rx.try_recv().is_err(),
        "invalid correlation ID must not reach the auth handler"
    );

    // Reusing the same valid envelope ID after correcting only in_reply_to
    // proves the malformed envelope was rejected before replay bookkeeping.
    response.id = response_id;
    response.in_reply_to = Some(challenge.id);
    in_tx.send(response).await.unwrap();
    assert_eq!(
        out_rx.recv().await.expect("auth session").msg_type,
        MessageType::AuthSession
    );
}

#[tokio::test]
async fn unauthenticated_commands_do_not_consume_replay_ids() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    let invite = session_invite_env("sess_reused_after_auth", "conv_reused_after_auth");
    in_tx.send(invite.clone()).await.unwrap();
    let rejection = out_rx.recv().await.expect("unauthenticated rejection");
    let error: rvoip_uctp::payloads::control::Error = rejection.decode_payload().unwrap();
    assert_eq!(error.code, 401);

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await.expect("authenticated event");
    in_tx.send(invite).await.unwrap();
    match events_rx.recv().await.expect("authenticated invite") {
        UctpSessionEvent::InboundInvite { sid, .. } => {
            assert_eq!(sid.as_str(), "sess_reused_after_auth");
        }
        other => panic!("expected InboundInvite, got {other:?}"),
    }
}

#[tokio::test]
async fn authenticated_command_without_required_scope_is_rejected() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        Arc::new(StablePrincipalValidator),
    );

    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.unwrap();
    in_tx
        .send(auth_response_env("no-scopes", &challenge.id))
        .await
        .unwrap();
    assert_eq!(
        out_rx.recv().await.unwrap().msg_type,
        MessageType::AuthSession
    );
    let _ = events_rx.recv().await;

    let invite = session_invite_env("sess_scope_denied", "conv_scope_denied");
    in_tx.send(invite).await.unwrap();
    let error = out_rx.recv().await.expect("scope error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 403);
    assert_eq!(payload.reason, "insufficient-scope");
    assert!(events_rx.try_recv().is_err());
}

#[tokio::test]
async fn second_auth_response_cannot_switch_peer_owner() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        Arc::new(StablePrincipalValidator),
    );

    in_tx.send(auth_hello_env()).await.unwrap();
    let challenge = out_rx.recv().await.unwrap();
    in_tx
        .send(auth_response_env("alice", &challenge.id))
        .await
        .unwrap();
    let _ = out_rx.recv().await;
    let _ = events_rx.recv().await;

    in_tx
        .send(auth_response_env("other-owner", "env_reauth"))
        .await
        .unwrap();
    let error = out_rx.recv().await.expect("owner-switch error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
    assert_eq!(
        coord.authenticated_principal().unwrap().subject,
        "user:alice"
    );
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
            assert!(!from.is_empty());
            assert_ne!(
                from, "part_alice",
                "wire sender identity is not authoritative"
            );
            assert_eq!(to, vec!["part_bob".to_string()]);
            assert_eq!(medium, "voice");
        }
        other => panic!("expected InboundInvite, got {:?}", other),
    }
}

#[tokio::test]
async fn resolver_rejection_precedes_session_commit_and_inbound_event() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());
    let resolver: Arc<dyn rvoip_uctp::state::SessionBindingResolver> = Arc::new(
        |_: &AuthenticatedPrincipal, _: &rvoip_uctp::ids::SessionId| {
            Err(rvoip_uctp::state::ResourceBindingError::forbidden(
                "attachment-token-denied",
            ))
        },
    );
    let bindings = rvoip_uctp::state::PeerResourceBindings::new(resolver);
    coord
        .set_resource_bindings(Arc::clone(&bindings))
        .expect("fresh coordinator accepts one resource authority");

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::Authenticated { .. })
    ));
    let invite = session_invite_env("sess_denied", "conv_denied");
    let invite_id = invite.id.clone();
    in_tx.send(invite).await.unwrap();

    let error = out_rx.recv().await.expect("resolver rejection error");
    assert_eq!(error.msg_type, MessageType::Error);
    assert_eq!(error.in_reply_to.as_deref(), Some(invite_id.as_str()));
    assert_eq!(error.sid.as_deref(), Some("sess_denied"));
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 403);
    assert_eq!(payload.category, "auth");
    assert_eq!(payload.reason, "attachment-token-denied");
    assert_eq!(coord.resource_snapshot().sessions, 0);
    assert!(events_rx.try_recv().is_err(), "InboundInvite must not emit");
    assert!(bindings
        .core_session(&rvoip_uctp::ids::SessionId::from_string("sess_denied"))
        .is_none());
}

#[tokio::test]
async fn multi_party_stream_subscribe_rejected_with_501() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_x").await;
    let _ = events_rx.recv().await;
    in_tx
        .send(connection_offer_env("sess_x", "conn_y", &["opus"]))
        .await
        .unwrap();
    let _ = events_rx.recv().await;

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

fn connection_ready_env(sid: &str, connid: &str) -> UctpEnvelope {
    UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
        .with_sid(sid)
        .with_connid(connid)
}

async fn establish_session(in_tx: &mpsc::Sender<UctpEnvelope>, sid: &str) {
    in_tx
        .send(session_invite_env(sid, &format!("conv_{sid}")))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
    establish_session(&in_tx, "sess_x").await;

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
    establish_session(&in_tx, "sess_x").await;

    let env = connection_offer_env("sess_x", "conn_y", &["opus"]);
    in_tx.send(env).await.unwrap();

    // No outbound envelope should arrive — accepting is silent in v0
    // (the spec doesn't mandate an immediate ack for connection.offer).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(out_rx.try_recv().is_err());
}

#[tokio::test]
async fn connection_offer_for_unknown_session_is_rejected_without_open_event() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // Authenticated
    let offer = connection_offer_env("sess_missing", "conn_orphan", &["opus"]);
    let offer_id = offer.id.clone();
    in_tx.send(offer).await.unwrap();

    let reply = out_rx.recv().await.expect("expected unknown-sid error");
    assert_eq!(reply.msg_type, MessageType::Error);
    assert_eq!(reply.in_reply_to.as_deref(), Some(offer_id.as_str()));
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 404);
    assert_eq!(payload.reason, "unknown-sid");
    assert!(events_rx.try_recv().is_err());
}

#[tokio::test]
async fn replayed_connection_offer_is_not_dispatched_twice() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await; // Authenticated
    establish_session(&in_tx, "sess_duplicate").await;
    let _ = events_rx.recv().await; // InboundInvite
    let offer = connection_offer_env("sess_duplicate", "conn_duplicate", &["opus"]);
    in_tx.send(offer.clone()).await.unwrap();
    in_tx.send(offer).await.unwrap();

    match events_rx.recv().await.expect("ConnectionOpened") {
        UctpSessionEvent::ConnectionOpened { sid, connid, .. } => {
            assert_eq!(sid.as_str(), "sess_duplicate");
            assert_eq!(connid.as_str(), "conn_duplicate");
        }
        other => panic!("expected ConnectionOpened, got {other:?}"),
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), events_rx.recv())
            .await
            .is_err(),
        "a replayed envelope ID must not repeat state-changing dispatch"
    );
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
    let snapshot = coord.resource_snapshot();
    assert_eq!(snapshot.sessions, 0);
    assert_eq!(snapshot.connections, 0);
    assert_eq!(snapshot.pending_replies, 0);
    assert!(!snapshot.authenticated);
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
async fn session_invite_retransmit_is_idempotent_and_does_not_count_against_cap() {
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
    let coord = UctpCoordinator::start_full_with_caps(
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
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), events_rx.recv())
            .await
            .is_err(),
        "a duplicate sid must not allocate another adapter route"
    );
    assert_eq!(coord.resource_snapshot().sessions, 1);
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
async fn connection_cap_rejects_excess_state_machines() {
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let caps = UctpCoordinatorCaps {
        max_connections_per_peer: 1,
        ..Default::default()
    };
    let _coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_connection_cap").await;
    let _ = events_rx.recv().await;

    in_tx
        .send(connection_offer_env(
            "sess_connection_cap",
            "conn_one",
            &["opus"],
        ))
        .await
        .unwrap();
    let _ = events_rx.recv().await;

    let second = connection_offer_env("sess_connection_cap", "conn_two", &["opus"]);
    in_tx.send(second).await.unwrap();
    let error = out_rx.recv().await.expect("connection cap error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 429);
    assert_eq!(payload.reason, "too-many-connections");
}

#[tokio::test]
async fn stream_offer_cap_rejects_oversized_negotiation() {
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let caps = UctpCoordinatorCaps {
        max_streams_per_connection: 1,
        ..Default::default()
    };
    let _coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_stream_cap").await;
    let _ = events_rx.recv().await;

    let mut offer = connection_offer_env("sess_stream_cap", "conn_stream_cap", &["opus"]);
    let mut payload: connection::ConnectionOffer = offer.decode_payload().unwrap();
    let mut second = payload.streams_offered[0].clone();
    second.id = "stream_two".into();
    payload.streams_offered.push(second);
    offer.payload = serde_json::to_value(payload).unwrap();
    in_tx.send(offer).await.unwrap();

    let error = out_rx.recv().await.expect("stream cap error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 429);
    assert_eq!(payload.reason, "too-many-streams");
    assert!(events_rx.try_recv().is_err());
}

#[tokio::test]
async fn stream_offer_cap_is_cumulative_across_reoffers() {
    use rvoip_uctp::state::UctpCoordinatorCaps;

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let caps = UctpCoordinatorCaps {
        max_streams_per_connection: 2,
        ..Default::default()
    };
    let coord = UctpCoordinator::start_full_with_caps(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
        rvoip_uctp::state::rejecting_handler(),
        caps,
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_cumulative_stream_cap").await;
    let _ = events_rx.recv().await;

    in_tx
        .send(connection_offer_env(
            "sess_cumulative_stream_cap",
            "conn_cumulative_stream_cap",
            &["opus"],
        ))
        .await
        .unwrap();
    let _ = events_rx.recv().await;
    in_tx
        .send(connection_ready_env(
            "sess_cumulative_stream_cap",
            "conn_cumulative_stream_cap",
        ))
        .await
        .unwrap();
    assert_eq!(
        out_rx.recv().await.expect("initial stream.opened").msg_type,
        MessageType::StreamOpened
    );
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::ConnectionConnected { .. })
    ));

    // This individual re-offer is within the configured cap, but accepting
    // both entries would allocate three stream handles cumulatively.
    let mut reoffer = connection_offer_env(
        "sess_cumulative_stream_cap",
        "conn_cumulative_stream_cap",
        &["opus"],
    );
    let mut payload: connection::ConnectionOffer = reoffer.decode_payload().unwrap();
    let mut second = payload.streams_offered[0].clone();
    second.id = "strm_2".into();
    payload.streams_offered.push(second);
    reoffer.payload = serde_json::to_value(payload).unwrap();
    in_tx.send(reoffer).await.unwrap();

    let error = out_rx.recv().await.expect("cumulative stream cap error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 429);
    assert_eq!(payload.reason, "too-many-streams");
    assert_eq!(coord.resource_snapshot().connections, 1);
    assert!(events_rx.try_recv().is_err());
}

#[tokio::test]
async fn fallback_stream_local_ids_are_peer_global_and_not_reused_after_teardown() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;

    let mut announced_ids = Vec::new();
    for suffix in ["a", "b"] {
        let sid = format!("sess_peer_{suffix}");
        let connid = format!("conn_peer_{suffix}");
        establish_session(&in_tx, &sid).await;
        in_tx
            .send(connection_offer_env(&sid, &connid, &["opus"]))
            .await
            .unwrap();
        in_tx
            .send(connection_ready_env(&sid, &connid))
            .await
            .unwrap();
        let opened = out_rx.recv().await.expect("stream.opened");
        assert_eq!(opened.msg_type, MessageType::StreamOpened);
        let payload: rvoip_uctp::payloads::stream::StreamOpened = opened.decode_payload().unwrap();
        announced_ids.push(payload.stream.stream_local_id);
        if suffix == "a" {
            in_tx
                .send(
                    UctpEnvelope::new(MessageType::ConnectionEnd, serde_json::json!({}))
                        .with_connid(connid),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    assert_eq!(announced_ids, vec![1, 2]);
}

#[tokio::test]
async fn external_media_binding_completes_before_stream_announcement() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    coord.enable_external_media_binding();
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::Authenticated { .. })
    ));
    establish_session(&in_tx, "sess_external_bind").await;
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::InboundInvite { .. })
    ));
    in_tx
        .send(connection_offer_env(
            "sess_external_bind",
            "conn_external_bind",
            &["opus"],
        ))
        .await
        .unwrap();
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::ConnectionOpened { .. })
    ));
    in_tx
        .send(connection_ready_env(
            "sess_external_bind",
            "conn_external_bind",
        ))
        .await
        .unwrap();

    let bind = events_rx.recv().await.expect("binding request");
    match bind {
        UctpSessionEvent::BindMediaStreams {
            sid,
            connid,
            streams,
            reply,
        } => {
            assert_eq!(sid.as_str(), "sess_external_bind");
            assert_eq!(connid.as_str(), "conn_external_bind");
            assert_eq!(streams.len(), 1);
            assert_eq!(streams[0].strm_id, "strm_1");
            reply.send(Ok(vec![77])).unwrap();
        }
        other => panic!("expected BindMediaStreams, got {other:?}"),
    }

    let opened = out_rx.recv().await.expect("stream.opened after binding");
    let payload: rvoip_uctp::payloads::stream::StreamOpened = opened.decode_payload().unwrap();
    assert_eq!(payload.stream.stream_local_id, 77);
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::ConnectionConnected { .. })
    ));
}

#[tokio::test]
async fn external_binding_failure_emits_no_stream_and_ready_can_retry() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    coord.enable_external_media_binding();
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_bind_retry").await;
    let _ = events_rx.recv().await;
    in_tx
        .send(connection_offer_env(
            "sess_bind_retry",
            "conn_bind_retry",
            &["opus"],
        ))
        .await
        .unwrap();
    let _ = events_rx.recv().await;

    in_tx
        .send(connection_ready_env("sess_bind_retry", "conn_bind_retry"))
        .await
        .unwrap();
    match events_rx.recv().await.expect("first binding request") {
        UctpSessionEvent::BindMediaStreams { reply, .. } => reply
            .send(Err(rvoip_uctp::UctpError::InvalidStreamBinding(
                "adapter-binding-failed",
            )))
            .unwrap(),
        other => panic!("expected BindMediaStreams, got {other:?}"),
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(75), out_rx.recv())
            .await
            .is_err(),
        "failed binding must not announce stream.opened"
    );
    assert!(events_rx.try_recv().is_err());

    in_tx
        .send(connection_ready_env("sess_bind_retry", "conn_bind_retry"))
        .await
        .unwrap();
    match events_rx.recv().await.expect("retry binding request") {
        UctpSessionEvent::BindMediaStreams { reply, .. } => {
            reply.send(Ok(vec![81])).unwrap();
        }
        other => panic!("expected BindMediaStreams retry, got {other:?}"),
    }
    let opened = out_rx.recv().await.expect("stream.opened after retry");
    let payload: rvoip_uctp::payloads::stream::StreamOpened = opened.decode_payload().unwrap();
    assert_eq!(payload.stream.stream_local_id, 81);
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::ConnectionConnected { .. })
    ));
}

#[tokio::test]
async fn duplicate_external_ids_are_rejected_atomically_and_can_be_corrected() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    coord.enable_external_media_binding();
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_duplicate_bind").await;
    let _ = events_rx.recv().await;

    let mut offer = connection_offer_env("sess_duplicate_bind", "conn_duplicate_bind", &["opus"]);
    let mut offer_payload: connection::ConnectionOffer = offer.decode_payload().unwrap();
    let mut second = offer_payload.streams_offered[0].clone();
    second.id = "strm_2".into();
    offer_payload.streams_offered.push(second);
    offer.payload = serde_json::to_value(offer_payload).unwrap();
    in_tx.send(offer).await.unwrap();
    let _ = events_rx.recv().await;

    in_tx
        .send(connection_ready_env(
            "sess_duplicate_bind",
            "conn_duplicate_bind",
        ))
        .await
        .unwrap();
    match events_rx.recv().await.expect("invalid binding request") {
        UctpSessionEvent::BindMediaStreams { streams, reply, .. } => {
            assert_eq!(streams.len(), 2);
            reply.send(Ok(vec![91, 91])).unwrap();
        }
        other => panic!("expected BindMediaStreams, got {other:?}"),
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(75), out_rx.recv())
            .await
            .is_err(),
        "duplicate IDs must not partially announce streams"
    );
    assert!(events_rx.try_recv().is_err());

    // ID 91 from the rejected batch was never reserved. Returning it in a
    // corrected all-unique batch proves validation is atomic.
    in_tx
        .send(connection_ready_env(
            "sess_duplicate_bind",
            "conn_duplicate_bind",
        ))
        .await
        .unwrap();
    match events_rx.recv().await.expect("corrected binding request") {
        UctpSessionEvent::BindMediaStreams { streams, reply, .. } => {
            assert_eq!(streams.len(), 2);
            reply.send(Ok(vec![91, 92])).unwrap();
        }
        other => panic!("expected corrected BindMediaStreams, got {other:?}"),
    }
    let mut announced = Vec::new();
    for _ in 0..2 {
        let opened = out_rx.recv().await.expect("corrected stream.opened");
        let payload: rvoip_uctp::payloads::stream::StreamOpened = opened.decode_payload().unwrap();
        announced.push(payload.stream.stream_local_id);
    }
    assert_eq!(announced, vec![91, 92]);
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::ConnectionConnected { .. })
    ));
}

#[tokio::test]
async fn connection_id_cannot_be_used_under_another_session() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let _coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_owner").await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_attacker").await;
    let _ = events_rx.recv().await;
    in_tx
        .send(connection_offer_env("sess_owner", "conn_bound", &["opus"]))
        .await
        .unwrap();
    let _ = events_rx.recv().await;

    let attack = UctpEnvelope {
        v: 1,
        msg_type: MessageType::DtmfSend,
        id: "env_cross_session_dtmf".into(),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_attacker".into()),
        connid: Some("conn_bound".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "digits": "9",
            "duration_ms": 100,
            "method": "rfc4733"
        }),
        signature: None,
    };
    in_tx.send(attack).await.unwrap();
    let error = out_rx.recv().await.expect("binding error");
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 403);
    assert_eq!(payload.reason, "connection-session-mismatch");
    assert!(events_rx.try_recv().is_err());
}

#[tokio::test]
async fn connection_end_accepts_connid_only_and_preserves_session_and_siblings() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_with_descriptor(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        answerer_with(&["opus"]),
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;
    establish_session(&in_tx, "sess_connection_end").await;
    let _ = events_rx.recv().await;

    for connid in ["conn_end_one", "conn_end_two"] {
        in_tx
            .send(connection_offer_env(
                "sess_connection_end",
                connid,
                &["opus"],
            ))
            .await
            .unwrap();
        assert!(matches!(
            events_rx.recv().await,
            Some(UctpSessionEvent::ConnectionOpened { .. })
        ));
    }
    assert_eq!(coord.resource_snapshot().connections, 2);

    // The protocol permits a connection-level end carrying only connid.
    in_tx
        .send(
            UctpEnvelope::new(MessageType::ConnectionEnd, serde_json::json!({}))
                .with_connid("conn_end_one"),
        )
        .await
        .unwrap();
    match events_rx.recv().await.expect("connection ended event") {
        UctpSessionEvent::ConnectionEnded { sid, connid, .. } => {
            assert_eq!(sid.as_str(), "sess_connection_end");
            assert_eq!(connid.as_str(), "conn_end_one");
        }
        other => panic!("expected ConnectionEnded, got {other:?}"),
    }
    let snapshot = coord.resource_snapshot();
    assert_eq!(snapshot.sessions, 1);
    assert_eq!(snapshot.connections, 1);
    assert!(
        out_rx.try_recv().is_err(),
        "connid-only end must not return 400"
    );

    // The sibling remains addressable after the first Connection ends.
    in_tx
        .send(
            UctpEnvelope::new(
                MessageType::DtmfSend,
                serde_json::json!({
                    "digits": "5",
                    "duration_ms": 80,
                    "method": "rfc4733"
                }),
            )
            .with_sid("sess_connection_end")
            .with_connid("conn_end_two"),
        )
        .await
        .unwrap();
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::Dtmf { connid, .. }) if connid.as_str() == "conn_end_two"
    ));
    assert_eq!(coord.resource_snapshot().connections, 1);
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
    establish_session(&in_tx, "sess_dtmf").await;

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
async fn inbound_message_send_preserves_binary_data_and_rejects_unsupported_reliability() {
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
    establish_session(&in_tx, "sess_message").await;
    in_tx
        .send(connection_offer_env(
            "sess_message",
            "conn_message",
            &["opus"],
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::MessageSend,
        id: format!("env_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: Some("conv_message".into()),
        sid: Some("sess_message".into()),
        connid: Some("conn_message".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "msg_id": message_id,
            "from": "part_alice",
            "to": "all",
            "content_type": "application/octet-stream",
            "label": "bridgefu.context.v1",
            "reliability": {"mode": "reliable_ordered"},
            "body": "AP8HKg==",
            "body_encoding": "base64",
            "attachments": []
        }),
        signature: None,
    };
    in_tx.send(env).await.unwrap();

    let message = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(UctpSessionEvent::DataMessage { connid, message }) = events_rx.recv().await
            {
                assert_eq!(connid.to_string(), "conn_message");
                return message;
            }
        }
    })
    .await
    .expect("data-message event timeout");
    assert_eq!(message.message_id.to_string(), message_id);
    assert_eq!(message.label, "bridgefu.context.v1");
    assert_eq!(message.content_type, "application/octet-stream");
    assert_eq!(message.bytes.as_ref(), &[0, 0xff, 7, 42]);

    let rejected_id = format!("env_{}", uuid::Uuid::new_v4().simple());
    let rejected = UctpEnvelope {
        v: 1,
        msg_type: MessageType::MessageSend,
        id: rejected_id.clone(),
        ts: Utc::now(),
        cid: Some("conv_message".into()),
        sid: Some("sess_message".into()),
        connid: Some("conn_message".into()),
        in_reply_to: None,
        payload: serde_json::json!({
            "msg_id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "from": "part_alice",
            "to": "all",
            "content_type": "text/plain",
            "label": "chat",
            "reliability": {"mode": "reliable_unordered"},
            "body": "unsupported",
            "body_encoding": "utf8",
            "attachments": []
        }),
        signature: None,
    };
    in_tx.send(rejected).await.unwrap();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let envelope = out_rx.recv().await.expect("coordinator output closed");
            if envelope.in_reply_to.as_deref() == Some(rejected_id.as_str()) {
                return envelope;
            }
        }
    })
    .await
    .expect("unsupported-reliability error timeout");
    assert_eq!(error.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = error.decode_payload().unwrap();
    assert_eq!(payload.code, 422);
    assert_eq!(payload.category, "capability");
    assert_eq!(payload.reason, "unsupported-reliability");
    while let Ok(event) = events_rx.try_recv() {
        assert!(
            !matches!(event, UctpSessionEvent::DataMessage { .. }),
            "a rejected message must not reach the adapter event stream"
        );
    }
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
    establish_session(&in_tx, "sess_q").await;

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
    // a new token for the same owner. Coordinator validates,
    // updates `PeerAuthState`, and replies with a fresh `auth.session`
    // envelope. The original identity_id / participant_id are
    // preserved across the refresh (continuity of logical session).
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        Arc::new(StablePrincipalValidator),
    );

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

    // Send auth.refresh with a different credential for the same owner.
    let refresh_env = UctpEnvelope::new(
        MessageType::AuthRefresh,
        serde_json::to_value(auth::AuthRefresh {
            method: "bearer".into(),
            credential: "refreshed-token-xyz".into(),
            actor_token: None,
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
    let remaining = payload.expires_at - Utc::now();
    assert!(remaining <= chrono::Duration::hours(1));
    assert!(remaining > chrono::Duration::minutes(59));

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
async fn auth_refresh_rejects_ownership_switch_and_preserves_prior_principal() {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let coord = UctpCoordinator::start(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        Arc::new(StablePrincipalValidator),
    );
    drive_auth_handshake(&in_tx, &mut out_rx).await;
    let _ = events_rx.recv().await;

    in_tx
        .send(UctpEnvelope::new(
            MessageType::AuthRefresh,
            serde_json::to_value(auth::AuthRefresh {
                method: "bearer".into(),
                credential: "other-owner".into(),
                actor_token: None,
            })
            .unwrap(),
        ))
        .await
        .unwrap();

    let reply = out_rx.recv().await.expect("refresh rejection");
    assert_eq!(reply.msg_type, MessageType::Error);
    let error: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(error.code, 401);
    assert_eq!(error.reason, "refresh-failed");
    assert_eq!(
        coord
            .authenticated_principal()
            .expect("prior principal retained")
            .subject,
        "user:alice"
    );

    in_tx
        .send(session_invite_env("sess_after_owner_reject", "conv_owner"))
        .await
        .unwrap();
    assert!(matches!(
        events_rx.recv().await,
        Some(UctpSessionEvent::InboundInvite { .. })
    ));
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
            actor_token: None,
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
