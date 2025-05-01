//! # SIP Content-Language Header
//!
//! This module provides an implementation of the SIP Content-Language header as defined in
//! [RFC 3261 Section 20.13](https://datatracker.ietf.org/doc/html/rfc3261#section-20.13).
//!
//! The Content-Language header field is used to indicate the language of the message body.
//! The syntax and semantics are identical to the HTTP Content-Language header field as defined
//! in [RFC 2616 Section 14.12](https://datatracker.ietf.org/doc/html/rfc2616#section-14.12).
//!
//! If no Content-Language header field is present, the default is that the content is intended
//! for all users, regardless of language preference.
//!
//! ## Format
//!
//! ```text
//! Content-Language: en
//! Content-Language: en-US
//! Content-Language: en-US, fr-CA
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::ContentLanguage;
//! use std::str::FromStr;
//!
//! // Create a Content-Language header
//! let mut content_language = ContentLanguage::new();
//! content_language.add_language("en-US");
//!
//! // Parse from a string
//! let content_language = ContentLanguage::from_str("en-US, fr-CA").unwrap();
//! assert!(content_language.has_language("en-US"));
//! assert!(content_language.has_language("fr-CA"));
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Content-Language header field (RFC 3261 Section 20.13).
///
/// The Content-Language header field is used to indicate the language of the message body.
/// It can contain one or more language tags, as defined in RFC 1766 (e.g., "en", "en-US", "fr").
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::ContentLanguage;
/// use std::str::FromStr;
///
/// // Create a Content-Language header
/// let mut content_language = ContentLanguage::new();
/// content_language.add_language("en-US");
/// content_language.add_language("fr-CA");
///
/// // Check if a language is included
/// assert!(content_language.has_language("en-US"));
/// assert!(content_language.has_language("fr-CA"));
///
/// // Convert to a string
/// assert_eq!(content_language.to_string(), "en-US, fr-CA");
///
/// // Parse from a string
/// let content_language = ContentLanguage::from_str("en, fr").unwrap();
/// assert!(content_language.has_language("en"));
/// assert!(content_language.has_language("fr"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentLanguage {
    /// List of language tags
    languages: Vec<String>,
}

impl ContentLanguage {
    /// Creates a new empty Content-Language header.
    ///
    /// # Returns
    ///
    /// A new empty `ContentLanguage` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::new();
    /// assert!(content_language.languages().is_empty());
    /// ```
    pub fn new() -> Self {
        ContentLanguage {
            languages: Vec::new(),
        }
    }

    /// Creates a Content-Language header with a single language.
    ///
    /// # Parameters
    ///
    /// - `language`: The language tag to include
    ///
    /// # Returns
    ///
    /// A new `ContentLanguage` instance with the specified language
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::single("en-US");
    /// assert!(content_language.has_language("en-US"));
    /// ```
    pub fn single(language: &str) -> Self {
        ContentLanguage {
            languages: vec![language.to_string()],
        }
    }

    /// Creates a Content-Language header with multiple languages.
    ///
    /// # Parameters
    ///
    /// - `languages`: A slice of language tags to include
    ///
    /// # Returns
    ///
    /// A new `ContentLanguage` instance with the specified languages
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
    /// assert!(content_language.has_language("en-US"));
    /// assert!(content_language.has_language("fr-CA"));
    /// ```
    pub fn with_languages<T: AsRef<str>>(languages: &[T]) -> Self {
        ContentLanguage {
            languages: languages.iter().map(|e| e.as_ref().to_string()).collect(),
        }
    }

    /// Adds a language tag to the list.
    ///
    /// # Parameters
    ///
    /// - `language`: The language tag to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let mut content_language = ContentLanguage::new();
    /// content_language.add_language("en-US");
    /// assert!(content_language.has_language("en-US"));
    /// ```
    pub fn add_language(&mut self, language: &str) {
        self.languages.push(language.to_string());
    }

    /// Removes a language tag from the list.
    ///
    /// # Parameters
    ///
    /// - `language`: The language tag to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let mut content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
    /// content_language.remove_language("en-US");
    /// assert!(!content_language.has_language("en-US"));
    /// assert!(content_language.has_language("fr-CA"));
    /// ```
    pub fn remove_language(&mut self, language: &str) {
        self.languages.retain(|e| !e.eq_ignore_ascii_case(language));
    }

    /// Checks if a language tag is included in the list.
    ///
    /// # Parameters
    ///
    /// - `language`: The language tag to check for
    ///
    /// # Returns
    ///
    /// `true` if the language is included, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
    /// assert!(content_language.has_language("en-US"));
    /// assert!(content_language.has_language("FR-ca")); // Case-insensitive
    /// assert!(!content_language.has_language("de"));
    /// ```
    pub fn has_language(&self, language: &str) -> bool {
        self.languages.iter().any(|e| e.eq_ignore_ascii_case(language))
    }

