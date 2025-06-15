use rvoip_session_core::api::control::SessionControl;
//! Session Integration Tests
//!
//! Comprehensive integration tests that combine all session components
//! (SessionImpl, StateManager, LifecycleManager, MediaCoordinator) to test
//! realistic end-to-end scenarios and component interactions.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::CallState,
    session::{SessionImpl, StateManager, lifecycle::LifecycleManager, media::MediaCoordinator},
    SessionError,
};
use common::*;

#[tokio::test]
async fn test_complete_session_lifecycle() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_complete_session_lifecycle");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_id = "complete_lifecycle_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // 1. Create session
        let session_result = integration_helper.create_session(session_id, CallState::Initiating).await;
        assert!(session_result.is_ok(), "Session creation should succeed");
        
        // 2. Setup media
        let media_result = integration_helper.setup_media(session_id, &sdp).await;
        assert!(media_result.is_ok(), "Media setup should succeed");
        
        // 3. Progress through call states
        integration_helper.update_session_state(session_id, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        
        // 4. Hold and resume cycle
        integration_helper.update_session_state(session_id, CallState::OnHold).await.unwrap();
        let hold_sdp = format!("{}; sendonly", sdp);
        integration_helper.update_media(session_id, &hold_sdp).await.unwrap();
        
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        let resume_sdp = format!("{}; sendrecv", sdp);
        integration_helper.update_media(session_id, &resume_sdp).await.unwrap();
        
        // 5. Terminate session
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Verify complete integration
        let session_info = integration_helper.get_session_info(session_id).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().current_state, CallState::Terminated);
        
        let lifecycle_events = integration_helper.get_lifecycle_events().await;
        assert!(lifecycle_events.len() > 0);
        
        let media_operations = integration_helper.get_media_operations().await;
        assert!(media_operations.len() > 0);
        
        println!("Completed test_complete_session_lifecycle");
    }).await;
    
    if result.is_err() {
        panic!("test_complete_session_lifecycle timed out");
    }
}

#[tokio::test]
async fn test_multiple_concurrent_sessions() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_multiple_concurrent_sessions");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_count = 5;
        let mut handles = Vec::new();
        
        // Create multiple concurrent sessions
        for session_num in 0..session_count {
            let helper_clone = SessionIntegrationHelper::new();
            let session_id = format!("concurrent_session_{}", session_num);
            
            let handle = tokio::spawn(async move {
                let sdp = SessionTestUtils::create_test_sdp();
                
                // Complete session lifecycle
                helper_clone.create_session(&session_id, CallState::Initiating).await.unwrap();
                helper_clone.setup_media(&session_id, &sdp).await.unwrap();
                
                helper_clone.update_session_state(&session_id, CallState::Ringing).await.unwrap();
                helper_clone.update_session_state(&session_id, CallState::Active).await.unwrap();
                
                // Hold cycle
                helper_clone.update_session_state(&session_id, CallState::OnHold).await.unwrap();
                let hold_sdp = format!("{}; hold", sdp);
                helper_clone.update_media(&session_id, &hold_sdp).await.unwrap();
                
                helper_clone.update_session_state(&session_id, CallState::Active).await.unwrap();
                let resume_sdp = format!("{}; resume", sdp);
                helper_clone.update_media(&session_id, &resume_sdp).await.unwrap();
                
                // Terminate
                helper_clone.update_session_state(&session_id, CallState::Terminated).await.unwrap();
                helper_clone.cleanup_media(&session_id).await.unwrap();
                
                session_id
            });
            handles.push(handle);
        }
        
        // Wait for all sessions to complete
        let mut completed_sessions = Vec::new();
        for handle in handles {
            let session_id = handle.await.unwrap();
            completed_sessions.push(session_id);
        }
        
        assert_eq!(completed_sessions.len(), session_count);
        println!("Completed test_multiple_concurrent_sessions with {} sessions", session_count);
    }).await;
    
    if result.is_err() {
        panic!("test_multiple_concurrent_sessions timed out");
    }
}

