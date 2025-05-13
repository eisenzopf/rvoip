//! # SIP Referred-By Header
//!
//! This module provides an implementation of the SIP Referred-By header as defined in
//! [RFC 3892](https://datatracker.ietf.org/doc/html/rfc3892).
//!
//! The Referred-By header identifies the entity that requested the current referral.
//! It provides a way to verify the identity of the party that initiated a referral.
//!
//! ## Purpose
//!
//! The Referred-By header serves several key purposes:
//!
//! - Identifies the referring party
//! - Enables recipient verification of the referring party's identity
//! - Provides context for why the referral is occurring
//! - Enables authentication of referrals via the cid parameter
//!
//! ## Common Use Cases
//!
//! - **Call Transfer**: Identifies who transferred a call
//! - **Call Authorization**: Provides context for authorizing a referral
//! - **Auditing**: Records who initiated important call control events
//! - **Authenticated Referrals**: Provides a mechanism to validate referrals
//!
//! ## Format
//!
//! ```text
//! Referred-By: <sip:alice@atlanta.example.com>
//! Referred-By: <sip:bob@biloxi.example.com>;cid=12345@biloxi.example.com
//! Referred-By: "Alice" <sip:alice@atlanta.example.com>;purpose=transfer
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Referred-By header
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let address = Address::new_with_display_name("Alice", uri);
//! let referred_by = ReferredBy::new(address);
//!
//! // Parse a Referred-By header from a string
//! let header = r#"<sip:bob@example.com;transport=tcp>"#;
//! let referred_by = ReferredBy::from_str(header).unwrap();
//! assert_eq!(referred_by.uri().scheme.to_string(), "sip");
//! ```

use crate::types::address::Address; 
use crate::parser::headers::parse_referred_by;
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents a Referred-By header as defined in RFC 3892
/// 
/// The Referred-By header field identifies the entity that requested the current 
/// referral. It enables verification of the referrer's identity and provides 
/// context for the referral.
/// 
/// Syntax (RFC 3892):
/// Referred-By = "Referred-By" HCOLON (name-addr / addr-spec) *( SEMI referredby-param )
/// referredby-param = generic-param / "cid" EQUAL token
///
/// The Referred-By header contains an address (either a SIP URI or a full address with display name)
/// and optional parameters that provide additional context or authentication for the referral.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a simple Referred-By with just a URI
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
/// let address = Address::new(uri);
/// let referred_by = ReferredBy::new(address);
/// assert_eq!(referred_by.to_string(), "<sip:alice@example.com>");
///
/// // Parse a Referred-By header with display name
/// let header = r#""Alice" <sip:alice@example.com>"#;
/// let referred_by = ReferredBy::from_str(header).unwrap();
/// assert_eq!(referred_by.address().display_name(), Some("Alice"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferredBy(pub Address);

