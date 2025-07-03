//! # SIP Accept Header
//! 
//! This module provides an implementation of the SIP Accept header field as defined in
//! [RFC 3261 Section 20.1](https://datatracker.ietf.org/doc/html/rfc3261#section-20.1).
//!
//! The Accept header field is used to indicate which media types are acceptable in responses.
//! It follows the same syntax as the HTTP Accept header field, allowing for media types
//! with quality values ("q-values") that indicate relative preference.
//!
//! If no Accept header is present, the server should assume that the client will accept
//! any media type in the response.
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::Accept;
//! use std::str::FromStr;
//! use std::collections::HashMap;
//!
//! // Parse an Accept header
//! let header = Accept::from_str("application/sdp;level=1;q=0.9, application/json;q=0.5").unwrap();
//!
//! // Create programmatically
//! use rvoip_sip_core::parser::headers::accept::AcceptValue;
//! use ordered_float::NotNan;
//!
//! let mut sdp = AcceptValue {
//!     m_type: "application".to_string(),
//!     m_subtype: "sdp".to_string(),
//!     q: Some(NotNan::new(0.9).unwrap()),
//!     params: HashMap::new(),
//! };
//! sdp.params.insert("level".to_string(), "1".to_string());
//!
//! let header = Accept::from_media_types(vec![sdp]);
//! ```

use crate::types::media_type::MediaType;
use crate::parser::headers::parse_accept;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use std::collections::HashMap;
use ordered_float::NotNan;
use crate::parser::headers::accept::AcceptValue;
use serde::{Deserialize, Serialize};
use crate::types::param::Param;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Accept header field (RFC 3261 Section 20.1).
///
/// The Accept header indicates which media types are acceptable in the response.
/// It contains a prioritized list of media types, each potentially with a quality value ("q-value")
/// that indicates its relative preference (from 0.0 to 1.0, with 1.0 being the default and highest priority).
///
/// As per RFC 3261, if this header is not present in a request, the server should assume the client
/// will accept any media type in the response.
///
/// # Media Type Matching
///
/// Media type matching follows the rules outlined in RFC 2616:
///
/// - Media types are matched by type and subtype
/// - Wildcards (`*`) can be used for either the type or subtype
/// - Media types with higher q-values are preferred over those with lower q-values
/// - Media types with the same q-value are ordered by their original order in the header
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::Accept;
/// use std::str::FromStr;
///
/// // Create from a header string
/// let header = Accept::from_str("application/sdp;q=0.9, application/json;q=0.5").unwrap();
///
/// // Create programmatically
/// use rvoip_sip_core::parser::headers::accept::AcceptValue;
/// use ordered_float::NotNan;
/// use std::collections::HashMap;
///
/// let sdp = AcceptValue {
///     m_type: "application".to_string(),
///     m_subtype: "sdp".to_string(),
///     q: Some(NotNan::new(0.9).unwrap()),
///     params: HashMap::new(),
/// };
///
/// let json = AcceptValue {
///     m_type: "application".to_string(),
///     m_subtype: "json".to_string(),
///     q: Some(NotNan::new(0.5).unwrap()),
///     params: HashMap::new(),
/// };
///
/// let header = Accept::from_media_types(vec![sdp, json]);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Accept(pub Vec<AcceptValue>);

impl Accept {
    /// Creates an empty Accept header.
    ///
    /// An empty Accept header means all media types are acceptable,
    /// according to RFC 3261 Section 20.1.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    ///
    /// let header = Accept::new();
    /// // Empty Accept header indicates all media types are acceptable
    /// ```
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Accept header with specified capacity.
    ///
    /// This is useful when you know approximately how many media types
    /// you'll be adding to avoid reallocations.
    ///
    /// # Parameters
    ///
    /// - `capacity`: The initial capacity for the media types vector
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    ///
    /// let mut header = Accept::with_capacity(3);
    /// // Can now add up to 3 media types without reallocation
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Accept header from an iterator of media types.
    ///
    /// # Parameters
    ///
    /// - `types`: An iterator yielding `AcceptValue` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    /// use rvoip_sip_core::parser::headers::accept::AcceptValue;
    /// use ordered_float::NotNan;
    /// use std::collections::HashMap;
    ///
    /// let sdp = AcceptValue {
    ///     m_type: "application".to_string(),
    ///     m_subtype: "sdp".to_string(),
    ///     q: None, // Default q-value is 1.0
    ///     params: HashMap::new(),
    /// };
    ///
    /// let header = Accept::from_media_types(vec![sdp]);
    /// ```
    pub fn from_media_types<I>(types: I) -> Self
    where
        I: IntoIterator<Item = AcceptValue>
    {
        Self(types.into_iter().collect())
    }

    /// Adds a media type to the list.
    ///
    /// # Parameters
    ///
    /// - `media_type`: The media type to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    /// use rvoip_sip_core::parser::headers::accept::AcceptValue;
    /// use std::collections::HashMap;
    ///
    /// let mut header = Accept::new();
    /// let sdp = AcceptValue {
    ///     m_type: "application".to_string(),
    ///     m_subtype: "sdp".to_string(),
    ///     q: None,
    ///     params: HashMap::new(),
    /// };
    /// header.push(sdp);
    /// ```
    pub fn push(&mut self, media_type: AcceptValue) {
        self.0.push(media_type);
    }

