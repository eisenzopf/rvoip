//! G-tail closeout — NACK round-trip via a lossy TURN relay.
//!
//! Two `RvoipPeerConnection` instances both configured with
//! `IceTransportPolicy::Relay` against the same coturn instance, with a
//! lossy UDP proxy in front of coturn's control port. The proxy drops
//! ~5 % of UDP datagrams in each direction, which translates to ~5 % loss
//! of the Send-Indication / Data-Indication-wrapped media payloads.
//! Under sustained Opus traffic the inbound side observes packet loss
//! and the registered RTCP-NACK feedback (see `peer/builder.rs`) round-
//! trips: the sender records non-zero NACK + retransmit counts.
//!
//! Uses a hermetic in-process TURN server, so relay setup failures are test
//! failures rather than skips.

mod support {
    pub mod coturn_fixture;
    pub mod lossy_turn_fixture;
}

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::config::IceTransportPolicy;
use rvoip_webrtc::media::{from_tracks, silent_opus_payload};
use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use support::lossy_turn_fixture::LossyTurnFixture;
use tokio::sync::Notify;

#[tokio::test]
#[cfg_attr(
    not(feature = "turn-fork-candidate"),
    ignore = "requires the owner-reviewed UDP TURN/NACK alpha-fork candidate"
)]
async fn nack_round_trip_through_lossy_turn() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let fixture = LossyTurnFixture::start(0.05, 0xCAFE)
        .await
        .expect("start lossy TURN fixture");

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![fixture.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    // Lossy relay handshakes take longer; budget generously.
    config.gather_timeout_secs = 20;

    let (offerer, answerer) =
        tokio::time::timeout(Duration::from_secs(60), connect_loopback(&config))
            .await
            .expect("lossy relay handshake timed out")
            .expect("lossy relay handshake failed");

    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 2,
        fmtp: None,
    };

    let offerer_ssrc = offerer.local_audio_ssrc().expect("offerer ssrc");
    let offerer_local = offerer.local_audio_track().expect("offerer local track");
    let offerer_stream = from_tracks(
        StreamId::new(),
        codec.clone(),
        offerer_local,
        offerer_ssrc,
        /* Opus PT */ 111,
        None,
    );
    // Enable outbound stats on the offerer so we can observe NACK counts.
    offerer_stream.enable_webrtc_stats(
        Arc::clone(offerer.peer_connection()),
        Arc::new(Notify::new()),
    );

    let answerer_ssrc = answerer.local_audio_ssrc().expect("answerer ssrc");
    let answerer_local = answerer.local_audio_track().expect("answerer local track");
    let answerer_stream = from_tracks(
        StreamId::new(),
        codec,
        answerer_local,
        answerer_ssrc,
        111,
        None,
    );
    answerer_stream.enable_webrtc_stats(
        Arc::clone(answerer.peer_connection()),
        Arc::new(Notify::new()),
    );

    let remote =
        RvoipPeerConnection::prime_remote_track(&offerer, &answerer, Duration::from_secs(15))
            .await
            .expect("answerer receives offerer track via lossy relay");
    answerer_stream.attach_remote(remote);

    let mut inbound = answerer_stream.frames_in();
    let loss_before_media = fixture.snapshot();
    fixture.enable_loss();

    // Pump 200 frames over ~4 s. At 5 % loss → ~10 lost on each direction.
    let pump_offerer = offerer_stream.clone();
    let pump = tokio::spawn(async move {
        for seq in 1..=200u16 {
            let payload = silent_opus_payload();
            if pump_offerer
                .frames_out()
                .send(rvoip_core::stream::MediaFrame {
                    stream_id: pump_offerer.id(),
                    kind: rvoip_core::stream::StreamKind::Audio,
                    payload,
                    timestamp_rtp: seq as u32 * 960,
                    captured_at: chrono::Utc::now(),
                    payload_type: None,
                })
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    // Drain inbound concurrently so SCTP/RTP back-pressure doesn't stall.
    let drain = tokio::spawn(async move {
        let mut received_timestamps = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(100), inbound.recv()).await {
                Ok(Some(frame)) => received_timestamps.push(frame.timestamp_rtp),
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        received_timestamps
    });

    let _ = pump.await;
    let received_timestamps = drain.await.expect("inbound drain task");
    let timestamp_gaps = received_timestamps
        .windows(2)
        .filter(|pair| pair[1].wrapping_sub(pair[0]) > 960)
        .count();

    // Let stats poller observe at least one cycle after the pump completes.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let inbound_stats = answerer_stream.webrtc_stats_snapshot();
    let outbound_stats = offerer_stream.webrtc_stats_snapshot();
    let loss_after_media = fixture.snapshot();
    let media_phase_drops = loss_after_media
        .client_packets_dropped
        .saturating_sub(loss_before_media.client_packets_dropped)
        + loss_after_media
            .server_packets_dropped
            .saturating_sub(loss_before_media.server_packets_dropped);

    eprintln!(
        "lossy_turn_nack: proxy={loss_after_media:?}, media_phase_drops={media_phase_drops}, received_frames={}, timestamp_gaps={timestamp_gaps}, inbound packets_lost={} jitter_ms={}, outbound nack_count={} retransmits={}",
        received_timestamps.len(),
        inbound_stats.packets_lost,
        inbound_stats.jitter_ms,
        outbound_stats.outbound.nack_count,
        outbound_stats.outbound.retransmitted_packets,
    );

    assert!(
        media_phase_drops > 0,
        "the deterministic TURN proxy did not drop any media-phase datagrams"
    );

    // W3C cumulative packetsLost may return to zero when every initially
    // missing RTP packet arrives through retransmission. Prove recovery from
    // the application-facing stream and prove the feedback/retransmission
    // path directly below instead of treating the final loss gauge as a
    // monotonic counter.
    assert_eq!(
        received_timestamps.len(),
        200,
        "NACK retransmission should recover every deterministically dropped media frame"
    );
    assert!(
        outbound_stats.outbound.nack_count > 0,
        "expected outbound NACK count > 0 — RTCP feedback should have round-tripped"
    );
    assert!(
        outbound_stats.outbound.retransmitted_packets > 0,
        "expected retransmitted packets > 0 after receiving NACK feedback"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
    fixture.close().await.expect("close TURN fixture");
}
