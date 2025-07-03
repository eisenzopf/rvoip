//! # SIP Supported Header
//!
//! This module provides an implementation of the SIP Supported header as defined in
//! [RFC 3261 Section 20.37](https://datatracker.ietf.org/doc/html/rfc3261#section-20.37).
//!
//! The Supported header field (also called by the short form "k") enumerates all the extensions 
//! supported by the User Agent Client (UAC) or User Agent Server (UAS). It contains a list of 
//! option tags that are understood by the UA.
//!
//! This header allows UAs to advertise their capabilities and for the recipient to understand
//! which extensions can be used in the current session.
//!
//! ## Common Option Tags
//!
//! Some common option tags include:
//!
//! - `100rel`: Support for reliable provisional responses (RFC 3262)
//! - `timer`: Support for session timers (RFC 4028)
//! - `path`: Support for the Path header field extension (RFC 3327)
//! - `outbound`: Support for the outbound registration mechanism (RFC 5626)
//! - `gruu`: Support for Globally Routable UA URIs (RFC 5627)
//!
//! ## Format
//!
//! ```rust
//! // Supported: 100rel, timer, path
//! // k: 100rel, path
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Supported header with multiple option tags
//! let supported = Supported::new(vec!["100rel".to_string(), "timer".to_string()]);
//! assert!(supported.supports("100rel"));
//!
//! // Parse from a string
//! let supported = Supported::from_str("timer, outbound").unwrap();
//! assert!(supported.supports("outbound"));
//! ```

use crate::error::Result;
use crate::types::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

/// Supported header (RFC 3261 Section 20.37)
///
/// The Supported header field enumerates all the extensions supported
/// by the User Agent Client (UAC) or User Agent Server (UAS).
///
/// The Supported header field contains a list of option tags, described
/// in Section 19.2, that are understood by the UA. A UA compliant to
/// this specification MUST include the option tag 'timer' in a
/// Supported header field in all requests and responses except ACK.
///
/// If no Supported header field is present, the recipient can assume that
/// the sender of the message is minimally compliant with this specification.
///
/// The Supported header can be used in many SIP request and response types to
/// indicate capabilities, most commonly in REGISTER and INVITE requests. It may
/// also be used by servers to indicate supported extensions in responses, particularly
/// when rejecting a request with a 420 (Bad Extension) status code.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a new Supported header with multiple extensions
/// let supported = Supported::new(vec![
///     "timer".to_string(),
///     "100rel".to_string(),
///     "path".to_string()
/// ]);
///
/// // Check if specific extensions are supported
/// assert!(supported.supports("timer"));
/// assert!(supported.supports("100rel"));
/// assert!(!supported.supports("outbound"));
///
/// // Format as a string for a SIP message
/// assert_eq!(supported.to_string(), "timer, 100rel, path");
/// ```
///
/// Example:
///   Supported: 100rel, timer, path
///   k: 100rel, path
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supported {
    /// List of option-tags supported
    pub option_tags: Vec<String>,
}

impl Supported {
    /// Create a new Supported header with the given option tags
    ///
    /// Initializes a new Supported header with a list of option tags, 
    /// indicating the extensions that the UA supports.
    ///
    /// # Parameters
    ///
    /// - `option_tags`: A vector of strings, each representing a supported extension
    ///
    /// # Returns
    ///
    /// A new `Supported` instance containing the specified option tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Supported header with multiple option tags
    /// let supported = Supported::new(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string() 
    /// ]);
    ///
    /// assert_eq!(supported.option_tags.len(), 2);
    /// assert!(supported.supports("timer"));
    /// assert!(supported.supports("100rel"));
    ///
    /// // Create an empty Supported header
    /// let empty = Supported::new(vec![]);
    /// assert_eq!(empty.option_tags.len(), 0);
    /// ```
    pub fn new(option_tags: Vec<String>) -> Self {
        Self { option_tags }
    }

    /// Create a new Supported header with a single option tag
    ///
    /// Convenience method to create a Supported header with just one 
    /// option tag. This is useful when a UA wants to advertise support
    /// for just a single extension.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to include, can be any type that can be converted into a String
    ///
    /// # Returns
    ///
    /// A new `Supported` instance containing the specified option tag
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Supported header with just the 'timer' option tag
    /// let supported = Supported::with_tag("timer");
    ///
    /// assert_eq!(supported.option_tags.len(), 1);
    /// assert!(supported.supports("timer"));
    /// assert!(!supported.supports("100rel"));
    ///
    /// // Using a String as input
    /// let tag = String::from("outbound");
    /// let supported = Supported::with_tag(tag);
    /// assert!(supported.supports("outbound"));
    /// ```
    pub fn with_tag(tag: impl Into<String>) -> Self {
        Self {
            option_tags: vec![tag.into()],
        }
    }

