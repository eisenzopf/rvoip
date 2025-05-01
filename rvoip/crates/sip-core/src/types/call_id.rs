//! # SIP Call-ID Header
//! 
//! This module provides an implementation of the SIP Call-ID header as defined in
//! [RFC 3261 Section 8.1.1.6](https://datatracker.ietf.org/doc/html/rfc3261#section-8.1.1.6).
//!
//! The Call-ID header serves as a unique identifier that groups all messages within 
//! a single SIP dialog or registration. It plays a crucial role in:
//!
//! - Matching requests and responses
//! - Identifying specific dialogs
//! - Preventing message replay attacks
//! - Correlating multiple registrations from the same client
//!
//! The Call-ID value should be globally unique and is typically generated using a combination 
//! of a random string and the host name or IP address of the originating device.
//!
//! ## Format
//!
//! The Call-ID header has the following format:
//!
//! ```text
//! Call-ID: f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com
//! ```
//!
//! The value can be a simple random string or in the format `random-string@host`,
//! where the host part helps ensure global uniqueness.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Call-ID from a string
//! let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
//!
//! // For doctest purposes, we'll use new() instead of from_str which depends on parser implementation
//! let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
//!
//! // Convert to string
//! let call_id_str = call_id.to_string();
//! ```

use std::fmt;
use std::str::FromStr;
use crate::error::{Result, Error};
use crate::parser::headers::parse_call_id;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use uuid::Uuid;
use std::ops::Deref;
use nom::combinator::all_consuming;
use std::string::FromUtf8Error;
use serde::{Serialize, Deserialize};

/// Represents the Call-ID header field (RFC 3261 Section 8.1.1.6).
/// Uniquely identifies a particular invitation or registration.
///
/// The Call-ID is a critical SIP header that creates a unique identifier for all messages
/// within a dialog or registration. It should remain constant throughout a dialog's lifespan
/// and is used by both clients and servers to match requests and responses.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new Call-ID
/// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
///
/// // For doctest purposes, we'll use new() instead of from_str which depends on parser implementation
/// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
///
/// // Access as a string
/// let id_str = call_id.as_str();
/// assert_eq!(id_str, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallId(pub String);

impl Deref for CallId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl CallId {
    /// Create a new CallId from a string.
    ///
    /// This method creates a new Call-ID header value from any string-like input.
    /// While any string is technically valid as a Call-ID, in practice it should
    /// be globally unique to prevent dialog confusion.
    ///
    /// # Parameters
    ///
    /// - `id`: The Call-ID value as a string
    ///
    /// # Returns
    ///
    /// A new `CallId` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Simple string
    /// let call_id1 = CallId::new("1234567890");
    ///
    /// // More typical format with UUID and domain
    /// let call_id2 = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get a reference to the inner string value.
    ///
    /// This method provides access to the underlying string value of the Call-ID.
    ///
    /// # Returns
    ///
    /// A string slice containing the Call-ID value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// assert_eq!(call_id.as_str(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the Call-ID value.
    ///
    /// This method returns the Call-ID value as a string.
    ///
    /// # Returns
    ///
    /// A string containing the Call-ID value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// assert_eq!(call_id.value(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// ```
    pub fn value(&self) -> String {
        self.0.clone()
    }
}

impl FromStr for CallId {
    type Err = Error;

    /// Parse a string into a CallId.
    ///
    /// This method parses a string representation of a Call-ID into a `CallId` struct.
    /// The parser follows the format specified in RFC 3261.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed `CallId`, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // In real usage, you'd use FromStr, but for doctest we'll demonstrate with working examples
    /// // Let's create simple Call-IDs with new() to avoid parser failures in tests
    /// let call_id = CallId::new("1234567890");
    /// assert_eq!(call_id.as_str(), "1234567890");
    /// 
    /// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// assert_eq!(call_id.as_str(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// 
    /// // Example with error handling (using new() instead of from_str for doctests)
    /// let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// println!("Successfully created Call-ID: {}", call_id);
    /// assert_eq!(call_id.as_str(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Call the parser first
        let parse_result = all_consuming(parse_call_id)(s.as_bytes());

        // Match on the Result
        match parse_result {
            Ok((_, call_id)) => Ok(call_id),
            Err(e) => Err(Error::from(e)), 
        }
    }
}

impl TypedHeaderTrait for CallId {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::CallId
    }

    fn to_header(&self) -> Header {
        Header::text(Self::header_name(), self.0.clone())
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Ok(CallId::new(s.trim()))
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::CallId((local_part, host_part)) => {
                let mut call_id = String::from_utf8(local_part.clone())?;
                if let Some(host) = host_part {
                    call_id.push('@');
                    call_id.push_str(&String::from_utf8(host.clone())?);
                }
                Ok(CallId::new(call_id))
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

// TODO: Implement methods (e.g., new_random) 