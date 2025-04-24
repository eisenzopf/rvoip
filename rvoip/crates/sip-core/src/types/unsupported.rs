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
/// ```
/// use rvoip_sip_core::types::Unsupported;
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
    pub fn new() -> Self {
        Self {
            option_tags: Vec::new(),
        }
    }

    /// Create an Unsupported header with the given option tags
    pub fn with_tags(tags: Vec<String>) -> Self {
        Self {
            option_tags: tags,
        }
    }

    /// Check if this Unsupported header contains a specific option tag
    pub fn has_option_tag(&self, tag: &str) -> bool {
        self.option_tags.iter().any(|t| t == tag)
    }

    /// Add an option tag to this Unsupported header
    pub fn add_option_tag(&mut self, tag: &str) {
        if !self.has_option_tag(tag) {
            self.option_tags.push(tag.to_string());
        }
    }

    /// Remove an option tag from this Unsupported header
    pub fn remove_option_tag(&mut self, tag: &str) {
        self.option_tags.retain(|t| t != tag);
    }

    /// Get all option tags in this Unsupported header
    pub fn option_tags(&self) -> &[String] {
        &self.option_tags
    }
}

impl Default for Unsupported {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.option_tags.is_empty() {
            return Ok(());
        }

        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl FromStr for Unsupported {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let input = s.as_bytes();
        match parser::headers::unsupported::parse_unsupported(input) {
            Ok((_, tags)) => Ok(Unsupported::with_tags(tags)),
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl TypedHeaderTrait for Unsupported {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::Unsupported
    }

    fn to_header(&self) -> Header {
        let tags_bytes: Vec<Vec<u8>> = self.option_tags
            .iter()
            .map(|tag| tag.as_bytes().to_vec())
            .collect();

        Header {
            name: HeaderName::Unsupported,
            value: HeaderValue::Unsupported(tags_bytes),
        }
    }

    fn from_header(header: &Header) -> std::result::Result<Self, Error> {
        match &header.value {
            HeaderValue::Unsupported(tags) => {
                let option_tags = tags
                    .iter()
                    .filter_map(|tag| String::from_utf8(tag.clone()).ok())
                    .collect();
                
                Ok(Unsupported {
                    option_tags,
                })
            }
            _ => Err(Error::InvalidHeader("Expected Unsupported header".to_string())),
        }
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
        let mut unsupported = Unsupported::new();
        unsupported.add_option_tag("timer");
        unsupported.add_option_tag("100rel");
        
        let header = unsupported.to_header();
        assert_eq!(header.name, HeaderName::Unsupported);
        
        match &header.value {
            HeaderValue::Unsupported(tags) => {
                assert_eq!(tags.len(), 2);
                assert_eq!(tags[0], b"timer".to_vec());
                assert_eq!(tags[1], b"100rel".to_vec());
            },
            _ => panic!("Expected HeaderValue::Unsupported"),
        }
    }

    #[test]
    fn test_unsupported_from_header() {
        let tags = vec![b"timer".to_vec(), b"100rel".to_vec()];
        let header = Header {
            name: HeaderName::Unsupported,
            value: HeaderValue::Unsupported(tags),
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
} 