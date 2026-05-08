mod common;

use chrono::{Duration as ChronoDuration, Utc};
use common::support_call;
use rvoip_orchestration_core::prelude::*;
use rvoip_registrar_core::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarService, Transport,
};

fn contact(uri: &str, q_value: f32) -> ContactInfo {
    ContactInfo {
        uri: uri.to_string(),
        instance_id: format!("deskphone-{q_value}"),
        transport: Transport::UDP,
        user_agent: "deskphone".to_string(),
        expires: Utc::now() + ChronoDuration::minutes(10),
        q_value,
        received: None,
        path: Vec::new(),
        methods: vec!["INVITE".to_string()],
        reg_id: None,
        flow_id: None,
        reachability: ContactReachability::Unknown,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let registrar = std::sync::Arc::new(RegistrarService::new().await.map_err(|error| {
        OrchestrationError::ContactResolutionFailed(
            AgentId::from("registrar"),
            format!("failed to start registrar: {error}"),
        )
    })?);
    let alice_aor = AddressOfRecord::parse("sip:alice@example.com").map_err(|error| {
        OrchestrationError::ContactResolutionFailed(
            AgentId::from("alice"),
            format!("invalid AOR: {error}"),
        )
    })?;
    registrar
        .register_aor(
            &alice_aor,
            contact("sip:alice@192.0.2.10:5060", 0.5),
            Some(300),
        )
        .await
        .map_err(|error| {
            OrchestrationError::ContactResolutionFailed(
                AgentId::from("alice"),
                format!("failed to register alice: {error}"),
            )
        })?;
    registrar
        .register_aor(
            &alice_aor,
            contact("sip:alice@192.0.2.20:5060", 1.0),
            Some(300),
        )
        .await
        .map_err(|error| {
            OrchestrationError::ContactResolutionFailed(
                AgentId::from("alice"),
                format!("failed to register alice: {error}"),
            )
        })?;

    let resolver = RegistrarContactResolver::new(registrar);
    let queue_id = QueueId::from("support");
    let mut agent = Agent::human("alice", "sip:placeholder@example.com");
    agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };
    agent.state = AgentState::Available;
    agent.skills.push(Skill::from("support"));

    let resolved = resolver.resolve_contact(&agent).await?;
    let orchestrator = Orchestrator::builder()
        .with_contact_resolver(resolver)
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
        .expect("registered SIP agent");

    println!(
        "registered SIP agent {} assigned to call {} via {}",
        assignment.agent_id, call_id, resolved.uri
    );
    Ok(())
}
