//! Unit tests for dialog state management
//!
//! Tests the DialogState enum and its behavior in isolation.

use rvoip_dialog_core::DialogState;

/// Test dialog state enum values and display
#[test]
fn test_dialog_state_display() {
    assert_eq!(DialogState::Initial.to_string(), "Initial");
    assert_eq!(DialogState::Early.to_string(), "Early");
    assert_eq!(DialogState::Confirmed.to_string(), "Confirmed");
    assert_eq!(DialogState::Recovering.to_string(), "Recovering");
    assert_eq!(DialogState::Terminated.to_string(), "Terminated");
}

/// Test dialog state equality comparisons
#[test]
fn test_dialog_state_equality() {
    assert_eq!(DialogState::Initial, DialogState::Initial);
    assert_eq!(DialogState::Early, DialogState::Early);
    assert_eq!(DialogState::Confirmed, DialogState::Confirmed);
    assert_eq!(DialogState::Recovering, DialogState::Recovering);
    assert_eq!(DialogState::Terminated, DialogState::Terminated);
    
    // Test inequality
    assert_ne!(DialogState::Initial, DialogState::Early);
    assert_ne!(DialogState::Early, DialogState::Confirmed);
    assert_ne!(DialogState::Confirmed, DialogState::Terminated);
}

/// Test dialog state serialization/deserialization
#[test]
fn test_dialog_state_serialization() {
    // Test serialization
    let initial_state = DialogState::Initial;
    let serialized = serde_json::to_string(&initial_state).unwrap();
    
    // Test deserialization
    let deserialized: DialogState = serde_json::from_str(&serialized).unwrap();
    assert_eq!(initial_state, deserialized);
    
    // Test all states
    let states = vec![
        DialogState::Initial,
        DialogState::Early,
        DialogState::Confirmed,
        DialogState::Recovering,
        DialogState::Terminated,
    ];
    
    for state in states {
        let serialized = serde_json::to_string(&state).unwrap();
        let deserialized: DialogState = serde_json::from_str(&serialized).unwrap();
        assert_eq!(state, deserialized);
    }
}

/// Test dialog state cloning
#[test]
fn test_dialog_state_cloning() {
    let original = DialogState::Confirmed;
    let cloned = original.clone();
    
    assert_eq!(original, cloned);
    
    // Verify they are independent (though for enums this is automatic)
    let _modified = DialogState::Terminated;
    assert_eq!(original, DialogState::Confirmed);
    assert_eq!(cloned, DialogState::Confirmed);
}

/// Test dialog state debug formatting
#[test]
fn test_dialog_state_debug() {
    let state = DialogState::Early;
    let debug_str = format!("{:?}", state);
    assert_eq!(debug_str, "Early");
    
    // Test all states have proper debug output
    let states = vec![
        (DialogState::Initial, "Initial"),
        (DialogState::Early, "Early"),
        (DialogState::Confirmed, "Confirmed"),
        (DialogState::Recovering, "Recovering"),
        (DialogState::Terminated, "Terminated"),
    ];
    
    for (state, expected_debug) in states {
        assert_eq!(format!("{:?}", state), expected_debug);
    }
}

/// Test dialog state in collections
#[test]
fn test_dialog_state_in_collections() {
    use std::collections::HashMap;
    
    let mut state_counts = HashMap::new();
    
    // Add some states
    state_counts.insert(DialogState::Initial, 5);
    state_counts.insert(DialogState::Confirmed, 10);
    state_counts.insert(DialogState::Terminated, 2);
    
    // Verify we can retrieve them
    assert_eq!(state_counts.get(&DialogState::Initial), Some(&5));
    assert_eq!(state_counts.get(&DialogState::Confirmed), Some(&10));
    assert_eq!(state_counts.get(&DialogState::Terminated), Some(&2));
    assert_eq!(state_counts.get(&DialogState::Early), None);
}

/// Test dialog state pattern matching
#[test]
fn test_dialog_state_pattern_matching() {
    fn classify_state(state: &DialogState) -> &'static str {
        match state {
            DialogState::Initial => "starting",
            DialogState::Early => "provisional",
            DialogState::Confirmed => "active",
            DialogState::Recovering => "unstable",
            DialogState::Terminated => "finished",
        }
    }
    
    assert_eq!(classify_state(&DialogState::Initial), "starting");
    assert_eq!(classify_state(&DialogState::Early), "provisional");
    assert_eq!(classify_state(&DialogState::Confirmed), "active");
    assert_eq!(classify_state(&DialogState::Recovering), "unstable");
    assert_eq!(classify_state(&DialogState::Terminated), "finished");
}

/// Test dialog state transition validity (conceptual)
#[test]
fn test_dialog_state_transition_validity() {
    // This tests logical state transitions (not enforced by the enum itself,
    // but represents valid RFC 3261 transitions)
    
    fn is_valid_transition(from: &DialogState, to: &DialogState) -> bool {
        match (from, to) {
            // From Initial
            (DialogState::Initial, DialogState::Early) => true,
            (DialogState::Initial, DialogState::Confirmed) => true,
            (DialogState::Initial, DialogState::Terminated) => true,
            
            // From Early
            (DialogState::Early, DialogState::Confirmed) => true,
            (DialogState::Early, DialogState::Terminated) => true,
            
            // From Confirmed
            (DialogState::Confirmed, DialogState::Recovering) => true,
            (DialogState::Confirmed, DialogState::Terminated) => true,
            
            // From Recovering
            (DialogState::Recovering, DialogState::Confirmed) => true,
            (DialogState::Recovering, DialogState::Terminated) => true,
            
            // From Terminated (no valid transitions)
            (DialogState::Terminated, _) => false,
            
            // Same state (always valid)
            (a, b) if a == b => true,
            
            // All other transitions are invalid
            _ => false,
        }
    }
    
    // Test valid transitions
    assert!(is_valid_transition(&DialogState::Initial, &DialogState::Early));
    assert!(is_valid_transition(&DialogState::Initial, &DialogState::Confirmed));
    assert!(is_valid_transition(&DialogState::Early, &DialogState::Confirmed));
    assert!(is_valid_transition(&DialogState::Confirmed, &DialogState::Recovering));
    assert!(is_valid_transition(&DialogState::Recovering, &DialogState::Confirmed));
    assert!(is_valid_transition(&DialogState::Confirmed, &DialogState::Terminated));
    
    // Test invalid transitions
    assert!(!is_valid_transition(&DialogState::Confirmed, &DialogState::Initial));
    assert!(!is_valid_transition(&DialogState::Terminated, &DialogState::Confirmed));
    assert!(!is_valid_transition(&DialogState::Early, &DialogState::Recovering));
    
    // Test same state transitions (should be valid)
    assert!(is_valid_transition(&DialogState::Confirmed, &DialogState::Confirmed));
} 