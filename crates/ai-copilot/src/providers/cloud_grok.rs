use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{CallContext, VoiceAiEvent, VoiceAiProvider};
use super::cloud_openai::parse_and_emit_analysis;

/// Grok Voice provider (x.ai OpenAI-compatible Chat Completions API)
pub struct GrokVoiceProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GrokVoiceProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
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
impl VoiceAiProvider for GrokVoiceProvider {
    fn name(&self) -> &str {
        "grok_voice"
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

        let resp = self
            .client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": &self.model,
                "messages": messages,
                "stream": false,
            }))
            .send()
            .await?;

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
            tracing::warn!("Failed to parse Grok response as JSON");
        }

        Ok(())
    }
}
