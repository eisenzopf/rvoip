use std::fmt;
use serde::{Serialize, Deserialize};

/// SIP session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is being initialized
    Initializing,
    
    /// Outgoing call is being established
    Dialing,
    
    /// Incoming call is being received
    Ringing,
    
    /// Call is connected and active
    Connected,
    
    /// Call is on hold
    OnHold,
    
    /// Call is being transferred
    Transferring,
    
    /// Call is being terminated
    Terminating,
    
    /// Call has ended
    Terminated,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Initializing => write!(f, "Initializing"),
            SessionState::Dialing => write!(f, "Dialing"),
            SessionState::Ringing => write!(f, "Ringing"),
            SessionState::Connected => write!(f, "Connected"),
            SessionState::OnHold => write!(f, "OnHold"),
            SessionState::Transferring => write!(f, "Transferring"),
            SessionState::Terminating => write!(f, "Terminating"),
            SessionState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Session direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDirection {
    /// Outgoing call
    Outgoing,
    /// Incoming call
    Incoming,
}

/// Transaction types that can be used in a session
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTransactionType {
    /// Initial INVITE transaction
    InitialInvite,
    /// Re-INVITE transaction
    ReInvite,
    /// BYE transaction
    Bye,
    /// CANCEL transaction
    Cancel,
    /// OPTIONS transaction
    Options,
    /// INFO transaction
    Info,
    /// MESSAGE transaction
    Message,
    /// REFER transaction
    Refer,
    /// NOTIFY transaction
    Notify,
    /// UPDATE transaction
    Update,
    /// REGISTER transaction (rare for sessions)
    Register,
    /// Other transaction type
    Other(String),
}

impl fmt::Display for SessionTransactionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionTransactionType::InitialInvite => write!(f, "InitialInvite"),
            SessionTransactionType::ReInvite => write!(f, "ReInvite"),
            SessionTransactionType::Bye => write!(f, "Bye"),
            SessionTransactionType::Cancel => write!(f, "Cancel"),
            SessionTransactionType::Options => write!(f, "Options"),
            SessionTransactionType::Info => write!(f, "Info"),
            SessionTransactionType::Message => write!(f, "Message"),
            SessionTransactionType::Refer => write!(f, "Refer"),
            SessionTransactionType::Notify => write!(f, "Notify"),
            SessionTransactionType::Update => write!(f, "Update"),
            SessionTransactionType::Register => write!(f, "Register"),
            SessionTransactionType::Other(s) => write!(f, "Other({})", s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Initializing.to_string(), "Initializing");
        assert_eq!(SessionState::Dialing.to_string(), "Dialing");
        assert_eq!(SessionState::Ringing.to_string(), "Ringing");
        assert_eq!(SessionState::Connected.to_string(), "Connected");
        assert_eq!(SessionState::OnHold.to_string(), "OnHold");
        assert_eq!(SessionState::Transferring.to_string(), "Transferring");
        assert_eq!(SessionState::Terminating.to_string(), "Terminating");
        assert_eq!(SessionState::Terminated.to_string(), "Terminated");
    }
    
    #[test]
    fn test_session_transaction_type_display() {
        assert_eq!(SessionTransactionType::InitialInvite.to_string(), "InitialInvite");
        assert_eq!(SessionTransactionType::ReInvite.to_string(), "ReInvite");
        assert_eq!(SessionTransactionType::Bye.to_string(), "Bye");
        assert_eq!(SessionTransactionType::Cancel.to_string(), "Cancel");
        assert_eq!(SessionTransactionType::Other("Custom".to_string()).to_string(), "Other(Custom)");
    }
} 