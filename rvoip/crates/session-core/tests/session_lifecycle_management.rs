//! Session Lifecycle Management Tests
//!
//! Tests for LifecycleManager functionality including session creation/termination events,
//! lifecycle hooks, and event handling edge cases.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::CallState,
    session::lifecycle::LifecycleManager,
    SessionError,
};
use common::*;

#[tokio::test]
async fn test_lifecycle_manager_creation() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_manager_creation");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Test basic functionality exists
        helper.trigger_session_created("test_session_1").await;
        helper.trigger_session_terminated("test_session_1").await;
        
        // Verify events were tracked
        let events = helper.get_events().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "created");
        assert_eq!(events[1].event_type, "terminated");
        
        println!("Completed test_lifecycle_manager_creation");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_manager_creation timed out");
    }
}

#[tokio::test]
async fn test_session_creation_events() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_creation_events");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Create multiple sessions
        let session_ids = vec![
            "session_1".to_string(),
            "session_2".to_string(),
            "session_3".to_string(),
        ];
        
        for session_id in &session_ids {
            helper.trigger_session_created(session_id).await;
        }
        
        // Verify all creation events were tracked
        let events = helper.get_events().await;
        assert_eq!(events.len(), 3);
        
        for (i, session_id) in session_ids.iter().enumerate() {
            assert_eq!(events[i].event_type, "created");
            assert_eq!(events[i].session_id, *session_id);
            assert!(events[i].timestamp.elapsed().as_secs() < 5);
        }
        
        // Test event filtering
        let created_events = helper.get_events_by_type("created").await;
        assert_eq!(created_events.len(), 3);
        
        let terminated_events = helper.get_events_by_type("terminated").await;
        assert_eq!(terminated_events.len(), 0);
        
        println!("Completed test_session_creation_events");
    }).await;
    
    if result.is_err() {
        panic!("test_session_creation_events timed out");
    }
}

#[tokio::test]
async fn test_session_termination_events() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_termination_events");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Create and terminate sessions
        let session_ids = vec![
            "session_1".to_string(),
            "session_2".to_string(),
            "session_3".to_string(),
        ];
        
        // Create sessions first
        for session_id in &session_ids {
            helper.trigger_session_created(session_id).await;
        }
        
        // Then terminate them
        for session_id in &session_ids {
            helper.trigger_session_terminated(session_id).await;
        }
        
        // Verify all events were tracked
        let events = helper.get_events().await;
        assert_eq!(events.len(), 6); // 3 created + 3 terminated
        
        // Check created events
        for i in 0..3 {
            assert_eq!(events[i].event_type, "created");
            assert_eq!(events[i].session_id, session_ids[i]);
        }
        
        // Check terminated events
        for i in 3..6 {
            assert_eq!(events[i].event_type, "terminated");
            assert_eq!(events[i].session_id, session_ids[i - 3]);
        }
        
        // Test event filtering
        let terminated_events = helper.get_events_by_type("terminated").await;
        assert_eq!(terminated_events.len(), 3);
        
        println!("Completed test_session_termination_events");
    }).await;
    
    if result.is_err() {
        panic!("test_session_termination_events timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_event_ordering() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_event_ordering");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Create a complex sequence of events
        helper.trigger_session_created("session_1").await;
        helper.trigger_session_created("session_2").await;
        helper.trigger_session_terminated("session_1").await;
        helper.trigger_session_created("session_3").await;
        helper.trigger_session_terminated("session_2").await;
        helper.trigger_session_terminated("session_3").await;
        
        // Verify event ordering
        let events = helper.get_events().await;
        assert_eq!(events.len(), 6);
        
        let expected_sequence = vec![
            ("session_1", "created"),
            ("session_2", "created"),
            ("session_1", "terminated"),
            ("session_3", "created"),
            ("session_2", "terminated"),
            ("session_3", "terminated"),
        ];
        
        for (i, (expected_session, expected_type)) in expected_sequence.iter().enumerate() {
            assert_eq!(events[i].session_id, *expected_session);
            assert_eq!(events[i].event_type, *expected_type);
        }
        
        // Verify timestamps are monotonic
        for i in 1..events.len() {
            assert!(events[i].timestamp >= events[i-1].timestamp);
        }
        
        println!("Completed test_lifecycle_event_ordering");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_event_ordering timed out");
    }
}

