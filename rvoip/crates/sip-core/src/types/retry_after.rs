//! # SIP Retry-After Header
//!
//! This module provides an implementation of the SIP Retry-After header as defined in
//! [RFC 3261 Section 20.33](https://datatracker.ietf.org/doc/html/rfc3261#section-20.33).
//!
//! The Retry-After header field indicates how long a service is expected to be unavailable
//! to the requesting client. It can be used with 503 (Service Unavailable) responses to
//! indicate how long the service is expected to be unavailable, with 404 (Not Found),
//! 600 (Busy), or 603 (Decline) responses to indicate when the called party might be available.
//!
//! ## Purpose
//!
//! The Retry-After header serves several purposes:
//!
//! - Provides an estimated time when a service will be available again
//! - Indicates when a busy user might be available for a new call attempt
//! - Helps with load management during service maintenance or overload
//!
//! ## Format
//!
//! ```
//! Retry-After: 180
//! Retry-After: 3600 (1 hour)
//! Retry-After: 3600;duration=1800
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//! use std::time::Duration;
//!
//! // Create a basic Retry-After header with a 60-second delay
//! let retry = RetryAfter::new(60);
//!
//! // Create a Retry-After header from a Duration
//! let retry = RetryAfter::from_duration(Duration::from_secs(120));
//!
//! // Create a more detailed Retry-After header
//! let retry = RetryAfter::new(3600)
//!     .with_comment("System maintenance")
//!     .with_duration(7200)
//!     .with_param(Param::new("reason", Some("server-update")));
//!
//! // Parse from a string
//! let retry = RetryAfter::from_str("180 (3 minutes)").unwrap();
//! ```

// Retry-After header type for SIP messages
// Format defined in RFC 3261 Section 20.33

use std::fmt;
use std::time::Duration;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Serialize, Deserialize};
use crate::error::{Error, Result};
use crate::parser::headers::retry_after::{parse_retry_after, RetryParam};
use crate::types::param::Param;

/// RetryAfter represents a Retry-After header value
/// Used to indicate how long a service is expected to be unavailable
/// 
/// RFC 3261 Section 20.33:
/// Retry-After = "Retry-After" HCOLON delta-seconds [ comment ] *( SEMI retry-param )
/// retry-param = ("duration" EQUAL delta-seconds) / generic-param
///
/// The Retry-After header indicates the time interval after which the client should retry
/// a request that has been temporarily rejected. It's commonly used with:
///
/// - 503 (Service Unavailable) responses: Indicating when the service will be available
/// - 404 (Not Found) responses: Suggesting when to retry for a target that is temporarily unavailable
/// - 600 (Busy) / 603 (Decline) responses: Indicating when the called party might be available
///
/// # Components
/// 
/// - `delay`: Required delta-seconds value indicating how long to wait before retrying
/// - `comment`: Optional comment in parentheses (e.g., human-readable explanation)
/// - `duration`: Optional duration parameter indicating how long the unavailability will last
/// - `parameters`: Additional parameters that may be application-specific
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Simple usage
/// let retry = RetryAfter::new(180);
/// assert_eq!(retry.to_string(), "180");
///
/// // With comment
/// let retry = RetryAfter::new(3600).with_comment("Server maintenance");
/// assert_eq!(retry.to_string(), "3600 (Server maintenance)");
///
/// // With duration parameter
/// let retry = RetryAfter::new(60).with_duration(120);
/// assert_eq!(retry.to_string(), "60;duration=120");
///
/// // Parse from a string
/// let retry = RetryAfter::from_str("300 (5 minutes)").unwrap();
/// assert_eq!(retry.delay, 300);
/// assert_eq!(retry.comment, Some("5 minutes".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryAfter {
    /// The delay in seconds
    pub delay: u32,
    
    /// Optional comment (e.g., explaining why retry is needed)
    pub comment: Option<String>,
    
    /// Optional duration parameter (special case from retry-param)
    pub duration: Option<u32>,
    
    /// Other parameters
    pub parameters: Vec<Param>,
}

impl RetryAfter {
    /// Create a new RetryAfter with just a delay
    ///
    /// Creates a basic Retry-After header with the specified delay in seconds
    /// and no additional parameters.
    ///
    /// # Parameters
    ///
    /// - `delay`: The time in seconds to wait before retrying
    ///
    /// # Returns
    ///
    /// A new `RetryAfter` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create with a 60-second delay
    /// let retry = RetryAfter::new(60);
    /// assert_eq!(retry.delay, 60);
    /// assert_eq!(retry.to_string(), "60");
    ///
    /// // Create with a longer delay (1 hour)
    /// let retry = RetryAfter::new(3600);
    /// assert_eq!(retry.to_string(), "3600");
    /// ```
    pub fn new(delay: u32) -> Self {
        RetryAfter {
            delay,
            comment: None,
            duration: None,
            parameters: Vec::new(),
        }
    }
    
