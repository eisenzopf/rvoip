//! Gap plan §2.4 — WS envelope-level SDP interception.
//!
//! Companion to `ws_bridge_flow.rs`. Where that test drives SDP directly
//! via `adapter.bridge_for(...)` for diagnostics, this test drives the
//! WebRTC handshake purely through wire `connection.offer` /
//! `connection.answer` envelopes — the WS server's interception layer
//! is responsible for plumbing the SDP into / out of the per-Connection
//! `WebRtcMediaBridge`.
//!
//! Architecture:
//! - One WS server adapter, one WS client.
//! - Client drives bearer auth + sends `session.invite`.
//! - Client constructs an offerer `WebRtcMediaBridge` locally, gets its
//!   local SDP, and sends `connection.offer` (with substrate_setup
//!   carrying the SDP) over the WS wire.
//! - Server's inbound pump intercepts the offer, applies the SDP to its
//!   per-route answerer bridge, and autonomously emits a
//!   `connection.answer` envelope. The outbound pump mutates that
//!   answer to inject the answerer's local SDP into `substrate_setup`.
//! - Client receives `connection.answer`, parses the SDP from
//!   `substrate_setup`, applies to the offerer bridge.
//! - Both sides reach `wait_connected`. No `bridge_for` calls on the
//!   server side from this test.

#![cfg(feature = "media-webrtc")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::events::Event;
use rvoip_core::ids::ConnectionId;
use rvoip_core::{Config, Orchestrator};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{
    auth,
    connection::{ConnectionAnswer, ConnectionOffer, StreamOffer, WebRtcSubstrateSetup},
    session::SessionInvite,
};
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig, WebRtcMediaBridge};
use tokio::net::TcpListener;
use url::Url;

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn auth_hello() -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_hello_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_ws_env".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "ws-env/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    }
}

fn auth_response(in_reply_to: String) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_resp_{}", uuid::Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(in_reply_to),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
            actor_token: None,
        })
        .unwrap(),
        signature: None,
    }
}

fn invite(sid: &str, participant: &str) -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_inv_{}", sid),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(SessionInvite {
            from: participant.into(),
            to: vec!["part_env_server".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn envelope_only_sdp_exchange_completes_dtls_handshake() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- Server adapter ---
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local_addr");
    let adapter = UctpWsAdapter::new(UctpWsConfig::new(listener, bearer_stub()))
        .await
        .expect("adapter");
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register ws");
    let mut events = orchestrator.subscribe_events();

    // --- Client dials in ---
    let url = Url::parse(&format!("ws://{}", server_addr)).expect("parse url");
    let client = UctpWsClient::connect(&url).await.expect("client connect");
    let mut inbound = client.take_inbound().expect("take inbound");

    // --- Bearer auth ---
    client.send(auth_hello()).await.expect("hello");
    let ch = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(ch.msg_type, MessageType::AuthChallenge);
    client.send(auth_response(ch.id)).await.expect("auth resp");
    let sess = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    assert_eq!(sess.msg_type, MessageType::AuthSession);

    // --- session.invite ---
    let sid = "sess_env_test";
    client
        .send(invite(sid, "part_env_client"))
        .await
        .expect("invite");

    // Wait for the server to materialize the per-connection route /
    // bridge (the WS adapter spawns the answerer-bridge construction
    // asynchronously off InboundInvite).
    let conn_id: ConnectionId = loop {
        match tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { connection_id, .. })) => break connection_id,
            _ => continue,
        }
    };

    // --- Client constructs its offerer bridge and gets local SDP ---
    let offerer = Arc::new(
        WebRtcMediaBridge::new_offerer()
            .await
            .expect("offerer bridge"),
    );
    let offer_setup = offerer
        .local_substrate_setup()
        .await
        .expect("offerer local SDP");

    // --- Send `connection.offer` envelope with substrate_setup ---
    let offer_env = UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionOffer,
        id: "env_offer_envtest".into(),
        ts: Utc::now(),
        cid: Some(format!("conv_{}", sid)),
        sid: Some(sid.into()),
        connid: Some("conn_envtest".into()),
        in_reply_to: None,
        payload: serde_json::to_value(ConnectionOffer {
            by_participant: "part_env_client".into(),
            substrate: "websocket+webrtc".into(),
            capabilities: serde_json::Value::Object(Default::default()),
            streams_offered: vec![StreamOffer {
                id: "strm_audio".into(),
                kind: "audio".into(),
                direction: "send-recv".into(),
                codec_preferences: vec!["opus".into()],
            }],
            substrate_setup: serde_json::to_value(offer_setup).unwrap(),
        })
        .unwrap(),
        signature: None,
    };
    client.send(offer_env).await.expect("send offer");

    // --- Receive `connection.answer` with substrate_setup populated by
    //     the server's outbound interception ---
    let answer_env = loop {
        let env = tokio::time::timeout(Duration::from_secs(10), inbound.recv())
            .await
            .expect("answer timeout")
            .expect("inbound closed");
        if env.msg_type == MessageType::ConnectionAnswer {
            break env;
        }
    };
    let answer_payload: ConnectionAnswer = answer_env.decode_payload().expect("decode answer");
    let setup: WebRtcSubstrateSetup = serde_json::from_value(answer_payload.substrate_setup)
        .expect(
            "server must inject answerer SDP into connection.answer.substrate_setup via outbound interception",
        );
    assert_eq!(setup.kind, "websocket+webrtc");
    assert!(
        setup.sdp.starts_with("v="),
        "answerer SDP malformed: {}",
        setup.sdp
    );

    // --- Apply server's answer to the offerer ---
    offerer
        .set_remote_substrate_setup(setup)
        .await
        .expect("offerer applies answer");

    // --- Both sides should reach DTLS-connected ---
    let connect_timeout = Duration::from_secs(15);
    offerer
        .wait_connected(connect_timeout)
        .await
        .expect("offerer connected");

    // Server-side answerer should also be connected. We can't get the
    // handle here without bridge_for (which is the whole point of the
    // test), but we can observe via adapter.streams() being non-empty
    // — the server's ready-watcher only registers the media stream
    // after wait_connected succeeds.
    let mut got_stream = false;
    for _ in 0..200u32 {
        if !adapter
            .streams(conn_id.clone())
            .await
            .expect("streams query")
            .is_empty()
        {
            got_stream = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        got_stream,
        "server-side answerer bridge never reached connected via envelope-only SDP exchange"
    );
}
