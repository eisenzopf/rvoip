//! # SIP CSeq Header
//!
//! This module provides an implementation of the SIP CSeq header as defined in
//! [RFC 3261 Section 20.16](https://datatracker.ietf.org/doc/html/rfc3261#section-20.16).
//!
//! The CSeq (Command Sequence) header serves multiple purposes in SIP:
//!
//! - It uniquely identifies transactions within a dialog
//! - It orders transactions in a dialog
//! - It distinguishes between new requests and retransmissions
//! - It matches requests to their corresponding responses
//!
//! ## Format
//!
//! ```text
//! CSeq: 4711 INVITE
//! CSeq: 21 BYE
//! ```
//!
//! The CSeq header consists of a sequence number (an unsigned 32-bit integer)
//! and a method name. The method name MUST match the method in the request line.
//!
//! ## Usage in Dialog Management
//!
//! Within a dialog, the CSeq value for requests sent by the dialog initiator 
//! increases by one for each new request. The CSeq for the dialog recipient 
//! also increases by one for each new request, but uses a separate counter from 
//! the initiator.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a CSeq for an INVITE request
//! let invite_cseq = CSeq::new(1, Method::Invite);
//! assert_eq!(invite_cseq.to_string(), "1 INVITE");
//!
//! // Parse a CSeq header
//! let cseq = CSeq::from_str("2 BYE").unwrap();
//! assert_eq!(cseq.sequence(), 2);
//! assert_eq!(cseq.method(), &Method::Bye);
//!
//! // Increment the sequence number for a new request
//! let next_cseq = cseq.increment();
//! assert_eq!(next_cseq.sequence(), 3);
//! ```

use crate::types::method::Method;
use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::parser::headers::cseq::{parse_cseq, full_parse_cseq};
use serde::{Serialize, Deserialize};
use crate::types::header::Header;
use crate::types::{HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};

/// Represents the CSeq header field (RFC 3261 Section 8.1.1.5).
/// 
/// The CSeq header field serves as a way to identify and order transactions.
/// It consists of a sequence number and a method. The method name in the
/// CSeq header field MUST match the method name in the start-line, and the
/// sequence number value MUST be expressible as a 32-bit unsigned integer.
/// 
/// The CSeq header is mandatory in all SIP requests and responses. It helps
/// to identify retransmissions, match responses to requests, and maintain
/// the order of transactions within a dialog.
/// 
/// # Examples
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// 
/// // Create from sequence number and Method enum
/// let cseq = CSeq::new(101, Method::Invite);
/// assert_eq!(cseq.to_string(), "101 INVITE");
/// 
/// // Create from string value
/// let cseq2: CSeq = "102 ACK".parse().unwrap();
/// assert_eq!(cseq2.sequence(), 102);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CSeq {
    /// Sequence number (32-bit unsigned integer)
    pub seq: u32,
    /// SIP method
    pub method: Method,
}

impl CSeq {
    /// Creates a new CSeq header with the specified sequence number and method.
    ///
    /// # Parameters
    ///
    /// - `seq`: The sequence number, a 32-bit unsigned integer
    /// - `method`: The SIP method (Invite, Bye, etc.)
    ///
    /// # Returns
    ///
    /// A new `CSeq` instance with the specified sequence number and method
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a CSeq for an INVITE request
    /// let cseq = CSeq::new(1, Method::Invite);
    /// assert_eq!(cseq.sequence(), 1);
    /// assert_eq!(cseq.method(), &Method::Invite);
    ///
    /// // Create a CSeq for a BYE request
    /// let cseq = CSeq::new(2, Method::Bye);
    /// assert_eq!(cseq.to_string(), "2 BYE");
    /// ```
    pub fn new(seq: u32, method: Method) -> Self {
        Self { seq, method }
    }
    
    /// Creates a new CSeq header with the specified sequence number and method string.
    /// 
    /// This is a convenience method that parses a method string into a `Method` enum
    /// and creates a CSeq header.
    /// 
    /// # Parameters
    ///
    /// - `seq`: The sequence number, a 32-bit unsigned integer
    /// - `method_str`: The method name as a string (e.g., "INVITE", "BYE")
    ///
    /// # Returns
    ///
    /// A Result containing the new `CSeq` instance, or an error if the method string
    /// cannot be parsed
    ///
    /// # Errors
    /// 
    /// Returns an error if the method string is not a valid SIP method.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a CSeq with a standard method name
    /// let cseq = CSeq::with_method_str(1, "INVITE").unwrap();
    /// assert_eq!(cseq.to_string(), "1 INVITE");
    ///
    /// // Create a CSeq with an extension method name
    /// let cseq = CSeq::with_method_str(2, "CUSTOM").unwrap();
    /// assert_eq!(cseq.method(), &Method::Extension("CUSTOM".to_string()));
    ///
    /// // Invalid method name (empty string)
    /// let result = CSeq::with_method_str(3, "");
    /// assert!(result.is_err());
    /// ```
    pub fn with_method_str(seq: u32, method_str: &str) -> Result<Self> {
        let method = Method::from_str(method_str)?;
        Ok(Self::new(seq, method))
    }
    
