mod common;

use common::{available_human, support_call};
use rvoip_orchestration_core::prelude::*;

#[deprecated(
    note = "Workforce/queue orchestration moves out of rvoip in PRD §13.3 step 7 — to Thelve or your own consumer. The SIP-only primitives this example exercises (UnifiedCoordinator, BridgeManager) live on at rvoip_sip::api / rvoip_core::bridge."
)]
#[tokio::main]
async fn main() -> Result<()> {
    let queue_id = QueueId::from("support");
    let agent = available_human("alice", "sip:alice@127.0.0.1:5071", &["support"]);

    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(agent.clone())
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
        .expect("available human agent");
    handle.accept_offer(&assignment.offer_id).await?;

    let connected = handle.get_call(&call_id).await?.expect("call exists");
    println!(
        "human queue connected call {} to agent {:?} with status {:?}",
        call_id, connected.assigned_agent_id, connected.status
    );
    Ok(())
}
