//! # SIP To Header
//!
//! This module provides an implementation of the SIP To header as defined in
//! [RFC 3261 Section 8.1.1.2](https://datatracker.ietf.org/doc/html/rfc3261#section-8.1.1.2).
//!
//! The To header field specifies the logical recipient of the request, or the
//! address-of-record of the user or resource that is the target of this request.
//! This may or may not be the ultimate recipient of the request.
//!
//! The To header field is a critical part of dialog identification and contains
//! a tag parameter that helps uniquely identify dialogs. A server reflects the
//! To header field in responses, and for initial requests, it adds a new tag parameter.
//!
//! ## Format
//!
//! ```
//! To: "Bob" <sip:bob@biloxi.com>;tag=a6c85cf
//! t: "Bob" <sip:bob@biloxi.com>;tag=a6c85cf
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a To header with a URI
//! let uri = Uri::from_str("sip:bob@example.com").unwrap();
//! let address = Address::new(uri);
//! let to = To::new(address);
//!
//! // Parse a To header from a string
//! let to = To::from_str("\"Bob\" <sip:bob@example.com>;tag=1928301774").unwrap();
//! assert_eq!(to.tag(), Some("1928301774"));
//! ```

use crate::types::{HeaderName, HeaderValue, Param, TypedHeader};
use crate::types::address::Address;
use std::fmt;
use std::str::FromStr;
use crate::error::{Error, Result};
use crate::parser::parse_address; // For FromStr
use std::ops::Deref;
use serde::{Serialize, Deserialize};
use nom::combinator;

/// Represents the To header field (RFC 3261 Section 8.1.1.3).
/// Contains the logical recipient of the request.
///
/// The To header field specifies the intended recipient of the request, often called
/// the "logical recipient." It can contain a SIP or SIPS URI and optionally a display
/// name. The To header may also contain parameters, with the "tag" parameter being
/// particularly important for dialog identification.
///
/// The To header's format is identical to that of the `From` header, but they serve
/// different purposes. The To header identifies the target of the request, while
/// the From header identifies the originator.
///
/// In dialog-based communications:
/// - For outgoing requests, the To header contains the remote party's address
/// - For responses, the To header is copied from the request
/// - After the initial dialog setup, the To header will include a tag parameter
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a To header with display name and URI
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
/// let mut address = Address::new(uri);
/// address.set_display_name("Alice Smith");
/// let to = To::new(address);
///
/// // Add a tag parameter for dialog identification
/// let mut to = To::from_str("<sip:bob@example.com>").unwrap();
/// to.set_tag("a7c85cf");
/// assert_eq!(to.tag(), Some("a7c85cf"));
///
/// // Convert to string for a SIP message
/// assert_eq!(to.to_string(), "<sip:bob@example.com>;tag=a7c85cf");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct To(pub Address);

impl To {
    /// Creates a new To header.
    ///
    /// Initializes a new To header using the provided Address, which contains
    /// both the URI and any associated display name or parameters.
    ///
    /// # Parameters
    ///
    /// - `address`: An Address instance containing the URI and optional display name
    ///
    /// # Returns
    ///
    /// A new `To` instance wrapping the provided address
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Simple To header with just a URI
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let to = To::new(address);
    ///
    /// // To header with display name
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let mut address = Address::new(uri);
    /// address.set_display_name("Alice Smith");
    /// let to = To::new(address);
    /// ```
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Gets the tag parameter value.
    ///
    /// Retrieves the "tag" parameter from the To header, which is used for
    /// dialog identification in SIP sessions. For requests within a dialog,
    /// this tag should be present. For initial requests, it will typically
    /// be absent, and the server adds it in the response.
    ///
    /// # Returns
    ///
    /// `Some(tag)` if the tag parameter is present, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // To header without a tag (typical in initial requests)
    /// let to = To::from_str("<sip:bob@example.com>").unwrap();
    /// assert_eq!(to.tag(), None);
    ///
    /// // To header with a tag (typical in responses or in-dialog requests)
    /// let to = To::from_str("<sip:bob@example.com>;tag=1928301774").unwrap();
    /// assert_eq!(to.tag(), Some("1928301774"));
    /// ```
    pub fn tag(&self) -> Option<&str> {
        self.0.tag()
    }

