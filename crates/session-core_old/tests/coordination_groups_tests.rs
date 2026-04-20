use rvoip_session_core::api::control::SessionControl;
// Tests for Coordination Session Groups
//
// Tests for session grouping and group management in session coordination.

mod common;

use std::time::Duration;
use rvoip_session_core::coordination::groups::SessionGroups;
use rvoip_session_core::api::types::SessionId;
use common::*;

#[tokio::test]
async fn test_session_groups_creation() {
    println!("🧪 Testing session groups creation...");
    
    // Test default creation
    let groups = SessionGroups::new();
    
    // Test getting empty group
    let sessions = groups.get_group_sessions("non_existent_group");
    assert!(sessions.is_empty(), "Non-existent group should return empty vector");
    
    println!("✅ Session groups creation tests passed");
}

#[tokio::test]
async fn test_basic_group_operations() {
    println!("🧪 Testing basic group operations...");
    
    let mut helper = GroupsTestHelper::new();
    
    // Create test sessions
    let session1 = helper.create_test_session();
    let session2 = helper.create_test_session();
    let session3 = helper.create_test_session();
    
    // Add sessions to a group
    helper.add_to_group("conference", session1.clone()).expect("Should add session1 to conference");
    helper.add_to_group("conference", session2.clone()).expect("Should add session2 to conference");
    helper.add_to_group("queue", session3.clone()).expect("Should add session3 to queue");
    
    // Verify group membership
    let conference_sessions = helper.get_group_sessions("conference");
    assert_eq!(conference_sessions.len(), 2, "Conference group should have 2 sessions");
    assert!(conference_sessions.contains(&session1), "Conference should contain session1");
    assert!(conference_sessions.contains(&session2), "Conference should contain session2");
    
    let queue_sessions = helper.get_group_sessions("queue");
    assert_eq!(queue_sessions.len(), 1, "Queue group should have 1 session");
    assert!(queue_sessions.contains(&session3), "Queue should contain session3");
    
    println!("✅ Basic group operations tests passed");
}

#[tokio::test]
async fn test_group_session_removal() {
    println!("🧪 Testing group session removal...");
    
    let mut helper = GroupsTestHelper::new();
    let sessions = helper.create_test_group("meeting", 3);
    
    // Verify initial state
    assert!(helper.verify_group_membership("meeting", &sessions), "All sessions should be in meeting group");
    assert_eq!(helper.get_group_count("meeting"), 3, "Meeting group should have 3 sessions");
    
    // Remove one session
    helper.remove_from_group("meeting", &sessions[1]).expect("Should remove session from group");
    assert_eq!(helper.get_group_count("meeting"), 2, "Meeting group should have 2 sessions after removal");
    
    let remaining_sessions = helper.get_group_sessions("meeting");
    assert!(!remaining_sessions.contains(&sessions[1]), "Removed session should not be in group");
    assert!(remaining_sessions.contains(&sessions[0]), "Other sessions should remain");
    assert!(remaining_sessions.contains(&sessions[2]), "Other sessions should remain");
    
    // Remove all sessions
    helper.remove_from_group("meeting", &sessions[0]).expect("Should remove session");
    helper.remove_from_group("meeting", &sessions[2]).expect("Should remove session");
    assert_eq!(helper.get_group_count("meeting"), 0, "Meeting group should be empty");
    
    println!("✅ Group session removal tests passed");
}

#[tokio::test]
async fn test_multiple_groups_management() {
    println!("🧪 Testing multiple groups management...");
    
    let mut helper = GroupsTestHelper::new();
    let group_names = CoordinationTestUtils::test_group_names();
    
    // Create sessions for each group
    let mut all_sessions = Vec::new();
    for (i, &group_name) in group_names.iter().enumerate() {
        let sessions = helper.create_test_group(group_name, i + 2); // 2, 3, 4, 5 sessions per group
        all_sessions.extend(sessions);
    }
    
    // Verify each group has correct number of sessions
    assert_eq!(helper.get_group_count("conference"), 2, "Conference should have 2 sessions");
    assert_eq!(helper.get_group_count("queue"), 3, "Queue should have 3 sessions");
    assert_eq!(helper.get_group_count("emergency"), 4, "Emergency should have 4 sessions");
    assert_eq!(helper.get_group_count("test_group"), 5, "Test_group should have 5 sessions");
    
    // Test that sessions are isolated between groups
    let conference_sessions = helper.get_group_sessions("conference");
    let queue_sessions = helper.get_group_sessions("queue");
    
    // Verify no overlap
    for session in &conference_sessions {
        assert!(!queue_sessions.contains(session), "Conference and queue sessions should not overlap");
    }
    
    println!("✅ Multiple groups management tests passed");
}

