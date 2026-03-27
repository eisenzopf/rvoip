use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{CallContext, VoiceAiEvent, VoiceAiProvider};
use super::cloud_openai::parse_and_emit_analysis;

/// Gemini Live provider (Google Generative Language API)
pub struct GeminiLiveProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GeminiLiveProvider {
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
impl VoiceAiProvider for GeminiLiveProvider {
    fn name(&self) -> &str {
        "gemini_live"
    }

    async fn process_text(
        &self,
        text: &str,
        context: &CallContext,
        event_tx: mpsc::Sender<VoiceAiEvent>,
    ) -> anyhow::Result<()> {
        let system_prompt = Self::build_system_prompt(context);

        // Build Gemini-style request body
        let mut contents = Vec::new();
        // System instruction as first user turn
        contents.push(serde_json::json!({
            "role": "user",
            "parts": [{"text": system_prompt}]
        }));
        contents.push(serde_json::json!({
            "role": "model",
            "parts": [{"text": "明白，我会按照要求分析客户发言并返回 JSON。"}]
        }));

        for msg in &context.conversation_history {
            let role = match msg.role.as_str() {
                "assistant" => "model",
                _ => "user",
            };
            contents.push(serde_json::json!({
                "role": role,
                "parts": [{"text": msg.content}]
            }));
        }
        contents.push(serde_json::json!({
            "role": "user",
            "parts": [{"text": text}]
        }));

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key,
        );

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "contents": contents,
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

            if let Some(content) =
                parsed["candidates"][0]["content"]["parts"][0]["text"].as_str()
            {
                parse_and_emit_analysis(content, &event_tx).await;
            }
        } else {
            tracing::warn!("Failed to parse Gemini response as JSON");
        }

        Ok(())
    }
}
