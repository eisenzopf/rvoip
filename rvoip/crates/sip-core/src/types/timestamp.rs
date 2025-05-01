//! # SIP Timestamp Header
//!
//! This module provides an implementation of the SIP Timestamp header as defined in
//! [RFC 3261 Section 20.38](https://datatracker.ietf.org/doc/html/rfc3261#section-20.38).
//!
//! The Timestamp header field describes when a request was first initiated by the UAC,
//! and optionally how long it took the UAS to process the request.
//!
//! ## Format
//!
//! ```text
//! Timestamp: 54.21
//! Timestamp: 54.21 0.3
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::Timestamp;
//! use std::str::FromStr;
//! use ordered_float::NotNan;
//!
//! // Create a Timestamp header with just the timestamp
//! let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
//! assert_eq!(timestamp.to_string(), "54.21");
//!
//! // Create a Timestamp header with timestamp and delay
//! let timestamp = Timestamp::new(
//!     NotNan::new(54.21).unwrap(),
//!     Some(NotNan::new(0.3).unwrap())
//! );
//! assert_eq!(timestamp.to_string(), "54.21 0.3");
//!
//! // Parse from a string
//! let timestamp = Timestamp::from_str("54.21 0.3").unwrap();
//! assert_eq!(timestamp.value().into_inner(), 54.21);
//! assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Timestamp header field (RFC 3261 Section 20.38).
///
/// The Timestamp header field describes when a request was first initiated by the UAC,
/// and optionally how long it took the UAS to process the request. It consists of a
/// mandatory timestamp value (in seconds) and an optional delay value (also in seconds).
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::Timestamp;
/// use std::str::FromStr;
/// use ordered_float::NotNan;
///
/// // Create a Timestamp header with just the timestamp
/// let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
/// assert_eq!(timestamp.value().into_inner(), 54.21);
/// assert_eq!(timestamp.delay(), None);
///
/// // Create a Timestamp header with timestamp and delay
/// let timestamp = Timestamp::new(
///     NotNan::new(54.21).unwrap(),
///     Some(NotNan::new(0.3).unwrap())
/// );
/// assert_eq!(timestamp.value().into_inner(), 54.21);
/// assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
///
/// // Parse from a string
/// let timestamp = Timestamp::from_str("54.21 0.3").unwrap();
/// assert_eq!(timestamp.value().into_inner(), 54.21);
/// assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    /// The timestamp value in seconds, expressing when the request was initiated
    value: NotNan<f32>,
    /// The optional delay value in seconds, expressing how long it took the UAS to process the request
    delay: Option<NotNan<f32>>,
}

impl Timestamp {
    /// Creates a new Timestamp header with the specified values.
    ///
    /// # Parameters
    ///
    /// - `value`: The timestamp value in seconds
    /// - `delay`: The optional delay value in seconds
    ///
    /// # Returns
    ///
    /// A new `Timestamp` instance with the specified values
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Timestamp;
    /// use ordered_float::NotNan;
    ///
    /// // Create a Timestamp header with just the timestamp
    /// let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
    /// assert_eq!(timestamp.value().into_inner(), 54.21);
    /// assert_eq!(timestamp.delay(), None);
    ///
    /// // Create a Timestamp header with timestamp and delay
    /// let timestamp = Timestamp::new(
    ///     NotNan::new(54.21).unwrap(),
    ///     Some(NotNan::new(0.3).unwrap())
    /// );
    /// assert_eq!(timestamp.value().into_inner(), 54.21);
    /// assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
    /// ```
    pub fn new(value: NotNan<f32>, delay: Option<NotNan<f32>>) -> Self {
        Timestamp { value, delay }
    }

    /// Returns the timestamp value in seconds.
    ///
    /// # Returns
    ///
    /// The timestamp value in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Timestamp;
    /// use ordered_float::NotNan;
    ///
    /// let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
    /// assert_eq!(timestamp.value().into_inner(), 54.21);
    /// ```
    pub fn value(&self) -> NotNan<f32> {
        self.value
    }

    /// Returns the delay value in seconds, if present.
    ///
    /// # Returns
    ///
    /// The delay value in seconds, if present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Timestamp;
    /// use ordered_float::NotNan;
    ///
    /// let timestamp = Timestamp::new(
    ///     NotNan::new(54.21).unwrap(),
    ///     Some(NotNan::new(0.3).unwrap())
    /// );
    /// assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
    /// ```
    pub fn delay(&self) -> Option<NotNan<f32>> {
        self.delay
    }

    /// Sets the timestamp value.
    ///
    /// # Parameters
    ///
    /// - `value`: The new timestamp value in seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Timestamp;
    /// use ordered_float::NotNan;
    ///
    /// let mut timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
    /// timestamp.set_value(NotNan::new(60.0).unwrap());
    /// assert_eq!(timestamp.value().into_inner(), 60.0);
    /// ```
    pub fn set_value(&mut self, value: NotNan<f32>) {
        self.value = value;
    }

    /// Sets the delay value.
    ///
    /// # Parameters
    ///
    /// - `delay`: The new delay value in seconds, or `None` to remove it
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Timestamp;
    /// use ordered_float::NotNan;
    ///
    /// let mut timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
    /// timestamp.set_delay(Some(NotNan::new(0.5).unwrap()));
    /// assert_eq!(timestamp.delay().unwrap().into_inner(), 0.5);
    ///
    /// timestamp.set_delay(None);
    /// assert_eq!(timestamp.delay(), None);
    /// ```
    pub fn set_delay(&mut self, delay: Option<NotNan<f32>>) {
        self.delay = delay;
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.delay {
            Some(delay) => write!(f, "{} {}", self.value, delay),
            None => write!(f, "{}", self.value),
        }
    }
}

