//! Gap #7: B2BUA bridge test.
//!
//! Tests the call-engine's ability to:
//! - Create a CallCenterEngine with an in-memory database
//! - Register agents in the AgentRegistry
//! - Manage call queues
//! - Track bridge/call stats

use std::time::Duration;

use anyhow::Result;
use serial_test::serial;
use tokio::time::timeout;

use rvoip_call_engine::prelude::*;
use rvoip_call_engine::agent::{AgentRegistry, Agent, AgentStatus};
use rvoip_call_engine::queue::{QueueManager, QueuedCall};
use rvoip_session_core::SessionId;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);

async fn make_engine() -> Result<std::sync::Arc<CallCenterEngine>> {
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:0".parse()?;
    config.general.local_media_addr = "127.0.0.1:0".parse()?;
    let engine = CallCenterEngine::new(config, Some(":memory:".to_string())).await?;
    Ok(engine)
}

fn make_agent(id: &str, skills: Vec<&str>) -> Agent {
    Agent {
        id: id.to_string(),
        sip_uri: format!("sip:{}@test.local", id),
        display_name: format!("Agent {}", id),
        skills: skills.into_iter().map(|s| s.to_string()).collect(),
        max_concurrent_calls: 2,
        status: AgentStatus::Available,
        department: Some("support".to_string()),
        extension: None,
    }
}

#[tokio::test]
#[serial]
async fn test_b2bua_engine_initial_state() -> Result<()> {
    timeout(TEST_TIMEOUT, async {
        let engine = make_engine().await?;
        let stats = engine.get_stats().await;

        assert_eq!(stats.active_calls, 0, "No active calls initially");
        assert_eq!(stats.active_bridges, 0, "No active bridges initially");
        assert_eq!(stats.queued_calls, 0, "No queued calls initially");

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out"))?
}

#[tokio::test]
#[serial]
async fn test_b2bua_agent_registry() -> Result<()> {
    timeout(TEST_TIMEOUT, async {
        let mut registry = AgentRegistry::new();

        let agent1 = make_agent("bridge-agent-1", vec!["sales"]);
        let agent2 = make_agent("bridge-agent-2", vec!["sales", "billing"]);

        registry.register_agent(agent1).await
            .map_err(|e| anyhow::anyhow!("Register 1: {e}"))?;
        registry.register_agent(agent2).await
            .map_err(|e| anyhow::anyhow!("Register 2: {e}"))?;

        let stats = registry.get_statistics();
        assert_eq!(stats.total, 2, "Should have 2 agents");
        assert_eq!(stats.available, 2, "Both agents should be available");

        // Set an agent to busy
        let fake_session = SessionId::new();
        registry.update_agent_status("bridge-agent-1", AgentStatus::Busy(vec![fake_session]))
            .map_err(|e| anyhow::anyhow!("Update status: {e}"))?;

        let status = registry.get_agent_status("bridge-agent-1");
        assert!(matches!(status, Some(AgentStatus::Busy(_))), "Agent should be busy");

        let stats = registry.get_statistics();
        assert_eq!(stats.available, 1, "Only one agent should be available");
        assert_eq!(stats.busy, 1, "One agent should be busy");

        // Find available agents
        let available = registry.find_available_agents();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0], "bridge-agent-2");

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out"))?
}

#[tokio::test]
#[serial]
async fn test_b2bua_queue_operations() -> Result<()> {
    timeout(TEST_TIMEOUT, async {
        let mut queue_mgr = QueueManager::new();

        queue_mgr.create_queue("default".to_string(), "Default Queue".to_string(), 50)
            .map_err(|e| anyhow::anyhow!("Create queue: {e}"))?;

        let queued = QueuedCall {
            session_id: SessionId::new(),
            caller_id: "customer-001".to_string(),
            priority: 5,
            queued_at: chrono::Utc::now(),
            estimated_wait_time: Some(30),
            retry_count: 0,
        };

        let position = queue_mgr.enqueue_call("default", queued)
            .map_err(|e| anyhow::anyhow!("Enqueue: {e}"))?;
        assert_eq!(position, 1, "First call should be at position 1");
        assert_eq!(queue_mgr.total_queued_calls(), 1, "Queue should have 1 call");

        let dequeued = queue_mgr.dequeue_for_agent("default")
            .map_err(|e| anyhow::anyhow!("Dequeue: {e}"))?;
        assert!(dequeued.is_some(), "Should dequeue the call");
        assert_eq!(queue_mgr.total_queued_calls(), 0, "Queue empty after dequeue");

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out"))?
}

#[tokio::test]
#[serial]
async fn test_b2bua_bridge_state_tracking() -> Result<()> {
    timeout(TEST_TIMEOUT, async {
        let engine = make_engine().await?;

        let stats = engine.get_stats().await;
        assert_eq!(stats.active_bridges, 0);

        let agent = make_agent("bridge-tracker-1", vec!["general"]);
        engine.register_agent(&agent).await
            .map_err(|e| anyhow::anyhow!("Register: {e}"))?;

        let stats = engine.get_stats().await;
        assert_eq!(stats.active_bridges, 0, "No bridge without a call");
        assert_eq!(stats.active_calls, 0, "No active call yet");

        let config = engine.config();
        assert!(config.general.max_concurrent_calls > 0);

        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Test timed out"))?
}
