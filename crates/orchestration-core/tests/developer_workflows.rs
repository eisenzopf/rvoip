mod support;

use chrono::{Duration as ChronoDuration, Utc};
use rvoip_orchestration_core::prelude::*;
use rvoip_sip_registrar::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarService, Transport,
};
use rvoip_sip::{CallState, UnifiedCoordinator};
use support::{available_ai, available_human, incoming_call, support_call};
use tokio::time::{timeout, Duration};

fn unused_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn local_session_config(name: &str) -> rvoip_sip::Config {
    let sip_port = unused_udp_port();
    let media_start = unused_udp_port();
    let mut config = rvoip_sip::Config::local(name, sip_port);
    config.media_port_start = media_start;
    config.media_port_end = media_start.saturating_add(20);
    config.unregister_on_shutdown_timeout_secs = 0;
    config
}

fn registered_contact(uri: impl Into<String>, q_value: f32) -> ContactInfo {
    ContactInfo {
        uri: uri.into(),
        instance_id: format!("instance-{}", uuid::Uuid::new_v4()),
        transport: Transport::UDP,
        user_agent: "orchestration-core-test".to_string(),
        expires: Utc::now() + ChronoDuration::minutes(5),
        q_value,
        received: None,
        path: Vec::new(),
        methods: vec!["INVITE".to_string()],
        reg_id: None,
        flow_id: None,
        reachability: ContactReachability::Unknown,
    }
}

#[tokio::test]
async fn speech_ivr_actions_say_transfer_and_hangup() {
    let orchestrator = Orchestrator::builder().build().await.unwrap();
    let handle = orchestrator.handle();
    let ai_id = AgentId::from("ivr-ai");
    let queue_id = QueueId::from("support");
    handle
        .upsert_queue(Queue::new(queue_id.clone(), "Support"))
        .await
        .unwrap();

    let say_call = support_call();
    let say_call_id = say_call.id.clone();
    handle.create_call(say_call).await.unwrap();
    handle
        .apply_voice_ai_action(
            say_call_id.clone(),
            ai_id.clone(),
            VoiceAiAction::Say {
                text: "For billing, press 1.".to_string(),
            },
        )
        .await
        .unwrap();
    let call = handle.get_call(&say_call_id).await.unwrap().unwrap();
    assert_eq!(call.status, CallStatus::InVoiceAi);
    assert_eq!(
        call.context.metadata.get("last_voice_ai_say"),
        Some(&"For billing, press 1.".to_string())
    );

    let transfer_call = support_call();
    let transfer_call_id = transfer_call.id.clone();
    handle.create_call(transfer_call).await.unwrap();
    handle
        .apply_voice_ai_action(
            transfer_call_id.clone(),
            ai_id.clone(),
            VoiceAiAction::TransferToQueue {
                queue_id: queue_id.clone(),
            },
        )
        .await
        .unwrap();
    let stats = handle.get_queue_stats(&queue_id).await.unwrap();
    assert_eq!(stats.queued_calls, 1);
    let transferred = handle.get_call(&transfer_call_id).await.unwrap().unwrap();
    assert_eq!(transferred.status, CallStatus::Queued);
    assert_eq!(
        transferred.context.metadata.get("handoff_from_agent_id"),
        Some(&ai_id.to_string())
    );

    let hangup_call = support_call();
    let hangup_call_id = hangup_call.id.clone();
    handle.create_call(hangup_call).await.unwrap();
    handle
        .apply_voice_ai_action(
            hangup_call_id.clone(),
            ai_id,
            VoiceAiAction::Hangup {
                reason: "self service complete".to_string(),
            },
        )
        .await
        .unwrap();
    let ended = handle.get_call(&hangup_call_id).await.unwrap().unwrap();
    assert_eq!(ended.status, CallStatus::Ended);
}

#[tokio::test]
async fn human_queue_assigns_and_accepts_through_handle() {
    let queue_id = QueueId::from("support");
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    handle
        .enqueue_call(
            call_id.clone(),
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await
        .unwrap();

    let assignment = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("assignment");
    assert_eq!(assignment.agent_id, human.id);
    handle.accept_offer(&assignment.offer_id).await.unwrap();

    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(call.status, CallStatus::Connected);
    assert_eq!(call.assigned_agent_id, Some(assignment.agent_id));
}

#[tokio::test]
async fn human_offer_connects_outbound_agent_leg() {
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_session_config(local_session_config("orchestrator"))
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    let offer_id = handle
        .offer_agent(call_id.clone(), human.id.clone())
        .await
        .unwrap();
    let agent_leg_id = handle.connect_agent_offer(&offer_id).await.unwrap();

    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    let offers = handle.list_offers_for_call(&call_id).await.unwrap();
    let offer = offers.iter().find(|offer| offer.id == offer_id).unwrap();
    let agent_leg = call
        .legs
        .iter()
        .find(|leg| leg.id == agent_leg_id)
        .expect("agent leg");

    assert_eq!(call.status, CallStatus::ConnectingAgent);
    assert_eq!(offer.status, AgentOfferStatus::Pending);
    assert_eq!(offer.agent_leg_id, Some(agent_leg_id));
    assert_eq!(agent_leg.role, CallLegRole::HumanAgent);
    assert_eq!(agent_leg.status, CallLegStatus::Dialing);
    assert_eq!(agent_leg.agent_id, Some(human.id));

    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(std::time::Duration::from_secs(0)))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn registrar_contact_resolver_uses_best_live_registered_contact() {
    let registrar = std::sync::Arc::new(RegistrarService::new().await.unwrap());
    let alice_aor = AddressOfRecord::parse("sip:alice@example.com").unwrap();
    registrar
        .register_aor(
            &alice_aor,
            registered_contact("sip:alice@127.0.0.1:5071", 0.2),
            Some(300),
        )
        .await
        .unwrap();
    registrar
        .register_aor(
            &alice_aor,
            registered_contact("sip:alice@127.0.0.1:5081", 0.9),
            Some(300),
        )
        .await
        .unwrap();

    let resolver = RegistrarContactResolver::new(registrar);
    let mut agent = Agent::human("alice", "sip:placeholder@example.com");
    agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };

    let contact = resolver.resolve_contact(&agent).await.unwrap();
    assert_eq!(contact.uri, "sip:alice@127.0.0.1:5081");
    assert_eq!(contact.source, ContactSource::Registrar);
    assert!(contact.expires_at.is_some());
}

