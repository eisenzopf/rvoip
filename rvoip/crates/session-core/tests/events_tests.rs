//! Event tests for session-core public API
//!
//! Simplified tests that focus on the core event functionality
//! without complex EventBus async patterns.

use rvoip_session_core::{
    events::SessionEvent,
    session::{SessionId, SessionState},
    dialog::DialogId,
};

#[test]
fn test_session_event_creation() {
    let session_id = SessionId::new();
    let dialog_id = DialogId::new();
    
    // Test creating different event types
    let created_event = SessionEvent::Created { 
        session_id: session_id.clone() 
    };
    
    let state_changed_event = SessionEvent::StateChanged { 
        session_id: session_id.clone(),
        old_state: SessionState::Initializing,
        new_state: SessionState::Dialing,
    };
    
    let dialog_updated_event = SessionEvent::DialogUpdated { 
        session_id: session_id.clone(),
        dialog_id,
    };
    
    let terminated_event = SessionEvent::Terminated { 
        session_id: session_id.clone(),
        reason: "Test termination".to_string(),
    };
    
    // Test passes if events can be created without errors
    assert!(matches!(created_event, SessionEvent::Created { .. }));
    assert!(matches!(state_changed_event, SessionEvent::StateChanged { .. }));
    assert!(matches!(dialog_updated_event, SessionEvent::DialogUpdated { .. }));
    assert!(matches!(terminated_event, SessionEvent::Terminated { .. }));
}

#[test]
fn test_session_state_variants() {
    // Test all session state variants can be created
    let states = vec![
        SessionState::Initializing,
        SessionState::Dialing,
        SessionState::Ringing,
        SessionState::Connected,
        SessionState::OnHold,
        SessionState::Terminating,
        SessionState::Terminated,
    ];
    
    // Verify each state has a string representation
    for state in states {
        let state_str = state.to_string();
        assert!(!state_str.is_empty());
    }
}

#[test]
fn test_session_id_generation() {
    let id1 = SessionId::new();
    let id2 = SessionId::new();
    
    // Each ID should be unique
    assert_ne!(id1, id2);
    
    // ID should have a string representation
    let id_str = id1.to_string();
    assert!(!id_str.is_empty());
}

#[test]
fn test_dialog_id_generation() {
    let id1 = DialogId::new();
    let id2 = DialogId::new();
    
    // Each ID should be unique
    assert_ne!(id1, id2);
    
    // ID should have a string representation
    let id_str = id1.to_string();
    assert!(!id_str.is_empty());
} 