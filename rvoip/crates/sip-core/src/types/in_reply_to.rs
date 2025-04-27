//! # SIP In-Reply-To Header
//!
//! This module provides an implementation of the SIP In-Reply-To header as defined in
//! [RFC 3261 Section 20.22](https://datatracker.ietf.org/doc/html/rfc3261#section-20.22).
//!
//! The In-Reply-To header field enumerates the Call-IDs of previous requests that this
//! request references or is a reply to. It serves to establish relationships between
//! different SIP dialogs or call sessions.
//!
//! ## Purpose
//!
//! The In-Reply-To header is primarily used to:
//!
//! - Link two or more calls together as part of a multi-call scenario
//! - Establish parent-child relationships between calls
//! - Reference a previous call that is being returned or continued
//! - Implement call transfers or consultations
//!
//! ## Format
//!
//! ```
//! In-Reply-To: 70710@saturn.bell-tel.com
//! In-Reply-To: 70710@saturn.bell-tel.com, 17320@venus.bell-tel.com
//! ```
//!
//! Multiple Call-IDs are separated by commas.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an In-Reply-To header with a single Call-ID
//! let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
//! 
//! // Create with multiple Call-IDs
//! let in_reply_to = InReplyTo::from_str("70710@saturn.bell-tel.com, 17320@venus.bell-tel.com").unwrap();
//! assert_eq!(in_reply_to.len(), 2);
//! ```

use std::fmt;
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use std::ops::Deref;

use crate::error::{Error, Result};
use crate::types::{HeaderName, HeaderValue, Header, TypedHeaderTrait, CallId};
use crate::parser::headers::in_reply_to::parse_in_reply_to;

/// Represents an In-Reply-To header field (RFC 3261 Section 20.22).
/// Contains one or more Call-IDs of previous requests to which this request is a reply.
///
/// The In-Reply-To header provides references to previous call sessions, allowing
/// a SIP request to establish relationships with earlier dialogs. This is useful for
/// implementing features like call returns, consultations, and transfers.
///
/// This implementation wraps a vector of `CallId` objects and implements `Deref` to 
/// provide direct access to the underlying vector's methods.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create with a single Call-ID
/// let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
/// assert_eq!(in_reply_to.len(), 1);
///
/// // Create with multiple Call-IDs
/// let call_ids = vec![
///     CallId("70710@saturn.bell-tel.com".to_string()),
///     CallId("17320@venus.bell-tel.com".to_string())
/// ];
/// let in_reply_to = InReplyTo::with_multiple(call_ids);
/// assert_eq!(in_reply_to.len(), 2);
///
/// // Parse from string
/// let in_reply_to = InReplyTo::from_str("70710@saturn.bell-tel.com, 17320@venus.bell-tel.com").unwrap();
/// assert!(in_reply_to.contains("17320@venus.bell-tel.com"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InReplyTo(pub Vec<CallId>);

impl InReplyTo {
    /// Creates a new In-Reply-To header with a single Call-ID.
    ///
    /// # Parameters
    ///
    /// - `call_id`: A string representing the Call-ID to include
    ///
    /// # Returns
    ///
    /// A new `InReplyTo` instance with the specified Call-ID
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// assert_eq!(in_reply_to.len(), 1);
    /// assert_eq!(in_reply_to[0].as_str(), "70710@saturn.bell-tel.com");
    /// ```
    pub fn new(call_id: impl Into<String>) -> Self {
        Self(vec![CallId(call_id.into())])
    }
    
