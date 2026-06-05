//! G1 — Data channel low-threshold + typed wrapper smoke test.
//!
//! Verifies the `RvoipDataChannel` wrapper exposes a working
//! `set_buffered_amount_low_threshold` / `buffered_amount_low_threshold`
//! pair and that send_text/send_binary surface the inner channel cleanly.

use std::time::Duration;

use rvoip_webrtc::peer::{DataChannelOptions, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use webrtc::data_channel::DataChannelEvent;

#[tokio::test]
async fn typed_wrapper_round_trips_text_and_binary() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");

    let dc_a = offerer
        .create_data_channel_typed("typed", DataChannelOptions::reliable())
        .await
        .expect("typed dc");

    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .expect("answer");
    offerer
        .set_remote_answer(&answer)
        .await
        .expect("set remote");

    tokio::try_join!(
        offerer.wait_connected(Duration::from_secs(15)),
        answerer.wait_connected(Duration::from_secs(15))
    )
    .expect("connected");

    let dc_b = answerer
        .wait_data_channel(Duration::from_secs(5))
        .await
        .expect("answerer dc");

    RvoipPeerConnection::wait_data_channel_open(dc_a.inner(), Duration::from_secs(5))
        .await
        .expect("offerer dc open");
    RvoipPeerConnection::wait_data_channel_open(&dc_b, Duration::from_secs(5))
        .await
        .expect("answerer dc open");

    // Configure low-threshold + read it back.
    dc_a.set_buffered_amount_low_threshold(1024)
        .await
        .expect("set threshold");
    let read_back = dc_a
        .buffered_amount_low_threshold()
        .await
        .expect("get threshold");
    assert_eq!(read_back, 1024, "low threshold round-trip");

    // send_text via the typed wrapper.
    dc_a.send_text("typed-hello").await.expect("send text");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut got_text = false;
    while tokio::time::Instant::now() < deadline && !got_text {
        if let Some(DataChannelEvent::OnMessage(m)) =
            RvoipPeerConnection::poll_data_channel(&dc_b, Duration::from_millis(100)).await
        {
            if m.is_string && String::from_utf8_lossy(&m.data) == "typed-hello" {
                got_text = true;
            }
        }
    }
    assert!(got_text, "expected typed-hello");

    // send_binary via the typed wrapper.
    let bin = b"\xde\xad\xbe\xef".to_vec();
    dc_a.send_binary(&bin).await.expect("send bin");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut got_bin = false;
    while tokio::time::Instant::now() < deadline && !got_bin {
        if let Some(DataChannelEvent::OnMessage(m)) =
            RvoipPeerConnection::poll_data_channel(&dc_b, Duration::from_millis(100)).await
        {
            if !m.is_string && m.data == bin {
                got_bin = true;
            }
        }
    }
    assert!(got_bin, "expected binary payload");

    assert_eq!(dc_a.label(), "typed");
}

/// G-tail closeout: the broadcast pump on `RvoipDataChannel` must surface
/// `DataChannelEvent::OnBufferedAmountLow` to `subscribe_buffered_amount_low()`
/// receivers when the underlying buffer drains below the configured low
/// threshold.
#[tokio::test]
async fn buffered_amount_low_event_fires_after_drain() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");

    let dc_a = offerer
        .create_data_channel_typed("backpressure", DataChannelOptions::reliable())
        .await
        .expect("typed dc");

    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .expect("answer");
    offerer
        .set_remote_answer(&answer)
        .await
        .expect("set remote");

    tokio::try_join!(
        offerer.wait_connected(Duration::from_secs(15)),
        answerer.wait_connected(Duration::from_secs(15))
    )
    .expect("connected");

    let dc_b = answerer
        .wait_data_channel(Duration::from_secs(5))
        .await
        .expect("answerer dc");

    // Open both sides BEFORE arming the pump, so `wait_data_channel_open`
    // can poll the raw inner channel without racing the pump.
    RvoipPeerConnection::wait_data_channel_open(dc_a.inner(), Duration::from_secs(5))
        .await
        .expect("offerer dc open");
    RvoipPeerConnection::wait_data_channel_open(&dc_b, Duration::from_secs(5))
        .await
        .expect("answerer dc open");

    // Arm the low-threshold event. Threshold = 1 byte; any non-empty send
    // briefly puts the buffer above the threshold and the event must fire
    // when the SCTP layer flushes it.
    dc_a.set_buffered_amount_low_threshold(1)
        .await
        .expect("set low threshold");
    let mut low_rx = dc_a.subscribe_buffered_amount_low();

    // Push enough data to guarantee the buffered amount transitions from
    // above to ≤ threshold at least once. A handful of small messages on
    // a fresh channel is plenty.
    for i in 0..8 {
        dc_a.send_text(&format!("drain-{i}"))
            .await
            .expect("send_text");
    }

    // Drain the answerer side in the background so SCTP keeps flowing.
    let dc_b_drain = dc_b.clone();
    let drain = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        while tokio::time::Instant::now() < deadline {
            let _ = RvoipPeerConnection::poll_data_channel(&dc_b_drain, Duration::from_millis(50))
                .await;
        }
    });

    let event_arrived = tokio::time::timeout(Duration::from_secs(5), low_rx.recv())
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);

    let _ = drain.await;

    assert!(
        event_arrived,
        "OnBufferedAmountLow event should have fired on the broadcast subscription"
    );
}
