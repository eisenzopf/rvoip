//! # SIP Max-Forwards Header
//!
//! This module provides an implementation of the SIP Max-Forwards header as defined in
//! [RFC 3261 Section 20.22](https://datatracker.ietf.org/doc/html/rfc3261#section-20.22).
//!
//! The Max-Forwards header field serves to limit the number of hops a request can 
//! transit on the way to its destination. It consists of an integer that is 
//! decremented by one at each hop.
//!
//! ## Purpose
//!
//! The Max-Forwards header is primarily used to:
//!
//! - Prevent request loops in a SIP network
//! - Limit the impact of routing errors
//! - Provide a simple mechanism for detecting routing loops
//! - Allow testing of a specific number of network hops
//!
//! ## Format
//!
//! ```
//! // Example format:
//! // Max-Forwards: 70
//! ```
//!
//! ## Usage
//!
//! Each proxy or redirect server decrements the value before forwarding the request.
//! If the value reaches zero before the request reaches its final destination,
//! the server will return a 483 (Too Many Hops) response.
//!
//! ## Recommended Values
//!
//! The default value for initial requests is typically 70, though this can be
//! configured based on network topology and requirements.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Max-Forwards header with the default value
//! let max_forwards = MaxForwards::new(70);
//! 
//! // Parse from a string
//! let max_forwards = MaxForwards::from_str("70").unwrap();
//! 
//! // Process at a proxy
//! if let Some(decremented) = max_forwards.decrement() {
//!     // Forward the request with the decremented value
//! } else {
//!     // Return 483 Too Many Hops
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::error::{Error, Result};

/// Represents the Max-Forwards header field (RFC 3261 Section 8.1.1.4).
/// Limits the number of proxies a request can traverse.
///
/// This header is mandatory in all SIP requests to prevent infinite loops in 
/// recursive and iterative routing situations. Each proxy that forwards the 
/// request must decrement this value by one. If a proxy or redirect server
/// receives a request with Max-Forwards set to 0, it must not forward the
/// request and should instead return a 483 (Too Many Hops) response.
///
/// The Max-Forwards value is an 8-bit unsigned integer (0-255), though in 
/// practice, values around 70 are common for initial requests.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new Max-Forwards header with a value of 70
/// let max_forwards = MaxForwards::new(70);
/// assert_eq!(max_forwards.to_string(), "70");
///
/// // Parse from a string
/// let max_forwards = MaxForwards::from_str("42").unwrap();
/// assert_eq!(max_forwards.0, 42);
///
/// // Check if the value has reached zero
/// assert!(!max_forwards.is_zero());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MaxForwards(pub u8);

impl MaxForwards {
    /// Creates a new Max-Forwards header value.
    ///
    /// # Parameters
    ///
    /// - `hops`: The number of allowed forwarding hops for the request
    ///
    /// # Returns
    ///
    /// A new `MaxForwards` instance with the specified value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Default value commonly used for initial requests
    /// let max_forwards = MaxForwards::new(70);
    /// assert_eq!(max_forwards.0, 70);
    ///
    /// // For testing specific routing paths
    /// let test_max_forwards = MaxForwards::new(3);
    /// assert_eq!(test_max_forwards.0, 3);
    /// ```
    pub fn new(hops: u8) -> Self {
        Self(hops)
    }

    /// Decrements the Max-Forwards value, returning None if it reaches zero.
    ///
    /// This method is used by proxies and redirect servers when forwarding a request.
    /// If the result would be zero, None is returned, indicating that the request
    /// should not be forwarded further.
    ///
    /// # Returns
    ///
    /// - `Some(MaxForwards)` with the decremented value if the current value is greater than zero
    /// - `None` if the current value is zero, indicating the request should not be forwarded
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Typical proxy forwarding logic
    /// let max_forwards = MaxForwards::new(3);
    ///
    /// // First hop
    /// let max_forwards = max_forwards.decrement().unwrap();
    /// assert_eq!(max_forwards.0, 2);
    ///
    /// // Second hop
    /// let max_forwards = max_forwards.decrement().unwrap();
    /// assert_eq!(max_forwards.0, 1);
    ///
    /// // Third hop
    /// let max_forwards = max_forwards.decrement().unwrap();
    /// assert_eq!(max_forwards.0, 0);
    ///
    /// // Should not forward further
    /// assert!(max_forwards.decrement().is_none());
    /// ```
    pub fn decrement(self) -> Option<Self> {
        if self.0 > 0 {
            Some(Self(self.0 - 1))
        } else {
            None
        }
    }

    /// Checks if the value is zero.
    ///
    /// This method provides a convenient way to check if a request has reached
    /// its maximum number of allowed hops and should not be forwarded further.
    ///
    /// # Returns
    ///
    /// `true` if the Max-Forwards value is zero, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let max_forwards = MaxForwards::new(1);
    /// assert!(!max_forwards.is_zero());
    ///
    /// // After decrementing
    /// let max_forwards = max_forwards.decrement().unwrap();
    /// assert!(max_forwards.is_zero());
    ///
    /// // Example proxy logic
    /// if max_forwards.is_zero() {
    ///     // Return 483 Too Many Hops
    /// } else {
    ///     // Forward the request
    /// }
    /// ```
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for MaxForwards {
    /// Formats the Max-Forwards header value as a string.
    ///
    /// The format simply shows the integer value without any additional formatting.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let max_forwards = MaxForwards::new(70);
    /// assert_eq!(max_forwards.to_string(), "70");
    ///
    /// // Using in a formatted string
    /// let header = format!("Max-Forwards: {}", max_forwards);
    /// assert_eq!(header, "Max-Forwards: 70");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MaxForwards {
    type Err = Error;

    /// Parses a string into a Max-Forwards header value.
    ///
    /// This method parses a string containing an unsigned 8-bit integer (0-255) 
    /// into a MaxForwards struct.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed MaxForwards, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The input string cannot be parsed as a u8 integer
    /// - The input string contains non-numeric characters
    /// - The input value is outside the range of a u8 (0-255)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a valid value
    /// let max_forwards = MaxForwards::from_str("70").unwrap();
    /// assert_eq!(max_forwards.0, 70);
    ///
    /// // Parse with surrounding whitespace
    /// let max_forwards = MaxForwards::from_str("  42  ").unwrap();
    /// assert_eq!(max_forwards.0, 42);
    ///
    /// // Parse an invalid value
    /// let result = MaxForwards::from_str("not a number");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        s.trim().parse::<u8>()
            .map(MaxForwards)
            .map_err(|e| Error::ParseError(
                format!("Invalid Max-Forwards value: {}", e)
            ))
    }
} 