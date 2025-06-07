//! Tests for Coordination Priority Management
//!
//! Tests for session priority assignment and management in session coordination.

mod common;

use std::time::Duration;
use rvoip_session_core::coordination::priority::{PriorityManager, Priority};
use rvoip_session_core::api::types::SessionId;
use common::*;

#[tokio::test]
async fn test_priority_manager_creation() {
    println!("🧪 Testing priority manager creation...");
    
    // Test default creation
    let manager = PriorityManager::new();
    let test_session = SessionId::new();
    
    // Default priority should be Normal
    assert_eq!(manager.get_priority(&test_session), Priority::Normal, "Default priority should be Normal");
    
    println!("✅ Priority manager creation tests passed");
}

#[tokio::test]
async fn test_basic_priority_assignment() {
    println!("🧪 Testing basic priority assignment...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create test sessions and assign different priorities
    let low_session = helper.create_session_with_priority(Priority::Low);
    let normal_session = helper.create_session_with_priority(Priority::Normal);
    let high_session = helper.create_session_with_priority(Priority::High);
    let critical_session = helper.create_session_with_priority(Priority::Critical);
    
    // Verify priorities are correctly assigned
    assert_eq!(helper.get_priority(&low_session), Priority::Low, "Low priority should be assigned");
    assert_eq!(helper.get_priority(&normal_session), Priority::Normal, "Normal priority should be assigned");
    assert_eq!(helper.get_priority(&high_session), Priority::High, "High priority should be assigned");
    assert_eq!(helper.get_priority(&critical_session), Priority::Critical, "Critical priority should be assigned");
    
    println!("✅ Basic priority assignment tests passed");
}

#[tokio::test]
async fn test_priority_ordering() {
    println!("🧪 Testing priority ordering...");
    
    let priorities = CoordinationTestUtils::all_priorities();
    
    // Verify the expected ordering
    assert_eq!(priorities[0], Priority::Low, "First priority should be Low");
    assert_eq!(priorities[1], Priority::Normal, "Second priority should be Normal");
    assert_eq!(priorities[2], Priority::High, "Third priority should be High");
    assert_eq!(priorities[3], Priority::Critical, "Fourth priority should be Critical");
    
    // Test ordering comparisons
    assert!(Priority::Low < Priority::Normal, "Low should be less than Normal");
    assert!(Priority::Normal < Priority::High, "Normal should be less than High");
    assert!(Priority::High < Priority::Critical, "High should be less than Critical");
    
    // Test validation utility
    assert!(CoordinationTestUtils::validate_priority_order(&priorities), "Priorities should be in correct order");
    
    println!("✅ Priority ordering tests passed");
}

#[tokio::test]
async fn test_priority_update_and_removal() {
    println!("🧪 Testing priority update and removal...");
    
    let mut helper = PriorityTestHelper::new();
    let session = helper.create_test_session();
    
    // Set initial priority
    helper.set_priority(session.clone(), Priority::Low).expect("Should set initial priority");
    assert_eq!(helper.get_priority(&session), Priority::Low, "Initial priority should be Low");
    
    // Update priority
    helper.set_priority(session.clone(), Priority::High).expect("Should update priority");
    assert_eq!(helper.get_priority(&session), Priority::High, "Updated priority should be High");
    
    // Update to Critical
    helper.set_priority(session.clone(), Priority::Critical).expect("Should update to Critical");
    assert_eq!(helper.get_priority(&session), Priority::Critical, "Updated priority should be Critical");
    
    // Remove session
    helper.cleanup_session(&session).expect("Should remove session");
    
    // After removal, should default to Normal
    assert_eq!(helper.get_priority(&session), Priority::Normal, "Removed session should default to Normal");
    
    println!("✅ Priority update and removal tests passed");
}

#[tokio::test]
async fn test_priority_multiple_sessions() {
    println!("🧪 Testing priority with multiple sessions...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create multiple sessions with different priorities
    let sessions = helper.create_priority_test_set();
    
    // Verify each session has the correct priority
    assert_eq!(helper.get_priority(sessions.get(&Priority::Low).unwrap()), Priority::Low);
    assert_eq!(helper.get_priority(sessions.get(&Priority::Normal).unwrap()), Priority::Normal);
    assert_eq!(helper.get_priority(sessions.get(&Priority::High).unwrap()), Priority::High);
    assert_eq!(helper.get_priority(sessions.get(&Priority::Critical).unwrap()), Priority::Critical);
    
    // Create a sorted list by priority
    let session_list: Vec<SessionId> = vec![
        sessions.get(&Priority::Low).unwrap().clone(),
        sessions.get(&Priority::Normal).unwrap().clone(),
        sessions.get(&Priority::High).unwrap().clone(),
        sessions.get(&Priority::Critical).unwrap().clone(),
    ];
    
    // Verify ordering
    assert!(helper.verify_priority_ordering(&session_list), "Sessions should be in priority order");
    
    println!("✅ Multiple sessions priority tests passed");
}

#[tokio::test]
async fn test_priority_edge_cases() {
    println!("🧪 Testing priority edge cases...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Test non-existent session (should return Normal)
    let non_existent = SessionId::new();
    assert_eq!(helper.get_priority(&non_existent), Priority::Normal, "Non-existent session should return Normal");
    
    // Test setting priority on non-existent session
    let result = helper.set_priority(non_existent.clone(), Priority::High);
    assert!(result.is_ok(), "Should be able to set priority on non-existent session");
    assert_eq!(helper.get_priority(&non_existent), Priority::High, "Priority should be set for new session");
    
    // Test removing non-existent session
    let another_non_existent = SessionId::new();
    let result = helper.cleanup_session(&another_non_existent);
    assert!(result.is_ok(), "Should be able to remove non-existent session without error");
    
    // Test setting same priority multiple times
    let session = helper.create_test_session();
    helper.set_priority(session.clone(), Priority::Critical).expect("Should set priority");
    helper.set_priority(session.clone(), Priority::Critical).expect("Should set same priority again");
    assert_eq!(helper.get_priority(&session), Priority::Critical, "Priority should remain Critical");
    
    println!("✅ Priority edge cases tests passed");
}

#[tokio::test]
async fn test_priority_concurrent_operations() {
    println!("🧪 Testing priority concurrent operations...");
    
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinSet;
    
    let helper = Arc::new(Mutex::new(PriorityTestHelper::new()));
    let mut join_set = JoinSet::new();
    
    // Pre-create sessions to avoid conflicts
    let sessions = {
        let mut h = helper.lock().unwrap();
        h.create_test_sessions(20)
    };
    
    // Spawn concurrent priority assignment tasks
    for i in 0..4 {
        let helper_clone = helper.clone();
        let sessions_chunk = sessions[i*5..(i+1)*5].to_vec();
        let priority = match i {
            0 => Priority::Low,
            1 => Priority::Normal, 
            2 => Priority::High,
            _ => Priority::Critical,
        };
        
        join_set.spawn(async move {
            let mut assignments = 0;
            for session in sessions_chunk {
                if let Ok(mut h) = helper_clone.try_lock() {
                    if h.set_priority(session, priority.clone()).is_ok() {
                        assignments += 1;
                    }
                }
                tokio::time::sleep(Duration::from_micros(50)).await;
            }
            (i, assignments, priority)
        });
    }
    
    // Collect results
    let mut total_assignments = 0;
    while let Some(result) = join_set.join_next().await {
        if let Ok((task_id, assignments, priority)) = result {
            println!("Task {} ({:?}) made {} assignments", task_id, priority, assignments);
            total_assignments += assignments;
        }
    }
    
    println!("Total priority assignments: {}", total_assignments);
    
    // Verify final state
    let final_helper = helper.lock().unwrap();
    assert_eq!(final_helper.test_session_count(), 20, "Should have 20 sessions");
    
    // Verify that priorities were assigned (can't guarantee specific assignments due to concurrency)
    assert!(total_assignments > 0, "Should have made some priority assignments");
    
    println!("✅ Priority concurrent operations tests passed");
}

#[tokio::test]
async fn test_priority_performance() {
    println!("🧪 Testing priority operation performance...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create many sessions for performance testing
    let sessions = helper.create_test_sessions(1000);
    
    // Test bulk priority assignment performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        for (i, session) in sessions.iter().enumerate() {
            let priority = match i % 4 {
                0 => Priority::Low,
                1 => Priority::Normal,
                2 => Priority::High,
                _ => Priority::Critical,
            };
            helper.set_priority(session.clone(), priority)?;
        }
        Ok::<(), rvoip_session_core::SessionError>(())
    });
    
    assert!(result.is_ok(), "Should be able to assign priorities to all sessions");
    println!("Assigned priorities to 1000 sessions in {:?}", duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(100),
        "priority_assignment_performance"
    );
    
    // Test bulk priority retrieval performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        let mut priorities = Vec::new();
        for session in &sessions {
            priorities.push(helper.get_priority(session));
        }
        priorities
    });
    
    assert_eq!(result.len(), 1000, "Should retrieve 1000 priorities");
    println!("Retrieved 1000 priorities in {:?}", duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(50),
        "priority_retrieval_performance"
    );
    
    println!("✅ Priority performance tests passed");
}

