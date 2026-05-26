//! Gap plan §5.1 — AAuth validator integration.
//!
//! End-to-end exercise of the UCTP coordinator's AAuth path:
//! `auth.hello` → `auth.challenge` → `auth.response { method: "aauth",
//! credential: <subject_token>, actor_token: Some(<actor_token>) }`
//! → `auth.session` with `assurance = "user-authorized"`.
//!
//! The coordinator's `start_full_with_aauth` constructor wires an
//! `AAuthValidator`; the test injects mock subject + actor validators
//! that yield UserAuthorized / actor claims with known scopes, then
//! asserts the wire response carries the combined assurance and the
//! emitted `UctpSessionEvent::Authenticated` carries
//! `IdentityAssurance::UserAuthorized` with the actor as `identity`
//! and the subject as `user_id`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rvoip_auth_core::{
    bearer::{BearerAuthError, BearerValidator},
    AAuthValidator, ActorClaims, ActorTokenValidator,
};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::IdentityId;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::auth,
    state::{
        default_v0_descriptor, rejecting_handler, UctpCoordinator, UctpCoordinatorCaps,
        UctpSessionEvent, ENVELOPE_CHANNEL_CAP,
    },
    types::MessageType,
};
use tokio::sync::mpsc;

struct StaticSubject {
    user_id: IdentityId,
    scopes: Vec<String>,
}

#[async_trait]
impl BearerValidator for StaticSubject {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(IdentityAssurance::UserAuthorized {
            identity: self.user_id.clone(),
            user_id: self.user_id.clone(),
            scopes: self.scopes.clone(),
        })
    }
}

struct StaticActor {
    identity: IdentityId,
    scopes: Vec<String>,
}

#[async_trait]
impl ActorTokenValidator for StaticActor {
    async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(ActorClaims {
            identity: self.identity.clone(),
            scopes: self.scopes.clone(),
        })
    }
}

fn id(s: &str) -> IdentityId {
    IdentityId::from_string(s.to_string())
}

fn hello() -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: "env_aauth_hello".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_aauth_test".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "aauth-test/0.1".into(),
            },
            auth_methods: vec!["aauth".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    signature: None,
    }
}

fn aauth_response(challenge_id: String, subject: &str, actor: Option<&str>) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: "env_aauth_resp".into(),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(challenge_id),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "aauth".into(),
            credential: subject.into(),
            actor_token: actor.map(|s| s.into()),
        })
        .unwrap(),
    signature: None,
    }
}

#[tokio::test]
async fn aauth_response_yields_user_authorized_assurance() {
    let subject = Arc::new(StaticSubject {
        user_id: id("user:alice"),
        scopes: vec!["calls.write".into()],
    });
    let actor = Arc::new(StaticActor {
        identity: id("agent:assistant-7"),
        scopes: vec!["calls.transfer".into()],
    });
    let aauth = AAuthValidator::new(subject.clone(), actor);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full_with_aauth(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        subject as Arc<dyn BearerValidator>,
        aauth,
        Arc::new(default_v0_descriptor()),
        rejecting_handler(),
        UctpCoordinatorCaps::default(),
    );

    // 1. auth.hello → auth.challenge
    in_tx.send(hello()).await.unwrap();
    let challenge = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("challenge timeout")
        .expect("out_rx closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);

    // 2. auth.response (method=aauth) → auth.session
    in_tx
        .send(aauth_response(challenge.id, "subject-token", Some("actor-token")))
        .await
        .unwrap();
    let session = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("session timeout")
        .expect("out_rx closed");
    assert_eq!(session.msg_type, MessageType::AuthSession);

    let session_payload: auth::AuthSession = session.decode_payload().unwrap();
    assert_eq!(
        session_payload.assurance, "user-authorized",
        "AAuth must elevate assurance to user-authorized"
    );

    // 3. Coordinator should also have emitted the Authenticated event
    //    with the combined IdentityAssurance.
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events_rx.recv())
        .await
        .expect("auth event timeout")
        .expect("events_rx closed");
    match event {
        UctpSessionEvent::Authenticated { assurance, .. } => match assurance {
            IdentityAssurance::UserAuthorized {
                user_id,
                identity,
                scopes,
            } => {
                assert_eq!(user_id.as_str(), "user:alice");
                assert_eq!(identity.as_str(), "agent:assistant-7");
                assert!(scopes.contains(&"calls.write".to_string()));
                assert!(scopes.contains(&"calls.transfer".to_string()));
            }
            other => panic!("expected UserAuthorized; got {other:?}"),
        },
        other => panic!("expected Authenticated event; got {other:?}"),
    }
}

#[tokio::test]
async fn aauth_with_missing_actor_token_rejects_401() {
    let subject = Arc::new(StaticSubject {
        user_id: id("user:alice"),
        scopes: vec![],
    });
    let actor = Arc::new(StaticActor {
        identity: id("agent:7"),
        scopes: vec![],
    });
    let aauth = AAuthValidator::new(subject.clone(), actor);

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start_full_with_aauth(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        subject as Arc<dyn BearerValidator>,
        aauth,
        Arc::new(default_v0_descriptor()),
        rejecting_handler(),
        UctpCoordinatorCaps::default(),
    );

    in_tx.send(hello()).await.unwrap();
    let challenge = out_rx.recv().await.unwrap();

    // No actor_token sent — must surface as 401.
    in_tx
        .send(aauth_response(challenge.id, "subject-token", None))
        .await
        .unwrap();
    let reply = out_rx.recv().await.unwrap();
    assert_eq!(reply.msg_type, MessageType::Error);
    let payload: rvoip_uctp::payloads::control::Error = reply.decode_payload().unwrap();
    assert_eq!(payload.code, 401);
}
