//! P5 — provider trait surface for the recording / transcription / AI
//! harness paths. Per `INTERFACE_DESIGN.md` §2.1 these trait shapes
//! live in `rvoip-core` so the Orchestrator can dispatch generically;
//! concrete impls (Whisper, Claude, S3, …) ship in consumer crates or
//! in `rvoip-harness` (which re-exports these and supplies no-op
//! defaults).

use crate::capability::CodecInfo;
use crate::error::Result;
use crate::ids::{ConnectionId, ParticipantId, RecordingId, StreamId};
use crate::stream::MediaFrame;
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;

// --- ASR ---------------------------------------------------------------

#[derive(Clone, Default)]
pub struct AsrConfig {
    pub language: Option<String>,
    pub model: Option<String>,
    pub partial_results: bool,
}

impl fmt::Debug for AsrConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsrConfig")
            .field("language_present", &self.language.is_some())
            .field("model_present", &self.model.is_some())
            .field("partial_results", &self.partial_results)
            .finish()
    }
}

#[derive(Clone)]
pub struct AsrResult {
    pub stream_id: StreamId,
    pub speaker: Option<ParticipantId>,
    pub text: String,
    pub confidence: f32,
    pub is_final: bool,
}

impl fmt::Debug for AsrResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsrResult")
            .field("stream_id", &self.stream_id)
            .field("speaker_present", &self.speaker.is_some())
            .field("text_present", &!self.text.is_empty())
            .field("text_bytes", &self.text.len())
            .field("confidence", &self.confidence)
            .field("is_final", &self.is_final)
            .finish()
    }
}

#[async_trait]
pub trait AsrStream: Send + Sync {
    async fn push(&self, frame: MediaFrame) -> Result<()>;
    async fn next(&self) -> Option<AsrResult>;
    async fn close(&self) -> Result<()>;
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn open_stream(
        &self,
        conn: ConnectionId,
        config: AsrConfig,
    ) -> Result<Box<dyn AsrStream>>;
}

// --- TTS ---------------------------------------------------------------

/// One synthesis request.
///
/// `destination_codec` is the codec negotiated on the stream the audio will
/// be played into, when there is one (`Orchestrator::play_audio` always sets
/// it; offline synthesis leaves it `None`). A provider that can synthesize
/// directly in that codec may do so and report
/// [`TtsAudioFormat::Encoded`]; every other provider reports
/// [`TtsAudioFormat::PcmS16Le`] and rvoip encodes for the destination.
///
/// `sample_rate_hz` is an optional PCM-rate *preference* and is never derived
/// from the destination codec: an RTP clock rate is not a PCM rate (Opus is
/// always 48000 on the wire, G.722 is 8000 on the wire but 16 kHz audio).
/// `play_audio` leaves it `None`, so the provider picks its native rate.
#[derive(Clone, Default)]
pub struct TtsRequest {
    pub voice: Option<String>,
    pub text: String,
    pub sample_rate_hz: Option<u32>,
    pub destination_codec: Option<CodecInfo>,
}

impl fmt::Debug for TtsRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TtsRequest")
            .field("voice_present", &self.voice.is_some())
            .field("text_present", &!self.text.is_empty())
            .field("text_bytes", &self.text.len())
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("destination_codec", &self.destination_codec)
            .finish()
    }
}

/// What the payloads of a [`TtsPlayback`]'s frames contain.
///
/// rvoip never forwards TTS payloads it cannot describe: the format decides
/// whether frames are encoded for the destination or only re-labelled, and
/// in both cases delivery is paced at real time (one 20 ms frame per 20 ms).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TtsAudioFormat {
    /// Mono signed 16-bit little-endian PCM at `sample_rate_hz` (8, 16, 24,
    /// 32 or 48 kHz). Frames may carry any number of samples; rvoip
    /// re-frames them into 20 ms chunks, zero-pads the final chunk, and
    /// encodes through the same encoder as `Orchestrator::play_pcm`.
    PcmS16Le { sample_rate_hz: u32 },
    /// Already encoded in `codec`, which must match the request's
    /// `destination_codec` by name, clock rate and channel count. Each frame
    /// must hold exactly one 20 ms packet. rvoip stamps the destination
    /// stream id, negotiated payload type and RTP timestamps.
    Encoded { codec: CodecInfo },
}

