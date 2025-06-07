//! Session Media Coordination Tests
//!
//! Tests for MediaCoordinator functionality including media setup, updates,
//! cleanup operations, and media-related edge cases.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::CallState,
    session::media::MediaCoordinator,
    SessionError,
};
use common::*;

#[tokio::test]
async fn test_media_coordinator_creation() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_coordinator_creation");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test basic functionality exists
        let session_id = "test_session_1";
        let sdp = SessionTestUtils::create_test_sdp();
        
        let setup_result = helper.setup_media(session_id, &sdp).await;
        assert!(setup_result.is_ok(), "Media setup should succeed");
        
        let cleanup_result = helper.cleanup_media(session_id).await;
        assert!(cleanup_result.is_ok(), "Media cleanup should succeed");
        
        println!("Completed test_media_coordinator_creation");
    }).await;
    
    if result.is_err() {
        panic!("test_media_coordinator_creation timed out");
    }
}

#[tokio::test]
async fn test_media_setup_operations() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_setup_operations");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test media setup for multiple sessions
        let sessions = vec![
            ("session_1", SessionTestUtils::create_test_sdp()),
            ("session_2", SessionTestUtils::create_test_sdp()),
            ("session_3", SessionTestUtils::create_test_sdp()),
        ];
        
        for (session_id, sdp) in &sessions {
            let result = helper.setup_media(session_id, sdp).await;
            assert!(result.is_ok(), "Media setup should succeed for session {}", session_id);
        }
        
        // Verify media setup history
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), 3);
        
        for (i, (session_id, _)) in sessions.iter().enumerate() {
            let operation = &operations[i];
            assert_eq!(operation.operation_type, "setup");
            assert_eq!(operation.session_id, *session_id);
            assert!(operation.success);
        }
        
        // Clean up all sessions
        for (session_id, _) in &sessions {
            let result = helper.cleanup_media(session_id).await;
            assert!(result.is_ok(), "Media cleanup should succeed for session {}", session_id);
        }
        
        println!("Completed test_media_setup_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_media_setup_operations timed out");
    }
}

#[tokio::test]
async fn test_media_update_operations() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_update_operations");
        
        let helper = MediaCoordinatorTestHelper::new();
        let session_id = "update_test_session";
        
        // Setup initial media
        let initial_sdp = SessionTestUtils::create_test_sdp();
        let setup_result = helper.setup_media(session_id, &initial_sdp).await;
        assert!(setup_result.is_ok(), "Initial media setup should succeed");
        
        // Update media multiple times
        for i in 1..=5 {
            let updated_sdp = format!("{}; updated_version={}", initial_sdp, i);
            let update_result = helper.update_media(session_id, &updated_sdp).await;
            assert!(update_result.is_ok(), "Media update {} should succeed", i);
        }
        
        // Verify operation history
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), 6); // 1 setup + 5 updates
        
        // Check setup operation
        assert_eq!(operations[0].operation_type, "setup");
        assert_eq!(operations[0].session_id, session_id);
        assert!(operations[0].success);
        
        // Check update operations
        for i in 1..=5 {
            let operation = &operations[i];
            assert_eq!(operation.operation_type, "update");
            assert_eq!(operation.session_id, session_id);
            assert!(operation.success);
        }
        
        // Cleanup
        let cleanup_result = helper.cleanup_media(session_id).await;
        assert!(cleanup_result.is_ok(), "Media cleanup should succeed");
        
        println!("Completed test_media_update_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_media_update_operations timed out");
    }
}

#[tokio::test]
async fn test_media_cleanup_operations() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_cleanup_operations");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Setup multiple sessions
        let session_ids = vec!["cleanup_1", "cleanup_2", "cleanup_3"];
        for session_id in &session_ids {
            let sdp = SessionTestUtils::create_test_sdp();
            let result = helper.setup_media(session_id, &sdp).await;
            assert!(result.is_ok(), "Media setup should succeed for {}", session_id);
        }
        
        // Cleanup sessions one by one
        for session_id in &session_ids {
            let result = helper.cleanup_media(session_id).await;
            assert!(result.is_ok(), "Media cleanup should succeed for {}", session_id);
        }
        
        // Verify operation history
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), 6); // 3 setups + 3 cleanups
        
        // Check all operations
        for i in 0..3 {
            // Setup operations
            let setup_op = &operations[i];
            assert_eq!(setup_op.operation_type, "setup");
            assert_eq!(setup_op.session_id, session_ids[i]);
            assert!(setup_op.success);
            
            // Cleanup operations
            let cleanup_op = &operations[i + 3];
            assert_eq!(cleanup_op.operation_type, "cleanup");
            assert_eq!(cleanup_op.session_id, session_ids[i]);
            assert!(cleanup_op.success);
        }
        
        println!("Completed test_media_cleanup_operations");
    }).await;
    
    if result.is_err() {
        panic!("test_media_cleanup_operations timed out");
    }
}

