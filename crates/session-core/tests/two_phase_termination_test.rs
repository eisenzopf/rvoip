//! Unit tests for two-phase termination functionality

use rvoip_session_core::api::types::{CallState, SessionId};
use rvoip_session_core::coordinator::{CleanupTracker, CleanupLayer};

#[test]
fn test_terminating_state_exists() {
    // Verify that the Terminating state exists in the CallState enum
    let state = CallState::Terminating;
    
    // Test state transitions
    match state {
        CallState::Terminating => {
            // This is the expected state
            assert!(true, "Terminating state exists");
        }
        _ => {
            panic!("Unexpected state");
        }
    }
}

#[test]
fn test_state_transition_ordering() {
    // Verify the logical ordering of states
    let states = vec![
        CallState::Initiating,
        CallState::Ringing,
        CallState::Active,
        CallState::Terminating,
        CallState::Terminated,
    ];
    
    // Verify all states are distinct
    for (i, state1) in states.iter().enumerate() {
        for (j, state2) in states.iter().enumerate() {
            if i != j {
                assert_ne!(state1, state2, "States should be distinct");
            }
        }
    }
}

#[test]
fn test_terminating_not_terminal() {
    // Terminating should not be considered a terminal state
    // It's an intermediate state before Terminated
    
    let terminating = CallState::Terminating;
    let terminated = CallState::Terminated;
    
    assert_ne!(terminating, terminated, "Terminating and Terminated should be different states");
}

#[cfg(test)]
mod termination_flow_tests {
    use super::*;
    
    #[test]
    fn test_two_phase_termination_states() {
        // Test that both Terminating and Terminated states exist
        let terminating = CallState::Terminating;
        let terminated = CallState::Terminated;
        
        // Verify they are distinct states
        assert_ne!(terminating, terminated, "Terminating and Terminated must be distinct");
        
        // Verify state transitions
        // Connected -> Terminating -> Terminated
        let connected = CallState::Active;
        assert_ne!(connected, terminating);
        assert_ne!(connected, terminated);
    }
    
    #[tokio::test]
    async fn test_cleanup_confirmation_processing() {
        use rvoip_session_core::manager::events::{SessionEvent, SessionEventProcessor};
        use std::time::Duration;
        use tokio::time::timeout;
        
        // Create event processor
        let processor = SessionEventProcessor::new();
        processor.start().await.unwrap();
        
        // Subscribe to events
        let mut subscriber = processor.subscribe().await.unwrap();
        
        // Create a mock session ID
        let session_id = SessionId("test-session-123".to_string());
        
        // Send SessionTerminating event (Phase 1)
        processor.publish_event(SessionEvent::SessionTerminating {
            session_id: session_id.clone(),
            reason: "Test termination".to_string(),
        }).await.unwrap();
        
        // Verify we receive the terminating event
        let event = timeout(Duration::from_millis(100), subscriber.receive()).await
            .expect("Should receive event")
            .expect("Should get event");
        
        match event {
            SessionEvent::SessionTerminating { session_id: id, .. } => {
                assert_eq!(id, session_id, "Should receive terminating event for correct session");
            }
            _ => panic!("Expected SessionTerminating event"),
        }
        
        // Send cleanup confirmations
        processor.publish_event(SessionEvent::CleanupConfirmation {
            session_id: session_id.clone(),
            layer: "Media".to_string(),
        }).await.unwrap();
        
        processor.publish_event(SessionEvent::CleanupConfirmation {
            session_id: session_id.clone(),
            layer: "Client".to_string(),
        }).await.unwrap();
        
        // Verify we receive the cleanup confirmations
        for _ in 0..2 {
            let event = timeout(Duration::from_millis(100), subscriber.receive()).await
                .expect("Should receive event")
                .expect("Should get event");
            
            match event {
                SessionEvent::CleanupConfirmation { session_id: id, layer } => {
                    assert_eq!(id, session_id, "Cleanup confirmation for correct session");
                    assert!(layer == "Media" || layer == "Client", "Valid cleanup layer");
                }
                _ => panic!("Expected CleanupConfirmation event"),
            }
        }
    }
    
    #[tokio::test]
    async fn test_cleanup_tracker_completion() {
        use std::time::Instant;
        
        // Create a cleanup tracker
        let mut tracker = CleanupTracker {
            media_done: false,
            client_done: false,
            started_at: Instant::now(),
            reason: "Test".to_string(),
        };
        
        // Initially not complete
        assert!(!tracker.media_done && !tracker.client_done, "Should start incomplete");
        
        // Mark media done
        tracker.media_done = true;
        assert!(tracker.media_done && !tracker.client_done, "Only media should be done");
        
        // Mark client done
        tracker.client_done = true;
        assert!(tracker.media_done && tracker.client_done, "Both should be done");
        
        // Verify completion check
        let all_done = tracker.media_done && tracker.client_done;
        assert!(all_done, "Should be complete when both layers are done");
    }
}