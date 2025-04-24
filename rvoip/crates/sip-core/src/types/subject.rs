use crate::error::Result;
use crate::types::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

/// Subject header (RFC 3261 Section 20.38)
///
/// The Subject header field provides a summary or indicates the nature
/// of the call, allowing call filtering without having to parse the
/// session description.  The session description does not have to use
/// the same subject indication as the invitation.
///
/// Example:
///   Subject: Project X Discussion
///   s: Lunch Plans
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject(pub String);

impl Subject {
    /// Create a new Subject header with the given text
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Check if the subject is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the subject text
    pub fn text(&self) -> &str {
        &self.0
    }

    /// Set the subject text
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.0 = text.into();
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Subject {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(Subject(s.to_string()))
    }
}

impl TypedHeaderTrait for Subject {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Subject
    }

    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.0.as_bytes().to_vec()),
        )
    }

    fn from_header(header: &Header) -> Result<Self> {
        match &header.value {
            HeaderValue::Raw(raw) => {
                // Parse raw value using the parser - it now returns Subject directly
                let (_, subject) = 
                    crate::parser::headers::subject::parse_subject(raw)
                        .map_err(|e| crate::error::Error::ParseError(format!("Failed to parse Subject header: {:?}", e)))?;
                Ok(subject)
            },
            _ => Err(crate::error::Error::ParseError(
                "Invalid header value type for Subject".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subject_creation() {
        let subject = Subject::new("Project Discussion");
        assert_eq!(subject.text(), "Project Discussion");
        assert!(!subject.is_empty());
    }

    #[test]
    fn test_subject_empty() {
        let subject = Subject::new("");
        assert_eq!(subject.text(), "");
        assert!(subject.is_empty());
    }

    #[test]
    fn test_subject_modification() {
        let mut subject = Subject::new("Original Subject");
        subject.set_text("Modified Subject");
        assert_eq!(subject.text(), "Modified Subject");
    }

    #[test]
    fn test_subject_display() {
        let subject = Subject::new("Test Subject");
        assert_eq!(format!("{}", subject), "Test Subject");
    }

    #[test]
    fn test_subject_fromstr() {
        let subject = Subject::from_str("Parsed Subject").unwrap();
        assert_eq!(subject.text(), "Parsed Subject");
    }

    #[test]
    fn test_subject_to_header() {
        let subject = Subject::new("Test Subject");
        let header = subject.to_header();
        assert_eq!(header.name, HeaderName::Subject);
        match &header.value {
            HeaderValue::Raw(raw) => {
                assert_eq!(std::str::from_utf8(raw).unwrap(), "Test Subject");
            },
            _ => panic!("Expected HeaderValue::Raw"),
        }
    }

    #[test]
    fn test_subject_from_header() {
        // Create a header with raw value
        let header = Header::new(
            HeaderName::Subject,
            HeaderValue::Raw(b"Test Subject".to_vec()),
        );
        
        // Convert to Subject
        let subject = Subject::from_header(&header).unwrap();
        
        // Check conversion
        assert_eq!(subject.text(), "Test Subject");
    }

    #[test]
    fn test_subject_roundtrip() {
        // Create a Subject
        let original = Subject::new("Project X Discussion");
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to Subject
        let roundtrip = Subject::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn test_subject_empty_roundtrip() {
        // Create an empty Subject
        let original = Subject::new("");
        
        // Convert to header
        let header = original.to_header();
        
        // Convert back to Subject
        let roundtrip = Subject::from_header(&header).unwrap();
        
        // Check equality
        assert_eq!(original, roundtrip);
    }
} 