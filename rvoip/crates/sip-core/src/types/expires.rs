//! # SIP Expires Header
//!
//! This module provides an implementation of the SIP Expires header as defined in
//! [RFC 3261 Section 20.19](https://datatracker.ietf.org/doc/html/rfc3261#section-20.19).
//!
//! The Expires header gives the relative time after which the message or content
//! expires. Its primary uses in SIP include:
//!
//! - Limiting the validity duration of registrations (REGISTER requests)
//! - Setting the subscription duration (SUBSCRIBE requests)
//! - Setting the validity period of event state (NOTIFY requests)
//! - Limiting the validity of a SIP message (any request/response)
//!
//! ## Format
//!
//! ```text
//! Expires: 3600
//! ```
//!
//! The value is a decimal integer number of seconds.
//!
//! ## Common Values
//!
//! - **0**: Indicates immediate expiration (often used to remove registrations or terminate subscriptions)
//! - **300-600**: Common for short-lived registrations or subscriptions (5-10 minutes)
//! - **3600**: Common for hourly registrations
//! - **86400**: Daily registrations (24 hours)
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an Expires header for a one-hour registration
//! let expires = Expires::new(3600);
//! assert_eq!(expires.to_string(), "3600");
//!
//! // Parse an Expires header
//! let expires = Expires::from_str("1800").unwrap();
//! assert_eq!(expires.0, 1800);
//! ```

use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use crate::parser::headers::parse_expires;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Represents the Expires header field (RFC 3261 Section 20.19).
/// Indicates the duration for which a registration or subscription is valid.
///
/// The Expires header specifies a relative time after which the message or
/// content will expire. It is a simple unsigned integer value representing
/// the number of seconds.
///
/// This header is commonly used in:
/// - REGISTER requests to set registration duration
/// - SUBSCRIBE requests to set subscription duration
/// - NOTIFY requests to indicate event state validity
/// - Any request/response to limit message validity
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new Expires header
/// let expires = Expires::new(3600); // 1 hour
///
/// // Parse from a string
/// let expires = Expires::from_str("1800").unwrap(); // 30 minutes
///
/// // Create an Expires indicating immediate expiration
/// let expires = Expires::new(0);
/// assert_eq!(expires.0, 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Expires(pub u32);

impl Expires {
    /// Creates a new Expires header value.
    ///
    /// # Parameters
    ///
    /// - `seconds`: The time in seconds until expiration
    ///
    /// # Returns
    ///
    /// A new `Expires` instance with the specified duration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create an Expires header for a one-hour registration
    /// let expires = Expires::new(3600);
    /// assert_eq!(expires.to_string(), "3600");
    ///
    /// // Create an Expires header for immediate expiration (de-registration)
    /// let expires = Expires::new(0);
    /// assert_eq!(expires.to_string(), "0");
    ///
    /// // Create an Expires header for a one-day registration
    /// let expires = Expires::new(86400);
    /// assert_eq!(expires.to_string(), "86400");
    /// ```
    pub fn new(seconds: u32) -> Self {
        Self(seconds)
    }
}

impl fmt::Display for Expires {
    /// Formats the Expires header as a string.
    ///
    /// Converts the Expires value to a simple decimal integer
    /// string representing seconds.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let expires = Expires::new(3600);
    /// assert_eq!(expires.to_string(), "3600");
    ///
    /// // Using in a formatted string
    /// let header = format!("Expires: {}", expires);
    /// assert_eq!(header, "Expires: 3600");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Expires {
    type Err = Error;

    /// Parses a string into an Expires header.
    ///
    /// This method parses a string representation of an Expires header
    /// into an Expires struct. The input string should be a valid
    /// decimal integer representing the number of seconds.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Expires, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a valid Expires header
    /// let expires = Expires::from_str("3600").unwrap();
    /// assert_eq!(expires.0, 3600);
    ///
    /// // Note: The parser requires a clean integer without spaces
    /// // let expires = Expires::from_str("  1800  ").unwrap(); // This would fail
    ///
    /// // Parse zero (immediate expiration)
    /// let expires = Expires::from_str("0").unwrap();
    /// assert_eq!(expires.0, 0);
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::expires::parse_expires;
        
        // Use map_err and From to convert Nom error to crate::Error::ParseError
        all_consuming(parse_expires)(s.as_bytes())
            .map_err(Error::from)
            .map(|(_, value)| Expires(value))
    }
}

// TODO: Implement methods if needed 