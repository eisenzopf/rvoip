use std::fmt;
use serde::{Serialize, Deserialize};

/// Represents the state of a SIP dialog as per RFC 3261
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DialogState {
    /// Early dialog state - initiated but not confirmed (after 1xx response)
    Early,
    
    /// Confirmed dialog state - after 2xx response
    Confirmed,
    
    /// Dialog is being terminated (after sending BYE)
    Terminating,
    
    /// Terminated dialog state - after BYE or error
    Terminated,
}

impl fmt::Display for DialogState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogState::Early => write!(f, "Early"),
            DialogState::Confirmed => write!(f, "Confirmed"),
            DialogState::Terminating => write!(f, "Terminating"),
            DialogState::Terminated => write!(f, "Terminated"),
        }
    }
}

impl Default for DialogState {
    fn default() -> Self {
        DialogState::Early
    }
} 