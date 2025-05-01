//! # SIP Min-Expires Header
//!
//! This module provides an implementation of the SIP Min-Expires header as defined in
//! [RFC 3261 Section 20.23](https://datatracker.ietf.org/doc/html/rfc3261#section-20.23).
//!
//! The Min-Expires header field conveys the minimum refresh interval supported for
//! soft-state elements managed by a server. The header field contains a delta-seconds
//! value that indicates the number of seconds the client should wait before refreshing.
//!
//! This header field is primarily used in REGISTER and 423 (Interval Too Brief) responses.
//!
//! ## Format
//!
//! ```text
//! Min-Expires: 60
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::MinExpires;
//! use std::str::FromStr;
//!
//! // Create a Min-Expires header
//! let min_expires = MinExpires::new(60);
//! assert_eq!(min_expires.value(), 60);
//!
//! // Parse from a string
//! let min_expires = MinExpires::from_str("3600").unwrap();
//! assert_eq!(min_expires.value(), 3600);
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Min-Expires header field (RFC 3261 Section 20.23).
///
/// The Min-Expires header field indicates the minimum refresh interval
/// supported for soft-state elements, expressed in delta-seconds.
///
/// This header field is often included in 423 (Interval Too Brief) responses
/// to indicate the minimum registration expiration time the server will accept.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::MinExpires;
/// use std::str::FromStr;
///
/// // Create a Min-Expires header with a value of 60 seconds
/// let min_expires = MinExpires::new(60);
/// assert_eq!(min_expires.value(), 60);
///
/// // Convert to a string
/// assert_eq!(min_expires.to_string(), "60");
///
/// // Parse from a string
/// let min_expires = MinExpires::from_str("3600").unwrap();
/// assert_eq!(min_expires.value(), 3600);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinExpires(u32);

impl MinExpires {
    /// Creates a new Min-Expires header with the specified value.
    ///
    /// # Parameters
    ///
    /// - `value`: The minimum expiration time in seconds
    ///
    /// # Returns
    ///
    /// A new `MinExpires` instance with the specified value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MinExpires;
    ///
    /// let min_expires = MinExpires::new(60);
    /// assert_eq!(min_expires.value(), 60);
    /// ```
    pub fn new(value: u32) -> Self {
        MinExpires(value)
    }

    /// Returns the minimum expiration time in seconds.
    ///
    /// # Returns
    ///
    /// The minimum expiration time in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MinExpires;
    ///
    /// let min_expires = MinExpires::new(60);
    /// assert_eq!(min_expires.value(), 60);
    /// ```
    pub fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for MinExpires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MinExpires {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "Min-Expires:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Min-Expires header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Parse the value as a u32
        let value = value_str.parse::<u32>()
            .map_err(|e| Error::ParseError(format!("Invalid Min-Expires value: {}", e)))?;
            
        Ok(MinExpires(value))
    }
}

// Implement TypedHeaderTrait for MinExpires
impl TypedHeaderTrait for MinExpires {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::MinExpires
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
                    MinExpires::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Integer(value) => {
                // Ensure value fits in u32
                if *value >= 0 && *value <= u32::MAX as i64 {
                    Ok(MinExpires(*value as u32))
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid {} value: {} (out of range)", Self::header_name(), value)
                    ))
                }
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
        let min_expires = MinExpires::new(60);
        assert_eq!(min_expires.value(), 60);
    }
    
    #[test]
    fn test_display() {
        let min_expires = MinExpires::new(60);
        assert_eq!(min_expires.to_string(), "60");
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let min_expires: MinExpires = "60".parse().unwrap();
        assert_eq!(min_expires.value(), 60);
        
        // With header name
        let min_expires: MinExpires = "Min-Expires: 3600".parse().unwrap();
        assert_eq!(min_expires.value(), 3600);
        
        // Invalid value
        let result: Result<MinExpires> = "not_a_number".parse();
        assert!(result.is_err());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let min_expires = MinExpires::new(60);
        let header = min_expires.to_header();
        
        assert_eq!(header.name, HeaderName::MinExpires);
        
        // Convert back from Header
        let min_expires2 = MinExpires::from_header(&header).unwrap();
        assert_eq!(min_expires.value(), min_expires2.value());
        
        // Test with Integer HeaderValue
        let integer_header = Header::new(HeaderName::MinExpires, HeaderValue::Integer(3600));
        let min_expires3 = MinExpires::from_header(&integer_header).unwrap();
        assert_eq!(min_expires3.value(), 3600);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(MinExpires::from_header(&wrong_header).is_err());
    }
} 