//! # SIP Date Header
//!
//! This module provides an implementation of the SIP Date header as defined in
//! [RFC 3261 Section 20.17](https://datatracker.ietf.org/doc/html/rfc3261#section-20.17).
//!
//! The Date header field contains the date and time when a message was originated.
//! The format used is identical to the HTTP Date header field format described in
//! [RFC 2616 Section 14.18](https://datatracker.ietf.org/doc/html/rfc2616#section-14.18),
//! which follows the Internet Message Format date and time specification
//! from [RFC 2822 Section 3.3](https://datatracker.ietf.org/doc/html/rfc2822#section-3.3).
//!
//! ## Format
//!
//! ```rust
//! use rvoip_sip_core::types::Date;
//! use chrono::{TimeZone, Utc};
//! 
//! // Create a date with a specific timestamp to see the format
//! let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
//! let date = Date::new(timestamp);
//! let formatted = date.to_string();
//! 
//! // Format is: "Day, DD Mon YYYY HH:MM:SS GMT"
//! // Example: "Wed, 15 Nov 2023 08:12:31 GMT"
//! assert!(formatted.contains("15 Nov 2023 08:12:31 GMT"));
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::Date;
//! use std::str::FromStr;
//! use chrono::{TimeZone, Utc, Datelike};
//!
//! // Create with the current timestamp
//! let now = Date::now();
//!
//! // Create with a specific timestamp
//! let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
//! let date = Date::new(timestamp);
//!
//! // Format as a string and verify the date parts
//! let formatted = date.to_string();
//! assert!(formatted.contains("15 Nov 2023 08:12:31"));
//!
//! // Parse from the formatted string
//! let parsed_date = Date::from_str(&formatted).unwrap();
//! assert_eq!(parsed_date.timestamp().date_naive().year(), 2023);
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use chrono::{DateTime, FixedOffset, Utc, TimeZone, Datelike, Timelike};
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Date header field (RFC 3261 Section 20.17).
///
/// The Date header field contains the date and time when a message was originated.
/// It uses the HTTP-date format from RFC 2616, which follows the RFC 2822 date and time
/// specification.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::Date;
/// use std::str::FromStr;
/// use chrono::{TimeZone, Utc, Datelike};
///
/// // Create with the current timestamp
/// let now = Date::now();
///
/// // Create with a specific timestamp
/// let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
/// let date = Date::new(timestamp);
///
/// // Format as a string and verify the date parts
/// let formatted = date.to_string();
/// assert!(formatted.contains("15 Nov 2023 08:12:31"));
///
/// // Parse from the formatted string
/// let parsed_date = Date::from_str(&formatted).unwrap();
/// assert_eq!(parsed_date.timestamp().date_naive().year(), 2023);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Date {
    /// The timestamp value
    timestamp: DateTime<FixedOffset>,
}

impl Date {
    /// Creates a new Date header with the specified timestamp.
    ///
    /// # Parameters
    ///
    /// - `timestamp`: The timestamp to include in the Date header
    ///
    /// # Returns
    ///
    /// A new `Date` instance with the specified timestamp
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Date;
    /// use chrono::{DateTime, TimeZone, Utc, Datelike};
    ///
    /// let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
    /// let date = Date::new(timestamp);
    /// assert_eq!(date.timestamp().date_naive().year(), 2023);
    /// ```
    pub fn new<Tz: TimeZone>(timestamp: DateTime<Tz>) -> Self
    where
        DateTime<Tz>: Into<DateTime<FixedOffset>>
    {
        Date {
            timestamp: timestamp.into(),
        }
    }

    /// Creates a new Date header with the current timestamp.
    ///
    /// # Returns
    ///
    /// A new `Date` instance with the current UTC timestamp
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Date;
    /// use chrono::Utc;
    ///
    /// let date = Date::now();
    /// let now = Utc::now();
    ///
    /// // The timestamps should be very close (within seconds)
    /// let diff = (now.timestamp() - date.timestamp().timestamp()).abs();
    /// assert!(diff < 5, "Timestamps should be close");
    /// ```
    pub fn now() -> Self {
        Self::new(Utc::now())
    }

