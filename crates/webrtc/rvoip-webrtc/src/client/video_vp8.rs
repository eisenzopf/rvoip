//! D3b — VP8 encoder + RFC 7741 packetizer wrapped as a [`VideoSource`].
//!
//! Composes:
//!
//! - [`crate::client::video::VideoSource`] — the trait we implement.
//! - `vpx-encode` (libvpx) — VP8 encoder (I420 input → VP8 byte stream).
//! - [`crate::media::packetize::vp8`] — RFC 7741 packetizer.
//!
//! Threading: libvpx's encoder context is `!Send`, so the encoder runs
//! on a dedicated `std::thread` worker; the `VideoSource` impl owns
//! only the inbound (raw YUV) and outbound (encoded RTP payloads)
//! channel ends, which keeps it `Send + Sync` for tokio.

use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc;
use vpx_encode::{Config as VpxConfig, Encoder as VpxEncoder, VideoCodecId};

use crate::client::video::{VideoCodec, VideoFrame, VideoSource, YuvFrame};
use crate::errors::{Result, WebRtcError};
use crate::media::packetize::vp8::{packetize_vp8, DEFAULT_MTU};

/// VP8 codec tuning knobs.
#[derive(Clone, Copy, Debug)]
pub struct Vp8Settings {
    pub width: u32,
    pub height: u32,
    /// Target bitrate (kbps). Default 1000 kbps (typical webcam).
    pub bitrate_kbps: u32,
    /// Frames per second (used for rate-control). Default 30.
    pub fps: u32,
    /// MTU for the packetizer. Default 1200 (matches webrtc-rs).
    pub mtu: usize,
}

impl Default for Vp8Settings {
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

/// VP8 [`VideoSource`]. Internally owns a worker thread running the
/// libvpx encoder; the public surface is fully async + Send.
pub struct Vp8VideoSource {
    yuv_tx: mpsc::Sender<YuvFrame>,
    encoded_rx: mpsc::Receiver<EncodedAccessUnit>,
    cfg: Vp8Settings,
    timestamp_rtp: u32,
}

struct EncodedAccessUnit {
    rtp_payloads: Vec<Bytes>,
    keyframe: bool,
}

impl Vp8VideoSource {
    /// Construct a new VP8 source and spawn its encoder worker thread.
    /// Frames pushed to [`Self::sender`] are encoded and packetized on
    /// the worker; the trait method consumes the result asynchronously.
    pub fn new(cfg: Vp8Settings) -> Result<Self> {
        let (yuv_tx, mut yuv_rx) = mpsc::channel::<YuvFrame>(8);
        let (encoded_tx, encoded_rx) = mpsc::channel::<EncodedAccessUnit>(8);
        let vpx_cfg = VpxConfig {
            width: cfg.width,
            height: cfg.height,
            timebase: [1, 90_000],
            bitrate: cfg.bitrate_kbps,
            codec: VideoCodecId::VP8,
        };

        thread::Builder::new()
            .name("rvoip-vp8-encoder".to_string())
            .spawn(move || {
                let mut encoder = match VpxEncoder::new(vpx_cfg) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::warn!(target: "rvoip_webrtc", ?err, "VP8 encoder init failed");
                        return;
                    }
                };
                let mtu = cfg.mtu;
                while let Some(frame) = yuv_rx.blocking_recv() {
                    let mut i420 = Vec::with_capacity(
                        frame.y.len() + frame.u.len() + frame.v.len(),
                    );
                    i420.extend_from_slice(&frame.y);
                    i420.extend_from_slice(&frame.u);
                    i420.extend_from_slice(&frame.v);
                    let pts = frame.capture_time.as_micros() as i64;
                    let packets = match encoder.encode(pts, &i420) {
                        Ok(p) => p,
                        Err(err) => {
                            tracing::trace!(target: "rvoip_webrtc", ?err, "VP8 encode failed; skipping frame");
                            continue;
                        }
                    };
                    let mut keyframe = false;
                    let mut rtp_payloads = Vec::new();
                    for packet in packets {
                        if packet.key {
                            keyframe = true;
                        }
                        for vp8 in packetize_vp8(packet.data, mtu) {
                            rtp_payloads.push(vp8.payload);
                        }
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
            .map_err(|e| WebRtcError::Adapter(format!("spawn VP8 encoder thread: {e}")))?;

        Ok(Self {
            yuv_tx,
            encoded_rx,
            cfg,
            timestamp_rtp: 0,
        })
    }

    /// Get a sender for pushing raw I420 frames into the encoder.
    pub fn sender(&self) -> mpsc::Sender<YuvFrame> {
        self.yuv_tx.clone()
    }
}

#[async_trait]
impl VideoSource for Vp8VideoSource {
    async fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        let Some(au) = self.encoded_rx.recv().await else {
            return Ok(None);
        };
        let tick = 90_000 / self.cfg.fps.max(1);
        let ts = self.timestamp_rtp;
        self.timestamp_rtp = self.timestamp_rtp.wrapping_add(tick);
        Ok(Some(VideoFrame::Encoded {
            codec: VideoCodec::Vp8,
            rtp_packets: au.rtp_payloads,
            timestamp_rtp: ts,
            keyframe: au.keyframe,
        }))
    }
}
