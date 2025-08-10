//! Tests for Terminating state in client-core

use rvoip_client_core::call::CallState;

#[test]
fn test_terminating_state_exists() {
    // Verify Terminating state exists in CallState enum
    let state = CallState::Terminating;
    
    match state {
        CallState::Terminating => {
            assert!(true, "Terminating state exists");
        }
        _ => panic!("Expected Terminating state"),
    }
}

#[test]
fn test_terminating_is_not_terminated() {
    let terminating = CallState::Terminating;
    let terminated = CallState::Terminated;
    
    // These should be different states
    assert_ne!(terminating, terminated, "Terminating and Terminated should be distinct states");
}

#[test]
fn test_state_progression() {
    // Test the expected progression of states
    let states = vec![
        CallState::Initiating,
        CallState::Proceeding,
        CallState::Ringing,
        CallState::Connected,
        CallState::Terminating,  // Phase 1
        CallState::Terminated,    // Phase 2
    ];
    
    // Verify all states are unique
    for i in 0..states.len() {
        for j in 0..states.len() {
            if i != j {
                assert_ne!(states[i], states[j], 
                    "State {:?} should not equal {:?}", states[i], states[j]);
            }
        }
    }
}

#[test]
fn test_terminating_state_properties() {
    // Test that Terminating has the expected properties
    let state = CallState::Terminating;
    
    // Should not be an initial state
    assert_ne!(state, CallState::Initiating);
    
    // Should not be a connected state
    assert_ne!(state, CallState::Connected);
    
    // Should be distinct from failed state
    assert_ne!(state, CallState::Failed);
    
    // Should be distinct from cancelled state
    assert_ne!(state, CallState::Cancelled);
}

#[cfg(test)]
mod state_mapping_tests {
    use super::*;
    
    #[test]
    fn test_session_to_client_state_mapping() {
        // This tests that we can map from session-core states to client-core states
        // In the actual implementation, this is done in ClientCallHandler::map_session_state_to_client_state
        
        // Simulate the mapping logic
        fn map_state(session_state: &str) -> CallState {
            match session_state {
                "Initiating" => CallState::Initiating,
                "Ringing" => CallState::Ringing,
                "Active" => CallState::Connected,
                "OnHold" => CallState::Connected, // Still connected, just on hold
                "Terminating" => CallState::Terminating,
                "Terminated" => CallState::Terminated,
                "Failed" => CallState::Failed,
                _ => CallState::Failed,
            }
        }
        
        // Test the mapping
        assert_eq!(map_state("Terminating"), CallState::Terminating);
        assert_eq!(map_state("Terminated"), CallState::Terminated);
        assert_ne!(map_state("Terminating"), map_state("Terminated"));
    }
}

#[cfg(test)]
mod event_handling_tests {
    use super::*;
    use rvoip_client_core::client::events::ClientCallHandler;
    use rvoip_client_core::call::{CallId, CallInfo, CallDirection};
    use std::sync::Arc;
    use dashmap::DashMap;
    use std::collections::HashMap;
    use chrono::Utc;
    use rvoip_session_core::CallHandler;
    
    #[tokio::test]
    async fn test_on_call_ended_phase_2() {
        // Test that on_call_ended properly handles Phase 2 (cleanup complete)
        let handler = ClientCallHandler::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        );
        
        // Add a call to the mappings
        let session_id = rvoip_session_core::api::types::SessionId("test-session".to_string());
        let call_id = CallId::new_v4();
        handler.call_mapping.insert(session_id.clone(), call_id);
        handler.session_mapping.insert(call_id, session_id.clone());
        
        // Add call info
        handler.call_info.insert(call_id, CallInfo {
            call_id: call_id,
            direction: CallDirection::Outgoing,
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
            sip_call_id: "".to_string(),
            metadata: HashMap::new(),
        });
        
        // Call on_call_ended (Phase 2)
        let session = rvoip_session_core::api::types::CallSession {
            id: session_id.clone(),
            from: "sip:alice@example.com".to_string(),
            to: "sip:bob@example.com".to_string(),
            state: rvoip_session_core::api::types::CallState::Terminated,
            started_at: Some(std::time::Instant::now()),
        };
        
        handler.on_call_ended(session, "Remote hangup").await;
        
        // Verify mappings are cleaned up (Phase 2 behavior)
        assert!(!handler.call_mapping.contains_key(&session_id), "Session mapping should be removed");
        assert!(!handler.session_mapping.contains_key(&call_id), "Call mapping should be removed");
        
        // Verify call info is updated but still exists (for history)
        let call_info = handler.call_info.get(&call_id).unwrap();
        assert_eq!(call_info.state, CallState::Terminated, "State should be Terminated");
        assert!(call_info.ended_at.is_some(), "Ended time should be set");
        assert_eq!(call_info.metadata.get("termination_reason"), Some(&"Remote hangup".to_string()));
    }
    
    #[tokio::test]
    async fn test_cleanup_confirmation_sent() {
        use tokio::sync::mpsc;
        use rvoip_session_core::manager::events::SessionEvent;
        use tokio::sync::RwLock;
        
        // Create a channel to capture session events
        let (tx, mut rx) = mpsc::channel(10);
        
        // Create handler with session event channel
        let handler = ClientCallHandler::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        );
        // Directly set the session event channel (since we're testing internal behavior)
        *handler.session_event_tx.write().await = Some(tx);
        
        // Add a call to the mappings
        let session_id = rvoip_session_core::api::types::SessionId("test-session".to_string());
        let call_id = CallId::new_v4();
        handler.call_mapping.insert(session_id.clone(), call_id);
        handler.session_mapping.insert(call_id, session_id.clone());
        
        // Add call info
        handler.call_info.insert(call_id, CallInfo {
            call_id: call_id,
            direction: CallDirection::Outgoing,
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
            sip_call_id: "".to_string(),
            metadata: HashMap::new(),
        });
        
        // Call on_call_ended
        let session = rvoip_session_core::api::types::CallSession {
            id: session_id.clone(),
            from: "sip:alice@example.com".to_string(),
            to: "sip:bob@example.com".to_string(),
            state: rvoip_session_core::api::types::CallState::Terminated,
            started_at: Some(std::time::Instant::now()),
        };
        
        handler.on_call_ended(session, "Test cleanup").await;
        
        // Verify cleanup confirmation was sent
        use tokio::time::{timeout, Duration};
        let event = timeout(Duration::from_millis(100), rx.recv()).await
            .expect("Should receive event")
            .expect("Should get event");
        
        match event {
            SessionEvent::CleanupConfirmation { session_id: id, layer } => {
                assert_eq!(id, session_id, "Cleanup confirmation for correct session");
                assert_eq!(layer, "Client", "Should be from Client layer");
            }
            _ => panic!("Expected CleanupConfirmation event"),
        }
    }
}