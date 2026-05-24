//! Documents intentional capability gaps (trickle ICE, simulcast, hosted TURN).

#![cfg(feature = "comprehensive")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rvoip_webrtc::client::{IceCandidate, Signaler, WsSignaler};
use rvoip_webrtc::peer::{connect_loopback, WebRtcFeatureSupport, RvoipPeerConnection};
use rvoip_webrtc::signaling::websocket::{serve_listener, SignalingMessage};
use rvoip_webrtc::sdp::{sdp_has_inline_ice_candidates, sdp_indicates_simulcast};
use rvoip_webrtc::{IceServerConfig, WebRtcAdapter, WebRtcConfig};

#[test]
fn feature_support_defaults_document_gaps() {
    let features = WebRtcFeatureSupport::default();
    assert!(!features.trickle_ice_signaling, "trickle ICE signaling is deferred");
    assert!(!features.simulcast, "simulcast is deferred");
    assert!(!features.turn_relay_server, "hosted TURN is out of scope");
}

#[test]
fn turn_config_accepts_external_credentials() {
    let config = WebRtcConfig::loopback().with_turn("turn:example.com:3478", "alice", "secret");
    assert_eq!(config.ice_servers.len(), 1);
    let turn = &config.ice_servers[0];
    assert_eq!(turn.urls, vec!["turn:example.com:3478"]);
    assert_eq!(turn.username.as_deref(), Some("alice"));
    assert_eq!(turn.credential.as_deref(), Some("secret"));
}

#[test]
fn stun_config_has_no_credentials() {
    let stun = IceServerConfig::stun("stun:stun.example.org");
    assert!(stun.username.is_none());
    assert!(stun.credential.is_none());
}

#[tokio::test]
async fn loopback_sdp_uses_full_gather_not_simulcast() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();

    let offerer = RvoipPeerConnection::new(&config, rvoip_webrtc::peer::PeerRole::Offerer)
        .await
        .expect("offerer");
    offerer.add_local_audio_track().await.expect("audio");
    offerer.add_local_video_track().await.expect("video");

    let offer_sdp = offerer.create_offer_and_gather().await.expect("offer");
    assert!(sdp_has_inline_ice_candidates(&offer_sdp), "v1 embeds ICE in SDP");
    assert!(!sdp_indicates_simulcast(&offer_sdp), "simulcast not negotiated");

    assert!(
        !offerer.gathered_ice_candidates().is_empty(),
        "local ICE candidates should be logged during gather"
    );

    offerer.close().await.ok();
}

#[tokio::test]
async fn ws_signaler_send_ice_is_noop_gap() {
    let signaler = WsSignaler::new("ws://127.0.0.1:1");
    let result = signaler
        .send_ice(&IceCandidate(r#"{"candidate":"..."}"#.into()))
        .await;
    assert!(result.is_ok(), "v1 uses full SDP gather; send_ice is intentionally inert");
}

#[tokio::test]
async fn ws_ice_candidate_message_not_implemented() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let server = tokio::spawn(async move {
        serve_listener(listener, adapter).await.ok();
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("connect");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "ice-candidate".into(),
            sdp: String::new(),
            connection_id: "conn-1".into(),
            candidate: r#"{"candidate":"candidate:1 1 UDP 2130706431 127.0.0.1 9 typ host"}"#
                .into(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send ice-candidate");

    let next = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timeout");
    match next {
        None | Some(Err(_)) => {}
        Some(Ok(msg)) => assert!(msg.is_close(), "server should close after unsupported ice-candidate"),
    }

    server.abort();
}

#[tokio::test]
async fn connect_loopback_records_ice_candidates_on_both_peers() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
        .await
        .expect("loopback");

    assert!(!offerer.gathered_ice_candidates().is_empty());
    assert!(!answerer.gathered_ice_candidates().is_empty());

    offerer.close().await.ok();
    answerer.close().await.ok();
}
