mod common;

use common::{available_ai, fake_runtime, say, support_call};
use rvoip_orchestration_core::prelude::*;

#[deprecated(
    note = "Workforce/queue orchestration moves out of rvoip in PRD §13.3 step 7 — to Thelve or your own consumer. Voice-AI runtime moves to rvoip-harness in PRD §13.3 step 6. The SIP-only primitives this example exercises (UnifiedCoordinator, BridgeManager) live on at rvoip_sip::api / rvoip_core::bridge."
)]
#[tokio::main]
async fn main() -> Result<()> {
    let queue_id = QueueId::from("ai-support");
    let runtime_id = VoiceAiId::from("support-ai-runtime");
    let ai = available_ai("support-ai-1", runtime_id.as_ref(), &["support"]);

    let orchestrator = Orchestrator::builder()
        .with_voice_ai_runtime(
            runtime_id,
            fake_runtime(vec![say("I can help with support.")]),
        )
        .with_queue(Queue::new(queue_id.clone(), "AI Support"))
        .with_agent(ai.clone())
        .build()
        .await?;
    let handle = orchestrator.handle();

    let call = support_call("sip:customer@example.com");
    let call_id = call.id.clone();
    handle.create_call(call).await?;
    handle
        .enqueue_call(
            call_id.clone(),
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await?;

    let assignment = handle
        .assign_next_call(&queue_id)
        .await?
        .expect("available AI agent");
    handle.accept_offer(&assignment.offer_id).await?;
    handle
        .apply_voice_ai_action(
            call_id.clone(),
            ai.id,
            VoiceAiAction::Say {
                text: "I can help with support.".to_string(),
            },
        )
        .await?;

    let call = handle.get_call(&call_id).await?.expect("call exists");
    println!("AI-only queue call {} is now {:?}", call_id, call.status);
    Ok(())
}
