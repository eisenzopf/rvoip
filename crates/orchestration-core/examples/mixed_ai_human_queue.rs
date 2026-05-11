mod common;

use common::{available_ai, available_human, fake_runtime, say, support_call};
use rvoip_orchestration_core::prelude::*;

#[deprecated(
    note = "Workforce/queue orchestration moves out of rvoip in PRD §13.3 step 7 — to Thelve or your own consumer. Voice-AI runtime moves to rvoip-harness in PRD §13.3 step 6. The SIP-only primitives this example exercises (UnifiedCoordinator, BridgeManager) live on at rvoip_sip::api / rvoip_core::bridge."
)]
#[tokio::main]
async fn main() -> Result<()> {
    let queue_id = QueueId::from("support");
    let runtime_id = VoiceAiId::from("triage-runtime");
    let mut queue = Queue::new(queue_id.clone(), "Mixed Support");
    queue.policy = QueuePolicy::AiFirstThenHuman;

    let mut ai = available_ai("triage-ai", runtime_id.as_ref(), &["support"]);
    ai.state = AgentState::Offline;
    let human = available_human("alice", "sip:alice@127.0.0.1:5071", &["support"]);

    let orchestrator = Orchestrator::builder()
        .with_voice_ai_runtime(runtime_id, fake_runtime(vec![say("AI triage")]))
        .with_queue(queue)
        .with_agent(ai)
        .with_agent(human.clone())
        .build()
        .await?;
    let handle = orchestrator.handle();

    let call = support_call("sip:customer@example.com");
    let call_id = call.id.clone();
    handle.create_call(call).await?;
    handle
        .enqueue_call(
            call_id,
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await?;

    let assignment = handle
        .assign_next_call(&queue_id)
        .await?
        .expect("human fallback");
    println!(
        "AI-first queue fell back to human agent {} for offer {}",
        assignment.agent_id, assignment.offer_id
    );
    assert_eq!(assignment.agent_id, human.id);
    Ok(())
}
