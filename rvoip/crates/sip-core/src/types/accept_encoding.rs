//! # SIP Accept-Encoding Header
//! 
//! This module provides an implementation of the SIP Accept-Encoding header field as defined in
//! [RFC 3261 Section 20.2](https://datatracker.ietf.org/doc/html/rfc3261#section-20.2).
//!
//! The Accept-Encoding header field is used to indicate what response content encodings are acceptable
//! to the client. The encoding is a property of the message, not of the body. The syntax and semantics
//! are identical to HTTP Accept-Encoding header field as defined in
//! [RFC 2616 Section 14.3](https://datatracker.ietf.org/doc/html/rfc2616#section-14.3).
//!
//! If no Accept-Encoding header field is present, the server SHOULD assume a default value of
//! "identity", i.e., no compression or encoding is permitted.
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::AcceptEncoding;
//! use std::str::FromStr;
//!
//! // Parse an Accept-Encoding header
//! let header = AcceptEncoding::from_str("gzip;q=1.0, identity; q=0.5, *;q=0").unwrap();
//!
//! // Check if an encoding is acceptable
//! assert!(header.accepts("gzip"));
//! assert!(!header.accepts("compress"));
//!
//! // Format as a string
//! assert_eq!(header.to_string(), "gzip;q=1.000, identity;q=0.500, *;q=0.000");
//! ```

use crate::parser::headers::accept_encoding::{EncodingInfo, parse_accept_encoding};
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Accept-Encoding header field (RFC 3261 Section 20.2).
///
/// The Accept-Encoding header indicates what response content encodings are acceptable
/// to the client. It contains a prioritized list of encoding tokens, each potentially with
/// a quality value ("q-value") that indicates its relative preference (from 0.0 to 1.0,
/// with 1.0 being the default and highest priority).
///
/// As per RFC 3261, if this header is not present in a request, the server should assume
/// a default value of "identity", i.e., no compression or encoding is permitted.
///
/// # Encoding matching
///
/// This implementation follows the encoding matching rules outlined in RFC 2616:
///
/// - Encodings are matched case-insensitively
/// - A wildcard (`*`) matches any encoding
/// - Encodings with higher q-values are preferred over encodings with lower q-values
/// - Encodings with the same q-value are ordered by their original order in the header
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::AcceptEncoding;
/// use std::str::FromStr;
///
/// // Create from a header string
/// let header = AcceptEncoding::from_str("gzip;q=1.0, identity; q=0.5, *;q=0").unwrap();
///
/// // Check if an encoding is acceptable
/// assert!(header.accepts("gzip"));
/// assert!(header.accepts("identity")); 
/// assert!(!header.accepts("compress")); // q=0 for * means not acceptable
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptEncoding(pub Vec<EncodingInfo>);

impl AcceptEncoding {
    /// Creates an empty Accept-Encoding header.
    ///
    /// An empty Accept-Encoding header means the default encoding "identity" is acceptable,
    /// according to RFC 3261 Section 20.2.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    ///
    /// let header = AcceptEncoding::new();
    /// assert!(header.accepts("identity"));
    /// ```
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Accept-Encoding header with specified capacity.
    ///
    /// This is useful when you know approximately how many encodings
    /// you'll be adding to avoid reallocations.
    ///
    /// # Parameters
    ///
    /// - `capacity`: The initial capacity for the encodings vector
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    ///
    /// let mut header = AcceptEncoding::with_capacity(3);
    /// // Can now add up to 3 encodings without reallocation
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Accept-Encoding header from an iterator of encoding info items.
    ///
    /// # Parameters
    ///
    /// - `encodings`: An iterator yielding `EncodingInfo` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    /// use rvoip_sip_core::parser::headers::accept_encoding::EncodingInfo;
    /// use ordered_float::NotNan;
    ///
    /// let gzip = EncodingInfo {
    ///     coding: "gzip".to_string(),
    ///     q: Some(NotNan::new(1.0).unwrap()),
    ///     params: vec![],
    /// };
    ///
    /// let identity = EncodingInfo {
    ///     coding: "identity".to_string(),
    ///     q: Some(NotNan::new(0.5).unwrap()),
    ///     params: vec![],
    /// };
    ///
    /// let header = AcceptEncoding::from_encodings(vec![gzip, identity]);
    /// assert_eq!(header.encodings().len(), 2);
    /// ```
    pub fn from_encodings<I>(encodings: I) -> Self
    where
        I: IntoIterator<Item = EncodingInfo>
    {
        Self(encodings.into_iter().collect())
    }

