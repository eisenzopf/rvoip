//! # SIP Subject Header
//!
//! This module provides an implementation of the SIP Subject header as defined in
//! [RFC 3261 Section 20.36](https://datatracker.ietf.org/doc/html/rfc3261#section-20.36).
//!
//! The Subject header field (also called by the short form "s") provides a summary
//! or indicates the nature of the call. It allows call filtering without having to
//! parse the session description. This header is primarily informational and is not
//! crucial for SIP message processing.
//!
//! ## Format
//!
//! ```
//! Subject: Project X Discussion
//! s: Lunch Plans
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Subject header
//! let subject = Subject::new("Team Meeting");
//! assert_eq!(subject.text(), "Team Meeting");
//!
//! // Parse from a string
//! let subject = Subject::from_str("Project Discussion").unwrap();
//! assert_eq!(subject.to_string(), "Project Discussion");
//! ```

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
/// The Subject header is optional in SIP messages and is typically included
/// in INVITE requests to provide context about the purpose of the call.
/// It can also be abbreviated as "s" in SIP messages.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a new Subject header
/// let subject = Subject::new("Weekly Team Sync");
/// 
/// // Access the subject text
/// assert_eq!(subject.text(), "Weekly Team Sync");
/// 
/// // Convert to string for inclusion in a SIP message
/// assert_eq!(subject.to_string(), "Weekly Team Sync");
/// ```
///
/// Example:
///   Subject: Project X Discussion
///   s: Lunch Plans
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject(pub String);

impl Subject {
    /// Create a new Subject header with the given text
    ///
    /// Initializes a new Subject header using the provided text string.
    /// The text can be any UTF-8 encoded string and represents the subject
    /// or nature of the call.
    ///
    /// # Parameters
    ///
    /// - `text`: The subject text, can be any type that can be converted into a String
    ///
    /// # Returns
    ///
    /// A new `Subject` instance containing the specified text
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create with a static string
    /// let subject = Subject::new("Project Discussion");
    ///
    /// // Create with a String
    /// let text = String::from("Conference Call");
    /// let subject = Subject::new(text);
    ///
    /// // Create with an empty subject
    /// let empty_subject = Subject::new("");
    /// assert!(empty_subject.is_empty());
    /// ```
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Check if the subject is empty
    ///
    /// Tests whether the Subject header contains any text.
    ///
    /// # Returns
    ///
    /// `true` if the subject text is empty, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let empty_subject = Subject::new("");
    /// assert!(empty_subject.is_empty());
    ///
    /// let subject = Subject::new("Not Empty");
    /// assert!(!subject.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the subject text
    ///
    /// Returns a reference to the subject text string.
    ///
    /// # Returns
    ///
    /// A string slice containing the subject text
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let subject = Subject::new("Team Meeting");
    /// assert_eq!(subject.text(), "Team Meeting");
    ///
    /// // Use the text in a custom message
    /// let message = format!("Call subject: {}", subject.text());
    /// assert_eq!(message, "Call subject: Team Meeting");
    /// ```
    pub fn text(&self) -> &str {
        &self.0
    }

    /// Set the subject text
    ///
    /// Updates the subject text with a new value.
    ///
    /// # Parameters
    ///
    /// - `text`: The new subject text, can be any type that can be converted into a String
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut subject = Subject::new("Original Subject");
    /// assert_eq!(subject.text(), "Original Subject");
    ///
    /// // Update the subject text
    /// subject.set_text("Updated Subject");
    /// assert_eq!(subject.text(), "Updated Subject");
    ///
    /// // Set to empty
    /// subject.set_text("");
    /// assert!(subject.is_empty());
    /// ```
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.0 = text.into();
    }
}

impl fmt::Display for Subject {
    /// Formats the Subject header as a string.
    ///
    /// Converts the Subject header to its string representation,
    /// which is simply the subject text.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let subject = Subject::new("Team Meeting");
    /// assert_eq!(subject.to_string(), "Team Meeting");
    /// assert_eq!(format!("{}", subject), "Team Meeting");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Subject {
    type Err = crate::error::Error;

    /// Parses a string into a Subject header.
    ///
    /// Since the Subject header is simply text, this just wraps the input
    /// string in a Subject struct. It will never fail unless the string
    /// itself is invalid UTF-8, which is handled by the Rust string type.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse as a Subject header
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Subject, which will always be Ok
    /// for valid UTF-8 strings
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let subject = Subject::from_str("Team Meeting").unwrap();
    /// assert_eq!(subject.text(), "Team Meeting");
    ///
    /// // Parse an empty subject
    /// let empty = Subject::from_str("").unwrap();
    /// assert!(empty.is_empty());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        Ok(Subject(s.to_string()))
    }
}

impl TypedHeaderTrait for Subject {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Subject` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Subject
    }

    /// Converts this Subject header into a generic Header.
    ///
    /// Creates a Header instance from this Subject, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A generic `Header` containing this Subject header's data
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let subject = Subject::new("Team Meeting");
    /// let header = subject.to_header();
    ///
    /// assert_eq!(header.name, HeaderName::Subject);
    /// ```
    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.0.as_bytes().to_vec()),
        )
    }

    /// Creates a Subject header from a generic Header.
    ///
    /// Converts a generic Header to a Subject instance, if the header
    /// represents a valid Subject header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Subject header, or an error if conversion fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a header with raw value
    /// let header = Header::new(
    ///     HeaderName::Subject,
    ///     HeaderValue::Raw(b"Team Meeting".to_vec()),
    /// );
    ///
    /// // Convert to Subject
    /// let subject = Subject::from_header(&header).unwrap();
    /// assert_eq!(subject.text(), "Team Meeting");
    /// ```
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