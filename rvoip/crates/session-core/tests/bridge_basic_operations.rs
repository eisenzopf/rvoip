//! Tests for Basic Bridge Operations
//!
//! Tests the core functionality of SessionBridge including creation, session management,
//! and bridge lifecycle operations (start/stop).

mod common;

use rvoip_session_core::{
    api::types::SessionId,
    bridge::{SessionBridge, BridgeId, BridgeConfig},
};
use common::*;

#[tokio::test]
async fn test_bridge_creation() {
    let bridge_id = "test-bridge-1";
    let bridge = create_test_bridge(bridge_id);
    
    // Verify initial state
    verify_bridge_state(&bridge, false, 0);
}

#[tokio::test]
async fn test_bridge_id_creation() {
    let bridge_id = BridgeId::new("conference-123");
    assert_eq!(bridge_id.0, "conference-123");
    
    let bridge_id2 = BridgeId::new("meeting-room-a");
    assert_ne!(bridge_id.0, bridge_id2.0);
}

#[tokio::test]
async fn test_bridge_config_default() {
    let config = BridgeConfig::default();
    
    assert_eq!(config.max_sessions, 10);
    assert_eq!(config.auto_start, true);
    assert_eq!(config.auto_stop_on_empty, true);
}

#[tokio::test]
async fn test_bridge_config_custom() {
    let config = BridgeConfig {
        max_sessions: 5,
        auto_start: false,
        auto_stop_on_empty: false,
    };
    
    assert_eq!(config.max_sessions, 5);
    assert_eq!(config.auto_start, false);
    assert_eq!(config.auto_stop_on_empty, false);
}

#[tokio::test]
async fn test_bridge_start_stop() {
    let mut bridge = create_test_bridge("lifecycle-test");
    
    // Initial state should be inactive
    assert!(!bridge.is_active());
    
    // Start bridge
    let start_result = bridge.start();
    assert!(start_result.is_ok());
    assert!(bridge.is_active());
    
    // Stop bridge
    let stop_result = bridge.stop();
    assert!(stop_result.is_ok());
    assert!(!bridge.is_active());
}

#[tokio::test]
async fn test_bridge_start_stop_multiple_times() {
    let mut bridge = create_test_bridge("multi-lifecycle-test");
    
    // Start and stop multiple times
    for _ in 0..3 {
        assert!(bridge.start().is_ok());
        assert!(bridge.is_active());
        
        assert!(bridge.stop().is_ok());
        assert!(!bridge.is_active());
    }
}

#[tokio::test]
async fn test_bridge_add_single_session() {
    let mut bridge = create_test_bridge("single-session-test");
    let session_id = SessionId("test-session-1".to_string());
    
    // Add session
    let add_result = bridge.add_session(session_id.clone());
    assert!(add_result.is_ok());
    
    // Verify state
    verify_bridge_state(&bridge, false, 1);
}

#[tokio::test]
async fn test_bridge_add_multiple_sessions() {
    let mut bridge = create_test_bridge("multi-session-test");
    let session_ids = create_test_session_ids(5);
    
    // Add all sessions
    for session_id in &session_ids {
        let add_result = bridge.add_session(session_id.clone());
        assert!(add_result.is_ok());
    }
    
    // Verify final state
    verify_bridge_state(&bridge, false, 5);
}

