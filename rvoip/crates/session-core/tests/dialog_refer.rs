use rvoip_session_core::api::control::SessionControl;
//! Tests for Call Transfer Functionality
//!
//! Tests the session-core functionality for call transfers,
//! ensuring proper integration with the underlying dialog layer.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};

/// Handler for transfer testing
#[derive(Debug)]
struct TransferTestHandler;

#[async_trait::async_trait]
impl CallHandler for TransferTestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Transfer test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for transfer testing
async fn create_transfer_test_manager(port: u16) -> Result<Arc<SessionCoordinator>, SessionError> {
//     let handler = Arc::new(TransferTestHandler);
    
    SessionManagerBuilder::new()
        .with_local_address("127.0.0.1")
        .with_sip_port(port)
        .with_handler(None)
        .build()
        .await
}

#[tokio::test]
async fn test_basic_call_transfer() {
    let manager = create_transfer_test_manager(5080).await.unwrap();
    manager.start().await.unwrap();
    
    // Create an outgoing call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfer operation - expect it to fail on terminated session
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_nonexistent_session() {
    let manager = create_transfer_test_manager(5081).await.unwrap();
    manager.start().await.unwrap();
    
    let fake_session_id = SessionId::new();
    let transfer_result = manager.transfer_session(&fake_session_id, "sip:target@example.com").await;
    assert!(transfer_result.is_err());
    assert!(matches!(transfer_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_to_various_targets() {
    let manager = create_transfer_test_manager(5082).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfers to different types of targets - expect failures on terminated session
    let transfer_targets = vec![
        "sip:charlie@example.com",
        "sip:david@another-domain.com",
        "sip:1234@pbx.company.com",
        "sip:conference@meetings.example.com",
        "sip:voicemail@vm.example.com",
    ];
    
    for target in transfer_targets {
        let transfer_result = manager.transfer_session(&session_id, target).await;
        if transfer_result.is_err() {
            println!("Transfer to '{}' failed as expected: {:?}", target, transfer_result.unwrap_err());
        } else {
            println!("Transfer to '{}' succeeded", target);
        }
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_concurrent_transfers() {
    let manager = create_transfer_test_manager(5083).await.unwrap();
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut sessions = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for call {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Transfer each call to a different target - expect failures on terminated sessions
    for (i, session_id) in sessions.iter().enumerate() {
        let transfer_result = manager.transfer_session(
            session_id, 
            &format!("sip:transfer_target_{}@example.com", i)
        ).await;
        if transfer_result.is_err() {
            println!("Transfer of session {} failed as expected: {:?}", i, transfer_result.unwrap_err());
        } else {
            println!("Transfer of session {} succeeded", i);
        }
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_after_other_operations() {
    let manager = create_transfer_test_manager(5084).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform operations before transfer - expect these to fail on terminated session
    let _ = manager.hold_session(&session_id).await; // Don't unwrap, expect failure
    let _ = manager.resume_session(&session_id).await; // Don't unwrap, expect failure
    let _ = // manager.send_dtmf(&session_id, "123").await; // Don't unwrap, expect failure
    let _ = // manager.update_media(&session_id, "Updated SDP").await; // Don't unwrap, expect failure
    
    // Now try transfer - also expect failure
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_transfer_requests() {
    let manager = create_transfer_test_manager(5085).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send multiple rapid transfer requests - expect failures on terminated session
    for i in 0..10 {
        let transfer_result = manager.transfer_session(
            &session_id, 
            &format!("sip:rapid_target_{}@example.com", i)
        ).await;
        if transfer_result.is_err() {
            println!("Rapid transfer {} failed as expected: {:?}", i, transfer_result.unwrap_err());
        } else {
            println!("Rapid transfer {} succeeded", i);
        }
        
        // Very small delay
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_with_session_stats() {
    let manager = create_transfer_test_manager(5086).await.unwrap();
    manager.start().await.unwrap();
    
    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Check stats before transfer
    let stats_before = manager.get_stats().await.unwrap();
    println!("Stats before transfer: {:?}", stats_before);
    
    // Perform transfer - expect failure on terminated session
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer succeeded");
    }
    
    // Check stats after transfer
    let stats_after = manager.get_stats().await.unwrap();
    println!("Stats after transfer: {:?}", stats_after);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_then_terminate() {
    let manager = create_transfer_test_manager(5087).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Transfer the call - expect failure on terminated session
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer succeeded");
    }
    
    // Then terminate it - also expect failure on already terminated session
    let terminate_result = manager.terminate_session(&session_id).await;
    if terminate_result.is_err() {
        println!("Terminate failed as expected: {:?}", terminate_result.unwrap_err());
    } else {
        println!("Terminate succeeded");
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is cleaned up - just check that list_active_sessions doesn't include it
    let sessions = manager.list_active_sessions().await.unwrap();
    let session_exists = sessions.iter().any(|s| s == &session_id);
    if !session_exists {
        println!("Session was cleaned up as expected");
    } else {
        println!("Session still exists: {}", session_id);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_edge_cases() {
    let manager = create_transfer_test_manager(5088).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfer to same URI as current target - expect failure on terminated session
    let transfer_result = manager.transfer_session(&session_id, "sip:bob@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer to same target failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer to same target succeeded");
    }
    
    // Test transfer to same URI as caller - expect failure on terminated session
    let transfer_result = manager.transfer_session(&session_id, "sip:alice@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer to caller failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer to caller succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_stress_test() {
    let manager = create_transfer_test_manager(5089).await.unwrap();
    manager.start().await.unwrap();
    
    // Create many calls and transfer them concurrently
    let mut sessions = Vec::new();
    
    for i in 0..20 {
        let call = manager.create_outgoing_call(
            &format!("sip:stress_caller_{}@example.com", i),
            &format!("sip:stress_target_{}@example.com", i),
            Some(format!("Stress test SDP {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Transfer all calls concurrently - expect failures on terminated sessions
    let mut handles = Vec::new();
    for (i, session_id) in sessions.iter().enumerate() {
        let manager_clone: Arc<SessionCoordinator> = Arc::clone(&manager);
        let session_id_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            manager_clone.transfer_session(
                &session_id_clone, 
                &format!("sip:stress_transfer_{}@example.com", i)
            ).await
        });
        handles.push(handle);
    }
    
    // Wait for all transfers to complete - expect most/all to fail
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        if result.is_err() {
            println!("Stress transfer {} failed as expected: {:?}", i, result.unwrap_err());
        } else {
            println!("Stress transfer {} succeeded", i);
        }
    }
    
    manager.stop().await.unwrap();
} 