use crate::config::AiCopilotConfig;
use super::VoiceAiProvider;
use super::{cloud_openai, cloud_grok, cloud_gemini, cascaded};

/// Create a provider instance from configuration
pub fn create_provider(config: &AiCopilotConfig) -> anyhow::Result<Box<dyn VoiceAiProvider>> {
    match config.route.as_str() {
        "cloud_s2s" => match config.cloud_s2s_provider.as_str() {
            "openai_realtime" => Ok(Box::new(cloud_openai::OpenAiRealtimeProvider::new(
                &config.openai_api_key,
                &config.openai_model,
            ))),
            "grok_voice" => Ok(Box::new(cloud_grok::GrokVoiceProvider::new(
                &config.grok_api_key,
                &config.grok_model,
            ))),
            "gemini_live" => Ok(Box::new(cloud_gemini::GeminiLiveProvider::new(
                &config.gemini_api_key,
                &config.gemini_model,
            ))),
            other => anyhow::bail!("unknown cloud_s2s provider: {}", other),
        },
        "cascaded" => Ok(Box::new(cascaded::CascadedProvider::new(config)?)),
        other => anyhow::bail!("unknown route: {}", other),
    }
}
