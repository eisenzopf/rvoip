use rvoip_session_core::api::control::SessionControl;
// Tests for Coordination Resource Management
//
// Tests for resource limits and allocation management in session coordination.

mod common;

use std::time::Duration;
use rvoip_session_core::coordination::resources::{ResourceManager, ResourceLimits};
use rvoip_session_core::api::types::SessionId;
use common::*;

#[tokio::test]
async fn test_resource_manager_creation() {
    println!("🧪 Testing resource manager creation...");
    
    // Test default creation
    let manager = ResourceManager::default();
    assert!(manager.can_create_session(), "Default manager should allow session creation");
    
    // Test custom limits creation
    let limits = ResourceLimits {
        max_sessions: 100,
        max_media_ports: 200,
    };
    let manager = ResourceManager::new(limits);
    assert!(manager.can_create_session(), "Custom manager should allow session creation");
    
    println!("✅ Resource manager creation tests passed");
}

#[tokio::test]
async fn test_basic_session_allocation() {
    println!("🧪 Testing basic session allocation...");
    
    let mut helper = ResourceTestHelper::new_with_limits(10, 20);
    
    // Test successful allocation
    let session_id = helper.allocate_session().expect("Should be able to allocate session");
    assert_eq!(helper.allocated_count(), 1, "Should have 1 allocated session");
    assert!(helper.manager().can_create_session(), "Should still be able to create more sessions");
    
    // Test deallocation
    helper.deallocate_session(&session_id).expect("Should be able to deallocate session");
    assert_eq!(helper.allocated_count(), 0, "Should have 0 allocated sessions");
    
    println!("✅ Basic session allocation tests passed");
}

#[tokio::test]
async fn test_session_limit_enforcement() {
    println!("🧪 Testing session limit enforcement...");
    
    let mut helper = ResourceTestHelper::new_with_limits(2, 4);
    
    // Allocate up to the limit
    let session1 = helper.allocate_session().expect("First allocation should succeed");
    let session2 = helper.allocate_session().expect("Second allocation should succeed");
    
    assert_eq!(helper.allocated_count(), 2, "Should have 2 allocated sessions");
    assert!(!helper.manager().can_create_session(), "Should not be able to create more sessions");
    
    // Try to exceed limit
    let result = helper.allocate_session();
    assert!(result.is_err(), "Third allocation should fail");
    
    // Deallocate one and try again
    helper.deallocate_session(&session1).expect("Should be able to deallocate");
    assert!(helper.manager().can_create_session(), "Should be able to create session after deallocation");
    
    let session3 = helper.allocate_session().expect("Allocation after deallocation should succeed");
    assert_eq!(helper.allocated_count(), 2, "Should have 2 allocated sessions again");
    
    // Cleanup
    helper.deallocate_session(&session2).expect("Should be able to deallocate");
    helper.deallocate_session(&session3).expect("Should be able to deallocate");
    
    println!("✅ Session limit enforcement tests passed");
}

#[tokio::test]
async fn test_multiple_allocations_and_deallocations() {
    println!("🧪 Testing multiple allocations and deallocations...");
    
    let mut helper = ResourceTestHelper::new_with_limits(5, 10);
    
    // Allocate multiple sessions
    let sessions = helper.allocate_sessions(4).expect("Should be able to allocate 4 sessions");
    assert_eq!(sessions.len(), 4, "Should have allocated 4 sessions");
    assert_eq!(helper.allocated_count(), 4, "Helper should track 4 allocated sessions");
    
    // Deallocate some sessions
    helper.deallocate_session(&sessions[0]).expect("Should be able to deallocate");
    helper.deallocate_session(&sessions[2]).expect("Should be able to deallocate");
    assert_eq!(helper.allocated_count(), 2, "Should have 2 allocated sessions remaining");
    
    // Allocate more sessions
    let new_sessions = helper.allocate_sessions(3).expect("Should be able to allocate 3 more sessions");
    assert_eq!(new_sessions.len(), 3, "Should have allocated 3 new sessions");
    assert_eq!(helper.allocated_count(), 5, "Should have 5 total allocated sessions");
    
    // Try to allocate one more (should fail)
    let result = helper.allocate_session();
    assert!(result.is_err(), "Should not be able to exceed limit");
    
    println!("✅ Multiple allocations and deallocations tests passed");
}

#[tokio::test]
async fn test_resource_edge_cases() {
    println!("🧪 Testing resource management edge cases...");
    
    // Test zero limits
    let mut helper = ResourceTestHelper::new_with_limits(0, 0);
    assert!(!helper.manager().can_create_session(), "Should not be able to create sessions with zero limit");
    
    let result = helper.allocate_session();
    assert!(result.is_err(), "Allocation should fail with zero limit");
    
    // Test single session limit
    let mut helper = ResourceTestHelper::new_with_limits(1, 2);
    let session = helper.allocate_session().expect("Should be able to allocate single session");
    assert!(!helper.manager().can_create_session(), "Should not be able to create more with limit of 1");
    
    // Test deallocation of non-existent session
    let fake_session = SessionId::new();
    let result = helper.deallocate_session(&fake_session);
    assert!(result.is_err(), "Should fail to deallocate non-existent session");
    
    // Cleanup
    helper.deallocate_session(&session).expect("Should be able to deallocate real session");
    
    println!("✅ Resource edge cases tests passed");
}

