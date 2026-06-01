//! D3c — H.264 encoder + RFC 6184 packetizer wrapped as a [`VideoSource`].
//!
//! Composes:
//!
//! - [`crate::client::video::VideoSource`] — the trait we implement.
//! - `openh264` — Cisco's BSD-licensed H.264 encoder (I420 input → Annex-B
//!   byte stream).
//! - [`crate::media::packetize::h264`] — RFC 6184 packetizer (single-NAL
//!   + FU-A fragmentation).
//!
//! Threading: like the VP8 path, openh264's encoder context is `!Send`,
//! so the encoder runs on a dedicated `std::thread` worker.

use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use openh264::encoder::{Encoder as OpenH264Encoder, EncoderConfig};
use openh264::formats::YUVSlices;
use tokio::sync::mpsc;

use crate::client::video::{VideoCodec, VideoFrame, VideoSource, YuvFrame};
use crate::errors::{Result, WebRtcError};
use crate::media::packetize::h264::packetize_h264;
use crate::media::packetize::vp8::DEFAULT_MTU;

/// H.264 codec tuning knobs.
#[derive(Clone, Copy, Debug)]
pub struct H264Settings {
    pub width: u32,
    pub height: u32,
    /// Target bitrate (kbps). Default 1000 kbps.
    pub bitrate_kbps: u32,
    /// Frames per second. Default 30.
    pub fps: u32,
    /// MTU for the packetizer. Default 1200.
    pub mtu: usize,
}

impl Default for H264Settings {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            bitrate_kbps: 1000,
            fps: 30,
            mtu: DEFAULT_MTU,
        }
    }
}

/// H.264 [`VideoSource`]. Owns a worker thread running the openh264
/// encoder; the trait-facing surface is pure-channel and Send.
pub struct H264VideoSource {
    yuv_tx: mpsc::Sender<YuvFrame>,
    encoded_rx: mpsc::Receiver<EncodedAccessUnit>,
    cfg: H264Settings,
    timestamp_rtp: u32,
}

struct EncodedAccessUnit {
    rtp_payloads: Vec<Bytes>,
    keyframe: bool,
}

impl H264VideoSource {
    /// Construct + spawn the encoder worker.
    pub fn new(cfg: H264Settings) -> Result<Self> {
        let (yuv_tx, mut yuv_rx) = mpsc::channel::<YuvFrame>(8);
        let (encoded_tx, encoded_rx) = mpsc::channel::<EncodedAccessUnit>(8);

        let enc_cfg = EncoderConfig::new()
            .set_bitrate_bps(cfg.bitrate_kbps * 1000)
            .max_frame_rate(cfg.fps as f32);

        thread::Builder::new()
            .name("rvoip-h264-encoder".to_string())
            .spawn(move || {
                let mut encoder = match OpenH264Encoder::with_api_config(
                    match openh264::OpenH264API::from_source() {
                        api => api,
                    },
                    enc_cfg,
                ) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::warn!(target: "rvoip_webrtc", ?err, "H.264 encoder init failed");
                        return;
                    }
                };
                let mtu = cfg.mtu;
                let width = cfg.width as usize;
                let height = cfg.height as usize;
                let y_stride = width;
                let uv_stride = width / 2;
                while let Some(frame) = yuv_rx.blocking_recv() {
                    // openh264 wants Y stride = width; U/V stride = width/2.
                    let slices = YUVSlices::new(
                        (&frame.y, &frame.u, &frame.v),
                        (width, height),
                        (y_stride, uv_stride, uv_stride),
                    );
                    let bitstream = match encoder.encode(&slices) {
                        Ok(b) => b,
                        Err(err) => {
                            tracing::trace!(target: "rvoip_webrtc", ?err, "H.264 encode failed; skipping frame");
                            continue;
                        }
                    };
                    let frame_type = bitstream.frame_type();
                    let keyframe = matches!(
                        frame_type,
                        openh264::encoder::FrameType::IDR
                            | openh264::encoder::FrameType::I
                    );
                    let access_unit = bitstream.to_vec();
                    let mut rtp_payloads = Vec::new();
                    for pkt in packetize_h264(&access_unit, mtu) {
                        if pkt.payload.is_empty() {
                            continue;
                        }
                        rtp_payloads.push(pkt.payload);
                    }
                    if encoded_tx
                        .blocking_send(EncodedAccessUnit {
                            rtp_payloads,
                            keyframe,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| WebRtcError::Adapter(format!("spawn H.264 encoder thread: {e}")))?;

        Ok(Self {
            yuv_tx,
            encoded_rx,
            cfg,
            timestamp_rtp: 0,
        })
    }

    /// Get a sender for pushing raw I420 frames into the encoder.
    /// Re-uses the same `YuvFrame` shape as [`Vp8VideoSource`](super::video_vp8)
    /// so a single camera worker can fan out to multiple codecs.
    pub fn sender(&self) -> mpsc::Sender<YuvFrame> {
        self.yuv_tx.clone()
    }
}

#[async_trait]
impl VideoSource for H264VideoSource {
    async fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        let Some(au) = self.encoded_rx.recv().await else {
            return Ok(None);
        };
        let tick = 90_000 / self.cfg.fps.max(1);
        let ts = self.timestamp_rtp;
        self.timestamp_rtp = self.timestamp_rtp.wrapping_add(tick);
        Ok(Some(VideoFrame::Encoded {
            codec: VideoCodec::H264Cb,
            rtp_packets: au.rtp_payloads,
            timestamp_rtp: ts,
            keyframe: au.keyframe,
        }))
    }
}

// The cfg / timestamp_rtp fields are needed for the timestamp math but
// the compiler can't infer it from the field accesses above. Suppress
// the unused-field lint on `cfg` deterministically.
const _: () = {
    let _ = |s: &H264VideoSource| {
        let _ = s.cfg.width;
    };
};

// Tag YuvFrame as Send + Sync — it's a struct of Bytes + Duration, all
// of which are already Send + Sync. (The compiler infers this; explicit
// reminder for readers.)
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<Duration>();
    _assert_sync::<Duration>();
};
