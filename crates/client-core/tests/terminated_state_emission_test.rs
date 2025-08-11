//! Test that verifies client-core properly receives Terminated state from on_call_ended
//! and emits the correct CallStateChanged event

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use dashmap::DashMap;
use chrono::Utc;
use std::collections::HashMap;

use rvoip_client_core::{
    client::events::ClientCallHandler,
    call::{CallId, CallState, CallInfo, CallDirection},
    events::{ClientEvent, ClientEventHandler, CallStatusInfo},
    ClientError,
};
use rvoip_session_core::{
    CallHandler,
    api::types::{SessionId, CallSession},
};

#[tokio::test]
async fn test_on_call_ended_emits_terminated_state() {
    // Create a channel to capture emitted events
    let (event_tx, mut event_rx) = broadcast::channel(10);
    
    // Create the handler with event channel
    let handler = ClientCallHandler::new(
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
    ).with_event_tx(event_tx);
    
    // Create a test client event handler to capture the CallStateChanged event
    struct TestEventHandler {
        received_state: Arc<RwLock<Option<CallState>>>,
    }
    
    #[async_trait::async_trait]
    impl ClientEventHandler for TestEventHandler {
        async fn on_incoming_call(&self, _info: rvoip_client_core::events::IncomingCallInfo) -> rvoip_client_core::events::CallAction {
            rvoip_client_core::events::CallAction::Ignore
        }
        
        async fn on_call_state_changed(&self, info: CallStatusInfo) {
            // Capture the new state
            let mut state = self.received_state.write().await;
            *state = Some(info.new_state);
        }
        
        async fn on_registration_status_changed(&self, _info: rvoip_client_core::events::RegistrationStatusInfo) {}
        async fn on_media_event(&self, _info: rvoip_client_core::events::MediaEventInfo) {}
        async fn on_client_error(&self, _error: ClientError, _call_id: Option<CallId>) {}
        async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    }
    
    let received_state = Arc::new(RwLock::new(None));
    let test_handler = Arc::new(TestEventHandler {
        received_state: received_state.clone(),
    });
    
    // Set the client event handler
    handler.set_event_handler(test_handler).await;
    
    // Setup: Create a call mapping
    let session_id = SessionId("test-session".to_string());
    let call_id = CallId::new_v4();
    
    handler.call_mapping.insert(session_id.clone(), call_id);
    handler.session_mapping.insert(call_id, session_id.clone());
    
    // Add call info
    handler.call_info.insert(call_id, CallInfo {
        call_id,
        direction: CallDirection::Outgoing,
        state: CallState::Connected, // Start in Connected state
        local_uri: "sip:alice@example.com".to_string(),
        remote_uri: "sip:bob@example.com".to_string(),
        remote_display_name: None,
        subject: None,
        created_at: Utc::now(),
        connected_at: Some(Utc::now()),
        ended_at: None,
        remote_addr: None,
        media_session_id: None,
        sip_call_id: "test-call".to_string(),
        metadata: HashMap::new(),
    });
    
    // Create a session with Terminated state (this is what should be passed after our fix)
    let terminated_session = CallSession {
        id: session_id.clone(),
        from: "sip:bob@example.com".to_string(),
        to: "sip:alice@example.com".to_string(),
        state: rvoip_session_core::api::types::CallState::Terminated, // This is the key - should be Terminated
        started_at: Some(std::time::Instant::now()),
    };
    
    // Call on_call_ended with the Terminated session
    handler.on_call_ended(terminated_session, "Remote hangup").await;
    
    // Give a moment for async processing
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Verify the CallStateChanged event was emitted with Terminated state
    let captured_state = received_state.read().await;
    assert_eq!(
        *captured_state,
        Some(CallState::Terminated),
        "Client event handler should receive Terminated state, not {:?}",
        *captured_state
    );
    
    // Also verify via the event channel
    if let Ok(event) = event_rx.recv().await {
        match event {
            ClientEvent::CallStateChanged { info, .. } => {
                assert_eq!(
                    info.new_state,
                    CallState::Terminated,
                    "Event should contain Terminated state, not {:?}",
                    info.new_state
                );
            }
            _ => panic!("Expected CallStateChanged event"),
        }
    } else {
        panic!("No event was emitted");
    }
    
    // Verify mappings are cleaned up (Phase 2 behavior)
    assert!(!handler.call_mapping.contains_key(&session_id), "Session mapping should be removed");
    assert!(!handler.session_mapping.contains_key(&call_id), "Call mapping should be removed");
    
    // Verify call info is updated with ended_at
    let call_info = handler.call_info.get(&call_id).unwrap();
    assert!(call_info.ended_at.is_some(), "Call should have ended_at timestamp");
    assert_eq!(
        call_info.metadata.get("termination_reason"),
        Some(&"Remote hangup".to_string()),
        "Termination reason should be stored"
    );
}

#[tokio::test]
async fn test_on_call_ended_with_terminating_state_bug() {
    // This test simulates the BUG scenario where on_call_ended receives
    // a session with Terminating state instead of Terminated
    
    let (event_tx, mut event_rx) = broadcast::channel(10);
    
    let handler = ClientCallHandler::new(
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
    ).with_event_tx(event_tx);
    
    // Setup call
    let session_id = SessionId("bug-session".to_string());
    let call_id = CallId::new_v4();
    
    handler.call_mapping.insert(session_id.clone(), call_id);
    handler.session_mapping.insert(call_id, session_id.clone());
    
    handler.call_info.insert(call_id, CallInfo {
        call_id,
        direction: CallDirection::Incoming,
        state: CallState::Connected,
        local_uri: "sip:alice@example.com".to_string(),
        remote_uri: "sip:bob@example.com".to_string(),
        remote_display_name: None,
        subject: None,
        created_at: Utc::now(),
        connected_at: Some(Utc::now()),
        ended_at: None,
        remote_addr: None,
        media_session_id: None,
        sip_call_id: "bug-call".to_string(),
        metadata: HashMap::new(),
    });
    
    // Create a session with TERMINATING state (this simulates the bug)
    let terminating_session = CallSession {
        id: session_id.clone(),
        from: "sip:bob@example.com".to_string(),
        to: "sip:alice@example.com".to_string(),
        state: rvoip_session_core::api::types::CallState::Terminating, // BUG: Wrong state!
        started_at: Some(std::time::Instant::now()),
    };
    
    // Call on_call_ended with the wrong (Terminating) state
    handler.on_call_ended(terminating_session, "Remote hangup (bug test)").await;
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // With the bug, this would emit Terminating state instead of Terminated
    if let Ok(event) = event_rx.recv().await {
        match event {
            ClientEvent::CallStateChanged { info, .. } => {
                // This test documents the BUG behavior - it would emit Terminating
                // After our fix, on_call_ended should always map to Terminated
                println!("Bug test: Received state {:?} (should be Terminated after fix)", info.new_state);
                
                // With our fix in session-core, this should never happen because
                // session-core now ensures the session has Terminated state before
                // calling on_call_ended
            }
            _ => {}
        }
    }
}