    /// Returns the list of language tags.
    ///
    /// # Returns
    ///
    /// A slice containing all language tags in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
    /// let languages = content_language.languages();
    /// assert_eq!(languages.len(), 2);
    /// assert_eq!(languages[0], "en-US");
    /// assert_eq!(languages[1], "fr-CA");
    /// ```
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// Checks if the list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the list contains no language tags, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentLanguage;
    ///
    /// let content_language = ContentLanguage::new();
    /// assert!(content_language.is_empty());
    ///
    /// let content_language = ContentLanguage::single("en");
    /// assert!(!content_language.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }
}

impl fmt::Display for ContentLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.languages.join(", "))
    }
}

impl Default for ContentLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for ContentLanguage {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "Content-Language:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Content-Language header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Empty string is a valid Content-Language (means no specific language)
        if value_str.is_empty() {
            return Ok(ContentLanguage::new());
        }
        
        // Split the string by commas and collect language tags
        let languages = value_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
            
        Ok(ContentLanguage { languages })
    }
}

// Implement TypedHeaderTrait for ContentLanguage
impl TypedHeaderTrait for ContentLanguage {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ContentLanguage
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    ContentLanguage::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::ContentLanguage(tokens) => {
                let languages = tokens
                    .iter()
                    .filter_map(|token| {
                        std::str::from_utf8(token).ok().map(|s| s.to_string())
                    })
                    .collect();
                Ok(ContentLanguage { languages })
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let content_language = ContentLanguage::new();
        assert!(content_language.is_empty());
        assert_eq!(content_language.to_string(), "");
    }
    
    #[test]
    fn test_single() {
        let content_language = ContentLanguage::single("en-US");
        assert_eq!(content_language.languages().len(), 1);
        assert_eq!(content_language.languages()[0], "en-US");
        assert_eq!(content_language.to_string(), "en-US");
    }
    
    #[test]
    fn test_with_languages() {
        let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
        assert_eq!(content_language.languages().len(), 2);
        assert_eq!(content_language.languages()[0], "en-US");
        assert_eq!(content_language.languages()[1], "fr-CA");
        assert_eq!(content_language.to_string(), "en-US, fr-CA");
    }
    
    #[test]
    fn test_add_remove_language() {
        let mut content_language = ContentLanguage::new();
        
        // Add languages
        content_language.add_language("en-US");
        content_language.add_language("fr-CA");
        
        assert_eq!(content_language.languages().len(), 2);
        assert!(content_language.has_language("en-US"));
        assert!(content_language.has_language("fr-CA"));
        
        // Remove a language
        content_language.remove_language("en-US");
        
        assert_eq!(content_language.languages().len(), 1);
        assert!(!content_language.has_language("en-US"));
        assert!(content_language.has_language("fr-CA"));
    }
    
    #[test]
    fn test_has_language() {
        let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
        
        // Check case-insensitive matching
        assert!(content_language.has_language("en-US"));
        assert!(content_language.has_language("EN-us"));
        assert!(content_language.has_language("fr-CA"));
        
        // Check non-existent language
        assert!(!content_language.has_language("de"));
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let content_language: ContentLanguage = "en-US".parse().unwrap();
        assert_eq!(content_language.languages().len(), 1);
        assert_eq!(content_language.languages()[0], "en-US");
        
        // Multiple languages
        let content_language: ContentLanguage = "en-US, fr-CA".parse().unwrap();
        assert_eq!(content_language.languages().len(), 2);
        assert_eq!(content_language.languages()[0], "en-US");
        assert_eq!(content_language.languages()[1], "fr-CA");
        
        // With header name
        let content_language: ContentLanguage = "Content-Language: en-US, fr-CA".parse().unwrap();
        assert_eq!(content_language.languages().len(), 2);
        assert_eq!(content_language.languages()[0], "en-US");
        assert_eq!(content_language.languages()[1], "fr-CA");
        
        // Empty
        let content_language: ContentLanguage = "".parse().unwrap();
        assert!(content_language.is_empty());
        
        // Empty with header name
        let content_language: ContentLanguage = "Content-Language:".parse().unwrap();
        assert!(content_language.is_empty());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let content_language = ContentLanguage::with_languages(&["en-US", "fr-CA"]);
        let header = content_language.to_header();
        
        assert_eq!(header.name, HeaderName::ContentLanguage);
        
        // Convert back from Header
        let content_language2 = ContentLanguage::from_header(&header).unwrap();
        assert_eq!(content_language.languages().len(), content_language2.languages().len());
        assert_eq!(content_language.languages()[0], content_language2.languages()[0]);
        assert_eq!(content_language.languages()[1], content_language2.languages()[1]);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(ContentLanguage::from_header(&wrong_header).is_err());
    }
} 