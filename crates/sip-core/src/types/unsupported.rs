//! # SIP Unsupported Header
//!
//! This module provides an implementation of the SIP Unsupported header as defined in
//! [RFC 3261 Section 20.41](https://datatracker.ietf.org/doc/html/rfc3261#section-20.41).
//!
//! The Unsupported header field lists the features not supported by the User Agent Server (UAS).
//! It is primarily used in 420 (Bad Extension) responses to indicate which extensions requested
//! in the `Require` header field are not supported by the server.
//!
//! When a UAS receives a request containing a `Require` header field with option tags it does not
//! understand or support, it responds with a 420 (Bad Extension) response containing an
//! Unsupported header field listing the option tags it does not support.
//!
//! ## Format
//!
//! ```rust
//! // Unsupported: timer, 100rel
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an Unsupported header with multiple option tags
//! let mut unsupported = Unsupported::new();
//! unsupported.add_option_tag("timer");
//! unsupported.add_option_tag("100rel");
//!
//! // Check if specific features are unsupported
//! assert!(unsupported.has_option_tag("timer"));
//!
//! // Parse from a string
//! let unsupported = Unsupported::from_str("timer, 100rel").unwrap();
//! assert!(unsupported.has_option_tag("100rel"));
//! ```

use std::fmt;
use std::str::FromStr;

use crate::parser;
use crate::Error;
use crate::types::{
    Header, HeaderName, HeaderValue, TypedHeaderTrait
};
use serde::{Serialize, Deserialize};
use nom::combinator::all_consuming;

/// Represents an Unsupported header as defined in RFC 3261 Section 20.41
///
/// The Unsupported header field lists the features not supported by the UAS.
/// It is commonly used in 420 (Bad Extension) responses to inform the client
/// which required extensions cannot be supported by the server.
///
/// When a server receives a request with a `Require` header containing option tags
/// it does not support, it must respond with a 420 response and include an
/// Unsupported header listing these unsupported option tags.
///
/// The header contains a comma-separated list of option tags, which are tokens
/// that identify specific SIP protocol extensions (such as "100rel", "timer", etc.).
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// 
/// // Create an empty Unsupported header
/// let mut unsupported = Unsupported::new();
/// 
/// // Add unsupported option tags
/// unsupported.add_option_tag("timer");
/// unsupported.add_option_tag("100rel");
/// 
/// // Check for specific unsupported features
/// assert!(unsupported.has_option_tag("timer"));
/// assert!(unsupported.has_option_tag("100rel"));
/// assert!(!unsupported.has_option_tag("path"));
/// 
/// // Get all unsupported tags
/// let tags = unsupported.option_tags();
/// assert_eq!(tags.len(), 2);
/// 
/// // Convert to a string for a SIP message
/// assert_eq!(unsupported.to_string(), "timer, 100rel");
/// ```
///
/// ```
/// use rvoip_sip_core::prelude::*;
/// 
/// let mut unsupported = Unsupported::new();
/// unsupported.add_option_tag("timer");
/// unsupported.add_option_tag("100rel");
/// 
/// assert!(unsupported.has_option_tag("timer"));
/// assert!(unsupported.has_option_tag("100rel"));
/// assert!(!unsupported.has_option_tag("path"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsupported {
    option_tags: Vec<String>,
}

