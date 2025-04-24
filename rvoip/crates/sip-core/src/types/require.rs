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
/// Example:
///   Require: 100rel, precondition
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Require {
    /// List of option-tags required by the client
    pub option_tags: Vec<String>,
}

impl Require {
    /// Create a new Require header with the given option tags
    pub fn new(option_tags: Vec<String>) -> Self {
        Self { option_tags }
    }

    /// Create a new Require header with a single option tag
    pub fn with_tag(tag: impl Into<String>) -> Self {
        Self {
            option_tags: vec![tag.into()],
        }
    }

    /// Check if a specific tag is required
    pub fn requires(&self, tag: &str) -> bool {
        self.option_tags.iter().any(|t| t == tag)
    }

    /// Add a new option tag
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.option_tags.push(tag.into());
    }

    /// Remove an option tag if it exists
    pub fn remove_tag(&mut self, tag: &str) {
        self.option_tags.retain(|t| t != tag);
    }
}

impl fmt::Display for Require {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl TypedHeaderTrait for Require {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Require
    }

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