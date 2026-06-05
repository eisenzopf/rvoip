//! WebRtcMediaBridge substrate_setup exchange + connect (Phase 7).

use std::time::Duration;

use rvoip_websocket::{BridgeRole, WebRtcMediaBridge};

#[tokio::test]
async fn media_bridge_substrate_setup_loopback() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let offerer = WebRtcMediaBridge::new_offerer()
        .await
        .expect("offerer bridge");
    let answerer = WebRtcMediaBridge::new_answerer()
        .await
        .expect("answerer bridge");

    assert_eq!(offerer.role(), BridgeRole::Offerer);
    assert_eq!(answerer.role(), BridgeRole::Answerer);

    let offer_setup = offerer
        .local_substrate_setup()
        .await
        .expect("offerer local SDP");
    assert_eq!(offer_setup.kind, "websocket+webrtc");
    assert!(offer_setup.sdp.contains("m=audio"));

    answerer
        .set_remote_substrate_setup(offer_setup)
        .await
        .expect("answerer applies offer");

    let answer_setup = answerer
        .local_substrate_setup()
        .await
        .expect("answerer local SDP");
    assert_eq!(answer_setup.kind, "websocket+webrtc");
    assert!(answer_setup.sdp.contains("m=audio"));

    offerer
        .set_remote_substrate_setup(answer_setup)
        .await
        .expect("offerer applies answer");

    let timeout = Duration::from_secs(10);
    offerer
        .wait_connected(timeout)
        .await
        .expect("offerer connected");
    answerer
        .wait_connected(timeout)
        .await
        .expect("answerer connected");

    assert!(
        offerer.media_stream().is_some(),
        "offerer should expose a WebRtcMediaStream after setup"
    );
    assert!(
        answerer.media_stream().is_some(),
        "answerer should expose a WebRtcMediaStream after setup"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
}
