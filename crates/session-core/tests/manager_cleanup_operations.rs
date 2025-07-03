use rvoip_session_core::api::control::SessionControl;
// Tests for CleanupManager Operations
//
// Tests the cleanup manager functionality including resource cleanup,
// session timeouts, and general cleanup management.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::{SessionId, CallState},
    manager::cleanup::CleanupManager,
};
use common::*;

#[tokio::test]
async fn test_cleanup_manager_creation() {
    let helper = CleanupTestHelper::new();
    
    // Initially should not be running
    // Note: CleanupManager doesn't expose is_running, so we test by operations
    let cleanup_manager = helper.cleanup_manager();
    assert!(cleanup_manager.start().await.is_ok());
    assert!(cleanup_manager.stop().await.is_ok());
}

#[tokio::test]
async fn test_cleanup_manager_start_stop() {
    let helper = CleanupTestHelper::new();
    
    // Start the manager
    helper.start().await.unwrap();
    
    // Stop the manager
    helper.stop().await.unwrap();
    
    // Should be able to start again
    helper.start().await.unwrap();
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_start_stop_multiple_times() {
    let helper = CleanupTestHelper::new();
    
    // Multiple start/stop cycles
    for i in 0..5 {
        println!("Cycle {}", i);
        helper.start().await.unwrap();
        helper.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_cleanup_session_operation() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    let session_id = SessionId("cleanup-test-session".to_string());
    
    // Add a test resource
    helper.add_test_resource("test-resource-1").await;
    
    // Cleanup specific session
    let result = helper.cleanup_session(&session_id).await;
    assert!(result.is_ok());
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_multiple_sessions() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    let session_ids = vec![
        SessionId("session-1".to_string()),
        SessionId("session-2".to_string()),
        SessionId("session-3".to_string()),
    ];
    
    // Add test resources for each session
    for i in 0..session_ids.len() {
        helper.add_test_resource(&format!("resource-{}", i)).await;
    }
    
    // Cleanup each session
    for session_id in &session_ids {
        let result = helper.cleanup_session(session_id).await;
        assert!(result.is_ok());
    }
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_force_cleanup_all() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    // Add multiple test resources
    for i in 0..10 {
        helper.add_test_resource(&format!("bulk-resource-{}", i)).await;
    }
    
    // Force cleanup all
    let result = helper.force_cleanup_all().await;
    assert!(result.is_ok());
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_nonexistent_session() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    let fake_session_id = SessionId("nonexistent-session".to_string());
    
    // Should not fail to cleanup nonexistent session
    let result = helper.cleanup_session(&fake_session_id).await;
    assert!(result.is_ok());
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_with_stopped_manager() {
    let helper = CleanupTestHelper::new();
    // Note: Manager is not started
    
    let session_id = SessionId("stopped-manager-test".to_string());
    
    // Operations should still work when manager is stopped
    let result = helper.cleanup_session(&session_id).await;
    assert!(result.is_ok());
    
    let result = helper.force_cleanup_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_cleanup_manager_concurrent_operations() {
    let cleanup_manager = Arc::new(CleanupManager::new());
    cleanup_manager.start().await.unwrap();
    
    let session_count = 10;
    let mut handles = Vec::new();
    
    // Spawn concurrent cleanup operations
    for i in 0..session_count {
        let manager_clone = Arc::clone(&cleanup_manager);
        let handle = tokio::spawn(async move {
            let session_id = SessionId(format!("concurrent-session-{}", i));
            
            // Perform cleanup operation
            manager_clone.cleanup_session(&session_id).await?;
            
            Ok::<(), rvoip_session_core::SessionError>(())
        });
        handles.push(handle);
    }
    
    // Wait for all operations to complete
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    
    cleanup_manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_stress_operations() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    let operation_count = 100;
    
    // Perform many cleanup operations rapidly
    for i in 0..operation_count {
        let session_id = SessionId(format!("stress-session-{}", i));
        helper.cleanup_session(&session_id).await.unwrap();
        
        // Occasionally do force cleanup
        if i % 10 == 0 {
            helper.force_cleanup_all().await.unwrap();
        }
    }
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_lifecycle_with_sessions() {
    let helper = CleanupTestHelper::new();
    
    // Start manager
    helper.start().await.unwrap();
    
    // Add some test resources
    for i in 0..5 {
        helper.add_test_resource(&format!("lifecycle-resource-{}", i)).await;
    }
    
    // Perform some cleanups
    for i in 0..3 {
        let session_id = SessionId(format!("lifecycle-session-{}", i));
        helper.cleanup_session(&session_id).await.unwrap();
    }
    
    // Stop manager (should clean up remaining resources)
    helper.stop().await.unwrap();
    
    // Restart and verify it works
    helper.start().await.unwrap();
    
    let session_id = SessionId("post-restart-session".to_string());
    helper.cleanup_session(&session_id).await.unwrap();
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_performance() {
    let cleanup_manager = Arc::new(CleanupManager::new());
    cleanup_manager.start().await.unwrap();
    
    let cleanup_count = 1000;
    let start = std::time::Instant::now();
    
    // Perform many cleanup operations
    for i in 0..cleanup_count {
        let session_id = SessionId(format!("perf-session-{}", i));
        cleanup_manager.cleanup_session(&session_id).await.unwrap();
    }
    
    let elapsed = start.elapsed();
    println!("Performed {} cleanups in {:?}", cleanup_count, elapsed);
    
    // Performance assertion
    assert!(elapsed < Duration::from_secs(10), "Cleanup operations took too long");
    
    cleanup_manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_integration_with_registry() {
    // Test cleanup manager working with a registry
    let mut registry_helper = RegistryTestHelper::new();
    let cleanup_helper = CleanupTestHelper::new();
    
    cleanup_helper.start().await.unwrap();
    
    // Add some sessions to registry
    let session1_id = registry_helper.add_test_session(
        "sip:alice@localhost",
        "sip:bob@localhost",
        CallState::Active
    ).await;
    
    let session2_id = registry_helper.add_test_session(
        "sip:charlie@localhost",
        "sip:david@localhost",
        CallState::Active
    ).await;
    
    registry_helper.verify_session_count(2).await;
    
    // Use cleanup manager to clean sessions
    cleanup_helper.cleanup_session(&session1_id).await.unwrap();
    cleanup_helper.cleanup_session(&session2_id).await.unwrap();
    
    // Note: CleanupManager doesn't directly interact with registry in the current implementation
    // This test verifies they can work together without conflicts
    
    cleanup_helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_error_conditions() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    // Test cleanup with various edge cases
    let edge_case_sessions = vec![
        SessionId("".to_string()), // Empty session ID
        SessionId("very-long-session-id-that-might-cause-issues-with-some-systems-if-they-have-length-limits".to_string()),
        SessionId("session-with-special-chars-!@#$%^&*()".to_string()),
        SessionId("session\nwith\nnewlines".to_string()),
    ];
    
    for session_id in edge_case_sessions {
        let result = helper.cleanup_session(&session_id).await;
        assert!(result.is_ok(), "Cleanup should handle edge case session ID: {:?}", session_id);
    }
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_repeated_operations() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    let session_id = SessionId("repeated-cleanup-session".to_string());
    
    // Cleanup the same session multiple times (should be idempotent)
    for i in 0..5 {
        println!("Cleanup iteration {}", i);
        let result = helper.cleanup_session(&session_id).await;
        assert!(result.is_ok());
    }
    
    // Force cleanup multiple times
    for i in 0..3 {
        println!("Force cleanup iteration {}", i);
        let result = helper.force_cleanup_all().await;
        assert!(result.is_ok());
    }
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_rapid_start_stop() {
    let helper = CleanupTestHelper::new();
    
    // Rapid start/stop cycles
    for i in 0..10 {
        helper.start().await.unwrap();
        
        // Quick operation
        let session_id = SessionId(format!("rapid-session-{}", i));
        helper.cleanup_session(&session_id).await.unwrap();
        
        helper.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_cleanup_manager_mixed_operations() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    // Mix different types of operations
    for i in 0..20 {
        match i % 3 {
            0 => {
                let session_id = SessionId(format!("mixed-session-{}", i));
                helper.cleanup_session(&session_id).await.unwrap();
            },
            1 => {
                helper.force_cleanup_all().await.unwrap();
            },
            2 => {
                helper.add_test_resource(&format!("mixed-resource-{}", i)).await;
            },
            _ => unreachable!(),
        }
    }
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_timeout_scenarios() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    // Simulate scenarios where cleanup might need to handle timeouts
    let session_id = SessionId("timeout-test-session".to_string());
    
    // Add some test resources
    helper.add_test_resource("timeout-resource").await;
    
    // Perform cleanup operation (should complete even if there are timeouts internally)
    let result = helper.cleanup_session(&session_id).await;
    assert!(result.is_ok());
    
    helper.stop().await.unwrap();
}

#[tokio::test]
async fn test_cleanup_manager_resource_tracking() {
    let helper = CleanupTestHelper::new();
    helper.start().await.unwrap();
    
    // Add tracked resources
    let resources = vec![
        "resource-1",
        "resource-2", 
        "resource-3",
    ];
    
    for resource in &resources {
        helper.add_test_resource(resource).await;
    }
    
    // Perform cleanup operations
    for i in 0..resources.len() {
        let session_id = SessionId(format!("tracking-session-{}", i));
        helper.cleanup_session(&session_id).await.unwrap();
    }
    
    // Force cleanup all
    helper.force_cleanup_all().await.unwrap();
    
    helper.stop().await.unwrap();
} 