    /// Check if a specific extension is supported
    ///
    /// Tests whether the Supported header contains a specific option tag,
    /// indicating support for that extension.
    ///
    /// # Parameters
    ///
    /// - `tag`: The option tag to check for
    ///
    /// # Returns
    ///
    /// `true` if the specified option tag is included in the Supported header,
    /// `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let supported = Supported::new(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string()
    /// ]);
    ///
    /// // Check for supported extensions
    /// assert!(supported.supports("timer"));
    /// assert!(supported.supports("100rel"));
    ///
    /// // Check for unsupported extensions
    /// assert!(!supported.supports("path"));
    /// assert!(!supported.supports("outbound"));
    /// ```
    pub fn supports(&self, tag: &str) -> bool {
        self.option_tags.iter().any(|t| t == tag)
    }

    /// Add a new option tag
    ///
    /// Adds a new option tag to the Supported header, indicating support
    /// for an additional extension.
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
    /// let mut supported = Supported::with_tag("timer");
    /// assert!(supported.supports("timer"));
    /// assert!(!supported.supports("100rel"));
    ///
    /// // Add support for 100rel
    /// supported.add_tag("100rel");
    /// assert!(supported.supports("100rel"));
    /// assert_eq!(supported.option_tags.len(), 2);
    ///
    /// // Add another tag as a String
    /// let tag = String::from("path");
    /// supported.add_tag(tag);
    /// assert!(supported.supports("path"));
    /// assert_eq!(supported.option_tags.len(), 3);
    /// ```
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.option_tags.push(tag.into());
    }

    /// Remove an option tag if it exists
    ///
    /// Removes the specified option tag from the Supported header,
    /// indicating that the UA no longer supports that extension.
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
    /// let mut supported = Supported::new(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string(),
    ///     "path".to_string()
    /// ]);
    /// assert_eq!(supported.option_tags.len(), 3);
    ///
    /// // Remove the 'timer' option tag
    /// supported.remove_tag("timer");
    /// assert!(!supported.supports("timer"));
    /// assert_eq!(supported.option_tags.len(), 2);
    ///
    /// // Removing a non-existent tag has no effect
    /// supported.remove_tag("outbound");
    /// assert_eq!(supported.option_tags.len(), 2);
    /// ```
    pub fn remove_tag(&mut self, tag: &str) {
        self.option_tags.retain(|t| t != tag);
    }
}

impl fmt::Display for Supported {
    /// Formats the Supported header as a string.
    ///
    /// Converts the Supported header to its string representation,
    /// which is a comma-separated list of option tags.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let supported = Supported::new(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string()
    /// ]);
    ///
    /// assert_eq!(supported.to_string(), "timer, 100rel");
    /// assert_eq!(format!("{}", supported), "timer, 100rel");
    ///
    /// // Empty Supported header
    /// let empty = Supported::new(vec![]);
    /// assert_eq!(empty.to_string(), "");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl FromStr for Supported {
    type Err = crate::error::Error;

    /// Parses a string into a Supported header.
    ///
    /// Converts a comma-separated list of option tags into a Supported struct.
    /// Each tag is trimmed to remove any surrounding whitespace.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse as a Supported header
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Supported header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple list
    /// let supported = Supported::from_str("timer, 100rel").unwrap();
    /// assert_eq!(supported.option_tags.len(), 2);
    /// assert!(supported.supports("timer"));
    /// assert!(supported.supports("100rel"));
    ///
    /// // Parse with extra whitespace
    /// let supported = Supported::from_str(" timer , 100rel, path ").unwrap();
    /// assert_eq!(supported.option_tags.len(), 3);
    /// assert!(supported.supports("path"));
    ///
    /// // Parse an empty string
    /// let empty = Supported::from_str("").unwrap();
    /// assert_eq!(empty.option_tags.len(), 0);
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let option_tags = if s.is_empty() {
            Vec::new()
        } else {
            s.split(',')
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .collect()
        };
        
        Ok(Supported { option_tags })
    }
}

