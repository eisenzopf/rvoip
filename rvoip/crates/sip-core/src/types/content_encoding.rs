//! # SIP Content-Encoding Header
//!
//! This module provides an implementation of the SIP Content-Encoding header as defined in
//! [RFC 3261 Section 20.12](https://datatracker.ietf.org/doc/html/rfc3261#section-20.12).
//!
//! The Content-Encoding header field is used as a modifier to the "media-type". When present,
//! its value indicates what additional content coding has been applied to the message body, and
//! thus what decoding mechanism must be applied in order to obtain the media-type referenced by
//! the Content-Type header field.
//!
//! Content-Encoding is primarily used to allow a body to be compressed using some algorithm
//! without losing the identity of its underlying media type.
//!
//! ## Format
//!
//! ```text
//! Content-Encoding: gzip
//! Content-Encoding: gzip, deflate
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::ContentEncoding;
//! use std::str::FromStr;
//!
//! // Create a Content-Encoding header
//! let mut content_encoding = ContentEncoding::new();
//! content_encoding.add_encoding("gzip");
//!
//! // Parse from a string
//! let content_encoding = ContentEncoding::from_str("gzip, deflate").unwrap();
//! assert!(content_encoding.has_encoding("gzip"));
//! assert!(content_encoding.has_encoding("deflate"));
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use nom::combinator::all_consuming;
use nom::error::ErrorKind as NomErrorKind;

/// Represents the Content-Encoding header field (RFC 3261 Section 20.12).
///
/// The Content-Encoding header field is used to indicate any additional content codings
/// that have been applied to the message body, beyond those implied by the Content-Type.
/// When present, this header field indicates what decoding mechanism must be applied to
/// obtain the media type referenced by the Content-Type header field.
///
/// Content-Encoding values are case-insensitive and the order of encodings is significant.
/// That is, the interpretation is that the body has been encoded with the first encoding,
/// then the second, and so on.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::ContentEncoding;
/// use std::str::FromStr;
///
/// // Create a Content-Encoding header
/// let mut content_encoding = ContentEncoding::new();
/// content_encoding.add_encoding("gzip");
/// content_encoding.add_encoding("deflate");
///
/// // Check if an encoding is included
/// assert!(content_encoding.has_encoding("gzip"));
/// assert!(content_encoding.has_encoding("deflate"));
///
/// // Convert to a string
/// assert_eq!(content_encoding.to_string(), "gzip, deflate");
///
/// // Parse from a string
/// let content_encoding = ContentEncoding::from_str("gzip").unwrap();
/// assert!(content_encoding.has_encoding("gzip"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentEncoding {
    /// List of encodings in order of application
    encodings: Vec<String>,
}

impl ContentEncoding {
    /// Creates a new empty Content-Encoding header.
    ///
    /// # Returns
    ///
    /// A new empty `ContentEncoding` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::new();
    /// assert!(content_encoding.encodings().is_empty());
    /// ```
    pub fn new() -> Self {
        ContentEncoding {
            encodings: Vec::new(),
        }
    }

    /// Creates a Content-Encoding header with a single encoding.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding to include
    ///
    /// # Returns
    ///
    /// A new `ContentEncoding` instance with the specified encoding
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::single("gzip");
    /// assert!(content_encoding.has_encoding("gzip"));
    /// ```
    pub fn single(encoding: &str) -> Self {
        ContentEncoding {
            encodings: vec![encoding.to_string()],
        }
    }

    /// Creates a Content-Encoding header with multiple encodings.
    ///
    /// # Parameters
    ///
    /// - `encodings`: A slice of encodings to include
    ///
    /// # Returns
    ///
    /// A new `ContentEncoding` instance with the specified encodings
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
    /// assert!(content_encoding.has_encoding("gzip"));
    /// assert!(content_encoding.has_encoding("deflate"));
    /// ```
    pub fn with_encodings<T: AsRef<str>>(encodings: &[T]) -> Self {
        ContentEncoding {
            encodings: encodings.iter().map(|e| e.as_ref().to_string()).collect(),
        }
    }

    /// Adds an encoding to the list.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let mut content_encoding = ContentEncoding::new();
    /// content_encoding.add_encoding("gzip");
    /// assert!(content_encoding.has_encoding("gzip"));
    /// ```
    pub fn add_encoding(&mut self, encoding: &str) {
        self.encodings.push(encoding.to_string());
    }

    /// Removes an encoding from the list.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let mut content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
    /// content_encoding.remove_encoding("gzip");
    /// assert!(!content_encoding.has_encoding("gzip"));
    /// assert!(content_encoding.has_encoding("deflate"));
    /// ```
    pub fn remove_encoding(&mut self, encoding: &str) {
        self.encodings.retain(|e| !e.eq_ignore_ascii_case(encoding));
    }

    /// Checks if an encoding is included in the list.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding to check for
    ///
    /// # Returns
    ///
    /// `true` if the encoding is included, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
    /// assert!(content_encoding.has_encoding("gzip"));
    /// assert!(content_encoding.has_encoding("Deflate")); // Case-insensitive
    /// assert!(!content_encoding.has_encoding("compress"));
    /// ```
    pub fn has_encoding(&self, encoding: &str) -> bool {
        self.encodings.iter().any(|e| e.eq_ignore_ascii_case(encoding))
    }

