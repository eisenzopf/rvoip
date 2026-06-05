//! Loopback data-channel ping/pong without signaling server.

#![cfg(feature = "comprehensive")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::client::comprehensive::prepare_offer_media;
use rvoip_webrtc::client::SessionMedium;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;

#[tokio::test]
async fn loopback_data_channel_ping_pong() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");

    let client_dc = prepare_offer_media(&offerer, SessionMedium::Audio)
        .await
        .expect("prepare");
    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .expect("answer");
    offerer
        .set_remote_answer(&answer)
        .await
        .expect("set answer");

    offerer
        .wait_connected(Duration::from_secs(10))
        .await
        .expect("offerer connected");
    answerer
        .wait_connected(Duration::from_secs(10))
        .await
        .expect("answerer connected");

    RvoipPeerConnection::wait_data_channel_open(&client_dc, Duration::from_secs(10))
        .await
        .expect("client dc open");
    let server_dc = answerer
        .wait_data_channel(Duration::from_secs(10))
        .await
        .expect("server dc");
    RvoipPeerConnection::wait_data_channel_open(&server_dc, Duration::from_secs(10))
        .await
        .expect("server dc open");

    let echo_dc = Arc::clone(&server_dc);
    tokio::spawn(async move {
        loop {
            let Some(event) =
                RvoipPeerConnection::poll_data_channel(&echo_dc, Duration::from_millis(200)).await
            else {
                continue;
            };
            if let webrtc::data_channel::DataChannelEvent::OnMessage(msg) = event {
                if msg.is_string && msg.data.as_ref() == b"ping" {
                    let _ = echo_dc.send_text("pong").await;
                }
            }
        }
    });

    client_dc.send_text("ping").await.expect("send ping");
    let reply = RvoipPeerConnection::recv_data_channel_text(&client_dc, Duration::from_secs(10))
        .await
        .expect("pong");
    assert_eq!(reply, "pong");
}