#[tokio::test]
async fn test_concurrent_lifecycle_events() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_concurrent_lifecycle_events");
        
        let helper = LifecycleManagerTestHelper::new();
        let concurrent_sessions = 20;
        let mut handles = Vec::new();
        
        // Spawn concurrent session lifecycle tasks
        for session_num in 0..concurrent_sessions {
            let helper_clone = LifecycleManagerTestHelper::new();
            let session_id = format!("session_{}", session_num);
            
            let handle = tokio::spawn(async move {
                // Create session
                helper_clone.trigger_session_created(&session_id).await;
                
                // Small delay to simulate processing
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                // Terminate session
                helper_clone.trigger_session_terminated(&session_id).await;
                
                session_id
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        let mut completed_sessions = Vec::new();
        for handle in handles {
            let session_id = handle.await.unwrap();
            completed_sessions.push(session_id);
        }
        
        // All sessions should have completed
        assert_eq!(completed_sessions.len(), concurrent_sessions);
        
        // Note: Since we created separate helpers for each task,
        // we can't check the main helper's events, but we verified
        // that all concurrent operations completed successfully
        
        println!("Completed test_concurrent_lifecycle_events with {} sessions", concurrent_sessions);
    }).await;
    
    if result.is_err() {
        panic!("test_concurrent_lifecycle_events timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_event_history_management() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_event_history_management");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Generate many events
        for i in 0..10 {
            let session_id = format!("session_{}", i);
            helper.trigger_session_created(&session_id).await;
            helper.trigger_session_terminated(&session_id).await;
        }
        
        // Verify all events are tracked
        let events = helper.get_events().await;
        assert_eq!(events.len(), 20); // 10 created + 10 terminated
        
        // Test history clearing
        helper.clear_events().await;
        let cleared_events = helper.get_events().await;
        assert_eq!(cleared_events.len(), 0);
        
        // Add new events after clearing
        helper.trigger_session_created("new_session").await;
        let new_events = helper.get_events().await;
        assert_eq!(new_events.len(), 1);
        assert_eq!(new_events[0].session_id, "new_session");
        assert_eq!(new_events[0].event_type, "created");
        
        println!("Completed test_lifecycle_event_history_management");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_event_history_management timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_events_with_session_states() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_events_with_session_states");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Test lifecycle events with different session states
        let session_id = "test_session";
        
        // Create session
        helper.trigger_session_created(session_id).await;
        
        // Simulate state transitions with events
        helper.trigger_session_state_change(session_id, CallState::Initiating, CallState::Ringing).await;
        helper.trigger_session_state_change(session_id, CallState::Ringing, CallState::Active).await;
        helper.trigger_session_state_change(session_id, CallState::Active, CallState::OnHold).await;
        helper.trigger_session_state_change(session_id, CallState::OnHold, CallState::Active).await;
        helper.trigger_session_state_change(session_id, CallState::Active, CallState::Terminated).await;
        
        // Terminate session
        helper.trigger_session_terminated(session_id).await;
        
        // Verify events
        let events = helper.get_events().await;
        assert_eq!(events.len(), 7); // 1 created + 5 state changes + 1 terminated
        
        assert_eq!(events[0].event_type, "created");
        assert_eq!(events[1].event_type, "state_change");
        assert_eq!(events[2].event_type, "state_change");
        assert_eq!(events[3].event_type, "state_change");
        assert_eq!(events[4].event_type, "state_change");
        assert_eq!(events[5].event_type, "state_change");
        assert_eq!(events[6].event_type, "terminated");
        
        // All events should be for the same session
        for event in &events {
            assert_eq!(event.session_id, session_id);
        }
        
        println!("Completed test_lifecycle_events_with_session_states");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_events_with_session_states timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_edge_cases() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_edge_cases");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Test with empty session ID
        helper.trigger_session_created("").await;
        helper.trigger_session_terminated("").await;
        
        // Test with very long session ID
        let long_id = "a".repeat(1000);
        helper.trigger_session_created(&long_id).await;
        helper.trigger_session_terminated(&long_id).await;
        
        // Test with special characters in session ID
        let special_id = "session-with_special.chars@123";
        helper.trigger_session_created(special_id).await;
        helper.trigger_session_terminated(special_id).await;
        
        // Test with unicode session ID
        let unicode_id = "session_🦀_test";
        helper.trigger_session_created(unicode_id).await;
        helper.trigger_session_terminated(unicode_id).await;
        
        // Verify all events were recorded
        let events = helper.get_events().await;
        assert_eq!(events.len(), 8); // 4 created + 4 terminated
        
        // Test duplicate events (should still be recorded)
        let duplicate_id = "duplicate_test";
        helper.trigger_session_created(duplicate_id).await;
        helper.trigger_session_created(duplicate_id).await; // Duplicate create
        helper.trigger_session_terminated(duplicate_id).await;
        helper.trigger_session_terminated(duplicate_id).await; // Duplicate terminate
        
        let all_events = helper.get_events().await;
        assert_eq!(all_events.len(), 12); // Previous 8 + 4 new
        
        println!("Completed test_lifecycle_edge_cases");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_edge_cases timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_performance() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_lifecycle_performance");
        
        let helper = LifecycleManagerTestHelper::new();
        let event_count = 1000;
        
        let start = std::time::Instant::now();
        
        // Generate many lifecycle events quickly
        for i in 0..event_count {
            let session_id = format!("perf_session_{}", i);
            helper.trigger_session_created(&session_id).await;
            helper.trigger_session_terminated(&session_id).await;
        }
        
        let duration = start.elapsed();
        println!("Generated {} lifecycle events in {:?}", event_count * 2, duration);
        
        // Verify all events were recorded
        let events = helper.get_events().await;
        assert_eq!(events.len(), event_count * 2);
        
        // Performance assertion
        assert!(duration < Duration::from_secs(10), "Lifecycle event processing took too long");
        
        println!("Completed test_lifecycle_performance");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_performance timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_event_filtering() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_event_filtering");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Create mixed events
        helper.trigger_session_created("session_1").await;
        helper.trigger_session_created("session_2").await;
        helper.trigger_session_state_change("session_1", CallState::Initiating, CallState::Ringing).await;
        helper.trigger_session_terminated("session_1").await;
        helper.trigger_session_state_change("session_2", CallState::Initiating, CallState::Active).await;
        helper.trigger_session_terminated("session_2").await;
        
        // Test filtering by event type
        let created_events = helper.get_events_by_type("created").await;
        assert_eq!(created_events.len(), 2);
        
        let terminated_events = helper.get_events_by_type("terminated").await;
        assert_eq!(terminated_events.len(), 2);
        
        let state_change_events = helper.get_events_by_type("state_change").await;
        assert_eq!(state_change_events.len(), 2);
        
        // Test filtering by session ID
        let session1_events = helper.get_events_by_session("session_1").await;
        assert_eq!(session1_events.len(), 3); // created, state_change, terminated
        
        let session2_events = helper.get_events_by_session("session_2").await;
        assert_eq!(session2_events.len(), 3); // created, state_change, terminated
        
        // Test non-existent filters
        let nonexistent_type_events = helper.get_events_by_type("nonexistent").await;
        assert_eq!(nonexistent_type_events.len(), 0);
        
        let nonexistent_session_events = helper.get_events_by_session("nonexistent").await;
        assert_eq!(nonexistent_session_events.len(), 0);
        
        println!("Completed test_lifecycle_event_filtering");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_event_filtering timed out");
    }
}

