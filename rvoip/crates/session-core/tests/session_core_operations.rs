use rvoip_session_core::api::control::SessionControl;
// Session Core Operations Tests
//
// Tests for SessionImpl core functionality including session creation,
// state management, and basic operations.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::{SessionId, CallState},
    session::SessionImpl,
    SessionError,
};
use common::*;

#[tokio::test]
async fn test_session_creation() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_creation");
        
        let session_id = SessionId::new();
        let session = SessionImpl::new(session_id.clone());
        
        // Verify initial state
        assert_eq!(session.call_session.id, session_id);
        assert_eq!(*session.state(), CallState::Initiating);
        
        println!("Completed test_session_creation");
    }).await;
    
    if result.is_err() {
        panic!("test_session_creation timed out");
    }
}

#[tokio::test]
async fn test_session_state_update() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_state_update");
        
        let mut helper = SessionImplTestHelper::new();
        let session_id = helper.create_test_session().await;
        
        // Test state update
        helper.update_session_state(&session_id, CallState::Ringing).await.unwrap();
        helper.verify_session_state(&session_id, CallState::Ringing).await;
        
        // Test another state update
        helper.update_session_state(&session_id, CallState::Active).await.unwrap();
        helper.verify_session_state(&session_id, CallState::Active).await;
        
        println!("Completed test_session_state_update");
    }).await;
    
    if result.is_err() {
        panic!("test_session_state_update timed out");
    }
}

#[tokio::test]
async fn test_session_with_custom_states() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_with_custom_states");
        
        let helper = SessionImplTestHelper::new();
        
        // Test creating sessions with different initial states
        let states = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Terminated,
        ];
        
        for state in states {
            let session_id = helper.create_test_session_with_state(state.clone()).await;
            helper.verify_session_state(&session_id, state).await;
        }
        
        println!("Completed test_session_with_custom_states");
    }).await;
    
    if result.is_err() {
        panic!("test_session_with_custom_states timed out");
    }
}

#[tokio::test]
async fn test_multiple_sessions_management() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_multiple_sessions_management");
        
        let helper = SessionImplTestHelper::new();
        let session_count = 10;
        let mut session_ids = Vec::new();
        
        // Create multiple sessions
        for i in 0..session_count {
            let session_id = helper.create_test_session().await;
            session_ids.push(session_id);
        }
        
        // Verify count
        assert_eq!(helper.session_count().await, session_count);
        
        // Update each session to a different state
        for (i, session_id) in session_ids.iter().enumerate() {
            let state = match i % 4 {
                0 => CallState::Ringing,
                1 => CallState::Active,
                2 => CallState::OnHold,
                _ => CallState::Terminated,
            };
            
            helper.update_session_state(session_id, state.clone()).await.unwrap();
            helper.verify_session_state(session_id, state).await;
        }
        
        println!("Completed test_multiple_sessions_management");
    }).await;
    
    if result.is_err() {
        panic!("test_multiple_sessions_management timed out");
    }
}

#[tokio::test]
async fn test_session_state_transitions() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_state_transitions");
        
        let helper = SessionImplTestHelper::new();
        let session_id = helper.create_test_session().await;
        
        // Test typical call flow
        let state_sequence = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Active,
            CallState::Terminated,
        ];
        
        // Verify initial state
        helper.verify_session_state(&session_id, CallState::Initiating).await;
        
        // Progress through states
        for state in state_sequence.iter().skip(1) {
            helper.update_session_state(&session_id, state.clone()).await.unwrap();
            helper.verify_session_state(&session_id, state.clone()).await;
        }
        
        println!("Completed test_session_state_transitions");
    }).await;
    
    if result.is_err() {
        panic!("test_session_state_transitions timed out");
    }
}

#[tokio::test]
async fn test_session_operations_on_nonexistent_session() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_operations_on_nonexistent_session");
        
        let helper = SessionImplTestHelper::new();
        let fake_session_id = SessionId("non-existent-session".to_string());
        
        // Try to update non-existent session
        let result = helper.update_session_state(&fake_session_id, CallState::Active).await;
        assert!(result.is_err());
        
        // Try to get non-existent session
        let session = helper.get_session(&fake_session_id).await;
        assert!(session.is_none());
        
        println!("Completed test_session_operations_on_nonexistent_session");
    }).await;
    
    if result.is_err() {
        panic!("test_session_operations_on_nonexistent_session timed out");
    }
}

#[tokio::test]
async fn test_session_with_failed_state() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_with_failed_state");
        
        let helper = SessionImplTestHelper::new();
        let session_id = helper.create_test_session().await;
        
        // Test updating to failed state with reason
        let failure_reason = "Network timeout";
        let failed_state = CallState::Failed(failure_reason.to_string());
        
        helper.update_session_state(&session_id, failed_state.clone()).await.unwrap();
        helper.verify_session_state(&session_id, failed_state).await;
        
        println!("Completed test_session_with_failed_state");
    }).await;
    
    if result.is_err() {
        panic!("test_session_with_failed_state timed out");
    }
}

#[tokio::test]
async fn test_session_cleanup_operations() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_cleanup_operations");
        
        let helper = SessionImplTestHelper::new();
        
        // Create multiple sessions
        for _ in 0..5 {
            helper.create_test_session().await;
        }
        
        assert_eq!(helper.session_count().await, 5);
        
        // Clear all sessions
        helper.clear_sessions().await;
        assert_eq!(helper.session_count().await, 0);
        
        println!("Completed test_session_cleanup_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_session_cleanup_operations timed out");
    }
}

