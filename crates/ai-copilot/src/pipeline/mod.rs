use std::sync::Arc;
use tokio::sync::mpsc;

use crate::db::ConversationRecorder;
use crate::providers::{CallContext, Message, VoiceAiEvent, VoiceAiProvider};

/// Orchestrates AI analysis for an active call
pub struct CopilotPipeline {
    provider: Box<dyn VoiceAiProvider>,
    recorder: Option<Arc<ConversationRecorder>>,
}

impl CopilotPipeline {
    pub fn new(
        provider: Box<dyn VoiceAiProvider>,
        recorder: Option<Arc<ConversationRecorder>>,
    ) -> Self {
        Self { provider, recorder }
    }

    /// Process customer text and emit events
    pub async fn process_customer_text(
        &self,
        text: &str,
        context: &mut CallContext,
        event_tx: mpsc::Sender<VoiceAiEvent>,
    ) -> anyhow::Result<()> {
        // Add to conversation history
        context.conversation_history.push(Message {
            role: "user".into(),
            content: text.to_string(),
        });

        let turn_index = context.conversation_history.len() as i32;

        // Record customer utterance asynchronously
        if let Some(recorder) = &self.recorder {
            let call_id = context.call_id.clone();
            let text_owned = text.to_string();
            let recorder = recorder.clone();
            tokio::spawn(async move {
                if let Err(e) = recorder
                    .record_turn(
                        &call_id,
                        turn_index,
                        "customer",
                        Some(&text_owned),
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    tracing::warn!("Failed to record turn: {}", e);
                }
            });
        }

        // Send transcription event
        let _ = event_tx
            .send(VoiceAiEvent::Transcription {
                speaker: "customer".into(),
                text: text.to_string(),
                is_final: true,
                confidence: 1.0,
            })
            .await;

        // Process with AI provider
        let start = std::time::Instant::now();
        self.provider
            .process_text(text, context, event_tx.clone())
            .await?;
        let latency = start.elapsed().as_millis() as i32;

        tracing::info!(
            provider = %self.provider.name(),
            latency_ms = latency,
            "AI analysis complete"
        );

        Ok(())
    }
}
