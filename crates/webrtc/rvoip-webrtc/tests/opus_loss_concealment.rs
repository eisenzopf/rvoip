//! Short inbound Opus loss is concealed across a real peer connection: the
//! remote track's negotiated codec reaches the inbound pump, which forwards a
//! lost-frame indication for each missing packet.

use std::time::Duration;

use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::config::WebRtcConfig;
use rvoip_webrtc::media::from_tracks;
use rvoip_webrtc::media::pump::silent_rtp_packet_for_ssrc;
use rvoip_webrtc::peer::{connect_loopback, RvoipPeerConnection};
use webrtc::media_stream::track_local::TrackLocal;

#[tokio::test]
async fn short_opus_loss_is_concealed_and_long_loss_is_not() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let (offerer, answerer) = connect_loopback(&config)
        .await
        .expect("offerer/answerer loopback");

    let answerer_stream = from_tracks(
        StreamId::new(),
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 2,
            fmtp: None,
            payload_type: None,
        },
        answerer.local_audio_track().expect("answerer local track"),
        answerer.local_audio_ssrc().expect("answerer ssrc"),
        111,
        None,
    );
    let remote =
        RvoipPeerConnection::prime_remote_track(&offerer, &answerer, Duration::from_secs(10))
            .await
            .expect("answerer receives offerer track after priming RTP");
    answerer_stream.attach_remote(remote);
    let mut inbound = answerer_stream.try_frames_in().unwrap();

    // Raw RTP on a timeline well ahead of the priming packets: 5000..=5002,
    // two lost (5003, 5004), 5005, then six lost (5006..=5011) and 5012.
    let local = offerer.local_audio_track().expect("offerer local track");
    let ssrc = offerer.local_audio_ssrc().expect("offerer ssrc");
    let frame = |sequence: u16| 4_800_000 + u32::from(sequence - 5000) * 960;
    for sequence in [5000u16, 5001, 5002, 5005, 5012] {
        local
            .write_rtp(silent_rtp_packet_for_ssrc(ssrc, sequence, frame(sequence)))
            .await
            .expect("write RTP");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Skip the priming packets, then collect through the last packet.
    let mut received = Vec::new();
    while received.last().map(|(timestamp, _)| *timestamp) != Some(frame(5012)) {
        let next = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("inbound timeout")
            .expect("inbound frame");
        if !received.is_empty() || next.timestamp_rtp == frame(5000) {
            received.push((next.timestamp_rtp, next.payload.len()));
        }
    }

    // Silent Opus frames are three bytes; a lost-frame indication is the
    // one-byte TOC.
    assert_eq!(
        received,
        vec![
            (frame(5000), 3),
            (frame(5001), 3),
            (frame(5002), 3),
            (frame(5003), 1),
            (frame(5004), 1),
            (frame(5005), 3),
            (frame(5012), 3),
        ]
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
}
