//! Phase 4 bridge integration smoke per `UCTP_IMPLEMENTATION_PLAN.md` §6.3.
//!
//! Brings up the orchestrator with all three adapters in-process, dials
//! a UCTP-over-QUIC client at it, and asserts:
//! - The cross-transport adapter list contains all three transports.
//! - `Event::ConnectionInbound` fires on the orchestrator's event bus
//!   when the UCTP client sends `session.invite`.
//!
//! The full SIP-side leg is exercised by the `sip_caller` demo binary;
//! covering it in-process would require the SIP listen port to actually
//! be reachable from a same-process StreamPeer (workable but adds 100+
//! lines for marginal value over the manual demo). v0 smoke covers the
//! load-bearing claim: cross-transport event normalization works.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::connection::Transport;
use rvoip_core::events::Event;
use rvoip_core::{Config, Orchestrator};
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session};
use rvoip_uctp::substrate::{
    dev_client_config_trusting, dispatch_by_alpn, make_server_endpoint, self_signed_for_dev,
};
use rvoip_uctp::types::MessageType;
use rvoip_webtransport::{UctpWtAdapter, UctpWtConfig};
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use tokio::net::TcpListener;
use url::Url;

const ALPN_UCTP: &[u8] = b"uctp/1";
const ALPN_H3: &[u8] = b"h3";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn bridge_smoke_three_adapters_register_and_fire_events() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // --- 1. Shared quinn::Endpoint (port 0 — kernel assigns) ---
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self-signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("tls");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec(), ALPN_H3.to_vec()];

    let quinn_ep = Arc::new(
        make_server_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(tls),
            quinn::TransportConfig::default(),
        )
        .expect("server endpoint"),
    );
    let server_addr = quinn_ep.local_addr().expect("local_addr");

    let mut routes = dispatch_by_alpn(Arc::clone(&quinn_ep), &[ALPN_UCTP, ALPN_H3])
        .expect("dispatcher");
    let quic_rx = routes.take(ALPN_UCTP).unwrap();
    let wt_rx = routes.take(ALPN_H3).unwrap();

    // --- 2. Build the three adapters ---
    let quic_adapter = UctpQuicAdapter::new(UctpQuicConfig::new(
        Arc::clone(&quinn_ep),
        quic_rx,
        bearer_stub(),
    ))
    .await
    .expect("quic adapter");
    let wt_adapter = UctpWtAdapter::new(UctpWtConfig::new(
        Arc::clone(&quinn_ep),
        wt_rx,
        bearer_stub(),
    ))
    .await
    .expect("wt adapter");

    // WebSocket on its own TCP port (kernel-assigned).
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.expect("ws bind");
    let ws_addr = ws_listener.local_addr().expect("ws addr");
    let ws_adapter = UctpWsAdapter::new(UctpWsConfig::new(ws_listener, bearer_stub()))
        .await
        .expect("ws adapter");

    // SIP on its own UDP port. Kernel-assigned port 0 keeps the test
    // runnable in parallel with other workspace tests.
    let sip_coordinator =
        UnifiedCoordinator::new(SipConfig::on("bridge-smoke", "127.0.0.1".parse().unwrap(), 0))
            .await
            .expect("sip coordinator");
    let sip_adapter = SipAdapter::new(sip_coordinator).await.expect("sip adapter");

    // --- 3. Register, verify cross-transport visibility ---
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(quic_adapter).expect("register quic");
    orchestrator.register(wt_adapter).expect("register wt");
    orchestrator.register(ws_adapter).expect("register ws");
    orchestrator.register(sip_adapter).expect("register sip");

    for transport in [
        Transport::Quic,
        Transport::WebTransport,
        Transport::WebSocket,
        Transport::Sip,
    ] {
        let found = orchestrator.adapter(transport).expect("registered");
        assert_eq!(found.transport(), transport);
    }

    let mut events = orchestrator.subscribe_events();

    // --- 4. Dial a UCTP-QUIC client, fire session.invite ---
    let client_socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("client bind");
    let client_ep = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        client_socket,
        Arc::new(quinn::TokioRuntime),
    )
    .expect("client endpoint");
    let client_cfg = dev_client_config_trusting(&cert_der).expect("client cfg");
    let client = UctpQuicClient::connect(
        &client_ep,
        server_addr,
        "localhost",
        Arc::new(client_cfg),
    )
    .await
    .expect("client connect");

    let invite = UctpEnvelope::new(
        MessageType::SessionInvite,
        serde_json::to_value(session::SessionInvite {
            from: "part_smoke".into(),
            to: vec!["sip:server@127.0.0.1".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    )
    .with_sid("sess_bridge_smoke".to_string())
    .with_cid("conv_bridge_smoke".to_string());
    client.send(invite).await.expect("send invite");

    // --- 5. Assert ConnectionInbound fires on the cross-transport bus for QUIC ---
    let mut saw_inbound = 0;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { .. })) => {
                saw_inbound += 1;
                break;
            }
            Ok(Ok(_other)) => continue,
            Ok(Err(_)) | Err(_) => continue,
        }
    }
    assert!(
        saw_inbound >= 1,
        "expected Event::ConnectionInbound on orchestrator bus from QUIC client within 6s"
    );

    // --- 6. Dial via WebSocket too; assert another ConnectionInbound ---
    let ws_url = Url::parse(&format!("ws://{}", ws_addr)).expect("parse ws url");
    let ws_client = UctpWsClient::connect(&ws_url).await.expect("ws connect");
    let ws_invite = UctpEnvelope::new(
        MessageType::SessionInvite,
        serde_json::to_value(session::SessionInvite {
            from: "part_ws_smoke".into(),
            to: vec!["sip:server@127.0.0.1".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
    )
    .with_sid("sess_ws_smoke".to_string())
    .with_cid("conv_ws_smoke".to_string());
    ws_client.send(ws_invite).await.expect("ws send invite");

    // The cross-transport `Event::ConnectionInbound` doesn't carry the
    // transport directly (the orchestrator normalizes it down to just
    // a connection_id + timestamp). Just assert a second
    // ConnectionInbound fires — the WS adapter test itself
    // (`crates/rvoip-websocket/tests/adapter.rs`) verifies the
    // transport label.
    let mut saw_ws_inbound = false;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(Event::ConnectionInbound { .. })) => {
                saw_ws_inbound = true;
                break;
            }
            Ok(Ok(_other)) => continue,
            Ok(Err(_)) | Err(_) => continue,
        }
    }
    assert!(
        saw_ws_inbound,
        "expected second Event::ConnectionInbound (from WS) within 6s"
    );

    // Touch the auth ID field to silence the unused import lint when the
    // test bypasses auth (we send session.invite without auth.hello first;
    // the coordinator still routes it because v0 doesn't enforce ordering).
    let _ = auth::AuthHello {
        device: auth::Device {
            id: "".into(),
            kind: "".into(),
            platform: "".into(),
            sdk_version: "".into(),
        },
        auth_methods: vec![],
        capabilities: serde_json::Value::Object(Default::default()),
    };
    let _ = Utc::now();
}
