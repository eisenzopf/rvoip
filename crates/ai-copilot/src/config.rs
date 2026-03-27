use serde::Deserialize;

/// Configuration for the AI Copilot subsystem
#[derive(Debug, Clone, Deserialize)]
pub struct AiCopilotConfig {
    pub enabled: bool,
    /// Route type: "cloud_s2s" | "local_s2s" | "cascaded"
    pub route: String,

    // Cloud S2S settings
    pub cloud_s2s_provider: String,
    pub openai_api_key: String,
    pub openai_model: String,
    pub grok_api_key: String,
    pub grok_model: String,
    pub gemini_api_key: String,
    pub gemini_model: String,

    // Cascaded LLM settings
    pub llm_url: String,
    pub llm_api_key: String,
    pub llm_model: String,

    // Feature toggles
    pub enable_transcription: bool,
    pub enable_intent: bool,
    pub enable_sentiment: bool,
    pub enable_suggestions: bool,
    pub enable_quality: bool,
    pub enable_rag: bool,
}

impl Default for AiCopilotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            route: "cascaded".into(),
            cloud_s2s_provider: "openai_realtime".into(),
            openai_api_key: String::new(),
            openai_model: "gpt-4o".into(),
            grok_api_key: String::new(),
            grok_model: "grok-3-fast".into(),
            gemini_api_key: String::new(),
            gemini_model: "gemini-2.5-flash".into(),
            llm_url: "http://127.0.0.1:8200/v1/chat/completions".into(),
            llm_api_key: String::new(),
            llm_model: "default".into(),
            enable_transcription: true,
            enable_intent: true,
            enable_sentiment: true,
            enable_suggestions: true,
            enable_quality: true,
            enable_rag: true,
        }
    }
}
