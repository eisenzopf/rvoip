//! # SIP-ETag Header
//!
//! This module provides an implementation of the SIP-ETag header as defined in
//! [RFC 3903](https://datatracker.ietf.org/doc/html/rfc3903).
//!
//! The SIP-ETag header field is returned in a 2xx response to a PUBLISH request,
//! and indicates the entity-tag associated with the published event state.
//! The client must include this entity-tag in SIP-If-Match headers of subsequent
//! PUBLISH requests for the same event state.
//!
//! ## Format
//!
//! ```text
//! SIP-ETag: dx200xyz
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::sip_etag::SipETag;
//! use std::str::FromStr;
//!
//! // Create a SIP-ETag
//! let etag = SipETag::new("dx200xyz");
//! assert_eq!(etag.to_string(), "dx200xyz");
//!
//! // Parse from string
//! let parsed = SipETag::from_str("abc123").unwrap();
//! assert_eq!(parsed.tag(), "abc123");
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use nom::combinator::all_consuming;

use crate::types::headers::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::parser::headers::parse_sip_etag;
use crate::{Error, Result};

/// Represents the SIP-ETag header value
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SipETag(String);

impl SipETag {
    /// Creates a new SIP-ETag with the given tag value
    ///
    /// # Parameters
    ///
    /// - `tag`: The entity tag value
    ///
    /// # Returns
    ///
    /// A new SIP-ETag header
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }
    
    /// Returns the entity tag value
    pub fn tag(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SipETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SipETag {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        let (_, etag) = all_consuming(parse_sip_etag)(s.as_bytes()).map_err(Error::from)?;
        Ok(etag)
    }
}

impl TypedHeaderTrait for SipETag {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::SipETag
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Invalid header name for SIP-ETag: expected {}, got {}",
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
                "SIP-ETag header value must be raw text".to_string()
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
    fn test_sip_etag_creation() {
        let etag = SipETag::new("test123");
        assert_eq!(etag.tag(), "test123");
        assert_eq!(etag.to_string(), "test123");
    }
    
    #[test]
    fn test_sip_etag_from_str() {
        let etag = SipETag::from_str("abc456").unwrap();
        assert_eq!(etag.tag(), "abc456");
        
        // Empty string should fail
        assert!(SipETag::from_str("").is_err());
    }
    
    #[test]
    fn test_sip_etag_header_conversion() {
        let etag = SipETag::new("xyz789");
        let header = etag.to_header();
        
        assert_eq!(header.name, HeaderName::SipETag);
        
        let parsed = SipETag::from_header(&header).unwrap();
        assert_eq!(parsed, etag);
    }
}