#[tokio::test]
async fn registrar_contact_resolver_preserves_aor_domain_and_metadata() {
    let registrar = std::sync::Arc::new(RegistrarService::new().await.unwrap());
    let alice_example = AddressOfRecord::parse("sip:alice@example.com").unwrap();
    let alice_other = AddressOfRecord::parse("sip:alice@other.example").unwrap();
    let mut example_contact = registered_contact("sip:alice@127.0.0.1:5071", 1.0);
    example_contact.transport = Transport::TCP;
    example_contact.received = Some("203.0.113.10:62000".to_string());
    example_contact.path = vec!["<sip:edge.example.com;lr>".to_string()];
    example_contact.instance_id = "urn:uuid:alice-example".to_string();
    example_contact.reg_id = Some(1);
    example_contact.flow_id = Some("flow-example".to_string());

    registrar
        .register_aor(&alice_example, example_contact, Some(300))
        .await
        .unwrap();
    registrar
        .register_aor(
            &alice_other,
            registered_contact("sip:alice@127.0.0.1:5081", 1.0),
            Some(300),
        )
        .await
        .unwrap();

    let resolver = RegistrarContactResolver::new(registrar);
    let mut example_agent = Agent::human("alice-example", "sip:placeholder@example.com");
    example_agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };
    let mut other_agent = Agent::human("alice-other", "sip:placeholder@example.com");
    other_agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@other.example".to_string(),
    };

    let example = resolver.resolve_contact(&example_agent).await.unwrap();
    assert_eq!(example.uri, "sip:alice@127.0.0.1:5071");
    assert_eq!(example.transport, Some(Transport::TCP));
    assert_eq!(example.received.as_deref(), Some("203.0.113.10:62000"));
    assert_eq!(example.path, vec!["<sip:edge.example.com;lr>".to_string()]);
    assert_eq!(
        example.instance_id.as_deref(),
        Some("urn:uuid:alice-example")
    );
    assert_eq!(example.reg_id, Some(1));
    assert_eq!(example.flow_id.as_deref(), Some("flow-example"));

    let other = resolver.resolve_contact(&other_agent).await.unwrap();
    assert_eq!(other.uri, "sip:alice@127.0.0.1:5081");
}

#[tokio::test]
async fn registrar_contact_resolver_skips_unreachable_contacts() {
    let registrar = std::sync::Arc::new(RegistrarService::new().await.unwrap());
    let alice = AddressOfRecord::parse("sip:alice@example.com").unwrap();
    registrar
        .register_aor(
            &alice,
            registered_contact("sip:alice@127.0.0.1:5071", 1.0),
            Some(300),
        )
        .await
        .unwrap();
    registrar
        .register_aor(
            &alice,
            registered_contact("sip:alice@127.0.0.1:5081", 0.1),
            Some(300),
        )
        .await
        .unwrap();
    registrar
        .set_contact_reachability(
            &alice,
            "sip:alice@127.0.0.1:5071",
            ContactReachability::Unreachable,
        )
        .await
        .unwrap();

    let resolver = RegistrarContactResolver::new(registrar);
    let mut agent = Agent::human("alice", "sip:placeholder@example.com");
    agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };

    let contact = resolver.resolve_contact(&agent).await.unwrap();
    assert_eq!(contact.uri, "sip:alice@127.0.0.1:5081");
}

#[tokio::test]
async fn registrar_contact_resolver_fails_unregistered_agent() {
    let registrar = std::sync::Arc::new(RegistrarService::new().await.unwrap());
    let resolver = RegistrarContactResolver::new(registrar);
    let mut agent = Agent::human("alice", "sip:placeholder@example.com");
    agent.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };

    let error = resolver.resolve_contact(&agent).await.unwrap_err();
    assert!(matches!(
        error,
        OrchestrationError::ContactResolutionFailed(_, ref reason)
            if reason.contains("failed to resolve registered SIP user")
    ));
}