    /// Create a RetryAfter from a Duration
    ///
    /// Convenience method to create a RetryAfter header from a standard
    /// Rust Duration.
    ///
    /// # Parameters
    ///
    /// - `duration`: The Duration to convert into a RetryAfter
    ///
    /// # Returns
    ///
    /// A new `RetryAfter` instance with the delay set to the number of seconds in the Duration
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::time::Duration;
    ///
    /// // Create from a 2-minute duration
    /// let duration = Duration::from_secs(120);
    /// let retry = RetryAfter::from_duration(duration);
    /// assert_eq!(retry.delay, 120);
    ///
    /// // The original duration can be retrieved
    /// assert_eq!(retry.as_duration(), duration);
    /// ```
    pub fn from_duration(duration: Duration) -> Self {
        RetryAfter::new(duration.as_secs() as u32)
    }
    
    /// Add a comment to the RetryAfter
    ///
    /// Adds a human-readable comment explaining the retry situation.
    /// This comment will be enclosed in parentheses when formatted.
    ///
    /// # Parameters
    ///
    /// - `comment`: The comment text to add
    ///
    /// # Returns
    ///
    /// The modified `RetryAfter` instance (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Add a simple comment
    /// let retry = RetryAfter::new(60)
    ///     .with_comment("Server overloaded");
    /// assert_eq!(retry.to_string(), "60 (Server overloaded)");
    ///
    /// // More descriptive comment
    /// let retry = RetryAfter::new(3600)
    ///     .with_comment("System maintenance in progress");
    /// assert_eq!(retry.to_string(), "3600 (System maintenance in progress)");
    /// ```
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }
    
    /// Set the duration parameter
    ///
    /// Adds a 'duration' parameter to indicate how long the unavailability
    /// will last. This is different from the delay, which indicates when to
    /// retry. The duration parameter is a special case in the SIP specification.
    ///
    /// # Parameters
    ///
    /// - `duration`: The duration in seconds
    ///
    /// # Returns
    ///
    /// The modified `RetryAfter` instance (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // With retry in 60 seconds, outage will last 120 seconds
    /// let retry = RetryAfter::new(60)
    ///     .with_duration(120);
    /// assert_eq!(retry.to_string(), "60;duration=120");
    ///
    /// // More complex example
    /// let retry = RetryAfter::new(300)
    ///     .with_comment("Planned maintenance")
    ///     .with_duration(3600);
    /// assert_eq!(retry.to_string(), "300 (Planned maintenance);duration=3600");
    /// ```
    pub fn with_duration(mut self, duration: u32) -> Self {
        self.duration = Some(duration);
        self
    }
    
    /// Add a parameter to the RetryAfter
    ///
    /// Adds an additional parameter to the RetryAfter header.
    /// These can be vendor-specific or application-specific extensions.
    ///
    /// # Parameters
    ///
    /// - `param`: The parameter to add
    ///
    /// # Returns
    ///
    /// The modified `RetryAfter` instance (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Add a custom parameter
    /// let retry = RetryAfter::new(60)
    ///     .with_param(Param::new("reason", Some("maintenance")));
    /// assert_eq!(retry.to_string(), "60;reason=maintenance");
    ///
    /// // Add multiple parameters
    /// let retry = RetryAfter::new(60)
    ///     .with_param(Param::new("reason", Some("maintenance")))
    ///     .with_param(Param::new("priority", Some("high")));
    /// ```
    pub fn with_param(mut self, param: Param) -> Self {
        self.parameters.push(param);
        self
    }
    
    /// Get the delay as a Duration
    ///
    /// Converts the delay seconds into a Rust Duration object
    /// for easier handling in Rust code.
    ///
    /// # Returns
    ///
    /// A Duration representing the delay time
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::time::Duration;
    ///
    /// let retry = RetryAfter::new(120);
    /// let duration = retry.as_duration();
    /// assert_eq!(duration, Duration::from_secs(120));
    ///
    /// // Use with std library time functions
    /// let retry = RetryAfter::new(60);
    /// let retry_at = std::time::Instant::now() + retry.as_duration();
    /// ```
    pub fn as_duration(&self) -> Duration {
        Duration::from_secs(self.delay.into())
    }
    
    /// Get a parameter value by name (case-insensitive)
    ///
    /// Retrieves the value of a parameter with the given name.
    /// The parameter name matching is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look for
    ///
    /// # Returns
    ///
    /// The parameter value as a string slice if found, None otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let retry = RetryAfter::new(60)
    ///     .with_param(Param::new("reason", Some("maintenance")));
    ///
    /// // Case-insensitive parameter retrieval
    /// assert_eq!(retry.get_param("reason"), Some("maintenance"));
    /// assert_eq!(retry.get_param("REASON"), Some("maintenance"));
    /// assert_eq!(retry.get_param("unknown"), None);
    /// ```
    pub fn get_param(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        for param in &self.parameters {
            if let Param::Other(key, Some(value)) = param {
                if key.to_lowercase() == name_lower {
                    return value.as_str();
                }
            }
        }
        None
    }
    
    /// Check if a parameter exists (case-insensitive)
    ///
    /// Tests whether a parameter with the given name exists.
    /// The parameter name matching is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to check for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let retry = RetryAfter::new(60)
    ///     .with_param(Param::new("reason", Some("maintenance")));
    ///
    /// // Case-insensitive parameter checking
    /// assert!(retry.has_param("reason"));
    /// assert!(retry.has_param("REASON"));
    /// assert!(!retry.has_param("unknown"));
    ///
    /// // Use in conditional logic
    /// if retry.has_param("reason") {
    ///     // Handle parameter
    ///     let reason = retry.get_param("reason").unwrap();
    /// }
    /// ```
    pub fn has_param(&self, name: &str) -> bool {
        self.parameters.iter().any(|p| {
            if let Param::Other(key, _) = p {
                key.to_lowercase() == name.to_lowercase()
            } else {
                false
            }
        })
    }
}

