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
    pub fn new(option_tags: Vec<String>) -> Self {
        Self { option_tags }
    }

    /// Create a new Supported header with a single option tag
    pub fn with_tag(tag: impl Into<String>) -> Self {
        Self {
            option_tags: vec![tag.into()],
        }
    }

    /// Check if a specific extension is supported
    pub fn supports(&self, tag: &str) -> bool {
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

impl fmt::Display for Supported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.option_tags.join(", "))
    }
}

impl FromStr for Supported {
    type Err = crate::error::Error;

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

    fn header_name() -> Self::Name {
        HeaderName::Supported
    }

    fn to_header(&self) -> Header {
        // Convert option tags to raw bytes
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

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