#[tokio::test]
async fn test_resource_limits_creation() {
    println!("🧪 Testing resource limits creation and validation...");
    
    // Test default limits
    let default_limits = ResourceLimits::default();
    assert_eq!(default_limits.max_sessions, 1000, "Default max sessions should be 1000");
    assert_eq!(default_limits.max_media_ports, 2000, "Default max media ports should be 2000");
    
    // Test custom limits
    let custom_limits = CoordinationTestUtils::create_test_limits(50, 100);
    assert_eq!(custom_limits.max_sessions, 50, "Custom max sessions should be 50");
    assert_eq!(custom_limits.max_media_ports, 100, "Custom max media ports should be 100");
    
    // Test small limits for edge case testing
    let small_limits = CoordinationTestUtils::create_small_limits();
    assert_eq!(small_limits.max_sessions, 2, "Small max sessions should be 2");
    assert_eq!(small_limits.max_media_ports, 4, "Small max media ports should be 4");
    
    // Test large limits for stress testing
    let large_limits = CoordinationTestUtils::create_large_limits();
    assert_eq!(large_limits.max_sessions, 10000, "Large max sessions should be 10000");
    assert_eq!(large_limits.max_media_ports, 20000, "Large max media ports should be 20000");
    
    println!("✅ Resource limits creation tests passed");
}

#[tokio::test]
async fn test_resource_allocation_consistency() {
    println!("🧪 Testing resource allocation consistency...");
    
    let mut helper = ResourceTestHelper::new_with_limits(10, 20);
    
    // Allocate and deallocate sessions in various patterns
    let session1 = helper.allocate_session().expect("Should allocate session 1");
    let session2 = helper.allocate_session().expect("Should allocate session 2");
    let session3 = helper.allocate_session().expect("Should allocate session 3");
    
    assert!(helper.verify_allocation_consistency(), "Allocation should be consistent");
    
    // Deallocate middle session
    helper.deallocate_session(&session2).expect("Should deallocate session 2");
    assert!(helper.verify_allocation_consistency(), "Allocation should remain consistent");
    
    // Allocate new session
    let session4 = helper.allocate_session().expect("Should allocate session 4");
    assert!(helper.verify_allocation_consistency(), "Allocation should remain consistent");
    
    // Cleanup all sessions
    helper.deallocate_session(&session1).expect("Should deallocate session 1");
    helper.deallocate_session(&session3).expect("Should deallocate session 3");
    helper.deallocate_session(&session4).expect("Should deallocate session 4");
    
    assert_eq!(helper.allocated_count(), 0, "Should have no allocated sessions");
    assert!(helper.verify_allocation_consistency(), "Final state should be consistent");
    
    println!("✅ Resource allocation consistency tests passed");
}

#[tokio::test]
async fn test_resource_allocation_performance() {
    println!("🧪 Testing resource allocation performance...");
    
    let mut helper = ResourceTestHelper::new_with_limits(1000, 2000);
    let config = CoordinationPerfTestConfig::fast();
    
    // Test allocation performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        helper.allocate_sessions(100)
    });
    
    assert!(result.is_ok(), "Should be able to allocate 100 sessions");
    let sessions = result.unwrap();
    assert_eq!(sessions.len(), 100, "Should have allocated 100 sessions");
    
    println!("Allocated 100 sessions in {:?}", duration);
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(100),
        "resource_allocation_performance"
    );
    
    // Test deallocation performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        for session in &sessions {
            helper.deallocate_session(session)?;
        }
        Ok::<(), rvoip_session_core::SessionError>(())
    });
    
    assert!(result.is_ok(), "Should be able to deallocate all sessions");
    println!("Deallocated 100 sessions in {:?}", duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(100),
        "resource_deallocation_performance"
    );
    
    println!("✅ Resource allocation performance tests passed");
}

#[tokio::test]
async fn test_resource_stress_allocation() {
    println!("🧪 Testing resource stress allocation...");
    
    let mut helper = ResourceTestHelper::new_with_limits(100, 200);
    let config = CoordinationPerfTestConfig::fast();
    
    // Run stress test
    let (allocated, duration) = helper.stress_test_allocations(&config)
        .expect("Stress test should complete");
    
    println!("Stress test allocated {} sessions in {:?}", allocated, duration);
    
    // Verify results
    assert!(allocated > 0, "Should have allocated some sessions");
    assert!(allocated <= config.max_sessions, "Should not exceed max sessions");
    assert!(duration <= config.test_duration * 2, "Should complete within reasonable time");
    
    // Verify final state
    assert!(helper.allocated_count() <= config.max_sessions, "Should not exceed limits");
    assert!(helper.verify_allocation_consistency(), "Should maintain consistency");
    
    println!("✅ Resource stress allocation tests passed");
}

