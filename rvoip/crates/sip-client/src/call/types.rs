use serde::{Serialize, Deserialize};

/// Direction of the call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call (we initiated it)
    Outgoing,
    /// Incoming call (we received it)
    Incoming,
}

/// State of a call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallState {
    /// Initial state
    Initial,
    /// Outgoing call: INVITE sent, waiting for response
    /// Incoming call: Ringing, 180 Ringing sent
    Ringing,
    /// Outgoing call: Received final response, waiting for ACK
    /// Incoming call: 200 OK sent, waiting for ACK
    Connecting,
    /// Call is established (media flowing)
    Established,
    /// Call is being terminated
    Terminating,
    /// Call is terminated
    Terminated,
    /// Call has failed (special terminated state with error)
    Failed,
}

/// Error type for invalid state transitions
#[derive(Debug, Clone)]
pub struct StateChangeError {
    pub current: CallState,
    pub requested: CallState,
    pub message: String,
}

impl std::fmt::Display for StateChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid state transition from {} to {}: {}", 
            self.current, self.requested, self.message)
    }
}

impl std::fmt::Display for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallState::Initial => write!(f, "Initial"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Connecting => write!(f, "Connecting"),
            CallState::Established => write!(f, "Established"),
            CallState::Terminating => write!(f, "Terminating"),
            CallState::Terminated => write!(f, "Terminated"),
            CallState::Failed => write!(f, "Failed"),
        }
    }
} 