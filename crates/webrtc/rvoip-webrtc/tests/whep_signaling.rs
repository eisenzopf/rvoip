//! WHEP POST + PATCH round-trip (feature `signaling-whip`).

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::ids::ConnectionId;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::whip;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

#[tokio::test]
async fn whep_post_patch_reaches_connected() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let whip_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        whip::serve_listener(listener, whip_adapter)
            .await
            .expect("whip serve")
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let base = format!("http://{addr}");

    let player = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("player peer");
    player
        .prepare_receive_only_offer()
        .await
        .expect("recvonly player");
    let player_offer = player
        .create_offer_and_gather()
        .await
        .expect("player offer");
    let answer_resp = client
        .post(format!("{base}/whep/subscriber"))
        .header("content-type", "application/sdp")
        .body(player_offer)
        .send()
        .await
        .expect("whep post");
    assert_eq!(answer_resp.status(), reqwest::StatusCode::CREATED);
    let location = answer_resp
        .headers()
        .get("location")
        .expect("location")
        .to_str()
        .expect("location ascii")
        .to_owned();
    let etag = answer_resp
        .headers()
        .get("etag")
        .expect("etag")
        .to_str()
        .expect("etag ascii")
        .to_owned();
    let answer_sdp = answer_resp.text().await.expect("answer body");
    assert!(answer_sdp.contains("m=audio"));
    assert!(answer_sdp.contains("a=sendonly"));
    player
        .set_remote_answer(&answer_sdp)
        .await
        .expect("apply WHEP answer");

    let connection_id =
        ConnectionId::from_string(location.rsplit('/').next().expect("WHEP connection id"));
    let (server_connected, player_connected) = tokio::join!(
        adapter.accept(connection_id.clone()),
        player.wait_connected(Duration::from_secs(10)),
    );
    server_connected.expect("server connected");
    player_connected.expect("player connected");

    let forbidden_renegotiation = client
        .patch(format!("{base}{location}"))
        .header("content-type", "application/sdp")
        .header("if-match", &etag)
        .body(answer_sdp)
        .send()
        .await
        .expect("WHEP renegotiation PATCH");
    assert_eq!(
        forbidden_renegotiation.status(),
        reqwest::StatusCode::CONFLICT
    );

    let delete = client
        .delete(format!("{base}{location}"))
        .header("if-match", &etag)
        .send()
        .await
        .expect("WHEP DELETE");
    assert_eq!(delete.status(), reqwest::StatusCode::OK);
    assert!(!adapter.is_connection_live(&connection_id));
    assert_eq!(adapter.metrics().active_http_resources, 0);

    player.close().await.expect("close player");
    server.abort();
}
