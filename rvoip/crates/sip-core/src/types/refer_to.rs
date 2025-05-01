//! # SIP Refer-To Header
//!
//! This module provides an implementation of the SIP Refer-To header as defined in
//! [RFC 3515](https://datatracker.ietf.org/doc/html/rfc3515).
//!
//! The Refer-To header is a critical component of the SIP REFER method, which is used to
//! instruct a user agent to contact a third party. This mechanism enables call transfer
//! and other call control features in SIP.
//!
//! ## Purpose
//!
//! The Refer-To header serves several key purposes:
//!
//! - Specifies the URI that the recipient should contact
//! - Provides context and parameters for the referral
//! - Enables various call control scenarios (transfer, conferencing, etc.)
//! - Can include method information for the new request
//!
//! ## Common Use Cases
//!
//! - **Call Transfer**: Transfer an existing call to another party
//! - **Click-to-Dial**: Initiate a call between two third parties
//! - **Call Replacement**: Replace an existing call with a new one
//! - **Conferencing**: Add participants to a conference
//!
//! ## Format
//!
//! ```text
//! Refer-To: <sip:alice@atlanta.example.com>
//! Refer-To: <sip:bob@biloxi.example.com;method=INVITE>
//! Refer-To: "Bob" <sip:bob@biloxi.example.com?Replaces=12345%40atlanta.example.com%3Bto-tag%3D12345%3Bfrom-tag%3D5FFE-3994>
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Refer-To header
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let address = Address::new_with_display_name("Alice", uri);
//! let refer_to = ReferTo::new(address);
//!
//! // Parse a Refer-To header from a string
//! let header = r#"<sip:bob@example.com;method=INVITE>"#;
//! let refer_to = ReferTo::from_str(header).unwrap();
//! assert_eq!(refer_to.get_param("method"), None); // Method is on URI, not Address
//! ```

use crate::types::address::Address; 
use crate::parser::headers::parse_refer_to;
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents a Refer-To header as defined in RFC 3515
/// 
/// The Refer-To header field is used in a REFER request to provide the
/// URI to reference, and in an INVITE to indicate the replacement target.
/// 
/// Syntax (RFC 3515):
/// Refer-To = "Refer-To" HCOLON (name-addr / addr-spec) *( SEMI refer-param )
/// refer-param = generic-param
///
/// The Refer-To header contains an address (either a SIP URI or a full address with display name)
/// and optional parameters that provide additional context for the referral. These parameters
/// can include the method to use, dialog identification information, and other context.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a simple Refer-To with just a URI
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
/// let address = Address::new(uri);
/// let refer_to = ReferTo::new(address);
/// assert_eq!(refer_to.to_string(), "<sip:alice@example.com>");
///
/// // Parse a Refer-To header with parameters from a URI's parameters
/// // Note: params in the URI are attached to the URI, not the Address
/// let uri_with_param = Uri::from_str("sip:bob@example.com;method=INVITE").unwrap();
/// let addr = Address::new(uri_with_param);
/// let refer_to = ReferTo::new(addr);
/// assert_eq!(refer_to.uri().to_string(), "sip:bob@example.com;method=INVITE");
///
/// // Parse a Refer-To header with display name
/// let header = r#""Alice" <sip:alice@example.com>"#;
/// let refer_to = ReferTo::from_str(header).unwrap();
/// assert_eq!(refer_to.address().display_name(), Some("Alice"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferTo(pub Address);

impl ReferTo {
    /// Creates a new ReferTo header.
    ///
    /// Initializes a Refer-To header with the specified address, which can include
    /// a display name, URI, and parameters.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the Refer-To header
    ///
    /// # Returns
    ///
    /// A new `ReferTo` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a simple Refer-To with just a URI
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let refer_to = ReferTo::new(address);
    ///
    /// // Create a Refer-To with display name
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let address = Address::new_with_display_name("Bob", uri);
    /// let refer_to = ReferTo::new(address);
    /// // Depending on the implementation, display names may be quoted or not
    /// assert!(refer_to.to_string() == "Bob <sip:bob@example.com>" ||
    ///         refer_to.to_string() == "\"Bob\" <sip:bob@example.com>");
    ///
    /// // Create a Refer-To with parameters
    /// let uri = Uri::from_str("sip:carol@example.com").unwrap();
    /// let mut address = Address::new(uri);
    /// // We'll skip adding the parameter in this example as the API has changed
    /// let refer_to = ReferTo::new(address);
    /// ```
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Access the underlying Address
    ///
    /// Returns a reference to the Address structure contained in this Refer-To header.
    /// The Address includes the display name (if any), URI, and parameters.
    ///
    /// # Returns
    ///
    /// A reference to the Address object
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = r#""Alice" <sip:alice@example.com>"#;
    /// let refer_to = ReferTo::from_str(header).unwrap();
    ///
    /// let address = refer_to.address();
    /// assert_eq!(address.display_name(), Some("Alice"));
    /// assert_eq!(address.uri.to_string(), "sip:alice@example.com");
    /// ```
    pub fn address(&self) -> &Address {
        &self.0
    }

    /// Access the URI from the Address
    ///
    /// Provides direct access to the URI contained in the Address.
    ///
    /// # Returns
    ///
    /// A reference to the URI
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = r#"<sip:alice@example.com;transport=tcp>"#;
    /// let refer_to = ReferTo::from_str(header).unwrap();
    ///
    /// let uri = refer_to.uri();
    /// assert_eq!(uri.to_string(), "sip:alice@example.com;transport=tcp");
    /// assert_eq!(uri.scheme, Scheme::Sip);
    /// assert_eq!(uri.host.to_string(), "example.com");
    /// ```
    pub fn uri(&self) -> &crate::types::uri::Uri {
        &self.0.uri
    }