impl FromStr for RetryAfter {
    type Err = Error;

    /// Parse a string into a RetryAfter
    ///
    /// Converts a string representation of a Retry-After header into a
    /// structured RetryAfter object.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed RetryAfter, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse simple delay
    /// let retry = RetryAfter::from_str("60").unwrap();
    /// assert_eq!(retry.delay, 60);
    ///
    /// // Parse with comment
    /// let retry = RetryAfter::from_str("3600 (1 hour)").unwrap();
    /// assert_eq!(retry.delay, 3600);
    /// assert_eq!(retry.comment, Some("1 hour".to_string()));
    ///
    /// // Parse with duration parameter
    /// let retry = RetryAfter::from_str("60;duration=3600").unwrap();
    /// assert_eq!(retry.delay, 60);
    /// assert_eq!(retry.duration, Some(3600));
    ///
    /// // Parse with additional parameters
    /// let retry = RetryAfter::from_str("60;reason=maintenance").unwrap();
    /// assert_eq!(retry.delay, 60);
    /// assert_eq!(retry.get_param("reason"), Some("maintenance"));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let result = all_consuming(parse_retry_after)(s.as_bytes())
            .map(|(_, value)| {
                let mut retry_after = RetryAfter::new(value.delay);
                
                if let Some(comment) = value.comment {
                    retry_after.comment = Some(comment);
                }
                
                for param in value.params {
                    match param {
                        RetryParam::Duration(duration) => {
                            retry_after.duration = Some(duration);
                        },
                        RetryParam::Generic(param) => {
                            retry_after.parameters.push(param);
                        }
                    }
                }
                
                retry_after
            })
            .map_err(|e| Error::from(e.to_owned()));
            
        result
    }
}

impl fmt::Display for RetryAfter {
    /// Formats the RetryAfter header as a string.
    ///
    /// Converts the header to its string representation, following
    /// the format specified in RFC 3261.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// // Simple delay
    /// let retry = RetryAfter::new(60);
    /// assert_eq!(retry.to_string(), "60");
    ///
    /// // With comment
    /// let retry = RetryAfter::new(120).with_comment("Server maintenance");
    /// assert_eq!(retry.to_string(), "120 (Server maintenance)");
    ///
    /// // With duration parameter
    /// let retry = RetryAfter::new(60).with_duration(120);
    /// assert_eq!(retry.to_string(), "60;duration=120");
    ///
    /// // Complex example
    /// let retry = RetryAfter::new(3600)
    ///     .with_comment("System upgrade")
    ///     .with_duration(7200)
    ///     .with_param(Param::new("reason", Some("maintenance")));
    /// assert_eq!(retry.to_string(), 
    ///            "3600 (System upgrade);duration=7200;reason=maintenance");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // First the delay
        write!(f, "{}", self.delay)?;
        
