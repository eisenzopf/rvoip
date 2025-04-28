//! # SIP From Header
//!
//! This module provides an implementation of the SIP From header as defined in
//! [RFC 3261 Section 20.20](https://datatracker.ietf.org/doc/html/rfc3261#section-20.20).
//!
//! The From header field indicates the logical identity of the initiator of the request.
//! It is one of the most important headers in a SIP message, serving several crucial roles:
//!
//! - Identifying the originator of a request
//! - Providing the Address-of-Record (AOR) of the initiator
//! - Containing the dialog identifier tag parameter
//! - Enabling call filtering and routing decisions
//!
//! ## Format
//!
//! The From header contains a SIP URI and optional parameters, most importantly the tag:
//!
//! ```text
//! From: "Alice Smith" <sip:alice@example.com>;tag=1928301774
//! From: sip:bob@example.org;tag=a7c6d8
//! ```
//!
//! ## Dialog Identification
//!
//! The combination of the From tag, To tag, and Call-ID forms the dialog ID,
//! which uniquely identifies a dialog between two user agents.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a From header from an Address with a display name
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let address = Address::new_with_display_name("Alice Smith", uri);
//! let from = From::new(address);
//!
//! // Display the From header
//! assert_eq!(from.to_string(), "\"Alice Smith\" <sip:alice@example.com>");
//!
//! // Parse a From header from a string
//! let from = From::from_str("\"Bob\" <sip:bob@example.org>").unwrap();
//! assert_eq!(from.address().display_name(), Some("Bob"));
//! assert_eq!(from.uri.to_string(), "sip:bob@example.org");
//! ```

use crate::types::{HeaderName, HeaderValue, Param, TypedHeader};
use crate::types::address::Address;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use crate::error::{Error, Result};
use crate::parser::parse_address; // For FromStr
use std::ops::Deref;
use nom::combinator;

/// Represents the From header field (RFC 3261 Section 8.1.1.3).
/// Contains the logical identity of the initiator of the request.
///
/// The From header is a critical component of SIP messages that identifies the 
/// initiator of a request. It consists of an Address, which may include a display name, 
/// a SIP URI, and parameters, most importantly the 'tag' parameter.
///
/// The 'tag' parameter, combined with the To tag and Call-ID, forms a dialog ID
/// that uniquely identifies a dialog between two user agents.
///
/// This implementation wraps an `Address` and implements `Deref` to provide direct
/// access to all Address methods.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a From header with a display name
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
/// let address = Address::new_with_display_name("Alice Smith", uri);
/// let from = From::new(address);
///
/// // Display the From header
/// assert_eq!(from.to_string(), "\"Alice Smith\" <sip:alice@example.com>");
///
/// // Parse a From header from a string
/// let from = From::from_str("\"Bob\" <sip:bob@example.org>").unwrap();
/// assert_eq!(from.address().display_name(), Some("Bob"));
/// assert_eq!(from.uri.to_string(), "sip:bob@example.org");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct From(pub Address);

impl From {
    /// Creates a new From header.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the From header
    ///
    /// # Returns
    ///
    /// A new `From` instance with the specified address
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a From header with just a URI
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let from = From::new(address);
    /// assert_eq!(from.to_string(), "<sip:alice@example.com>");
    ///
    /// // Create a From header with a display name and URI
    /// let uri = Uri::from_str("sip:bob@example.org").unwrap();
    /// let address = Address::new_with_display_name("Bob", uri);
    /// let from = From::new(address);
    /// assert_eq!(from.to_string(), "Bob <sip:bob@example.org>");
    /// ```
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Returns a reference to the inner Address.
    ///
    /// This method provides access to the wrapped Address instance
    /// for cases where you need to work with it directly.
    ///
    /// # Returns
    ///
    /// A reference to the inner Address
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a From header
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new_with_display_name("Alice", uri);
    /// let from = From::new(address);
    ///
    /// // Access the inner Address
    /// assert_eq!(from.address().display_name(), Some("Alice"));
    /// ```
    pub fn address(&self) -> &Address {
        &self.0
    }

