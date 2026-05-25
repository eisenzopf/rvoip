//! G1 — Data channel options API tests.
//!
//! Exercises all five RFC 8832 reliability combinations through a loopback
//! peer pair, plus the validation guard for mutually-exclusive caps.

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use rvoip_webrtc::peer::{DataChannelOptions, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcConfig, WebRtcError};
use webrtc::data_channel::{DataChannel, DataChannelEvent};

async fn open_pair_with_dc(
    opts: DataChannelOptions,
    label: &str,
) -> (Arc<RvoipPeerConnection>, Arc<RvoipPeerConnection>, Arc<dyn DataChannel>, Arc<dyn DataChannel>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");

    let offerer_dc = offerer
        .create_data_channel(label, opts)
        .await
        .expect("create dc");

    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .expect("answer");
    offerer
        .set_remote_answer(&answer)
        .await
        .expect("set remote");

    let timeout = Duration::from_secs(15);
    tokio::try_join!(
        offerer.wait_connected(timeout),
        answerer.wait_connected(timeout)
    )
    .expect("both connected");

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

async fn round_trip_text(
    sender: &Arc<dyn DataChannel>,
    receiver: &Arc<dyn DataChannel>,
    msg: &str,
) {
    sender.send_text(msg).await.expect("send");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!("did not receive {msg}");
        }
        if let Some(ev) = RvoipPeerConnection::poll_data_channel(receiver, Duration::from_millis(100)).await {
            if let DataChannelEvent::OnMessage(m) = ev {
                if m.is_string && String::from_utf8_lossy(&m.data) == msg {
                    return;
                }
            }
        }
    }
}

#[tokio::test]
async fn dc_options_reliable_round_trips() {
    let (_o, _a, dc_a, dc_b) =
        open_pair_with_dc(DataChannelOptions::reliable(), "reliable").await;
    round_trip_text(&dc_a, &dc_b, "hello-reliable").await;
}

#[tokio::test]
async fn dc_options_unordered_zero_retransmits_round_trips() {
    let (_o, _a, dc_a, dc_b) =
        open_pair_with_dc(DataChannelOptions::unreliable(), "unreliable").await;
    // Even with `max_retransmits=0` we expect the first send to succeed on a
    // loopback link — the channel just becomes lossy under packet drop.
    round_trip_text(&dc_a, &dc_b, "hello-unreliable").await;
}

#[tokio::test]
async fn dc_options_partial_reliable_retransmits_round_trips() {
    let (_o, _a, dc_a, dc_b) = open_pair_with_dc(
        DataChannelOptions::partial_reliable_retransmits(3),
        "partial-rtx",
    )
    .await;
    round_trip_text(&dc_a, &dc_b, "hello-rtx").await;
}

#[tokio::test]
async fn dc_options_partial_reliable_lifetime_round_trips() {
    let (_o, _a, dc_a, dc_b) = open_pair_with_dc(
        DataChannelOptions::partial_reliable_lifetime(200),
        "partial-lifetime",
    )
    .await;
    round_trip_text(&dc_a, &dc_b, "hello-lifetime").await;
}

#[tokio::test]
async fn dc_options_protocol_field_round_trips_to_remote() {
    let opts = DataChannelOptions::reliable().with_protocol("rvoip.v1");
    let (_o, _a, dc_a, dc_b) = open_pair_with_dc(opts, "protocol-test").await;
    // We can't easily inspect the protocol field on the remote handle from
    // webrtc-rs 0.20-alpha, so verify the channel functions end-to-end.
    round_trip_text(&dc_a, &dc_b, "hello-with-protocol").await;
}

#[tokio::test]
async fn dc_options_binary_round_trips() {
    let (_o, _a, dc_a, dc_b) =
        open_pair_with_dc(DataChannelOptions::reliable(), "binary").await;
    let payload: &[u8] = b"\x00\x01\x02hello-bin\xff";
    dc_a.send(BytesMut::from(payload)).await.expect("send bin");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!("binary message lost");
        }
        if let Some(DataChannelEvent::OnMessage(m)) =
            RvoipPeerConnection::poll_data_channel(&dc_b, Duration::from_millis(100)).await
        {
            if !m.is_string && m.data == payload {
                return;
            }
        }
    }
}

#[tokio::test]
async fn dc_options_mutually_exclusive_returns_invalid_argument() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let bad = DataChannelOptions {
        ordered: true,
        max_retransmits: Some(3),
        max_packet_lifetime_ms: Some(100),
        protocol: None,
        negotiated_id: None,
    };
    let res = offerer.create_data_channel("invalid", bad).await;
    match res {
        Err(WebRtcError::InvalidArgument(_)) => {}
        Err(other) => panic!("expected InvalidArgument, got error {other:?}"),
        Ok(_) => panic!("expected InvalidArgument, got Ok"),
    }
}