#[tokio::test]
async fn test_session_failure_scenarios() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_failure_scenarios");
        
        let integration_helper = SessionIntegrationHelper::new();
        
        // Test 1: Early failure during ringing
        let early_failure_session = "early_failure_session";
        integration_helper.create_session(early_failure_session, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(early_failure_session, &SessionTestUtils::create_test_sdp()).await.unwrap();
        integration_helper.update_session_state(early_failure_session, CallState::Ringing).await.unwrap();
        
        // Simulate failure
        let failure_state = CallState::Failed("Network timeout".to_string());
        integration_helper.update_session_state(early_failure_session, failure_state.clone()).await.unwrap();
        integration_helper.cleanup_media(early_failure_session).await.unwrap();
        
        let session_info = integration_helper.get_session_info(early_failure_session).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().current_state, failure_state);
        
        // Test 2: Failure during active call
        let active_failure_session = "active_failure_session";
        integration_helper.create_session(active_failure_session, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(active_failure_session, &SessionTestUtils::create_test_sdp()).await.unwrap();
        integration_helper.update_session_state(active_failure_session, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(active_failure_session, CallState::Active).await.unwrap();
        
        // Simulate failure during active call
        let active_failure_state = CallState::Failed("Media connection lost".to_string());
        integration_helper.update_session_state(active_failure_session, active_failure_state.clone()).await.unwrap();
        integration_helper.cleanup_media(active_failure_session).await.unwrap();
        
        let active_session_info = integration_helper.get_session_info(active_failure_session).await;
        assert!(active_session_info.is_some());
        assert_eq!(active_session_info.unwrap().current_state, active_failure_state);
        
        // Test 3: Media failure scenario
        let media_failure_session = "media_failure_session";
        integration_helper.create_session(media_failure_session, CallState::Initiating).await.unwrap();
        
        // Try to setup media with invalid SDP
        let invalid_sdp = "invalid_sdp_content";
        let media_result = integration_helper.setup_media(media_failure_session, invalid_sdp).await;
        // Media setup might fail or succeed depending on implementation
        
        // Continue with session lifecycle
        integration_helper.update_session_state(media_failure_session, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(media_failure_session, CallState::Failed("Media setup failed".to_string())).await.unwrap();
        integration_helper.cleanup_media(media_failure_session).await.unwrap();
        
        println!("Completed test_session_failure_scenarios");
    }).await;
    
    if result.is_err() {
        panic!("test_session_failure_scenarios timed out");
    }
}

#[tokio::test]
async fn test_session_state_media_synchronization() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_state_media_synchronization");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_id = "sync_test_session";
        let base_sdp = SessionTestUtils::create_test_sdp();
        
        // Create session and setup media
        integration_helper.create_session(session_id, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(session_id, &base_sdp).await.unwrap();
        
        // Test state transitions with corresponding media updates
        let test_scenarios = vec![
            (CallState::Ringing, format!("{}; state=ringing", base_sdp)),
            (CallState::Active, format!("{}; state=active", base_sdp)),
            (CallState::OnHold, format!("{}; state=onhold;sendonly", base_sdp)),
            (CallState::Active, format!("{}; state=active;sendrecv", base_sdp)),
        ];
        
        for (target_state, corresponding_sdp) in test_scenarios {
            // Update session state
            integration_helper.update_session_state(session_id, target_state.clone()).await.unwrap();
            
            // Update media to match state
            integration_helper.update_media(session_id, &corresponding_sdp).await.unwrap();
            
            // Verify state synchronization
            let session_info = integration_helper.get_session_info(session_id).await;
            assert!(session_info.is_some());
            assert_eq!(session_info.unwrap().current_state, target_state);
        }
        
        // Final cleanup
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Verify comprehensive history
        let lifecycle_events = integration_helper.get_lifecycle_events().await;
        let media_operations = integration_helper.get_media_operations().await;
        
        assert!(lifecycle_events.len() > 5); // Multiple state changes
        assert!(media_operations.len() > 5); // Setup + multiple updates + cleanup
        
        println!("Completed test_session_state_media_synchronization");
    }).await;
    
    if result.is_err() {
        panic!("test_session_state_media_synchronization timed out");
    }
}