impl TypedHeaderTrait for Supported {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Supported` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Supported
    }

    /// Converts this Supported header into a generic Header.
    ///
    /// Creates a Header instance from this Supported header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A generic `Header` containing this Supported header's data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let supported = Supported::new(vec![
    ///     "timer".to_string(),
    ///     "100rel".to_string()
    /// ]);
    ///
    /// let header = supported.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::Supported);
    /// // The header value contains the comma-separated list of tags
    /// ```
    fn to_header(&self) -> Header {
        // Convert option tags to raw bytes
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

    /// Creates a Supported header from a generic Header.
    ///
    /// Converts a generic Header to a Supported instance, if the header
    /// represents a valid Supported header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Supported header, or an error if conversion fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a header with raw value
    /// let header = Header::new(
    ///     HeaderName::Supported,
    ///     HeaderValue::Raw(b"timer, 100rel".to_vec()),
    /// );
    ///
    /// // Convert to Supported
    /// let supported = Supported::from_header(&header).unwrap();
    /// assert_eq!(supported.option_tags.len(), 2);
    /// assert!(supported.supports("timer"));
    /// assert!(supported.supports("100rel"));
    /// ```
    fn from_header(header: &Header) -> Result<Self> {
        match &header.value {
            HeaderValue::Raw(raw) => {
                // Parse raw value using the parser
                let (_, option_tags) = 
                    crate::parser::headers::supported::parse_supported(raw)
                        .map_err(|e| crate::error::Error::ParseError(format!("Failed to parse Supported header: {:?}", e)))?;
                Ok(Self { option_tags })
            },
            HeaderValue::Supported(tags) => {
                // Convert Vec<Vec<u8>> to Vec<String>
                let option_tags = tags.iter()
                    .map(|tag| String::from_utf8_lossy(tag).to_string())
                    .collect();
                Ok(Self { option_tags })
            },
            _ => Err(crate::error::Error::ParseError(
                "Invalid header value type for Supported".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_creation() {
        let supported = Supported::new(vec!["timer".to_string(), "100rel".to_string()]);
        assert_eq!(supported.option_tags.len(), 2);
        assert!(supported.supports("timer"));
        assert!(supported.supports("100rel"));
        assert!(!supported.supports("path"));
    }

    #[test]
    fn test_supported_with_tag() {
        let supported = Supported::with_tag("timer");
        assert_eq!(supported.option_tags.len(), 1);
        assert!(supported.supports("timer"));
    }

    #[test]
    fn test_supported_modification() {
        let mut supported = Supported::with_tag("timer");
        supported.add_tag("100rel");
        assert_eq!(supported.option_tags.len(), 2);
        assert!(supported.supports("100rel"));

        supported.remove_tag("timer");
        assert_eq!(supported.option_tags.len(), 1);
        assert!(!supported.supports("timer"));
        assert!(supported.supports("100rel"));
    }

    #[test]
    fn test_supported_display() {
        let supported = Supported::new(vec!["timer".to_string(), "100rel".to_string()]);
        assert_eq!(format!("{}", supported), "timer, 100rel");
    }

    #[test]
    fn test_supported_fromstr() {
        let supported = Supported::from_str("timer, 100rel").unwrap();
        assert_eq!(supported.option_tags.len(), 2);
        assert!(supported.supports("timer"));
        assert!(supported.supports("100rel"));
        
        // Test with whitespace
        let supported = Supported::from_str(" timer , 100rel ").unwrap();
        assert_eq!(supported.option_tags.len(), 2);
        assert!(supported.supports("timer"));
        assert!(supported.supports("100rel"));
        
        // Test with empty string
        let supported = Supported::from_str("").unwrap();
        assert_eq!(supported.option_tags.len(), 0);
    }

    #[test]
    fn test_supported_to_header() {
        let supported = Supported::new(vec!["timer".to_string(), "100rel".to_string()]);
        let header = supported.to_header();
        assert_eq!(header.name, HeaderName::Supported);
        match &header.value {
            HeaderValue::Raw(raw) => {
                assert_eq!(std::str::from_utf8(raw).unwrap(), "timer, 100rel");
            },
            _ => panic!("Expected HeaderValue::Raw"),
        }
    }

    #[test]
    fn test_supported_from_header() {
        // Create a header with raw value
        let header = Header::new(
            HeaderName::Supported,
            HeaderValue::Raw(b"timer, 100rel".to_vec()),
        );
        
        // Convert to Supported
        let supported = Supported::from_header(&header).unwrap();
        
        // Check conversion
        assert_eq!(supported.option_tags.len(), 2);
        assert_eq!(supported.option_tags[0], "timer");
        assert_eq!(supported.option_tags[1], "100rel");
    }

    #[test]
    fn test_supported_roundtrip() {
        // Create a Supported header
        let original = Supported::new(vec!["timer".to_string(), "100rel".to_string()]);
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to Supported
        let roundtrip = Supported::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn test_supported_empty_roundtrip() {
        // Create an empty Supported header
        let original = Supported::new(vec![]);
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to Supported
        let roundtrip = Supported::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }
} 