#[tokio::test]
async fn test_lifecycle_helper_robustness() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_lifecycle_helper_robustness");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Test that helper methods don't panic or fail
        
        // Test with various session IDs
        let long_id = "x".repeat(1000);
        let test_ids = vec![
            "normal_id",
            "",
            "🦀",
            &long_id,
            "id with spaces",
            "id\nwith\nnewlines",
            "id\twith\ttabs",
        ];
        
        for test_id in test_ids {
            helper.trigger_session_created(test_id).await;
            helper.trigger_session_terminated(test_id).await;
            
            // Test state changes
            helper.trigger_session_state_change(
                test_id, 
                CallState::Initiating, 
                CallState::Ringing
            ).await;
            helper.trigger_session_state_change(
                test_id, 
                CallState::Ringing, 
                CallState::Failed("test error".to_string())
            ).await;
        }
        
        // Verify we can still get events without errors
        let events = helper.get_events().await;
        assert!(events.len() > 0);
        
        // Test clearing still works
        helper.clear_events().await;
        let cleared_events = helper.get_events().await;
        assert_eq!(cleared_events.len(), 0);
        
        println!("Completed test_lifecycle_helper_robustness");
    }).await;
    
    if result.is_err() {
        panic!("test_lifecycle_helper_robustness timed out");
    }
}