#[tokio::test]
async fn failed_queue_agent_connection_requeues_and_excludes_failed_agent() {
    let queue_id = QueueId::from("support");
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    handle
        .enqueue_call(
            call_id.clone(),
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await
        .unwrap();

    let assignment = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("assignment");
    handle
        .fail_agent_connection(&assignment.offer_id, AgentOfferStatus::Failed, "busy")
        .await
        .unwrap();

    let stats = handle.get_queue_stats(&queue_id).await.unwrap();
    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(stats.queued_calls, 1);
    assert_eq!(call.status, CallStatus::Queued);
    assert_eq!(call.assigned_agent_id, None);
    assert!(handle.assign_next_call(&queue_id).await.unwrap().is_none());
}

#[tokio::test]
async fn failed_queue_agent_connection_immediately_retries_next_eligible_agent() {
    let queue_id = QueueId::from("support");
    let alice = available_human("alice");
    let bob = available_human("bob");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(alice)
        .with_agent(bob)
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    handle
        .enqueue_call(
            call_id.clone(),
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await
        .unwrap();

    let first = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("first assignment");
    let second = handle
        .fail_agent_connection_and_retry_next(&first.offer_id, AgentOfferStatus::Failed, "busy")
        .await
        .unwrap()
        .expect("second assignment");

    assert_ne!(first.agent_id, second.agent_id);
    assert_eq!(second.call_id, call_id);
    assert_eq!(
        handle
            .get_queue_stats(&queue_id)
            .await
            .unwrap()
            .queued_calls,
        0
    );

    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(call.status, CallStatus::OfferingAgent);
    assert_eq!(call.assigned_agent_id, Some(second.agent_id.clone()));

    let offers = handle.list_offers_for_call(&call_id).await.unwrap();
    let first_offer = offers
        .iter()
        .find(|offer| offer.id == first.offer_id)
        .expect("first offer");
    let second_offer = offers
        .iter()
        .find(|offer| offer.id == second.offer_id)
        .expect("second offer");
    assert_eq!(first_offer.status, AgentOfferStatus::Failed);
    assert_eq!(second_offer.status, AgentOfferStatus::Reserved);
}

#[tokio::test]
async fn bridge_agent_offer_requires_caller_leg() {
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_session_config(local_session_config("orchestrator"))
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    let offer_id = handle.offer_agent(call_id, human.id).await.unwrap();
    handle.connect_agent_offer(&offer_id).await.unwrap();

    let error = handle.bridge_agent_offer(&offer_id).await.unwrap_err();
    assert!(error.to_string().contains("no caller leg"));

    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(std::time::Duration::from_secs(0)))
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_inbound_human_agent_bridge_and_caller_teardown() {
    let caller_config = local_session_config("caller");
    let orchestrator_config = local_session_config("support");
    let agent_config = local_session_config("alice");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let agent = UnifiedCoordinator::new(agent_config.clone()).await.unwrap();

    let agent_id = AgentId::from("alice");
    let agent_uri = format!("sip:alice@127.0.0.1:{}", agent_config.sip_port);
    let mut human = Agent::human(agent_id.clone(), agent_uri);
    human.state = AgentState::Available;
    human.skills.push(Skill::from("support"));

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(OrchestrationConfig {
                session: orchestrator_config.clone(),
                ..OrchestrationConfig::default()
            })
            .with_session_config(orchestrator_config.clone())
            .with_agent(human.clone())
            .with_router(StaticRouter::new(RouteDecision::OfferAgent {
                agent_id: agent_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let (call_id, offer_id) = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::AgentReserved {
                    call_id, offer_id, ..
                } => return (call_id, offer_id),
                _ => {}
            }
        }
    })
    .await
    .expect("agent reserved");

    let agent_leg_id = handle.connect_agent_offer(&offer_id).await.unwrap();
    let outcome_task = {
        let handle = handle.clone();
        let offer_id = offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let agent_incoming = timeout(Duration::from_secs(10), agent.get_incoming_call())
        .await
        .expect("agent incoming call")
        .expect("agent incoming call info");
    agent.accept_call(&agent_incoming.session_id).await.unwrap();

    let bridge_id = outcome_task.await.unwrap().unwrap().expect("bridge id");

    let connected = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(connected.status, CallStatus::Connected);
    assert_eq!(connected.active_bridge_id, Some(bridge_id.clone()));
    assert_eq!(handle.active_bridge_count().await, 1);
    assert!(connected.legs.iter().any(|leg| {
        leg.id == agent_leg_id
            && leg.role == CallLegRole::HumanAgent
            && leg.status == CallLegStatus::Bridged
    }));

    let mut teardown_rx = orchestration_events.subscribe();
    caller.hangup(&caller_session_id).await.unwrap();
    let bridge_ended = timeout(Duration::from_secs(10), async {
        loop {
            let event = teardown_rx.recv().await.unwrap().event;
            if matches!(event, OrchestrationEvent::BridgeEnded { bridge_id: ref ended, .. } if ended == &bridge_id)
            {
                return event;
            }
        }
    })
    .await
    .expect("bridge ended");
    assert!(matches!(
        bridge_ended,
        OrchestrationEvent::BridgeEnded { .. }
    ));

    timeout(Duration::from_secs(5), async {
        loop {
            let call = handle.get_call(&call_id).await.unwrap().unwrap();
            let agent = handle.get_agent(&agent_id).await.unwrap().unwrap();
            if call.status == CallStatus::Ended
                && call.active_bridge_id.is_none()
                && agent.state == AgentState::WrapUp
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("teardown state");

    assert_eq!(handle.active_bridge_count().await, 0);
    handle
        .complete_wrap_up(agent_id.clone(), call_id.clone())
        .await
        .unwrap();
    assert_eq!(
        handle.get_agent(&agent_id).await.unwrap().unwrap().state,
        AgentState::Available
    );

    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_registered_sip_agent_resolves_contact_and_bridges() {
    let caller_config = local_session_config("caller-registered");
    let orchestrator_config = local_session_config("support-registered");
    let agent_config = local_session_config("alice-registered");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let agent = UnifiedCoordinator::new(agent_config.clone()).await.unwrap();
    let registrar = std::sync::Arc::new(RegistrarService::new().await.unwrap());
    let registered_uri = format!("sip:alice@127.0.0.1:{}", agent_config.sip_port);
    let alice_aor = AddressOfRecord::parse("sip:alice@example.com").unwrap();
    registrar
        .register_aor(
            &alice_aor,
            registered_contact(registered_uri.clone(), 1.0),
            Some(300),
        )
        .await
        .unwrap();

    let agent_id = AgentId::from("alice");
    let mut human = Agent::human(agent_id.clone(), "sip:placeholder@example.com");
    human.connector = AgentConnector::RegisteredSipUser {
        aor: "sip:alice@example.com".to_string(),
    };
    human.state = AgentState::Available;
    human.skills.push(Skill::from("support"));

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(OrchestrationConfig {
                session: orchestrator_config.clone(),
                ..OrchestrationConfig::default()
            })
            .with_session_config(orchestrator_config.clone())
            .with_contact_resolver(RegistrarContactResolver::new(registrar))
            .with_agent(human)
            .with_router(StaticRouter::new(RouteDecision::OfferAgent {
                agent_id: agent_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let (call_id, offer_id) = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::AgentReserved {
                    call_id, offer_id, ..
                } => return (call_id, offer_id),
                _ => {}
            }
        }
    })
    .await
    .expect("agent reserved");

    let agent_leg_id = handle.connect_agent_offer(&offer_id).await.unwrap();
    let outcome_task = {
        let handle = handle.clone();
        let offer_id = offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let agent_incoming = timeout(Duration::from_secs(10), agent.get_incoming_call())
        .await
        .expect("agent incoming call")
        .expect("agent incoming call info");
    agent.accept_call(&agent_incoming.session_id).await.unwrap();

    let bridge_id = outcome_task.await.unwrap().unwrap().expect("bridge id");
    let connected = handle.get_call(&call_id).await.unwrap().unwrap();
    let agent_leg = connected
        .legs
        .iter()
        .find(|leg| leg.id == agent_leg_id)
        .expect("agent leg");
    assert_eq!(connected.status, CallStatus::Connected);
    assert_eq!(connected.active_bridge_id, Some(bridge_id.clone()));
    assert_eq!(agent_leg.uri, registered_uri);
    assert_eq!(agent_leg.status, CallLegStatus::Bridged);

    let mut teardown_rx = orchestration_events.subscribe();
    caller.hangup(&caller_session_id).await.unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            let event = teardown_rx.recv().await.unwrap().event;
            if matches!(event, OrchestrationEvent::BridgeEnded { bridge_id: ref ended, .. } if ended == &bridge_id)
            {
                break;
            }
        }
    })
    .await
    .expect("bridge ended");

    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_agent_hangup_tears_down_caller_bridge_and_wraps_up() {
    let caller_config = local_session_config("caller-agent-teardown");
    let orchestrator_config = local_session_config("support-agent-teardown");
    let agent_config = local_session_config("alice-agent-teardown");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let agent = UnifiedCoordinator::new(agent_config.clone()).await.unwrap();

    let agent_id = AgentId::from("alice");
    let agent_uri = format!("sip:alice@127.0.0.1:{}", agent_config.sip_port);
    let mut human = Agent::human(agent_id.clone(), agent_uri);
    human.state = AgentState::Available;
    human.skills.push(Skill::from("support"));

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(OrchestrationConfig {
                session: orchestrator_config.clone(),
                ..OrchestrationConfig::default()
            })
            .with_session_config(orchestrator_config.clone())
            .with_agent(human)
            .with_router(StaticRouter::new(RouteDecision::OfferAgent {
                agent_id: agent_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let (call_id, offer_id) = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::AgentReserved {
                    call_id, offer_id, ..
                } => return (call_id, offer_id),
                _ => {}
            }
        }
    })
    .await
    .expect("agent reserved");

    let _agent_leg_id = handle.connect_agent_offer(&offer_id).await.unwrap();
    let outcome_task = {
        let handle = handle.clone();
        let offer_id = offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let agent_incoming = timeout(Duration::from_secs(10), agent.get_incoming_call())
        .await
        .expect("agent incoming call")
        .expect("agent incoming call info");
    agent.accept_call(&agent_incoming.session_id).await.unwrap();

    let bridge_id = outcome_task.await.unwrap().unwrap().expect("bridge id");
    assert_eq!(handle.active_bridge_count().await, 1);

    let mut teardown_rx = orchestration_events.subscribe();
    agent.hangup(&agent_incoming.session_id).await.unwrap();
    let bridge_ended = timeout(Duration::from_secs(10), async {
        loop {
            let event = teardown_rx.recv().await.unwrap().event;
            if matches!(event, OrchestrationEvent::BridgeEnded { bridge_id: ref ended, .. } if ended == &bridge_id)
            {
                return event;
            }
        }
    })
    .await
    .expect("bridge ended");
    assert!(matches!(
        bridge_ended,
        OrchestrationEvent::BridgeEnded { reason, .. } if reason.contains("agent leg ended")
    ));

    timeout(Duration::from_secs(5), async {
        loop {
            let call = handle.get_call(&call_id).await.unwrap().unwrap();
            let agent = handle.get_agent(&agent_id).await.unwrap().unwrap();
            let caller_leg_ended = call
                .legs
                .iter()
                .any(|leg| leg.role == CallLegRole::Caller && leg.status == CallLegStatus::Ended);
            let agent_leg_ended = call.legs.iter().any(|leg| {
                leg.role == CallLegRole::HumanAgent && leg.status == CallLegStatus::Ended
            });
            if call.status == CallStatus::Ended
                && call.active_bridge_id.is_none()
                && caller_leg_ended
                && agent_leg_ended
                && agent.state == AgentState::WrapUp
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("teardown state");

    timeout(Duration::from_secs(5), async {
        loop {
            match caller.get_state(&caller_session_id).await {
                Ok(
                    CallState::Terminating
                    | CallState::Terminated
                    | CallState::Cancelled
                    | CallState::Failed(_),
                ) => break,
                Err(_) => break,
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("caller session ended");

    assert_eq!(handle.active_bridge_count().await, 0);
    handle
        .complete_wrap_up(agent_id.clone(), call_id.clone())
        .await
        .unwrap();
    assert_eq!(
        handle.get_agent(&agent_id).await.unwrap().unwrap().state,
        AgentState::Available
    );

    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_failed_agent_offer_retries_and_bridges_second_agent() {
    let caller_config = local_session_config("caller-retry");
    let orchestrator_config = local_session_config("support-retry");
    let first_agent_config = local_session_config("alice-retry");
    let second_agent_config = local_session_config("bob-retry");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let first_agent = UnifiedCoordinator::new(first_agent_config.clone())
        .await
        .unwrap();
    let second_agent = UnifiedCoordinator::new(second_agent_config.clone())
        .await
        .unwrap();

    let queue_id = QueueId::from("support");
    let first_agent_id = AgentId::from("alice");
    let second_agent_id = AgentId::from("bob");

    let mut first_human = Agent::human(
        first_agent_id.clone(),
        format!("sip:alice@127.0.0.1:{}", first_agent_config.sip_port),
    );
    first_human.state = AgentState::Available;
    first_human.skills.push(Skill::from("support"));

    let mut second_human = Agent::human(
        second_agent_id.clone(),
        format!("sip:bob@127.0.0.1:{}", second_agent_config.sip_port),
    );
    second_human.state = AgentState::Offline;
    second_human.skills.push(Skill::from("support"));

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(OrchestrationConfig {
                session: orchestrator_config.clone(),
                ..OrchestrationConfig::default()
            })
            .with_session_config(orchestrator_config.clone())
            .with_queue(Queue::new(queue_id.clone(), "Support"))
            .with_agent(first_human)
            .with_agent(second_human)
            .with_router(StaticRouter::new(RouteDecision::Queue {
                queue_id: queue_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let call_id = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::CallQueued {
                    call_id,
                    queue_id: queued,
                } if queued == queue_id => return call_id,
                _ => {}
            }
        }
    })
    .await
    .expect("call queued");

    let first_assignment = handle
        .assign_and_connect_next_call(&queue_id)
        .await
        .unwrap()
        .expect("first assignment");
    assert_eq!(first_assignment.agent_id, first_agent_id);

    handle
        .update_agent_state(second_agent_id.clone(), AgentState::Available)
        .await
        .unwrap();

    let outcome_task = {
        let handle = handle.clone();
        let offer_id = first_assignment.offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let first_incoming = timeout(Duration::from_secs(10), first_agent.get_incoming_call())
        .await
        .expect("first agent incoming call")
        .expect("first agent incoming call info");
    first_agent
        .reject_call(&first_incoming.session_id, 486, "Busy Here")
        .await
        .unwrap();

    let second_incoming = timeout(Duration::from_secs(10), second_agent.get_incoming_call())
        .await
        .expect("second agent incoming call")
        .expect("second agent incoming call info");
    second_agent
        .accept_call(&second_incoming.session_id)
        .await
        .unwrap();

    let bridge_id = outcome_task.await.unwrap().unwrap().expect("bridge id");

    let connected = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(connected.status, CallStatus::Connected);
    assert_eq!(connected.assigned_agent_id, Some(second_agent_id.clone()));
    assert_eq!(connected.active_bridge_id, Some(bridge_id.clone()));
    assert_eq!(handle.active_bridge_count().await, 1);

    let offers = handle.list_offers_for_call(&call_id).await.unwrap();
    let first_offer = offers
        .iter()
        .find(|offer| offer.id == first_assignment.offer_id)
        .expect("first offer");
    let accepted_offer = offers
        .iter()
        .find(|offer| offer.agent_id == second_agent_id)
        .expect("second offer");
    assert_eq!(first_offer.status, AgentOfferStatus::Failed);
    assert_eq!(accepted_offer.status, AgentOfferStatus::Accepted);

    let mut teardown_rx = orchestration_events.subscribe();
    caller.hangup(&caller_session_id).await.unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            let event = teardown_rx.recv().await.unwrap().event;
            if matches!(event, OrchestrationEvent::BridgeEnded { bridge_id: ref ended, .. } if ended == &bridge_id)
            {
                break;
            }
        }
    })
    .await
    .expect("bridge ended");

    timeout(Duration::from_secs(5), async {
        loop {
            let call = handle.get_call(&call_id).await.unwrap().unwrap();
            let agent = handle.get_agent(&second_agent_id).await.unwrap().unwrap();
            if call.status == CallStatus::Ended
                && call.active_bridge_id.is_none()
                && agent.state == AgentState::WrapUp
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("teardown state");
    assert_eq!(handle.active_bridge_count().await, 0);

    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    first_agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    second_agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_no_answer_offer_times_out_and_bridges_second_agent() {
    let caller_config = local_session_config("caller-timeout");
    let orchestrator_config = local_session_config("support-timeout");
    let first_agent_config = local_session_config("alice-timeout");
    let second_agent_config = local_session_config("bob-timeout");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let first_agent = UnifiedCoordinator::new(first_agent_config.clone())
        .await
        .unwrap();
    let second_agent = UnifiedCoordinator::new(second_agent_config.clone())
        .await
        .unwrap();

    let queue_id = QueueId::from("support");
    let first_agent_id = AgentId::from("alice");
    let second_agent_id = AgentId::from("bob");

    let mut first_human = Agent::human(
        first_agent_id.clone(),
        format!("sip:alice@127.0.0.1:{}", first_agent_config.sip_port),
    );
    first_human.state = AgentState::Available;
    first_human.skills.push(Skill::from("support"));

    let mut second_human = Agent::human(
        second_agent_id.clone(),
        format!("sip:bob@127.0.0.1:{}", second_agent_config.sip_port),
    );
    second_human.state = AgentState::Offline;
    second_human.skills.push(Skill::from("support"));

    let mut orchestration_config = OrchestrationConfig {
        session: orchestrator_config.clone(),
        ..OrchestrationConfig::default()
    };
    orchestration_config.assignment.outbound_answer_timeout = Duration::from_millis(700);

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(orchestration_config)
            .with_session_config(orchestrator_config.clone())
            .with_queue(Queue::new(queue_id.clone(), "Support"))
            .with_agent(first_human)
            .with_agent(second_human)
            .with_router(StaticRouter::new(RouteDecision::Queue {
                queue_id: queue_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let call_id = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::CallQueued {
                    call_id,
                    queue_id: queued,
                } if queued == queue_id => return call_id,
                _ => {}
            }
        }
    })
    .await
    .expect("call queued");

    let first_assignment = handle
        .assign_and_connect_next_call(&queue_id)
        .await
        .unwrap()
        .expect("first assignment");
    assert_eq!(first_assignment.agent_id, first_agent_id);

    let _first_incoming = timeout(Duration::from_secs(10), first_agent.get_incoming_call())
        .await
        .expect("first agent incoming call")
        .expect("first agent incoming call info");

    handle
        .update_agent_state(second_agent_id.clone(), AgentState::Available)
        .await
        .unwrap();

    let outcome_task = {
        let handle = handle.clone();
        let offer_id = first_assignment.offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let second_incoming = timeout(Duration::from_secs(10), second_agent.get_incoming_call())
        .await
        .expect("second agent incoming call")
        .expect("second agent incoming call info");
    second_agent
        .accept_call(&second_incoming.session_id)
        .await
        .unwrap();

    let bridge_id = outcome_task.await.unwrap().unwrap().expect("bridge id");
    let connected = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(connected.status, CallStatus::Connected);
    assert_eq!(connected.assigned_agent_id, Some(second_agent_id.clone()));
    assert_eq!(connected.active_bridge_id, Some(bridge_id.clone()));

    let offers = handle.list_offers_for_call(&call_id).await.unwrap();
    let first_offer = offers
        .iter()
        .find(|offer| offer.id == first_assignment.offer_id)
        .expect("first offer");
    let accepted_offer = offers
        .iter()
        .find(|offer| offer.agent_id == second_agent_id)
        .expect("second offer");
    assert_eq!(first_offer.status, AgentOfferStatus::TimedOut);
    assert_eq!(accepted_offer.status, AgentOfferStatus::Accepted);

    let mut teardown_rx = orchestration_events.subscribe();
    caller.hangup(&caller_session_id).await.unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            let event = teardown_rx.recv().await.unwrap().event;
            if matches!(event, OrchestrationEvent::BridgeEnded { bridge_id: ref ended, .. } if ended == &bridge_id)
            {
                break;
            }
        }
    })
    .await
    .expect("bridge ended");

    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    first_agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    second_agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_connection_failure_cleans_offer_and_requeues_call() {
    let caller_config = local_session_config("caller-cleanup");
    let orchestrator_config = local_session_config("support-cleanup");
    let first_agent_config = local_session_config("alice-cleanup");

    let caller = UnifiedCoordinator::new(caller_config.clone())
        .await
        .unwrap();
    let first_agent = UnifiedCoordinator::new(first_agent_config.clone())
        .await
        .unwrap();

    let queue_id = QueueId::from("support");
    let first_agent_id = AgentId::from("alice");
    let ai_agent_id = AgentId::from("support-ai");

    let mut first_human = Agent::human(
        first_agent_id.clone(),
        format!("sip:alice@127.0.0.1:{}", first_agent_config.sip_port),
    );
    first_human.state = AgentState::Available;
    first_human.skills.push(Skill::from("support"));

    let mut ai_agent = available_ai("support-ai");
    ai_agent.state = AgentState::Offline;

    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder()
            .with_config(OrchestrationConfig {
                session: orchestrator_config.clone(),
                ..OrchestrationConfig::default()
            })
            .with_session_config(orchestrator_config.clone())
            .with_queue(Queue::new(queue_id.clone(), "Support"))
            .with_agent(first_human)
            .with_agent(ai_agent)
            .with_router(StaticRouter::new(RouteDecision::Queue {
                queue_id: queue_id.clone(),
            }))
            .build()
            .await
            .unwrap(),
    );
    let handle = orchestrator.handle();
    let orchestration_events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut orch_rx = orchestration_events.subscribe();
    let caller_session_id = caller
        .make_call(
            &caller_config.local_uri,
            &format!("sip:support@127.0.0.1:{}", orchestrator_config.sip_port),
        )
        .await
        .unwrap();

    let call_id = timeout(Duration::from_secs(10), async {
        loop {
            match orch_rx.recv().await.unwrap().event {
                OrchestrationEvent::CallQueued {
                    call_id,
                    queue_id: queued,
                } if queued == queue_id => return call_id,
                _ => {}
            }
        }
    })
    .await
    .expect("call queued");

    let first_assignment = handle
        .assign_and_connect_next_call(&queue_id)
        .await
        .unwrap()
        .expect("first assignment");
    assert_eq!(first_assignment.agent_id, first_agent_id);

    handle
        .update_agent_state(ai_agent_id.clone(), AgentState::Available)
        .await
        .unwrap();

    let outcome_task = {
        let handle = handle.clone();
        let offer_id = first_assignment.offer_id.clone();
        tokio::spawn(async move { handle.wait_for_agent_offer_outcome(&offer_id).await })
    };

    let first_incoming = timeout(Duration::from_secs(10), first_agent.get_incoming_call())
        .await
        .expect("first agent incoming call")
        .expect("first agent incoming call info");
    first_agent
        .reject_call(&first_incoming.session_id, 486, "Busy Here")
        .await
        .unwrap();

    assert!(outcome_task.await.unwrap().unwrap().is_none());

    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    let stats = handle.get_queue_stats(&queue_id).await.unwrap();
    let offers = handle.list_offers_for_call(&call_id).await.unwrap();
    let ai_offer = offers
        .iter()
        .find(|offer| offer.agent_id == ai_agent_id)
        .expect("AI retry offer");

    assert_eq!(call.status, CallStatus::Queued);
    assert_eq!(call.assigned_agent_id, None);
    assert_eq!(stats.queued_calls, 1);
    assert_eq!(ai_offer.status, AgentOfferStatus::Failed);
    assert!(matches!(
        ai_offer.failure_reason.as_deref(),
        Some(reason) if reason.contains("does not create an outbound SIP leg")
    ));
    assert_eq!(
        handle.get_agent(&ai_agent_id).await.unwrap().unwrap().state,
        AgentState::Available
    );

    caller.hangup(&caller_session_id).await.unwrap();
    caller
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    first_agent
        .shutdown_gracefully(Some(Duration::from_secs(0)))
        .await
        .unwrap();
    if let Some(coordinator) = handle.coordinator().cloned() {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(0)))
            .await
            .unwrap();
    }
    run_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn ai_only_queue_uses_voice_ai_agent_as_normal_agent() {
    let queue_id = QueueId::from("support");
    let ai = available_ai("support-ai");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(ai.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();
    handle
        .enqueue_call(
            call_id,
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await
        .unwrap();

    let assignment = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("AI assignment");
    assert_eq!(assignment.agent_id, ai.id);
}

#[tokio::test]
async fn mixed_queue_falls_back_from_ai_to_human() {
    let queue_id = QueueId::from("support");
    let mut queue = Queue::new(queue_id.clone(), "Support");
    queue.policy = QueuePolicy::AiFirstThenHuman;
    let mut ai = available_ai("support-ai");
    ai.state = AgentState::Offline;
    let human = available_human("alice");

    let orchestrator = Orchestrator::builder()
        .with_queue(queue)
        .with_agent(ai)
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let call = support_call();
    handle.create_call(call.clone()).await.unwrap();
    handle
        .enqueue_call(
            call.id,
            QueueTarget {
                queue_id: queue_id.clone(),
                ..QueueTarget::default()
            },
        )
        .await
        .unwrap();

    let assignment = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("human fallback");
    assert_eq!(assignment.agent_id, human.id);
}

#[tokio::test]
async fn ai_handoff_to_human_preserves_call_context() {
    let queue_id = QueueId::from("support");
    let ai = available_ai("triage-ai");
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_agent(ai.clone())
        .with_agent(human.clone())
        .build()
        .await
        .unwrap();
    let handle = orchestrator.handle();

    let mut call = support_call();
    call.context.external_ref = Some("ticket-42".to_string());
    call.context
        .metadata
        .insert("intent".to_string(), "billing".to_string());
    let call_id = call.id.clone();
    handle.create_call(call).await.unwrap();

    let offer_id = handle
        .offer_agent(call_id.clone(), ai.id.clone())
        .await
        .unwrap();
    handle.accept_offer(&offer_id).await.unwrap();
    handle
        .apply_voice_ai_action(
            call_id.clone(),
            ai.id,
            VoiceAiAction::TransferToQueue {
                queue_id: queue_id.clone(),
            },
        )
        .await
        .unwrap();

    let assignment = handle
        .assign_next_call(&queue_id)
        .await
        .unwrap()
        .expect("human handoff");
    assert_eq!(assignment.agent_id, human.id);
    handle.accept_offer(&assignment.offer_id).await.unwrap();

    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    assert_eq!(call.context.external_ref.as_deref(), Some("ticket-42"));
    assert_eq!(
        call.context.metadata.get("intent"),
        Some(&"billing".to_string())
    );
    assert_eq!(call.assigned_agent_id, Some(human.id));
}

#[tokio::test]
async fn inbound_route_reject_creates_and_ends_call() {
    let orchestrator = Orchestrator::builder()
        .with_router(StaticRouter::new(RouteDecision::Reject {
            status: 486,
            reason: "Busy Here".to_string(),
        }))
        .build()
        .await
        .unwrap();

    let call_id = orchestrator
        .handle_incoming_call(incoming_call("sip:support@example.com"))
        .await
        .unwrap();
    let call = orchestrator
        .handle()
        .get_call(&call_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.status, CallStatus::Ended);
    assert!(matches!(
        call.disposition,
        Some(CallDisposition::Rejected { status: 486, .. })
    ));
    assert_eq!(call.legs.len(), 1);
    assert_eq!(call.legs[0].role, CallLegRole::Caller);
}

#[tokio::test]
async fn inbound_route_queue_enqueues_call() {
    let queue_id = QueueId::from("support");
    let orchestrator = Orchestrator::builder()
        .with_queue(Queue::new(queue_id.clone(), "Support"))
        .with_router(StaticRouter::new(RouteDecision::Queue {
            queue_id: queue_id.clone(),
        }))
        .build()
        .await
        .unwrap();

    let call_id = orchestrator
        .handle_incoming_call(incoming_call("sip:support@example.com"))
        .await
        .unwrap();
    let handle = orchestrator.handle();
    let call = handle.get_call(&call_id).await.unwrap().unwrap();
    let stats = handle.get_queue_stats(&queue_id).await.unwrap();
    assert_eq!(call.status, CallStatus::Queued);
    assert_eq!(stats.queued_calls, 1);
}

#[tokio::test]
async fn inbound_route_offer_agent_reserves_agent() {
    let human = available_human("alice");
    let orchestrator = Orchestrator::builder()
        .with_agent(human.clone())
        .with_router(StaticRouter::new(RouteDecision::OfferAgent {
            agent_id: human.id.clone(),
        }))
        .build()
        .await
        .unwrap();

    let call_id = orchestrator
        .handle_incoming_call(incoming_call("sip:support@example.com"))
        .await
        .unwrap();
    let call = orchestrator
        .handle()
        .get_call(&call_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.status, CallStatus::OfferingAgent);
    assert_eq!(call.assigned_agent_id, Some(human.id));
}

#[tokio::test]
async fn inbound_route_dial_sip_uri_is_explicitly_failed() {
    let orchestrator = Orchestrator::builder()
        .with_router(StaticRouter::new(RouteDecision::DialSipUri {
            uri: "sip:external@example.com".to_string(),
        }))
        .build()
        .await
        .unwrap();

    let call_id = orchestrator
        .handle_incoming_call(incoming_call("sip:support@example.com"))
        .await
        .unwrap();
    let call = orchestrator
        .handle()
        .get_call(&call_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.status, CallStatus::Failed);
    assert!(matches!(
        call.disposition,
        Some(CallDisposition::Failed { ref reason })
            if reason.contains("unsupported until outbound dialing is implemented")
    ));
}