    /// Adds an encoding to the list.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding info to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    /// use rvoip_sip_core::parser::headers::accept_encoding::EncodingInfo;
    ///
    /// let mut header = AcceptEncoding::new();
    /// let gzip = EncodingInfo {
    ///     coding: "gzip".to_string(),
    ///     q: None,
    ///     params: vec![],
    /// };
    /// header.push(gzip);
    /// assert_eq!(header.encodings().len(), 1);
    /// ```
    pub fn push(&mut self, encoding: EncodingInfo) {
        self.0.push(encoding);
    }

    /// Returns the list of encodings in this header.
    ///
    /// # Returns
    ///
    /// A slice containing all encoding info items in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    /// use std::str::FromStr;
    ///
    /// let header = AcceptEncoding::from_str("gzip;q=0.8, identity").unwrap();
    /// let encodings = header.encodings();
    /// assert_eq!(encodings.len(), 2);
    /// ```
    pub fn encodings(&self) -> &[EncodingInfo] {
        &self.0
    }

    /// Checks if a specific encoding is acceptable.
    ///
    /// Performs a basic encoding match, respecting wildcards and case-insensitivity.
    /// According to RFC 3261, if the header is empty, the default encoding "identity" is acceptable.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding to check (e.g., "gzip", "identity")
    ///
    /// # Returns
    ///
    /// `true` if the encoding is acceptable, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptEncoding;
    /// use std::str::FromStr;
    ///
    /// let header = AcceptEncoding::from_str("gzip;q=1.0, identity; q=0.5, *;q=0").unwrap();
    ///
    /// assert!(header.accepts("gzip"));       // Explicitly allowed with q=1.0
    /// assert!(header.accepts("identity"));   // Explicitly allowed with q=0.5
    /// assert!(!header.accepts("compress"));  // Matched by wildcard with q=0
    /// ```
    pub fn accepts(&self, encoding: &str) -> bool {
        // If empty, accept the default "identity" encoding
        if self.0.is_empty() {
            return encoding.eq_ignore_ascii_case("identity");
        }

        // First check for exact matches
        for encoding_info in &self.0 {
            if encoding_info.coding.eq_ignore_ascii_case(encoding) {
                // Encoding is explicitly mentioned
                // Check if q-value is > 0
                return encoding_info.q_value() > 0.0;
            }
        }
        
        // Then check for wildcard
        for encoding_info in &self.0 {
            if encoding_info.coding == "*" {
                // Wildcard match
                // Check if q-value is > 0
                return encoding_info.q_value() > 0.0;
            }
        }
        
        // Default behavior - if not mentioned and no wildcard, it's not acceptable
        false
    }
}

impl fmt::Display for AcceptEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Create a sorted copy of the encodings
        let mut sorted_encodings = self.0.clone();
        sorted_encodings.sort();
        
        let encoding_strings: Vec<String> = sorted_encodings.iter().map(|enc| enc.to_string()).collect();
        write!(f, "{}", encoding_strings.join(", "))
    }
}

// Helper function to parse from owned bytes
fn parse_from_owned_bytes(bytes: Vec<u8>) -> Result<Vec<EncodingInfo>> {
    match all_consuming(parse_accept_encoding)(bytes.as_slice()) {
        Ok((_, encodings)) => Ok(encodings),
        Err(e) => Err(Error::ParseError(
            format!("Failed to parse Accept-Encoding header: {:?}", e)
        ))
    }
}

impl FromStr for AcceptEncoding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header without the name
        // (e.g., "gzip;q=0.8, identity" instead of "Accept-Encoding: gzip;q=0.8, identity")
        let input_bytes = if !s.contains(':') {
            format!("Accept-Encoding: {}", s).into_bytes()
        } else {
            s.as_bytes().to_vec()
        };
        
        // Parse using our helper function that takes ownership of the bytes
        parse_from_owned_bytes(input_bytes).map(AcceptEncoding)
    }
}