#[tokio::test]
async fn test_bridge_add_duplicate_session() {
    let mut bridge = create_test_bridge("duplicate-session-test");
    let session_id = SessionId("duplicate-session".to_string());
    
    // Add session first time
    assert!(bridge.add_session(session_id.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    // Add same session again (should still work in HashSet)
    assert!(bridge.add_session(session_id.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1); // Count should remain 1
}

#[tokio::test]
async fn test_bridge_remove_session() {
    let mut bridge = create_test_bridge("remove-session-test");
    let session_id = SessionId("removable-session".to_string());
    
    // Add then remove session
    assert!(bridge.add_session(session_id.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    assert!(bridge.remove_session(&session_id).is_ok());
    verify_bridge_state(&bridge, false, 0);
}

#[tokio::test]
async fn test_bridge_remove_nonexistent_session() {
    let mut bridge = create_test_bridge("remove-nonexistent-test");
    let session_id = SessionId("nonexistent-session".to_string());
    
    // Try to remove session that was never added
    let remove_result = bridge.remove_session(&session_id);
    assert!(remove_result.is_ok()); // Should succeed (HashSet.remove returns bool)
    verify_bridge_state(&bridge, false, 0);
}

#[tokio::test]
async fn test_bridge_add_remove_sequence() {
    let mut bridge = create_test_bridge("add-remove-sequence");
    let session_ids = create_test_session_ids(3);
    
    // Add all sessions
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    verify_bridge_state(&bridge, false, 3);
    
    // Remove sessions one by one
    for (i, session_id) in session_ids.iter().enumerate() {
        assert!(bridge.remove_session(session_id).is_ok());
        verify_bridge_state(&bridge, false, 2 - i);
    }
}

#[tokio::test]
async fn test_bridge_session_management_with_lifecycle() {
    let mut bridge = create_test_bridge("lifecycle-with-sessions");
    let session_ids = create_test_session_ids(2);
    
    // Add sessions to inactive bridge
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    verify_bridge_state(&bridge, false, 2);
    
    // Start bridge with sessions
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 2);
    
    // Add session to active bridge
    let new_session = SessionId("active-bridge-session".to_string());
    assert!(bridge.add_session(new_session.clone()).is_ok());
    verify_bridge_state(&bridge, true, 3);
    
    // Remove session from active bridge
    assert!(bridge.remove_session(&new_session).is_ok());
    verify_bridge_state(&bridge, true, 2);
    
    // Stop bridge
    assert!(bridge.stop().is_ok());
    verify_bridge_state(&bridge, false, 2);
}

#[tokio::test]
async fn test_bridge_session_manager_wrapper() {
    let mut manager = BridgeSessionManager::new("wrapper-test");
    let session_ids = create_test_session_ids(3);
    
    // Test wrapper methods
    for session_id in &session_ids {
        assert!(manager.add_session(session_id.clone()).is_ok());
    }
    
    manager.verify_consistency();
    assert_eq!(manager.sessions().len(), 3);
    
    // Start bridge through wrapper
    assert!(manager.start_bridge().is_ok());
    assert!(manager.bridge().is_active());
    
    // Remove sessions through wrapper
    for session_id in &session_ids {
        assert!(manager.remove_session(session_id).is_ok());
    }
    
    manager.verify_consistency();
    assert_eq!(manager.sessions().len(), 0);
}

#[tokio::test]
async fn test_bridge_with_session_scenario() {
    // Test small bridge scenario
    let (bridge, session_ids) = create_bridge_scenario("small-scenario", 2, false);
    verify_bridge_state(&bridge, false, 2);
    assert_eq!(session_ids.len(), 2);
    
    // Test active bridge scenario
    let (bridge, session_ids) = create_bridge_scenario("active-scenario", 3, true);
    verify_bridge_state(&bridge, true, 3);
    assert_eq!(session_ids.len(), 3);
}

#[tokio::test]
async fn test_multiple_bridges_creation() {
    let bridges = create_multiple_bridges(5, "multi-bridge");
    
    assert_eq!(bridges.len(), 5);
    
    for (i, bridge) in bridges.iter().enumerate() {
        verify_bridge_state(bridge, false, 0);
        // Note: We can't easily verify the bridge ID without exposing it in the API
        println!("Created bridge {}", i);
    }
}

#[tokio::test]
async fn test_bridge_state_consistency() {
    let mut bridge = create_test_bridge("consistency-test");
    let session_ids = create_test_session_ids(10);
    
    // Complex sequence of operations
    assert!(bridge.start().is_ok());
    
    // Add sessions in batches
    for chunk in session_ids.chunks(3) {
        for session_id in chunk {
            assert!(bridge.add_session(session_id.clone()).is_ok());
        }
        // Verify state after each batch
        assert!(bridge.is_active());
    }
    
    verify_bridge_state(&bridge, true, 10);
    
    // Remove sessions in reverse order
    for session_id in session_ids.iter().rev() {
        assert!(bridge.remove_session(session_id).is_ok());
    }
    
    verify_bridge_state(&bridge, true, 0);
    
    assert!(bridge.stop().is_ok());
    verify_bridge_state(&bridge, false, 0);
} 