#[tokio::test]
async fn test_media_operations_without_setup() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_operations_without_setup");
        
        let helper = MediaCoordinatorTestHelper::new();
        let session_id = "no_setup_session";
        
        // Try to update media without setup
        let sdp = SessionTestUtils::create_test_sdp();
        let update_result = helper.update_media(session_id, &sdp).await;
        
        // This might succeed or fail depending on implementation
        // Let's verify the operation was recorded
        let operations = helper.get_operations().await;
        assert!(operations.len() > 0);
        assert_eq!(operations[0].operation_type, "update");
        assert_eq!(operations[0].session_id, session_id);
        
        // Try to cleanup media without setup
        let cleanup_result = helper.cleanup_media(session_id).await;
        
        // Verify both operations were recorded
        let all_operations = helper.get_operations().await;
        assert_eq!(all_operations.len(), 2);
        assert_eq!(all_operations[1].operation_type, "cleanup");
        assert_eq!(all_operations[1].session_id, session_id);
        
        println!("Completed test_media_operations_without_setup");
    }).await;
    
    if result.is_err() {
        panic!("test_media_operations_without_setup timed out");
    }
}

#[tokio::test]
async fn test_concurrent_media_operations() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_concurrent_media_operations");
        
        let helper = MediaCoordinatorTestHelper::new();
        let concurrent_sessions = 10;
        let mut handles = Vec::new();
        
        // Spawn concurrent media operations
        for session_num in 0..concurrent_sessions {
            let helper_clone = MediaCoordinatorTestHelper::new();
            let session_id = format!("concurrent_session_{}", session_num);
            
            let handle = tokio::spawn(async move {
                let sdp = SessionTestUtils::create_test_sdp();
                
                // Setup media
                let setup_result = helper_clone.setup_media(&session_id, &sdp).await;
                assert!(setup_result.is_ok(), "Setup should succeed for {}", session_id);
                
                // Update media
                let updated_sdp = format!("{}; concurrent_update", sdp);
                let update_result = helper_clone.update_media(&session_id, &updated_sdp).await;
                assert!(update_result.is_ok(), "Update should succeed for {}", session_id);
                
                // Cleanup media
                let cleanup_result = helper_clone.cleanup_media(&session_id).await;
                assert!(cleanup_result.is_ok(), "Cleanup should succeed for {}", session_id);
                
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
        
        println!("Completed test_concurrent_media_operations with {} sessions", concurrent_sessions);
    }).await;
    
    if result.is_err() {
        panic!("test_concurrent_media_operations timed out");
    }
}

#[tokio::test]
async fn test_media_operation_error_handling() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_operation_error_handling");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test with invalid SDP
        let session_id = "error_test_session";
        let invalid_sdp = "invalid_sdp_content";
        
        // Try operations with invalid SDP
        let setup_result = helper.setup_media(session_id, invalid_sdp).await;
        let update_result = helper.update_media(session_id, invalid_sdp).await;
        let cleanup_result = helper.cleanup_media(session_id).await;
        
        // Operations might succeed or fail, but should be recorded
        let operations = helper.get_operations().await;
        assert!(operations.len() >= 3);
        
        // Test with empty session ID
        let empty_session_result = helper.setup_media("", &SessionTestUtils::create_test_sdp()).await;
        
        // Test with very long session ID
        let long_session_id = "x".repeat(1000);
        let long_id_result = helper.setup_media(&long_session_id, &SessionTestUtils::create_test_sdp()).await;
        
        // Verify all operations were recorded
        let all_operations = helper.get_operations().await;
        assert!(all_operations.len() >= 5);
        
        println!("Completed test_media_operation_error_handling");
    }).await;
    
    if result.is_err() {
        panic!("test_media_operation_error_handling timed out");
    }
}

