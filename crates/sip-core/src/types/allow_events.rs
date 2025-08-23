//! # SIP Allow-Events Header
//!
//! This module provides an implementation of the Allow-Events header as defined in
//! [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
//!
//! The Allow-Events header field indicates which event packages are supported by
//! a user agent or proxy. It is typically included in OPTIONS responses and can
//! also appear in 489 (Bad Event) responses.
//!
//! ## Format
//!
//! ```text
//! Allow-Events: presence, message-summary
//! Allow-Events: dialog, conference, refer
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::allow_events::AllowEvents;
//! use std::str::FromStr;
//!
//! // Create an Allow-Events header
//! let allow = AllowEvents::new(vec!["presence", "dialog"]);
//! assert_eq!(allow.to_string(), "presence, dialog");
//!
//! // Parse from string
//! let parsed = AllowEvents::from_str("presence, message-summary, dialog").unwrap();
//! assert_eq!(parsed.events().len(), 3);
//! assert!(parsed.supports("presence"));
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use nom::combinator::all_consuming;

use crate::types::headers::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::parser::headers::parse_allow_events;
use crate::{Error, Result};

/// Represents the Allow-Events header value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowEvents {
    events: Vec<String>,
}

impl AllowEvents {
    /// Creates a new Allow-Events header with the given event packages
    ///
    /// # Parameters
    ///
    /// - `events`: List of supported event packages
    ///
    /// # Returns
    ///
    /// A new Allow-Events header
    pub fn new<I, S>(events: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            events: events.into_iter().map(|e| e.into()).collect(),
        }
    }
    
    /// Returns the list of supported event packages
    pub fn events(&self) -> &[String] {
        &self.events
    }
    
    /// Checks if a specific event package is supported
    ///
    /// # Parameters
    ///
    /// - `event`: The event package name to check
    ///
    /// # Returns
    ///
    /// `true` if the event is supported, `false` otherwise
    pub fn supports(&self, event: &str) -> bool {
        self.events.iter().any(|e| e.eq_ignore_ascii_case(event))
    }
    
    /// Adds an event package to the list of supported events
    ///
    /// # Parameters
    ///
    /// - `event`: The event package to add
    pub fn add_event(&mut self, event: impl Into<String>) {
        let event = event.into();
        if !self.supports(&event) {
            self.events.push(event);
        }
    }
}

impl fmt::Display for AllowEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.events.join(", "))
    }
}

impl FromStr for AllowEvents {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        let (_, allow_events) = all_consuming(parse_allow_events)(s.as_bytes()).map_err(Error::from)?;
        Ok(allow_events)
    }
}

impl TypedHeaderTrait for AllowEvents {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::AllowEvents
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Invalid header name for Allow-Events: expected {}, got {}",
                Self::header_name(),
                header.name
            )));
        }
        
        match &header.value {
            HeaderValue::Raw(bytes) => {
                let value = String::from_utf8_lossy(bytes);
                Self::from_str(&value)
            }
            _ => Err(Error::InvalidHeader(
                "Allow-Events header value must be raw text".to_string()
            )),
        }
    }
    
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }
}

impl Default for AllowEvents {
    fn default() -> Self {
        // Default to supporting common event packages
        Self::new(vec!["presence", "dialog", "message-summary"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_allow_events_creation() {
        let allow = AllowEvents::new(vec!["presence", "dialog"]);
        assert_eq!(allow.events().len(), 2);
        assert!(allow.supports("presence"));
        assert!(allow.supports("dialog"));
        assert!(!allow.supports("conference"));
    }
    
    #[test]
    fn test_allow_events_from_str() {
        let allow = AllowEvents::from_str("presence, message-summary, dialog").unwrap();
        assert_eq!(allow.events().len(), 3);
        assert!(allow.supports("presence"));
        assert!(allow.supports("message-summary"));
        assert!(allow.supports("dialog"));
        
        // Empty string should fail
        assert!(AllowEvents::from_str("").is_err());
        
        // Whitespace handling
        let allow2 = AllowEvents::from_str("  presence  ,  dialog  ").unwrap();
        assert_eq!(allow2.events().len(), 2);
    }
    
    #[test]
    fn test_allow_events_add() {
        let mut allow = AllowEvents::new(vec!["presence"]);
        assert_eq!(allow.events().len(), 1);
        
        allow.add_event("dialog");
        assert_eq!(allow.events().len(), 2);
        
        // Adding duplicate should not increase count
        allow.add_event("presence");
        assert_eq!(allow.events().len(), 2);
    }
    
    #[test]
    fn test_allow_events_header_conversion() {
        let allow = AllowEvents::new(vec!["presence", "dialog"]);
        let header = allow.to_header();
        
        assert_eq!(header.name, HeaderName::AllowEvents);
        
        let parsed = AllowEvents::from_header(&header).unwrap();
        assert_eq!(parsed, allow);
    }
}