impl FromStr for Timestamp {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "Timestamp:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Timestamp header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Split the string by whitespace
        let parts: Vec<&str> = value_str.split_whitespace().collect();
        if parts.is_empty() || parts.len() > 2 {
            return Err(Error::ParseError("Invalid Timestamp format".to_string()));
        }
        
        // Parse the timestamp value
        let value = parts[0].parse::<f32>()
            .map_err(|e| Error::ParseError(format!("Invalid timestamp value: {}", e)))?;
        let value = NotNan::new(value)
            .map_err(|e| Error::ParseError(format!("Invalid timestamp value: {}", e)))?;
            
        // Parse the delay value, if present
        let delay = if parts.len() > 1 {
            let delay = parts[1].parse::<f32>()
                .map_err(|e| Error::ParseError(format!("Invalid delay value: {}", e)))?;
            let delay = NotNan::new(delay)
                .map_err(|e| Error::ParseError(format!("Invalid delay value: {}", e)))?;
            Some(delay)
        } else {
            None
        };
            
        Ok(Timestamp { value, delay })
    }
}

// Implement TypedHeaderTrait for Timestamp
impl TypedHeaderTrait for Timestamp {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Timestamp
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
                    Timestamp::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Timestamp(value, delay) => {
                let value = NotNan::new(*value)
                    .map_err(|e| Error::InvalidHeader(format!("Invalid timestamp value: {}", e)))?;
                let delay = delay.map(|d| NotNan::new(d))
                    .transpose()
                    .map_err(|e| Error::InvalidHeader(format!("Invalid delay value: {}", e)))?;
                Ok(Timestamp { value, delay })
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
        // With just timestamp
        let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
        assert_eq!(timestamp.value().into_inner(), 54.21);
        assert_eq!(timestamp.delay(), None);
        
        // With timestamp and delay
        let timestamp = Timestamp::new(
            NotNan::new(54.21).unwrap(),
            Some(NotNan::new(0.3).unwrap())
        );
        assert_eq!(timestamp.value().into_inner(), 54.21);
        assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
    }
    
    #[test]
    fn test_setters() {
        let mut timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
        
        // Set value
        timestamp.set_value(NotNan::new(60.0).unwrap());
        assert_eq!(timestamp.value().into_inner(), 60.0);
        
        // Set delay
        timestamp.set_delay(Some(NotNan::new(0.5).unwrap()));
        assert_eq!(timestamp.delay().unwrap().into_inner(), 0.5);
        
        // Remove delay
        timestamp.set_delay(None);
        assert_eq!(timestamp.delay(), None);
    }
    
    #[test]
    fn test_display() {
        // With just timestamp
        let timestamp = Timestamp::new(NotNan::new(54.21).unwrap(), None);
        assert_eq!(timestamp.to_string(), "54.21");
        
        // With timestamp and delay
        let timestamp = Timestamp::new(
            NotNan::new(54.21).unwrap(),
            Some(NotNan::new(0.3).unwrap())
        );
        assert_eq!(timestamp.to_string(), "54.21 0.3");
    }
    
    #[test]
    fn test_from_str() {
        // With just timestamp
        let timestamp: Timestamp = "54.21".parse().unwrap();
        assert_eq!(timestamp.value().into_inner(), 54.21);
        assert_eq!(timestamp.delay(), None);
        
        // With timestamp and delay
        let timestamp: Timestamp = "54.21 0.3".parse().unwrap();
        assert_eq!(timestamp.value().into_inner(), 54.21);
        assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
        
        // With header name
        let timestamp: Timestamp = "Timestamp: 54.21 0.3".parse().unwrap();
        assert_eq!(timestamp.value().into_inner(), 54.21);
        assert_eq!(timestamp.delay().unwrap().into_inner(), 0.3);
        
        // Invalid formats
        let result: Result<Timestamp> = "".parse();
        assert!(result.is_err());
        
        let result: Result<Timestamp> = "not_a_number".parse();
        assert!(result.is_err());
        
        let result: Result<Timestamp> = "54.21 0.3 0.4".parse();
        assert!(result.is_err());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let timestamp = Timestamp::new(
            NotNan::new(54.21).unwrap(),
            Some(NotNan::new(0.3).unwrap())
        );
        let header = timestamp.to_header();
        
        assert_eq!(header.name, HeaderName::Timestamp);
        
        // Convert back from Header
        let timestamp2 = Timestamp::from_header(&header).unwrap();
        assert_eq!(timestamp.value(), timestamp2.value());
        assert_eq!(timestamp.delay(), timestamp2.delay());
        
        // Test with Timestamp HeaderValue
        let timestamp_header = Header::new(
            HeaderName::Timestamp,
            HeaderValue::Timestamp(54.21, Some(0.3))
        );
        let timestamp3 = Timestamp::from_header(&timestamp_header).unwrap();
        assert_eq!(timestamp3.value().into_inner(), 54.21);
        assert_eq!(timestamp3.delay().unwrap().into_inner(), 0.3);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(Timestamp::from_header(&wrong_header).is_err());
    }
} 