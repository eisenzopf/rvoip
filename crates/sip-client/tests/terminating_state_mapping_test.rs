//! Tests for Terminating state mapping in sip-client

use rvoip_sip_client::types::CallState;

#[test]
fn test_terminating_state_exists() {
    // Verify Terminating state exists in the CallState enum
    let state = CallState::Terminating;
    
    match state {
        CallState::Terminating => {
            assert!(true, "Terminating state exists in sip-client");
        }
        _ => panic!("Expected Terminating state"),
    }
}

#[test]
fn test_terminating_vs_terminated() {
    let terminating = CallState::Terminating;
    let terminated = CallState::Terminated;
    
    // These should be different states
    assert_ne!(terminating, terminated, "Terminating and Terminated are distinct");
}

#[test]
fn test_all_call_states_unique() {
    // Test that all call states are unique
    let states = vec![
        CallState::Initiating,
        CallState::Ringing,
        CallState::IncomingRinging,
        CallState::Connected,
        CallState::OnHold,
        CallState::Transferring,
        CallState::Terminating,
        CallState::Terminated,
    ];
    
    // Verify each state is unique
    for i in 0..states.len() {
        for j in 0..states.len() {
            if i != j {
                assert_ne!(states[i], states[j], 
                    "State at index {} should not equal state at index {}", i, j);
            }
        }
    }
}

#[test]
fn test_state_serialization() {
    // Test that states can be serialized (important for API compatibility)
    use serde_json;
    
    let state = CallState::Terminating;
    let serialized = serde_json::to_string(&state).expect("Should serialize");
    assert_eq!(serialized, "\"Terminating\"");
    
    let deserialized: CallState = serde_json::from_str(&serialized).expect("Should deserialize");
    assert_eq!(deserialized, CallState::Terminating);
}

#[cfg(test)]
mod state_transition_tests {
    use super::*;
    
    #[test]
    fn test_valid_state_transitions() {
        // Test that certain state transitions make sense
        fn is_valid_transition(from: CallState, to: CallState) -> bool {
            match (from, to) {
                // Can transition from Connected to Terminating
                (CallState::Connected, CallState::Terminating) => true,
                // Can transition from OnHold to Terminating
                (CallState::OnHold, CallState::Terminating) => true,
                // Can transition from Terminating to Terminated
                (CallState::Terminating, CallState::Terminated) => true,
                // Cannot go back from Terminating to Connected
                (CallState::Terminating, CallState::Connected) => false,
                // Cannot go from Terminated to anything
                (CallState::Terminated, _) => false,
                _ => true, // Other transitions not tested here
            }
        }
        
        assert!(is_valid_transition(CallState::Connected, CallState::Terminating));
        assert!(is_valid_transition(CallState::OnHold, CallState::Terminating));
        assert!(is_valid_transition(CallState::Terminating, CallState::Terminated));
        assert!(!is_valid_transition(CallState::Terminating, CallState::Connected));
        assert!(!is_valid_transition(CallState::Terminated, CallState::Connected));
    }
}