    /// Returns the list of encodings.
    ///
    /// # Returns
    ///
    /// A slice containing all encodings in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
    /// let encodings = content_encoding.encodings();
    /// assert_eq!(encodings.len(), 2);
    /// assert_eq!(encodings[0], "gzip");
    /// assert_eq!(encodings[1], "deflate");
    /// ```
    pub fn encodings(&self) -> &[String] {
        &self.encodings
    }

    /// Checks if the list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the list contains no encodings, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ContentEncoding;
    ///
    /// let content_encoding = ContentEncoding::new();
    /// assert!(content_encoding.is_empty());
    ///
    /// let content_encoding = ContentEncoding::single("gzip");
    /// assert!(!content_encoding.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.encodings.is_empty()
    }
}

impl fmt::Display for ContentEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.encodings.join(", "))
    }
}

impl Default for ContentEncoding {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for ContentEncoding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "Content-Encoding:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Content-Encoding header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Empty string is a valid Content-Encoding (means no encoding)
        if value_str.is_empty() {
            return Ok(ContentEncoding::new());
        }
        
        // Split the string by commas and collect encodings
        let encodings = value_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
            
        Ok(ContentEncoding { encodings })
    }
}

// Implement TypedHeaderTrait for ContentEncoding
impl TypedHeaderTrait for ContentEncoding {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ContentEncoding
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
                    ContentEncoding::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::ContentEncoding(tokens) => {
                let encodings = tokens
                    .iter()
                    .filter_map(|token| {
                        std::str::from_utf8(token).ok().map(|s| s.to_string())
                    })
                    .collect();
                Ok(ContentEncoding { encodings })
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
        let content_encoding = ContentEncoding::new();
        assert!(content_encoding.is_empty());
        assert_eq!(content_encoding.to_string(), "");
    }
    
    #[test]
    fn test_single() {
        let content_encoding = ContentEncoding::single("gzip");
        assert_eq!(content_encoding.encodings().len(), 1);
        assert_eq!(content_encoding.encodings()[0], "gzip");
        assert_eq!(content_encoding.to_string(), "gzip");
    }
    
    #[test]
    fn test_with_encodings() {
        let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
        assert_eq!(content_encoding.encodings().len(), 2);
        assert_eq!(content_encoding.encodings()[0], "gzip");
        assert_eq!(content_encoding.encodings()[1], "deflate");
        assert_eq!(content_encoding.to_string(), "gzip, deflate");
    }
    
    #[test]
    fn test_add_remove_encoding() {
        let mut content_encoding = ContentEncoding::new();
        
        // Add encodings
        content_encoding.add_encoding("gzip");
        content_encoding.add_encoding("deflate");
        
        assert_eq!(content_encoding.encodings().len(), 2);
        assert!(content_encoding.has_encoding("gzip"));
        assert!(content_encoding.has_encoding("deflate"));
        
        // Remove an encoding
        content_encoding.remove_encoding("gzip");
        
        assert_eq!(content_encoding.encodings().len(), 1);
        assert!(!content_encoding.has_encoding("gzip"));
        assert!(content_encoding.has_encoding("deflate"));
    }
    
    #[test]
    fn test_has_encoding() {
        let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
        
        // Check case-insensitive matching
        assert!(content_encoding.has_encoding("gzip"));
        assert!(content_encoding.has_encoding("GZIP"));
        assert!(content_encoding.has_encoding("Deflate"));
        
        // Check non-existent encoding
        assert!(!content_encoding.has_encoding("compress"));
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let content_encoding: ContentEncoding = "gzip".parse().unwrap();
        assert_eq!(content_encoding.encodings().len(), 1);
        assert_eq!(content_encoding.encodings()[0], "gzip");
        
        // Multiple encodings
        let content_encoding: ContentEncoding = "gzip, deflate".parse().unwrap();
        assert_eq!(content_encoding.encodings().len(), 2);
        assert_eq!(content_encoding.encodings()[0], "gzip");
        assert_eq!(content_encoding.encodings()[1], "deflate");
        
        // With header name
        let content_encoding: ContentEncoding = "Content-Encoding: gzip, deflate".parse().unwrap();
        assert_eq!(content_encoding.encodings().len(), 2);
        assert_eq!(content_encoding.encodings()[0], "gzip");
        assert_eq!(content_encoding.encodings()[1], "deflate");
        
        // Empty
        let content_encoding: ContentEncoding = "".parse().unwrap();
        assert!(content_encoding.is_empty());
        
        // Empty with header name
        let content_encoding: ContentEncoding = "Content-Encoding:".parse().unwrap();
        assert!(content_encoding.is_empty());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let content_encoding = ContentEncoding::with_encodings(&["gzip", "deflate"]);
        let header = content_encoding.to_header();
        
        assert_eq!(header.name, HeaderName::ContentEncoding);
        
        // Convert back from Header
        let content_encoding2 = ContentEncoding::from_header(&header).unwrap();
        assert_eq!(content_encoding.encodings().len(), content_encoding2.encodings().len());
        assert_eq!(content_encoding.encodings()[0], content_encoding2.encodings()[0]);
        assert_eq!(content_encoding.encodings()[1], content_encoding2.encodings()[1]);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(ContentEncoding::from_header(&wrong_header).is_err());
    }
} 