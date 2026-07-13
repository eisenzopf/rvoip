//! D3 — `VideoSource` / `VideoSink` trait surface and the `VideoFrame`
//! payload enum.
//!
//! This module ships the trait shape callers will program against. The
//! concrete encoder/packetizer implementations land separately:
//!
//! - **D3b — `client-video-vp8`** (deferred): `NokhwaCameraSource` →
//!   `Vp8Encoder` (`vpx-encode`) → RFC 7741 packetizer.
//! - **D3c — `client-video-h264`** (deferred): `NokhwaCameraSource` →
//!   `H264Encoder` (`openh264`) → RFC 6184 STAP-A / FU-A packetizer.
//!
//! Why deferred: both require workspace `Cargo.toml` additions for
//! `nokhwa` / `vpx-encode` / `openh264`. `openh264` in particular fetches
//! a Cisco-provided binary at build time and changes the licensing
//! footprint of the crate. That work belongs in its own PR.
//!
//! See [`docs/GAP_PLAN.md`](../../../docs/GAP_PLAN.md) §3.1 D3 for the
//! full plan and risk write-up.

use async_trait::async_trait;
use bytes::Bytes;
use std::fmt;
use std::time::Duration;

use crate::errors::Result;

/// One raw I420 frame ready for encoding. Shared between the VP8 and
/// H.264 video sources (`client-video-vp8` / `client-video-h264`) so a
/// single camera capture worker can drive multiple codecs.
#[derive(Clone)]
pub struct YuvFrame {
    pub y: Bytes,
    pub u: Bytes,
    pub v: Bytes,
    pub capture_time: Duration,
}

impl fmt::Debug for YuvFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("YuvFrame")
            .field("y_bytes", &self.y.len())
            .field("u_bytes", &self.u.len())
            .field("v_bytes", &self.v.len())
            .field("capture_time", &self.capture_time)
            .finish()
    }
}

/// One outbound video frame.
///
/// Two shapes are supported:
///
/// 1. **`Encoded`** — a stream of already-RTP-packetized video payloads.
///    This is the shape the outbound pump consumes today (mirrors the
///    `AudioSource` contract — see [`crate::media::pump`]).
/// 2. **`YuvI420`** — a raw camera frame intended to be encoded +
///    packetized later in the pipeline. Today no shipped backend consumes
///    this variant; D3b/c will add `Vp8VideoSource` /
///    `H264VideoSource` that ingest `YuvI420` and yield `Encoded` packets.
#[derive(Clone)]
pub enum VideoFrame {
    /// One or more RTP packets carrying an encoded video payload.
    /// `rtp_packets` is the *full* RTP wire image (header + payload) per
    /// packet; the outbound pump writes each via `TrackLocalStaticRTP::write_rtp`.
    Encoded {
        codec: VideoCodec,
        rtp_packets: Vec<Bytes>,
        timestamp_rtp: u32,
        keyframe: bool,
    },
    /// Raw I420 camera frame, pre-encode. `y`, `u`, `v` planes use the
    /// usual 4:2:0 layout (`u`/`v` are half-width and half-height).
    YuvI420 {
        width: u32,
        height: u32,
        y: Bytes,
        u: Bytes,
        v: Bytes,
        capture_time: Duration,
    },
}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoded {
                codec,
                rtp_packets,
                timestamp_rtp,
                keyframe,
            } => {
                let rtp_bytes = rtp_packets
                    .iter()
                    .fold(0usize, |total, packet| total.saturating_add(packet.len()));
                formatter
                    .debug_struct("VideoFrame::Encoded")
                    .field("codec", codec)
                    .field("rtp_packet_count", &rtp_packets.len())
                    .field("rtp_bytes", &rtp_bytes)
                    .field("timestamp_rtp", timestamp_rtp)
                    .field("keyframe", keyframe)
                    .finish()
            }
            Self::YuvI420 {
                width,
                height,
                y,
                u,
                v,
                capture_time,
            } => formatter
                .debug_struct("VideoFrame::YuvI420")
                .field("width", width)
                .field("height", height)
                .field("y_bytes", &y.len())
                .field("u_bytes", &u.len())
                .field("v_bytes", &v.len())
                .field("capture_time", capture_time)
                .finish(),
        }
    }
}

/// Video codec the encoded frames are using. The MIME type maps to the
/// SDP m-line codec registered by [`crate::peer::builder::build_media_engine`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoCodec {
    Vp8,
    Vp9,
    /// Constrained-baseline H.264, profile-level-id `42e01f`, packetization-mode 1.
    H264Cb,
}

/// Source of outbound video frames. Mirrors [`crate::client::AudioSource`].
#[async_trait]
pub trait VideoSource: Send + Sync {
    /// Produce the next outbound video frame. Returning `Ok(None)` signals
    /// end-of-stream and stops the runner.
    async fn next_frame(&mut self) -> Result<Option<VideoFrame>>;
}

/// Sink for inbound video frames received from the peer.
#[async_trait]
pub trait VideoSink: Send + Sync {
    /// Consume one inbound `VideoFrame`. Errors do not stop the runner.
    async fn write_frame(&mut self, frame: VideoFrame) -> Result<()>;
}

/// D3 — fixture source that yields no frames. Used by tests that want a
/// `VideoSource` placeholder until real device-backed sources land.
pub struct NullVideoSource;

#[async_trait]
impl VideoSource for NullVideoSource {
    async fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        // Block the runtime briefly so a tight polling loop doesn't spin.
        tokio::time::sleep(Duration::from_millis(33)).await;
        Ok(Some(VideoFrame::YuvI420 {
            width: 0,
            height: 0,
            y: Bytes::new(),
            u: Bytes::new(),
            v: Bytes::new(),
            capture_time: Duration::ZERO,
        }))
    }
}

/// D3 — discard-everything sink, for tests.
pub struct NullVideoSink;

#[async_trait]
impl VideoSink for NullVideoSink {
    async fn write_frame(&mut self, _frame: VideoFrame) -> Result<()> {
        Ok(())
    }
}
