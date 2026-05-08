mod common;

use common::{fake_runtime, say, support_call, transfer_to_queue};
use rvoip_orchestration_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let runtime_id = VoiceAiId::from("triage-runtime");
    let queue_id = QueueId::from("support");
    let ai = common::available_ai("triage-ai", runtime_id.as_ref(), &["ivr", "support"]);

    let orchestrator = Orchestrator::builder()
        .with_voice_ai_runtime(
            runtime_id.clone(),
            fake_runtime(vec![
                say("How can I help?"),
                transfer_to_queue(queue_id.as_ref()),
            ]),
        )
        .with_agent(ai.clone())
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .build()
        .await?;
    let handle = orchestrator.handle();

    let call = support_call("sip:caller@example.com");
    let call_id = call.id.clone();
    handle.create_call(call).await?;

    let offer_id = handle.offer_agent(call_id.clone(), ai.id.clone()).await?;
    handle.accept_offer(&offer_id).await?;
    handle
        .apply_voice_ai_action(
            call_id.clone(),
            ai.id.clone(),
            VoiceAiAction::Say {
                text: "How can I help?".to_string(),
            },
        )
        .await?;
    handle
        .apply_voice_ai_action(
            call_id.clone(),
            ai.id,
            VoiceAiAction::TransferToQueue {
                queue_id: queue_id.clone(),
            },
        )
        .await?;

    let stats = handle.get_queue_stats(&queue_id).await?;
    println!(
        "speech IVR queued call {} in {} (queued={})",
        call_id, queue_id, stats.queued_calls
    );
    Ok(())
}
