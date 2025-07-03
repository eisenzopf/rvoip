//! # SIP Priority Header
//! 
//! This module provides an implementation of the SIP Priority header as defined in
//! [RFC 3261 Section 20.26](https://datatracker.ietf.org/doc/html/rfc3261#section-20.26).
//!
//! The Priority header field indicates the urgency of a request as perceived by the 
//! client. It helps receiving UAs or proxies to appropriately prioritize signaling operations
//! based on urgency.
//!
//! ## Standard Priority Values
//!
//! SIP defines four standard priority values, in decreasing order of urgency:
//!
//! - `emergency`: Emergency sessions that involve human safety
//! - `urgent`: Urgent sessions that must be answered immediately
//! - `normal`: Normal sessions with no particular urgency (default)
//! - `non-urgent`: Sessions that do not require immediate response
//!
//! ## Extended Priority Values
//!
//! Per RFC 3261, the Priority field can also accept:
//!
//! - Numeric priority values
//! - Extension tokens for application-specific priority levels
//!
//! ## Usage
//!
//! The Priority header is primarily used to:
//! - Indicate urgency of requests to user agents
//! - Guide proxy forwarding behavior
//! - Help prioritize limited resources during processing
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create standard priorities
//! let emergency = Priority::Emergency;
//! let normal = Priority::default(); // Returns Normal
//!
//! // Parse from string
//! let urgent = Priority::from_str("urgent").unwrap();
//! assert_eq!(urgent, Priority::Urgent);
//!
//! // Compare priorities
//! assert!(emergency.is_higher_than(&urgent));
//! assert!(urgent.is_higher_than(&normal));
//!
//! // Create custom priorities
//! let numeric = Priority::Other(5);
//! let token = Priority::from_token("high-priority");
//! ```