#[tokio::test]
async fn test_media_operation_history() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_operation_history");
        
        let helper = MediaCoordinatorTestHelper::new();
        let session_id = "history_test_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // Perform a sequence of operations
        helper.setup_media(session_id, &sdp).await;
        helper.update_media(session_id, &format!("{}; update1", sdp)).await;
        helper.update_media(session_id, &format!("{}; update2", sdp)).await;
        helper.cleanup_media(session_id).await;
        
        // Test history retrieval
        let all_operations = helper.get_operations().await;
        assert_eq!(all_operations.len(), 4);
        
        // Test filtering by operation type
        let setup_ops = helper.get_operations_by_type("setup").await;
        assert_eq!(setup_ops.len(), 1);
        assert_eq!(setup_ops[0].session_id, session_id);
        
        let update_ops = helper.get_operations_by_type("update").await;
        assert_eq!(update_ops.len(), 2);
        
        let cleanup_ops = helper.get_operations_by_type("cleanup").await;
        assert_eq!(cleanup_ops.len(), 1);
        
        // Test filtering by session
        let session_ops = helper.get_operations_by_session(session_id).await;
        assert_eq!(session_ops.len(), 4);
        
        // Test clearing history
        helper.clear_operations().await;
        let cleared_ops = helper.get_operations().await;
        assert_eq!(cleared_ops.len(), 0);
        
        println!("Completed test_media_operation_history");
    }).await;
    
    if result.is_err() {
        panic!("test_media_operation_history timed out");
    }
}

#[tokio::test]
async fn test_media_sdp_variations() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_sdp_variations");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test with different SDP variations
        let standard_sdp = SessionTestUtils::create_test_sdp();
        let large_sdp = format!("v=0\r\ns={}", "x".repeat(1000));
        let sdp_variations = vec![
            ("empty", ""),
            ("minimal", "v=0"),
            ("standard", standard_sdp.as_str()),
            ("with_audio", "v=0\r\nm=audio 49170 RTP/AVP 0"),
            ("with_video", "v=0\r\nm=video 5004 RTP/AVP 96"),
            ("complex", "v=0\r\no=alice 2890844526 2890844527 IN IP4 host.atlanta.com\r\ns=-\r\nc=IN IP4 host.atlanta.com\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000"),
            ("unicode", "v=0\r\ns=🦀 Rust Session"),
            ("large", large_sdp.as_str()),
        ];
        
        let variations_len = sdp_variations.len();
        for (variant_name, sdp_content) in sdp_variations {
            let session_id = format!("sdp_test_{}", variant_name);
            
            // Test setup with this SDP variation
            let setup_result = helper.setup_media(&session_id, sdp_content).await;
            // Don't assert success/failure as some variations might be invalid
            
            // Test update with this SDP variation
            let update_result = helper.update_media(&session_id, sdp_content).await;
            
            // Test cleanup
            let cleanup_result = helper.cleanup_media(&session_id).await;
        }
        
        // Verify all operations were recorded
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), variations_len * 3); // setup + update + cleanup
        
        println!("Completed test_media_sdp_variations");
    }).await;
    
    if result.is_err() {
        panic!("test_media_sdp_variations timed out");
    }
}

#[tokio::test]
async fn test_media_performance() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_media_performance");
        
        let helper = MediaCoordinatorTestHelper::new();
        let operation_count = 100;
        let sdp = SessionTestUtils::create_test_sdp();
        
        let start = std::time::Instant::now();
        
        // Perform many media operations quickly
        for i in 0..operation_count {
            let session_id = format!("perf_session_{}", i);
            
            helper.setup_media(&session_id, &sdp).await;
            helper.update_media(&session_id, &format!("{}; perf_update", sdp)).await;
            helper.cleanup_media(&session_id).await;
        }
        
        let duration = start.elapsed();
        println!("Performed {} media operations in {:?}", operation_count * 3, duration);
        
        // Verify all operations were recorded
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), operation_count * 3);
        
        // Performance assertion
        assert!(duration < Duration::from_secs(10), "Media operations took too long");
        
        println!("Completed test_media_performance");
    }).await;
    
    if result.is_err() {
        panic!("test_media_performance timed out");
    }
}

