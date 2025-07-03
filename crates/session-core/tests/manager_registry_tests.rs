use rvoip_session_core::api::control::SessionControl;
// Tests for SessionRegistry Operations
//
// Tests the session registry functionality including session storage,
// lookup operations, statistics tracking, and registry management.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::{CallState, SessionId, CallSession},
    manager::registry::SessionRegistry,
};
use common::*;

#[tokio::test]
async fn test_registry_creation() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_registry_creation");
        let registry = SessionRegistry::new();
        
        // Initial state should be empty
        assert_eq!(registry.active_session_count().await, 0);
        
        let stats = registry.get_stats().await.unwrap();
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.total_sessions, 0);
        println!("Completed test_registry_creation");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_creation timed out after 5 seconds");
    }
}

#[tokio::test]
async fn test_registry_session_registration() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_session_registration");
        let mut helper = RegistryTestHelper::new();
        
        // Register a session
        let session_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Initiating
        ).await;
        
        // Verify session count
        helper.verify_session_count(1).await;
        
        // Verify session exists
        let session = helper.verify_session_exists(&session_id).await;
        assert_eq!(session.from, "sip:alice@localhost");
        assert_eq!(session.to, "sip:bob@localhost");
        assert_eq!(session.state, CallState::Initiating);
        println!("Completed test_registry_session_registration");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_session_registration timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_multiple_sessions() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_registry_multiple_sessions");
        let mut helper = RegistryTestHelper::new();
        let mut session_ids = Vec::new();
        
        // Register multiple sessions
        for i in 0..5 {
            let session_id = helper.add_test_session(
                &format!("sip:caller{}@localhost", i),
                &format!("sip:callee{}@localhost", i),
                CallState::Initiating
            ).await;
            session_ids.push(session_id);
        }
        
        // Verify count
        helper.verify_session_count(5).await;
        
        // Verify all sessions exist
        for session_id in &session_ids {
            helper.verify_session_exists(session_id).await;
        }
        
        let stats = helper.get_stats().await;
        assert_eq!(stats.active_sessions, 5);
        assert_eq!(stats.total_sessions, 5);
        println!("Completed test_registry_multiple_sessions");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_multiple_sessions timed out after 15 seconds");
    }
}

