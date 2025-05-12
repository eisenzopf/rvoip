//! # SIP RSeq (Response Sequence) Header
//!
//! This module provides an implementation of the SIP RSeq header as defined in
//! [RFC 3262](https://datatracker.ietf.org/doc/html/rfc3262).
//!
//! The RSeq header field contains a single numeric value that determines the order of 
//! provisional responses to a request. It is used by the reliable provisional 
//! response extension (100rel) to enable reliability of provisional responses.
//!
//! A UAC can tell that a server supports reliable provisional responses because 
//! the 100rel option tag will be present in a Supported header in a response from 
//! that server. The UAC can also insist that the server use reliable provisional 
//! responses by placing a Require header with the value 100rel in the request.
//!
//! When a server generates a reliable response (i.e. one for which it will expect
//! a PRACK), it MUST include an RSeq header field in the response. The value of the 
//! header field is a sequence number that MUST be unique for each provisional 
//! response sent by the UAS within a single transaction.
//!
//! ## Format
//!
//! ```
//! RSeq: 1
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a new RSeq header
//! let rseq = RSeq::new(1);
//! assert_eq!(rseq.value, 1);
//!
//! // Parse from a string
//! let rseq = RSeq::from_str("42").unwrap();
//! assert_eq!(rseq.value, 42);
//! ```

use crate::error::Result;
use crate::types::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::{StatusCode, TypedHeader, Method, Require};
use crate::types::{Response, Version};
use crate::types::headers::header_access::HeaderAccess;

/// RSeq header (RFC 3262)
///
/// The RSeq header is used to establish proper ordering of provisional responses
/// in conjunction with the 100rel extension. It contains a single sequence number
/// that is used to match PRACK requests to the provisional responses they acknowledge.
///
/// The sequence number is initialized by the UAS and incremented by one for each
/// reliable provisional response in a single transaction.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a new RSeq header for the first reliable provisional response
/// let rseq = RSeq::new(1);
///
/// // Access the sequence value
/// assert_eq!(rseq.value, 1);
///
/// // Format as a string for a SIP message
/// assert_eq!(rseq.to_string(), "1");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RSeq {
    /// The sequence number
    pub value: u32,
}

impl RSeq {
    /// Create a new RSeq header with the given value
    ///
    /// # Parameters
    ///
    /// - `value`: The sequence number for this provisional response
    ///
    /// # Returns
    ///
    /// A new `RSeq` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create an RSeq header with value 1
    /// let rseq = RSeq::new(1);
    /// assert_eq!(rseq.value, 1);
    ///
    /// // Create an RSeq header with a larger value
    /// let rseq = RSeq::new(42);
    /// assert_eq!(rseq.value, 42);
    /// ```
    pub fn new(value: u32) -> Self {
        Self { value }
    }

    /// Increment the RSeq value and return a new RSeq
    ///
    /// This is useful when generating a sequence of reliable provisional
    /// responses within the same transaction.
    ///
    /// # Returns
    ///
    /// A new `RSeq` instance with a value incremented by 1
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let rseq1 = RSeq::new(1);
    /// let rseq2 = rseq1.next();
    /// assert_eq!(rseq2.value, 2);
    ///
    /// // Original value is unchanged
    /// assert_eq!(rseq1.value, 1);
    /// ```
    pub fn next(&self) -> Self {
        Self {
            value: self.value + 1,
        }
    }
}

impl fmt::Display for RSeq {
    /// Format the RSeq header value as a string
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let rseq = RSeq::new(42);
    /// assert_eq!(rseq.to_string(), "42");
    /// assert_eq!(format!("{}", rseq), "42");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl FromStr for RSeq {
    type Err = crate::error::Error;

    /// Parse an RSeq header from a string
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A `Result` containing the parsed `RSeq` or an error
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple value
    /// let rseq = RSeq::from_str("1").unwrap();
    /// assert_eq!(rseq.value, 1);
    ///
    /// // Parse with whitespace
    /// let rseq = RSeq::from_str(" 42 ").unwrap();
    /// assert_eq!(rseq.value, 42);
    ///
    /// // Invalid input
    /// let result = RSeq::from_str("invalid");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        match trimmed.parse::<u32>() {
            Ok(value) => Ok(RSeq { value }),
            Err(_) => Err(crate::error::Error::ParseError(
                format!("Invalid RSeq value: {}", s)
            )),
        }
    }
}

impl TypedHeaderTrait for RSeq {
    type Name = HeaderName;

    /// Get the header name for RSeq
    ///
    /// # Returns
    ///
    /// The header name (`HeaderName::RSeq`)
    fn header_name() -> Self::Name {
        // Use the proper HeaderName::RSeq variant
        HeaderName::RSeq
    }

    /// Convert this RSeq header to a generic Header
    ///
    /// # Returns
    ///
    /// A generic `Header` containing this RSeq header's data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let rseq = RSeq::new(42);
    /// let header = rseq.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::RSeq);
    /// // The header value contains the sequence number
    /// ```
    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

