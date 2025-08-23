//! # SIP-If-Match Header
//!
//! This module provides an implementation of the SIP-If-Match header as defined in
//! [RFC 3903](https://datatracker.ietf.org/doc/html/rfc3903).
//!
//! The SIP-If-Match header field is used in PUBLISH requests to indicate the
//! entity-tag of the event state that the request is refreshing, modifying or
//! removing. It must match the entity-tag previously returned in a SIP-ETag
//! header.
//!
//! ## Format
//!
//! ```text
//! SIP-If-Match: dx200xyz
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::sip_if_match::SipIfMatch;
//! use std::str::FromStr;
//!
//! // Create a SIP-If-Match header
//! let if_match = SipIfMatch::new("dx200xyz");
//! assert_eq!(if_match.to_string(), "dx200xyz");
//!
//! // Parse from string
//! let parsed = SipIfMatch::from_str("abc123").unwrap();
//! assert_eq!(parsed.tag(), "abc123");
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use nom::combinator::all_consuming;

use crate::types::headers::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::parser::headers::parse_sip_if_match;
use crate::{Error, Result};

/// Represents the SIP-If-Match header value
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SipIfMatch(String);

impl SipIfMatch {
    /// Creates a new SIP-If-Match with the given tag value
    ///
    /// # Parameters
    ///
    /// - `tag`: The entity tag value to match
    ///
    /// # Returns
    ///
    /// A new SIP-If-Match header
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }
    
    /// Returns the entity tag value
    pub fn tag(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SipIfMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SipIfMatch {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        let (_, if_match) = all_consuming(parse_sip_if_match)(s.as_bytes()).map_err(Error::from)?;
        Ok(if_match)
    }
}

impl TypedHeaderTrait for SipIfMatch {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::SipIfMatch
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Invalid header name for SIP-If-Match: expected {}, got {}",
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
                "SIP-If-Match header value must be raw text".to_string()
            )),
        }
    }
    
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sip_if_match_creation() {
        let if_match = SipIfMatch::new("test123");
        assert_eq!(if_match.tag(), "test123");
        assert_eq!(if_match.to_string(), "test123");
    }
    
    #[test]
    fn test_sip_if_match_from_str() {
        let if_match = SipIfMatch::from_str("abc456").unwrap();
        assert_eq!(if_match.tag(), "abc456");
        
        // Empty string should fail
        assert!(SipIfMatch::from_str("").is_err());
    }
    
    #[test]
    fn test_sip_if_match_header_conversion() {
        let if_match = SipIfMatch::new("xyz789");
        let header = if_match.to_header();
        
        assert_eq!(header.name, HeaderName::SipIfMatch);
        
        let parsed = SipIfMatch::from_header(&header).unwrap();
        assert_eq!(parsed, if_match);
    }
}