    /// Creates a new In-Reply-To header with multiple Call-IDs.
    ///
    /// # Parameters
    ///
    /// - `call_ids`: A vector of CallId objects
    ///
    /// # Returns
    ///
    /// A new `InReplyTo` instance with the provided Call-IDs
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_ids = vec![
    ///     CallId("70710@saturn.bell-tel.com".to_string()),
    ///     CallId("17320@venus.bell-tel.com".to_string())
    /// ];
    /// let in_reply_to = InReplyTo::with_multiple(call_ids);
    /// assert_eq!(in_reply_to.len(), 2);
    /// ```
    pub fn with_multiple(call_ids: Vec<CallId>) -> Self {
        Self(call_ids)
    }
    
    /// Creates a new In-Reply-To header with multiple Call-IDs from strings.
    ///
    /// This is a convenience method that converts a vector of strings to CallId objects.
    ///
    /// # Parameters
    ///
    /// - `call_ids`: A vector of strings representing Call-IDs
    ///
    /// # Returns
    ///
    /// A new `InReplyTo` instance with the provided Call-IDs
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_ids = vec![
    ///     "70710@saturn.bell-tel.com".to_string(),
    ///     "17320@venus.bell-tel.com".to_string()
    /// ];
    /// let in_reply_to = InReplyTo::with_multiple_strings(call_ids);
    /// assert_eq!(in_reply_to.len(), 2);
    /// ```
    pub fn with_multiple_strings(call_ids: Vec<String>) -> Self {
        Self(call_ids.into_iter().map(CallId).collect())
    }
    
    /// Adds a Call-ID to the list.
    ///
    /// # Parameters
    ///
    /// - `call_id`: A string representing the Call-ID to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// in_reply_to.add("17320@venus.bell-tel.com");
    /// 
    /// assert_eq!(in_reply_to.len(), 2);
    /// assert!(in_reply_to.contains("17320@venus.bell-tel.com"));
    /// ```
    pub fn add(&mut self, call_id: impl Into<String>) {
        self.0.push(CallId(call_id.into()));
    }
    
    /// Returns true if the header contains the specified Call-ID.
    ///
    /// This method provides a convenient way to check if a specific
    /// Call-ID is referenced by this In-Reply-To header.
    ///
    /// # Parameters
    ///
    /// - `call_id`: The Call-ID string to check for
    ///
    /// # Returns
    ///
    /// `true` if the Call-ID is found, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// in_reply_to.add("17320@venus.bell-tel.com");
    /// 
    /// assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
    /// assert!(in_reply_to.contains("17320@venus.bell-tel.com"));
    /// assert!(!in_reply_to.contains("12345@example.com"));
    /// ```
    pub fn contains(&self, call_id: &str) -> bool {
        self.0.iter().any(|id| id.0 == call_id)
    }
}

// Implement Deref to access Vec methods directly
impl Deref for InReplyTo {
    type Target = Vec<CallId>;
    
    /// Dereferences to the inner vector of CallId objects.
    ///
    /// This implementation allows using an InReplyTo header wherever a
    /// Vec<CallId> reference is expected, providing direct access to all
    /// vector methods like `len()`, `iter()`, indexing, etc.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    ///
    /// // Use vector methods directly
    /// assert_eq!(in_reply_to.len(), 1);
    /// assert_eq!(in_reply_to[0].as_str(), "70710@saturn.bell-tel.com");
    ///
    /// // Iterate through Call-IDs
    /// for call_id in in_reply_to.iter() {
    ///     println!("Referenced Call-ID: {}", call_id.as_str());
    /// }
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Implement Display for formatting in SIP messages
impl fmt::Display for InReplyTo {
    /// Formats the In-Reply-To header as a string.
    ///
    /// The format follows the SIP specification, with multiple
    /// Call-IDs separated by commas.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Single Call-ID
    /// let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// assert_eq!(in_reply_to.to_string(), "70710@saturn.bell-tel.com");
    ///
    /// // Multiple Call-IDs
    /// let mut in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// in_reply_to.add("17320@venus.bell-tel.com");
    /// assert_eq!(in_reply_to.to_string(), "70710@saturn.bell-tel.com, 17320@venus.bell-tel.com");
    ///
    /// // Using in a formatted string
    /// let header = format!("In-Reply-To: {}", in_reply_to);
    /// assert_eq!(header, "In-Reply-To: 70710@saturn.bell-tel.com, 17320@venus.bell-tel.com");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|id| id.0.clone()).collect::<Vec<String>>().join(", "))
    }
}

// Implement FromStr to enable parsing from string
impl FromStr for InReplyTo {
    type Err = Error;
    
    /// Parses a string into an In-Reply-To header.
    ///
    /// This method parses a comma-separated list of Call-IDs into an
    /// InReplyTo struct. Each Call-ID is trimmed of whitespace.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed InReplyTo, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns an error if the input string is empty or doesn't contain any valid Call-IDs.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a single Call-ID
    /// let in_reply_to = InReplyTo::from_str("70710@saturn.bell-tel.com").unwrap();
    /// assert_eq!(in_reply_to.len(), 1);
    ///
    /// // Parse multiple Call-IDs
    /// let in_reply_to = InReplyTo::from_str("70710@saturn.bell-tel.com, 17320@venus.bell-tel.com").unwrap();
    /// assert_eq!(in_reply_to.len(), 2);
    /// assert_eq!(in_reply_to[0].as_str(), "70710@saturn.bell-tel.com");
    /// assert_eq!(in_reply_to[1].as_str(), "17320@venus.bell-tel.com");
    ///
    /// // Whitespace handling
    /// let in_reply_to = InReplyTo::from_str("  70710@saturn.bell-tel.com  ,  17320@venus.bell-tel.com  ").unwrap();
    /// assert_eq!(in_reply_to.len(), 2);
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // We need to parse just the comma-separated CallIds, not the full header with prefix
        let ids: Vec<String> = s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        
        if ids.is_empty() {
            return Err(Error::ParseError("Empty In-Reply-To value".to_string()));
        }

        Ok(InReplyTo(ids.into_iter().map(CallId).collect()))
    }
}

// Implement TypedHeaderTrait for header manipulation
impl TypedHeaderTrait for InReplyTo {
    type Name = HeaderName;
    
