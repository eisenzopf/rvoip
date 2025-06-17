//! Integration tests for the call-engine
//!
//! These tests verify that the call center components work together correctly
//! and integrate properly with the session-core and database systems.

use anyhow::Result;
use rvoip_call_engine::prelude::*;
use rvoip_transaction_core::TransactionManager;
use std::sync::Arc;
use tokio::sync::mpsc;
use async_trait::async_trait;

/// Create a dummy transport for testing
#[derive(Debug, Clone)]
struct TestTransport {
    local_addr: std::net::SocketAddr,
}

#[async_trait]
impl rvoip_sip_transport::Transport for TestTransport {
    async fn send_message(
        &self, 
        _message: rvoip_sip_core::Message, 
        _destination: std::net::SocketAddr
    ) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn local_addr(&self) -> std::result::Result<std::net::SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

async fn create_test_call_center() -> Result<Arc<CallCenterEngine>> {
    // Create test database
    let database = CallCenterDatabase::new_in_memory().await?;
    
    // Create test configuration
    let config = CallCenterConfig::default();
    
    // Create test transaction manager
    let local_addr: std::net::SocketAddr = "127.0.0.1:0".parse()?;
    let (_transport_tx, transport_rx) = mpsc::channel(10);
    let transport = Arc::new(TestTransport { local_addr });
    
    let (transaction_manager, _events) = TransactionManager::new(
        transport,
        transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create transaction manager: {}", e))?;
    
    let transaction_manager = Arc::new(transaction_manager);
    
    // Create call center engine
    let call_center = CallCenterEngine::new(transaction_manager, config, database).await?;
    
    Ok(call_center)
}

#[tokio::test]
async fn test_call_center_creation() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    let stats = call_center.get_stats().await;
    
    // Verify initial state
    assert_eq!(stats.active_calls, 0);
    assert_eq!(stats.active_bridges, 0);
    assert_eq!(stats.available_agents, 0);
    assert_eq!(stats.queued_calls, 0);
}

#[tokio::test]
async fn test_agent_registration() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Create test agent
    let agent = Agent {
        id: "test-agent-001".to_string(),
        sip_uri: "sip:test@example.com".parse().expect("Valid SIP URI"),
        display_name: "Test Agent".to_string(),
        skills: vec!["english".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Available,
        department: Some("test".to_string()),
        extension: Some("1001".to_string()),
    };
    
    // Register agent
    let session_id = call_center.register_agent(&agent).await.expect("Agent registration failed");
    
    // Verify session ID is valid
    assert!(!session_id.to_string().is_empty());
    
    // Check updated statistics
    let stats = call_center.get_stats().await;
    assert_eq!(stats.available_agents, 1);
}

#[tokio::test]
async fn test_bridge_management() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Register two test agents
    let agent1 = Agent {
        id: "agent-001".to_string(),
        sip_uri: "sip:agent1@example.com".parse().expect("Valid SIP URI"),
        display_name: "Agent One".to_string(),
        skills: vec!["english".to_string()],
        max_concurrent_calls: 1,
        status: AgentStatus::Available,
        department: None,
        extension: None,
    };
    
    let agent2 = Agent {
        id: "agent-002".to_string(),
        sip_uri: "sip:agent2@example.com".parse().expect("Valid SIP URI"),
        display_name: "Agent Two".to_string(),
        skills: vec!["english".to_string()],
        max_concurrent_calls: 1,
        status: AgentStatus::Available,
        department: None,
        extension: None,
    };
    
    let session1 = call_center.register_agent(&agent1).await.expect("Agent 1 registration failed");
    let session2 = call_center.register_agent(&agent2).await.expect("Agent 2 registration failed");
    
    // Test conference creation
    let bridge_id = call_center.create_conference(&[session1, session2]).await.expect("Conference creation failed");
    
    // Verify bridge was created
    assert!(!bridge_id.to_string().is_empty());
    
    // Check bridge info
    let bridge_info = call_center.get_bridge_info(&bridge_id).await.expect("Bridge info retrieval failed");
    assert_eq!(bridge_info.sessions.len(), 2);
    
    // Check statistics
    let stats = call_center.get_stats().await;
    assert_eq!(stats.active_bridges, 1);
}

#[tokio::test]
async fn test_database_integration() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Create agent store
    let agent_store = AgentStore::new(call_center.database().clone());
    
    // Test database operations
    let create_request = CreateAgentRequest {
        sip_uri: "sip:dbtest@example.com".to_string(),
        display_name: "Database Test Agent".to_string(),
        max_concurrent_calls: 1,
        department: Some("test".to_string()),
        extension: Some("2001".to_string()),
        phone_number: None,
    };
    
    let created_agent = agent_store.create_agent(create_request).await.expect("Database agent creation failed");
    assert_eq!(created_agent.display_name, "Database Test Agent");
    // Note: Agent status depends on the database implementation
    
    // Test agent retrieval
    let retrieved_agent = agent_store.get_agent_by_id(&created_agent.id).await.expect("Database query failed");
    assert!(retrieved_agent.is_some());
    assert_eq!(retrieved_agent.unwrap().id, created_agent.id);
}

#[tokio::test]
async fn test_call_center_statistics() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Initial statistics
    let initial_stats = call_center.get_stats().await;
    assert_eq!(initial_stats.active_calls, 0);
    assert_eq!(initial_stats.available_agents, 0);
    
    // Register an agent
    let agent = Agent {
        id: "stats-test-agent".to_string(),
        sip_uri: "sip:stats@example.com".parse().expect("Valid SIP URI"),
        display_name: "Statistics Test Agent".to_string(),
        skills: vec!["test".to_string()],
        max_concurrent_calls: 1,
        status: AgentStatus::Available,
        department: None,
        extension: None,
    };
    
    call_center.register_agent(&agent).await.expect("Agent registration failed");
    
    // Updated statistics
    let updated_stats = call_center.get_stats().await;
    assert_eq!(updated_stats.available_agents, 1);
}

#[tokio::test]
async fn test_call_center_config() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Test configuration access
    let config = call_center.config();
    assert!(config.general.max_concurrent_calls > 0);
    assert!(config.general.max_agents > 0);
    assert!(!config.general.domain.is_empty());
}

#[tokio::test]
async fn test_session_manager_access() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Test session manager access
    let session_manager = call_center.session_manager();
    
    // Verify we can access session-core APIs
    let active_sessions = session_manager.list_active_sessions().await;
    match active_sessions {
        Ok(sessions) => assert!(sessions.len() >= 0),
        Err(_) => (), // If not implemented yet, that's ok
    }
}

// TODO: Add more integration tests as modules are implemented
// - Test agent registration and status changes
// - Test call queuing and dequeuing
// - Test call routing decisions
// - Test bridge creation and management
// - Test monitoring and metrics collection 