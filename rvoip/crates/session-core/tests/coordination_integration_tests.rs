use rvoip_session_core::api::control::SessionControl;
//! Tests for Coordination Integration
//!
//! Integration tests that combine resources, priority, and groups coordination components.

mod common;

use std::time::Duration;
use rvoip_session_core::coordination::{
    resources::ResourceLimits,
    priority::Priority,
};
use rvoip_session_core::api::types::SessionId;
use common::*;

#[tokio::test]
async fn test_coordination_integration_basic() {
    println!("🧪 Testing basic coordination integration...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(10, 20);
    
    // Create a coordinated session with all components
    let session_id = helper.create_coordinated_session("conference", Priority::High)
        .expect("Should create coordinated session");
    
    // Verify all components are coordinated
    assert_eq!(helper.resource_helper().allocated_count(), 1, "Should have allocated 1 resource");
    assert_eq!(helper.priority_helper().get_priority(&session_id), Priority::High, "Should have High priority");
    assert!(helper.groups_helper().get_group_sessions("conference").contains(&session_id), "Should be in conference group");
    
    // Clean up
    helper.cleanup_coordinated_session(&session_id, "conference")
        .expect("Should cleanup coordinated session");
    
    assert_eq!(helper.resource_helper().allocated_count(), 0, "Should have no allocated resources");
    assert_eq!(helper.priority_helper().get_priority(&session_id), Priority::Normal, "Should return to Normal priority");
    assert!(!helper.groups_helper().get_group_sessions("conference").contains(&session_id), "Should not be in conference group");
    
    println!("✅ Basic coordination integration tests passed");
}

#[tokio::test]
async fn test_coordination_resource_limits_with_priorities() {
    println!("🧪 Testing coordination with resource limits and priorities...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(3, 6);
    let mut sessions = Vec::new();
    
    // Create sessions with different priorities
    let priorities = [Priority::Critical, Priority::High, Priority::Normal];
    
    for (i, &priority) in priorities.iter().enumerate() {
        let group_name = format!("group_{}", i);
        let session_id = helper.create_coordinated_session(&group_name, priority)
            .expect("Should create coordinated session");
        sessions.push((session_id, group_name, priority));
    }
    
    // Verify we're at the resource limit
    assert_eq!(helper.resource_helper().allocated_count(), 3, "Should be at resource limit");
    assert!(!helper.resource_helper().manager().can_create_session(), "Should not be able to create more sessions");
    
    // Try to create one more (should fail due to resource limit)
    let result = helper.create_coordinated_session("overflow", Priority::Low);
    assert!(result.is_err(), "Should fail to create session beyond resource limit");
    
    // Verify priorities are correctly assigned
    for (session_id, _, expected_priority) in &sessions {
        assert_eq!(helper.priority_helper().get_priority(session_id), *expected_priority,
                   "Session should have correct priority");
    }
    
    // Clean up one session and verify we can create another
    let (cleanup_session, cleanup_group, _) = &sessions[1];
    helper.cleanup_coordinated_session(cleanup_session, cleanup_group)
        .expect("Should cleanup session");
    
    assert!(helper.resource_helper().manager().can_create_session(), "Should be able to create session after cleanup");
    
    let new_session = helper.create_coordinated_session("new_group", Priority::High)
        .expect("Should create new session after cleanup");
    
    assert_eq!(helper.resource_helper().allocated_count(), 3, "Should be back at limit with new session");
    
    println!("✅ Coordination with resource limits and priorities tests passed");
}

#[tokio::test]
async fn test_coordination_complex_group_scenarios() {
    println!("🧪 Testing coordination complex group scenarios...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(20, 40);
    
    // Scenario 1: Conference call with priority escalation
    let mut conference_sessions = Vec::new();
    for i in 0..5 {
        let priority = if i == 0 { Priority::High } else { Priority::Normal };
        let session_id = helper.create_coordinated_session("conference", priority)
            .expect("Should create conference session");
        conference_sessions.push(session_id);
    }
    
    // Verify conference setup
    assert_eq!(helper.groups_helper().get_group_count("conference"), 5, "Conference should have 5 participants");
    assert_eq!(helper.priority_helper().get_priority(&conference_sessions[0]), Priority::High, "First participant should be high priority");
    
    // Scenario 2: Emergency call interrupts conference
    let emergency_session = helper.create_coordinated_session("emergency", Priority::Critical)
        .expect("Should create emergency session");
    
    // Emergency session should also be added to monitoring group
    helper.groups_helper_mut().add_to_group("monitoring", emergency_session.clone())
        .expect("Should add emergency to monitoring");
    
    // Verify emergency session is in both groups
    assert!(helper.groups_helper().get_group_sessions("emergency").contains(&emergency_session));
    assert!(helper.groups_helper().get_group_sessions("monitoring").contains(&emergency_session));
    assert_eq!(helper.priority_helper().get_priority(&emergency_session), Priority::Critical);
    
    // Scenario 3: Queue management
    let mut queue_sessions = Vec::new();
    for i in 0..3 {
        let session_id = helper.create_coordinated_session("queue", Priority::Low)
            .expect("Should create queued session");
        queue_sessions.push(session_id);
    }
    
    // Escalate one queued call
    helper.priority_helper_mut().set_priority(queue_sessions[1].clone(), Priority::High)
        .expect("Should escalate priority");
    
    // Move escalated call from queue to active
    helper.groups_helper_mut().remove_from_group("queue", &queue_sessions[1])
        .expect("Should remove from queue");
    helper.groups_helper_mut().add_to_group("active", queue_sessions[1].clone())
        .expect("Should add to active");
    
    // Verify final state
    assert_eq!(helper.groups_helper().get_group_count("conference"), 5);
    assert_eq!(helper.groups_helper().get_group_count("emergency"), 1);
    assert_eq!(helper.groups_helper().get_group_count("monitoring"), 1);
    assert_eq!(helper.groups_helper().get_group_count("queue"), 2);
    assert_eq!(helper.groups_helper().get_group_count("active"), 1);
    
    // Verify total resource usage
    assert_eq!(helper.resource_helper().allocated_count(), 9, "Should have 9 total allocated sessions");
    
    println!("✅ Coordination complex group scenarios tests passed");
}

#[tokio::test]
async fn test_coordination_priority_based_resource_management() {
    println!("🧪 Testing coordination priority-based resource management...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(5, 10);
    
    // Create sessions with different priorities
    let mut sessions = Vec::new();
    
    // Fill up resources with mixed priorities
    for i in 0..5 {
        let priority = match i % 4 {
            0 => Priority::Critical,
            1 => Priority::High,
            2 => Priority::Normal,
            _ => Priority::Low,
        };
        let group = format!("group_{}", i);
        let session_id = helper.create_coordinated_session(&group, priority)
            .expect("Should create session");
        sessions.push((session_id, group, priority));
    }
    
    // Verify we're at capacity
    assert_eq!(helper.resource_helper().allocated_count(), 5, "Should be at capacity");
    assert!(!helper.resource_helper().manager().can_create_session(), "Should not allow more sessions");
    
    // Test priority-based decisions by checking current state
    let mut priority_counts = [0; 4]; // Low, Normal, High, Critical
    for (session_id, _, _) in &sessions {
        match helper.priority_helper().get_priority(session_id) {
            Priority::Low => priority_counts[0] += 1,
            Priority::Normal => priority_counts[1] += 1,
            Priority::High => priority_counts[2] += 1,
            Priority::Critical => priority_counts[3] += 1,
        }
    }
    
    println!("Priority distribution: Low={}, Normal={}, High={}, Critical={}", 
             priority_counts[0], priority_counts[1], priority_counts[2], priority_counts[3]);
    
    // Simulate resource pressure - remove lowest priority session
    let mut lowest_priority_session = None;
    let mut lowest_priority = Priority::Critical;
    
    for (session_id, group, priority) in &sessions {
        if *priority <= lowest_priority {
            lowest_priority = *priority;
            lowest_priority_session = Some((session_id.clone(), group.clone()));
        }
    }
    
    if let Some((session_id, group)) = lowest_priority_session {
        helper.cleanup_coordinated_session(&session_id, &group)
            .expect("Should cleanup lowest priority session");
        
        // Now we should be able to add a new high-priority session
        let new_session = helper.create_coordinated_session("urgent", Priority::Critical)
            .expect("Should create urgent session");
        
        assert_eq!(helper.priority_helper().get_priority(&new_session), Priority::Critical);
        assert!(helper.groups_helper().get_group_sessions("urgent").contains(&new_session));
    }
    
    println!("✅ Coordination priority-based resource management tests passed");
}

#[tokio::test]
async fn test_coordination_concurrent_operations() {
    println!("🧪 Testing coordination concurrent operations...");
    
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinSet;
    
    let helper = Arc::new(Mutex::new(CoordinationIntegrationHelper::new_with_limits(50, 100)));
    let mut join_set = JoinSet::new();
    
    // Spawn concurrent coordination tasks
    for i in 0..10 {
        let helper_clone = helper.clone();
        let task_id = i;
        
        join_set.spawn(async move {
            let mut created_sessions = Vec::new();
            
            for j in 0..5 {
                if let Ok(mut h) = helper_clone.try_lock() {
                    let group_name = format!("task_{}_{}", task_id, j);
                    let priority = match (task_id + j) % 4 {
                        0 => Priority::Low,
                        1 => Priority::Normal,
                        2 => Priority::High,
                        _ => Priority::Critical,
                    };
                    
                    if let Ok(session_id) = h.create_coordinated_session(&group_name, priority) {
                        created_sessions.push((session_id, group_name));
                    }
                }
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
            
            (task_id, created_sessions.len())
        });
    }
    
    // Collect results
    let mut total_created = 0;
    while let Some(result) = join_set.join_next().await {
        if let Ok((task_id, created_count)) = result {
            println!("Task {} created {} coordinated sessions", task_id, created_count);
            total_created += created_count;
        }
    }
    
    println!("Total coordinated sessions created: {}", total_created);
    
    // Verify final state
    let final_helper = helper.lock().unwrap();
    assert!(total_created > 0, "Should have created some sessions");
    assert!(final_helper.resource_helper().allocated_count() <= 50, "Should not exceed resource limits");
    assert!(final_helper.verify_coordination_consistency(), "Should maintain coordination consistency");
    
    println!("✅ Coordination concurrent operations tests passed");
}

#[tokio::test]
async fn test_coordination_stress_integration() {
    println!("🧪 Testing coordination stress integration...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(100, 200);
    let config = CoordinationPerfTestConfig::fast();
    
    let priorities = CoordinationTestUtils::all_priorities();
    let groups = CoordinationTestUtils::test_group_names();
    
    // Run coordinated stress test
    let (completed, duration) = CoordinationTestUtils::run_coordination_stress_test(
        || {
            let priority = priorities[fastrand::usize(0..priorities.len())].clone();
            let group = groups[fastrand::usize(0..groups.len())];
            
            if helper.resource_helper().manager().can_create_session() {
                helper.create_coordinated_session(group, priority)?;
            }
            Ok(())
        },
        1000,
        config.test_duration
    );
    
    println!("Completed {} coordinated operations in {:?}", completed, duration);
    
    // Verify results
    assert!(completed > 0, "Should have completed some operations");
    assert!(duration <= config.test_duration * 2, "Should complete within reasonable time");
    
    // Verify coordination consistency
    assert!(helper.verify_coordination_consistency(), "Should maintain consistency under stress");
    
    // Verify resource limits respected
    assert!(helper.resource_helper().allocated_count() <= 100, "Should not exceed resource limits");
    
    // Verify groups and priorities are functioning
    let mut total_group_sessions = 0;
    for group_name in groups {
        total_group_sessions += helper.groups_helper().get_group_count(group_name);
    }
    assert!(total_group_sessions > 0, "Should have sessions in groups");
    
    println!("✅ Coordination stress integration tests passed");
}

#[tokio::test]
async fn test_coordination_lifecycle_integration() {
    println!("🧪 Testing coordination lifecycle integration...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(15, 30);
    
    // Phase 1: System startup - create initial sessions
    let mut active_sessions = Vec::new();
    for i in 0..5 {
        let session_id = helper.create_coordinated_session("startup", Priority::Normal)
            .expect("Should create startup session");
        active_sessions.push((session_id, "startup".to_string()));
    }
    
    println!("Phase 1: Created {} startup sessions", active_sessions.len());
    
    // Phase 2: Business hours - add conference and queue sessions
    for i in 0..3 {
        let session_id = helper.create_coordinated_session("conference", Priority::High)
            .expect("Should create conference session");
        active_sessions.push((session_id, "conference".to_string()));
    }
    
    for i in 0..4 {
        let session_id = helper.create_coordinated_session("queue", Priority::Low)
            .expect("Should create queue session");
        active_sessions.push((session_id, "queue".to_string()));
    }
    
    println!("Phase 2: Added conference and queue sessions, total: {}", active_sessions.len());
    
    // Phase 3: Emergency situation - add critical sessions
        let emergency_session = helper.create_coordinated_session("emergency", Priority::Critical)
        .expect("Should create emergency session");
    active_sessions.push((emergency_session.clone(), "emergency".to_string()));

    // Add emergency session to monitoring group as well
    helper.groups_helper_mut().add_to_group("monitoring", emergency_session.clone())
        .expect("Should add to monitoring");
    
    println!("Phase 3: Added emergency session");
    
    // Phase 4: Load balancing - escalate some queue sessions
    let queue_sessions: Vec<_> = helper.groups_helper().get_group_sessions("queue");
    if queue_sessions.len() >= 2 {
        helper.priority_helper_mut().set_priority(queue_sessions[0].clone(), Priority::High)
            .expect("Should escalate priority");
        
        // Move to active group
        helper.groups_helper_mut().remove_from_group("queue", &queue_sessions[0])
            .expect("Should remove from queue");
        helper.groups_helper_mut().add_to_group("active", queue_sessions[0].clone())
            .expect("Should add to active");
    }
    
    println!("Phase 4: Performed load balancing");
    
    // Phase 5: System shutdown - clean up sessions by priority
    let mut cleanup_order = Vec::new();
    
    // First cleanup Low priority sessions
    for (session_id, group) in &active_sessions {
        if helper.priority_helper().get_priority(session_id) == Priority::Low {
            cleanup_order.push((session_id.clone(), group.clone()));
        }
    }
    
    // Then Normal priority
    for (session_id, group) in &active_sessions {
        if helper.priority_helper().get_priority(session_id) == Priority::Normal {
            cleanup_order.push((session_id.clone(), group.clone()));
        }
    }
    
    // Clean up in priority order
    for (session_id, group) in cleanup_order {
        if group == "emergency" {
            // Remove from monitoring group too
            helper.groups_helper_mut().remove_from_group("monitoring", &session_id)
                .expect("Should remove from monitoring");
        }
        helper.cleanup_coordinated_session(&session_id, &group)
            .expect("Should cleanup session");
    }
    
    println!("Phase 5: Cleaned up low and normal priority sessions");
    
    // Verify final state - should have High and Critical priority sessions left
    let remaining_count = helper.resource_helper().allocated_count();
    println!("Remaining sessions after cleanup: {}", remaining_count);
    
    // Verify consistency throughout lifecycle
    assert!(helper.verify_coordination_consistency(), "Should maintain consistency throughout lifecycle");
    
    println!("✅ Coordination lifecycle integration tests passed");
}

#[tokio::test]
async fn test_coordination_error_handling_integration() {
    println!("🧪 Testing coordination error handling integration...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(2, 4);
    
    // Fill up resources
    let session1 = helper.create_coordinated_session("group1", Priority::Normal)
        .expect("Should create first session");
    let session2 = helper.create_coordinated_session("group2", Priority::High)
        .expect("Should create second session");
    
    // Try to exceed resource limit
    let result = helper.create_coordinated_session("overflow", Priority::Critical);
    assert!(result.is_err(), "Should fail when resources exhausted");
    
    // Verify system state is consistent after error
    assert_eq!(helper.resource_helper().allocated_count(), 2, "Should have 2 allocated resources");
    assert!(helper.verify_coordination_consistency(), "Should maintain consistency after error");
    
    // Test partial cleanup error handling
    let fake_session = SessionId::new();
    let result = helper.cleanup_coordinated_session(&fake_session, "nonexistent");
    assert!(result.is_err(), "Should fail to cleanup non-existent session");
    
    // Verify real sessions are unaffected
    assert_eq!(helper.resource_helper().allocated_count(), 2, "Real sessions should be unaffected");
    assert_eq!(helper.priority_helper().get_priority(&session1), Priority::Normal);
    assert_eq!(helper.priority_helper().get_priority(&session2), Priority::High);
    
    // Test successful cleanup after errors
    helper.cleanup_coordinated_session(&session1, "group1")
        .expect("Should cleanup real session after errors");
    
    assert_eq!(helper.resource_helper().allocated_count(), 1, "Should have 1 session after cleanup");
    assert!(helper.verify_coordination_consistency(), "Should maintain consistency after recovery");
    
    println!("✅ Coordination error handling integration tests passed");
}

#[tokio::test]
async fn test_coordination_performance_integration() {
    println!("🧪 Testing coordination performance integration...");
    
    let mut helper = CoordinationIntegrationHelper::new_with_limits(1000, 2000);
    
    // Test coordinated session creation performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        let mut sessions = Vec::new();
        for i in 0..100 {
            let priority = match i % 4 {
                0 => Priority::Low,
                1 => Priority::Normal,
                2 => Priority::High,
                _ => Priority::Critical,
            };
            let group = format!("perf_group_{}", i % 10);
            
            if let Ok(session_id) = helper.create_coordinated_session(&group, priority) {
                sessions.push(session_id);
            }
        }
        sessions
    });
    
    assert!(result.len() > 0, "Should create some coordinated sessions");
    println!("Created {} coordinated sessions in {:?}", result.len(), duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(200),
        "coordinated_session_creation_performance"
    );
    
    // Test coordinated lookup performance
    let (lookup_result, lookup_duration) = CoordinationTestUtils::measure_operation_performance(|| {
        let mut lookups = 0;
        for session_id in &result {
            // Test all coordination aspects
            let _ = helper.priority_helper().get_priority(session_id);
            lookups += 1;
            
            // Test group membership
            for i in 0..10 {
                let group = format!("perf_group_{}", i);
                let _ = helper.groups_helper().get_group_sessions(&group);
            }
        }
        lookups
    });
    
    assert_eq!(lookup_result, result.len(), "Should perform lookups for all sessions");
    println!("Performed {} coordination lookups in {:?}", lookup_result, lookup_duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        lookup_duration,
        Duration::from_millis(100),
        "coordination_lookup_performance"
    );
    
    // Test coordinated cleanup performance
    let (cleanup_result, cleanup_duration) = CoordinationTestUtils::measure_operation_performance(|| {
        let mut cleaned = 0;
        for (i, session_id) in result.iter().enumerate() {
            let group = format!("perf_group_{}", i % 10);
            if helper.cleanup_coordinated_session(session_id, &group).is_ok() {
                cleaned += 1;
            }
        }
        cleaned
    });
    
    assert!(cleanup_result > 0, "Should cleanup some sessions");
    println!("Cleaned up {} coordinated sessions in {:?}", cleanup_result, cleanup_duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        cleanup_duration,
        Duration::from_millis(200),
        "coordinated_cleanup_performance"
    );
    
    // Verify final consistency
    assert!(helper.verify_coordination_consistency(), "Should maintain consistency after performance test");
    
    println!("✅ Coordination performance integration tests passed");
} 