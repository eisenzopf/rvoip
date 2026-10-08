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
    harness::{TtsAudioFormat, TtsPlayback},
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
        check_pcm_source_rate(sample_rate_hz)?;
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

/// Packetization time of every core-owned playback frame.
pub(crate) const FRAME_PERIOD: Duration = Duration::from_millis(20);

/// How far a paced playback may fall behind its schedule before the schedule
/// re-anchors instead of bursting the backlog onto the transport.
const MAX_PACING_LAG: Duration = Duration::from_millis(60);

fn check_pcm_source_rate(sample_rate_hz: u32) -> Result<()> {
    if matches!(sample_rate_hz, 8_000 | 16_000 | 24_000 | 32_000 | 48_000) {
        Ok(())
    } else {
        Err(RvoipError::AdmissionRejected("unsupported PCM source rate"))
    }
}

fn static_payload_type(name: &str) -> Option<u8> {
    match name.to_ascii_uppercase().as_str() {
        "PCMU" => Some(0),
        "PCMA" => Some(8),
        "G722" => Some(9),
        "G729" => Some(18),
        _ => None,
    }
}

/// Payload type to stamp on playback frames: the negotiated one, or the
/// codec's static assignment. Dynamic codecs without a negotiated value are
/// refused rather than guessed.
fn destination_payload_type(codec: &CodecInfo) -> Result<u8> {
    let default_pt = static_payload_type(&codec.name);
    let payload_type = codec
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
    Ok(payload_type)
}

/// Destination identity and RTP timeline for consecutive 20 ms frames.
pub(crate) struct FrameStamp {
    stream_id: StreamId,
    payload_type: u8,
    timestamp: u32,
    ticks_per_frame: u32,
}
impl FrameStamp {
    fn new(stream_id: StreamId, codec: &CodecInfo) -> Result<Self> {
        let ticks_per_frame = codec.clock_rate_hz / 50;
        if ticks_per_frame == 0 {
            return Err(RvoipError::AdmissionRejected(
                "invalid negotiated playback clock rate",
            ));
        }
        Ok(Self {
            stream_id,
            payload_type: destination_payload_type(codec)?,
            timestamp: 0,
            ticks_per_frame,
        })
    }
    fn frame(&mut self, payload: Bytes) -> MediaFrame {
        let frame = MediaFrame {
            stream_id: self.stream_id.clone(),
            kind: StreamKind::Audio,
            payload,
            timestamp_rtp: self.timestamp,
            captured_at: Utc::now(),
            payload_type: Some(self.payload_type),
        };
        self.timestamp = self.timestamp.wrapping_add(self.ticks_per_frame);
        frame
    }
}

/// The single PCM-to-wire encoder for core-owned playback. `play_pcm` and
/// PCM-producing TTS providers both encode through it, so identical input
/// yields identical wire output: media-core resampling and channel mapping,
/// then the negotiated codec, then destination stream identity.
pub(crate) struct PcmEncoder {
    encoder: Box<dyn AudioCodec>,
    converter: FormatConverter,
    conversion: ConversionParams,
    source_rate_hz: u32,
    stamp: FrameStamp,
}
impl PcmEncoder {
    pub(crate) fn new(stream_id: StreamId, codec: CodecInfo, source_rate_hz: u32) -> Result<Self> {
        check_pcm_source_rate(source_rate_hz)?;
        let expected_rate = match codec.name.to_ascii_uppercase().as_str() {
            "PCMU" | "PCMA" => 8_000,
            "OPUS" => 48_000,
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
        let stamp = FrameStamp::new(stream_id, &codec)?;
        let spec = AudioCodecSpec::new(
            &codec.name,
            stamp.payload_type,
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
            encoder,
            converter: FormatConverter::new(),
            conversion: ConversionParams::new(rate, codec.channels),
            source_rate_hz,
            stamp,
        })
    }
    /// Mono source samples in one 20 ms frame.
    pub(crate) fn samples_per_frame(&self) -> usize {
        self.source_rate_hz as usize / 50
    }
    /// Encode exactly one 20 ms frame of mono source PCM.
    pub(crate) fn encode(&mut self, samples: Vec<i16>) -> Result<MediaFrame> {
        debug_assert_eq!(samples.len(), self.samples_per_frame());
        let input = AudioFrame::new(samples, self.source_rate_hz, 1, self.stamp.timestamp);
        let audio = self
            .converter
            .convert_frame(&input, &self.conversion)
            .map_err(|_| RvoipError::InvalidState("PCM playback conversion failed"))?
            .frame;
        let payload = self
            .encoder
            .encode(&audio)
            .map_err(|_| RvoipError::InvalidState("PCM playback encoding failed"))?;
        Ok(self.stamp.frame(Bytes::from(payload)))
    }
}

