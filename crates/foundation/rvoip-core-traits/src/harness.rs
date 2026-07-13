//! P5 — provider trait surface for the recording / transcription / AI
//! harness paths. Per `INTERFACE_DESIGN.md` §2.1 these trait shapes
//! live in `rvoip-core` so the Orchestrator can dispatch generically;
//! concrete impls (Whisper, Claude, S3, …) ship in consumer crates or
//! in `rvoip-harness` (which re-exports these and supplies no-op
//! defaults).

use crate::error::Result;
use crate::ids::{ConnectionId, ParticipantId, StreamId};
use crate::stream::MediaFrame;
use async_trait::async_trait;
use std::fmt;

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

#[derive(Clone, Default)]
pub struct TtsRequest {
    pub voice: Option<String>,
    pub text: String,
    pub sample_rate_hz: Option<u32>,
}

impl fmt::Debug for TtsRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TtsRequest")
            .field("voice_present", &self.voice.is_some())
            .field("text_present", &!self.text.is_empty())
            .field("text_bytes", &self.text.len())
            .field("sample_rate_hz", &self.sample_rate_hz)
            .finish()
    }
}

#[async_trait]
pub trait TtsPlayback: Send + Sync {
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
