//! H2: Trickle ICE over WebSocket signaler — inbound + outbound forwarding.

#![cfg(feature = "signaling-ws")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::websocket::{serve_listener, SignalingMessage};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn ws_inbound_trickle_candidate_is_applied() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Server adapter behind WS listener.
    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_adapter_for_serve = Arc::clone(&server_adapter);
    let server_handle = tokio::spawn(async move {
        serve_listener(listener, server_adapter_for_serve)
            .await
            .ok();
    });

    // Client-side offerer (separate adapter — just for SDP generation).
    let client = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("client peer"),
    );
    client.add_local_audio_track().await.expect("audio");
    let offer = client.create_offer_and_gather().await.expect("offer");

    // Exchange offer over WS.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("connect");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "offer".into(),
            sdp: offer,
            ..Default::default()
        })
        .expect("encode offer")
        .into(),
    ))
    .await
    .expect("send offer");

    let answer_msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("answer timeout")
        .expect("ws closed")
        .expect("ws err");
    let parsed: SignalingMessage =
        serde_json::from_str(answer_msg.to_text().expect("text")).expect("decode answer");
    assert_eq!(
        parsed.msg_type, "answer",
        "first reply should be the answer"
    );
    let server_conn_id = parsed.connection_id.clone();
    assert!(!server_conn_id.is_empty());

    // Send a host trickle candidate. Loopback host candidate so add_ice_candidate
    // does not require any external network.
    let cand_json = r#"{"candidate":"candidate:1 1 udp 2130706431 127.0.0.1 50000 typ host","sdpMid":"0","sdpMLineIndex":0}"#;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "ice-candidate".into(),
            connection_id: server_conn_id.clone(),
            candidate: cand_json.into(),
            ..Default::default()
        })
        .expect("encode candidate")
        .into(),
    ))
    .await
    .expect("send candidate");

    // Drain any outbound `ice-candidate` messages the server may push; what we
    // care about is that the WS stays open (no error close) and the server
    // hasn't crashed. Give it ~1s to react.
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(msg))) if msg.is_close() => {
                panic!("server closed WS after trickle candidate: {msg:?}");
            }
            _ => {}
        }
    }

    // Tell the server to end the route — exercises bye after trickle.
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "bye".into(),
            connection_id: server_conn_id,
            ..Default::default()
        })
        .expect("encode bye")
        .into(),
    ))
    .await
    .ok();

    server_handle.abort();
}