/// Caller-fed PCM (`play_pcm`) through the shared encoder.
pub(crate) struct PcmFrameProducer {
    source: PcmPlaybackSource,
    encoder: PcmEncoder,
}
impl PcmFrameProducer {
    pub(crate) fn new(
        source: PcmPlaybackSource,
        stream_id: StreamId,
        codec: CodecInfo,
    ) -> Result<Self> {
        let encoder = PcmEncoder::new(stream_id, codec, source.sample_rate_hz)?;
        Ok(Self { source, encoder })
    }
    pub(crate) async fn next_frame(&mut self) -> Result<Option<MediaFrame>> {
        match self.source.receiver.recv().await {
            Some(samples) => self.encoder.encode(samples).map(Some),
            None => Ok(None),
        }
    }
}

/// Re-frames little-endian PCM payloads of any length into 20 ms chunks.
/// Holds the provider's current payload plus at most one partial chunk.
pub(crate) struct PcmChunker {
    bytes_per_chunk: usize,
    pending: Bytes,
    partial: Vec<u8>,
}
impl PcmChunker {
    pub(crate) fn new(samples_per_chunk: usize) -> Self {
        let bytes_per_chunk = samples_per_chunk * 2;
        Self {
            bytes_per_chunk,
            pending: Bytes::new(),
            partial: Vec::with_capacity(bytes_per_chunk),
        }
    }
    /// Queue the next payload. Call only after `pop` returned `None`.
    pub(crate) fn push(&mut self, payload: Bytes) {
        debug_assert!(self.pending.is_empty());
        self.pending = payload;
    }
    /// The next complete chunk, if enough bytes are buffered.
    pub(crate) fn pop(&mut self) -> Option<Vec<i16>> {
        let take = (self.bytes_per_chunk - self.partial.len()).min(self.pending.len());
        self.partial.extend_from_slice(&self.pending.split_to(take));
        if self.partial.len() < self.bytes_per_chunk {
            return None;
        }
        let chunk = self
            .partial
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        self.partial.clear();
        Some(chunk)
    }
    /// The zero-padded remainder at end of stream, if any.
    pub(crate) fn finish(&mut self) -> Option<Vec<i16>> {
        debug_assert!(self.pending.is_empty());
        if self.partial.is_empty() {
            return None;
        }
        self.partial.resize(self.bytes_per_chunk, 0);
        self.pop()
    }
}

/// Turns a TTS playback's declared output into destination-ready frames.
pub(crate) enum TtsFrameAdapter {
    /// PCM re-framed to 20 ms and encoded by [`PcmEncoder`].
    Pcm {
        chunker: PcmChunker,
        encoder: PcmEncoder,
        ended: bool,
    },
    /// Provider-encoded 20 ms packets, re-stamped for the destination.
    Encoded(FrameStamp),
}
impl TtsFrameAdapter {
    pub(crate) fn new(
        format: TtsAudioFormat,
        stream_id: StreamId,
        destination: CodecInfo,
    ) -> Result<Self> {
        match format {
            TtsAudioFormat::PcmS16Le { sample_rate_hz } => {
                let encoder = PcmEncoder::new(stream_id, destination, sample_rate_hz)?;
                Ok(Self::Pcm {
                    chunker: PcmChunker::new(encoder.samples_per_frame()),
                    encoder,
                    ended: false,
                })
            }
            TtsAudioFormat::Encoded { codec }
                if codec.name.eq_ignore_ascii_case(&destination.name)
                    && codec.clock_rate_hz == destination.clock_rate_hz
                    && codec.channels == destination.channels =>
            {
                Ok(Self::Encoded(FrameStamp::new(stream_id, &destination)?))
            }
            TtsAudioFormat::Encoded { .. } => Err(RvoipError::AdmissionRejected(
                "TTS output codec does not match the destination codec",
            )),
            _ => Err(RvoipError::AdmissionRejected(
                "unsupported TTS audio format",
            )),
        }
    }
    pub(crate) async fn next_frame(
        &mut self,
        playback: &dyn TtsPlayback,
    ) -> Result<Option<MediaFrame>> {
        match self {
            Self::Encoded(stamp) => Ok(playback
                .next_frame()
                .await
                .map(|frame| stamp.frame(frame.payload))),
            Self::Pcm {
                chunker,
                encoder,
                ended,
            } => loop {
                if let Some(chunk) = chunker.pop() {
                    return encoder.encode(chunk).map(Some);
                }
                if *ended {
                    return Ok(None);
                }
                match playback.next_frame().await {
                    Some(frame) => chunker.push(frame.payload),
                    None => {
                        *ended = true;
                        return chunker
                            .finish()
                            .map(|chunk| encoder.encode(chunk))
                            .transpose();
                    }
                }
            },
        }
    }
}

