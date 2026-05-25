//! `AudioSource` / `AudioSink` traits for plugging real microphone / speaker
//! backends (e.g. `cpal`, `AVFoundation`, `WASAPI`) into a [`SessionHandle`].
//!
//! Production callers implement [`AudioSource::next_packet`] to feed Opus RTP
//! packets into the outbound `frames_out` channel of a `WebRtcMediaStream`.
//! Inbound packets arrive on `frames_in` and can be passed to an `AudioSink`.
//!
//! The crate ships a fixture backend ([`FixtureAudioSource`]) that emits the
//! existing silent / RFC-7587-compliant Opus burst — useful for tests and
//! demos but **not a substitute for a real audio device backend**. Add a
//! cpal-backed source in a follow-up `client-cpal` feature.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, StreamKind};

use crate::errors::Result;
use crate::media::pump::{silent_rtp_payload_for_ssrc, OPUS_PT_DEFAULT};

/// Source of outbound audio frames. Implementations return one RTP-ready
/// `MediaFrame` per call and pace themselves (the runner enforces the
/// 20 ms cadence between calls when paced).
#[async_trait]
pub trait AudioSource: Send + Sync {
    /// Produce the next outbound audio frame. Returning `Ok(None)` signals
    /// end-of-stream and stops the runner.
    async fn next_packet(&mut self) -> Result<Option<MediaFrame>>;
}

/// Sink for inbound audio frames received from the peer.
#[async_trait]
pub trait AudioSink: Send + Sync {
    /// Consume one inbound `MediaFrame`. Errors do not stop the runner.
    async fn write_packet(&mut self, frame: MediaFrame) -> Result<()>;
}

/// Pacing policy for [`run_audio`].
#[derive(Clone, Copy, Debug)]
pub enum AudioPacing {
    /// Send as fast as the source produces (e.g. when the source is already
    /// paced — cpal callback, file reader with its own timing).
    Unpaced,
    /// Insert a fixed delay between packets (e.g. 20 ms for Opus).
    Fixed(Duration),
}

impl Default for AudioPacing {
    fn default() -> Self {
        Self::Fixed(Duration::from_millis(20))
    }
}

/// Spawn paired outbound/inbound pumps that bridge an [`AudioSource`] and
/// [`AudioSink`] to a `WebRtcMediaStream`'s frame channels.
///
/// Returns join handles for the outbound and inbound tasks. Drop them to
/// abort, or `.await` them for graceful exit (outbound exits when the source
/// returns `Ok(None)`; inbound exits when the channel closes).
pub fn run_audio(
    mut source: Box<dyn AudioSource>,
    mut sink: Box<dyn AudioSink>,
    frames_out: mpsc::Sender<MediaFrame>,
    mut frames_in: mpsc::Receiver<MediaFrame>,
    pacing: AudioPacing,
) -> (JoinHandle<()>, JoinHandle<()>) {
    let out = tokio::spawn(async move {
        loop {
            match source.next_packet().await {
                Ok(Some(frame)) => {
                    if frames_out.send(frame).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
            if let AudioPacing::Fixed(d) = pacing {
                tokio::time::sleep(d).await;
            }
        }
    });
    let inb = tokio::spawn(async move {
        while let Some(frame) = frames_in.recv().await {
            let _ = sink.write_packet(frame).await;
        }
    });
    (out, inb)
}

/// In-memory audio source that emits silent Opus RTP packets for the given
/// SSRC at 20 ms intervals. Suitable for tests, loopback demos, and as a
/// reference implementation of [`AudioSource`].
pub struct FixtureAudioSource {
    stream_id: StreamId,
    ssrc: u32,
    seq: u16,
    timestamp: u32,
}

impl FixtureAudioSource {
    pub fn new(stream_id: StreamId, ssrc: u32) -> Self {
        Self {
            stream_id,
            ssrc,
            seq: 1,
            timestamp: 0,
        }
    }
}

#[async_trait]
impl AudioSource for FixtureAudioSource {
    async fn next_packet(&mut self) -> Result<Option<MediaFrame>> {
        let payload = silent_rtp_payload_for_ssrc(self.ssrc, self.seq, self.timestamp);
        let frame = MediaFrame {
            stream_id: self.stream_id.clone(),
            kind: StreamKind::Audio,
            payload,
            timestamp_rtp: self.timestamp,
            captured_at: Utc::now(),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(960); // 20 ms @ 48 kHz
        Ok(Some(frame))
    }
}

/// Discard-everything sink — useful for tests that only care about outbound.
pub struct NullAudioSink;

#[async_trait]
impl AudioSink for NullAudioSink {
    async fn write_packet(&mut self, _frame: MediaFrame) -> Result<()> {
        Ok(())
    }
}

/// Counting sink: records how many frames were received. Useful for asserting
/// inbound delivery in tests.
pub struct CountingAudioSink {
    pub count: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub last_payload: std::sync::Arc<parking_lot::Mutex<Option<Bytes>>>,
}

impl CountingAudioSink {
    pub fn new() -> Self {
        Self {
            count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_payload: std::sync::Arc::new(parking_lot::Mutex::new(None)),
        }
    }
}

impl Default for CountingAudioSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioSink for CountingAudioSink {
    async fn write_packet(&mut self, frame: MediaFrame) -> Result<()> {
        self.count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_payload.lock() = Some(frame.payload);
        Ok(())
    }
}

#[allow(dead_code)]
fn _opus_pt_check() {
    // Reference symbol so the import lints don't complain — every fixture
    // frame uses the default Opus payload type (111) the engine registers.
    let _ = OPUS_PT_DEFAULT;
}