#[tokio::test]
async fn test_session_performance_integration() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        println!("Starting test_session_performance_integration");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_count = 50;
        let sdp = SessionTestUtils::create_test_sdp();
        
        let start_time = std::time::Instant::now();
        
        // Create many sessions quickly
        for i in 0..session_count {
            let session_id = format!("perf_session_{}", i);
            
            // Complete rapid session lifecycle
            integration_helper.create_session(&session_id, CallState::Initiating).await.unwrap();
            integration_helper.setup_media(&session_id, &sdp).await.unwrap();
            integration_helper.update_session_state(&session_id, CallState::Ringing).await.unwrap();
            integration_helper.update_session_state(&session_id, CallState::Active).await.unwrap();
            integration_helper.update_session_state(&session_id, CallState::Terminated).await.unwrap();
            integration_helper.cleanup_media(&session_id).await.unwrap();
        }
        
        let duration = start_time.elapsed();
        println!("Processed {} complete session lifecycles in {:?}", session_count, duration);
        
        // Performance assertions
        assert!(duration < Duration::from_secs(20), "Session processing took too long");
        
        // Verify all operations were recorded
        let lifecycle_events = integration_helper.get_lifecycle_events().await;
        let media_operations = integration_helper.get_media_operations().await;
        
        assert!(lifecycle_events.len() >= session_count * 4); // Multiple state changes per session
        assert!(media_operations.len() >= session_count * 2); // Setup + cleanup per session
        
        println!("Completed test_session_performance_integration");
    }).await;
    
    if result.is_err() {
        panic!("test_session_performance_integration timed out");
    }
}

#[tokio::test]
async fn test_session_error_recovery() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_error_recovery");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_id = "error_recovery_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // Create session
        integration_helper.create_session(session_id, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(session_id, &sdp).await.unwrap();
        
        // Progress to active state
        integration_helper.update_session_state(session_id, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        
        // Simulate temporary failure and recovery
        integration_helper.update_session_state(session_id, CallState::OnHold).await.unwrap();
        
        // Try invalid media update (simulate error)
        let invalid_sdp = "invalid_recovery_sdp";
        let error_result = integration_helper.update_media(session_id, invalid_sdp).await;
        // Error handling varies by implementation
        
        // Recover with valid media update
        let recovery_sdp = format!("{}; recovered", sdp);
        integration_helper.update_media(session_id, &recovery_sdp).await.unwrap();
        
        // Resume normal operation
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        
        // Multiple recovery attempts
        for i in 1..=3 {
            integration_helper.update_session_state(session_id, CallState::OnHold).await.unwrap();
            let recovery_attempt_sdp = format!("{}; recovery_attempt_{}", sdp, i);
            integration_helper.update_media(session_id, &recovery_attempt_sdp).await.unwrap();
            integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        }
        
        // Final successful termination
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Verify final state
        let session_info = integration_helper.get_session_info(session_id).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().current_state, CallState::Terminated);
        
        println!("Completed test_session_error_recovery");
    }).await;
    
    if result.is_err() {
        panic!("test_session_error_recovery timed out");
    }
}

#[tokio::test]
async fn test_session_component_isolation() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_component_isolation");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_id = "isolation_test_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // Test that component failures don't affect others
        
        // 1. Create session successfully
        integration_helper.create_session(session_id, CallState::Initiating).await.unwrap();
        
        // 2. Media setup with potentially problematic SDP
        let problematic_sdp = "v=0\r\ns=Potentially problematic SDP\r\n🦀";
        let media_result = integration_helper.setup_media(session_id, problematic_sdp).await;
        // Don't assert success/failure - just ensure it doesn't crash
        
        // 3. State manager should still work
        let state_result = integration_helper.update_session_state(session_id, CallState::Ringing).await;
        assert!(state_result.is_ok(), "State updates should work despite media issues");
        
        // 4. Lifecycle events should still be recorded
        let lifecycle_events_before = integration_helper.get_lifecycle_events().await;
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        let lifecycle_events_after = integration_helper.get_lifecycle_events().await;
        assert!(lifecycle_events_after.len() > lifecycle_events_before.len());
        
        // 5. Media recovery should be possible
        let recovery_result = integration_helper.update_media(session_id, &sdp).await;
        // Should work regardless of previous media state
        
        // 6. Normal termination should work
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Verify isolation worked
        let final_session_info = integration_helper.get_session_info(session_id).await;
        assert!(final_session_info.is_some());
        assert_eq!(final_session_info.unwrap().current_state, CallState::Terminated);
        
        println!("Completed test_session_component_isolation");
    }).await;
    
    if result.is_err() {
        panic!("test_session_component_isolation timed out");
    }
}