/// Real-time pacing on a fixed schedule: each frame is released one
/// [`FRAME_PERIOD`] after the previous frame's *scheduled* release, so
/// encode and send time never accumulate into drift. A source that falls
/// more than [`MAX_PACING_LAG`] behind (a provider stall) re-anchors the
/// schedule at the current time instead of bursting its backlog.
#[derive(Default)]
pub(crate) struct FramePacer {
    next_release: Option<tokio::time::Instant>,
}
impl FramePacer {
    /// When the next frame may be sent; the first frame goes immediately.
    pub(crate) fn next_release(&mut self) -> tokio::time::Instant {
        let now = tokio::time::Instant::now();
        let release = match self.next_release {
            Some(due) if now.saturating_duration_since(due) <= MAX_PACING_LAG => due,
            _ => now,
        };
        self.next_release = Some(release + FRAME_PERIOD);
        release
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
        let b = producer.next_frame().await.unwrap().unwrap();
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

    #[test]
    fn chunker_reframes_uneven_payloads_and_pads_the_tail() {
        let mut chunker = PcmChunker::new(4);
        let le =
            |samples: &[i16]| -> Bytes { samples.iter().flat_map(|s| s.to_le_bytes()).collect() };
        assert!(chunker.pop().is_none());
        chunker.push(le(&[1, 2, 3]));
        assert!(chunker.pop().is_none());
        chunker.push(le(&[4, 5, 6, 7, 8, 9, -10]));
        assert_eq!(chunker.pop(), Some(vec![1, 2, 3, 4]));
        assert_eq!(chunker.pop(), Some(vec![5, 6, 7, 8]));
        assert!(chunker.pop().is_none());
        assert_eq!(chunker.finish(), Some(vec![9, -10, 0, 0]));
        assert!(chunker.finish().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn pacer_holds_a_fixed_schedule_despite_processing_time() {
        let mut pacer = FramePacer::default();
        let start = tokio::time::Instant::now();
        for n in 0..50_u32 {
            let release = pacer.next_release();
            assert_eq!(release - start, FRAME_PERIOD * n);
            tokio::time::sleep_until(release).await;
            // Encode/send work after each release must not push the
            // schedule back.
            tokio::time::sleep(Duration::from_millis(7)).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pacer_re_anchors_after_a_source_stall_instead_of_bursting() {
        let mut pacer = FramePacer::default();
        let start = tokio::time::Instant::now();
        pacer.next_release();
        // Slightly late: still on the original schedule (catch up).
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(pacer.next_release() - start, FRAME_PERIOD);
        // Stalled far beyond the lag bound: re-anchor at now.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let now = tokio::time::Instant::now();
        assert_eq!(pacer.next_release(), now);
        assert_eq!(pacer.next_release(), now + FRAME_PERIOD);
    }

    #[test]
    fn tts_output_must_match_the_destination() {
        let opus = CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
            payload_type: Some(111),
        };
        assert!(TtsFrameAdapter::new(
            TtsAudioFormat::Encoded { codec: opus },
            StreamId::new(),
            pcmu()
        )
        .is_err());
        assert!(TtsFrameAdapter::new(
            TtsAudioFormat::Encoded { codec: pcmu() },
            StreamId::new(),
            pcmu()
        )
        .is_ok());
        assert!(TtsFrameAdapter::new(
            TtsAudioFormat::PcmS16Le {
                sample_rate_hz: 22_050
            },
            StreamId::new(),
            pcmu()
        )
        .is_err());
        assert!(TtsFrameAdapter::new(
            TtsAudioFormat::PcmS16Le {
                sample_rate_hz: 24_000
            },
            StreamId::new(),
            pcmu()
        )
        .is_ok());
    }
}