#[tokio::test]
async fn test_session_concurrent_operations() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_concurrent_operations");
        
        let helper = SessionImplTestHelper::new();
        let concurrent_tasks = 10;
        let mut handles = Vec::new();
        
        // Spawn concurrent tasks
        for task_id in 0..concurrent_tasks {
            let helper_clone = SessionImplTestHelper::new_with_config(helper.config.clone());
            let handle = tokio::spawn(async move {
                let session_id = helper_clone.create_test_session().await;
                
                // Perform some state updates
                helper_clone.update_session_state(&session_id, CallState::Ringing).await.unwrap();
                helper_clone.update_session_state(&session_id, CallState::Active).await.unwrap();
                helper_clone.update_session_state(&session_id, CallState::Terminated).await.unwrap();
                
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
        
        assert_eq!(completed_sessions.len(), concurrent_tasks);
        println!("Completed test_session_concurrent_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_session_concurrent_operations timed out");
    }
}

#[tokio::test]
async fn test_session_performance_basic() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_session_performance_basic");
        
        let helper = SessionImplTestHelper::new_with_config(SessionTestConfig::fast());
        let session_count = 100;
        
        let start = std::time::Instant::now();
        
        // Create many sessions quickly
        for _ in 0..session_count {
            helper.create_test_session().await;
        }
        
        let creation_time = start.elapsed();
        println!("Created {} sessions in {:?}", session_count, creation_time);
        
        // Verify count
        assert_eq!(helper.session_count().await, session_count);
        
        // Performance assertion
        assert!(creation_time < Duration::from_secs(5), "Session creation took too long");
        
        println!("Completed test_session_performance_basic");
    }).await;
    
    if result.is_err() {
        panic!("test_session_performance_basic timed out");
    }
}

#[tokio::test]
async fn test_session_edge_cases() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_edge_cases");
        
        let helper = SessionImplTestHelper::new();
        
        // Test with empty session ID (should still work)
        let empty_session_id = SessionId("".to_string());
        let session = SessionImpl::new(empty_session_id.clone());
        assert_eq!(session.call_session.id, empty_session_id);
        
        // Test with very long session ID
        let long_id = "a".repeat(1000);
        let long_session_id = SessionId(long_id);
        let session = SessionImpl::new(long_session_id.clone());
        assert_eq!(session.call_session.id, long_session_id);
        
        // Test rapid state changes
        let session_id = helper.create_test_session().await;
        for _ in 0..10 {
            helper.update_session_state(&session_id, CallState::Ringing).await.unwrap();
            helper.update_session_state(&session_id, CallState::Active).await.unwrap();
        }
        
        println!("Completed test_session_edge_cases");
    }).await;
    
    if result.is_err() {
        panic!("test_session_edge_cases timed out");
    }
}

#[tokio::test]
async fn test_session_helper_functionality() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_helper_functionality");
        
        let helper = SessionImplTestHelper::new();
        
        // Test creating session with specific state
        let active_session = helper.create_test_session_with_state(CallState::Active).await;
        helper.verify_session_state(&active_session, CallState::Active).await;
        
        let hold_session = helper.create_test_session_with_state(CallState::OnHold).await;
        helper.verify_session_state(&hold_session, CallState::OnHold).await;
        
        // Test session retrieval
        let retrieved_session = helper.get_session(&active_session).await;
        assert!(retrieved_session.is_some());
        assert_eq!(*retrieved_session.unwrap().state(), CallState::Active);
        
        // Test session count tracking
        assert_eq!(helper.session_count().await, 2);
        
        println!("Completed test_session_helper_functionality");
    }).await;
    
    if result.is_err() {
        panic!("test_session_helper_functionality timed out");
    }
}

#[tokio::test]
async fn test_session_stress_operations() {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        println!("Starting test_session_stress_operations");
        
        let helper = SessionImplTestHelper::new_with_config(SessionTestConfig::stress());
        let session_count = 500;
        let mut session_ids = Vec::new();
        
        // Create many sessions
        for _ in 0..session_count {
            let session_id = helper.create_test_session().await;
            session_ids.push(session_id);
        }
        
        // Rapidly update all sessions
        for session_id in &session_ids {
            helper.update_session_state(session_id, CallState::Ringing).await.unwrap();
            helper.update_session_state(session_id, CallState::Active).await.unwrap();
        }
        
        // Verify final count and states
        assert_eq!(helper.session_count().await, session_count);
        
        for session_id in &session_ids {
            helper.verify_session_state(session_id, CallState::Active).await;
        }
        
        println!("Completed test_session_stress_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_session_stress_operations timed out");
    }
}

#[tokio::test]
async fn test_session_configuration_variants() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_configuration_variants");
        
        // Test with fast config
        let fast_helper = SessionImplTestHelper::new_with_config(SessionTestConfig::fast());
        let fast_session = fast_helper.create_test_session().await;
        fast_helper.verify_session_state(&fast_session, CallState::Initiating).await;
        
        // Test with stress config  
        let stress_helper = SessionImplTestHelper::new_with_config(SessionTestConfig::stress());
        let stress_session = stress_helper.create_test_session().await;
        stress_helper.verify_session_state(&stress_session, CallState::Initiating).await;
        
        // Test with default config
        let default_helper = SessionImplTestHelper::new();
        let default_session = default_helper.create_test_session().await;
        default_helper.verify_session_state(&default_session, CallState::Initiating).await;
        
        println!("Completed test_session_configuration_variants");
    }).await;
    
    if result.is_err() {
        panic!("test_session_configuration_variants timed out");
    }
} 