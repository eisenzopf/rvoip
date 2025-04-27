//! # SIP Require Header
//!
//! This module provides an implementation of the SIP Require header as defined in
//! [RFC 3261 Section 20.32](https://datatracker.ietf.org/doc/html/rfc3261#section-20.32).
//!
//! The Require header field is used by user agents to tell servers about SIP extensions that
//! the UA expects the server to support in order to properly process the request. For a
//! server that does not support the required extensions, the proper response is 420 (Bad Extension).
//!
//! ## Purpose
//!
//! The Require header serves as a mechanism to:
//!
//! - Inform UAS about required protocol extensions needed for proper request processing
//! - Enable protocol extensibility while ensuring backward compatibility
//! - Force rejection of requests when required extensions are not supported
//!
//! ## Format
//!
//! ```
//! Require: 100rel
//! Require: 100rel, precondition, timer
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a Require header with multiple option tags
//! let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
//!
//! // Create a Require header with a single option tag
//! let require = Require::with_tag("100rel");
//!
//! // Check if a specific tag is required
//! if require.requires("100rel") {
//!     // The 100rel extension is required
//! }
//! ```

use crate::error::Result;
use crate::types::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use std::fmt;
use serde::{Deserialize, Serialize};

/// Require header (RFC 3261 Section 20.32)
///
/// The Require header field is used by clients to tell UAS about options that 
/// the client expects the server to support in order to properly process the request.
/// 
/// Although an optional header field, the Require MUST NOT be ignored if it
/// is present. The server MUST respond with a 420 (Bad Extension) if it
/// does not understand the option.
///
/// # Format
///
/// ```
/// Require: option-tag1, option-tag2, ...
/// ```
///
/// Where each option-tag is a token that identifies a SIP extension or feature.
/// 
/// # Common Option Tags
///
/// SIP defines several standard option tags:
/// 
/// - `100rel`: Indicates the client requires reliable provisional responses
/// - `precondition`: Indicates support for the precondition framework
/// - `timer`: Indicates support for the SIP session timers extension
/// - `replaces`: Indicates support for the SIP Replaces header
/// 
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a Require header with multiple option tags
/// let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
///
/// // Create with a single tag
/// let require = Require::with_tag("100rel");
/// 
/// // Check if the "100rel" extension is required
/// if require.requires("100rel") {
///     // Handle reliable provisional responses
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Require {
    /// List of option-tags required by the client
    pub option_tags: Vec<String>,
}

impl Require {
    /// Create a new Require header with the given option tags
    ///
    /// Initializes a Require header containing multiple option tags that the client
    /// expects the server to support.
    ///
    /// # Parameters
    ///
    /// - `option_tags`: A vector of option tag strings
    ///
    /// # Returns
    ///
    /// A new `Require` instance containing the specified option tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Require header with multiple option tags
    /// let require = Require::new(vec![
    ///     "100rel".to_string(),
    ///     "precondition".to_string(),
    ///     "timer".to_string()
    /// ]);
    /// 
    /// assert_eq!(require.option_tags.len(), 3);
    /// assert!(require.requires("100rel"));
    /// assert!(require.requires("precondition"));
    /// assert!(require.requires("timer"));
    /// ```
    pub fn new(option_tags: Vec<String>) -> Self {
        Self { option_tags }
    }

    /// Create a new Require header with a single option tag
    ///
    /// Convenience method to create a Require header with just one option tag.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to include, can be any type that can be converted into a String
    ///
    /// # Returns
    ///
    /// A new `Require` instance containing the specified option tag
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Require header requiring reliable provisional responses
    /// let require = Require::with_tag("100rel");
    /// 
    /// assert_eq!(require.option_tags.len(), 1);
    /// assert!(require.requires("100rel"));
    /// assert!(!require.requires("timer"));
    /// ```
    pub fn with_tag(tag: impl Into<String>) -> Self {
        Self {
            option_tags: vec![tag.into()],
        }
    }

    /// Check if a specific tag is required
    ///
    /// Tests whether the specified option tag is present in this Require header.
    /// This indicates that the client requires support for the extension 
    /// identified by this tag.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to check for
    ///
    /// # Returns
    ///
    /// `true` if the tag is present, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
    /// 
    /// // Check for required extensions
    /// assert!(require.requires("100rel"));
    /// assert!(require.requires("precondition"));
    /// assert!(!require.requires("timer"));
    /// 
    /// // Use in conditional logic
    /// if require.requires("100rel") {
    ///     // Handle reliable provisional responses
    /// }
    /// ```
    pub fn requires(&self, tag: &str) -> bool {
        self.option_tags.iter().any(|t| t == tag)
    }

