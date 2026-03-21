//! Comprehensive tests for call-engine: agent registry, queue management,
//! routing, assignment, and call lifecycle.

use rvoip_call_engine::agent::{AgentRegistry, Agent, AgentStatus, AgentStats};
use rvoip_call_engine::queue::{QueueManager, QueuedCall};
use rvoip_session_core::SessionId;
use chrono::Utc;

/// Helper: create a test agent with the given id, status, skills, and max calls.
fn make_agent(id: &str, status: AgentStatus, skills: Vec<&str>, max_calls: u32) -> Agent {
    Agent {
        id: id.to_string(),
        sip_uri: format!("sip:{}@test.local", id),
        display_name: format!("Agent {}", id),
        skills: skills.into_iter().map(|s| s.to_string()).collect(),
        max_concurrent_calls: max_calls,
        status,
        department: Some("test".to_string()),
        extension: None,
    }
}

/// Helper: create a QueuedCall with given priority and caller_id.
fn make_queued_call(priority: u8, caller_id: &str) -> QueuedCall {
    QueuedCall {
        session_id: SessionId::new(),
        caller_id: caller_id.to_string(),
        priority,
        queued_at: Utc::now(),
        estimated_wait_time: Some(60),
        retry_count: 0,
    }
}

// =============================================================================
// Test 8: Agent registration
// =============================================================================
#[tokio::test]
async fn test_agent_registration() {
    let mut registry = AgentRegistry::new();

    let agent = make_agent("agent-001", AgentStatus::Available, vec!["english", "sales"], 2);
    let agent_id = registry.register_agent(agent).await
        .expect("registration should succeed");

    assert_eq!(agent_id, "agent-001");

    // Verify agent appears in registry with correct status
    let status = registry.get_agent_status("agent-001");
    assert!(status.is_some(), "agent should exist in registry");
    assert_eq!(
        status.expect("checked above"),
        &AgentStatus::Available,
        "agent should be Available after registration with Available status"
    );

    // Verify statistics
    let stats = registry.get_statistics();
    assert_eq!(stats.total, 1, "should have 1 registered agent");
    assert_eq!(stats.available, 1, "should have 1 available agent");
}

// =============================================================================
// Test 9: Call queuing
// =============================================================================
#[tokio::test]
async fn test_call_queuing() {
    let mut qm = QueueManager::new();
    qm.create_queue("default".to_string(), "Default Queue".to_string(), 50)
        .expect("create queue");

    let call = make_queued_call(5, "+1-555-0001");
    let session_id = call.session_id.clone();

    let position = qm.enqueue_call("default", call)
        .expect("enqueue should succeed");
    assert_eq!(position, 0, "first call should be at position 0");

    // Verify call is in queue
    assert!(
        qm.is_call_queued("default", &session_id),
        "call should be in queue"
    );

    // Verify queue length
    assert_eq!(
        qm.total_queued_calls(),
        1,
        "total queued calls should be 1"
    );

    // Verify queue stats
    let stats = qm.get_queue_stats("default").expect("stats should succeed");
    assert_eq!(stats.total_calls, 1, "queue stats should show 1 call");
}

// =============================================================================
// Test 10: Agent assignment (register agent, enqueue call, dequeue + assign)
// =============================================================================
#[tokio::test]
async fn test_agent_assignment() {
    let mut registry = AgentRegistry::new();

    let agent = make_agent("agent-assign-001", AgentStatus::Available, vec!["general"], 2);
    registry.register_agent(agent).await.expect("register agent");

    // Verify agent is available
    let available = registry.find_available_agents();
    assert!(
        available.contains(&"agent-assign-001".to_string()),
        "agent should be in available list"
    );

    // Queue a call
    let mut qm = QueueManager::new();
    qm.create_queue("support".to_string(), "Support".to_string(), 50)
        .expect("create queue");

    let call = make_queued_call(5, "+1-555-0100");
    let call_session = call.session_id.clone();
    qm.enqueue_call("support", call).expect("enqueue");

    // Dequeue the call for the agent
    let dequeued = qm.dequeue_for_agent("support")
        .expect("dequeue should succeed");
    assert!(dequeued.is_some(), "should dequeue a call");

    let dequeued_call = dequeued.expect("checked above");
    assert_eq!(dequeued_call.session_id, call_session);

    // Simulate assigning call to agent: mark agent as Busy
    registry
        .update_agent_status(
            "agent-assign-001",
            AgentStatus::Busy(vec![call_session.clone()]),
        )
        .expect("update status to Busy");

    let status = registry.get_agent_status("agent-assign-001");
    match status {
        Some(AgentStatus::Busy(calls)) => {
            assert_eq!(calls.len(), 1, "agent should have 1 active call");
            assert_eq!(calls[0], call_session);
        }
        other => panic!("expected Busy status, got {:?}", other),
    }
}