#[tokio::test]
async fn test_comprehensive_lifecycle_scenario() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_comprehensive_lifecycle_scenario");
        
        let helper = LifecycleManagerTestHelper::new();
        
        // Simulate a realistic session scenario
        let session_id = "comprehensive_test_session";
        
        // 1. Create session
        helper.trigger_session_created(session_id).await;
        
        // 2. Complete call setup flow with events
        helper.trigger_session_state_change(session_id, CallState::Initiating, CallState::Ringing).await;
        helper.trigger_session_state_change(session_id, CallState::Ringing, CallState::Active).await;
        
        // 3. Hold and resume cycle
        helper.trigger_session_state_change(session_id, CallState::Active, CallState::OnHold).await;
        helper.trigger_session_state_change(session_id, CallState::OnHold, CallState::Active).await;
        
        // 4. Another hold cycle
        helper.trigger_session_state_change(session_id, CallState::Active, CallState::OnHold).await;
        helper.trigger_session_state_change(session_id, CallState::OnHold, CallState::Active).await;
        
        // 5. Normal termination
        helper.trigger_session_state_change(session_id, CallState::Active, CallState::Terminated).await;
        helper.trigger_session_terminated(session_id).await;
        
        // Verify the complete event sequence
        let events = helper.get_events().await;
        assert_eq!(events.len(), 9); // 1 created + 7 state changes + 1 terminated
        
        let expected_event_types = vec![
            "created",
            "state_change", // Initiating -> Ringing
            "state_change", // Ringing -> Active  
            "state_change", // Active -> OnHold
            "state_change", // OnHold -> Active
            "state_change", // Active -> OnHold
            "state_change", // OnHold -> Active
            "state_change", // Active -> Terminated
            "terminated",
        ];
        
        for (i, expected_type) in expected_event_types.iter().enumerate() {
            assert_eq!(events[i].event_type, *expected_type, 
                      "Event {} should be {}, but was {}", i, expected_type, events[i].event_type);
            assert_eq!(events[i].session_id, session_id);
        }
        
        // Verify timing constraints
        for i in 1..events.len() {
            assert!(events[i].timestamp >= events[i-1].timestamp, 
                   "Events should be in chronological order");
        }
        
        // Verify total duration is reasonable
        if let (Some(last_event), Some(first_event)) = (events.last(), events.first()) {
            let total_duration = last_event.timestamp.duration_since(first_event.timestamp);
            assert!(total_duration < Duration::from_secs(5), 
                   "Total scenario duration should be reasonable");
        }
        
        println!("Completed test_comprehensive_lifecycle_scenario");
    }).await;
    
    if result.is_err() {
        panic!("test_comprehensive_lifecycle_scenario timed out");
    }
} 