#[tokio::test]
async fn test_registry_session_unregistration() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_session_unregistration");
        let mut helper = RegistryTestHelper::new();
        
        // Register sessions
        let session1_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Active
        ).await;
        
        let session2_id = helper.add_test_session(
            "sip:charlie@localhost",
            "sip:david@localhost",
            CallState::Active
        ).await;
        
        helper.verify_session_count(2).await;
        
        // Unregister one session
        helper.registry().unregister_session(&session1_id).await.unwrap();
        
        // Verify counts updated
        helper.verify_session_count(1).await;
        helper.verify_session_not_exists(&session1_id).await;
        helper.verify_session_exists(&session2_id).await;
        
        let stats = helper.get_stats().await;
        assert_eq!(stats.active_sessions, 1);
        assert_eq!(stats.total_sessions, 2); // Total includes terminated
        println!("Completed test_registry_session_unregistration");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_session_unregistration timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_session_update() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_session_update");
        let mut helper = RegistryTestHelper::new();
        
        let session_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Initiating
        ).await;
        
        // Update session state
        let updated_session = create_test_call_session(
            session_id.clone(),
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Active
        );
        
        helper.registry().update_session(session_id.clone(), updated_session).await.unwrap();
        
        // Verify updated state
        let session = helper.verify_session_exists(&session_id).await;
        assert_eq!(session.state, CallState::Active);
        println!("Completed test_registry_session_update");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_session_update timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_list_active_sessions() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_list_active_sessions");
        let mut helper = RegistryTestHelper::new();
        let mut session_ids = Vec::new();
        
        // Add several sessions
        for i in 0..3 {
            let session_id = helper.add_test_session(
                &format!("sip:user{}@localhost", i),
                "sip:target@localhost",
                CallState::Active
            ).await;
            session_ids.push(session_id);
        }
        
        // Get active sessions list
        let active_sessions = helper.registry().list_active_sessions().await.unwrap();
        
        assert_eq!(active_sessions.len(), 3);
        for session_id in &session_ids {
            assert!(active_sessions.contains(session_id));
        }
        println!("Completed test_registry_list_active_sessions");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_list_active_sessions timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_get_all_sessions() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_get_all_sessions");
        let mut helper = RegistryTestHelper::new();
        
        // Add sessions with different states
        let _session1 = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Initiating
        ).await;
        
        let _session2 = helper.add_test_session(
            "sip:charlie@localhost",
            "sip:david@localhost",
            CallState::Active
        ).await;
        
        let _session3 = helper.add_test_session(
            "sip:eve@localhost",
            "sip:frank@localhost",
            CallState::OnHold
        ).await;
        
        // Get all sessions
        let all_sessions = helper.registry().get_all_sessions().await.unwrap();
        
        assert_eq!(all_sessions.len(), 3);
        
        // Verify different states are present
        let states: Vec<_> = all_sessions.iter().map(|s| &s.state).collect();
        assert!(states.contains(&&CallState::Initiating));
        assert!(states.contains(&&CallState::Active));
        assert!(states.contains(&&CallState::OnHold));
        println!("Completed test_registry_get_all_sessions");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_get_all_sessions timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_find_sessions_by_caller() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_find_sessions_by_caller");
        let mut helper = RegistryTestHelper::new();
        
        // Add sessions with same caller
        let caller = "sip:alice@localhost";
        let _session1 = helper.add_test_session(
            caller,
            "sip:bob@localhost",
            CallState::Active
        ).await;
        
        let _session2 = helper.add_test_session(
            caller,
            "sip:charlie@localhost",
            CallState::Active
        ).await;
        
        // Add session with different caller
        let _session3 = helper.add_test_session(
            "sip:eve@localhost",
            "sip:frank@localhost",
            CallState::Active
        ).await;
        
        // Find sessions by caller
        let alice_sessions = helper.registry().find_sessions_by_caller(caller).await.unwrap();
        
        assert_eq!(alice_sessions.len(), 2);
        for session in &alice_sessions {
            assert_eq!(session.from, caller);
        }
        
        // Verify different caller returns different results
        let eve_sessions = helper.registry().find_sessions_by_caller("sip:eve@localhost").await.unwrap();
        assert_eq!(eve_sessions.len(), 1);
        println!("Completed test_registry_find_sessions_by_caller");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_find_sessions_by_caller timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_mark_session_failed() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_mark_session_failed");
        let mut helper = RegistryTestHelper::new();
        
        let session_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Active
        ).await;
        
        helper.verify_session_count(1).await;
        
        // Mark session as failed
        helper.registry().mark_session_failed(&session_id).await.unwrap();
        
        // Session should be removed from active sessions
        helper.verify_session_count(0).await;
        helper.verify_session_not_exists(&session_id).await;
        
        // Failed sessions should be tracked in stats
        let stats = helper.get_stats().await;
        assert_eq!(stats.failed_sessions, 1);
        assert_eq!(stats.active_sessions, 0);
        println!("Completed test_registry_mark_session_failed");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_mark_session_failed timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_clear_all() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_clear_all");
        let mut helper = RegistryTestHelper::new();
        
        // Add multiple sessions
        for i in 0..5 {
            helper.add_test_session(
                &format!("sip:user{}@localhost", i),
                "sip:target@localhost",
                CallState::Active
            ).await;
        }
        
        helper.verify_session_count(5).await;
        
        // Clear all sessions
        helper.registry().clear_all().await.unwrap();
        
        // Verify registry is empty
        helper.verify_session_count(0).await;
        
        let active_sessions = helper.registry().list_active_sessions().await.unwrap();
        assert!(active_sessions.is_empty());
        println!("Completed test_registry_clear_all");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_clear_all timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_statistics_tracking() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_registry_statistics_tracking");
        let mut helper = RegistryTestHelper::new();
        
        // Initial stats
        let initial_stats = helper.get_stats().await;
        assert_eq!(initial_stats.total_sessions, 0);
        assert_eq!(initial_stats.active_sessions, 0);
        assert_eq!(initial_stats.failed_sessions, 0);
        
        // Add sessions
        let session1_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Active
        ).await;
        
        let session2_id = helper.add_test_session(
            "sip:charlie@localhost",
            "sip:david@localhost",
            CallState::Active
        ).await;
        
        // Check stats after adding
        let after_add_stats = helper.get_stats().await;
        assert_eq!(after_add_stats.total_sessions, 2);
        assert_eq!(after_add_stats.active_sessions, 2);
        assert_eq!(after_add_stats.failed_sessions, 0);
        
        // Mark one as failed
        helper.registry().mark_session_failed(&session1_id).await.unwrap();
        
        // Check stats after failure
        let after_fail_stats = helper.get_stats().await;
        assert_eq!(after_fail_stats.total_sessions, 2);
        assert_eq!(after_fail_stats.active_sessions, 1);
        assert_eq!(after_fail_stats.failed_sessions, 1);
        
        // Unregister remaining session
        helper.registry().unregister_session(&session2_id).await.unwrap();
        
        // Check final stats
        let final_stats = helper.get_stats().await;
        assert_eq!(final_stats.total_sessions, 2);
        assert_eq!(final_stats.active_sessions, 0);
        assert_eq!(final_stats.failed_sessions, 1);
        println!("Completed test_registry_statistics_tracking");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_statistics_tracking timed out after 15 seconds");
    }
}

