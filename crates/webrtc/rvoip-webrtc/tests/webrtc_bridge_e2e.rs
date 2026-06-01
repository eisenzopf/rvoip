//! Phase 9 bridge demo — WHIP WebRTC leg bridged to a synthetic QUIC leg via the orchestrator.

#![cfg(feature = "signaling-whip")]

#[path = "support/mock_quic_leg.rs"]
mod mock_quic_leg;

use std::sync::Arc;
use std::time::Duration;

use mock_quic_leg::MockQuicLeg;
use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::Direction;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcServerBuilder};

#[tokio::test]
async fn whip_webrtc_bridged_to_quic_leg_via_orchestrator() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let session_id = SessionId::new();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("build server");

    let quic_leg = MockQuicLeg::new();
    let (quic_conn, _quic_stream) = quic_leg
        .provision_inbound(session_id.clone(), "opus")
        .await;

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register webrtc");
    orchestrator
        .register(quic_leg as Arc<dyn ConnectionAdapter>)
        .expect("register quic mock");

    let mut events = orchestrator.subscribe_events();

    // WHIP publisher (foreign WebRTC client).
    let publisher = WebRtcAdapter::new(WebRtcConfig::loopback());
    let pub_handle = publisher
        .originate(OriginateRequest {
            session_id: session_id.clone(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: publisher.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let pub_conn = pub_handle.connection.id.clone();
    let offer_sdp = publisher.local_sdp(&pub_conn).expect("offer");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let whip_base = format!("http://{}", server.whip_addr().expect("whip"));
    let resp = client
        .post(format!("{whip_base}/whip/live"))
        .header("content-type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .expect("whip post");
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let answer_sdp = resp.text().await.expect("answer");
    publisher
        .apply_remote_answer(pub_conn.clone(), &answer_sdp)
        .await
        .expect("apply answer");

    let webrtc_conn = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event timeout")
            .expect("bus");
        if let Event::ConnectionInbound { connection_id, .. } = event {
            if server.adapter().routes().contains_key(&connection_id) {
                break connection_id;
            }
        }
    };

    orchestrator
        .route_inbound_connection(
            webrtc_conn.clone(),
            InboundAction::Accept {
                session_id: session_id.clone(),
                participant_id: ParticipantId::new(),
            },
        )
        .await
        .expect("accept webrtc");

    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("connected timeout")
            .expect("bus");
        if let Event::ConnectionConnected { connection_id, .. } = &event {
            if *connection_id == webrtc_conn {
                break;
            }
        }
    }

    let _bridge_id = orchestrator
        .bridge_connections(webrtc_conn.clone(), quic_conn)
        .await
        .expect("bridge");

    let bridged = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("bridged event timeout")
        .expect("bus");
    assert!(
        matches!(bridged, Event::ConnectionsBridged { .. }),
        "expected ConnectionsBridged, got {bridged:?}"
    );

    // Frame flow through real WebRTC ICE is verified in rvoip-core's mock bridge
    // tests; here we prove WHIP → orchestrator → bridge_connections wiring.
    publisher.accept(pub_conn).await.expect("publisher accept");

    server.shutdown().await;
}