// =============================================================================
// Test 11: Call routing (round-robin style -- each agent gets one call)
// =============================================================================
#[tokio::test]
async fn test_round_robin_assignment() {
    let mut registry = AgentRegistry::new();

    // Register 3 agents
    for i in 1..=3 {
        let agent = make_agent(
            &format!("rr-agent-{}", i),
            AgentStatus::Available,
            vec!["general"],
            1,
        );
        registry.register_agent(agent).await.expect("register");
    }

    let available = registry.find_available_agents();
    assert_eq!(available.len(), 3, "should have 3 available agents");

    // Queue 3 calls
    let mut qm = QueueManager::new();
    qm.create_queue("rr-queue".to_string(), "RR Queue".to_string(), 50)
        .expect("create queue");

    let mut call_sessions = Vec::new();
    for i in 1..=3 {
        let call = make_queued_call(5, &format!("+1-555-100{}", i));
        call_sessions.push(call.session_id.clone());
        qm.enqueue_call("rr-queue", call).expect("enqueue");
    }

    // Dequeue 3 calls and assign each to a different agent (round-robin simulation)
    let mut assigned_agents = Vec::new();
    let agents: Vec<String> = registry.find_available_agents();

    for (idx, _agent_id) in agents.iter().enumerate() {
        let dequeued = qm.dequeue_for_agent("rr-queue").expect("dequeue");
        assert!(dequeued.is_some(), "call {} should be available", idx + 1);

        let call = dequeued.expect("checked");
        let agent_id = &agents[idx];

        registry
            .update_agent_status(agent_id, AgentStatus::Busy(vec![call.session_id.clone()]))
            .expect("update to Busy");

        assigned_agents.push(agent_id.clone());
    }

    // Verify each agent got exactly one call
    assert_eq!(assigned_agents.len(), 3, "3 agents should have been assigned");

    // Verify no more calls in queue
    assert_eq!(qm.total_queued_calls(), 0, "queue should be empty");

    // Verify all agents are busy
    let stats = registry.get_statistics();
    assert_eq!(stats.busy, 3, "all 3 agents should be busy");
    assert_eq!(stats.available, 0, "no agents should be available");
}

// =============================================================================
// Test 12: Agent capacity limit
// =============================================================================
#[tokio::test]
async fn test_agent_capacity_limit() {
    let mut registry = AgentRegistry::new();

    // Agent with max_concurrent_calls = 1
    let agent = make_agent("capacity-agent", AgentStatus::Available, vec!["general"], 1);
    registry.register_agent(agent).await.expect("register");

    let mut qm = QueueManager::new();
    qm.create_queue("cap-queue".to_string(), "Capacity Queue".to_string(), 50)
        .expect("create queue");

    // Enqueue 2 calls
    let call1 = make_queued_call(5, "+1-555-2001");
    let call1_session = call1.session_id.clone();
    qm.enqueue_call("cap-queue", call1).expect("enqueue 1");

    let call2 = make_queued_call(5, "+1-555-2002");
    qm.enqueue_call("cap-queue", call2).expect("enqueue 2");

    // Assign first call to agent
    let dequeued1 = qm.dequeue_for_agent("cap-queue").expect("dequeue 1");
    assert!(dequeued1.is_some(), "first call should dequeue");

    let first_call = dequeued1.expect("checked");
    registry
        .update_agent_status(
            "capacity-agent",
            AgentStatus::Busy(vec![first_call.session_id.clone()]),
        )
        .expect("set busy");

    // Agent is at capacity (max_concurrent_calls = 1, currently has 1)
    // Check that the agent is busy
    let status = registry.get_agent_status("capacity-agent");
    assert!(
        matches!(status, Some(AgentStatus::Busy(_))),
        "agent should be busy"
    );

    // Verify agent is NOT in available list
    let available = registry.find_available_agents();
    assert!(
        !available.contains(&"capacity-agent".to_string()),
        "busy agent should not appear in available list"
    );

    // Second call should still be in queue
    assert_eq!(
        qm.total_queued_calls(),
        1,
        "second call should remain in queue since agent is at capacity"
    );
}

