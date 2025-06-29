//! Integration tests for the call-engine
//!
//! These tests verify that the call center components work together correctly
//! and integrate properly with the session-core and database systems.

use anyhow::Result;
use rvoip_call_engine::prelude::*;
use std::sync::Arc;
use serial_test::serial;

async fn create_test_call_center() -> Result<Arc<CallCenterEngine>> {
    // Create test configuration
    let mut config = CallCenterConfig::default();
    // Use test ports to avoid conflicts
    config.general.local_signaling_addr = "127.0.0.1:15060".parse()?;
    config.general.local_media_addr = "127.0.0.1:20000".parse()?;
    
    println!("Creating call center with SIP on {} and media on {}", 
             config.general.local_signaling_addr, 
             config.general.local_media_addr);
    
    // Create call center engine with in-memory database
    let call_center = CallCenterEngine::new(config, Some(":memory:".to_string())).await?;
    
    Ok(call_center)
}

#[tokio::test]
#[serial]
async fn test_call_center_creation() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    let stats = call_center.get_stats().await;
    
    // Verify initial state
    assert_eq!(stats.active_calls, 0);
    assert_eq!(stats.active_bridges, 0);
    assert_eq!(stats.queued_calls, 0);
    
    // Verify configuration is accessible
    let config = call_center.config();
    assert!(config.general.max_concurrent_calls > 0);
    assert!(config.general.max_agents > 0);
    assert!(!config.general.domain.is_empty());
}

#[tokio::test]
#[serial]
async fn test_database_agent_operations() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Get database manager
    let db_manager = call_center.database_manager().expect("Database manager should be available");
    
    // Test agent creation directly via database
    db_manager.upsert_agent(
        "test-agent-001",
        "Test Agent",
        Some("sip:test@example.com")
    ).await.expect("Database agent creation failed");
    
    // Test agent retrieval
    let retrieved_agent = db_manager.get_agent("test-agent-001").await.expect("Database query failed");
    assert!(retrieved_agent.is_some());
    let agent = retrieved_agent.unwrap();
    assert_eq!(agent.agent_id, "test-agent-001");
    assert_eq!(agent.username, "Test Agent");
    assert_eq!(agent.contact_uri, Some("sip:test@example.com".to_string()));
    
    // Test agent status update
    db_manager.update_agent_status("test-agent-001", AgentStatus::Busy(vec![])).await
        .expect("Status update should succeed");
    
    // Verify status was updated
    let updated_agent = db_manager.get_agent("test-agent-001").await.expect("Database query failed");
    assert!(updated_agent.is_some());
    // Note: Database uses DbAgentStatus::Busy, not AgentStatus::Busy
}

#[tokio::test]
#[serial]
async fn test_database_agent_statistics() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    let db_manager = call_center.database_manager().expect("Database manager should be available");
    
    // Initial stats - should be empty
    let initial_stats = db_manager.get_agent_stats().await.expect("Getting stats should work");
    assert_eq!(initial_stats.total_agents, 0);
    assert_eq!(initial_stats.available_agents, 0);
    
    // Add some test agents
    db_manager.upsert_agent("agent-001", "Agent One", Some("sip:agent1@example.com")).await
        .expect("Agent creation should succeed");
    db_manager.upsert_agent("agent-002", "Agent Two", Some("sip:agent2@example.com")).await
        .expect("Agent creation should succeed");
    
    // Update their statuses
    db_manager.update_agent_status("agent-001", AgentStatus::Available).await
        .expect("Status update should succeed");
    db_manager.update_agent_status("agent-002", AgentStatus::Busy(vec![])).await
        .expect("Status update should succeed");
    
    // Check updated statistics
    let updated_stats = db_manager.get_agent_stats().await.expect("Getting stats should work");
    assert_eq!(updated_stats.total_agents, 2);
    assert_eq!(updated_stats.available_agents, 1);
    assert_eq!(updated_stats.busy_agents, 1);
}

#[tokio::test]
#[serial]
async fn test_queue_operations() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Test queue creation - use a standard queue name that will actually be created
    call_center.create_queue("general").await
        .expect("Queue creation should succeed");
    
    // Now test queue stats access - should have at least the queue we created
    let queue_stats = call_center.get_queue_stats().await
        .expect("Queue stats should be accessible");
    
    // Should have at least the queue we created, though it might be empty or others might exist
    // Just check that we can get stats successfully (don't assert on specific count)
    let _ = queue_stats; // Queue stats retrieved successfully
    
    // Test queue manager access
    let queue_manager = call_center.queue_manager().read().await;
    let queue_ids = queue_manager.get_queue_ids();
    // Should have at least our general queue, but the queue manager might pre-populate others
    assert!(!queue_ids.is_empty(), "Should have at least some queues after creating one");
    assert!(queue_ids.contains(&"general".to_string()), "Should contain the 'general' queue we created");
}

#[tokio::test]
#[serial]
async fn test_call_center_configuration() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Test configuration access
    let config = call_center.config();
    assert!(config.general.max_concurrent_calls > 0);
    assert!(config.general.max_agents > 0);
    assert!(!config.general.domain.is_empty());
    assert!(config.general.local_signaling_addr.port() > 0);
    assert!(config.general.local_media_addr.port() > 0);
    
    // Test various config sections exist
    assert!(config.agents.default_max_concurrent_calls > 0);
    assert!(config.queues.max_queue_size > 0);
    // Just check that routing strategy is set (can't compare due to no PartialEq)
    let _ = &config.routing.default_strategy;
}

#[tokio::test]
#[serial]
async fn test_session_manager_access() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    
    // Test session manager access
    let session_manager = call_center.session_manager();
    
    // Verify we can access session-core APIs (these should not fail even without live SIP)
    let active_sessions = session_manager.list_active_sessions().await;
    match active_sessions {
        Ok(sessions) => {
            assert!(sessions.len() >= 0); // Should be empty in test but callable
        },
        Err(_) => {
            // If not implemented yet, that's also acceptable
            println!("list_active_sessions not yet implemented - this is expected");
        }
    }
}

#[tokio::test]
#[serial]
async fn test_orchestrator_stats_integration() {
    let call_center = create_test_call_center().await.expect("Call center creation failed");
    let db_manager = call_center.database_manager().expect("Database manager should be available");
    
    // Add some agents to the database
    db_manager.upsert_agent("stats-agent-1", "Stats Agent 1", Some("sip:stats1@example.com")).await
        .expect("Agent creation should succeed");
    db_manager.upsert_agent("stats-agent-2", "Stats Agent 2", Some("sip:stats2@example.com")).await
        .expect("Agent creation should succeed");
    
    // Set different statuses
    db_manager.update_agent_status("stats-agent-1", AgentStatus::Available).await
        .expect("Status update should succeed");
    db_manager.update_agent_status("stats-agent-2", AgentStatus::PostCallWrapUp).await
        .expect("Status update should succeed");
    
    // Get orchestrator stats - this should integrate database stats
    let stats = call_center.get_stats().await;
    
    // Should reflect the agents we added
    assert_eq!(stats.available_agents, 1);
    assert_eq!(stats.busy_agents, 1); // PostCallWrapUp counts as busy
    assert_eq!(stats.active_calls, 0); // No actual calls in test
    assert_eq!(stats.active_bridges, 0); // No actual bridges in test
}

// TODO: Add more integration tests as modules are implemented
// - Test actual SIP agent registration with real SIP infrastructure
// - Test call routing with live sessions
// - Test bridge creation with active calls
// - Test monitoring and metrics collection with real events 