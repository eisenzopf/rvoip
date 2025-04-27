//! # SIP Record-Route Header
//!
//! This module provides an implementation of the SIP Record-Route header as defined in
//! [RFC 3261 Section 20.30](https://datatracker.ietf.org/doc/html/rfc3261#section-20.30).
//!
//! The Record-Route header field is inserted by proxies in a request to force future
//! requests in the dialog to be routed through the proxy. Each proxy server that needs
//! to remain in the signaling path for subsequent requests adds its own address to this header.
//!
//! ## Purpose and Usage
//!
//! Record-Route is used to:
//! - Ensure that proxies remain in the signaling path for the entire dialog
//! - Maintain stateful processing for a dialog
//! - Implement advanced routing features
//! - Handle NAT traversal scenarios
//!
//! ## Dialog Route Construction
//!
//! When a user agent receives a request with Record-Route headers, it:
//! 1. Stores the Record-Route header field values in order
//! 2. Uses them to construct Route headers for subsequent requests in the dialog
//! 3. For responses, uses them in reverse order
//!
//! ## Format
//!
//! ```
//! Record-Route: <sip:p1.example.com;lr>
//! Record-Route: <sip:p2.domain.com;lr>,<sip:p3.example.net;lr>
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Parse a Record-Route header
//! let record_route = RecordRoute::from_str("<sip:proxy1.example.com;lr>,<sip:proxy2.example.net;lr>").unwrap();
//!
//! // Access entries
//! assert_eq!(record_route.len(), 2);
//! assert!(record_route[0].uri.to_string().contains("proxy1"));
//!
//! // Create a Record-Route header
//! let mut entries = Vec::new();
//! // Entries would be created and added here
//! let record_route = RecordRoute::new(entries);
//! ```

use crate::parser::headers::parse_record_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::record_route::RecordRouteEntry;
use serde::{Deserialize, Serialize};

/// Typed Record-Route header.
///
/// The Record-Route header is used by SIP proxies to remain in the signaling path
/// for all subsequent requests within a dialog. Each proxy that wishes to stay
/// in the path adds a Record-Route entry containing its address.
///
/// This struct wraps a vector of `RecordRouteEntry` objects, each representing
/// a single routing entry. The entries are stored in the order they appear in the
/// SIP message (from top to bottom).
///
/// When used for route construction:
/// - For requests in a dialog, entries are used in the order they appear
/// - For responses, entries are used in reverse order
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Parse a Record-Route header with multiple entries
/// let header = "<sip:proxy1.example.com;lr>,<sip:proxy2.example.net;lr>";
/// let record_route = RecordRoute::from_str(header).unwrap();
///
/// // Iterate through entries
/// for entry in &record_route {
///     println!("Proxy: {}", entry.uri);
///     // Check for loose routing parameter
///     if entry.has_param("lr") {
///         println!("Using loose routing");
///     }
/// }
///
/// // Convert back to string
/// let header_str = record_route.to_string();
/// assert!(header_str.contains("proxy1"));
/// assert!(header_str.contains("proxy2"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRoute(pub Vec<RecordRouteEntry>);

impl RecordRoute {
    /// Creates a new RecordRoute header.
    ///
    /// This constructor initializes a Record-Route header with a list of entries.
    /// Each entry typically contains a SIP URI with an 'lr' parameter to indicate
    /// loose routing.
    ///
    /// # Parameters
    ///
    /// - `list`: A vector of `RecordRouteEntry` objects representing proxies in the routing path
    ///
    /// # Returns
    ///
    /// A new `RecordRoute` instance containing the specified entries
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create RecordRouteEntry objects (typically by parsing)
    /// let entry1 = RecordRouteEntry::from_str("<sip:proxy1.example.com;lr>").unwrap();
    /// let entry2 = RecordRouteEntry::from_str("<sip:proxy2.example.net;lr>").unwrap();
    ///
    /// // Create a new Record-Route header
    /// let entries = vec![entry1, entry2];
    /// let record_route = RecordRoute::new(entries);
    ///
    /// // Verify the entries
    /// assert_eq!(record_route.len(), 2);
    /// ```
    pub fn new(list: Vec<RecordRouteEntry>) -> Self {
        Self(list)
    }
}

impl fmt::Display for RecordRoute {
    /// Formats the Record-Route header as a string.
    ///
    /// This method serializes the Record-Route header into its canonical
    /// string representation according to RFC 3261. Multiple entries are
    /// separated by commas.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let record_route = RecordRoute::from_str("<sip:proxy1.example.com;lr>,<sip:proxy2.example.net;lr>").unwrap();
    ///
    /// // Convert to string
    /// let header_str = record_route.to_string();
    /// assert!(header_str.contains("proxy1.example.com"));
    /// assert!(header_str.contains("proxy2.example.net"));
    ///
    /// // Use in a formatted SIP message
    /// let formatted = format!("Record-Route: {}", record_route);
    /// assert!(formatted.starts_with("Record-Route: <sip:"));
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|rr| rr.to_string()).collect::<Vec<String>>().join(", "))
    }
}

impl FromStr for RecordRoute {
    type Err = Error;

    /// Parses a string into a RecordRoute header.
    ///
    /// This method parses a string containing one or more Record-Route entries
    /// separated by commas. It uses the nom parser for Record-Route headers
    /// defined in the crate's parser module.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed RecordRoute, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns a ParseError if the input string cannot be parsed as a valid
    /// Record-Route header value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a single entry
    /// let single = RecordRoute::from_str("<sip:proxy.example.com;lr>").unwrap();
    /// assert_eq!(single.len(), 1);
    ///
    /// // Parse multiple entries
    /// let multiple = RecordRoute::from_str("<sip:p1.example.com;lr>, <sip:p2.example.net;lr>").unwrap();
    /// assert_eq!(multiple.len(), 2);
    ///
    /// // Parsing error
    /// let result = RecordRoute::from_str("invalid<value");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        match all_consuming(parse_record_route)(s.as_bytes()) {
            Ok((_, rr_header)) => Ok(rr_header),
            Err(e) => Err(Error::ParseError( 
                format!("Failed to parse Record-Route header: {:?}", e)
            ))
        }
    }
}

impl Deref for RecordRoute {
    type Target = Vec<RecordRouteEntry>;

    /// Dereferences to the inner vector of RecordRouteEntry objects.
    ///
    /// This implementation allows using a RecordRoute header wherever a
    /// Vec<RecordRouteEntry> reference is expected, providing direct access
    /// to all vector methods like `len()`, `iter()`, indexing, etc.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let record_route = RecordRoute::from_str("<sip:p1.example.com;lr>,<sip:p2.example.net;lr>").unwrap();
    ///
    /// // Use vector methods directly
    /// assert_eq!(record_route.len(), 2);
    /// assert!(!record_route.is_empty());
    ///
    /// // Access by index
    /// let first_entry = &record_route[0];
    /// assert!(first_entry.uri.to_string().contains("p1.example.com"));
    ///
    /// // Iterate through entries
    /// for entry in record_route.iter() {
    ///     assert!(entry.uri.to_string().contains("example"));
    /// }
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 