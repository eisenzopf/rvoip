// Priority types for SIP Priority header
// Values according to RFC 3261 Section 20.26

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::error::Error;

/// Priority values for SIP Priority header
/// As defined in RFC 3261 Section 20.26
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Emergency call
    Emergency,
    
    /// Urgent call/message
    Urgent,
    
    /// Normal call/message (default)
    Normal,
    
    /// Non-urgent call/message
    NonUrgent,
    
    /// Numeric priority value (extension)
    Other(u8),
    
    /// Other token priority value (extension)
    Token(String),
}

impl Priority {
    /// Get the numeric priority value
    /// Lower values are higher priority
    pub fn value(&self) -> u8 {
        match self {
            Self::Emergency => 0,
            Self::Urgent => 1,
            Self::Normal => 2,
            Self::NonUrgent => 3,
            Self::Other(val) => *val,
            Self::Token(_) => 99, // Custom tokens have lowest priority by default
        }
    }
    
    /// Check if this priority is higher than another
    pub fn is_higher_than(&self, other: &Priority) -> bool {
        self.value() < other.value()
    }
    
    /// Get the default priority (Normal)
    pub fn default() -> Self {
        Self::Normal
    }
    
    /// Create a new priority from a token
    pub fn from_token(token: &str) -> Self {
        match token.to_lowercase().as_str() {
            "emergency" => Self::Emergency,
            "urgent" => Self::Urgent,
            "normal" => Self::Normal,
            "non-urgent" => Self::NonUrgent,
            other => {
                // Try to parse as a number first
                if let Ok(val) = other.parse::<u8>() {
                    Self::Other(val)
                } else {
                    // Otherwise treat as token
                    Self::Token(other.to_string())
                }
            }
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Emergency => write!(f, "emergency"),
            Self::Urgent => write!(f, "urgent"),
            Self::Normal => write!(f, "normal"),
            Self::NonUrgent => write!(f, "non-urgent"),
            Self::Other(val) => write!(f, "{}", val),
            Self::Token(token) => write!(f, "{}", token),
        }
    }
}

impl FromStr for Priority {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check if string is empty
        if s.is_empty() {
            return Err(Error::InvalidInput("Empty Priority value".to_string()));
        }
        
        Ok(Self::from_token(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_priority_display() {
        assert_eq!(Priority::Emergency.to_string(), "emergency");
        assert_eq!(Priority::Urgent.to_string(), "urgent");
        assert_eq!(Priority::Normal.to_string(), "normal");
        assert_eq!(Priority::NonUrgent.to_string(), "non-urgent");
        assert_eq!(Priority::Other(5).to_string(), "5");
        assert_eq!(Priority::Token("custom".to_string()).to_string(), "custom");
    }
    
    #[test]
    fn test_priority_from_str() {
        assert_eq!("emergency".parse::<Priority>().unwrap(), Priority::Emergency);
        assert_eq!("URGENT".parse::<Priority>().unwrap(), Priority::Urgent);
        assert_eq!("normal".parse::<Priority>().unwrap(), Priority::Normal);
        assert_eq!("non-urgent".parse::<Priority>().unwrap(), Priority::NonUrgent);
        assert_eq!("5".parse::<Priority>().unwrap(), Priority::Other(5));
        assert_eq!("custom".parse::<Priority>().unwrap(), Priority::Token("custom".to_string()));
        assert_eq!("high-priority".parse::<Priority>().unwrap(), Priority::Token("high-priority".to_string()));
        assert!("".parse::<Priority>().is_err());
    }
    
    #[test]
    fn test_priority_comparison() {
        assert!(Priority::Emergency.is_higher_than(&Priority::Urgent));
        assert!(Priority::Urgent.is_higher_than(&Priority::Normal));
        assert!(Priority::Normal.is_higher_than(&Priority::NonUrgent));
        assert!(Priority::Other(1).is_higher_than(&Priority::Other(2)));
        assert!(Priority::Other(10).is_higher_than(&Priority::Token("any".to_string())));
        assert!(!Priority::Normal.is_higher_than(&Priority::Emergency));
    }
    
    #[test]
    fn test_from_token() {
        assert_eq!(Priority::from_token("emergency"), Priority::Emergency);
        assert_eq!(Priority::from_token("URGENT"), Priority::Urgent);
        assert_eq!(Priority::from_token("normal"), Priority::Normal);
        assert_eq!(Priority::from_token("Non-Urgent"), Priority::NonUrgent);
        assert_eq!(Priority::from_token("10"), Priority::Other(10));
        assert_eq!(Priority::from_token("high-priority"), Priority::Token("high-priority".to_string()));
        assert_eq!(Priority::from_token("custom_token"), Priority::Token("custom_token".to_string()));
    }
} 