impl ReferredBy {
    /// Creates a new ReferredBy header.
    ///
    /// Initializes a Referred-By header with the specified address, which can include
    /// a display name, URI, and parameters.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the Referred-By header
    ///
    /// # Returns
    ///
    /// A new `ReferredBy` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a simple Referred-By with just a URI
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let referred_by = ReferredBy::new(address);
    ///
    /// // Create a Referred-By with display name
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let address = Address::new_with_display_name("Bob", uri);
    /// let referred_by = ReferredBy::new(address);
    /// // Depending on the implementation, display names may be quoted or not
    /// assert!(referred_by.to_string() == "Bob <sip:bob@example.com>" ||
    ///         referred_by.to_string() == "\"Bob\" <sip:bob@example.com>");
    /// ```
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Access the underlying Address
    ///
    /// Returns a reference to the Address structure contained in this Referred-By header.
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
    /// let referred_by = ReferredBy::from_str(header).unwrap();
    ///
    /// let address = referred_by.address();
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
    /// let referred_by = ReferredBy::from_str(header).unwrap();
    ///
    /// let uri = referred_by.uri();
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
    /// let referred_by = ReferredBy::from_str(header).unwrap();
    ///
    /// // Address params are empty, but the URI has the parameter
    /// let params = referred_by.params();
    /// assert_eq!(params.len(), 0);
    /// assert!(referred_by.uri().to_string().contains("transport=tcp"));
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
    /// let referred_by = ReferredBy::new(address);
    ///
    /// // Case-insensitive parameter check
    /// assert!(referred_by.has_param("lr"));
    /// assert!(referred_by.has_param("LR"));
    /// assert!(!referred_by.has_param("unknown"));
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
    /// // Create a Referred-By header with a cid parameter
    /// let header = r#"<sip:alice@example.com>;cid=12345@example.com"#;
    /// let referred_by = ReferredBy::from_str(header).unwrap();
    ///
    /// // Verify we can get the cid parameter
    /// assert_eq!(referred_by.get_param("cid"), Some(Some("12345@example.com")));
    ///
    /// // Non-existent parameter
    /// assert_eq!(referred_by.get_param("unknown"), None);
    /// ```
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        self.0.get_param(key)
    }

    /// Get the Content-ID (cid) parameter value
    ///
    /// Retrieves the value of the "cid" parameter if present. The cid parameter
    /// links to an S/MIME body part containing a signature that can be used to
    /// verify the identity of the referring party.
    ///
    /// # Returns
    ///
    /// The value of the "cid" parameter, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a Referred-By header with cid parameter
    /// let header = r#"<sip:alice@example.com>;cid=12345@example.com"#;
    /// let referred_by = ReferredBy::from_str(header).unwrap();
    ///
    /// // Get the cid parameter
    /// assert_eq!(referred_by.cid(), Some("12345@example.com"));
    /// ```
    pub fn cid(&self) -> Option<&str> {
        // Extract the cid parameter value
        self.get_param("cid").and_then(|opt_val| opt_val)
    }

    /// Set the Content-ID (cid) parameter
    ///
    /// Creates a new ReferredBy instance with the cid parameter set to the specified value.
    /// The cid parameter references an S/MIME body part containing a signature for authentication.
    ///
    /// # Parameters
    ///
    /// - `cid_value`: The cid parameter value to set (without the 'cid=' prefix)
    ///
    /// # Returns
    ///
    /// A new ReferredBy instance with the cid parameter set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a ReferredBy header
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let referred_by = ReferredBy::new(address);
    ///
    /// // Add a cid parameter
    /// let referred_by_with_cid = referred_by.with_cid("12345@example.com");
    /// assert_eq!(referred_by_with_cid.cid(), Some("12345@example.com"));
    /// ```
    pub fn with_cid(self, cid_value: &str) -> Self {
        // Create a new Address with the same properties as the original
        let mut new_address = self.0.clone();
        
        // Add/update the cid parameter
        new_address.set_param("cid", Some(cid_value));
        
        // Return a new ReferredBy with the updated Address
        Self(new_address)
    }
}

impl fmt::Display for ReferredBy {
    /// Formats the Referred-By header as a string.
    ///
    /// This method converts the Referred-By header to its string representation,
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
    /// let referred_by = ReferredBy::new(address);
    /// assert_eq!(referred_by.to_string(), "<sip:alice@example.com>");
    ///
    /// // With display name
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let address = Address::new_with_display_name("Bob", uri);
    /// let referred_by = ReferredBy::new(address);
    /// // Depending on the implementation, display names may be quoted or not
    /// let display_str = referred_by.to_string();
    /// assert!(display_str == "Bob <sip:bob@example.com>" || 
    ///        display_str == "\"Bob\" <sip:bob@example.com>");
    ///
    /// // In a complete header
    /// let header = format!("Referred-By: {}", referred_by);
    /// // The header includes the Referred-By name and the address with display name
    /// assert!(header.starts_with("Referred-By:"));
    /// assert!(header.contains("Bob"));
    /// assert!(header.contains("sip:bob@example.com"));
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

// Implement TypedHeaderTrait for ReferredBy
impl TypedHeaderTrait for ReferredBy {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ReferredBy
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::ReferredBy(self.clone()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::ReferredBy(referred_by) => Ok(referred_by.clone()),
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    ReferredBy::from_str(s.trim())
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

impl FromStr for ReferredBy {
    type Err = Error;

    /// Parses a string into a ReferredBy header.
    ///
    /// This method converts a string representation of a Referred-By header into
    /// a structured ReferredBy object. It supports both name-addr and addr-spec formats
    /// as well as parameters.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ReferredBy, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns an error if the input string cannot be parsed as a valid Referred-By header.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple URI
    /// let referred_by = ReferredBy::from_str("<sip:alice@example.com>").unwrap();
    /// assert_eq!(referred_by.uri().to_string(), "sip:alice@example.com");
    ///
    /// // Parse with display name
    /// let referred_by = ReferredBy::from_str("\"Bob\" <sip:bob@example.com>").unwrap();
    /// assert_eq!(referred_by.address().display_name(), Some("Bob"));
    ///
    /// // Parse with cid parameter
    /// let referred_by = ReferredBy::from_str("<sip:carol@example.com>;cid=12345@example.com").unwrap();
    /// assert_eq!(referred_by.get_param("cid"), Some(Some("12345@example.com")));
    ///
    /// // Invalid input
    /// let result = ReferredBy::from_str("invalid-input");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Parse using the referred_by parser which handles both name-addr and addr-spec
        // formats as well as any parameters following the address.
        all_consuming(parse_referred_by)(s.as_bytes())
            .map(|(_rem, address)| ReferredBy::new(address))
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Uri, Scheme};
    use crate::types::address::Address;
    use crate::types::param::Param;
    use std::str::FromStr;

    #[test]
    fn test_referred_by_new() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new(uri);
        let referred_by = ReferredBy::new(address);
        assert_eq!(referred_by.uri().to_string(), "sip:alice@example.com");
    }