    /// Gets the tag parameter value.
    ///
    /// The tag parameter is critical for dialog identification in SIP.
    /// Combined with the To tag and Call-ID, it forms a unique dialog ID.
    ///
    /// # Returns
    ///
    /// An Option containing the tag as a string slice if present, or None if not
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a From header with a tag
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut address = Address::new(uri);
    /// address.set_tag("1928301774");
    /// let from = From::new(address);
    ///
    /// // Get the tag
    /// assert_eq!(from.tag(), Some("1928301774"));
    ///
    /// // From header without a tag
    /// let uri = Uri::from_str("sip:bob@example.org").unwrap();
    /// let address = Address::new(uri);
    /// let from = From::new(address);
    /// assert_eq!(from.tag(), None);
    /// ```
    pub fn tag(&self) -> Option<&str> {
        self.0.tag()
    }

    /// Sets or replaces the tag parameter.
    ///
    /// In a SIP dialog, the From tag is generated by the request initiator
    /// and remains the same for all messages in that dialog.
    ///
    /// # Parameters
    ///
    /// - `tag`: The tag value to set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a From header without a tag
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let mut from = From::new(address);
    ///
    /// // Add a tag
    /// from.set_tag("1928301774");
    /// assert_eq!(from.tag(), Some("1928301774"));
    ///
    /// // Update the tag
    /// from.set_tag("new-tag-value");
    /// assert_eq!(from.tag(), Some("new-tag-value"));
    /// ```
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.0.set_tag(tag)
    }
}

// Delegate Display and FromStr to Address
impl fmt::Display for From {
    /// Formats the From header as a string.
    ///
    /// The format follows the SIP specification, including display name if present,
    /// the URI, and any parameters, particularly the tag.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // From header with just a URI
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let from = From::new(address);
    /// assert_eq!(from.to_string(), "<sip:alice@example.com>");
    ///
    /// // From header with display name, URI, and tag
    /// let uri = Uri::from_str("sip:bob@example.org").unwrap();
    /// let mut address = Address::new_with_display_name("Bob", uri);
    /// address.set_tag("abc123");
    /// let from = From::new(address);
    /// assert_eq!(from.to_string(), "Bob <sip:bob@example.org>;tag=abc123");
    ///
    /// // Using in a formatted string
    /// let header = format!("From: {}", from);
    /// assert_eq!(header, "From: Bob <sip:bob@example.org>;tag=abc123");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for From {
    type Err = crate::error::Error;

    /// Parses a string into a From header.
    ///
    /// This method can parse standard SIP From header values with display names,
    /// URIs, and parameters, particularly the tag parameter.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed From header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple From header
    /// let from = From::from_str("<sip:alice@example.com>").unwrap();
    /// assert_eq!(from.uri.to_string(), "sip:alice@example.com");
    /// assert_eq!(from.address().display_name(), None);
    ///
    /// // Parse with display name
    /// let from = From::from_str("\"Bob\" <sip:bob@example.org>").unwrap();
    /// assert_eq!(from.address().display_name(), Some("Bob"));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming, handle input type, map result and error
        nom::combinator::all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| From(addr))
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

// Optionally implement Deref to access all Address methods directly
impl Deref for From {
    type Target = Address;

    /// Dereferences to the inner Address.
    ///
    /// This implementation allows using a From header wherever an Address
    /// is expected, providing direct access to all Address methods.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a From header
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new_with_display_name("Alice", uri);
    /// let from = From::new(address);
    ///
    /// // Access Address methods directly through the deref implementation
    /// assert_eq!(from.uri.to_string(), "sip:alice@example.com");
    ///
    /// // Use in contexts expecting an Address reference
    /// fn takes_address(addr: &Address) -> String {
    ///     addr.uri.to_string()
    /// }
    /// assert_eq!(takes_address(&from), "sip:alice@example.com");
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ... Display/FromStr impls ... 
