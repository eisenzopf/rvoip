use std::fmt;
use serde::{Serialize, Deserialize};

/// Represents the state of a dialog
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DialogState {
    /// Initial state (before any response has been sent/received)
    Initial,
    
    /// Early dialog (1xx responses with tag)
    Early,
    
    /// Confirmed dialog (2xx response)
    Confirmed,
    
    /// Dialog is in the process of recovering from a network failure
    Recovering,
    
    /// Dialog has been terminated
    Terminated,
}

impl fmt::Display for DialogState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogState::Initial => write!(f, "Initial"),
            DialogState::Early => write!(f, "Early"),
            DialogState::Confirmed => write!(f, "Confirmed"),
            DialogState::Recovering => write!(f, "Recovering"),
            DialogState::Terminated => write!(f, "Terminated"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dialog_state_display() {
        assert_eq!(DialogState::Early.to_string(), "Early");
        assert_eq!(DialogState::Confirmed.to_string(), "Confirmed");
        assert_eq!(DialogState::Terminated.to_string(), "Terminated");
    }
} 