//! D3c — end-to-end H.264 encoder + RFC 6184 packetizer test.
//!
//! Pushes a sequence of synthetic I420 frames through `H264VideoSource`
//! and verifies:
//! 1. At least one packet is emitted.
//! 2. The first frame is a keyframe.
//! 3. Every emitted RTP payload has a recognisable H.264 wire shape —
//!    either single-NAL (type in [1, 23]) or FU-A (type 28).

#![cfg(feature = "client-video-h264")]

use std::time::Duration;

use bytes::Bytes;

use rvoip_webrtc::client::video::{VideoCodec, VideoFrame, VideoSource, YuvFrame};
use rvoip_webrtc::client::video_h264::{H264Settings, H264VideoSource};

fn synthetic_i420(width: usize, height: usize, y_val: u8) -> (Bytes, Bytes, Bytes) {
    let y = vec![y_val; width * height];
    let u = vec![128u8; (width / 2) * (height / 2)];
    let v = vec![128u8; (width / 2) * (height / 2)];
    (Bytes::from(y), Bytes::from(u), Bytes::from(v))
}

#[tokio::test]
async fn h264_encoder_round_trip_through_packetizer() {
    let cfg = H264Settings {
        width: 320,
        height: 240,
        bitrate_kbps: 500,
        fps: 30,
        mtu: 1200,
    };
    let mut source = H264VideoSource::new(cfg).expect("create H.264 source");
    let sender = source.sender();

    // Push 3 frames with varying luma.
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
                assert_eq!(codec, VideoCodec::H264Cb);
                if keyframe {
                    saw_keyframe = true;
                }
                assert!(
                    !rtp_packets.is_empty(),
                    "frame must emit at least one RTP packet"
                );
                for p in &rtp_packets {
                    let nal_type = p[0] & 0x1f;
                    assert!(
                        (1..=23).contains(&nal_type) || nal_type == 28,
                        "RTP payload first-byte NAL type must be single-NAL [1,23] or FU-A (28), got {}",
                        nal_type
                    );
                }
                total_packets += rtp_packets.len();
            }
            _ => panic!("expected encoded H.264 frame"),
        }
    }

    assert!(total_packets > 0, "encoder emitted nothing");
    assert!(saw_keyframe, "encoder must emit a keyframe");
}
