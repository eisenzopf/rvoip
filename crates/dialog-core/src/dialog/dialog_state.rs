//! Dialog state management
//!
//! Represents the various states a SIP dialog can be in during its lifecycle.

use std::fmt;
use serde::{Serialize, Deserialize};

/// Represents the state of a dialog
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DialogState {
    /// Dialog is in initial state, before establishment
    Initial,
    
    /// Dialog is in early state (provisional response received)
    Early,
    
    /// Dialog is confirmed and established
    Confirmed,
    
    /// Dialog is in recovery mode due to some failure
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

impl DialogState {
    /// Check if the dialog is active (can process requests)
    pub fn is_active(&self) -> bool {
        matches!(self, DialogState::Confirmed | DialogState::Early)
    }
    
    /// Check if the dialog is terminated
    pub fn is_terminated(&self) -> bool {
        matches!(self, DialogState::Terminated)
    }
    
    /// Check if the dialog is in recovery mode
    pub fn is_recovering(&self) -> bool {
        matches!(self, DialogState::Recovering)
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