#[tokio::test]
async fn test_registry_concurrent_operations() {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        println!("Starting test_registry_concurrent_operations");
        let registry = Arc::new(SessionRegistry::new());
        let mut handles = Vec::new();
        
        // Spawn concurrent tasks for registration
        for i in 0..10 {
            let registry_clone = Arc::clone(&registry);
            let handle = tokio::spawn(async move {
                let session_id = SessionId(format!("concurrent-session-{}", i));
                let session = create_test_call_session(
                    session_id.clone(),
                    &format!("sip:user{}@localhost", i),
                    "sip:target@localhost",
                    CallState::Active
                );
                
                registry_clone.register_session(session_id.clone(), session).await?;
                
                // Perform some operations
                let retrieved = registry_clone.get_session(&session_id).await?;
                assert!(retrieved.is_some());
                
                Ok::<SessionId, rvoip_session_core::SessionError>(session_id)
            });
            handles.push(handle);
        }
        
        // Wait for all tasks
        let mut session_ids = Vec::new();
        for handle in handles {
            let session_id = handle.await.unwrap().unwrap();
            session_ids.push(session_id);
        }
        
        // Verify all sessions were registered
        assert_eq!(registry.active_session_count().await, 10);
        
        let stats = registry.get_stats().await.unwrap();
        assert_eq!(stats.active_sessions, 10);
        assert_eq!(stats.total_sessions, 10);
        println!("Completed test_registry_concurrent_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_concurrent_operations timed out after 20 seconds");
    }
}

#[tokio::test]
async fn test_registry_session_lifecycle_complete() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_registry_session_lifecycle_complete");
        let mut helper = RegistryTestHelper::new();
        
        // Register session
        let session_id = helper.add_test_session(
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Initiating
        ).await;
        
        // Update through various states
        let states = [
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Active,
            CallState::Terminating,
        ];
        
        for state in &states {
            let updated_session = create_test_call_session(
                session_id.clone(),
                "sip:alice@localhost",
                "sip:bob@localhost",
                state.clone()
            );
            
            helper.registry().update_session(session_id.clone(), updated_session).await.unwrap();
            
            let retrieved = helper.verify_session_exists(&session_id).await;
            assert_eq!(retrieved.state, *state);
        }
        
        // Finally unregister
        helper.registry().unregister_session(&session_id).await.unwrap();
        helper.verify_session_not_exists(&session_id).await;
        println!("Completed test_registry_session_lifecycle_complete");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_session_lifecycle_complete timed out after 15 seconds");
    }
}