impl Unsupported {
    /// Create a new empty Unsupported header
    ///
    /// Initializes a new Unsupported header with an empty list of option tags.
    ///
    /// # Returns
    ///
    /// A new empty `Unsupported` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let unsupported = Unsupported::new();
    /// assert!(unsupported.option_tags().is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            option_tags: Vec::new(),
        }
    }

    /// Create an Unsupported header with the given option tags
    ///
    /// Initializes a new Unsupported header with a list of option tags,
    /// indicating the features that the server does not support.
    ///
    /// # Parameters
    ///
    /// - `tags`: A vector of strings, each representing an unsupported feature
    ///
    /// # Returns
    ///
    /// A new `Unsupported` instance containing the specified option tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create an Unsupported header with multiple option tags
    /// let tags = vec!["timer".to_string(), "100rel".to_string()];
    /// let unsupported = Unsupported::with_tags(tags);
    ///
    /// assert_eq!(unsupported.option_tags().len(), 2);
    /// assert!(unsupported.has_option_tag("timer"));
    /// assert!(unsupported.has_option_tag("100rel"));
    /// ```
    pub fn with_tags(tags: Vec<String>) -> Self {
        Self {
            option_tags: tags,
        }
    }

    /// Check if this Unsupported header contains a specific option tag
    ///
    /// Tests whether the Unsupported header lists a specific option tag
    /// as an unsupported feature.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to check for
    ///
    /// # Returns
    ///
    /// `true` if the specified option tag is included in the Unsupported header,
    /// `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut unsupported = Unsupported::new();
    /// unsupported.add_option_tag("timer");
    ///
    /// // Check for listed options
    /// assert!(unsupported.has_option_tag("timer"));
    /// assert!(!unsupported.has_option_tag("100rel"));
    /// ```
    pub fn has_option_tag(&self, tag: &str) -> bool {
        self.option_tags.iter().any(|t| t == tag)
    }

    /// Add an option tag to this Unsupported header
    ///
    /// Adds a new option tag to the Unsupported header, indicating an
    /// additional unsupported feature. If the tag is already present,
    /// it will not be added again (no duplicates).
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut unsupported = Unsupported::new();
    ///
    /// // Add unsupported options
    /// unsupported.add_option_tag("timer");
    /// assert!(unsupported.has_option_tag("timer"));
    ///
    /// // Add another option
    /// unsupported.add_option_tag("100rel");
    /// assert!(unsupported.has_option_tag("100rel"));
    ///
    /// // Adding duplicates has no effect
    /// unsupported.add_option_tag("timer");
    /// assert_eq!(unsupported.option_tags().len(), 2);
    /// ```
    pub fn add_option_tag(&mut self, tag: &str) {
        if !self.has_option_tag(tag) {
            self.option_tags.push(tag.to_string());
        }
    }

    /// Remove an option tag from this Unsupported header
    ///
    /// Removes the specified option tag from the Unsupported header,
    /// if it was previously listed as unsupported.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create with initial options
    /// let mut unsupported = Unsupported::with_tags(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string()
    /// ]);
    /// assert_eq!(unsupported.option_tags().len(), 2);
    ///
    /// // Remove an option
    /// unsupported.remove_option_tag("timer");
    /// assert!(!unsupported.has_option_tag("timer"));
    /// assert_eq!(unsupported.option_tags().len(), 1);
    ///
    /// // Removing non-existent tag has no effect
    /// unsupported.remove_option_tag("path");
    /// assert_eq!(unsupported.option_tags().len(), 1);
    /// ```
    pub fn remove_option_tag(&mut self, tag: &str) {
        self.option_tags.retain(|t| t != tag);
    }

    /// Get all option tags in this Unsupported header
    ///
    /// Returns a slice of all option tags listed in this Unsupported header,
    /// representing all the features not supported by the server.
    ///
    /// # Returns
    ///
    /// A slice containing all the option tags in this Unsupported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut unsupported = Unsupported::new();
    /// unsupported.add_option_tag("timer");
    /// unsupported.add_option_tag("100rel");
    ///
    /// let tags = unsupported.option_tags();
    /// assert_eq!(tags.len(), 2);
    /// assert_eq!(tags[0], "timer");
    /// assert_eq!(tags[1], "100rel");
    /// ```
    pub fn option_tags(&self) -> &[String] {
        &self.option_tags
    }
}

