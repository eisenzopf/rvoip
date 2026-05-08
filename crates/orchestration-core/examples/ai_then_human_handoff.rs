mod common;

use common::{available_ai, available_human, fake_runtime, say, support_call, transfer_to_queue};
use rvoip_orchestration_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let queue_id = QueueId::from("support");
    let runtime_id = VoiceAiId::from("triage-runtime");
    let ai = available_ai("triage-ai", runtime_id.as_ref(), &["triage"]);
    let human = available_human("alice", "sip:alice@127.0.0.1:5071", &["support"]);

    let orchestrator = Orchestrator::builder()
        .with_voice_ai_runtime(
            runtime_id,
            fake_runtime(vec![
                say("Let me triage that."),
                transfer_to_queue(queue_id.as_ref()),
            ]),
        )
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(ai.clone())
        .with_agent(human.clone())
        .build()
        .await?;
    let handle = orchestrator.handle();

    let mut call = support_call("sip:customer@example.com");
    call.context.external_ref = Some("ticket-123".to_string());
    call.context
        .metadata
        .insert("intent".to_string(), "billing".to_string());
    let call_id = call.id.clone();
    handle.create_call(call).await?;

    let ai_offer = handle.offer_agent(call_id.clone(), ai.id.clone()).await?;
    handle.accept_offer(&ai_offer).await?;
    handle
        .apply_voice_ai_action(
            call_id.clone(),
            ai.id,
            VoiceAiAction::TransferToQueue {
                queue_id: queue_id.clone(),
            },
        )
        .await?;

    let human_assignment = handle
        .assign_next_call(&queue_id)
        .await?
        .expect("human handoff assignment");
    handle.accept_offer(&human_assignment.offer_id).await?;

    let call = handle.get_call(&call_id).await?.expect("call exists");
    println!(
        "handoff preserved intent {:?} and connected to {:?}",
        call.context.metadata.get("intent"),
        call.assigned_agent_id
    );
    assert_eq!(call.assigned_agent_id, Some(human.id));
    Ok(())
}
