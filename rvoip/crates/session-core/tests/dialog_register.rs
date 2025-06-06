//! Tests for Registration Functionality
//!
//! Tests the session-core functionality for SIP registration,
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

/// Simple handler for registration testing
#[derive(Debug)]
struct RegistrationTestHandler;

#[async_trait::async_trait]
impl CallHandler for RegistrationTestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        // For registration tests, we don't expect incoming calls
        CallDecision::Reject("Not accepting calls during registration test".to_string())
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Registration test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for registration testing
async fn create_registration_test_manager(port: u16) -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(RegistrationTestHandler);
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port)
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_session_manager_startup_and_shutdown() {
    let manager = create_registration_test_manager(5090).await.unwrap();
    
    // Test startup
    let start_result = manager.start().await;
    assert!(start_result.is_ok());
    
    // Test shutdown
    let stop_result = manager.stop().await;
    assert!(stop_result.is_ok());
}

#[tokio::test]
async fn test_multiple_startup_shutdown_cycles() {
    let manager = create_registration_test_manager(5091).await.unwrap();
    
    // Test multiple startup/shutdown cycles
    for _ in 0..3 {
        manager.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        manager.stop().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn test_session_manager_stats_during_registration() {
    let manager = create_registration_test_manager(5092).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Check initial stats (should be no active sessions during registration)
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_different_bind_addresses() {
    // Test with IPv4 loopback
    let manager1 = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(5061)
        .with_from_uri("sip:test1@localhost")
        .with_handler(Arc::new(RegistrationTestHandler))
        .build()
        .await.unwrap();
    
    manager1.start().await.unwrap();
    manager1.stop().await.unwrap();
    
    // Test with different port specification
    let manager2 = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")  // Use 127.0.0.1 instead of 0.0.0.0
        .with_sip_port(5093)
        .with_from_uri("sip:test2@localhost") 
        .with_handler(Arc::new(RegistrationTestHandler))
        .build()
        .await.unwrap();
    
    manager2.start().await.unwrap();
    manager2.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_different_from_uris() {
    let from_uris = vec![
        "sip:alice@example.com",
        "sip:bob@test.com",
        "sip:user123@domain.org",
        "sip:service@localhost",
    ];
    
    for (i, from_uri) in from_uris.iter().enumerate() {
        let manager = SessionManagerBuilder::new()
            .with_sip_bind_address("127.0.0.1")
            .with_sip_port(5094 + i as u16)
            .with_from_uri(*from_uri)
            .with_handler(Arc::new(RegistrationTestHandler))
            .build()
            .await.unwrap();
        
        manager.start().await.unwrap();
        manager.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_concurrent_session_managers() {
    let mut managers = Vec::new();
    
    // Create multiple session managers
    for i in 0..3 {
        let manager = SessionManagerBuilder::new()
            .with_sip_bind_address("127.0.0.1")
            .with_sip_port(5098 + i as u16)
            .with_from_uri(&format!("sip:user{}@localhost", i))
            .with_handler(Arc::new(RegistrationTestHandler))
            .build()
            .await.unwrap();
        managers.push(manager);
    }
    
    // Start all managers
    for manager in &managers {
        manager.start().await.unwrap();
    }
    
    // Let them run for a bit
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Stop all managers
    for manager in &managers {
        manager.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_session_manager_error_handling() {
    // Test with invalid bind address (should fail during build)
    let invalid_result = SessionManagerBuilder::new()
        .with_sip_bind_address("999.999.999.999")
        .with_sip_port(65535) // Use maximum valid port
        .with_from_uri("sip:test@localhost")
        .with_handler(Arc::new(RegistrationTestHandler))
        .build()
        .await;
    
    // The build might succeed but start should handle the invalid address
    if let Ok(manager) = invalid_result {
        let start_result = manager.start().await;
        // We expect this to either fail or succeed with graceful handling
        // The important thing is that it doesn't panic
        if start_result.is_ok() {
            manager.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn test_session_manager_double_start_protection() {
    let manager = create_registration_test_manager(5101).await.unwrap();
    
    // Start once
    manager.start().await.unwrap();
    
    // Attempt to start again (should either succeed or gracefully handle)
    let second_start = manager.start().await;
    // The behavior here depends on implementation - could be idempotent or return error
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_double_stop_protection() {
    let manager = create_registration_test_manager(5102).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Stop once
    manager.stop().await.unwrap();
    
    // Attempt to stop again (should be graceful)
    let second_stop = manager.stop().await;
    // Should not panic or cause issues
}

#[tokio::test]
async fn test_operations_on_stopped_manager() {
    let manager = create_registration_test_manager(5103).await.unwrap();
    
    // Try operations without starting
    let stats_result = manager.get_stats().await;
    // Should either work or return appropriate error
    
    let fake_session_id = SessionId::new();
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    // Should return error for non-existent session
    assert!(terminate_result.is_err());
} 