    /// Returns the timestamp value.
    ///
    /// # Returns
    ///
    /// The timestamp from this Date header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::Date;
    /// use chrono::{TimeZone, Utc, Datelike};
    ///
    /// let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
    /// let date = Date::new(timestamp);
    /// 
    /// let ts = date.timestamp();
    /// assert_eq!(ts.date_naive().year(), 2023);
    /// assert_eq!(ts.date_naive().month(), 11);
    /// assert_eq!(ts.date_naive().day(), 15);
    /// ```
    pub fn timestamp(&self) -> &DateTime<FixedOffset> {
        &self.timestamp
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format according to RFC 2822 date format as required by RFC 3261
        // Example: "Tue, 15 Nov 2023 08:12:31 GMT"
        write!(f, "{}", self.timestamp.format("%a, %d %b %Y %H:%M:%S GMT"))
    }
}

impl FromStr for Date {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') && s.split(':').count() > 2 {
            // Strip the "Date:" prefix, being careful not to split on the time part (which contains colons)
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Date header format".to_string()));
            }
            
            let header_name = parts[0].trim();
            if header_name.eq_ignore_ascii_case("date") {
                // This looks like a Date header, so parse just the value part
                parts[1].trim()
            } else {
                // Not a Date header, try to parse the whole string
                s.trim()
            }
        } else {
            // No header name, parse the whole string
            s.trim()
        };
        
        // Try different date format variations
        // RFC 2822 format: "Tue, 15 Nov 2023 08:12:31 GMT"
        let timestamp = chrono::DateTime::parse_from_rfc2822(value_str)
            .or_else(|_| chrono::DateTime::parse_from_str(value_str, "%a, %d %b %Y %H:%M:%S GMT"))
            .or_else(|_| chrono::DateTime::parse_from_str(value_str, "%a, %d %b %Y %H:%M:%S %z"))
            .map_err(|e| Error::ParseError(format!("Invalid date format: {}", e)))?;
            
        Ok(Date { timestamp })
    }
}

// Implement TypedHeaderTrait for Date
impl TypedHeaderTrait for Date {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Date
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
                    Date::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Date(bytes) => {
                // Try to convert the raw bytes to a string and parse it
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Date::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
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
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_new() {
        let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
        let date = Date::new(timestamp);
        
        assert_eq!(date.timestamp().date_naive().year(), 2023);
        assert_eq!(date.timestamp().date_naive().month(), 11);
        assert_eq!(date.timestamp().date_naive().day(), 15);
        assert_eq!(date.timestamp().hour(), 8);
        assert_eq!(date.timestamp().minute(), 12);
        assert_eq!(date.timestamp().second(), 31);
    }
    
    #[test]
    fn test_now() {
        let date = Date::now();
        let now = Utc::now();
        
        // The timestamps should be very close (within seconds)
        let diff = (now.timestamp() - date.timestamp().timestamp()).abs();
        assert!(diff < 5, "Timestamps should be close");
    }
    
    #[test]
    fn test_display() {
        let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
        let date = Date::new(timestamp);
        
        // Use the %a format to get actual day name rather than hardcoding
        let expected = format!("{}", timestamp.format("%a, %d %b %Y %H:%M:%S GMT"));
        assert_eq!(date.to_string(), expected);
    }
    
    #[test]
    fn test_from_str() {
        // Standard format
        let date: Date = "Wed, 15 Nov 2023 08:12:31 GMT".parse().unwrap();
        assert_eq!(date.timestamp().year(), 2023);
        assert_eq!(date.timestamp().month(), 11);
        assert_eq!(date.timestamp().day(), 15);
        assert_eq!(date.timestamp().hour(), 8);
        assert_eq!(date.timestamp().minute(), 12);
        assert_eq!(date.timestamp().second(), 31);
        
        // With header name
        let date: Date = "Date: Wed, 15 Nov 2023 08:12:31 GMT".parse().unwrap();
        assert_eq!(date.timestamp().year(), 2023);
        
        // With different timezone
        let date: Date = "Wed, 15 Nov 2023 08:12:31 +0000".parse().unwrap();
        assert_eq!(date.timestamp().year(), 2023);
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let timestamp = Utc.with_ymd_and_hms(2023, 11, 15, 8, 12, 31).unwrap();
        let date = Date::new(timestamp);
        let header = date.to_header();
        
        assert_eq!(header.name, HeaderName::Date);
        
        // Convert back from Header
        let date2 = Date::from_header(&header).unwrap();
        assert_eq!(date.timestamp(), date2.timestamp());
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(Date::from_header(&wrong_header).is_err());
    }
} 