#[tokio::test]
async fn test_session_edge_case_integration() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_session_edge_case_integration");
        
        let integration_helper = SessionIntegrationHelper::new();
        
        // Test various edge case session IDs
        let long_id = "x".repeat(500);
        let edge_case_sessions = vec![
            ("", "empty_id"),
            ("🦀", "unicode_id"),
            (&long_id, "long_id"),
            ("session with spaces", "spaces_id"),
            ("session\nwith\nnewlines", "newlines_id"),
            ("session\twith\ttabs", "tabs_id"),
            ("session-with_special.chars@123", "special_chars_id"),
        ];
        
        for (session_id, test_name) in edge_case_sessions {
            println!("Testing edge case: {}", test_name);
            
            // Complete session lifecycle with edge case ID
            let creation_result = integration_helper.create_session(session_id, CallState::Initiating).await;
            // Don't assert success - some edge cases might fail
            
            if creation_result.is_ok() {
                let sdp = SessionTestUtils::create_test_sdp();
                integration_helper.setup_media(session_id, &sdp).await.ok();
                integration_helper.update_session_state(session_id, CallState::Ringing).await.ok();
                integration_helper.update_session_state(session_id, CallState::Active).await.ok();
                integration_helper.update_session_state(session_id, CallState::Terminated).await.ok();
                integration_helper.cleanup_media(session_id).await.ok();
            }
        }
        
        // Test rapid session creation and deletion
        for i in 0..10 {
            let rapid_session_id = format!("rapid_session_{}", i);
            integration_helper.create_session(&rapid_session_id, CallState::Initiating).await.ok();
            integration_helper.setup_media(&rapid_session_id, &SessionTestUtils::create_test_sdp()).await.ok();
            integration_helper.update_session_state(&rapid_session_id, CallState::Terminated).await.ok();
            integration_helper.cleanup_media(&rapid_session_id).await.ok();
        }
        
        // Verify system is still functional
        let normal_session = "normal_after_edge_cases";
        integration_helper.create_session(normal_session, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(normal_session, &SessionTestUtils::create_test_sdp()).await.unwrap();
        integration_helper.update_session_state(normal_session, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(normal_session, CallState::Active).await.unwrap();
        integration_helper.update_session_state(normal_session, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(normal_session).await.unwrap();
        
        let session_info = integration_helper.get_session_info(normal_session).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().current_state, CallState::Terminated);
        
        println!("Completed test_session_edge_case_integration");
    }).await;
    
    if result.is_err() {
        panic!("test_session_edge_case_integration timed out");
    }
}

#[tokio::test]
async fn test_comprehensive_session_integration() {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        println!("Starting test_comprehensive_session_integration");
        
        let integration_helper = SessionIntegrationHelper::new();
        let session_id = "comprehensive_integration_session";
        let base_sdp = SessionTestUtils::create_test_sdp();
        
        // Phase 1: Session Creation and Initial Setup
        println!("Phase 1: Session Creation and Initial Setup");
        integration_helper.create_session(session_id, CallState::Initiating).await.unwrap();
        integration_helper.setup_media(session_id, &base_sdp).await.unwrap();
        
        // Phase 2: Call Establishment
        println!("Phase 2: Call Establishment");
        integration_helper.update_session_state(session_id, CallState::Ringing).await.unwrap();
        let ringing_sdp = format!("{}; phase=ringing", base_sdp);
        integration_helper.update_media(session_id, &ringing_sdp).await.unwrap();
        
        // Phase 3: Call Answer and Active State
        println!("Phase 3: Call Answer and Active State");
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        let active_sdp = format!("{}; phase=active;sendrecv", base_sdp);
        integration_helper.update_media(session_id, &active_sdp).await.unwrap();
        
        // Phase 4: Multiple Hold/Resume Cycles
        println!("Phase 4: Multiple Hold/Resume Cycles");
        for cycle in 1..=3 {
            // Hold
            integration_helper.update_session_state(session_id, CallState::OnHold).await.unwrap();
            let hold_sdp = format!("{}; cycle={};sendonly", base_sdp, cycle);
            integration_helper.update_media(session_id, &hold_sdp).await.unwrap();
            
            // Resume
            integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
            let resume_sdp = format!("{}; cycle={};sendrecv", base_sdp, cycle);
            integration_helper.update_media(session_id, &resume_sdp).await.unwrap();
        }
        
        // Phase 5: Media Renegotiation
        println!("Phase 5: Media Renegotiation");
        for renegotiation in 1..=2 {
            let renegotiation_sdp = format!("{}; renegotiation={};codec=updated", base_sdp, renegotiation);
            integration_helper.update_media(session_id, &renegotiation_sdp).await.unwrap();
        }
        
        // Phase 6: Final Hold and Termination
        println!("Phase 6: Final Hold and Termination");
        integration_helper.update_session_state(session_id, CallState::OnHold).await.unwrap();
        let final_hold_sdp = format!("{}; final_hold=true;sendonly", base_sdp);
        integration_helper.update_media(session_id, &final_hold_sdp).await.unwrap();
        
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Phase 7: Comprehensive Verification
        println!("Phase 7: Comprehensive Verification");
        
        // Verify final session state
        let final_session_info = integration_helper.get_session_info(session_id).await;
        assert!(final_session_info.is_some());
        assert_eq!(final_session_info.unwrap().current_state, CallState::Terminated);
        
        // Verify lifecycle events
        let lifecycle_events = integration_helper.get_lifecycle_events().await;
        assert!(lifecycle_events.len() >= 10); // Multiple state changes throughout phases
        
        // Verify media operations
        let media_operations = integration_helper.get_media_operations().await;
        assert!(media_operations.len() >= 10); // Setup + multiple updates + cleanup
        
        // Verify event ordering and consistency
        let mut previous_timestamp = None;
        for event in &lifecycle_events {
            if let Some(prev_ts) = previous_timestamp {
                assert!(event.timestamp >= prev_ts, "Events should be in chronological order");
            }
            previous_timestamp = Some(event.timestamp);
            assert_eq!(event.session_id, session_id, "All events should be for the same session");
        }
        
        // Verify media operation consistency
        let mut previous_media_timestamp = None;
        for operation in &media_operations {
            if let Some(prev_ts) = previous_media_timestamp {
                assert!(operation.timestamp >= prev_ts, "Media operations should be in chronological order");
            }
            previous_media_timestamp = Some(operation.timestamp);
            assert_eq!(operation.session_id, session_id, "All operations should be for the same session");
        }
        
        println!("Completed test_comprehensive_session_integration with {} lifecycle events and {} media operations", 
                 lifecycle_events.len(), media_operations.len());
    }).await;
    
    if result.is_err() {
        panic!("test_comprehensive_session_integration timed out");
    }
}