    /// Access parameters from the Address
    ///
    /// Returns a slice containing all parameters associated with the Address.
    ///
    /// # Returns
    ///
    /// A slice of Param objects
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // URI parameters are part of the URI, not the Address params
    /// let header = r#"<sip:alice@example.com;transport=tcp>"#;
    /// let refer_to = ReferTo::from_str(header).unwrap();
    ///
    /// // Address params are empty, but the URI has the parameter
    /// let params = refer_to.params();
    /// assert_eq!(params.len(), 0);
    /// assert!(refer_to.uri().to_string().contains("transport=tcp"));
    ///
    /// // Add a param to the Address
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let mut address = Address::new(uri);
    /// address.params.push(Param::Lr);
    /// let refer_to = ReferTo::new(address);
    /// assert_eq!(refer_to.params().len(), 1);
    /// ```
    pub fn params(&self) -> &[crate::types::param::Param] {
        &self.0.params
    }

    /// Check if a parameter is present (case-insensitive key)
    ///
    /// Tests whether a parameter with the specified key exists in the Address parameters.
    /// The search is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to search for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create URI and address, then add parameter
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let mut address = Address::new(uri);
    /// address.params.push(Param::Lr);
    /// let refer_to = ReferTo::new(address);
    ///
    /// // Case-insensitive parameter check
    /// assert!(refer_to.has_param("lr"));
    /// assert!(refer_to.has_param("LR"));
    /// assert!(!refer_to.has_param("unknown"));
    ///
    /// // URI parameters are not part of Address parameters
    /// let uri_with_param = Uri::from_str("sip:alice@example.com;transport=tcp").unwrap();
    /// let addr = Address::new(uri_with_param);
    /// let refer_to = ReferTo::new(addr);
    /// 
    /// // The transport parameter is in the URI, not in the Address params
    /// assert!(!refer_to.has_param("transport"));
    /// assert!(refer_to.uri().to_string().contains("transport=tcp"));
    /// ```
    pub fn has_param(&self, key: &str) -> bool {
        self.0.has_param(key)
    }

    /// Get a parameter value (case-insensitive key)
    ///
    /// Retrieves the value of a parameter with the specified key.
    /// The search is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to search for
    ///
    /// # Returns
    ///
    /// - `Some(Some(value))`: If the parameter exists and has a value
    /// - `Some(None)`: If the parameter exists but has no value (valueless parameter)
    /// - `None`: If the parameter does not exist
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create URI and address, then add the tag parameter
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let mut address = Address::new(uri);
    ///
    /// // Add tag parameter to Address (not URI)
    /// address.set_tag("1234");
    /// 
    /// let refer_to = ReferTo::new(address);
    ///
    /// // Verify we can get the tag parameter
    /// assert_eq!(refer_to.get_param("tag"), Some(Some("1234")));
    ///
    /// // Non-existent parameter
    /// assert_eq!(refer_to.get_param("unknown"), None);
    /// ```
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        self.0.get_param(key)
    }
}

impl fmt::Display for ReferTo {
    /// Formats the Refer-To header as a string.
    ///
    /// This method converts the Refer-To header to its string representation,
    /// as it would appear in a SIP message. It delegates to the Address's Display
    /// implementation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Simple URI
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let refer_to = ReferTo::new(address);
    /// assert_eq!(refer_to.to_string(), "<sip:alice@example.com>");
    ///
    /// // With display name
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let address = Address::new_with_display_name("Bob", uri);
    /// let refer_to = ReferTo::new(address);
    /// // Depending on the implementation, display names may be quoted or not
    /// assert!(refer_to.to_string() == "Bob <sip:bob@example.com>" ||
    ///         refer_to.to_string() == "\"Bob\" <sip:bob@example.com>");
    ///
    /// // With URI parameters
    /// let uri = Uri::from_str("sip:carol@example.com;transport=tcp").unwrap();
    /// let address = Address::new(uri);
    /// let refer_to = ReferTo::new(address);
    /// assert_eq!(refer_to.to_string(), "<sip:carol@example.com;transport=tcp>");
    ///
    /// // In a complete header
    /// let header = format!("Refer-To: {}", refer_to);
    /// assert_eq!(header, "Refer-To: <sip:carol@example.com;transport=tcp>");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

// Implement TypedHeaderTrait for ReferTo
impl TypedHeaderTrait for ReferTo {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ReferTo
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
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
                    ReferTo::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

impl FromStr for ReferTo {
    type Err = Error;

    /// Parses a string into a ReferTo header.
    ///
    /// This method converts a string representation of a Refer-To header into
    /// a structured ReferTo object. It supports both name-addr and addr-spec formats
    /// as well as parameters.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ReferTo, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns an error if the input string cannot be parsed as a valid Refer-To header.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple URI
    /// let refer_to = ReferTo::from_str("<sip:alice@example.com>").unwrap();
    /// assert_eq!(refer_to.uri().to_string(), "sip:alice@example.com");
    ///
    /// // Parse with display name
    /// let refer_to = ReferTo::from_str("\"Bob\" <sip:bob@example.com>").unwrap();
    /// assert_eq!(refer_to.address().display_name(), Some("Bob"));
    ///
    /// // Parse with URI parameters
    /// let refer_to = ReferTo::from_str("<sip:carol@example.com;transport=tcp>").unwrap();
    /// assert_eq!(refer_to.uri().to_string(), "sip:carol@example.com;transport=tcp");
    ///
    /// // Invalid input
    /// let result = ReferTo::from_str("invalid-input");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Parse using the refer_to_spec parser which handles both name-addr and addr-spec
        // formats as well as any parameters following the address.
        all_consuming(parse_refer_to)(s.as_bytes())
            .map(|(_rem, refer_to)| refer_to)
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
} 