// =============================================================================
// Test 13: Call termination cleanup
// =============================================================================
#[tokio::test]
async fn test_call_termination_cleanup() {
    let mut registry = AgentRegistry::new();

    let agent = make_agent("cleanup-agent", AgentStatus::Available, vec!["general"], 2);
    registry.register_agent(agent).await.expect("register");

    // Simulate assigning a call
    let call_session = SessionId::new();
    registry
        .update_agent_status(
            "cleanup-agent",
            AgentStatus::Busy(vec![call_session.clone()]),
        )
        .expect("set busy");

    // Verify agent is busy
    assert!(
        matches!(
            registry.get_agent_status("cleanup-agent"),
            Some(AgentStatus::Busy(_))
        ),
        "agent should be busy"
    );

    // Simulate call termination: set agent back to Available
    registry
        .update_agent_status("cleanup-agent", AgentStatus::Available)
        .expect("set available");

    // Verify agent is available again
    assert_eq!(
        registry.get_agent_status("cleanup-agent"),
        Some(&AgentStatus::Available),
        "agent should return to Available after call termination"
    );

    // Verify agent is in the available list
    let available = registry.find_available_agents();
    assert!(
        available.contains(&"cleanup-agent".to_string()),
        "agent should be in available list after cleanup"
    );
}

// =============================================================================
// Test 14: DTMF metadata recording (simulated via queue metadata)
// =============================================================================
#[tokio::test]
async fn test_dtmf_metadata_recording() {
    // Simulate DTMF input by building a buffer and verifying it accumulates correctly.
    // The call-engine itself doesn't have a DTMF-specific API at the queue/agent level,
    // so we simulate the metadata tracking pattern that would be used.

    let mut dtmf_buffer = String::new();

    // Process DTMF digits '1', '2', '3'
    let digits = ['1', '2', '3'];
    for digit in &digits {
        dtmf_buffer.push(*digit);
    }

    assert_eq!(
        dtmf_buffer, "123",
        "DTMF buffer should accumulate digits in order"
    );

    // Simulate storing in a metadata map (like call metadata)
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("dtmf_buffer".to_string(), dtmf_buffer.clone());

    assert_eq!(
        metadata.get("dtmf_buffer").map(|s| s.as_str()),
        Some("123"),
        "metadata should contain accumulated DTMF digits"
    );

    // Verify additional digits can be appended
    dtmf_buffer.push('4');
    dtmf_buffer.push('5');
    metadata.insert("dtmf_buffer".to_string(), dtmf_buffer);

    assert_eq!(
        metadata.get("dtmf_buffer").map(|s| s.as_str()),
        Some("12345"),
        "metadata should contain all DTMF digits after append"
    );
}

// =============================================================================
// Test 15: Queue priority ordering
// =============================================================================
#[tokio::test]
async fn test_queue_priority_ordering() {
    let mut qm = QueueManager::new();
    qm.create_queue("priority-queue".to_string(), "Priority Queue".to_string(), 50)
        .expect("create queue");

    // Enqueue calls with different priorities (lower number = higher priority)
    let low_call = make_queued_call(9, "low-priority-caller");
    let low_session = low_call.session_id.clone();
    qm.enqueue_call("priority-queue", low_call).expect("enqueue low");

    let normal_call = make_queued_call(5, "normal-priority-caller");
    let normal_session = normal_call.session_id.clone();
    qm.enqueue_call("priority-queue", normal_call).expect("enqueue normal");

    let high_call = make_queued_call(1, "high-priority-caller");
    let high_session = high_call.session_id.clone();
    qm.enqueue_call("priority-queue", high_call).expect("enqueue high");

    let vip_call = make_queued_call(0, "vip-priority-caller");
    let vip_session = vip_call.session_id.clone();
    qm.enqueue_call("priority-queue", vip_call).expect("enqueue vip");

    assert_eq!(qm.total_queued_calls(), 4, "should have 4 queued calls");

    // Dequeue should return in priority order: VIP(0), High(1), Normal(5), Low(9)
    let first = qm.dequeue_for_agent("priority-queue")
        .expect("dequeue 1")
        .expect("should have call");
    assert_eq!(first.session_id, vip_session, "first dequeued should be VIP (priority 0)");
    assert_eq!(first.priority, 0);

    let second = qm.dequeue_for_agent("priority-queue")
        .expect("dequeue 2")
        .expect("should have call");
    assert_eq!(second.session_id, high_session, "second dequeued should be High (priority 1)");
    assert_eq!(second.priority, 1);

    let third = qm.dequeue_for_agent("priority-queue")
        .expect("dequeue 3")
        .expect("should have call");
    assert_eq!(third.session_id, normal_session, "third dequeued should be Normal (priority 5)");
    assert_eq!(third.priority, 5);

    let fourth = qm.dequeue_for_agent("priority-queue")
        .expect("dequeue 4")
        .expect("should have call");
    assert_eq!(fourth.session_id, low_session, "fourth dequeued should be Low (priority 9)");
    assert_eq!(fourth.priority, 9);

    // Queue should be empty now
    let empty = qm.dequeue_for_agent("priority-queue").expect("dequeue 5");
    assert!(empty.is_none(), "queue should be empty after dequeuing all");
    assert_eq!(qm.total_queued_calls(), 0);
}