#[tokio::test]
async fn test_session_in_multiple_groups() {
    println!("🧪 Testing session in multiple groups...");
    
    let mut helper = GroupsTestHelper::new();
    let session = helper.create_test_session();
    
    // Add same session to multiple groups
    helper.add_to_group("conference", session.clone()).expect("Should add to conference");
    helper.add_to_group("emergency", session.clone()).expect("Should add to emergency");
    helper.add_to_group("monitor", session.clone()).expect("Should add to monitor");
    
    // Verify session appears in all groups
    assert!(helper.get_group_sessions("conference").contains(&session), "Session should be in conference");
    assert!(helper.get_group_sessions("emergency").contains(&session), "Session should be in emergency");
    assert!(helper.get_group_sessions("monitor").contains(&session), "Session should be in monitor");
    
    // Remove from one group, should remain in others
    helper.remove_from_group("conference", &session).expect("Should remove from conference");
    assert!(!helper.get_group_sessions("conference").contains(&session), "Session should not be in conference");
    assert!(helper.get_group_sessions("emergency").contains(&session), "Session should still be in emergency");
    assert!(helper.get_group_sessions("monitor").contains(&session), "Session should still be in monitor");
    
    println!("✅ Session in multiple groups tests passed");
}

#[tokio::test]
async fn test_group_edge_cases() {
    println!("🧪 Testing group edge cases...");
    
    let mut helper = GroupsTestHelper::new();
    
    // Test adding same session to same group multiple times
    let session = helper.create_test_session();
    helper.add_to_group("test", session.clone()).expect("Should add session first time");
    helper.add_to_group("test", session.clone()).expect("Should add session second time");
    helper.add_to_group("test", session.clone()).expect("Should add session third time");
    
    // Should have multiple entries (current implementation allows duplicates)
    let sessions = helper.get_group_sessions("test");
    assert!(sessions.len() >= 1, "Should have at least one entry");
    
    // Test removing non-existent session from group
    let non_existent = SessionId::new();
    let result = helper.remove_from_group("test", &non_existent);
    assert!(result.is_ok(), "Should handle removing non-existent session gracefully");
    
    // Test removing from non-existent group
    let result = helper.remove_from_group("non_existent", &session);
    assert!(result.is_ok(), "Should handle removing from non-existent group gracefully");
    
    // Test empty group name
    let result = helper.add_to_group("", session.clone());
    assert!(result.is_ok(), "Should handle empty group name");
    
    println!("✅ Group edge cases tests passed");
}

#[tokio::test]
async fn test_group_performance() {
    println!("🧪 Testing group operation performance...");
    
    let mut helper = GroupsTestHelper::new();
    let sessions = helper.create_test_sessions(1000);
    
    // Test bulk group assignment performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        for (i, session) in sessions.iter().enumerate() {
            let group_name = match i % 4 {
                0 => "group_a",
                1 => "group_b", 
                2 => "group_c",
                _ => "group_d",
            };
            helper.add_to_group(group_name, session.clone())?;
        }
        Ok::<(), rvoip_session_core::SessionError>(())
    });
    
    assert!(result.is_ok(), "Should be able to add all sessions to groups");
    println!("Added 1000 sessions to groups in {:?}", duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(100),
        "group_assignment_performance"
    );
    
    // Test bulk group retrieval performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        let mut total_sessions = 0;
        for group_name in ["group_a", "group_b", "group_c", "group_d"] {
            total_sessions += helper.get_group_sessions(group_name).len();
        }
        total_sessions
    });
    
    assert_eq!(result, 1000, "Should retrieve all 1000 sessions");
    println!("Retrieved all group sessions in {:?}", duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_millis(50),
        "group_retrieval_performance"
    );
    
    println!("✅ Group performance tests passed");
}

#[tokio::test]
async fn test_group_concurrent_operations() {
    println!("🧪 Testing group concurrent operations...");
    
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinSet;
    
    let helper = Arc::new(Mutex::new(GroupsTestHelper::new()));
    let mut join_set = JoinSet::new();
    
    // Pre-create sessions to avoid conflicts
    let sessions = {
        let mut h = helper.lock().unwrap();
        h.create_test_sessions(40)
    };
    
    // Spawn concurrent group assignment tasks
    for i in 0..4 {
        let helper_clone = helper.clone();
        let sessions_chunk = sessions[i*10..(i+1)*10].to_vec();
        let group_name = match i {
            0 => "concurrent_a",
            1 => "concurrent_b",
            2 => "concurrent_c",
            _ => "concurrent_d",
        };
        
        join_set.spawn(async move {
            let mut assignments = 0;
            for session in sessions_chunk {
                if let Ok(mut h) = helper_clone.try_lock() {
                    if h.add_to_group(group_name, session).is_ok() {
                        assignments += 1;
                    }
                }
                tokio::time::sleep(Duration::from_micros(50)).await;
            }
            (i, assignments, group_name)
        });
    }
    
    // Collect results
    let mut total_assignments = 0;
    while let Some(result) = join_set.join_next().await {
        if let Ok((task_id, assignments, group_name)) = result {
            println!("Task {} ({}) made {} assignments", task_id, group_name, assignments);
            total_assignments += assignments;
        }
    }
    
    println!("Total group assignments: {}", total_assignments);
    
    // Verify final state
    let final_helper = helper.lock().unwrap();
    assert_eq!(final_helper.test_session_count(), 40, "Should have 40 sessions");
    assert!(total_assignments > 0, "Should have made some assignments");
    
    println!("✅ Group concurrent operations tests passed");
}

