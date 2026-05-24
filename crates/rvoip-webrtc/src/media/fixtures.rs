//! Synthetic encoded media fixtures for deterministic WebRTC tests.
//!
//! These are not live camera/microphone captures, but structurally valid Opus and
//! VP8 RTP payloads suitable for exercising encode/decode plumbing in loopback.

use std::sync::Arc;
use std::time::Duration;

use rtc::rtp;
use webrtc::media_stream::track_local::TrackLocal;

use crate::peer::RvoipPeerConnection;

/// Minimal Opus payload (TOC + silence frame bytes).
pub const OPUS_FIXTURE_PAYLOAD: &[u8] = &[0xF8, 0xFF, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00];

/// Minimal VP8 keyframe payload (P=0, start of partition, keyframe bit set).
pub const VP8_KEYFRAME_FIXTURE_PAYLOAD: &[u8] = &[0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a];

pub fn opus_rtp_packet_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker: seq % 50 == 0,
            payload_type: 111,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::from_static(OPUS_FIXTURE_PAYLOAD),
    }
}

pub fn vp8_rtp_packet_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker: seq == 1,
            payload_type: crate::peer::builder::VP8_PAYLOAD_TYPE,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::from_static(VP8_KEYFRAME_FIXTURE_PAYLOAD),
    }
}

/// Send fixture-encoded RTP bursts on local audio/video tracks.
pub async fn send_fixture_media_burst(peer: &Arc<RvoipPeerConnection>, include_video: bool) {
    let audio_local = peer.local_audio_track();
    let audio_ssrc = peer.local_audio_ssrc();
    let video_local = if include_video {
        peer.local_video_track()
    } else {
        None
    };
    let video_ssrc = peer.local_video_ssrc();

    for seq in 1..=25u16 {
        if let (Some(track), Some(ssrc)) = (&audio_local, audio_ssrc) {
            let pkt = opus_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 960);
            let _ = track.write_rtp(pkt).await;
        }
        if let (Some(track), Some(ssrc)) = (&video_local, video_ssrc) {
            let pkt = vp8_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 3000);
            let _ = track.write_rtp(pkt).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