    /// Get the sequence number.
    ///
    /// # Returns
    ///
    /// The sequence number as a u32
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let cseq = CSeq::new(42, Method::Invite);
    /// assert_eq!(cseq.sequence(), 42);
    ///
    /// // Using the sequence number in calculations
    /// let next_seq = cseq.sequence() + 1;
    /// assert_eq!(next_seq, 43);
    /// ```
    pub fn sequence(&self) -> u32 {
        self.seq
    }
    
    /// Get the method.
    ///
    /// # Returns
    ///
    /// A reference to the Method enum
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let cseq = CSeq::new(42, Method::Invite);
    /// assert_eq!(cseq.method(), &Method::Invite);
    ///
    /// // Use in pattern matching
    /// match cseq.method() {
    ///     &Method::Invite => println!("This is an INVITE!"),
    ///     &Method::Bye => println!("This is a BYE!"),
    ///     _ => println!("This is some other method"),
    /// }
    /// ```
    pub fn method(&self) -> &Method {
        &self.method
    }
    
    /// Increments the sequence number by 1 and returns a new CSeq with the same method.
    ///
    /// This is useful for creating a new CSeq for the next request in a dialog
    /// using the same method.
    /// 
    /// # Returns
    ///
    /// A new `CSeq` with the sequence number incremented by 1 and the same method
    ///
    /// # Panics
    /// 
    /// Panics if the sequence number would overflow (exceed u32::MAX).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let cseq = CSeq::new(1, Method::Invite);
    /// let next_cseq = cseq.increment();
    ///
    /// assert_eq!(next_cseq.sequence(), 2);
    /// assert_eq!(next_cseq.method(), &Method::Invite);
    ///
    /// // Original CSeq is unchanged
    /// assert_eq!(cseq.sequence(), 1);
    /// ```
    pub fn increment(&self) -> Self {
        Self {
            seq: self.seq.checked_add(1).expect("CSeq sequence number overflow"),
            method: self.method.clone(),
        }
    }
    
    /// Increments the sequence number by 1 and changes the method.
    ///
    /// This is useful for creating a new CSeq for the next request in a dialog
    /// with a different method.
    /// 
    /// # Parameters
    ///
    /// - `method`: The new method to use
    ///
    /// # Returns
    ///
    /// A new `CSeq` with the sequence number incremented by 1 and the specified method
    /// 
    /// # Panics
    /// 
    /// Panics if the sequence number would overflow (exceed u32::MAX).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let invite_cseq = CSeq::new(1, Method::Invite);
    /// let bye_cseq = invite_cseq.increment_with_method(Method::Bye);
    ///
    /// assert_eq!(bye_cseq.sequence(), 2);
    /// assert_eq!(bye_cseq.method(), &Method::Bye);
    ///
    /// // Using for multiple sequential requests
    /// let cseq1 = CSeq::new(1, Method::Invite);
    /// let cseq2 = cseq1.increment_with_method(Method::Ack);
    /// let cseq3 = cseq2.increment_with_method(Method::Bye);
    ///
    /// assert_eq!(cseq3.sequence(), 3);
    /// assert_eq!(cseq3.method(), &Method::Bye);
    /// ```
    pub fn increment_with_method(&self, method: Method) -> Self {
        Self {
            seq: self.seq.checked_add(1).expect("CSeq sequence number overflow"),
            method,
        }
    }
}

impl fmt::Display for CSeq {
    /// Formats the CSeq as a string.
    ///
    /// The format follows the SIP specification: sequence number, followed by
    /// a space, followed by the method name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let cseq = CSeq::new(42, Method::Invite);
    /// assert_eq!(cseq.to_string(), "42 INVITE");
    ///
    /// // Using in a formatted string
    /// let header = format!("CSeq: {}", cseq);
    /// assert_eq!(header, "CSeq: 42 INVITE");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.seq, self.method) // Method already implements Display
    }
}

impl FromStr for CSeq {
    type Err = Error;

    /// Parses a string into a CSeq.
    ///
    /// This method can parse both the full header (with "CSeq:" prefix) and
    /// just the header value. It is case-insensitive for the header name.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed CSeq, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse just the value
    /// let cseq = CSeq::from_str("42 INVITE").unwrap();
    /// assert_eq!(cseq.sequence(), 42);
    /// assert_eq!(cseq.method(), &Method::Invite);
    ///
    /// // Parse the full header
    /// let cseq = CSeq::from_str("CSeq: 43 BYE").unwrap();
    /// assert_eq!(cseq.sequence(), 43);
    /// assert_eq!(cseq.method(), &Method::Bye);
    ///
    /// // Case-insensitive header name
    /// let cseq = CSeq::from_str("cseq: 44 ACK").unwrap();
    /// assert_eq!(cseq.sequence(), 44);
    /// assert_eq!(cseq.method(), &Method::Ack);
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let trimmed_s = s.trim();
        