#[tokio::test]
async fn test_priority_stress_operations() {
    println!("🧪 Testing priority stress operations...");
    
    let mut helper = PriorityTestHelper::new();
    let config = CoordinationPerfTestConfig::fast();
    
    // Run stress test with priority operations
    let (completed, duration) = CoordinationTestUtils::run_coordination_stress_test(
        || {
            let session = helper.create_test_session();
            let priority = match fastrand::usize(0..4) {
                0 => Priority::Low,
                1 => Priority::Normal,
                2 => Priority::High,
                _ => Priority::Critical,
            };
            helper.set_priority(session, priority)?;
            Ok::<(), rvoip_session_core::SessionError>(())
        },
        1000,
        config.test_duration
    );
    
    println!("Completed {} priority operations in {:?}", completed, duration);
    
    // Verify results
    assert!(completed > 0, "Should have completed some operations");
    assert!(duration <= config.test_duration * 2, "Should complete within reasonable time");
    
    // Verify final state consistency
    assert!(helper.test_session_count() > 0, "Should have created some sessions");
    
    println!("✅ Priority stress operations tests passed");
}

#[tokio::test]
async fn test_priority_sorting_and_filtering() {
    println!("🧪 Testing priority sorting and filtering...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create sessions with mixed priorities
    let mut mixed_sessions = Vec::new();
    let priorities = [Priority::Critical, Priority::Low, Priority::High, Priority::Normal, Priority::Critical, Priority::Low];
    
    for priority in priorities {
        let session = helper.create_session_with_priority(priority);
        mixed_sessions.push(session);
    }
    
    // Get priorities for sorting test
    let mut session_priorities: Vec<(SessionId, Priority)> = mixed_sessions
        .iter()
        .map(|session| (session.clone(), helper.get_priority(session)))
        .collect();
    
    // Sort by priority
    session_priorities.sort_by(|a, b| a.1.cmp(&b.1));
    
    // Verify sorted order
    let sorted_priorities: Vec<Priority> = session_priorities.iter().map(|(_, p)| p.clone()).collect();
    assert!(CoordinationTestUtils::validate_priority_order(&sorted_priorities), "Sorted priorities should be in order");
    
    // Verify we have the expected distribution
    let low_count = sorted_priorities.iter().filter(|&p| *p == Priority::Low).count();
    let normal_count = sorted_priorities.iter().filter(|&p| *p == Priority::Normal).count();
    let high_count = sorted_priorities.iter().filter(|&p| *p == Priority::High).count();
    let critical_count = sorted_priorities.iter().filter(|&p| *p == Priority::Critical).count();
    
    assert_eq!(low_count, 2, "Should have 2 Low priority sessions");
    assert_eq!(normal_count, 1, "Should have 1 Normal priority session");
    assert_eq!(high_count, 1, "Should have 1 High priority session");
    assert_eq!(critical_count, 2, "Should have 2 Critical priority sessions");
    
    println!("✅ Priority sorting and filtering tests passed");
}

#[tokio::test]
async fn test_priority_lifecycle_integration() {
    println!("🧪 Testing priority lifecycle integration...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create session and set initial priority
    let session = helper.create_test_session();
    helper.set_priority(session.clone(), Priority::Low).expect("Should set initial priority");
    
    // Simulate priority escalation during session lifecycle
    assert_eq!(helper.get_priority(&session), Priority::Low, "Should start with Low priority");
    
    // Escalate to Normal
    helper.set_priority(session.clone(), Priority::Normal).expect("Should escalate to Normal");
    assert_eq!(helper.get_priority(&session), Priority::Normal, "Should be Normal priority");
    
    // Escalate to High (urgent situation)
    helper.set_priority(session.clone(), Priority::High).expect("Should escalate to High");
    assert_eq!(helper.get_priority(&session), Priority::High, "Should be High priority");
    
    // Escalate to Critical (emergency)
    helper.set_priority(session.clone(), Priority::Critical).expect("Should escalate to Critical");
    assert_eq!(helper.get_priority(&session), Priority::Critical, "Should be Critical priority");
    
    // End session lifecycle
    helper.cleanup_session(&session).expect("Should cleanup session");
    assert_eq!(helper.get_priority(&session), Priority::Normal, "Should return to Normal after cleanup");
    
    println!("✅ Priority lifecycle integration tests passed");
}

#[tokio::test]
async fn test_priority_memory_efficiency() {
    println!("🧪 Testing priority memory efficiency...");
    
    let mut helper = PriorityTestHelper::new();
    
    // Create many sessions to test memory usage
    let session_count = 5000;
    let sessions = helper.create_test_sessions(session_count);
    
    // Assign priorities to all sessions
    for (i, session) in sessions.iter().enumerate() {
        let priority = match i % 4 {
            0 => Priority::Low,
            1 => Priority::Normal,
            2 => Priority::High,
            _ => Priority::Critical,
        };
        helper.set_priority(session.clone(), priority).expect("Should set priority");
    }
    
    // Verify all priorities are correctly set
    let mut priority_counts = [0; 4];
    for session in &sessions {
        match helper.get_priority(session) {
            Priority::Low => priority_counts[0] += 1,
            Priority::Normal => priority_counts[1] += 1,
            Priority::High => priority_counts[2] += 1,
            Priority::Critical => priority_counts[3] += 1,
        }
    }
    
    // Verify distribution
    assert_eq!(priority_counts[0], 1250, "Should have 1250 Low priority sessions");
    assert_eq!(priority_counts[1], 1250, "Should have 1250 Normal priority sessions");
    assert_eq!(priority_counts[2], 1250, "Should have 1250 High priority sessions");
    assert_eq!(priority_counts[3], 1250, "Should have 1250 Critical priority sessions");
    
    // Test cleanup performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        for session in &sessions {
            helper.cleanup_session(session)?;
        }
        Ok::<(), rvoip_session_core::SessionError>(())
    });
    
    assert!(result.is_ok(), "Should be able to cleanup all sessions");
    println!("Cleaned up {} sessions in {:?}", session_count, duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(500),
        "priority_cleanup_performance"
    );
    
    println!("✅ Priority memory efficiency tests passed");
}

#[tokio::test]
async fn test_priority_enum_properties() {
    println!("🧪 Testing priority enum properties...");
    
    // Test numeric values
    assert_eq!(Priority::Low as u8, 1, "Low priority should have value 1");
    assert_eq!(Priority::Normal as u8, 2, "Normal priority should have value 2");
    assert_eq!(Priority::High as u8, 3, "High priority should have value 3");
    assert_eq!(Priority::Critical as u8, 4, "Critical priority should have value 4");
    
    // Test ordering traits
    assert!(Priority::Low < Priority::Normal);
    assert!(Priority::Normal < Priority::High);
    assert!(Priority::High < Priority::Critical);
    
    // Test equality
    assert_eq!(Priority::Low, Priority::Low);
    assert_ne!(Priority::Low, Priority::High);
    
    // Test cloning
    let priority = Priority::High;
    let cloned = priority.clone();
    assert_eq!(priority, cloned, "Cloned priority should be equal");
    
    // Test debug formatting
    let debug_str = format!("{:?}", Priority::Critical);
    assert!(debug_str.contains("Critical"), "Debug format should contain 'Critical'");
    
    println!("✅ Priority enum properties tests passed");
} 