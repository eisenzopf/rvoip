use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

pub mod cloud_openai;
pub mod cloud_grok;
pub mod cloud_gemini;
pub mod cascaded;
pub mod factory;

/// Unified event emitted by any VoiceAI provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VoiceAiEvent {
    /// Real-time transcription
    Transcription {
        speaker: String,
        text: String,
        is_final: bool,
        confidence: f32,
    },
    /// Intent detection
    Intent {
        intent: String,
        confidence: f32,
    },
    /// Sentiment analysis
    Sentiment {
        value: f32,
        label: String,
    },
    /// AI suggested response (streaming chunks)
    SuggestionStart,
    SuggestionChunk {
        text: String,
    },
    SuggestionEnd,
    /// Audio response (TTS output, base64-encoded PCM)
    AudioResponse {
        pcm_base64: String,
        sample_rate: u32,
    },
    /// Knowledge base references
    KnowledgeRef {
        articles: Vec<KnowledgeArticle>,
    },
    /// Quality checklist update
    QualityCheck {
        items: Vec<QualityItem>,
    },
    /// Error
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeArticle {
    pub id: String,
    pub title: String,
    pub relevance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityItem {
    pub name: String,
    pub checked: bool,
    pub reminder: bool,
}

/// Call context -- accumulated state for one phone call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallContext {
    pub call_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_department: String,
    pub customer_number: String,
    pub customer_vip: bool,
    pub customer_vip_level: i32,
    pub queue_name: String,
    /// RAG-retrieved article texts
    pub knowledge_articles: Vec<String>,
    /// Relevant talk scripts
    pub talk_scripts: Vec<String>,
    /// Quality check rules
    pub quality_rules: Vec<String>,
    pub conversation_history: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// "system", "user" (customer), or "assistant" (AI/agent)
    pub role: String,
    pub content: String,
}

/// The unified provider trait -- all three routes implement this
#[async_trait]
pub trait VoiceAiProvider: Send + Sync {
    /// Provider name
    fn name(&self) -> &str;

    /// Process a text input (customer utterance) and emit events
    async fn process_text(
        &self,
        text: &str,
        context: &CallContext,
        event_tx: mpsc::Sender<VoiceAiEvent>,
    ) -> anyhow::Result<()>;

    /// Process audio frames (for Route A/B that handle audio directly).
    /// Default: not supported (Route C does ASR separately).
    async fn process_audio(
        &self,
        _pcm_samples: &[i16],
        _sample_rate: u32,
        _context: &CallContext,
        _event_tx: mpsc::Sender<VoiceAiEvent>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("audio processing not supported by this provider")
    }

    /// Whether this provider handles audio directly (Route A/B) or needs text (Route C)
    fn supports_audio(&self) -> bool {
        false
    }
}