    /// Sets or replaces the tag parameter.
    ///
    /// Sets the "tag" parameter in the To header. This is typically used by servers
    /// when generating responses to initial requests, to establish dialog identification.
    /// It may also be used when creating requests within an existing dialog.
    ///
    /// The tag value should be globally unique and cryptographically random to
    /// ensure dialog identification security.
    ///
    /// # Parameters
    ///
    /// - `tag`: The tag value to set, can be any type that can be converted into a String
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a To header and add a tag
    /// let mut to = To::from_str("<sip:bob@example.com>").unwrap();
    /// assert_eq!(to.tag(), None);
    ///
    /// // Add a tag (as a server might do when responding)
    /// to.set_tag("a6c85cf");
    /// assert_eq!(to.tag(), Some("a6c85cf"));
    ///
    /// // Replace an existing tag
    /// to.set_tag("1928301774");
    /// assert_eq!(to.tag(), Some("1928301774"));
    /// ```
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.0.set_tag(tag)
    }
}

// Delegate Display and FromStr to Address
impl fmt::Display for To {
    /// Formats the To header as a string.
    ///
    /// Converts the To header to its string representation suitable for inclusion
    /// in a SIP message. The format follows the SIP specifications, with 
    /// display-name and parameters appropriately formatted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    /// use std::str::FromStr;
    ///
    /// // Basic To header
    /// let to = To::from_str("<sip:bob@example.com>").unwrap();
    /// assert_eq!(to.to_string(), "<sip:bob@example.com>");
    ///
    /// // To header with display name
    /// let to = To::from_str("\"Bob Smith\" <sip:bob@example.com>").unwrap();
    /// assert_eq!(to.to_string(), "\"Bob Smith\" <sip:bob@example.com>");
    ///
    /// // To header with tag parameter
    /// let to = To::from_str("<sip:bob@example.com>;tag=1928301774").unwrap();
    /// assert_eq!(to.to_string(), "<sip:bob@example.com>;tag=1928301774");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for To {
    type Err = crate::error::Error;

    /// Parses a string into a To header.
    ///
    /// Converts a string representation of a To header into a To object.
    /// The string should be in the format defined by RFC 3261, which includes
    /// an optional display-name, a URI, and optional parameters.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse as a To header
    ///
    /// # Returns
    ///
    /// A Result containing the parsed To header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple To header
    /// let to = To::from_str("<sip:bob@example.com>").unwrap();
    /// assert_eq!(to.uri().host(), "example.com");
    ///
    /// // Parse with display name
    /// let to = To::from_str("\"Bob Smith\" <sip:bob@example.com>").unwrap();
    /// assert_eq!(to.display_name(), Some("Bob Smith"));
    ///
    /// // Parse with tag parameter
    /// let to = To::from_str("<sip:bob@example.com>;tag=1928301774").unwrap();
    /// assert_eq!(to.tag(), Some("1928301774"));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming, handle input type, map result and error
        nom::combinator::all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| To(addr))
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

// Optionally implement Deref to access all Address methods directly
impl Deref for To {
    type Target = Address;

    /// Provides transparent access to the underlying Address.
    ///
    /// By implementing Deref, all methods available on the Address struct
    /// can be called directly on a To instance, without having to explicitly
    /// access the inner Address.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let to = To::from_str("\"Bob\" <sip:bob@example.com>;tag=1928301774").unwrap();
    ///
    /// // Can directly call Address methods on a To instance
    /// assert_eq!(to.display_name(), Some("Bob"));
    /// assert_eq!(to.uri().host(), "example.com");
    /// assert!(to.has_param("tag"));
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ... Display/FromStr impls ... 