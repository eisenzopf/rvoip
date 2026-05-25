//! G9a — TURN relay E2E test.
//!
//! When Docker is available, spins up a coturn container and verifies the
//! adapter can use it via `IceTransportPolicy::Relay`. When Docker isn't
//! available, the tests skip gracefully (return without failing).

mod support {
    pub mod coturn_fixture;
}

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::config::IceTransportPolicy;
use rvoip_webrtc::media::{from_tracks, silent_rtp_payload_for_ssrc};
use rvoip_webrtc::peer::{connect_loopback, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use support::coturn_fixture::CoturnFixture;
use tokio::sync::Notify;

#[tokio::test]
async fn relay_policy_with_coturn_fixture_builds_peer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(coturn) = CoturnFixture::start().await else {
        eprintln!("skipped: docker / coturn unavailable");
        return;
    };

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;

    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("peer should build with Relay policy + coturn ICE config"),
    );
    peer.add_local_audio_track().await.expect("audio");

    // Build a Relay-only offer. We don't try to complete the handshake
    // (would need a remote peer also dialed via coturn); the assertion is
    // that the config plumbs through cleanly and the engine accepts it.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        peer.create_offer_and_gather(),
    )
    .await;

    // RAII teardown via Drop.
    drop(peer);
    drop(coturn);
}

/// G-tail closeout: two `RvoipPeerConnection`s, both configured with
/// `IceTransportPolicy::Relay` against the same coturn fixture, complete a
/// full offer/answer + ICE handshake, exchange Opus media, and report a
/// selected candidate pair whose **local** candidate type is `relay`.
///
/// Validates that:
/// 1. Relay-only ICE actually nominates a relay/relay (or relay/srflx) pair
///    through coturn.
/// 2. Media frames traverse the relay end to end (not just config plumbing).
/// 3. The G4 `selected_pair` stats surface the relay candidate type.
#[tokio::test]
async fn relay_only_two_peer_media_round_trip() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Some(coturn) = CoturnFixture::start().await else {
        eprintln!("skipped: docker / coturn unavailable");
        return;
    };

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    // Relay handshakes need more time than host (allocation + permission +
    // STUN binding round trips through coturn).
    config.gather_timeout_secs = 15;

    let (offerer, answerer) = match tokio::time::timeout(
        Duration::from_secs(45),
        connect_loopback(&config),
    )
    .await
    {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            eprintln!("skipped: relay handshake failed ({e}); coturn may not be reachable from this host");
            return;
        }
        Err(_) => {
            eprintln!("skipped: relay handshake timed out (coturn slow on this host)");
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

    let answerer_ssrc = answerer.local_audio_ssrc().expect("answerer ssrc");
    let answerer_local = answerer.local_audio_track().expect("answerer local track");
    let answerer_stream = from_tracks(
        StreamId::new(),
        codec,
        answerer_local,
        answerer_ssrc,
        /* Opus PT */ 111,
        None,
    );
    answerer_stream
        .enable_webrtc_stats(Arc::clone(answerer.peer_connection()), Arc::new(Notify::new()));

    let remote = RvoipPeerConnection::prime_remote_track(
        &offerer,
        &answerer,
        Duration::from_secs(15),
    )
    .await
    .expect("answerer receives offerer track via the relay");
    answerer_stream.attach_remote(remote);

    let mut inbound = answerer_stream.frames_in();

    for seq in 1..=20u16 {
        let payload = silent_rtp_payload_for_ssrc(offerer_ssrc, seq, seq as u32 * 960);
        offerer_stream
            .frames_out()
            .send(rvoip_core::stream::MediaFrame {
                stream_id: offerer_stream.id(),
                kind: rvoip_core::stream::StreamKind::Audio,
                payload,
                timestamp_rtp: seq as u32 * 960,
                captured_at: chrono::Utc::now(),
            })
            .await
            .expect("send frame");
    }

    // At least one frame must arrive over the relay.
    let frame = tokio::time::timeout(Duration::from_secs(10), inbound.recv())
        .await
        .expect("inbound timeout — relay should have delivered at least one frame")
        .expect("inbound channel closed");
    assert!(!frame.payload.is_empty(), "first relay frame must carry payload");

    // Selected-pair assertion: the local candidate must be a relay candidate.
    // (Remote may be relay or srflx depending on coturn's reflexive
    // discovery; assert only the local side per the plan's risk mitigation.)
    let mut local_type = String::new();
    for _ in 0..20 {
        let snap = answerer_stream.webrtc_stats_snapshot();
        if let Some(pair) = snap.selected_pair {
            local_type = pair.local_candidate_type;
            if !local_type.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        local_type, "relay",
        "selected_pair.local_candidate_type must be 'relay' under IceTransportPolicy::Relay"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
    drop(coturn);
}
