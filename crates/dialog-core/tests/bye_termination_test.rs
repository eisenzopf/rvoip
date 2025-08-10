//! Tests for two-phase termination in BYE handler

use rvoip_dialog_core::events::SessionCoordinationEvent;

#[test]
fn test_call_terminating_event_exists() {
    // Verify CallTerminating event variant exists
    let event = SessionCoordinationEvent::CallTerminating {
        dialog_id: rvoip_dialog_core::dialog::DialogId::new(),
        reason: "BYE received".to_string(),
    };
    
    match event {
        SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
            assert_eq!(reason, "BYE received");
            assert!(!dialog_id.to_string().is_empty());
        }
        _ => panic!("Expected CallTerminating event"),
    }
}

#[test]
fn test_cleanup_confirmation_event_exists() {
    // Verify CleanupConfirmation event variant exists
    let event = SessionCoordinationEvent::CleanupConfirmation {
        dialog_id: rvoip_dialog_core::dialog::DialogId::new(),
        layer: "media".to_string(),
    };
    
    match event {
        SessionCoordinationEvent::CleanupConfirmation { dialog_id, layer } => {
            assert_eq!(layer, "media");
            assert!(!dialog_id.to_string().is_empty());
        }
        _ => panic!("Expected CleanupConfirmation event"),
    }
}

#[test]
fn test_terminating_and_terminated_are_distinct() {
    let dialog_id = rvoip_dialog_core::dialog::DialogId::new();
    
    let terminating = SessionCoordinationEvent::CallTerminating {
        dialog_id: dialog_id.clone(),
        reason: "Phase 1".to_string(),
    };
    
    let terminated = SessionCoordinationEvent::CallTerminated {
        dialog_id: dialog_id.clone(),
        reason: "Phase 2".to_string(),
    };
    
    // Verify they are different variants
    match (&terminating, &terminated) {
        (SessionCoordinationEvent::CallTerminating { .. }, 
         SessionCoordinationEvent::CallTerminated { .. }) => {
            // This is expected - they are different variants
            assert!(true);
        }
        _ => panic!("Events should be different variants"),
    }
}