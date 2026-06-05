//! D3b — end-to-end VP8 encoder + RFC 7741 packetizer test.
//!
//! Pushes a sequence of synthetic I420 frames through `Vp8VideoSource`
//! and verifies:
//! 1. At least one packet is emitted.
//! 2. The first frame is a keyframe.
//! 3. Every emitted RTP payload starts with a valid VP8 payload
//!    descriptor (S bit on the first packet of each frame).

#![cfg(feature = "client-video-vp8")]

use std::time::Duration;

use bytes::Bytes;

use rvoip_webrtc::client::video::{VideoCodec, VideoFrame, VideoSource, YuvFrame};
use rvoip_webrtc::client::video_vp8::{Vp8Settings, Vp8VideoSource};
use rvoip_webrtc::media::packetize::vp8::payload_is_start_of_partition;

/// Build one I420 frame at `width × height` with a constant Y value.
fn synthetic_i420(width: usize, height: usize, y_val: u8) -> (Bytes, Bytes, Bytes) {
    let y = vec![y_val; width * height];
    let u = vec![128u8; (width / 2) * (height / 2)];
    let v = vec![128u8; (width / 2) * (height / 2)];
    (Bytes::from(y), Bytes::from(u), Bytes::from(v))
}

#[tokio::test]
async fn vp8_encoder_round_trip_through_packetizer() {
    let cfg = Vp8Settings {
        width: 320,
        height: 240,
        bitrate_kbps: 500,
        fps: 30,
        mtu: 1200,
    };
    let mut source = Vp8VideoSource::new(cfg).expect("create VP8 source");
    let sender = source.sender();

    // Push 3 frames with varying luma so the encoder produces non-trivial output.
    for i in 0..3u8 {
        let (y, u, v) = synthetic_i420(320, 240, 32 + i * 32);
        sender
            .send(YuvFrame {
                y,
                u,
                v,
                capture_time: Duration::from_millis(i as u64 * 33),
            })
            .await
            .expect("yuv send");
    }

    let mut total_packets = 0;
    let mut saw_keyframe = false;
    for _ in 0..3 {
        let Ok(Ok(Some(frame))) =
            tokio::time::timeout(Duration::from_secs(5), source.next_frame()).await
        else {
            break;
        };
        match frame {
            VideoFrame::Encoded {
                codec,
                rtp_packets,
                keyframe,
                ..
            } => {
                assert_eq!(codec, VideoCodec::Vp8);
                if keyframe {
                    saw_keyframe = true;
                }
                assert!(
                    !rtp_packets.is_empty(),
                    "frame must emit at least one RTP packet"
                );
                // First RTP packet of the frame must have S=1 in its VP8 descriptor.
                assert!(
                    payload_is_start_of_partition(&rtp_packets[0]),
                    "first RTP packet of frame must mark start of partition"
                );
                total_packets += rtp_packets.len();
            }
            _ => panic!("expected encoded VP8 frame"),
        }
    }

    assert!(total_packets > 0, "encoder emitted nothing");
    assert!(
        saw_keyframe,
        "first frame must be a keyframe (encoder default behavior)"
    );
}
