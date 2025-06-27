use rvoip_session_core::api::control::SessionControl;
// Tests for Bridge Integration with Session Managers
//
// Tests how bridges integrate with SessionCoordinator and real call sessions,
// including bridging multiple active calls together.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::{SessionId, CallState},
    bridge::SessionBridge,
};
use common::*;

#[tokio::test]
async fn test_bridge_integration_helper_creation() {
    let helper = BridgeIntegrationHelper::new(2, 1).await.unwrap();
    
    assert_eq!(helper.managers.len(), 2);
    assert_eq!(helper.bridges.len(), 1);
    
    // Verify managers are running
    for manager in &helper.managers {
        let stats = manager.get_stats().await.unwrap();
        assert_eq!(stats.active_sessions, 0);
    }
    
    // Verify bridge initial state
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state, (false, 0)); // inactive, 0 sessions
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_with_single_call() {
    let helper = BridgeIntegrationHelper::new(2, 1).await.unwrap();
    
    // Create a call between managers
    let call = helper.create_call_between_managers(0, 1).await.unwrap();
    let session_id = call.id().clone();
    
    // Add session to bridge
    let add_result = helper.add_session_to_bridge(0, session_id.clone()).await;
    assert!(add_result.is_ok());
    
    // Verify bridge state
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state.1, 1); // 1 session in bridge
    
    // Start bridge
    let start_result = helper.start_bridge(0).await;
    assert!(start_result.is_ok());
    
    // Verify bridge is active
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state, (true, 1)); // active, 1 session
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_with_multiple_calls() {
    let helper = BridgeIntegrationHelper::new(4, 1).await.unwrap();
    
    // Create multiple calls
    let call1 = helper.create_call_between_managers(0, 1).await.unwrap();
    let call2 = helper.create_call_between_managers(2, 3).await.unwrap();
    
    let session1_id = call1.id().clone();
    let session2_id = call2.id().clone();
    
    // Add both sessions to bridge
    assert!(helper.add_session_to_bridge(0, session1_id.clone()).await.is_ok());
    assert!(helper.add_session_to_bridge(0, session2_id.clone()).await.is_ok());
    
    // Verify bridge has both sessions
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state.1, 2); // 2 sessions in bridge
    
    // Start bridge
    assert!(helper.start_bridge(0).await.is_ok());
    
    // Verify active bridge with 2 sessions
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state, (true, 2));
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_with_conference_scenario() {
    // Create a 3-way conference bridge scenario
    let helper = BridgeIntegrationHelper::new(3, 1).await.unwrap();
    
    // Create calls from manager 0 to managers 1 and 2 (like a conference host)
    let call1 = helper.create_call_between_managers(0, 1).await.unwrap();
    let call2 = helper.create_call_between_managers(0, 2).await.unwrap();
    
    let session1_id = call1.id().clone();
    let session2_id = call2.id().clone();
    
    // Add sessions to conference bridge
    assert!(helper.add_session_to_bridge(0, session1_id.clone()).await.is_ok());
    assert!(helper.add_session_to_bridge(0, session2_id.clone()).await.is_ok());
    
    // Start conference bridge
    assert!(helper.start_bridge(0).await.is_ok());
    
    // Verify conference is active
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state, (true, 2));
    
    println!("✓ Conference bridge established with 2 participants");
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_session_lifecycle_integration() {
    let helper = BridgeIntegrationHelper::new(2, 1).await.unwrap();
    
    // Create call
    let call = helper.create_call_between_managers(0, 1).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify call exists in manager
    let session = helper.managers[0].find_session(&session_id).await.unwrap();
    assert!(session.is_some());
    
    // Add to bridge before starting
    assert!(helper.add_session_to_bridge(0, session_id.clone()).await.is_ok());
    assert!(helper.start_bridge(0).await.is_ok());
    
    // Bridge should be active with 1 session
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state, (true, 1));
    
    // Terminate the call
    let terminate_result = helper.managers[0].terminate_session(&session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    
    // Note: In a real implementation, the bridge should be notified when sessions end
    // and automatically remove them. For now, we test the bridge state independently.
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_multiple_bridges_with_different_sessions() {
    let helper = BridgeIntegrationHelper::new(4, 2).await.unwrap();
    
    // Create two separate calls
    let call1 = helper.create_call_between_managers(0, 1).await.unwrap();
    let call2 = helper.create_call_between_managers(2, 3).await.unwrap();
    
    // Add calls to different bridges
    assert!(helper.add_session_to_bridge(0, call1.id().clone()).await.is_ok());
    assert!(helper.add_session_to_bridge(1, call2.id().clone()).await.is_ok());
    
    // Start both bridges
    assert!(helper.start_bridge(0).await.is_ok());
    assert!(helper.start_bridge(1).await.is_ok());
    
    // Verify both bridges are active with 1 session each
    let bridge1_state = helper.get_bridge_state(0).await.unwrap();
    let bridge2_state = helper.get_bridge_state(1).await.unwrap();
    
    assert_eq!(bridge1_state, (true, 1));
    assert_eq!(bridge2_state, (true, 1));
    
    println!("✓ Multiple independent bridges working correctly");
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_with_session_manager_stats() {
    let helper = BridgeIntegrationHelper::new(3, 1).await.unwrap();
    
    // Check initial stats
    for manager in &helper.managers {
        let stats = manager.get_stats().await.unwrap();
        assert_eq!(stats.active_sessions, 0);
    }
    
    // Create calls
    let call1 = helper.create_call_between_managers(0, 1).await.unwrap();
    let call2 = helper.create_call_between_managers(0, 2).await.unwrap();
    
    // Check stats after creating calls
    let stats = helper.managers[0].get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 2);
    
    // Add to bridge
    assert!(helper.add_session_to_bridge(0, call1.id().clone()).await.is_ok());
    assert!(helper.add_session_to_bridge(0, call2.id().clone()).await.is_ok());
    assert!(helper.start_bridge(0).await.is_ok());
    
    // Stats should still show active sessions (they're now bridged)
    let stats = helper.managers[0].get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 2);
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_error_handling_with_invalid_sessions() {
    let helper = BridgeIntegrationHelper::new(2, 1).await.unwrap();
    
    // Try to add non-existent session to bridge
    let fake_session_id = SessionId("fake-session-id".to_string());
    let add_result = helper.add_session_to_bridge(0, fake_session_id).await;
    
    // This should succeed at the bridge level (bridge doesn't validate session existence)
    // In a real implementation, you might want bridge to validate sessions exist
    assert!(add_result.is_ok());
    
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state.1, 1); // Bridge tracks the session even if it's fake
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_integration_boundary_conditions() {
    let helper = BridgeIntegrationHelper::new(1, 1).await.unwrap();
    
    // Test with invalid bridge index
    let fake_session = SessionId("test-session".to_string());
    let result = helper.add_session_to_bridge(999, fake_session).await;
    assert!(result.is_err());
    
    // Test get state with invalid bridge index
    let state = helper.get_bridge_state(999).await;
    assert!(state.is_none());
    
    // Test start bridge with invalid index
    let result = helper.start_bridge(999).await;
    assert!(result.is_err());
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_integration_cleanup() {
    let helper = BridgeIntegrationHelper::new(2, 2).await.unwrap();
    
    // Create some calls and add to bridges
    let call = helper.create_call_between_managers(0, 1).await.unwrap();
    assert!(helper.add_session_to_bridge(0, call.id().clone()).await.is_ok());
    assert!(helper.start_bridge(0).await.is_ok());
    
    // Verify bridge is active
    let bridge_state = helper.get_bridge_state(0).await.unwrap();
    assert_eq!(bridge_state.0, true);
    
    // Cleanup should stop all bridges and managers
    let cleanup_result = helper.cleanup().await;
    assert!(cleanup_result.is_ok());
    
    println!("✓ Bridge integration cleanup completed successfully");
}

#[tokio::test]
async fn test_bridge_with_real_call_establishment() {
    // Create regular session manager pair for real call establishment
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create a bridge
    let mut bridge = SessionBridge::new("real-call-bridge".to_string());
    
    // Establish real call
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    
    // Add caller session to bridge
    let add_result = bridge.add_session(call.id().clone());
    assert!(add_result.is_ok());
    
    // If callee session exists, add it too
    if let Some(callee_id) = callee_session_id {
        let add_result = bridge.add_session(callee_id);
        assert!(add_result.is_ok());
        verify_bridge_state(&bridge, false, 2);
    } else {
        verify_bridge_state(&bridge, false, 1);
    }
    
    // Start bridge
    assert!(bridge.start().is_ok());
    assert!(bridge.is_active());
    
    println!("✓ Bridge successfully integrated with real call establishment");
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
} 