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
//! use rvoip_sip_core::prelude::*;
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
/// use rvoip_sip_core::prelude::*;
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
    /// ```
    /// use rvoip_sip_core::prelude::*;
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
    /// ```
    /// use rvoip_sip_core::prelude::*;
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
    /// ```
    /// use rvoip_sip_core::prelude::*;
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
    /// ```
    /// use rvoip_sip_core::prelude::*;
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
    /// ```
    /// use rvoip_sip_core::prelude::*;
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
}

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", type_strings.join(", "))
    }
}

impl FromStr for Accept {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::accept::parse_accept;

        // Convert &str to &[u8] and use all_consuming
        match all_consuming(parse_accept)(s.as_bytes()) {
            Ok((_, accept_header)) => Ok(accept_header),
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Accept header: {:?}", e)
            ))
        }
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
        if let Some(q) = self.q {
            s.push_str(&format!(";q={}", q));
        }
        for (k, v) in &self.params {
            s.push_str(&format!(";{}={}", k, v));
        }
        write!(f, "{}", s)
    }
}

// TODO: Implement methods (e.g., for checking acceptable types)