#[tokio::test]
async fn test_session_integration_helper_functionality() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_integration_helper_functionality");
        
        let integration_helper = SessionIntegrationHelper::new();
        
        // Test helper functionality thoroughly
        let session_id = "helper_test_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // Test session operations
        integration_helper.create_session(session_id, CallState::Initiating).await.unwrap();
        integration_helper.update_session_state(session_id, CallState::Ringing).await.unwrap();
        integration_helper.update_session_state(session_id, CallState::Active).await.unwrap();
        
        // Test media operations
        integration_helper.setup_media(session_id, &sdp).await.unwrap();
        integration_helper.update_media(session_id, &format!("{}; updated", sdp)).await.unwrap();
        
        // Test information retrieval
        let session_info = integration_helper.get_session_info(session_id).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().current_state, CallState::Active);
        
        let lifecycle_events = integration_helper.get_lifecycle_events().await;
        assert!(lifecycle_events.len() > 0);
        
        let media_operations = integration_helper.get_media_operations().await;
        assert!(media_operations.len() > 0);
        
        // Test filtering
        let created_events = integration_helper.get_lifecycle_events_by_type("created").await;
        assert!(created_events.len() > 0);
        
        let state_change_events = integration_helper.get_lifecycle_events_by_type("state_change").await;
        assert!(state_change_events.len() >= 2); // Initiating->Ringing, Ringing->Active
        
        let session_events = integration_helper.get_lifecycle_events_by_session(session_id).await;
        assert!(session_events.len() > 0);
        
        let setup_operations = integration_helper.get_media_operations_by_type("setup").await;
        assert_eq!(setup_operations.len(), 1);
        
        let update_operations = integration_helper.get_media_operations_by_type("update").await;
        assert_eq!(update_operations.len(), 1);
        
        let session_operations = integration_helper.get_media_operations_by_session(session_id).await;
        assert_eq!(session_operations.len(), 2); // setup + update
        
        // Test cleanup
        integration_helper.update_session_state(session_id, CallState::Terminated).await.unwrap();
        integration_helper.cleanup_media(session_id).await.unwrap();
        
        // Test history clearing
        integration_helper.clear_lifecycle_events().await;
        integration_helper.clear_media_operations().await;
        
        let cleared_events = integration_helper.get_lifecycle_events().await;
        let cleared_operations = integration_helper.get_media_operations().await;
        
        assert_eq!(cleared_events.len(), 0);
        assert_eq!(cleared_operations.len(), 0);
        
        println!("Completed test_session_integration_helper_functionality");
    }).await;
    
    if result.is_err() {
        panic!("test_session_integration_helper_functionality timed out");
    }
} 