#[tokio::test]
async fn test_group_stress_operations() {
    println!("🧪 Testing group stress operations...");
    
    let mut helper = GroupsTestHelper::new();
    let config = CoordinationPerfTestConfig::fast();
    let group_names = ["stress_a", "stress_b", "stress_c", "stress_d"];
    
    // Run stress test with group operations
    let (completed, duration) = CoordinationTestUtils::run_coordination_stress_test(
        || {
            let session = helper.create_test_session();
            let group_name = group_names[fastrand::usize(0..group_names.len())];
            helper.add_to_group(group_name, session)?;
            Ok::<(), rvoip_session_core::SessionError>(())
        },
        1000,
        config.test_duration
    );
    
    println!("Completed {} group operations in {:?}", completed, duration);
    
    // Verify results
    assert!(completed > 0, "Should have completed some operations");
    assert!(duration <= config.test_duration * 2, "Should complete within reasonable time");
    
    // Verify final state consistency
    assert!(helper.test_session_count() > 0, "Should have created some sessions");
    
    // Check that groups were created
    let mut total_group_sessions = 0;
    for group_name in group_names {
        total_group_sessions += helper.get_group_count(group_name);
    }
    assert!(total_group_sessions > 0, "Should have sessions in groups");
    
    println!("✅ Group stress operations tests passed");
}

#[tokio::test]
async fn test_group_memory_efficiency() {
    println!("🧪 Testing group memory efficiency...");
    
    let mut helper = GroupsTestHelper::new();
    let session_count = 5000;
    let group_count = 100;
    
    // Create many sessions and distribute across many groups
    let sessions = helper.create_test_sessions(session_count);
    
    for (i, session) in sessions.iter().enumerate() {
        let group_name = format!("group_{}", i % group_count);
        helper.add_to_group(&group_name, session.clone()).expect("Should add to group");
    }
    
    // Verify distribution
    let sessions_per_group = session_count / group_count;
    for i in 0..group_count {
        let group_name = format!("group_{}", i);
        let group_size = helper.get_group_count(&group_name);
        // Allow some variance due to integer division
        assert!(group_size >= sessions_per_group - 1 && group_size <= sessions_per_group + 1,
                "Group {} should have ~{} sessions, got {}", group_name, sessions_per_group, group_size);
    }
    
    // Test cleanup performance
    let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
        // Remove all sessions from all groups
        for session in &sessions {
            for i in 0..group_count {
                let group_name = format!("group_{}", i);
                helper.remove_from_group(&group_name, session)?;
            }
        }
        Ok::<(), rvoip_session_core::SessionError>(())
    });
    
    assert!(result.is_ok(), "Should be able to cleanup all sessions");
    println!("Cleaned up {} sessions from {} groups in {:?}", session_count, group_count, duration);
    
    CoordinationTestUtils::assert_performance_acceptable(
        duration,
        Duration::from_secs(2),
        "group_cleanup_performance"
    );
    
    println!("✅ Group memory efficiency tests passed");
}

