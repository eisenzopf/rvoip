//! WebSocket JSON signaling — outbound answer routing (feature `signaling-ws`).

#![cfg(feature = "signaling-ws")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_webrtc::signaling::websocket::{serve_listener, SignalingMessage};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

async fn wait_route_absent(adapter: &WebRtcAdapter, connection_id: &ConnectionId) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while adapter.routes().contains_key(connection_id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("leased WebSocket route cleanup timeout");
}

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
            transport: None,
            context: Default::default(),
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
            candidate: String::new(),
            request_id: String::new(),
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
async fn ws_offer_with_connection_id_renegotiates_existing_connection() {
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

    let offerer = WebRtcAdapter::new(config);
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
            transport: None,
            context: Default::default(),
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
            sdp: offer_sdp.clone(),
            connection_id: String::new(),
            candidate: String::new(),
            request_id: String::new(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send initial offer");

    let answer_text = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("answer timeout")
        .expect("ws frame")
        .expect("ws ok");
    let answer: SignalingMessage = serde_json::from_str(answer_text.to_text().unwrap()).unwrap();
    assert_eq!(answer.msg_type, "answer");
    let connection_id = answer.connection_id;
    assert!(!connection_id.is_empty());

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "offer".into(),
            sdp: offer_sdp,
            connection_id: connection_id.clone(),
            candidate: String::new(),
            request_id: String::new(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send renegotiation offer");

    let reneg_answer_text = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("renegotiation answer timeout")
        .expect("renegotiation ws frame")
        .expect("renegotiation ws ok");
    let reneg_answer: SignalingMessage =
        serde_json::from_str(reneg_answer_text.to_text().unwrap()).unwrap();
    assert_eq!(reneg_answer.msg_type, "answer");
    assert_eq!(reneg_answer.connection_id, connection_id);
    assert!(!reneg_answer.sdp.is_empty());

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
            transport: None,
            context: Default::default(),
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
            candidate: String::new(),
            request_id: String::new(),
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

#[tokio::test]
async fn exact_protocol_socket_scopes_bye_and_reclaims_all_routes_on_disconnect() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        serve_listener(listener, server_adapter)
            .await
            .expect("serve")
    });

    let offerer = WebRtcAdapter::new(config);
    let handle = offerer
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: offerer.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("originate");
    let offer = offerer
        .local_sdp(&handle.connection.id)
        .expect("local offer");

    let mut request = format!("ws://{address}")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static("rvoip.webrtc.v1"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect");
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok()),
        Some("rvoip.webrtc.v1")
    );

    async fn open_route(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        offer: &str,
        request_id: &str,
    ) -> ConnectionId {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&SignalingMessage {
                    msg_type: "offer".into(),
                    sdp: offer.into(),
                    connection_id: String::new(),
                    candidate: String::new(),
                    request_id: request_id.into(),
                })
                .expect("offer JSON")
                .into(),
            ))
            .await
            .expect("send offer");
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("answer timeout")
            .expect("answer frame")
            .expect("answer read");
        let answer: SignalingMessage =
            serde_json::from_str(frame.to_text().expect("text answer")).expect("answer JSON");
        assert_eq!(answer.msg_type, "answer");
        assert_eq!(answer.request_id, request_id);
        ConnectionId::from_string(answer.connection_id)
    }

    let first = open_route(&mut socket, &offer, "request-1").await;
    let second = open_route(&mut socket, &offer, "request-2").await;
    assert!(adapter.routes().contains_key(&first));
    assert!(adapter.routes().contains_key(&second));

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&SignalingMessage {
                msg_type: "bye".into(),
                connection_id: first.to_string(),
                ..Default::default()
            })
            .expect("bye JSON")
            .into(),
        ))
        .await
        .expect("send BYE");
    wait_route_absent(&adapter, &first).await;
    assert!(adapter.routes().contains_key(&second));

    let third = open_route(&mut socket, &offer, "request-3").await;
    assert!(adapter.routes().contains_key(&third));
    socket.close(None).await.expect("close socket");
    wait_route_absent(&adapter, &second).await;
    wait_route_absent(&adapter, &third).await;
    server.abort();
}