    #[test]
    fn test_referred_by_display() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new(uri);
        let referred_by = ReferredBy::new(address);
        assert_eq!(referred_by.to_string(), "<sip:alice@example.com>");

        // With display name (could be quoted or unquoted depending on implementation)
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let address = Address::new_with_display_name("Bob", uri);
        let referred_by = ReferredBy::new(address);
        let display_str = referred_by.to_string();
        assert!(display_str == "\"Bob\" <sip:bob@example.com>" || 
                display_str == "Bob <sip:bob@example.com>",
                "Expected either quoted or unquoted display name, got: {}", display_str);
    }

    #[test]
    fn test_referred_by_from_str() {
        // Simple URI
        let referred_by = ReferredBy::from_str("<sip:alice@example.com>").unwrap();
        assert_eq!(referred_by.uri().scheme, Scheme::Sip);
        assert_eq!(referred_by.uri().host.to_string(), "example.com");
        assert_eq!(referred_by.uri().user, Some("alice".to_string()));

        // With display name
        let referred_by = ReferredBy::from_str("\"Bob\" <sip:bob@example.com>").unwrap();
        assert_eq!(referred_by.address().display_name(), Some("Bob"));
        assert_eq!(referred_by.uri().user, Some("bob".to_string()));

        // With parameters
        let referred_by = ReferredBy::from_str("<sip:carol@example.com>;cid=12345@example.com").unwrap();
        assert_eq!(referred_by.params().len(), 1);
        assert!(referred_by.has_param("cid"));
        let param_value = referred_by.get_param("cid");
        assert!(param_value.is_some());
        let value_option = param_value.unwrap();
        assert!(value_option.is_some());
        assert!(value_option.unwrap().contains("12345"));
    }

    #[test]
    fn test_referred_by_invalid_syntax() {
        // Empty string
        let result = ReferredBy::from_str("");
        assert!(result.is_err());

        // Invalid URI
        let result = ReferredBy::from_str("not-a-uri");
        assert!(result.is_err());
    }

    #[test]
    fn test_referred_by_typed_header_trait() {
        // Create a ReferredBy header
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new(uri);
        let referred_by = ReferredBy::new(address);

        // Convert to Header
        let header = referred_by.to_header();
        assert_eq!(header.name, HeaderName::ReferredBy);

        // Convert back to ReferredBy
        let recovered = ReferredBy::from_header(&header).unwrap();
        assert_eq!(recovered.to_string(), referred_by.to_string());
    }

    #[test]
    fn test_referred_by_with_uri_parameters() {
        // Create URI with parameters
        let uri = Uri::from_str("sip:alice@example.com;transport=tcp").unwrap();
        let address = Address::new(uri);
        let referred_by = ReferredBy::new(address);

        // The parameter should be in the URI, not in the Address params
        assert!(referred_by.params().is_empty());
        assert!(referred_by.uri().to_string().contains("transport=tcp"));
    }

    #[test]
    fn test_referred_by_with_address_parameters() {
        // Create Address with parameters
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let mut address = Address::new(uri);
        
        // Add tag parameter to Address
        address.set_tag("1234");
        
        let referred_by = ReferredBy::new(address);

        // The parameter should be in the Address params
        assert_eq!(referred_by.params().len(), 1);
        assert!(referred_by.has_param("tag"));
        assert_eq!(referred_by.get_param("tag"), Some(Some("1234")));
    }

    #[test]
    fn test_referred_by_with_cid_parameter() {
        // Parse a Referred-By header with cid parameter
        let referred_by = ReferredBy::from_str("<sip:alice@example.com>;cid=12345@atlanta.example.com").unwrap();
        
        // Check the cid parameter
        assert!(referred_by.has_param("cid"));
        let param_value = referred_by.get_param("cid");
        assert!(param_value.is_some());
        let value_option = param_value.unwrap();
        assert!(value_option.is_some());
        let value = value_option.unwrap();
        assert!(value.contains("12345"));
        assert!(value.contains("atlanta.example.com"));
    }
} 