        // Try parsing as a full header first (with "CSeq:" prefix)
        let full_result = all_consuming(full_parse_cseq)(trimmed_s.as_bytes());
        if let Ok((_, cseq)) = full_result {
            return Ok(cseq);
        }
        
        // If that fails, try parsing just the value part
        all_consuming(parse_cseq)(trimmed_s.as_bytes())
            .map(|(_, cseq)| cseq)
            .map_err(Error::from)
    }
}

// Add TypedHeaderTrait implementation
impl TypedHeaderTrait for CSeq {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::CSeq` enum variant
    fn header_name() -> Self::Name {
        HeaderName::CSeq
    }

    /// Converts this CSeq header into a generic Header.
    ///
    /// Creates a Header instance from this CSeq header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A Header instance representing this CSeq header
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::CSeq(self.clone()))
    }

    /// Creates a CSeq header from a generic Header.
    ///
    /// Attempts to parse and convert a generic Header into a CSeq header.
    /// This will succeed if the header is a valid CSeq header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed CSeq header if successful, or an error otherwise
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::CSeq {
            return Err(Error::InvalidHeader(format!(
                "Expected CSeq header, got {:?}", header.name
            )));
        }

        // Try to use the pre-parsed value if available
        if let HeaderValue::CSeq(value) = &header.value {
            return Ok(value.clone());
        }

        // Otherwise parse from raw value
        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    s.parse::<CSeq>()
                } else {
                    Err(Error::ParseError("Invalid UTF-8 in CSeq header".to_string()))
                }
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected value type for CSeq header: {:?}", header.value
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_from_str() {
        // Test parsing with just value
        let value = "101 INVITE";
        let cseq: CSeq = value.parse().unwrap();
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
        
        // Test parsing with full header
        let header = "CSeq: 202 ACK";
        let cseq2: CSeq = header.parse().unwrap();
        assert_eq!(cseq2.seq, 202);
        assert_eq!(cseq2.method, Method::Ack);
        
        // Test with lowercase header name
        let header_lower = "cseq: 303 BYE";
        let cseq3: CSeq = header_lower.parse().unwrap();
        assert_eq!(cseq3.seq, 303);
        assert_eq!(cseq3.method, Method::Bye);
    }
    
    #[test]
    fn test_display() {
        let cseq = CSeq::new(101, Method::Invite);
        assert_eq!(cseq.to_string(), "101 INVITE");
    }
    
    #[test]
    fn test_with_method_str() {
        // Standard method
        let cseq = CSeq::with_method_str(101, "INVITE").unwrap();
        assert_eq!(cseq.seq, 101);
        assert_eq!(cseq.method, Method::Invite);
        
        // Lowercase methods are treated as extensions
        let lowercase_result = CSeq::with_method_str(101, "invite").unwrap();
        assert_eq!(lowercase_result.method, Method::Extension("invite".into()),
                   "Lowercase methods are treated as extensions, not standard methods");
        
        // Custom/invalid method names are accepted as extensions
        let custom_result = CSeq::with_method_str(101, "INVALID-METHOD-NAME").unwrap();
        assert_eq!(custom_result.method, Method::Extension("INVALID-METHOD-NAME".into()),
                  "Custom method names are accepted as extensions");
        
        // Test with empty method (should fail)
        let empty_result = CSeq::with_method_str(101, "");
        assert!(empty_result.is_err(), "Empty method name should be rejected");
    }
    
    #[test]
    fn test_increment() {
        let cseq = CSeq::new(101, Method::Invite);
        let incremented = cseq.increment();
        assert_eq!(incremented.seq, 102);
        assert_eq!(incremented.method, Method::Invite);
        
        // Test method change
        let with_new_method = cseq.increment_with_method(Method::Bye);
        assert_eq!(with_new_method.seq, 102);
        assert_eq!(with_new_method.method, Method::Bye);
    }

    #[test]
    fn test_cseq_typed_header_trait() {
        // Create a CSeq header
        let cseq = CSeq::new(12345, Method::Register);

        // Test header_name()
        assert_eq!(CSeq::header_name(), HeaderName::CSeq);

        // Test to_header()
        let header = cseq.to_header();
        assert_eq!(header.name, HeaderName::CSeq);

        // Test from_header()
        let round_trip = CSeq::from_header(&header).unwrap();
        assert_eq!(round_trip, cseq);
    }
} 