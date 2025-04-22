use super::types::CallState;

/// Checks if a state transition is valid
pub fn is_valid_state_transition(from: CallState, to: CallState) -> bool {
    match (from, to) {
        // Initial state can transition to Ringing, Terminating or Failed
        (CallState::Initial, CallState::Ringing) => true,
        (CallState::Initial, CallState::Terminating) => true,
        (CallState::Initial, CallState::Failed) => true,
        
        // Ringing can transition to Connecting, Established, Terminating or Failed
        (CallState::Ringing, CallState::Connecting) => true,
        (CallState::Ringing, CallState::Established) => true,
        (CallState::Ringing, CallState::Terminating) => true,
        (CallState::Ringing, CallState::Failed) => true,
        
        // Connecting can transition to Established, Terminating or Failed
        (CallState::Connecting, CallState::Established) => true,
        (CallState::Connecting, CallState::Terminating) => true,
        (CallState::Connecting, CallState::Failed) => true,
        
        // Established can transition to Terminating or Failed
        (CallState::Established, CallState::Terminating) => true,
        (CallState::Established, CallState::Failed) => true,
        
        // Terminating can transition to Terminated or Failed
        (CallState::Terminating, CallState::Terminated) => true,
        (CallState::Terminating, CallState::Failed) => true,
        
        // Terminated and Failed are terminal states
        (CallState::Terminated, _) => false,
        (CallState::Failed, _) => false,
        
        // Any other transition is invalid
        _ => false,
    }
} 