// Priority types for SIP Priority header
// Values according to RFC 3261 Section 20.26

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::error::Error;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Priority values for SIP Priority header
/// As defined in RFC 3261 Section 20.26
///
/// The Priority header field indicates the importance or urgency of a SIP request.
/// It helps endpoints and proxies make decisions about request handling and
/// resource allocation based on the perceived urgency.
///
/// This enum represents the standard priority levels defined in RFC 3261,
/// as well as support for numeric values and extension tokens.
///
/// # Standard Priorities
///
/// The standard priorities, from highest to lowest, are:
/// - `Emergency`: For emergency communications involving human safety
/// - `Urgent`: For urgent communications that require immediate attention
/// - `Normal`: For ordinary communications (default)
/// - `NonUrgent`: For communications that can be delayed
///
/// # Extensions
///
/// In addition to the standard values, the Priority can be:
/// - `Other(u8)`: A numeric priority value where lower numbers indicate higher priority
/// - `Token(String)`: Application-specific priority defined as a string token
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Standard priorities
/// let emergency = Priority::Emergency;
/// let urgent = Priority::Urgent;
/// let normal = Priority::Normal;
/// let non_urgent = Priority::NonUrgent;
///
/// // Checking if one priority is higher than another
/// assert!(emergency.is_higher_than(&urgent));
/// assert!(urgent.is_higher_than(&normal));
/// assert!(normal.is_higher_than(&non_urgent));
///
/// // Custom priorities
/// let numeric = Priority::Other(10);
/// let token = Priority::Token("high-priority".to_string());
///
/// // Parsing from a string
/// assert_eq!(Priority::from_str("emergency").unwrap(), Priority::Emergency);
/// assert_eq!(Priority::from_str("5").unwrap(), Priority::Other(5));
/// ```
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
    ///
    /// This method returns a numeric representation of the priority level,
    /// where lower numbers indicate higher priority.
    ///
    /// # Return Values
    ///
    /// - `Emergency`: 0 (highest priority)
    /// - `Urgent`: 1
    /// - `Normal`: 2
    /// - `NonUrgent`: 3
    /// - `Other(val)`: The provided value
    /// - `Token(_)`: 99 (lowest priority by default)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(Priority::Emergency.value(), 0);
    /// assert_eq!(Priority::Urgent.value(), 1);
    /// assert_eq!(Priority::Normal.value(), 2);
    /// assert_eq!(Priority::NonUrgent.value(), 3);
    /// assert_eq!(Priority::Other(5).value(), 5);
    /// assert_eq!(Priority::Token("custom".to_string()).value(), 99);
    /// ```
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
    ///
    /// Compares the numeric priority values (lower values are higher priority) and
    /// returns true if this priority is higher than the other.
    ///
    /// # Parameters
    ///
    /// - `other`: The priority to compare against
    ///
    /// # Returns
    ///
    /// `true` if this priority is higher than the other, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Standard priorities
    /// assert!(Priority::Emergency.is_higher_than(&Priority::Urgent));
    /// assert!(Priority::Urgent.is_higher_than(&Priority::Normal));
    /// assert!(Priority::Normal.is_higher_than(&Priority::NonUrgent));
    ///
    /// // Custom numeric priorities
    /// assert!(Priority::Other(1).is_higher_than(&Priority::Other(2)));
    /// assert!(Priority::Emergency.is_higher_than(&Priority::Other(5)));
    ///
    /// // Token priorities
    /// assert!(Priority::Normal.is_higher_than(&Priority::Token("custom".to_string())));
    /// ```
    pub fn is_higher_than(&self, other: &Priority) -> bool {
        self.value() < other.value()
    }
    
    /// Get the default priority (Normal)
    ///
    /// Returns the default priority value, which is Normal according to RFC 3261.
    ///
    /// # Returns
    ///
    /// A `Priority::Normal` value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let default = Priority::default();
    /// assert_eq!(default, Priority::Normal);
    /// ```
    pub fn default() -> Self {
        Self::Normal
    }
    
    /// Create a new priority from a token
    ///
    /// Parses a string token into a Priority value. The parsing is case-insensitive
    /// for the standard priority values.
    ///
    /// # Parameters
    ///
    /// - `token`: The string token to parse
    ///
    /// # Returns
    ///
    /// A `Priority` enum variant:
    /// - For "emergency", "urgent", "normal", "non-urgent": The corresponding standard value
    /// - For numeric strings: `Priority::Other` with the parsed value
    /// - For other strings: `Priority::Token` with the string
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Standard priorities (case-insensitive)
    /// assert_eq!(Priority::from_token("emergency"), Priority::Emergency);
    /// assert_eq!(Priority::from_token("URGENT"), Priority::Urgent);
    /// assert_eq!(Priority::from_token("normal"), Priority::Normal);
    /// assert_eq!(Priority::from_token("non-urgent"), Priority::NonUrgent);
    ///
    /// // Numeric priority
    /// assert_eq!(Priority::from_token("5"), Priority::Other(5));
    ///
    /// // Custom token
    /// assert_eq!(
    ///     Priority::from_token("high-priority"),
    ///     Priority::Token("high-priority".to_string())
    /// );
    /// ```
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
    /// Formats the Priority as a string.
    ///
    /// This method converts the priority value to its canonical string
    /// representation according to RFC 3261.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(Priority::Emergency.to_string(), "emergency");
    /// assert_eq!(Priority::Urgent.to_string(), "urgent");
    /// assert_eq!(Priority::Normal.to_string(), "normal");
    /// assert_eq!(Priority::NonUrgent.to_string(), "non-urgent");
    /// assert_eq!(Priority::Other(5).to_string(), "5");
    /// assert_eq!(Priority::Token("custom".to_string()).to_string(), "custom");
    ///
    /// // Using in a formatted string
    /// let priority = Priority::Urgent;
    /// let header = format!("Priority: {}", priority);
    /// assert_eq!(header, "Priority: urgent");
    /// ```
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
    
    /// Parses a string into a Priority.
    ///
    /// This method converts a string to the corresponding Priority enum variant.
    /// It leverages the `from_token` method for the actual parsing logic.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Priority, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns an `Error::InvalidInput` if the input string is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Standard priorities
    /// assert_eq!(Priority::from_str("emergency").unwrap(), Priority::Emergency);
    /// assert_eq!(Priority::from_str("URGENT").unwrap(), Priority::Urgent);
    /// assert_eq!(Priority::from_str("normal").unwrap(), Priority::Normal);
    /// assert_eq!(Priority::from_str("non-urgent").unwrap(), Priority::NonUrgent);
    ///
    /// // Numeric priority
    /// assert_eq!(Priority::from_str("5").unwrap(), Priority::Other(5));
    ///
    /// // Custom token
    /// assert_eq!(
    ///     Priority::from_str("high-priority").unwrap(),
    ///     Priority::Token("high-priority".to_string())
    /// );
    ///
    /// // Error case
    /// assert!(Priority::from_str("").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check if string is empty
        if s.is_empty() {
            return Err(Error::InvalidInput("Empty Priority value".to_string()));
        }
        
        Ok(Self::from_token(s))
    }
}

// Implement TypedHeaderTrait for Priority
impl TypedHeaderTrait for Priority {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Priority
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self, Error> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Priority::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Priority(bytes) => {
                // Convert the byte vector to a string and then parse it
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Priority::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
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