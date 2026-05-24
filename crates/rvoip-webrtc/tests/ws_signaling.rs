//! WebSocket JSON signaling — outbound answer routing (feature `signaling-ws`).

#![cfg(feature = "signaling-ws")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::signaling::websocket::{serve_listener, SignalingMessage};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn ws_inbound_offer_returns_answer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let ws_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        serve_listener(listener, ws_adapter)
            .await
            .expect("ws serve")
    });

    let mut events = adapter.subscribe_events();

    let offerer = WebRtcAdapter::new(config);
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
        })
        .await
        .expect("originate");
    let offer_sdp = offerer.local_sdp(&handle.connection.id).expect("offer sdp");

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("ws connect");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "offer".into(),
            sdp: offer_sdp,
            connection_id: String::new(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send offer");

    let answer_text = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("answer timeout")
        .expect("ws frame")
        .expect("ws ok");
    let answer: SignalingMessage = serde_json::from_str(answer_text.to_text().unwrap()).unwrap();
    assert_eq!(answer.msg_type, "answer");
    assert!(!answer.sdp.is_empty());
    assert!(!answer.connection_id.is_empty());

    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel");
    assert!(matches!(event, AdapterEvent::InboundConnection { .. }));

    server.abort();
}

#[tokio::test]
async fn ws_outbound_answer_routes_by_connection_id() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

    let handle = adapter
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: adapter.capabilities(),
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id.clone();
    let offer_sdp = adapter.local_sdp(&conn_id).expect("offer sdp");

    let answerer = WebRtcAdapter::new(config);
    let inbound_id = answerer
        .apply_remote_offer(&offer_sdp)
        .await
        .expect("answer");
    let answer_sdp = answerer.local_sdp(&inbound_id).expect("answer sdp");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let ws_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        serve_listener(listener, ws_adapter)
            .await
            .expect("ws serve")
    });

    let mut events = adapter.subscribe_events();

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("ws connect");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "answer".into(),
            sdp: answer_sdp,
            connection_id: conn_id.to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send answer");

    let ack = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("ack timeout")
        .expect("ack frame")
        .expect("ack ok");
    let ack_msg: SignalingMessage = serde_json::from_str(ack.to_text().unwrap()).unwrap();
    assert_eq!(ack_msg.msg_type, "ack");

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel");
    assert!(matches!(event, AdapterEvent::Connected { .. }));

    server.abort();
}