#[tokio::test]
async fn test_registry_edge_cases() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_edge_cases");
        let mut helper = RegistryTestHelper::new();
        
        // Test unregistering non-existent session
        let fake_session_id = SessionId("non-existent".to_string());
        let result = helper.registry().unregister_session(&fake_session_id).await;
        assert!(result.is_ok()); // Should not fail
        
        // Test getting non-existent session
        let session = helper.registry().get_session(&fake_session_id).await.unwrap();
        assert!(session.is_none());
        
        // Test marking non-existent session as failed
        let result = helper.registry().mark_session_failed(&fake_session_id).await;
        assert!(result.is_ok()); // Should not fail
        
        // Test finding sessions by non-existent caller
        let sessions = helper.registry().find_sessions_by_caller("sip:nobody@localhost").await.unwrap();
        assert!(sessions.is_empty());
        println!("Completed test_registry_edge_cases");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_edge_cases timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_duplicate_registration() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_registry_duplicate_registration");
        let mut helper = RegistryTestHelper::new();
        
        let session_id = SessionId("duplicate-test".to_string());
        let session1 = create_test_call_session(
            session_id.clone(),
            "sip:alice@localhost",
            "sip:bob@localhost",
            CallState::Initiating
        );
        
        // Register session
        helper.registry().register_session(session_id.clone(), session1).await.unwrap();
        helper.verify_session_count(1).await;
        
        // Register same session ID again with different data
        let session2 = create_test_call_session(
            session_id.clone(),
            "sip:charlie@localhost",
            "sip:david@localhost",
            CallState::Active
        );
        
        helper.registry().register_session(session_id.clone(), session2).await.unwrap();
        
        // Should still have only one session (overwritten)
        helper.verify_session_count(1).await;
        
        // Verify the session has the updated data
        let retrieved = helper.verify_session_exists(&session_id).await;
        assert_eq!(retrieved.from, "sip:charlie@localhost");
        assert_eq!(retrieved.to, "sip:david@localhost");
        assert_eq!(retrieved.state, CallState::Active);
        println!("Completed test_registry_duplicate_registration");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_duplicate_registration timed out after 10 seconds");
    }
}

#[tokio::test]
async fn test_registry_performance() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        println!("Starting test_registry_performance");
        let registry = Arc::new(SessionRegistry::new());
        let session_count = 1000;
        
        // Measure registration performance
        let start = std::time::Instant::now();
        
        for i in 0..session_count {
            let session_id = SessionId(format!("perf-session-{}", i));
            let session = create_test_call_session(
                session_id.clone(),
                &format!("sip:user{}@localhost", i),
                "sip:target@localhost",
                CallState::Active
            );
            
            registry.register_session(session_id, session).await.unwrap();
        }
        
        let registration_time = start.elapsed();
        println!("Registered {} sessions in {:?}", session_count, registration_time);
        
        // Verify count
        assert_eq!(registry.active_session_count().await, session_count);
        
        // Measure lookup performance
        let lookup_start = std::time::Instant::now();
        
        for i in 0..session_count {
            let session_id = SessionId(format!("perf-session-{}", i));
            let session = registry.get_session(&session_id).await.unwrap();
            assert!(session.is_some());
        }
        
        let lookup_time = lookup_start.elapsed();
        println!("Looked up {} sessions in {:?}", session_count, lookup_time);
        
        // Performance assertions
        assert!(registration_time < Duration::from_secs(10), "Registration took too long");
        assert!(lookup_time < Duration::from_secs(5), "Lookup took too long");
        println!("Completed test_registry_performance");
    }).await;
    
    if result.is_err() {
        panic!("test_registry_performance timed out after 30 seconds");
    }
} 