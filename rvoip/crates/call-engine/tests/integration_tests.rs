//! Integration tests for the call center engine
//!
//! These tests verify that all modules work together correctly.

use rvoip_call_engine::prelude::*;
use tokio_test;

#[tokio::test]
async fn test_call_center_initialization() {
    // Test basic call center initialization
    let database = CallCenterDatabase::new_in_memory().await.expect("Database creation failed");
    let config = CallCenterConfig::default();
    
    let call_center = CallCenterEngine::new(config, database).await;
    assert!(call_center.is_ok(), "Call center initialization should succeed");
}

#[tokio::test]
async fn test_call_center_statistics() {
    // Test statistics collection
    let database = CallCenterDatabase::new_in_memory().await.expect("Database creation failed");
    let config = CallCenterConfig::default();
    
    let call_center = CallCenterEngine::new(config, database).await.expect("Call center creation failed");
    let stats = call_center.get_statistics();
    
    // Initially should have zero active calls and bridges
    assert_eq!(stats.active_calls, 0);
    assert_eq!(stats.active_bridges, 0);
    assert_eq!(stats.total_calls_handled, 0);
}

// TODO: Add more integration tests as modules are implemented
// - Test agent registration and status changes
// - Test call queuing and dequeuing
// - Test call routing decisions
// - Test bridge creation and management
// - Test monitoring and metrics collection 