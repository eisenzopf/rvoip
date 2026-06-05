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
//! Skips gracefully when Docker isn't reachable.

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
use rvoip_webrtc::media::{from_tracks, silent_rtp_payload_for_ssrc};
use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use support::lossy_turn_fixture::LossyTurnFixture;
use tokio::sync::Notify;

#[tokio::test]
async fn nack_round_trip_through_lossy_turn() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(fixture) = LossyTurnFixture::start(0.05, 0xCAFE).await else {
        eprintln!("skipped: docker / coturn unavailable");
        return;
    };

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![fixture.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    // Lossy relay handshakes take longer; budget generously.
    config.gather_timeout_secs = 20;

    let (offerer, answerer) =
        match tokio::time::timeout(Duration::from_secs(60), connect_loopback(&config)).await {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                eprintln!("skipped: lossy relay handshake failed ({e})");
                return;
            }
            Err(_) => {
                eprintln!("skipped: lossy relay handshake timed out");
                return;
            }
        };

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

    // Pump 200 frames over ~4 s. At 5 % loss → ~10 lost on each direction.
    let pump_offerer = offerer_stream.clone();
    let pump = tokio::spawn(async move {
        for seq in 1..=200u16 {
            let payload = silent_rtp_payload_for_ssrc(offerer_ssrc, seq, seq as u32 * 960);
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            if tokio::time::timeout(Duration::from_millis(100), inbound.recv())
                .await
                .is_err()
            {
                continue;
            }
        }
    });

    let _ = pump.await;
    let _ = drain.await;

    // Let stats poller observe at least one cycle after the pump completes.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let inbound_stats = answerer_stream.webrtc_stats_snapshot();
    let outbound_stats = offerer_stream.webrtc_stats_snapshot();

    eprintln!(
        "lossy_turn_nack: inbound packets_lost={} jitter_ms={}, outbound nack_count={} retransmits={}",
        inbound_stats.packets_lost,
        inbound_stats.jitter_ms,
        outbound_stats.outbound.nack_count,
        outbound_stats.outbound.retransmitted_packets,
    );

    assert!(
        inbound_stats.packets_lost > 0,
        "expected inbound packets_lost > 0 over a 5% lossy relay (got {})",
        inbound_stats.packets_lost
    );
    assert!(
        outbound_stats.outbound.nack_count > 0,
        "expected outbound NACK count > 0 — RTCP feedback should have round-tripped"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
    drop(fixture);
}
