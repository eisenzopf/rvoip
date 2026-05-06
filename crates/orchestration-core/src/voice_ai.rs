use crate::error::Result;
use crate::ids::*;
use crate::types::{CallContext, CallerIdentity};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_media_core::types::AudioFrame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct VoiceAiRuntime {
    pub asr: Arc<dyn AsrProvider>,
    pub tts: Arc<dyn TtsProvider>,
    pub dialog: Arc<dyn DialogManager>,
    pub recording: Option<Arc<dyn RecordingSink>>,
    pub config: VoiceAiRuntimeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceAiRuntimeConfig {
    pub answer_before_start: bool,
    pub allow_early_media: bool,
    pub enable_barge_in: bool,
    pub default_language: Option<String>,
    pub audio_format: AudioFormat,
}

impl Default for VoiceAiRuntimeConfig {
    fn default() -> Self {
        Self {
            answer_before_start: true,
            allow_early_media: false,
            enable_barge_in: true,
            default_language: None,
            audio_format: AudioFormat::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceAiSession {
    pub id: VoiceAiSessionId,
    pub call_id: CallId,
    pub caller_leg_id: CallLegId,
    pub agent_id: AgentId,
    pub status: VoiceAiSessionStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceAiSessionStatus {
    Starting,
    Greeting,
    Listening,
    Thinking,
    Speaking,
    Transferring,
    Ending,
    Ended,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceAiAction {
    Continue,
    Say { text: String },
    TransferToQueue { queue_id: QueueId },
    TransferToAgent { agent_id: AgentId },
    TransferToSipUri { uri: String },
    Hangup { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u8,
    pub encoding: AudioEncoding,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 8_000,
            channels: 1,
            encoding: AudioEncoding::Pcm16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioEncoding {
    Pcm16,
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn start_session(&self, config: AsrConfig) -> Result<Box<dyn AsrSession>>;
}

#[async_trait]
pub trait AsrSession: Send + Sync {
    async fn push_audio(&mut self, frame: AudioFrame) -> Result<()>;
    async fn next_transcript(&mut self) -> Result<Option<TranscriptEvent>>;
    async fn finish(&mut self) -> Result<()>;
    async fn cancel(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrConfig {
    pub format: AudioFormat,
    pub language: Option<String>,
    pub enable_partials: bool,
    pub enable_endpointing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TranscriptEvent {
    Partial {
        text: String,
        confidence: Option<f32>,
    },
    Final {
        text: String,
        confidence: Option<f32>,
    },
    EndOfUtterance,
    Error {
        reason: String,
    },
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, request: TtsRequest) -> Result<Box<dyn TtsStream>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub format: AudioFormat,
    pub voice: Option<String>,
    pub can_be_interrupted: bool,
}

#[async_trait]
pub trait TtsStream: Send + Sync {
    async fn next_audio(&mut self) -> Result<Option<AudioFrame>>;
    async fn cancel(&mut self) -> Result<()>;
}

#[async_trait]
pub trait DialogManager: Send + Sync {
    async fn start_call(&self, context: DialogCallContext) -> Result<DialogSessionId>;
    async fn on_transcript(
        &self,
        session_id: &DialogSessionId,
        transcript: TranscriptEvent,
    ) -> Result<DialogTurn>;
    async fn on_dtmf(&self, session_id: &DialogSessionId, digit: char) -> Result<DialogTurn>;
    async fn end_call(&self, session_id: &DialogSessionId) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogCallContext {
    pub call_id: CallId,
    pub agent_id: AgentId,
    pub caller: CallerIdentity,
    pub call_context: CallContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogTurn {
    pub say: Vec<String>,
    pub action: VoiceAiAction,
    pub metadata: HashMap<String, String>,
}

#[async_trait]
pub trait RecordingSink: Send + Sync {
    async fn start_recording(&self, call_id: &CallId, leg_id: &CallLegId) -> Result<RecordingId>;
    async fn write_audio(&self, recording_id: &RecordingId, frame: AudioFrame) -> Result<()>;
    async fn stop_recording(&self, recording_id: &RecordingId) -> Result<()>;
}
