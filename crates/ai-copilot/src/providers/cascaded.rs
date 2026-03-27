use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::config::AiCopilotConfig;
use super::{CallContext, VoiceAiEvent, VoiceAiProvider};
use super::cloud_openai::parse_and_emit_analysis;

/// Cascaded provider: chains ASR -> LLM -> TTS.
///
/// Currently implements the LLM analysis portion (text-in, analysis-out).
/// ASR and TTS are separate crate concerns handled by the pipeline.
pub struct CascadedProvider {
    llm_url: String,
    llm_api_key: String,
    llm_model: String,
    client: reqwest::Client,
}

impl CascadedProvider {
    pub fn new(config: &AiCopilotConfig) -> anyhow::Result<Self> {
        Ok(Self {
            llm_url: config.llm_url.clone(),
            llm_api_key: config.llm_api_key.clone(),
            llm_model: config.llm_model.clone(),
            client: reqwest::Client::new(),
        })
    }

    fn build_system_prompt(context: &CallContext) -> String {
        let vip_tag = if context.customer_vip { "(VIP)" } else { "" };
        format!(
            "你是 rvoip 呼叫中心的 AI 助手。\n\n\
            坐席: {} ({})\n\
            客户: {} {}\n\
            队列: {}\n\n\
            相关知识库:\n{}\n\n\
            话术参考:\n{}\n\n\
            质检要求:\n{}\n\n\
            请分析客户的最新发言，返回 JSON:\n\
            {{\"intent\": \"...\", \"sentiment\": 0.0, \"sentiment_label\": \"...\", \
            \"suggestion\": \"...\", \"quality_checklist\": {{...}}}}",
            context.agent_name,
            context.agent_department,
            context.customer_number,
            vip_tag,
            context.queue_name,
            context.knowledge_articles.join("\n"),
            context.talk_scripts.join("\n"),
            context.quality_rules.join("\n"),
        )
    }
}

#[async_trait]
impl VoiceAiProvider for CascadedProvider {
    fn name(&self) -> &str {
        "cascaded"
    }

    async fn process_text(
        &self,
        text: &str,
        context: &CallContext,
        event_tx: mpsc::Sender<VoiceAiEvent>,
    ) -> anyhow::Result<()> {
        let system_prompt = Self::build_system_prompt(context);

        let mut messages = vec![serde_json::json!({"role": "system", "content": system_prompt})];
        for msg in &context.conversation_history {
            messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
        }
        messages.push(serde_json::json!({"role": "user", "content": text}));

        let mut request = self
            .client
            .post(&self.llm_url)
            .json(&serde_json::json!({
                "model": &self.llm_model,
                "messages": messages,
                "stream": false,
            }));

        if !self.llm_api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.llm_api_key));
        }

        let resp = request.send().await?;
        let body = resp.text().await?;

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(err_msg) = parsed["error"]["message"].as_str() {
                let _ = event_tx
                    .send(VoiceAiEvent::Error {
                        message: err_msg.to_string(),
                    })
                    .await;
                return Ok(());
            }

            if let Some(content) = parsed["choices"][0]["message"]["content"].as_str() {
                parse_and_emit_analysis(content, &event_tx).await;
            }
        } else {
            tracing::warn!("Failed to parse cascaded LLM response as JSON");
        }

        Ok(())
    }
}