#[tokio::test]
async fn test_group_lifecycle_scenarios() {
    println!("🧪 Testing group lifecycle scenarios...");
    
    let mut helper = GroupsTestHelper::new();
    
    // Scenario 1: Conference call lifecycle
    let conference_sessions = helper.create_test_group("conference_call", 5);
    assert_eq!(helper.get_group_count("conference_call"), 5, "Conference should start with 5 sessions");
    
    // Someone leaves the conference
    helper.remove_from_group("conference_call", &conference_sessions[2]).expect("Should remove session");
    assert_eq!(helper.get_group_count("conference_call"), 4, "Conference should have 4 sessions after leave");
    
    // Someone joins the conference
    let new_participant = helper.create_test_session();
    helper.add_to_group("conference_call", new_participant).expect("Should add new participant");
    assert_eq!(helper.get_group_count("conference_call"), 5, "Conference should have 5 sessions after join");
    
    // Scenario 2: Call queue lifecycle
    let mut queue_sessions = Vec::new();
    
    // Calls enter queue
    for i in 0..10 {
        let session = helper.create_test_session();
        helper.add_to_group("call_queue", session.clone()).expect("Should add to queue");
        queue_sessions.push(session);
    }
    assert_eq!(helper.get_group_count("call_queue"), 10, "Queue should have 10 waiting calls");
    
    // Calls are answered (removed from queue)
    for session in &queue_sessions[0..5] {
        helper.remove_from_group("call_queue", session).expect("Should remove from queue");
    }
    assert_eq!(helper.get_group_count("call_queue"), 5, "Queue should have 5 waiting calls after answering");
    
    // Scenario 3: Emergency group management
    let emergency_session = helper.create_test_session();
    helper.add_to_group("emergency", emergency_session.clone()).expect("Should add emergency session");
    helper.add_to_group("priority", emergency_session.clone()).expect("Should add to priority");
    helper.add_to_group("monitoring", emergency_session.clone()).expect("Should add to monitoring");
    
    // Emergency session should be in all relevant groups
    assert!(helper.get_group_sessions("emergency").contains(&emergency_session));
    assert!(helper.get_group_sessions("priority").contains(&emergency_session));
    assert!(helper.get_group_sessions("monitoring").contains(&emergency_session));
    
    println!("✅ Group lifecycle scenarios tests passed");
}

#[tokio::test]
async fn test_group_validation_and_consistency() {
    println!("🧪 Testing group validation and consistency...");
    
    let mut helper = GroupsTestHelper::new();
    
    // Create known sessions and groups
    let sessions = helper.create_test_sessions(10);
    let groups = ["alpha", "beta", "gamma"];
    
    // Add sessions to groups in a controlled manner
    for (i, session) in sessions.iter().enumerate() {
        let group = groups[i % groups.len()];
        helper.add_to_group(group, session.clone()).expect("Should add session to group");
    }
    
    // Verify distribution
    for group in groups {
        let group_sessions = helper.get_group_sessions(group);
        let expected_count = (10 + groups.len() - 1) / groups.len(); // Ceiling division
        assert!(group_sessions.len() <= expected_count + 1, "Group {} should have reasonable number of sessions", group);
    }
    
    // Test consistency after operations
    let test_session = &sessions[0];
    let original_groups: Vec<String> = groups.iter()
        .filter(|&group| helper.get_group_sessions(group).contains(test_session))
        .map(|s| s.to_string())
        .collect();
    
    // Remove and re-add session
    for group in &original_groups {
        helper.remove_from_group(group, test_session).expect("Should remove session");
    }
    
    // Verify session is removed from all groups
    for group in groups {
        assert!(!helper.get_group_sessions(group).contains(test_session), 
                "Session should not be in group {} after removal", group);
    }
    
    // Re-add to original groups
    for group in &original_groups {
        helper.add_to_group(group, test_session.clone()).expect("Should re-add session");
    }
    
    // Verify session is back in original groups
    for group in &original_groups {
        assert!(helper.get_group_sessions(group).contains(test_session),
                "Session should be back in group {} after re-adding", group);
    }
    
    println!("✅ Group validation and consistency tests passed");
}

#[tokio::test]
async fn test_group_boundary_conditions() {
    println!("🧪 Testing group boundary conditions...");
    
    let mut helper = GroupsTestHelper::new();
    
    // Test with very long group names
    let long_group_name = "a".repeat(1000);
    let session = helper.create_test_session();
    let result = helper.add_to_group(&long_group_name, session.clone());
    assert!(result.is_ok(), "Should handle very long group names");
    
    // Test with special characters in group names
    let special_groups = ["group with spaces", "group-with-dashes", "group_with_underscores", "group.with.dots"];
    for group_name in special_groups {
        let result = helper.add_to_group(group_name, session.clone());
        assert!(result.is_ok(), "Should handle group name: {}", group_name);
        
        let group_sessions = helper.get_group_sessions(group_name);
        assert!(group_sessions.contains(&session), "Session should be in group: {}", group_name);
    }
    
    // Test with empty group after all sessions removed
    let temp_sessions = helper.create_test_sessions(3);
    for session in &temp_sessions {
        helper.add_to_group("temp_group", session.clone()).expect("Should add session");
    }
    assert_eq!(helper.get_group_count("temp_group"), 3, "Temp group should have 3 sessions");
    
    // Remove all sessions
    for session in &temp_sessions {
        helper.remove_from_group("temp_group", session).expect("Should remove session");
    }
    assert_eq!(helper.get_group_count("temp_group"), 0, "Temp group should be empty");
    
    // Empty group should still be queryable
    let empty_sessions = helper.get_group_sessions("temp_group");
    assert!(empty_sessions.is_empty(), "Empty group should return empty vector");
    
    println!("✅ Group boundary conditions tests passed");
} 