#[tokio::test]
async fn test_resource_concurrent_operations() {
    println!("🧪 Testing resource concurrent operations...");
    
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinSet;
    
    let helper = Arc::new(Mutex::new(ResourceTestHelper::new_with_limits(50, 100)));
    let mut join_set = JoinSet::new();
    
    // Spawn concurrent allocation tasks
    for i in 0..10 {
        let helper_clone = helper.clone();
        join_set.spawn(async move {
            let mut allocations = Vec::new();
            for _ in 0..5 {
                if let Ok(mut h) = helper_clone.try_lock() {
                    if let Ok(session_id) = h.allocate_session() {
                        allocations.push(session_id);
                    }
                }
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
            (i, allocations)
        });
    }
    
    // Collect results
    let mut total_allocated = 0;
    while let Some(result) = join_set.join_next().await {
        if let Ok((task_id, allocations)) = result {
            println!("Task {} allocated {} sessions", task_id, allocations.len());
            total_allocated += allocations.len();
        }
    }
    
    println!("Total allocated across all tasks: {}", total_allocated);
    
    // Verify final state
    let final_helper = helper.lock().unwrap();
    assert!(final_helper.allocated_count() <= 50, "Should not exceed session limit");
    assert!(final_helper.verify_allocation_consistency(), "Should maintain consistency");
    
    println!("✅ Resource concurrent operations tests passed");
}

#[tokio::test]
async fn test_resource_edge_case_scenarios() {
    println!("🧪 Testing resource edge case scenarios...");
    
    let scenarios = CoordinationTestUtils::create_edge_case_scenarios();
    
    for (name, max_sessions, max_media_ports) in scenarios {
        println!("Testing scenario: {}", name);
        
        let mut helper = ResourceTestHelper::new_with_limits(max_sessions, max_media_ports);
        
        // Test allocation behavior for this scenario
        match max_sessions {
            0 => {
                // Should not be able to allocate any sessions
                assert!(!helper.manager().can_create_session(), "Should not allow sessions with 0 limit");
                assert!(helper.allocate_session().is_err(), "Allocation should fail with 0 limit");
            },
            1 => {
                // Should be able to allocate exactly one session
                let session = helper.allocate_session().expect("Should allocate single session");
                assert!(!helper.manager().can_create_session(), "Should not allow more than 1 session");
                assert!(helper.allocate_session().is_err(), "Second allocation should fail");
                helper.deallocate_session(&session).expect("Should deallocate session");
            },
            _ => {
                // Should be able to allocate up to the limit
                let sessions = helper.allocate_sessions(max_sessions.min(10))
                    .expect("Should allocate sessions up to limit");
                assert_eq!(
                    sessions.len(),
                    max_sessions.min(10),
                    "Should allocate expected number of sessions"
                );
            }
        }
        
        assert!(helper.verify_allocation_consistency(), "Should maintain consistency for {}", name);
    }
    
    println!("✅ Resource edge case scenarios tests passed");
}

#[tokio::test]
async fn test_resource_limit_boundary_conditions() {
    println!("🧪 Testing resource limit boundary conditions...");
    
    let mut helper = ResourceTestHelper::new_with_limits(3, 6);
    
    // Test allocation right at the boundary
    let session1 = helper.allocate_session().expect("Should allocate session 1");
    let session2 = helper.allocate_session().expect("Should allocate session 2");
    let session3 = helper.allocate_session().expect("Should allocate session 3");
    
    // Verify we're at the limit
    assert_eq!(helper.allocated_count(), 3, "Should be at session limit");
    assert!(!helper.manager().can_create_session(), "Should not allow more sessions");
    
    // Try one more allocation (should fail)
    assert!(helper.allocate_session().is_err(), "Should fail to allocate beyond limit");
    
    // Deallocate one and verify we can allocate again
    helper.deallocate_session(&session2).expect("Should deallocate session");
    assert!(helper.manager().can_create_session(), "Should allow allocation after deallocation");
    
    let session4 = helper.allocate_session().expect("Should allocate after deallocation");
    assert_eq!(helper.allocated_count(), 3, "Should be back at limit");
    
    // Cleanup
    helper.deallocate_session(&session1).expect("Should deallocate session 1");
    helper.deallocate_session(&session3).expect("Should deallocate session 3");
    helper.deallocate_session(&session4).expect("Should deallocate session 4");
    
    assert_eq!(helper.allocated_count(), 0, "Should have no sessions after cleanup");
    assert!(helper.manager().can_create_session(), "Should allow allocation after full cleanup");
    
    println!("✅ Resource limit boundary conditions tests passed");
} 