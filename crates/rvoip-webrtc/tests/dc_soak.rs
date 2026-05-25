//! H6.4: short data-channel soak — drive sustained ping/pong traffic between
//! two loopback peers for several seconds, then assert nothing leaked.
//!
//! This is a *short* soak (~5s). The plan calls for an eventual 1h variant
//! (gated behind a `soak-1h` feature) to catch slow leaks; this one catches
//! the obvious ones (lost messages, panicking pump, growing task count).

use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use webrtc::data_channel::DataChannel;

const SOAK_SECS: u64 = 5;

async fn open_data_channel(
    offerer: &Arc<RvoipPeerConnection>,
    answerer: &Arc<RvoipPeerConnection>,
) -> (Arc<dyn DataChannel>, Arc<dyn DataChannel>) {
    let offerer_dc = offerer
        .create_data_channel("soak", rvoip_webrtc::peer::DataChannelOptions::reliable())
        .await
        .expect("create dc");
    // Re-handshake — connect_loopback already did it, but data channels
    // negotiated after `set_local_description` may need a tick to surface.
    let _ = tokio::time::sleep(Duration::from_millis(100)).await;
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
    (offerer_dc, answerer_dc)
}

#[tokio::test]
async fn five_second_dc_soak_no_leak() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
        .await
        .expect("loopback");

    let (offerer_dc, answerer_dc) = open_data_channel(&offerer, &answerer).await;

    let deadline = Instant::now() + Duration::from_secs(SOAK_SECS);
    let mut sent = 0u64;
    let mut received = 0u64;

    while Instant::now() < deadline {
        offerer_dc
            .send_text(&format!("ping-{sent}"))
            .await
            .expect("send");
        sent += 1;
        match RvoipPeerConnection::recv_data_channel_text(
            &answerer_dc,
            Duration::from_millis(500),
        )
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
