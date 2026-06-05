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

// --- ASR ---------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct AsrConfig {
    pub language: Option<String>,
    pub model: Option<String>,
    pub partial_results: bool,
}

#[derive(Clone, Debug)]
pub struct AsrResult {
    pub stream_id: StreamId,
    pub speaker: Option<ParticipantId>,
    pub text: String,
    pub confidence: f32,
    pub is_final: bool,
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

#[derive(Clone, Debug, Default)]
pub struct TtsRequest {
    pub voice: Option<String>,
    pub text: String,
    pub sample_rate_hz: Option<u32>,
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

#[derive(Clone, Debug)]
pub enum DialogAction {
    Say { text: String, voice: Option<String> },
    Listen,
    End,
}

#[async_trait]
pub trait DialogManager: Send + Sync {
    async fn turn(&self, transcript: &AsrResult) -> Result<DialogAction>;
}

// --- RecordingSink -----------------------------------------------------

#[derive(Clone, Debug)]
pub struct RecordingArtifact {
    pub url: String,
    pub bytes_written: u64,
    pub duration_ms: u64,
    pub content_hash: String,
}

#[async_trait]
pub trait RecordingSink: Send + Sync {
    async fn write(&self, frame: MediaFrame) -> Result<()>;
    async fn close(&self) -> Result<RecordingArtifact>;
}
