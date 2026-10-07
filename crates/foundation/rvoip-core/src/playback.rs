//! Bounded, caller-fed PCM playback. Asset retrieval and decoding belong to
//! the caller; this module accepts mono signed 16-bit PCM in 20 ms chunks.
//!
//! ```no_run
//! # async fn example(orch: &rvoip_core::Orchestrator, conn: rvoip_core::ConnectionId) -> Result<(), Box<dyn std::error::Error>> {
//! use rvoip_core::playback::PcmPlaybackSource;
//! let (mut input, source) = PcmPlaybackSource::channel(8_000, 4)?;
//! let playback = orch.play_pcm(conn, source).await?;
//! input.send(&[0_i16; 160]).await?; // 20 ms at 8 kHz
//! input.finish(&[]).await?; // close the source
//! let outcome = playback.wait().await?;
//! # let _ = outcome;
//! # Ok(())
//! # }
//! ```
//!
//! For longer assets, feed chunks concurrently with playback. Queue capacity
//! bounds queued PCM, independently of caller-owned assets and transport
//! queues. Completion means transport-queue acceptance, not remote playout.
use crate::{
    capability::CodecInfo,
    error::{Result, RvoipError},
    ids::StreamId,
    stream::{MediaFrame, StreamKind},
};
use bytes::Bytes;
use chrono::Utc;
use rvoip_media_core::{
    codec::{AudioCodec, AudioCodecSpec},
    processing::format::{ConversionParams, FormatConverter},
    types::{AudioFrame, SampleRate},
};
use std::time::Duration;
use tokio::sync::mpsc;

/// Maximum queued audio: one second at the source rate (50 x 20 ms).
pub const MAX_PCM_BUFFER_FRAMES: usize = 50;

/// Single producer for a bounded PCM playback source. Dropping it signals
/// end-of-stream; pending sends stop when playback is cancelled or fails.
pub struct PcmPlaybackSender {
    sender: mpsc::Sender<Vec<i16>>,
    samples_per_frame: usize,
}
impl PcmPlaybackSender {
    /// Enqueue exactly 20 ms of mono PCM. Reserve bounded capacity before
    /// copying the caller's samples, so blocked sends hold no SDK audio copy.
    pub async fn send(&mut self, samples: &[i16]) -> Result<()> {
        if samples.len() != self.samples_per_frame {
            return Err(RvoipError::AdmissionRejected(
                "PCM chunk must contain exactly 20 ms",
            ));
        }
        let permit = self
            .sender
            .reserve()
            .await
            .map_err(|_| RvoipError::InvalidState("PCM playback has ended"))?;
        permit.send(samples.to_vec());
        Ok(())
    }
    /// Enqueue an optional final chunk of at most 20 ms, zero-padding it to
    /// the codec frame size, then close the producer. Empty means no tail.
    pub async fn finish(self, tail: &[i16]) -> Result<()> {
        if tail.len() > self.samples_per_frame {
            return Err(RvoipError::AdmissionRejected("PCM tail exceeds 20 ms"));
        }
        if !tail.is_empty() {
            let permit = self
                .sender
                .reserve()
                .await
                .map_err(|_| RvoipError::InvalidState("PCM playback has ended"))?;
            let mut samples = vec![0; self.samples_per_frame];
            samples[..tail.len()].copy_from_slice(tail);
            permit.send(samples);
        }
        Ok(())
    }
}

/// One owned receiver; pass it to `Orchestrator::play_pcm`. No URL, path,
/// file format, network client, or unbounded audio buffer is accepted.
pub struct PcmPlaybackSource {
    receiver: mpsc::Receiver<Vec<i16>>,
    sample_rate_hz: u32,
}
impl PcmPlaybackSource {
    /// Build a bounded mono source at 8, 16, 24, 32, or 48 kHz. Queue
    /// capacity must be 1..=50 frames. Every chunk represents 20 ms.
    pub fn channel(
        sample_rate_hz: u32,
        capacity_frames: usize,
    ) -> Result<(PcmPlaybackSender, Self)> {
        if !matches!(sample_rate_hz, 8_000 | 16_000 | 24_000 | 32_000 | 48_000) {
            return Err(RvoipError::AdmissionRejected("unsupported PCM source rate"));
        }
        if !(1..=MAX_PCM_BUFFER_FRAMES).contains(&capacity_frames) {
            return Err(RvoipError::AdmissionRejected(
                "PCM queue capacity must be 1..=50",
            ));
        }
        let (sender, receiver) = mpsc::channel(capacity_frames);
        Ok((
            PcmPlaybackSender {
                sender,
                samples_per_frame: sample_rate_hz as usize / 50,
            },
            Self {
                receiver,
                sample_rate_hz,
            },
        ))
    }
}

