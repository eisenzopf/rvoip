// Priority types for SIP Priority header
// Values according to RFC 3261 Section 20.26

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::error::Error;

/// Priority values for SIP Priority header
/// As defined in RFC 3261 Section 20.26
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Emergency call
    Emergency,
    
    /// Urgent call/message
    Urgent,
    
    /// Normal call/message (default)
    Normal,
    
    /// Non-urgent call/message
    NonUrgent,
    
    /// Other priority value (extension)
    Other(u8),
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
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Emergency => write!(f, "emergency"),
            Self::Urgent => write!(f, "urgent"),
            Self::Normal => write!(f, "normal"),
            Self::NonUrgent => write!(f, "non-urgent"),
            Self::Other(val) => write!(f, "{}", val),
        }
    }
}

impl FromStr for Priority {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "emergency" => Ok(Self::Emergency),
            "urgent" => Ok(Self::Urgent),
            "normal" => Ok(Self::Normal),
            "non-urgent" => Ok(Self::NonUrgent),
            // Try to parse as a number for extensions
            _ => match s.parse::<u8>() {
                Ok(val) => Ok(Self::Other(val)),
                Err(_) => Err(Error::InvalidValue("Invalid Priority value".to_string())),
            },
        }
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
    }
    
    #[test]
    fn test_priority_from_str() {
        assert_eq!("emergency".parse::<Priority>().unwrap(), Priority::Emergency);
        assert_eq!("URGENT".parse::<Priority>().unwrap(), Priority::Urgent);
        assert_eq!("normal".parse::<Priority>().unwrap(), Priority::Normal);
        assert_eq!("non-urgent".parse::<Priority>().unwrap(), Priority::NonUrgent);
        assert_eq!("5".parse::<Priority>().unwrap(), Priority::Other(5));
        assert!("invalid".parse::<Priority>().is_err());
    }
    
    #[test]
    fn test_priority_comparison() {
        assert!(Priority::Emergency.is_higher_than(&Priority::Urgent));
        assert!(Priority::Urgent.is_higher_than(&Priority::Normal));
        assert!(Priority::Normal.is_higher_than(&Priority::NonUrgent));
        assert!(Priority::Other(1).is_higher_than(&Priority::Other(2)));
        assert!(!Priority::Normal.is_higher_than(&Priority::Emergency));
    }
} 