use std::fmt;
use serde::{Serialize, Deserialize};

/// SIP dialog state as defined in RFC 3261
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogState {
    /// Early dialog - created from provisional response
    Early,
    
    /// Confirmed dialog - created from final response
    Confirmed,
    
    /// Terminated dialog - ended by BYE or other means
    Terminated,
}

impl fmt::Display for DialogState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogState::Early => write!(f, "Early"),
            DialogState::Confirmed => write!(f, "Confirmed"),
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