pub(crate) struct PcmFrameProducer {
    source: PcmPlaybackSource,
    encoder: Box<dyn AudioCodec>,
    converter: FormatConverter,
    conversion: ConversionParams,
    stream_id: StreamId,
    payload_type: u8,
    timestamp: u32,
    clock_rate_hz: u32,
    last_delivery: Option<tokio::time::Instant>,
}
impl PcmFrameProducer {
    pub(crate) fn new(
        source: PcmPlaybackSource,
        stream_id: StreamId,
        codec: CodecInfo,
    ) -> Result<Self> {
        let (default_pt, expected_rate) = match codec.name.to_ascii_uppercase().as_str() {
            "PCMU" => (Some(0), 8_000),
            "PCMA" => (Some(8), 8_000),
            "OPUS" => (None, 48_000),
            _ => return Err(RvoipError::UnsupportedCodec(codec.name)),
        };
        if codec.clock_rate_hz != expected_rate
            || !(1..=2).contains(&codec.channels)
            || (expected_rate == 8_000 && codec.channels != 1)
        {
            return Err(RvoipError::AdmissionRejected(
                "unsupported PCM playback codec shape",
            ));
        }
        let payload_type =
            codec
                .payload_type
                .or(default_pt)
                .ok_or(RvoipError::AdmissionRejected(
                    "dynamic playback codec requires negotiated payload type",
                ))?;
        if payload_type > 127 || (payload_type < 96 && Some(payload_type) != default_pt) {
            return Err(RvoipError::AdmissionRejected(
                "invalid negotiated playback payload type",
            ));
        }
        let spec = AudioCodecSpec::new(
            &codec.name,
            payload_type,
            codec.clock_rate_hz,
            codec.channels,
        )
        .with_fmtp(codec.fmtp.as_deref());
        let encoder = spec
            .build()
            .map_err(|_| RvoipError::UnsupportedCodec(codec.name.clone()))?;
        let rate = SampleRate::from_hz(expected_rate)
            .ok_or(RvoipError::InvalidState("unsupported target sample rate"))?;
        Ok(Self {
            source,
            encoder,
            converter: FormatConverter::new(),
            conversion: ConversionParams::new(rate, codec.channels),
            stream_id,
            payload_type,
            timestamp: 0,
            clock_rate_hz: expected_rate,
            last_delivery: None,
        })
    }
    pub(crate) async fn next_frame(&mut self) -> Result<Option<MediaFrame>> {
        let Some(samples) = self.source.receiver.recv().await else {
            return Ok(None);
        };
        if let Some(previous) = self.last_delivery {
            tokio::time::sleep_until(previous + Duration::from_millis(20)).await;
        }
        let input = AudioFrame::new(samples, self.source.sample_rate_hz, 1, self.timestamp);
        let audio = self
            .converter
            .convert_frame(&input, &self.conversion)
            .map_err(|_| RvoipError::InvalidState("PCM playback conversion failed"))?
            .frame;
        let payload = self
            .encoder
            .encode(&audio)
            .map_err(|_| RvoipError::InvalidState("PCM playback encoding failed"))?;
        let frame = MediaFrame {
            stream_id: self.stream_id.clone(),
            kind: StreamKind::Audio,
            payload: Bytes::from(payload),
            timestamp_rtp: self.timestamp,
            captured_at: Utc::now(),
            payload_type: Some(self.payload_type),
        };
        self.timestamp = self.timestamp.wrapping_add(self.clock_rate_hz / 50);
        Ok(Some(frame))
    }
    pub(crate) fn delivered(&mut self) {
        self.last_delivery = Some(tokio::time::Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pcmu() -> CodecInfo {
        CodecInfo {
            name: "PCMU".into(),
            clock_rate_hz: 8_000,
            channels: 1,
            fmtp: None,
            payload_type: None,
        }
    }
    #[tokio::test]
    async fn pcm_bounds_and_tail_are_enforced() {
        assert!(PcmPlaybackSource::channel(0, 1).is_err());
        assert!(PcmPlaybackSource::channel(8_000, 0).is_err());
        assert!(PcmPlaybackSource::channel(8_000, 51).is_err());
        let (mut sender, mut source) = PcmPlaybackSource::channel(8_000, 1).unwrap();
        assert!(sender.send(&[0; 159]).await.is_err());
        sender.send(&[0; 160]).await.unwrap();
        assert_eq!(source.receiver.len(), 1);
        let blocked = tokio::time::timeout(Duration::from_millis(10), sender.send(&[0; 160])).await;
        assert!(blocked.is_err());
        assert_eq!(source.receiver.recv().await.unwrap().len(), 160);
        sender.finish(&[1000; 3]).await.unwrap();
        let tail = source.receiver.recv().await.unwrap();
        assert_eq!(&tail[..3], &[1000; 3]);
        assert!(tail[3..].iter().all(|sample| *sample == 0));
        assert!(source.receiver.recv().await.is_none());
    }
    #[tokio::test]
    async fn pcm_encodes_known_wire_samples_and_target_identity() {
        let (mut sender, source) = PcmPlaybackSource::channel(8_000, 2).unwrap();
        sender.send(&[1000; 160]).await.unwrap();
        sender.finish(&[]).await.unwrap();
        let stream = StreamId::new();
        let mut producer = PcmFrameProducer::new(source, stream.clone(), pcmu()).unwrap();
        let frame = producer.next_frame().await.unwrap().unwrap();
        assert_eq!(frame.stream_id, stream);
        assert_eq!(frame.payload_type, Some(0));
        assert_eq!(frame.payload.as_ref(), &[0xce; 160]);
        assert!(producer.next_frame().await.unwrap().is_none());
    }
    #[tokio::test]
    async fn pcm_encodes_pcma_without_a_transport_specific_payload_guess() {
        let (sender, source) = PcmPlaybackSource::channel(8_000, 1).unwrap();
        sender.finish(&[1000; 160]).await.unwrap();
        let mut codec = pcmu();
        codec.name = "PCMA".into();
        codec.payload_type = Some(103);
        let mut producer = PcmFrameProducer::new(source, StreamId::new(), codec).unwrap();
        let frame = producer.next_frame().await.unwrap().unwrap();
        assert_eq!(frame.payload_type, Some(103));
        assert_eq!(frame.payload.as_ref(), &[0xfa; 160]);
    }
    #[tokio::test]
    async fn pcm_resamples_and_preserves_twenty_ms_timestamp_spacing() {
        let (mut sender, source) = PcmPlaybackSource::channel(48_000, 2).unwrap();
        sender.send(&[0; 960]).await.unwrap();
        sender.finish(&[0; 960]).await.unwrap();
        let mut producer = PcmFrameProducer::new(source, StreamId::new(), pcmu()).unwrap();
        let a = producer.next_frame().await.unwrap().unwrap();
        producer.delivered();
        let start = tokio::time::Instant::now();
        let b = producer.next_frame().await.unwrap().unwrap();
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert_eq!(a.payload.len(), 160);
        assert_eq!(b.payload.as_ref(), &[0xff; 160]);
        assert_eq!(b.timestamp_rtp.wrapping_sub(a.timestamp_rtp), 160);
    }
    #[test]
    fn invalid_negotiated_codec_is_rejected_before_playback() {
        for (name, rate, channels, pt) in [
            ("PCMU", 48_000, 1, Some(0)),
            ("PCMA", 8_000, 1, Some(0)),
            ("OPUS", 48_000, 2, None),
            ("PCMU", 8_000, 2, Some(0)),
            ("PCMU", 8_000, 1, Some(128)),
        ] {
            let (_, source) = PcmPlaybackSource::channel(8_000, 1).unwrap();
            assert!(PcmFrameProducer::new(
                source,
                StreamId::new(),
                CodecInfo {
                    name: name.into(),
                    clock_rate_hz: rate,
                    channels,
                    fmtp: None,
                    payload_type: pt
                }
            )
            .is_err());
        }
    }
    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn pcm_encodes_negotiated_opus_and_upmixes_mono() {
        let (sender, source) = PcmPlaybackSource::channel(8_000, 1).unwrap();
        sender.finish(&[1000; 160]).await.unwrap();
        let mut producer = PcmFrameProducer::new(
            source,
            StreamId::new(),
            CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 2,
                fmtp: None,
                payload_type: Some(109),
            },
        )
        .unwrap();
        let frame = producer.next_frame().await.unwrap().unwrap();
        assert_eq!(frame.payload_type, Some(109));
        let mut decoder = AudioCodecSpec::new("opus", 109, 48_000, 2).build().unwrap();
        let decoded = decoder.decode(&frame.payload).unwrap();
        assert_eq!(decoded.samples.len(), 1920);
        assert_eq!(decoded.channels, 2);
    }
}