// Implement TypedHeaderTrait for AcceptEncoding
impl TypedHeaderTrait for AcceptEncoding {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::AcceptEncoding
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
                    AcceptEncoding::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::AcceptEncoding(encodings) => {
                Ok(AcceptEncoding(encodings.clone()))
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
    use ordered_float::NotNan;
    use crate::types::param::Param;

    #[test]
    fn test_from_str() {
        // Test with header name
        let header_str = "Accept-Encoding: gzip;q=0.8, identity, compress;q=0.7";
        let accept_enc: AcceptEncoding = header_str.parse().unwrap();
        
        assert_eq!(accept_enc.0.len(), 3);
        assert!(accept_enc.accepts("gzip"));
        assert!(accept_enc.accepts("identity"));
        assert!(accept_enc.accepts("compress"));
        
        // Test without header name
        let value_str = "gzip;q=0.5, *;q=0.1";
        let accept_enc2: AcceptEncoding = value_str.parse().unwrap();
        
        assert_eq!(accept_enc2.0.len(), 2);
        assert!(accept_enc2.accepts("gzip"));
        assert!(accept_enc2.accepts("identity")); // Matched by wildcard
    }
    
    #[test]
    fn test_accepts() {
        // Create test encodings
        let gzip = EncodingInfo {
            coding: "gzip".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let identity = EncodingInfo {
            coding: "identity".to_string(),
            q: None, // Default 1.0
            params: vec![],
        };
        
        let wildcard = EncodingInfo {
            coding: "*".to_string(),
            q: Some(NotNan::new(0.1).unwrap()),
            params: vec![],
        };
        
        // Test with encodings
        let accept_enc = AcceptEncoding(vec![gzip.clone(), identity.clone()]);
        
        assert!(accept_enc.accepts("gzip"), "Should accept exact match");
        assert!(accept_enc.accepts("identity"), "Should accept exact match");
        assert!(!accept_enc.accepts("compress"), "Should not accept non-matching encoding");
        
        // Test with wildcard
        let accept_enc_wildcard = AcceptEncoding(vec![gzip.clone(), wildcard.clone()]);
        
        assert!(accept_enc_wildcard.accepts("compress"), "Should accept any encoding with wildcard");
        
        // Test with zero q-value
        let wildcard_zero = EncodingInfo {
            coding: "*".to_string(),
            q: Some(NotNan::new(0.0).unwrap()),
            params: vec![],
        };
        
        let accept_enc_zero = AcceptEncoding(vec![gzip.clone(), wildcard_zero]);
        
        assert!(accept_enc_zero.accepts("gzip"), "Should accept explicit encoding");
        assert!(!accept_enc_zero.accepts("compress"), "Should not accept wildcard with q=0");
        
        // Test empty Accept-Encoding (default is identity only)
        let empty_accept_enc = AcceptEncoding::new();
        assert!(empty_accept_enc.accepts("identity"), "Empty Accept-Encoding should accept identity");
        assert!(!empty_accept_enc.accepts("gzip"), "Empty Accept-Encoding should reject non-identity");
    }
    
    #[test]
    fn test_display() {
        // Create test encodings
        let gzip = EncodingInfo {
            coding: "gzip".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let identity = EncodingInfo {
            coding: "identity".to_string(),
            q: None, // Default 1.0
            params: vec![],
        };
        
        // Test display
        let accept_enc = AcceptEncoding(vec![gzip.clone(), identity.clone()]);
        let display_str = accept_enc.to_string();
        
        assert!(display_str.contains("identity"), "Should contain identity encoding");
        assert!(display_str.contains("gzip;q=0.800"), "Should contain gzip with q-value");
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create and convert to Header
        let accept_enc = AcceptEncoding(vec![
            EncodingInfo {
                coding: "gzip".to_string(),
                q: Some(NotNan::new(0.8).unwrap()),
                params: vec![],
            },
            EncodingInfo {
                coding: "identity".to_string(),
                q: None, // Default 1.0
                params: vec![],
            },
        ]);
        
        let header = accept_enc.to_header();
        assert_eq!(header.name, HeaderName::AcceptEncoding);
        
        // Convert back from Header
        let accept_enc2 = AcceptEncoding::from_header(&header).unwrap();
        assert_eq!(accept_enc.to_string(), accept_enc2.to_string());
        
        // Test with wrong header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(AcceptEncoding::from_header(&wrong_header).is_err());
    }
} 