    /// Returns the header name for In-Reply-To.
    ///
    /// # Returns
    ///
    /// The `HeaderName::InReplyTo` enum variant
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(InReplyTo::header_name(), HeaderName::InReplyTo);
    /// ```
    fn header_name() -> Self::Name {
        HeaderName::InReplyTo
    }
    
    /// Converts the In-Reply-To header to a generic Header.
    ///
    /// This method is used when constructing a SIP message to convert
    /// the typed InReplyTo into a generic Header structure.
    ///
    /// # Returns
    ///
    /// A generic `Header` with the correct name and value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let in_reply_to = InReplyTo::new("70710@saturn.bell-tel.com");
    /// let header = in_reply_to.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::InReplyTo);
    /// ```
    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(), 
            HeaderValue::Raw(self.to_string().into_bytes())
        )
    }
    
    /// Converts a generic Header to an In-Reply-To header.
    ///
    /// This method is used when parsing a SIP message to convert
    /// a generic Header into a typed InReplyTo struct.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed InReplyTo, or an error if conversion fails
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The header name is not InReplyTo
    /// - The header value cannot be converted to UTF-8
    /// - The header value cannot be parsed as a list of Call-IDs
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Header with In-Reply-To
    /// let header = Header::new(
    ///     HeaderName::InReplyTo,
    ///     HeaderValue::Raw("70710@saturn.bell-tel.com".as_bytes().to_vec())
    /// );
    ///
    /// // Convert to typed InReplyTo
    /// let in_reply_to = InReplyTo::from_header(&header).unwrap();
    /// assert_eq!(in_reply_to.len(), 1);
    /// assert_eq!(in_reply_to[0].as_str(), "70710@saturn.bell-tel.com");
    /// ```
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::InReplyTo {
            return Err(Error::ParseError(format!(
                "Expected In-Reply-To header, got {}", header.name
            )));
        }
        
        // Get the raw value
        let value_str = match &header.value {
            HeaderValue::Raw(bytes) => String::from_utf8(bytes.clone())?,
            HeaderValue::InReplyTo(in_reply_to) => in_reply_to.to_string(),
            _ => return Err(Error::ParseError(format!(
                "Expected In-Reply-To header value, got {:?}", header.value
            ))),
        };
        
        // Parse the value
        Self::from_str(&value_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_in_reply_to_new() {
        let irt = InReplyTo::new("abc123@example.com");
        assert_eq!(irt.0.len(), 1);
        assert_eq!(irt.0[0].0, "abc123@example.com");
    }
    
    #[test]
    fn test_in_reply_to_with_multiple() {
        let ids = vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ];
        let irt = InReplyTo::with_multiple(ids);
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_with_multiple_strings() {
        let strings = vec![
            "id1@domain.com".to_string(),
            "id2@domain.com".to_string(),
        ];
        let irt = InReplyTo::with_multiple_strings(strings);
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_add() {
        let mut irt = InReplyTo::new("id1@domain.com");
        irt.add("id2@domain.com");
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_contains() {
        let irt = InReplyTo::with_multiple(vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ]);
        assert!(irt.contains("id1@domain.com"));
        assert!(irt.contains("id2@domain.com"));
        assert!(!irt.contains("id3@domain.com"));
    }
    
    #[test]
    fn test_in_reply_to_display() {
        let irt = InReplyTo::with_multiple(vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ]);
        assert_eq!(irt.to_string(), "id1@domain.com, id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_from_str() {
        let irt_str = "id1@domain.com, id2@domain.com";
        let irt = InReplyTo::from_str(irt_str).unwrap();
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_header_trait() {
        // Test header_name
        assert_eq!(InReplyTo::header_name(), HeaderName::InReplyTo);
        
        // Test to_header
        let irt = InReplyTo::new("id@domain.com");
        let header = irt.to_header();
        assert_eq!(header.name, HeaderName::InReplyTo);
        
        // Test from_header
        let irt2 = InReplyTo::from_header(&header).unwrap();
        assert_eq!(irt, irt2);
    }
} 