#[async_trait]
pub trait TtsPlayback: Send + Sync {
    /// The format of every frame this playback yields. Must not change
    /// during the playback.
    fn audio_format(&self) -> TtsAudioFormat;
    async fn next_frame(&self) -> Option<MediaFrame>;
    async fn cancel(&self) -> Result<()>;
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, request: TtsRequest) -> Result<Box<dyn TtsPlayback>>;
}

// --- DialogManager -----------------------------------------------------

#[derive(Clone)]
pub enum DialogAction {
    Say { text: String, voice: Option<String> },
    Listen,
    End,
}

impl fmt::Debug for DialogAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Say { text, voice } => formatter
                .debug_struct("Say")
                .field("text_present", &!text.is_empty())
                .field("text_bytes", &text.len())
                .field("voice_present", &voice.is_some())
                .finish(),
            Self::Listen => formatter.write_str("Listen"),
            Self::End => formatter.write_str("End"),
        }
    }
}

#[async_trait]
pub trait DialogManager: Send + Sync {
    async fn turn(&self, transcript: &AsrResult) -> Result<DialogAction>;
}

// --- RecordingSink -----------------------------------------------------

#[derive(Clone)]
pub struct RecordingArtifact {
    pub url: String,
    pub bytes_written: u64,
    pub duration_ms: u64,
    pub content_hash: String,
}

impl fmt::Debug for RecordingArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingArtifact")
            .field("url_present", &!self.url.is_empty())
            .field("url_bytes", &self.url.len())
            .field("bytes_written", &self.bytes_written)
            .field("duration_ms", &self.duration_ms)
            .field("content_hash_present", &!self.content_hash.is_empty())
            .field("content_hash_bytes", &self.content_hash.len())
            .finish()
    }
}

#[async_trait]
pub trait RecordingSink: Send + Sync {
    async fn write(&self, frame: MediaFrame) -> Result<()>;
    async fn close(&self) -> Result<RecordingArtifact>;
}

/// Opens one sink per recording.
///
/// A registered [`RecordingSink`] is a single shared instance, which is
/// sufficient only while at most one recording is ever in flight: two
/// concurrent recordings on the same registered name write into the same
/// sink, and the first `stop_recording` closes it out from under the second.
/// For a deployment recording many calls at once — and especially for many
/// tenants at once — that mixes audio and attributes it to whichever
/// recording stopped first.
///
/// Registering a factory instead gives every [`start_recording`] its own
/// sink instance, so writes and the closing artifact belong to exactly one
/// recording. The `recording_id` is supplied so an implementation can name
/// its destination deterministically.
///
/// [`start_recording`]: https://docs.rs/rvoip-core
#[async_trait]
pub trait RecordingSinkFactory: Send + Sync {
    async fn open(&self, recording_id: &RecordingId) -> Result<Arc<dyn RecordingSink>>;
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn harness_diagnostics_never_render_provider_or_media_text() {
        const CANARY: &str = "harness-canary\r\nAuthorization: exposed";
        let values = [
            format!(
                "{:?}",
                AsrConfig {
                    language: Some(CANARY.into()),
                    model: Some(CANARY.into()),
                    partial_results: true,
                }
            ),
            format!(
                "{:?}",
                AsrResult {
                    stream_id: StreamId::from_string(CANARY),
                    speaker: Some(ParticipantId::from_string(CANARY)),
                    text: CANARY.into(),
                    confidence: 0.9,
                    is_final: true,
                }
            ),
            format!(
                "{:?}",
                TtsRequest {
                    voice: Some(CANARY.into()),
                    text: CANARY.into(),
                    sample_rate_hz: Some(48_000),
                    destination_codec: Some(CodecInfo {
                        name: CANARY.into(),
                        clock_rate_hz: 8_000,
                        channels: 1,
                        fmtp: Some(CANARY.into()),
                        payload_type: Some(0),
                    }),
                }
            ),
            format!(
                "{:?}",
                DialogAction::Say {
                    text: CANARY.into(),
                    voice: Some(CANARY.into()),
                }
            ),
            format!(
                "{:?}",
                RecordingArtifact {
                    url: CANARY.into(),
                    bytes_written: 10,
                    duration_ms: 20,
                    content_hash: CANARY.into(),
                }
            ),
        ];
        for debug in values {
            assert!(!debug.contains(CANARY), "harness value leaked: {debug}");
        }
    }
}
