//! # SIP Organization Header
//!
//! This module provides an implementation of the SIP Organization header as defined in
//! [RFC 3261 Section 20.27](https://datatracker.ietf.org/doc/html/rfc3261#section-20.27).
//!
//! The Organization header field conveys the name of the organization to which the
//! entity issuing the request or response belongs. It is primarily informational
//! and can be used by the recipient's user agent to display organization information
//! about the caller or callee.
//!
//! ## Header Format
//!
//! The Organization header has a simple format consisting of a token or quoted string:
//!
//! ```
//! Organization: Rudeless Ventures
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a new Organization header
//! let org = Organization::new("Rudeless Ventures");
//!
//! // Parse from a string
//! let parsed_org = Organization::from_str("Example Corp.").unwrap();
//!
//! // Convert to a generic Header for inclusion in a SIP message
//! let header = org.to_header();
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::header::{HeaderValue, HeaderName, Header, TypedHeaderTrait};
use crate::parser::headers::organization::parse_organization;

/// Represents the Organization header field as defined in RFC 3261 Section 20.27
/// 
/// The Organization header field indicates the identity of the organizational entity 
/// associated with the user agent (for example, "Rudeless Ventures").
///
/// This header is optional in SIP messages and is primarily used for informational
/// purposes. It allows recipients to see which organization sent a request or response.
///
/// The Organization header's value is a simple text string that can contain the name
/// of a company, institution, or any other organizational entity.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a new Organization header
/// let org = Organization::new("Rudeless Ventures");
///
/// // Convert to a string
/// assert_eq!(org.to_string(), "Rudeless Ventures");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Organization(pub String);

impl Organization {
    /// Creates a new Organization header value
    ///
    /// Initializes a new `Organization` instance with the provided organization name.
    ///
    /// # Parameters
    ///
    /// - `org`: The organization name, which can be any type that can be converted into a `String`
    ///
    /// # Returns
    ///
    /// A new `Organization` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create with a string literal
    /// let org1 = Organization::new("Rudeless Ventures");
    ///
    /// // Create with a String
    /// let name = String::from("Example Corp.");
    /// let org2 = Organization::new(name);
    /// ```
    pub fn new<S: Into<String>>(org: S) -> Self {
        Organization(org.into())
    }

    /// Returns the organization name as a string slice
    ///
    /// Provides access to the underlying organization name without taking ownership.
    ///
    /// # Returns
    ///
    /// A string slice (`&str`) containing the organization name
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let org = Organization::new("Rudeless Ventures");
    /// assert_eq!(org.as_str(), "Rudeless Ventures");
    ///
    /// // Useful for comparing without allocating
    /// assert!(org.as_str().contains("Ventures"));
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Organization {
    type Err = crate::error::Error;

    /// Parses an Organization header from a string.
    ///
    /// Converts a string representation of an Organization header into an
    /// `Organization` object. The parsing is performed using the parser defined
    /// in the `crate::parser::headers::organization` module.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// - `Ok(Organization)`: If parsing succeeds
    /// - `Err`: If parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple organization name
    /// let org = Organization::from_str("Example Corp.").unwrap();
    /// assert_eq!(org.as_str(), "Example Corp.");
    ///
    /// // Parse an organization name with special characters
    /// let org = Organization::from_str("Acme ÜÖ GmbH").unwrap();
    /// assert_eq!(org.as_str(), "Acme ÜÖ GmbH");
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse the input as bytes
        let bytes = s.as_bytes();
        let (_, org) = parse_organization(bytes)
            .map_err(|_| crate::error::Error::ParseError("Failed to parse Organization header".into()))?;
        
        // Return the parsed Organization
        Ok(org)
    }
}

impl fmt::Display for Organization {
    /// Formats the Organization header as a string.
    ///
    /// Converts the `Organization` object to its string representation suitable
    /// for inclusion in a SIP message.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let org = Organization::new("Rudeless Ventures");
    /// assert_eq!(format!("{}", org), "Rudeless Ventures");
    ///
    /// // Useful when generating complete SIP headers
    /// let header_str = format!("Organization: {}", org);
    /// assert_eq!(header_str, "Organization: Rudeless Ventures");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TypedHeaderTrait for Organization {
    type Name = HeaderName;
    
    /// Returns the HeaderName for the Organization header.
    ///
    /// Implements the required method from `TypedHeaderTrait` to provide the
    /// appropriate header name constant.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Organization` constant
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(Organization::header_name(), HeaderName::Organization);
    /// ```
    fn header_name() -> Self::Name {
        HeaderName::Organization
    }
    
    /// Converts the Organization object to a generic Header.
    ///
    /// Transforms this typed `Organization` header into a generic `Header` object
    /// that can be included in a SIP message.
    ///
    /// # Returns
    ///
    /// A `Header` instance representing this Organization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let org = Organization::new("Rudeless Ventures");
    /// let header = org.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::Organization);
    /// assert_eq!(header.value.as_text().unwrap(), "Rudeless Ventures");
    /// ```
    fn to_header(&self) -> Header {
        Header::text(HeaderName::Organization, &self.0)
    }
    
    /// Creates an Organization object from a generic Header.
    ///
    /// Attempts to convert a generic `Header` into a typed `Organization` object,
    /// verifying that the header's name is correct and its value can be interpreted
    /// as an organization name.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic `Header` to convert
    ///
    /// # Returns
    ///
    /// - `Ok(Organization)`: If conversion succeeds
    /// - `Err`: If the header is not a valid Organization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a generic header
    /// let header = Header::text(HeaderName::Organization, "Example Corp.");
    ///
    /// // Convert to a typed Organization header
    /// let org = Organization::from_header(&header).unwrap();
    /// assert_eq!(org.as_str(), "Example Corp.");
    ///
    /// // Attempting to convert an incorrect header type will fail
    /// let wrong_header = Header::text(HeaderName::From, "someone@example.com");
    /// assert!(Organization::from_header(&wrong_header).is_err());
    /// ```
    fn from_header(header: &Header) -> Result<Self, crate::error::Error> {
        if header.name != HeaderName::Organization {
            return Err(crate::error::Error::ParseError(
                format!("Expected Organization header, got {}", header.name)
            ));
        }
        
        // Get the value as a string
        if let Some(text) = header.value.as_text() {
            Ok(Organization(text.to_string()))
        } else {
            Err(crate::error::Error::ParseError("Invalid Organization header value".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_organization_parsing() {
        let test_cases = [
            "Rudeless Ventures",
            "Example Corp.",
            "IETF SIP Working Group",
            "",
            "Acme ÜÖ GmbH", // UTF-8 characters
        ];

        for test_case in test_cases {
            let org = Organization::from_str(test_case).unwrap();
            assert_eq!(org.as_str(), test_case);
            
            // Test TypedHeaderTrait implementation
            let header = Header::text(HeaderName::Organization, test_case);
            let parsed_org = Organization::from_header(&header).unwrap();
            assert_eq!(parsed_org, org);
            
            // Test conversion back to Header
            let converted_header = org.to_header();
            assert_eq!(converted_header.name, HeaderName::Organization);
            assert_eq!(converted_header.value.as_text().unwrap(), test_case);
        }
    }

    #[test]
    fn test_organization_display() {
        let org = Organization::new("Rudeless Ventures");
        assert_eq!(format!("{}", org), "Rudeless Ventures");
    }
} 