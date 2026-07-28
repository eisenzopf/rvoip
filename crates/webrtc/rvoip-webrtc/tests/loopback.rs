//! End-to-end loopback: two peer connections + RTP frame on outbound track.

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::config::WebRtcConfig;
use rvoip_webrtc::media::{from_tracks, silent_opus_payload};
use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use tokio::sync::Notify;

#[tokio::test]
async fn loopback_peer_connections_connect() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let (offerer, answerer) = connect_loopback(&config)
        .await
        .expect("offer/answer loopback");

    offerer.close().await.ok();
    answerer.close().await.ok();
}

#[tokio::test]
async fn loopback_rtp_inbound_round_trip() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let (offerer, answerer) = connect_loopback(&config)
        .await
        .expect("offerer/answerer loopback");

    let codec = CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
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
    answerer_stream.enable_webrtc_stats(
        Arc::clone(answerer.peer_connection()),
        Arc::new(Notify::new()),
    );

    let remote =
        RvoipPeerConnection::prime_remote_track(&offerer, &answerer, Duration::from_secs(10))
            .await
            .expect("answerer receives offerer track after priming RTP");
    answerer_stream.attach_remote(remote);

    let mut inbound = answerer_stream.frames_in();

    for seq in 1..=5u16 {
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

    let frame = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("inbound timeout")
        .expect("inbound frame");

    assert!(!frame.payload.is_empty());

    offerer.close().await.ok();
    answerer.close().await.ok();
}
