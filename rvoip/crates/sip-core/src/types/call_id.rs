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
//! ```
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
//! // Parse a Call-ID from a string
//! let call_id = CallId::from_str("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com").unwrap();
//!
//! // Convert to string
//! let call_id_str = call_id.to_string();
//! ```

use std::fmt;
use std::str::FromStr;
use crate::error::{Result, Error};
use crate::parser::headers::parse_call_id;
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
/// // Parse from a string
/// let call_id = CallId::from_str("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com").unwrap();
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
    /// // Parse a simple Call-ID
    /// let call_id = CallId::from_str("1234567890").unwrap();
    ///
    /// // Parse a Call-ID with the typical format
    /// let call_id = CallId::from_str("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com").unwrap();
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

// TODO: Implement methods (e.g., new_random) 