    /// Checks if a specific media type is acceptable (basic check).
    ///
    /// This method performs a basic match using type and subtype, with support for wildcards.
    /// It doesn't yet account for parameters or q-values for prioritization.
    ///
    /// # Parameters
    ///
    /// - `media_type`: The media type to check
    ///
    /// # Returns
    ///
    /// `true` if the media type is acceptable, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    /// use rvoip_sip_core::parser::headers::accept::AcceptValue;
    /// use std::str::FromStr;
    /// use std::collections::HashMap;
    ///
    /// // Create an Accept header that accepts all application/* types
    /// let header = Accept::from_str("application/*").unwrap();
    ///
    /// // Create a test media type
    /// let sdp = AcceptValue {
    ///     m_type: "application".to_string(),
    ///     m_subtype: "sdp".to_string(),
    ///     q: None,
    ///     params: HashMap::new(),
    /// };
    ///
    /// assert!(header.accepts(&sdp));
    /// ```
    ///
    /// # Note
    ///
    /// This implementation is basic and will be enhanced in the future to properly
    /// handle parameters and q-values for more accurate matching.
    pub fn accepts(&self, media_type: &AcceptValue) -> bool {
        self.0.iter().any(|accepted_type| {
            // Simple type/subtype match
            (accepted_type.m_type == "*" || accepted_type.m_type == media_type.m_type) &&
            (accepted_type.m_subtype == "*" || accepted_type.m_subtype == media_type.m_subtype)
            // TODO: Parameter matching (like q values)
        })
    }

    /// Checks if this Accept header accepts a specific media type by type and subtype strings.
    ///
    /// # Arguments
    ///
    /// * `type_str` - The media type (e.g., "application")
    /// * `subtype_str` - The media subtype (e.g., "sdp")
    ///
    /// # Returns
    ///
    /// `true` if the specified media type is acceptable, `false` otherwise
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Accept;
    /// use std::str::FromStr;
    ///
    /// let accept = Accept::from_str("application/sdp;q=0.9, application/json;q=0.5").unwrap();
    /// assert!(accept.accepts_type("application", "sdp"));
    /// assert!(accept.accepts_type("application", "json"));
    /// assert!(!accept.accepts_type("text", "plain"));
    /// ```
    pub fn accepts_type(&self, type_str: &str, subtype_str: &str) -> bool {
        // Check if the media type is in the Accept header
        for accept_val in &self.0 {
            if (accept_val.m_type == "*" || accept_val.m_type.eq_ignore_ascii_case(type_str)) &&
                (accept_val.m_subtype == "*" || accept_val.m_subtype.eq_ignore_ascii_case(subtype_str)) {
                // Check q value - if q=0, then the media type is not acceptable
                if let Some(q) = accept_val.q {
                    if q.into_inner() <= 0.0 {
                        return false;
                    }
                }
                return true;
            }
        }
        
        // If no explicit match, check for wildcard
        for accept_val in &self.0 {
            if accept_val.m_type == "*" && accept_val.m_subtype == "*" {
                // Check q value - if q=0, then the media type is not acceptable
                if let Some(q) = accept_val.q {
                    if q.into_inner() <= 0.0 {
                        return false;
                    }
                }
                return true;
            }
        }
        
        // Default behavior: if Not mentioned and no wildcard, return false
        false
    }

    /// Returns a list of media types defined in this Accept header.
    ///
    /// # Returns
    ///
    /// A vector of media types with their parameters
    pub fn media_types(&self) -> &[AcceptValue] {
        &self.0
    }
}

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", type_strings.join(", "))
    }
}

// Helper function for the FromStr implementation
fn parse_from_owned_bytes(bytes: Vec<u8>) -> Result<Vec<AcceptValue>> {
    // Check if the input starts with "Accept:" and strip it if present
    let bytes_to_parse = if bytes.len() > 8 && 
        bytes[0..6].eq_ignore_ascii_case(b"Accept") && 
        bytes[6] == b':' {
        // Skip the header name and colon, and any leading whitespace
        let mut i = 7;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        &bytes[i..]
    } else {
        &bytes
    };

    match all_consuming(parse_accept)(bytes_to_parse) {
        Ok((_, accept)) => Ok(accept.0),
        Err(e) => Err(Error::ParseError(
            format!("Failed to parse Accept header: {:?}", e)
        ))
    }
}

impl FromStr for Accept {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header without the name
        // (e.g., "application/sdp;q=0.8" instead of "Accept: application/sdp;q=0.8")
        let input_bytes = if !s.contains(':') {
            format!("Accept: {}", s).into_bytes()
        } else {
            s.as_bytes().to_vec()
        };
        
        // Parse using our helper function that takes ownership of the bytes
        parse_from_owned_bytes(input_bytes)
            .map(Accept::from_media_types)
    }
}

/// Formats an `AcceptValue` as a string according to SIP specifications.
///
/// The format follows: `type/subtype;param1=value1;param2=value2;q=value`.
/// Quality values (`q`) are always formatted with 3 decimal places.
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::parser::headers::accept::AcceptValue;
/// use ordered_float::NotNan;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("level".to_string(), "1".to_string());
///
/// let media_type = AcceptValue {
///     m_type: "application".to_string(),
///     m_subtype: "sdp".to_string(),
///     q: Some(NotNan::new(0.9).unwrap()),
///     params,
/// };
///
/// // Results in: "application/sdp;level=1;q=0.9"
/// let formatted = media_type.to_string();
/// ```
impl fmt::Display for AcceptValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = format!("{}/{}", self.m_type, self.m_subtype);
        
        // First output params other than 'q'
        for (k, v) in &self.params {
            if k != "q" {
                s.push_str(&format!(";{}={}", k, v));
            }
        }
        
        // Then output q parameter if it exists
        if let Some(q) = self.q {
            s.push_str(&format!(";q={}", q));
        }
        
        write!(f, "{}", s)
    }
}

impl TypedHeaderTrait for Accept {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Accept
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Accept(self.0.clone()))
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
                    Accept::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Accept(accept_values) => Ok(Accept(accept_values.clone())),
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}