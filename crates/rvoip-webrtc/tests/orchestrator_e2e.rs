//! Phase 9 — WebRtcServer + Orchestrator dual-role E2E.
//!
//! WHIP publish → orchestrator `ConnectionInbound` → accept → `ConnectionConnected`.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::Direction;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcServerBuilder};

#[tokio::test]
async fn whip_inbound_flows_through_orchestrator() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("build server");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register WebRtc adapter");

    let mut events = orchestrator.subscribe_events();

    let publisher = WebRtcAdapter::new(WebRtcConfig::loopback());
    let handle = publisher
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: publisher.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id.clone();
    let offer_sdp = publisher.local_sdp(&conn_id).expect("offer sdp");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client");
    let whip_base = format!("http://{}", server.whip_addr().expect("whip addr"));
    let resp = client
        .post(format!("{whip_base}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .expect("whip post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let answer_sdp = resp.text().await.expect("answer body");
    assert!(answer_sdp.contains("m=audio"));

    publisher
        .apply_remote_answer(conn_id, &answer_sdp)
        .await
        .expect("publisher applies answer");

    let inbound = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("inbound event timeout")
        .expect("event bus open");
    let server_conn = match inbound {
        Event::ConnectionInbound { connection_id, .. } => connection_id,
        other => panic!("expected ConnectionInbound, got {other:?}"),
    };

    orchestrator
        .route_inbound_connection(
            server_conn.clone(),
            InboundAction::Accept {
                session_id: SessionId::new(),
                participant_id: ParticipantId::new(),
            },
        )
        .await
        .expect("orchestrator accept");

    let connected = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("connected event timeout")
        .expect("event bus open");
    assert!(
        matches!(connected, Event::ConnectionConnected { ref connection_id, .. } if *connection_id == server_conn),
        "expected ConnectionConnected for {server_conn}, got {connected:?}"
    );

    server.shutdown().await;
}
