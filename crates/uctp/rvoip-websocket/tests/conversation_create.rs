//! UP-3 — `conversation.create` over WebSocket without a pre-opened Conversation.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::conversation::ConversationState;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConversationId, ParticipantId};
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::SessionMedium;
use rvoip_core::{Config, Orchestrator};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::auth;
use rvoip_uctp::payloads::conversation::{
    ConversationCreate, ConversationOpened, ConversationPolicy,
};
use rvoip_uctp::payloads::session::SessionInvite;
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use tokio::net::TcpListener;
use url::Url;

async fn auth_client(
    client: &UctpWsClient,
    inbound: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: "env_hello".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_ws_conv".into(),
                    kind: "browser".into(),
                    platform: "test".into(),
                    sdk_version: "test/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("challenge timeout")
        .expect("challenge");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthResponse,
            id: "env_response".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: Some(challenge.id),
            payload: serde_json::to_value(auth::AuthResponse {
                method: "bearer".into(),
                credential: "test-token".into(),
                actor_token: None,
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("response");
    let session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("auth.session");
    assert_eq!(session.msg_type, MessageType::AuthSession);
}

#[tokio::test]
async fn conversation_create_then_invite_reuses_open_conversation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");
    let orchestrator = Orchestrator::new(Config::default());
    let adapter = UctpWsAdapter::new(
        UctpWsConfig::new(listener, bearer_stub()).with_orchestrator(Arc::clone(&orchestrator)),
    )
    .await
    .expect("adapter");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    let mut events = orchestrator.subscribe_events();

    let url = Url::parse(&format!("ws://{server_addr}")).expect("url");
    let client = UctpWsClient::connect(&url).await.expect("connect");
    let mut inbound = client.take_inbound().expect("inbound");
    auth_client(&client, &mut inbound).await;

    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::ConversationCreate,
            id: "env_create".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(ConversationCreate {
                tenant_id: "ten_local".into(),
                policy: ConversationPolicy::Persistent,
                idle_close_secs: None,
                metadata: serde_json::json!({}),
                initial_participants: vec![],
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("create");

    let opened = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("opened timeout")
        .expect("opened");
    assert_eq!(opened.msg_type, MessageType::ConversationOpened);
    let cid = opened.cid.clone().expect("assigned cid");
    let payload: ConversationOpened = opened.decode_payload().expect("opened payload");
    assert_eq!(payload.policy, ConversationPolicy::Persistent);

    client
        .send(
            UctpEnvelope::new(
                MessageType::ConversationCreate,
                serde_json::to_value(ConversationCreate {
                    tenant_id: "ten_local".into(),
                    policy: ConversationPolicy::Persistent,
                    idle_close_secs: None,
                    metadata: serde_json::json!({}),
                    initial_participants: vec![],
                })
                .unwrap(),
            )
            .with_cid(cid.clone()),
        )
        .await
        .expect("second create");
    let opened_again = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("idempotent opened timeout")
        .expect("idempotent opened");
    assert_eq!(opened_again.msg_type, MessageType::ConversationOpened);
    assert_eq!(opened_again.cid.as_deref(), Some(cid.as_str()));

    client
        .send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(SessionInvite {
                    from: "part_widget".into(),
                    to: vec!["part_ai".into()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::Value::Object(Default::default()),
                })
                .unwrap(),
            )
            .with_cid(cid.clone())
            .with_sid("sess_widget_1"),
        )
        .await
        .expect("invite");

    let connection_id = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event timeout")
            .expect("event bus");
        match event {
            Event::ConnectionInbound { connection_id, .. } => break connection_id,
            _ => continue,
        }
    };

    let conversation_id = ConversationId::from_string(cid.clone());
    let session_id = orchestrator
        .start_session(conversation_id.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session on created conversation");
    let participant = ParticipantId::new();
    orchestrator
        .join_session(
            session_id.clone(),
            participant.clone(),
            ParticipantKind::Human,
            ParticipantRole::Customer,
        )
        .await
        .expect("join");
    orchestrator
        .route_inbound_connection(
            connection_id,
            InboundAction::Accept {
                session_id: session_id.clone(),
                participant_id: participant,
            },
        )
        .await
        .expect("accept");

    let conversation = orchestrator
        .conversation(&conversation_id)
        .expect("conversation");
    let conversation = conversation.read().expect("lock");
    assert_eq!(conversation.state, ConversationState::Open);
    assert_eq!(conversation.sessions.len(), 1);
    assert_eq!(orchestrator.live_conversation_ids().len(), 1);
}

struct ForceCid(String);

#[async_trait::async_trait]
impl rvoip_websocket::ConversationCreateHook for ForceCid {
    async fn resolve_cid(
        &self,
        requested_cid: Option<String>,
        _tenant_id: String,
        _metadata: serde_json::Value,
    ) -> Option<String> {
        requested_cid
            .filter(|cid| !cid.is_empty())
            .or_else(|| Some(self.0.clone()))
    }
}

#[tokio::test]
async fn conversation_create_hook_assigns_cid_before_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");
    let orchestrator = Orchestrator::new(Config::default());
    let forced = ConversationId::new().to_string();
    let adapter = UctpWsAdapter::new(
        UctpWsConfig::new(listener, bearer_stub())
            .with_orchestrator(Arc::clone(&orchestrator))
            .with_conversation_create_hook(Arc::new(ForceCid(forced.clone()))),
    )
    .await
    .expect("adapter");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");

    let url = Url::parse(&format!("ws://{server_addr}")).expect("url");
    let client = UctpWsClient::connect(&url).await.expect("connect");
    let mut inbound = client.take_inbound().expect("inbound");
    auth_client(&client, &mut inbound).await;

    client
        .send(UctpEnvelope {
            v: 1,
            msg_type: MessageType::ConversationCreate,
            id: "env_create_hook".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::to_value(ConversationCreate {
                tenant_id: "ten_local".into(),
                policy: ConversationPolicy::Persistent,
                idle_close_secs: None,
                metadata: serde_json::json!({}),
                initial_participants: vec![],
            })
            .unwrap(),
            signature: None,
        })
        .await
        .expect("create");

    let opened = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("opened timeout")
        .expect("opened");
    assert_eq!(opened.msg_type, MessageType::ConversationOpened);
    assert_eq!(opened.cid.as_deref(), Some(forced.as_str()));
    let conversation = orchestrator
        .conversation(&ConversationId::from_string(forced.clone()))
        .expect("hook cid opened");
    assert_eq!(
        conversation.read().expect("lock").state,
        ConversationState::Open
    );
}
