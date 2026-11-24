//! G9a — TURN relay E2E test.
//!
//! Starts a hermetic in-process TURN server and verifies the adapter can use
//! it via `IceTransportPolicy::Relay`. Relay setup failures fail the test.

mod support {
    pub mod coturn_fixture;
}

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::config::IceTransportPolicy;
use rvoip_webrtc::media::{from_tracks, silent_opus_payload};
use rvoip_webrtc::peer::{connect_loopback, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;
use support::coturn_fixture::CoturnFixture;
use tokio::sync::Notify;

#[tokio::test]
async fn relay_policy_with_coturn_fixture_builds_peer() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let coturn = CoturnFixture::start().await.expect("start TURN fixture");

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;

    let peer = Arc::new(
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
            .await
            .expect("peer should build with Relay policy + coturn ICE config"),
    );
    peer.add_local_audio_track().await.expect("audio");

    let offer = tokio::time::timeout(Duration::from_secs(8), peer.create_offer_and_gather())
        .await
        .expect("offer deadline")
        .expect("relay offer");
    let candidates: Vec<_> = offer
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .collect();
    assert!(
        !candidates.is_empty(),
        "relay policy must advertise a candidate"
    );
    assert!(candidates.iter().all(|line| line.contains(" typ relay")));
    peer.close().await.expect("close relay peer");

    drop(peer);
    coturn.wait_for_no_allocations().await;
    coturn.close().await.expect("close TURN fixture");
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
    let coturn = CoturnFixture::start().await.expect("start TURN fixture");

    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    // Relay handshakes need more time than host (allocation + permission +
    // STUN binding round trips through coturn).
    config.gather_timeout_secs = 15;

    let (offerer, answerer) =
        tokio::time::timeout(Duration::from_secs(45), connect_loopback(&config))
            .await
            .expect("relay handshake timed out")
            .expect("relay handshake failed");

    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 2,
        fmtp: None,
        payload_type: None,
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
    answerer_stream.enable_webrtc_stats(
        Arc::clone(answerer.peer_connection()),
        Arc::new(Notify::new()),
    );

    let remote =
        RvoipPeerConnection::prime_remote_track(&offerer, &answerer, Duration::from_secs(15))
            .await
            .expect("answerer receives offerer track via the relay");
    answerer_stream.attach_remote(remote);

    let mut inbound = answerer_stream.try_frames_in().unwrap();

    for seq in 1..=20u16 {
        let payload = silent_opus_payload();
        offerer_stream
            .frames_out()
            .send(rvoip_core::stream::MediaFrame {
                stream_id: offerer_stream.id(),
                kind: rvoip_core::stream::StreamKind::Audio,
                payload,
                timestamp_rtp: seq as u32 * 960,
                captured_at: chrono::Utc::now(),
                payload_type: None,
            })
            .await
            .expect("send frame");
    }

    // At least one frame must arrive over the relay.
    let frame = tokio::time::timeout(Duration::from_secs(10), inbound.recv())
        .await
        .expect("inbound timeout — relay should have delivered at least one frame")
        .expect("inbound channel closed");
    assert!(
        !frame.payload.is_empty(),
        "first relay frame must carry payload"
    );

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

    // Verify the reverse media direction too; this must traverse the second
    // allocation and its permission rather than a directly reachable host.
    let reverse =
        RvoipPeerConnection::prime_remote_track(&answerer, &offerer, Duration::from_secs(10))
            .await
            .expect("reverse relay track");
    offerer_stream.attach_remote(reverse);
    let mut reverse_inbound = offerer_stream.try_frames_in().unwrap();
    answerer_stream
        .frames_out()
        .send(rvoip_core::stream::MediaFrame {
            stream_id: answerer_stream.id(),
            kind: rvoip_core::stream::StreamKind::Audio,
            payload: silent_opus_payload(),
            timestamp_rtp: 960 * 40,
            captured_at: chrono::Utc::now(),
            payload_type: None,
        })
        .await
        .unwrap();
    assert!(
        !tokio::time::timeout(Duration::from_secs(10), reverse_inbound.recv())
            .await
            .unwrap()
            .unwrap()
            .payload
            .is_empty()
    );
    assert_eq!(coturn.allocation_count().await, 2);
    offerer.close().await.ok();
    answerer.close().await.ok();
    coturn.wait_for_no_allocations().await;
    coturn.close().await.expect("close TURN fixture");
}

#[tokio::test]
async fn relay_peer_drop_releases_allocation() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let coturn = CoturnFixture::start().await.unwrap();
    let mut config = WebRtcConfig::loopback();
    config.ice_servers = vec![coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    let peer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .unwrap();
    assert_eq!(coturn.allocation_count().await, 1);
    drop(peer);
    coturn.wait_for_no_allocations().await;
    coturn.close().await.unwrap();
}

#[tokio::test]
async fn relay_policy_fails_closed_without_usable_credentials() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let coturn = CoturnFixture::start().await.unwrap();
    let mut config = WebRtcConfig::loopback();
    config.ice_transport_policy = IceTransportPolicy::Relay;
    assert!(RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .is_err());
    let mut server = coturn.ice_config();
    server.credential = Some("wrong-test-password".into());
    config.ice_servers = vec![server];
    config.gather_timeout_secs = 1;
    assert!(tokio::time::timeout(
        Duration::from_secs(3),
        RvoipPeerConnection::new(&config, PeerRole::Offerer)
    )
    .await
    .expect("bounded failed allocation")
    .is_err());
    coturn.wait_for_no_allocations().await;
    coturn.close().await.unwrap();
}

#[tokio::test]
async fn allocation_falls_back_to_a_reachable_server() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let coturn = CoturnFixture::start().await.unwrap();
    let mut config = WebRtcConfig::loopback();
    let mut unavailable = coturn.ice_config();
    unavailable.urls = vec![format!(
        "turn:{}?transport=udp",
        blackhole.local_addr().unwrap()
    )];
    config.ice_servers = vec![unavailable, coturn.ice_config()];
    config.ice_transport_policy = IceTransportPolicy::Relay;
    config.turn_attempt_timeout_secs = 1;
    config.gather_timeout_secs = 4;
    let peer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("second TURN server must be attempted");
    peer.close().await.unwrap();
    coturn.wait_for_no_allocations().await;
    coturn.close().await.unwrap();
}