    /// Add a new option tag
    ///
    /// Adds a new option tag to this Require header.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to add, can be any type that can be converted into a String
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Start with one option tag
    /// let mut require = Require::with_tag("100rel");
    /// assert_eq!(require.option_tags.len(), 1);
    /// 
    /// // Add another tag
    /// require.add_tag("precondition");
    /// assert_eq!(require.option_tags.len(), 2);
    /// assert!(require.requires("100rel"));
    /// assert!(require.requires("precondition"));
    /// ```
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.option_tags.push(tag.into());
    }

    /// Remove an option tag if it exists
    ///
    /// Removes the specified option tag from this Require header if it is present.
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
    /// // Create a Require header with multiple tags
    /// let mut require = Require::new(vec![
    ///     "100rel".to_string(),
    ///     "precondition".to_string(),
    ///     "timer".to_string()
    /// ]);
    /// 
    /// // Remove a tag
    /// require.remove_tag("precondition");
    /// 
    /// // Verify it was removed
    /// assert_eq!(require.option_tags.len(), 2);
    /// assert!(require.requires("100rel"));
    /// assert!(!require.requires("precondition"));
    /// assert!(require.requires("timer"));
    /// ```
    pub fn remove_tag(&mut self, tag: &str) {
        self.option_tags.retain(|t| t != tag);
    }
}

impl fmt::Display for Require {
    /// Formats the Require header as a string.
    ///
    /// Converts the header to its string representation, with option
    /// tags separated by commas, according to the format specified in RFC 3261.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// // Single tag
    /// let require = Require::with_tag("100rel");
    /// assert_eq!(require.to_string(), "100rel");
    ///
    /// // Multiple tags
    /// let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
    /// assert_eq!(require.to_string(), "100rel, precondition");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl TypedHeaderTrait for Require {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Require` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Require
    }

    /// Converts this Require header into a generic Header.
    ///
    /// # Returns
    ///
    /// A generic `Header` containing this Require header's data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
    /// let header = require.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::Require);
    /// ```
    fn to_header(&self) -> Header {
        // Convert Vec<String> to Vec<Vec<u8>> for HeaderValue::Require
        let option_tags_bytes: Vec<Vec<u8>> = self.option_tags
            .iter()
            .map(|tag| tag.as_bytes().to_vec())
            .collect();

        Header::new(
            Self::header_name(),
            HeaderValue::Require(option_tags_bytes),
        )
    }

    /// Creates a Require header from a generic Header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Require header, or an error if conversion fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a header with raw value
    /// let header = Header::new(
    ///     HeaderName::Require,
    ///     HeaderValue::Raw(b"100rel, precondition".to_vec()),
    /// );
    ///
    /// // Convert to Require
    /// let require = Require::from_header(&header).unwrap();
    ///
    /// // Check conversion
    /// assert_eq!(require.option_tags.len(), 2);
    /// assert_eq!(require.option_tags[0], "100rel");
    /// assert_eq!(require.option_tags[1], "precondition");
    /// ```
    fn from_header(header: &Header) -> Result<Self> {
        match &header.value {
            HeaderValue::Require(option_tags) => {
                // Convert Vec<Vec<u8>> to Vec<String>
                let string_tags = option_tags.iter().map(|tag| {
                    String::from_utf8_lossy(tag).to_string()
                }).collect();
                
                Ok(Self {
                    option_tags: string_tags,
                })
            },
            HeaderValue::Raw(raw) => {
                // Parse raw value using the parser
                let (_, parsed) = 
                    crate::parser::headers::require::parse_require(raw)
                        .map_err(|e| crate::error::Error::ParseError(format!("Failed to parse Require header: {:?}", e)))?;
                Ok(Self { option_tags: parsed })
            }
            _ => Err(crate::error::Error::ParseError(
                "Invalid header value type for Require".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_creation() {
        let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
        assert_eq!(require.option_tags.len(), 2);
        assert!(require.requires("100rel"));
        assert!(require.requires("precondition"));
        assert!(!require.requires("timer"));
    }

    #[test]
    fn test_require_with_tag() {
        let require = Require::with_tag("100rel");
        assert_eq!(require.option_tags.len(), 1);
        assert!(require.requires("100rel"));
    }

    #[test]
    fn test_require_modification() {
        let mut require = Require::with_tag("100rel");
        require.add_tag("precondition");
        assert_eq!(require.option_tags.len(), 2);
        assert!(require.requires("precondition"));

        require.remove_tag("100rel");
        assert_eq!(require.option_tags.len(), 1);
        assert!(!require.requires("100rel"));
        assert!(require.requires("precondition"));
    }

    #[test]
    fn test_require_display() {
        let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
        assert_eq!(format!("{}", require), "100rel, precondition");
    }

    #[test]
    fn test_require_to_header() {
        let require = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
        let header = require.to_header();
        assert_eq!(header.name, HeaderName::Require);
        match header.value {
            HeaderValue::Require(tags) => {
                assert_eq!(tags.len(), 2);
                assert_eq!(String::from_utf8_lossy(&tags[0]), "100rel");
                assert_eq!(String::from_utf8_lossy(&tags[1]), "precondition");
            }
            _ => panic!("Expected HeaderValue::Require"),
        }
    }

    #[test]
    fn test_require_from_header() {
        // Create a header with raw value
        let header = Header::new(
            HeaderName::Require,
            HeaderValue::Raw(b"100rel, precondition".to_vec()),
        );
        
        // Convert to Require
        let require = Require::from_header(&header).unwrap();
        
        // Check conversion
        assert_eq!(require.option_tags.len(), 2);
        assert_eq!(require.option_tags[0], "100rel");
        assert_eq!(require.option_tags[1], "precondition");
    }

    #[test]
    fn test_require_roundtrip() {
        // Create a Require
        let original = Require::new(vec!["100rel".to_string(), "precondition".to_string()]);
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to Require
        let roundtrip = Require::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }
} 