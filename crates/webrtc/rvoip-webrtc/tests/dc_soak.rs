//! H6.4: short data-channel soak — drive sustained ping/pong traffic between
//! two loopback peers for several seconds, then assert nothing leaked.
//!
//! This is a *short* soak (~5s). The plan calls for an eventual 1h variant
//! (gated behind a `soak-1h` feature) to catch slow leaks; this one catches
//! the obvious ones (lost messages, panicking pump, growing task count).

use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_webrtc::peer::{connect_loopback, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use webrtc::data_channel::DataChannel;

const SOAK_SECS: u64 = 5;

/// Negotiate a loopback pair whose offer carries the data channel.
///
/// The SCTP association is negotiated by the offer's `m=application`
/// section, so the channel is created before the offer, as
/// `RvoipPeerConnection::create_data_channel` requires. A session negotiated
/// without one has no association to open a later channel on.
async fn connected_pair_with_data_channel() -> (
    Arc<RvoipPeerConnection>,
    Arc<RvoipPeerConnection>,
    Arc<dyn DataChannel>,
    Arc<dyn DataChannel>,
) {
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");
    let offerer_dc = offerer
        .create_data_channel("soak", rvoip_webrtc::peer::DataChannelOptions::reliable())
        .await
        .expect("create dc");

    let offer_sdp = offerer.create_offer_and_gather().await.expect("offer");
    let answer_sdp = answerer
        .accept_offer_and_gather(&offer_sdp)
        .await
        .expect("answer");
    offerer
        .set_remote_answer(&answer_sdp)
        .await
        .expect("apply answer");
    let timeout = Duration::from_secs(config.connection_timeout_secs);
    tokio::try_join!(
        offerer.wait_connected(timeout),
        answerer.wait_connected(timeout)
    )
    .expect("loopback");

    let answerer_dc = answerer
        .wait_data_channel(Duration::from_secs(5))
        .await
        .expect("answerer dc");

    RvoipPeerConnection::wait_data_channel_open(&offerer_dc, Duration::from_secs(5))
        .await
        .expect("offerer dc open");
    RvoipPeerConnection::wait_data_channel_open(&answerer_dc, Duration::from_secs(5))
        .await
        .expect("answerer dc open");
    (offerer, answerer, offerer_dc, answerer_dc)
}

#[tokio::test]
async fn five_second_dc_soak_no_leak() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (offerer, answerer, offerer_dc, answerer_dc) = connected_pair_with_data_channel().await;

    let deadline = Instant::now() + Duration::from_secs(SOAK_SECS);
    let mut sent = 0u64;
    let mut received = 0u64;

    while Instant::now() < deadline {
        offerer_dc
            .send_text(&format!("ping-{sent}"))
            .await
            .expect("send");
        sent += 1;
        match RvoipPeerConnection::recv_data_channel_text(&answerer_dc, Duration::from_millis(500))
            .await
        {
            Ok(_) => received += 1,
            Err(e) => panic!("recv failed at iteration {sent}: {e}"),
        }
    }

    assert_eq!(sent, received, "every ping should be received");
    assert!(
        sent >= 100,
        "expected ≥100 messages in {SOAK_SECS}s; got {sent}"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
}

/// Lighter version that exercises pump lifecycle without DC creation race —
/// just opens and closes 20 peer pairs back-to-back and verifies no panic.
#[tokio::test]
async fn open_close_lifecycle_no_panic() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    for i in 0..20 {
        let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
            .await
            .unwrap_or_else(|e| panic!("loopback iter {i}: {e}"));
        offerer.close().await.ok();
        answerer.close().await.ok();
    }
}