        // Optional comment
        if let Some(comment) = &self.comment {
            write!(f, " ({})", comment)?;
        }
        
        // Duration parameter if present
        if let Some(duration) = self.duration {
            write!(f, ";duration={}", duration)?;
        }
        
        // Other parameters
        for param in &self.parameters {
            write!(f, ";{}", param)?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};
    
    #[test]
    fn test_retry_after_display_simple() {
        let retry = RetryAfter::new(60);
        assert_eq!(retry.to_string(), "60");
    }
    
    #[test]
    fn test_retry_after_with_comment() {
        let retry = RetryAfter::new(120)
            .with_comment("Server maintenance");
        assert_eq!(retry.to_string(), "120 (Server maintenance)");
    }
    
    #[test]
    fn test_retry_after_with_duration() {
        let retry = RetryAfter::new(60)
            .with_duration(120);
        assert_eq!(retry.to_string(), "60;duration=120");
    }
    
    #[test]
    fn test_retry_after_with_params() {
        let retry = RetryAfter::new(60)
            .with_param(Param::Other("reason".to_string(), Some(GenericValue::Token("maintenance".to_string()))));
        assert_eq!(retry.to_string(), "60;reason=maintenance");
    }
    
    #[test]
    fn test_retry_after_complex() {
        let retry = RetryAfter::new(3600)
            .with_comment("System upgrade")
            .with_duration(7200)
            .with_param(Param::Other("reason".to_string(), Some(GenericValue::Token("maintenance".to_string()))));
        assert_eq!(retry.to_string(), "3600 (System upgrade);duration=7200;reason=maintenance");
    }
    
    #[test]
    fn test_retry_after_from_duration() {
        let duration = Duration::from_secs(300);
        let retry = RetryAfter::from_duration(duration);
        assert_eq!(retry.delay, 300);
        assert_eq!(retry.as_duration(), duration);
    }
    
    #[test]
    fn test_retry_after_from_str() {
        // Test basic parsing
        let retry = RetryAfter::from_str("60").unwrap();
        assert_eq!(retry.delay, 60);
        assert_eq!(retry.comment, None);
        assert_eq!(retry.duration, None);
        assert!(retry.parameters.is_empty());
        
        // Test with comment
        let retry = RetryAfter::from_str("120 (Server maintenance)").unwrap();
        assert_eq!(retry.delay, 120);
        assert_eq!(retry.comment, Some("Server maintenance".to_string()));
        
        // Test with duration
        let retry = RetryAfter::from_str("60;duration=120").unwrap();
        assert_eq!(retry.delay, 60);
        assert_eq!(retry.duration, Some(120));
        
        // Test with other parameters
        let retry = RetryAfter::from_str("60;reason=maintenance").unwrap();
        assert_eq!(retry.delay, 60);
        assert!(retry.has_param("reason"));
        assert_eq!(retry.get_param("reason"), Some("maintenance"));
        
        // Test complex case
        let retry = RetryAfter::from_str("3600 (System upgrade);duration=7200;reason=maintenance").unwrap();
        assert_eq!(retry.delay, 3600);
        assert_eq!(retry.comment, Some("System upgrade".to_string()));
        assert_eq!(retry.duration, Some(7200));
        assert!(retry.has_param("reason"));
    }
    
    #[test]
    fn test_retry_after_rfc_examples() {
        // Examples based on RFC 3261 Section 20.33
        let retry = RetryAfter::from_str("18000 (5 hours)").unwrap();
        assert_eq!(retry.delay, 18000);
        assert_eq!(retry.comment, Some("5 hours".to_string()));
        
        let retry = RetryAfter::from_str("120").unwrap();
        assert_eq!(retry.delay, 120);
        
        let retry = RetryAfter::from_str("3600;duration=1800").unwrap();
        assert_eq!(retry.delay, 3600);
        assert_eq!(retry.duration, Some(1800));
    }
    
    #[test]
    fn test_retry_after_invalid_input() {
        // Invalid delta-seconds
        assert!(RetryAfter::from_str("abc").is_err());
        
        // Invalid format
        assert!(RetryAfter::from_str("120 Server maintenance").is_err()); // Missing parentheses
        
        // Invalid parameter format
        assert!(RetryAfter::from_str("120;duration=abc").is_err()); // Non-numeric duration
        
        // Unclosed comment
        assert!(RetryAfter::from_str("120 (Server maintenance").is_err());
    }
} 