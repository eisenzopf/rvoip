use rvoip_session_core::api::control::SessionControl;
// Tests for Bridge Error Handling and Edge Cases
//
// Tests error conditions, edge cases, and boundary conditions for bridge functionality.
// Ensures bridges handle failures gracefully and maintain consistency.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::SessionId,
    bridge::{SessionBridge, BridgeId, BridgeConfig},
    Result,
};
use common::*;

#[tokio::test]
async fn test_bridge_operations_on_stopped_bridge() {
    let mut bridge = create_test_bridge("stopped-bridge-test");
    let session_id = SessionId("test-session".to_string());
    
    // Bridge starts inactive
    assert!(!bridge.is_active());
    
    // Operations should work on inactive bridge
    assert!(bridge.add_session(session_id.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    assert!(bridge.remove_session(&session_id).is_ok());
    verify_bridge_state(&bridge, false, 0);
    
    // Start and stop
    assert!(bridge.start().is_ok());
    assert!(bridge.stop().is_ok());
    
    // Operations should still work after stopping
    assert!(bridge.add_session(session_id.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
}

#[tokio::test]
async fn test_bridge_double_start_stop() {
    let mut bridge = create_test_bridge("double-ops-test");
    
    // Double start should be idempotent
    assert!(bridge.start().is_ok());
    assert!(bridge.is_active());
    
    assert!(bridge.start().is_ok()); // Should not fail
    assert!(bridge.is_active());
    
    // Double stop should be idempotent
    assert!(bridge.stop().is_ok());
    assert!(!bridge.is_active());
    
    assert!(bridge.stop().is_ok()); // Should not fail
    assert!(!bridge.is_active());
}

#[tokio::test]
async fn test_bridge_empty_session_operations() {
    let mut bridge = create_test_bridge("empty-ops-test");
    
    // Remove from empty bridge
    let nonexistent_session = SessionId("nonexistent".to_string());
    assert!(bridge.remove_session(&nonexistent_session).is_ok());
    verify_bridge_state(&bridge, false, 0);
    
    // Start empty bridge
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 0);
    
    // Stop empty bridge
    assert!(bridge.stop().is_ok());
    verify_bridge_state(&bridge, false, 0);
}

#[tokio::test]
async fn test_bridge_with_invalid_session_ids() {
    let mut bridge = create_test_bridge("invalid-session-test");
    
    // Test with empty session ID
    let empty_session = SessionId("".to_string());
    assert!(bridge.add_session(empty_session.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    assert!(bridge.remove_session(&empty_session).is_ok());
    verify_bridge_state(&bridge, false, 0);
    
    // Test with very long session ID
    let long_session = SessionId("a".repeat(1000));
    assert!(bridge.add_session(long_session.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    assert!(bridge.remove_session(&long_session).is_ok());
    verify_bridge_state(&bridge, false, 0);
    
    // Test with special characters
    let special_session = SessionId("session-with-!@#$%^&*()_+-={}[]|\\:;\"'<>?,./".to_string());
    assert!(bridge.add_session(special_session.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
}

#[tokio::test]
async fn test_bridge_boundary_conditions() {
    let mut bridge = create_test_bridge("boundary-test");
    
    // Test with exactly one session
    let session1 = SessionId("single-session".to_string());
    assert!(bridge.add_session(session1.clone()).is_ok());
    verify_bridge_state(&bridge, false, 1);
    
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 1);
    
    assert!(bridge.remove_session(&session1).is_ok());
    verify_bridge_state(&bridge, true, 0);
    
    // Test rapid add/remove of same session
    for _ in 0..10 {
        assert!(bridge.add_session(session1.clone()).is_ok());
        assert!(bridge.remove_session(&session1).is_ok());
    }
    verify_bridge_state(&bridge, true, 0);
}

#[tokio::test]
async fn test_bridge_session_manager_error_conditions() {
    let mut manager = BridgeSessionCoordinator::new("error-test");
    
    // Test removing session that was never added
    let fake_session = SessionId("never-added".to_string());
    assert!(manager.remove_session(&fake_session).is_ok()); // Should not fail
    
    manager.verify_consistency();
    
    // Test multiple start/stop cycles
    for _ in 0..5 {
        assert!(manager.start_bridge().is_ok());
        assert!(manager.stop_bridge().is_ok());
    }
    
    manager.verify_consistency();
}

#[tokio::test]
async fn test_bridge_integration_error_scenarios() {
    let helper = BridgeIntegrationHelper::new(2, 1).await.unwrap();
    
    // Test adding session to non-existent bridge
    let session_id = SessionId("test-session".to_string());
    let result = helper.add_session_to_bridge(999, session_id).await;
    assert!(result.is_err());
    
    // Test starting non-existent bridge
    let result = helper.start_bridge(999).await;
    assert!(result.is_err());
    
    // Test getting state of non-existent bridge
    let state = helper.get_bridge_state(999).await;
    assert!(state.is_none());
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_concurrent_error_conditions() {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    
    let bridge = Arc::new(Mutex::new(create_test_bridge("concurrent-error-test")));
    let mut handles = Vec::new();
    
    // Spawn tasks that might conflict
    for i in 0..10 {
        let bridge_clone = bridge.clone();
        let handle = tokio::spawn(async move {
            let session_id = SessionId(format!("concurrent-session-{}", i));
            
            // Each task tries to add and remove the same session multiple times
            for _ in 0..20 {
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    let _ = bridge_guard.add_session(session_id.clone());
                }
                
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    let _ = bridge_guard.remove_session(&session_id);
                }
            }
        });
        handles.push(handle);
    }
    
    // Wait for all tasks
    for handle in handles {
        handle.await.expect("Concurrent task failed");
    }
    
    // Bridge should be in a consistent state
    let bridge_guard = bridge.lock().await;
    println!("Final bridge state after concurrent operations: {} sessions", bridge_guard.session_count());
    
    // The exact count is unpredictable due to race conditions, but it should be valid
    assert!(bridge_guard.session_count() <= 10); // At most one session per task
}

#[tokio::test]
async fn test_bridge_resource_exhaustion_simulation() {
    // Simulate resource exhaustion by creating many bridges
    let bridge_count = 100;
    let mut bridges = Vec::new();
    
    // Create many bridges
    for i in 0..bridge_count {
        let bridge = create_test_bridge(&format!("resource-test-{}", i));
        bridges.push(bridge);
    }
    
    // Add sessions to each bridge
    for (i, bridge) in bridges.iter_mut().enumerate() {
        let session_id = SessionId(format!("session-{}", i));
        assert!(bridge.add_session(session_id).is_ok());
        assert!(bridge.start().is_ok());
    }
    
    // Verify all bridges are working
    for bridge in &bridges {
        verify_bridge_state(bridge, true, 1);
    }
    
    println!("✓ Successfully created and managed {} bridges", bridge_count);
}

#[tokio::test]
async fn test_bridge_config_edge_cases() {
    // Test with zero max sessions
    let config = BridgeConfig {
        max_sessions: 0,
        auto_start: true,
        auto_stop_on_empty: true,
    };
    
    // Note: Current implementation doesn't use config, but test the config itself
    assert_eq!(config.max_sessions, 0);
    
    // Test with very large max sessions
    let large_config = BridgeConfig {
        max_sessions: usize::MAX,
        auto_start: false,
        auto_stop_on_empty: false,
    };
    
    assert_eq!(large_config.max_sessions, usize::MAX);
}

#[tokio::test]
async fn test_bridge_id_edge_cases() {
    // Test with empty bridge ID
    let empty_id = BridgeId::new("");
    assert_eq!(empty_id.0, "");
    
    // Test with very long bridge ID
    let long_id = BridgeId::new(&"x".repeat(10000));
    assert_eq!(long_id.0.len(), 10000);
    
    // Test with special characters
    let special_id = BridgeId::new("bridge-!@#$%^&*()_+-={}[]|\\:;\"'<>?,./");
    assert!(special_id.0.contains("!@#$"));
    
    // Test with unicode characters
    let unicode_id = BridgeId::new("bridge-🌉-测试-🎵");
    assert!(unicode_id.0.contains("🌉"));
}

#[tokio::test]
async fn test_bridge_state_consistency_under_errors() {
    let mut bridge = create_test_bridge("consistency-error-test");
    let session_ids = create_test_session_ids(10);
    
    // Add sessions
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    
    // Start bridge
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 10);
    
    // Simulate error conditions by rapid operations
    for _ in 0..100 {
        // Try to add already existing sessions (should be idempotent)
        for session_id in &session_ids {
            let _ = bridge.add_session(session_id.clone());
        }
        
        // Try to remove non-existent sessions
        let fake_session = SessionId("fake-session".to_string());
        let _ = bridge.remove_session(&fake_session);
        
        // Rapid start/stop
        let _ = bridge.stop();
        let _ = bridge.start();
    }
    
    // Bridge should still be in a consistent state
    verify_bridge_state(&bridge, true, 10);
    
    // All original sessions should still be present
    for session_id in &session_ids {
        assert!(bridge.remove_session(session_id).is_ok());
    }
    
    verify_bridge_state(&bridge, true, 0);
}

#[tokio::test]
async fn test_bridge_cleanup_after_errors() {
    let mut bridges = Vec::new();
    
    // Create bridges and intentionally cause some "errors" (edge cases)
    for i in 0..10 {
        let mut bridge = create_test_bridge(&format!("cleanup-test-{}", i));
        
        // Add sessions
        let session_ids = create_test_session_ids(5);
        for session_id in &session_ids {
            assert!(bridge.add_session(session_id.clone()).is_ok());
        }
        
        // Start bridge
        assert!(bridge.start().is_ok());
        
        // Simulate some error conditions
        if i % 2 == 0 {
            // For even bridges, remove all sessions but keep bridge active
            for session_id in &session_ids {
                assert!(bridge.remove_session(session_id).is_ok());
            }
        } else {
            // For odd bridges, stop bridge but keep sessions
            assert!(bridge.stop().is_ok());
        }
        
        bridges.push(bridge);
    }
    
    // Verify all bridges are in valid states
    for (i, bridge) in bridges.iter().enumerate() {
        if i % 2 == 0 {
            verify_bridge_state(bridge, true, 0); // Active but empty
        } else {
            verify_bridge_state(bridge, false, 5); // Inactive with sessions
        }
    }
    
    println!("✓ All bridges maintained valid state after error simulation");
}

#[tokio::test]
async fn test_bridge_memory_safety_edge_cases() {
    // Test scenarios that might cause memory issues
    
    // Test 1: Rapid creation and destruction of session IDs
    let mut bridge = create_test_bridge("memory-safety-test");
    
    for i in 0..1000 {
        let session_id = SessionId(format!("temp-session-{}", i));
        assert!(bridge.add_session(session_id.clone()).is_ok());
        assert!(bridge.remove_session(&session_id).is_ok());
    }
    
    verify_bridge_state(&bridge, false, 0);
    
    // Test 2: Large session ID strings
    for i in 0..100 {
        let large_session_id = SessionId(format!("large-session-{}-{}", i, "x".repeat(1000)));
        assert!(bridge.add_session(large_session_id.clone()).is_ok());
        assert!(bridge.remove_session(&large_session_id).is_ok());
    }
    
    verify_bridge_state(&bridge, false, 0);
    
    println!("✓ Memory safety edge cases handled correctly");
} 