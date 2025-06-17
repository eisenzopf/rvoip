use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        control::SessionControl,
        handlers::CallHandler,
        builder::SessionManagerBuilder,
        types::{IncomingCall, CallSession, CallDecision, CallState},
    },
};

/// Simple handler that accepts all calls
#[derive(Debug)]
struct TestHandler;

#[async_trait::async_trait]
impl CallHandler for TestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }

    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
}

#[tokio::test]
async fn test_wait_for_answer_already_active() {
    // Create session manager
    let manager = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(5900)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .unwrap();

    manager.start().await.unwrap();

    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Manually update state to Active (simulating answered call)
    if let Ok(Some(mut session)) = manager.registry.get_session(&session_id).await {
        session.state = CallState::Active;
        let _ = manager.registry.register_session(session_id.clone(), session).await;
    }
    
    // wait_for_answer should return immediately since it's already active
    let result = manager.wait_for_answer(&session_id, Duration::from_secs(5)).await;
    assert!(result.is_ok(), "Should succeed for already active call");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_wait_for_answer_timeout() {
    // Create session manager
    let manager = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(5901)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .unwrap();

    manager.start().await.unwrap();

    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait for answer with short timeout (call will stay in Initiating state)
    let result = manager.wait_for_answer(&session_id, Duration::from_millis(100)).await;
    assert!(result.is_err(), "Should timeout");
    
    if let Err(e) = result {
        assert!(matches!(e, SessionError::Timeout(_)), "Should be timeout error");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_wait_for_answer_failed_state() {
    // Create session manager
    let manager = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(5902)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .unwrap();

    manager.start().await.unwrap();

    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Manually update state to Failed
    if let Ok(Some(mut session)) = manager.registry.get_session(&session_id).await {
        session.state = CallState::Failed("Test failure".to_string());
        let _ = manager.registry.register_session(session_id.clone(), session).await;
    }
    
    // wait_for_answer should return error for failed call
    let result = manager.wait_for_answer(&session_id, Duration::from_secs(5)).await;
    assert!(result.is_err(), "Should fail for failed call");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_wait_for_answer_nonexistent_session() {
    // Create session manager
    let manager = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(5903)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .unwrap();

    manager.start().await.unwrap();

    // Try to wait for a non-existent session
    let fake_session_id = rvoip_session_core::api::types::SessionId::new();
    let result = manager.wait_for_answer(&fake_session_id, Duration::from_secs(1)).await;
    
    assert!(result.is_err(), "Should fail for non-existent session");
    assert!(matches!(result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_wait_for_answer_with_state_transition() {
    // Create session manager
    let manager = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(5904)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .unwrap();

    manager.start().await.unwrap();

    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Spawn a task to change state after a delay
    let manager_clone = manager.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Update state to Active
        if let Ok(Some(mut session)) = manager_clone.registry.get_session(&session_id_clone).await {
            let old_state = session.state.clone();
            session.state = CallState::Active;
            let _ = manager_clone.registry.register_session(session_id_clone.clone(), session).await;
            
            // Emit state change event
            let _ = manager_clone.event_tx.send(rvoip_session_core::manager::events::SessionEvent::StateChanged {
                session_id: session_id_clone,
                old_state,
                new_state: CallState::Active,
            }).await;
        }
    });
    
    // Wait for answer
    let result = manager.wait_for_answer(&session_id, Duration::from_secs(2)).await;
    assert!(result.is_ok(), "Should succeed when state changes to Active");
    
    manager.stop().await.unwrap();
} 