impl Default for Unsupported {
    /// Provides the default value for `Unsupported`, which is an empty header.
    ///
    /// # Returns
    ///
    /// A new empty `Unsupported` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::default::Default;
    ///
    /// let unsupported = Unsupported::default();
    /// assert!(unsupported.option_tags().is_empty());
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Unsupported {
    /// Formats the Unsupported header as a string.
    ///
    /// Converts the Unsupported header to its string representation,
    /// which is a comma-separated list of option tags. If the header
    /// contains no option tags, an empty string is returned.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let mut unsupported = Unsupported::new();
    /// assert_eq!(unsupported.to_string(), "");
    ///
    /// unsupported.add_option_tag("timer");
    /// assert_eq!(unsupported.to_string(), "timer");
    ///
    /// unsupported.add_option_tag("100rel");
    /// assert_eq!(unsupported.to_string(), "timer, 100rel");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.option_tags.is_empty() {
            return Ok(());
        }

        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl FromStr for Unsupported {
    type Err = Error;

    /// Parses a string into an Unsupported header.
    ///
    /// Converts a comma-separated list of option tags into an Unsupported struct.
    /// Each tag is expected to be a token as defined in the SIP specifications.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse as an Unsupported header
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Unsupported header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple list
    /// let unsupported = Unsupported::from_str("timer, 100rel").unwrap();
    /// assert_eq!(unsupported.option_tags().len(), 2);
    /// assert!(unsupported.has_option_tag("timer"));
    /// assert!(unsupported.has_option_tag("100rel"));
    ///
    /// // Parse an empty string
    /// let empty = Unsupported::from_str("").unwrap();
    /// assert_eq!(empty.option_tags().len(), 0);
    /// ```
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // Special case for empty string
        if s.trim().is_empty() {
            return Ok(Unsupported::new());
        }

        // For non-empty strings, use the parser
        let input = s.as_bytes();
        match parser::headers::unsupported::parse_unsupported(input) {
            Ok((_, tags)) => Ok(Unsupported::with_tags(tags)),
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl TypedHeaderTrait for Unsupported {
    type Name = HeaderName;
    
    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Unsupported` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Unsupported
    }

    /// Converts this Unsupported header into a generic Header.
    ///
    /// Creates a Header instance from this Unsupported header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A generic `Header` containing this Unsupported header's data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut unsupported = Unsupported::new();
    /// unsupported.add_option_tag("timer");
    /// unsupported.add_option_tag("100rel");
    ///
    /// let header = unsupported.to_header();
    /// assert_eq!(header.name, HeaderName::Unsupported);
    /// // The header value contains the option tags
    /// ```
    fn to_header(&self) -> Header {
        let value_string = self.to_string();
        let value = crate::types::headers::HeaderValue::Raw(value_string.into_bytes());
        Header::new(Self::header_name(), value)
    }

    /// Creates an Unsupported header from a generic Header.
    ///
    /// Converts a generic Header to an Unsupported instance, if the header
    /// represents a valid Unsupported header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Unsupported header, or an error if conversion fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a header with Unsupported value
    /// let header = Header::new(
    ///     HeaderName::Unsupported,
    ///     HeaderValue::Raw("timer, 100rel".as_bytes().to_vec())
    /// );
    ///
    /// // Convert to Unsupported
    /// let unsupported = Unsupported::from_header(&header).unwrap();
    /// assert_eq!(unsupported.option_tags().len(), 2);
    /// assert!(unsupported.has_option_tag("timer"));
    /// assert!(unsupported.has_option_tag("100rel"));
    /// ```
    fn from_header(header: &Header) -> crate::error::Result<Self> {
        if header.name != HeaderName::Unsupported {
            return Err(Error::InvalidHeader(format!("Expected Unsupported header, got {}", header.name)));
        }
        
        // Use the parser to convert the header value into an Unsupported header
        use crate::parser::headers::unsupported::parse_unsupported;
        use nom::combinator::all_consuming;
        
        // Get the raw bytes from the header value
        let bytes = match &header.value {
            crate::types::headers::HeaderValue::Raw(bytes) => bytes,
            _ => return Err(Error::InvalidHeader("Expected raw header value".to_string())),
        };
        
        // Parse the header value
        let option_tags = all_consuming(parse_unsupported)(bytes)
            .map_err(Error::from)
            .map(|(_, v)| v)?;
            
        Ok(Unsupported::with_tags(option_tags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_new() {
        let unsupported = Unsupported::new();
        assert!(unsupported.option_tags().is_empty());
    }

    #[test]
    fn test_unsupported_with_tags() {
        let tags = vec!["timer".to_string(), "100rel".to_string()];
        let unsupported = Unsupported::with_tags(tags.clone());
        assert_eq!(unsupported.option_tags(), tags);
    }

    #[test]
    fn test_unsupported_has_option_tag() {
        let mut unsupported = Unsupported::new();
        unsupported.add_option_tag("timer");
        
        assert!(unsupported.has_option_tag("timer"));
        assert!(!unsupported.has_option_tag("100rel"));
    }

    #[test]
    fn test_unsupported_add_option_tag() {
        let mut unsupported = Unsupported::new();
        unsupported.add_option_tag("timer");
        unsupported.add_option_tag("100rel");
        
        assert_eq!(unsupported.option_tags(), &["timer".to_string(), "100rel".to_string()]);

        // Adding duplicate should not change anything
        unsupported.add_option_tag("timer");
        assert_eq!(unsupported.option_tags(), &["timer".to_string(), "100rel".to_string()]);
    }

    #[test]
    fn test_unsupported_remove_option_tag() {
        let mut unsupported = Unsupported::with_tags(vec!["timer".to_string(), "100rel".to_string()]);
        unsupported.remove_option_tag("timer");
        
        assert_eq!(unsupported.option_tags(), &["100rel".to_string()]);
        
        // Removing non-existent tag should not change anything
        unsupported.remove_option_tag("path");
        assert_eq!(unsupported.option_tags(), &["100rel".to_string()]);
    }

    #[test]
    fn test_unsupported_display() {
        let mut unsupported = Unsupported::new();
        assert_eq!(unsupported.to_string(), "");
        
        unsupported.add_option_tag("timer");
        assert_eq!(unsupported.to_string(), "timer");
        
        unsupported.add_option_tag("100rel");
        assert_eq!(unsupported.to_string(), "timer, 100rel");
    }

    #[test]
    fn test_unsupported_from_str() {
        let unsupported: Unsupported = "timer, 100rel".parse().unwrap();
        assert_eq!(unsupported.option_tags(), &["timer".to_string(), "100rel".to_string()]);
    }

    #[test]
    fn test_unsupported_to_header() {
        let unsupported = Unsupported::with_tags(vec!["timer".to_string(), "100rel".to_string()]);
        let header = unsupported.to_header();
        
        assert_eq!(header.name, HeaderName::Unsupported);
        match &header.value {
            crate::types::headers::HeaderValue::Raw(bytes) => {
                let value_string = String::from_utf8_lossy(bytes).to_string();
                assert_eq!(value_string, "timer, 100rel");
            },
            _ => panic!("Expected HeaderValue::Raw"),
        }
    }

    #[test]
    fn test_unsupported_from_header() {
        // Create the header with a raw string
        let raw_value = "timer, 100rel".as_bytes().to_vec();
        let header = Header {
            name: HeaderName::Unsupported,
            value: crate::types::headers::HeaderValue::Raw(raw_value),
        };
        
        let unsupported = Unsupported::from_header(&header).unwrap();
        assert_eq!(unsupported.option_tags(), &["timer".to_string(), "100rel".to_string()]);
    }

    #[test]
    fn test_unsupported_roundtrip() {
        let tags = vec!["timer".to_string(), "100rel".to_string()];
        let original = Unsupported::with_tags(tags);
        
        let header = original.to_header();
        let roundtrip = Unsupported::from_header(&header).unwrap();
        
        assert_eq!(original, roundtrip);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serialization() {
        let unsupported = Unsupported::with_tags(vec!["timer".to_string(), "100rel".to_string()]);
        let json = serde_json::to_string(&unsupported).unwrap();
        
        // Convert to header
        let header = unsupported.to_header();
        let header = unsupported.to_header(); // Just for the test, don't need to manipulate raw value
    }
} 