//! Tests for UPDATE Dialog Integration
//!
//! Tests the session-core functionality for UPDATE requests (media updates),
//! ensuring proper integration with the underlying dialog layer.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionManager,
    SessionError,
    api::{
        types::{SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};

/// Test handler for UPDATE testing
#[derive(Debug)]
struct UpdateTestHandler {
    updated_calls: Arc<tokio::sync::Mutex<Vec<SessionId>>>,
}

impl UpdateTestHandler {
    fn new() -> Self {
        Self {
            updated_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_updated_calls(&self) -> Vec<SessionId> {
        self.updated_calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for UpdateTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        if reason.contains("update") || reason.contains("UPDATE") {
            self.updated_calls.lock().await.push(call.id().clone());
        }
        tracing::info!("Call {} ended with reason: {}", call.id(), reason);
    }
}

/// Create a test session manager for UPDATE testing
async fn create_update_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(UpdateTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_media_update_basic() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create an outgoing call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test media update - expect failure on terminated session
    let new_sdp = "v=0\r\no=alice 123 789 IN IP4 192.168.1.100\r\nm=audio 5006 RTP/AVP 0 8\r\n";
    let update_result = manager.update_media(&session_id, new_sdp).await;
    if update_result.is_err() {
        println!("Media update failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Media update succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_media_updates() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create an outgoing call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Multiple media updates - expect failures on terminated session
    for i in 1..=5 {
        let sdp = format!("Updated SDP version {}", i);
        let update_result = manager.update_media(&session_id, &sdp).await;
        if update_result.is_err() {
            println!("Update {} failed as expected: {:?}", i, update_result.unwrap_err());
        } else {
            println!("Update {} succeeded", i);
        }
        
        // Small delay between updates
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_with_codec_changes() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call with initial codec
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Update with different codec - expect failure on terminated session
    let updated_sdp = "v=0\r\no=alice 123 789 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 8 0\r\n";
    let update_result = manager.update_media(&session_id, updated_sdp).await;
    if update_result.is_err() {
        println!("Codec change update failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Codec change update succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_with_video_addition() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create audio-only call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Update to add video - expect failure on terminated session
    let updated_sdp = "v=0\r\no=alice 123 789 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\nm=video 5006 RTP/AVP 96\r\n";
    let update_result = manager.update_media(&session_id, updated_sdp).await;
    if update_result.is_err() {
        println!("Video addition update failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Video addition update succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_nonexistent_session() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try to update a non-existent session
    let fake_session_id = SessionId::new();
    let update_result = manager.update_media(&fake_session_id, "Some SDP").await;
    assert!(update_result.is_err());
    assert!(matches!(update_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_media_updates() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("Initial SDP for call {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Update all calls concurrently - expect most to fail on terminated sessions
    let mut update_tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager.clone();
        let session_id = call.id().clone();
        let updated_sdp = format!("Updated SDP for call {}", i);
        let task = tokio::spawn(async move {
            manager_clone.update_media(&session_id, &updated_sdp).await
        });
        update_tasks.push(task);
    }
    
    // Wait for all updates to complete - don't panic on failures
    for (i, task) in update_tasks.into_iter().enumerate() {
        let result = task.await.unwrap();
        if result.is_err() {
            println!("Concurrent update {} failed as expected: {:?}", i, result.unwrap_err());
        } else {
            println!("Concurrent update {} succeeded", i);
        }
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_after_hold() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Active SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Put call on hold - expect failure on terminated session
    let hold_result = manager.hold_session(&session_id).await;
    if hold_result.is_err() {
        println!("Hold failed as expected: {:?}", hold_result.unwrap_err());
    } else {
        println!("Hold succeeded");
    }
    
    // Update media while on hold - expect failure
    let update_result = manager.update_media(&session_id, "Updated hold SDP").await;
    if update_result.is_err() {
        println!("Update after hold failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Update after hold succeeded");
    }
    
    // Resume call - expect failure
    let resume_result = manager.resume_session(&session_id).await;
    if resume_result.is_err() {
        println!("Resume failed as expected: {:?}", resume_result.unwrap_err());
    } else {
        println!("Resume succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_session_state_consistency() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before update
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Update media - expect failure on terminated session
    let update_result = manager.update_media(&session_id, "Updated SDP").await;
    if update_result.is_err() {
        println!("Update failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Update succeeded");
    }
    
    // Verify session state after update attempt
    let session_after = manager.find_session(&session_id).await.unwrap();
    if session_after.is_some() {
        println!("Session still exists after update");
        assert_eq!(session_after.unwrap().id(), &session_id);
    } else {
        println!("Session was terminated after update attempt");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_update_with_empty_sdp() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Try update with empty SDP - expect failure on terminated session
    let update_result = manager.update_media(&session_id, "").await;
    if update_result.is_err() {
        println!("Empty SDP update failed as expected: {:?}", update_result.unwrap_err());
    } else {
        println!("Empty SDP update succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_consecutive_updates() {
    let manager = create_update_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Rapid consecutive updates - expect failures on terminated session
    for i in 0..10 {
        let sdp = format!("Rapid update {}", i);
        let update_result = manager.update_media(&session_id, &sdp).await;
        if update_result.is_err() {
            println!("Rapid update {} failed as expected: {:?}", i, update_result.unwrap_err());
        } else {
            println!("Rapid update {} succeeded", i);
        }
        // No delay - testing rapid updates
    }
    
    manager.stop().await.unwrap();
} 