#[tokio::test]
async fn test_media_lifecycle_integration() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_lifecycle_integration");
        
        let helper = MediaCoordinatorTestHelper::new();
        let session_id = "lifecycle_integration_session";
        let sdp = SessionTestUtils::create_test_sdp();
        
        // Simulate complete session media lifecycle
        
        // 1. Initial setup
        let setup_result = helper.setup_media(session_id, &sdp).await;
        assert!(setup_result.is_ok(), "Initial media setup should succeed");
        
        // 2. Early update (during negotiation)
        let negotiation_sdp = format!("{}; negotiating", sdp);
        let negotiation_result = helper.update_media(session_id, &negotiation_sdp).await;
        assert!(negotiation_result.is_ok(), "Negotiation update should succeed");
        
        // 3. Established media
        let established_sdp = format!("{}; established", sdp);
        let established_result = helper.update_media(session_id, &established_sdp).await;
        assert!(established_result.is_ok(), "Established update should succeed");
        
        // 4. Hold scenario (media pause)
        let hold_sdp = format!("{}; sendonly", sdp);
        let hold_result = helper.update_media(session_id, &hold_sdp).await;
        assert!(hold_result.is_ok(), "Hold update should succeed");
        
        // 5. Resume scenario (media resume)
        let resume_sdp = format!("{}; sendrecv", sdp);
        let resume_result = helper.update_media(session_id, &resume_sdp).await;
        assert!(resume_result.is_ok(), "Resume update should succeed");
        
        // 6. Final cleanup
        let cleanup_result = helper.cleanup_media(session_id).await;
        assert!(cleanup_result.is_ok(), "Final cleanup should succeed");
        
        // Verify complete operation sequence
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), 6); // setup + 4 updates + cleanup
        
        let expected_operations = vec!["setup", "update", "update", "update", "update", "cleanup"];
        for (i, expected_op) in expected_operations.iter().enumerate() {
            assert_eq!(operations[i].operation_type, *expected_op);
            assert_eq!(operations[i].session_id, session_id);
            assert!(operations[i].success, "Operation {} should be successful", i);
        }
        
        println!("Completed test_media_lifecycle_integration");
    }).await;
    
    if result.is_err() {
        panic!("test_media_lifecycle_integration timed out");
    }
}

#[tokio::test]
async fn test_media_edge_cases() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_edge_cases");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test edge case session IDs
        let long_id = "x".repeat(1000);
        let edge_case_ids = vec![
            "",                           // Empty
            "🦀",                        // Unicode
            "session with spaces",        // Spaces
            "session\nwith\nnewlines",   // Newlines
            "session\twith\ttabs",       // Tabs
            &long_id,                    // Very long
            "session-with_special.chars@123", // Special characters
        ];
        
        let sdp = SessionTestUtils::create_test_sdp();
        
        for session_id in edge_case_ids {
            // Test all operations with edge case session IDs
            helper.setup_media(session_id, &sdp).await;
            helper.update_media(session_id, &sdp).await;
            helper.cleanup_media(session_id).await;
        }
        
        // Test multiple operations on same session
        let duplicate_session = "duplicate_ops_session";
        helper.setup_media(duplicate_session, &sdp).await;
        helper.setup_media(duplicate_session, &sdp).await; // Duplicate setup
        helper.update_media(duplicate_session, &sdp).await;
        helper.update_media(duplicate_session, &sdp).await; // Duplicate update
        helper.cleanup_media(duplicate_session).await;
        helper.cleanup_media(duplicate_session).await; // Duplicate cleanup
        
        // Verify all operations were recorded
        let operations = helper.get_operations().await;
        assert!(operations.len() > 0);
        
        println!("Completed test_media_edge_cases with {} operations", operations.len());
    }).await;
    
    if result.is_err() {
        panic!("test_media_edge_cases timed out");
    }
}

#[tokio::test]
async fn test_media_helper_robustness() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_helper_robustness");
        
        let helper = MediaCoordinatorTestHelper::new();
        
        // Test that helper methods don't panic or fail unexpectedly
        
        // Test operations retrieval when empty
        let empty_operations = helper.get_operations().await;
        assert_eq!(empty_operations.len(), 0);
        
        // Test filtering when empty
        let empty_setup_ops = helper.get_operations_by_type("setup").await;
        assert_eq!(empty_setup_ops.len(), 0);
        
        let empty_session_ops = helper.get_operations_by_session("nonexistent").await;
        assert_eq!(empty_session_ops.len(), 0);
        
        // Test clearing when empty
        helper.clear_operations().await;
        
        // Add some operations
        let session_id = "robustness_test";
        let sdp = SessionTestUtils::create_test_sdp();
        
        helper.setup_media(session_id, &sdp).await;
        helper.update_media(session_id, &sdp).await;
        helper.cleanup_media(session_id).await;
        
        // Test operations retrieval after adding
        let operations = helper.get_operations().await;
        assert_eq!(operations.len(), 3);
        
        // Test filtering after adding
        let setup_ops = helper.get_operations_by_type("setup").await;
        assert_eq!(setup_ops.len(), 1);
        
        let session_ops = helper.get_operations_by_session(session_id).await;
        assert_eq!(session_ops.len(), 3);
        
        // Test clearing after adding
        helper.clear_operations().await;
        let cleared_operations = helper.get_operations().await;
        assert_eq!(cleared_operations.len(), 0);
        
        println!("Completed test_media_helper_robustness");
    }).await;
    
    if result.is_err() {
        panic!("test_media_helper_robustness timed out");
    }
} 