    /// Create an RSeq header from a generic Header
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A `Result` containing the parsed `RSeq` or an error
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a header with raw value
    /// let header = Header::new(
    ///     HeaderName::RSeq,
    ///     HeaderValue::Raw(b"42".to_vec()),
    /// );
    ///
    /// // Convert to RSeq
    /// let rseq = RSeq::from_header(&header).unwrap();
    /// assert_eq!(rseq.value, 42);
    /// ```
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() && 
           header.name != HeaderName::Other("rseq".to_string()) {
            return Err(crate::error::Error::ParseError(
                format!("Invalid header name for RSeq: {:?}", header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(raw) => {
                let s = std::str::from_utf8(raw)
                    .map_err(|_| crate::error::Error::ParseError(
                        "Invalid UTF-8 in RSeq header value".to_string()
                    ))?;
                Self::from_str(s)
            },
            _ => Err(crate::error::Error::ParseError(
                "Invalid header value type for RSeq".to_string()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StatusCode, TypedHeader, Method, Require};
    use crate::types::{Response, Header, Version};
    use crate::types::headers::header_access::HeaderAccess;

    #[test]
    fn test_rseq_creation() {
        let rseq = RSeq::new(1);
        assert_eq!(rseq.value, 1);
    }

    #[test]
    fn test_rseq_next() {
        let rseq1 = RSeq::new(1);
        let rseq2 = rseq1.next();
        assert_eq!(rseq2.value, 2);
        // Make sure original is unchanged
        assert_eq!(rseq1.value, 1);
    }

    #[test]
    fn test_rseq_display() {
        let rseq = RSeq::new(42);
        assert_eq!(format!("{}", rseq), "42");
    }

    #[test]
    fn test_rseq_fromstr() {
        // Simple case
        let rseq = RSeq::from_str("1").unwrap();
        assert_eq!(rseq.value, 1);
        
        // With whitespace
        let rseq = RSeq::from_str(" 42 ").unwrap();
        assert_eq!(rseq.value, 42);
        
        // Invalid input
        let result = RSeq::from_str("invalid");
        assert!(result.is_err());
        
        // Empty string
        let result = RSeq::from_str("");
        assert!(result.is_err());
    }

    #[test]
    fn test_rseq_to_header() {
        let rseq = RSeq::new(42);
        let header = rseq.to_header();
        
        assert_eq!(header.name, HeaderName::RSeq);
        match &header.value {
            HeaderValue::Raw(raw) => {
                assert_eq!(std::str::from_utf8(raw).unwrap(), "42");
            },
            _ => panic!("Expected HeaderValue::Raw"),
        }
    }

    #[test]
    fn test_rseq_from_header() {
        // Create a header with raw value
        let header = Header::new(
            HeaderName::RSeq,
            HeaderValue::Raw(b"42".to_vec()),
        );
        
        // Convert to RSeq
        let rseq = RSeq::from_header(&header).unwrap();
        assert_eq!(rseq.value, 42);
        
        // Case insensitive header name
        let header = Header::new(
            HeaderName::Other("rseq".to_string()),
            HeaderValue::Raw(b"42".to_vec()),
        );
        
        let rseq = RSeq::from_header(&header).unwrap();
        assert_eq!(rseq.value, 42);
        
        // Wrong header name
        let header = Header::new(
            HeaderName::From,
            HeaderValue::Raw(b"42".to_vec()),
        );
        
        let result = RSeq::from_header(&header);
        assert!(result.is_err());
    }

    #[test]
    fn test_rseq_roundtrip() {
        // Create an RSeq header
        let original = RSeq::new(42);
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to RSeq
        let roundtrip = RSeq::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }
    
    #[test]
    fn test_rseq_in_reliable_provisional_response() {
        // Create a response directly
        let mut response = Response::new(StatusCode::SessionProgress);
        response.version = Version::default();
        response.reason = Some("Session Progress".to_string());
        
        // Add required headers for a reliable provisional response
        let require_header = TypedHeader::Require(Require::with_tag("100rel"));
        let rseq_header = TypedHeader::RSeq(RSeq::new(1));
        
        response.headers.push(require_header);
        response.headers.push(rseq_header);
            
        // Verify the RSeq header was properly added
        let rseq_header = response.headers.iter().find(|h| h.name() == HeaderName::RSeq);
        assert!(rseq_header.is_some());
        
        // Extract and verify RSeq value
        if let Some(header) = rseq_header {
            match header {
                TypedHeader::RSeq(rseq) => {
                    assert_eq!(rseq.value, 1);
                },
                _ => panic!("Expected RSeq header"),
            }
        }
    }
    
    #[test]
    fn test_rseq_sequence_in_dialog() {
        // Create two responses with different RSeq values
        let mut response1 = Response::new(StatusCode::SessionProgress);
        response1.reason = Some("Session Progress".to_string());
        let rseq1 = RSeq::new(1);
        response1.headers.push(TypedHeader::RSeq(rseq1));
        
        let mut response2 = Response::new(StatusCode::SessionProgress);
        response2.reason = Some("Session Progress".to_string());
        let rseq2 = RSeq::new(2);
        response2.headers.push(TypedHeader::RSeq(rseq2));
        
        // Extract and verify the RSeq values
        let header1 = response1.headers.iter().find(|h| h.name() == HeaderName::RSeq).unwrap();
        let header2 = response2.headers.iter().find(|h| h.name() == HeaderName::RSeq).unwrap();
        
        if let (TypedHeader::RSeq(rseq1), TypedHeader::RSeq(rseq2)) = (header1, header2) {
            // Verify sequence incremented correctly
            assert_eq!(rseq1.value, 1);
            assert_eq!(rseq2.value, 2);
            assert_eq!(rseq2.value, rseq1.value + 1);
        } else {
            panic!("Expected RSeq headers");
        }
    }
    
    #[test]
    fn test_typed_header_conversion() {
        // Create an RSeq header
        let rseq = RSeq::new(42);
        
        // Convert to TypedHeader enum
        let typed_header = TypedHeader::RSeq(rseq);
        
        // Verify the header name
        assert_eq!(typed_header.name(), HeaderName::RSeq);
        
        // Test display formatting
        let header_str = typed_header.to_string();
        assert_eq!(header_str, "RSeq: 42");
    }
} 