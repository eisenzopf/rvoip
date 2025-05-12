use crate::error::{Error, Result};
use crate::types::{
    retry_after::RetryAfter,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use std::time::Duration;
use super::HeaderSetter;

/// RetryAfter header builder
///
/// This module provides builder methods for the Retry-After header in SIP responses.
///
/// ## SIP Retry-After Header Overview
///
/// The Retry-After header is defined in [RFC 3261 Section 20.33](https://datatracker.ietf.org/doc/html/rfc3261#section-20.33)
/// and provides information about when a client should retry a request after receiving certain response codes.
///
/// ## Purpose of Retry-After Header
///
/// The Retry-After header serves several important purposes in SIP:
///
/// 1. Indicates how long a service is expected to be unavailable to the requesting client
/// 2. Provides guidance on when to retry a failed request 
/// 3. Helps with load balancing and traffic management during service outages
/// 4. Can include additional context like maintenance duration or reason for unavailability
///
/// ## Common Usage Scenarios
///
/// - **503 (Service Unavailable)**: Indicates when the service will be available again
/// - **404 (Not Found)**: Suggests when to retry for a target that is temporarily unavailable
/// - **480 (Temporarily Unavailable)**: Indicates when the user might be available
/// - **486 (Busy Here)** or **600 (Busy Everywhere)**: Suggests when to retry calling a busy user
/// - **500 (Server Internal Error)**: Indicates when the server might recover
///
/// ## Examples
///
/// ## Service Unavailable with Retry Time
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
///
/// // Scenario: Server is temporarily unavailable for 5 minutes
///
/// // Create a 503 Service Unavailable response with Retry-After
/// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, Some("Service Unavailable"))
///     .from("SIP Server", "sip:server@example.com", Some("to-tag"))
///     .to("Client", "sip:client@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Register)
///     // Add Retry-After header with 300 seconds (5 minutes)
///     .retry_after(300)
///     .build();
/// ```
///
/// ## Busy User with Comment
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
///
/// // Scenario: Called party is busy but will be available in 10 minutes
///
/// // Create a 486 Busy Here response with Retry-After
/// let response = SimpleResponseBuilder::new(StatusCode::BusyHere, Some("Busy Here"))
///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Invite)
///     // Add Retry-After with comment
///     .retry_after_with_comment(600, "In a meeting")
///     .build();
/// ```
///
/// ## Maintenance Window with Duration
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
/// use std::time::Duration;
///
/// // Scenario: Server undergoing maintenance for 2 hours, retry after 30 minutes
///
/// // Create a 503 Service Unavailable response with detailed Retry-After
/// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, Some("Maintenance"))
///     .from("SIP Server", "sip:server@example.com", Some("to-tag"))
///     .to("Client", "sip:client@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Subscribe)
///     // Add Retry-After with duration parameter (retry in 30 min, outage lasts 2 hours)
///     .retry_after_duration(
///         30 * 60,         // 30 minutes in seconds
///         120 * 60,        // 2 hours in seconds
///         Some("System maintenance")
///     )
///     .build();
/// ```
///
/// ## Using Duration Object for Timing
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
/// use std::time::Duration;
///
/// // Scenario: Service will be down for 1 hour
///
/// // Create a response with Retry-After using Duration
/// let down_time = Duration::from_secs(60 * 60); // 1 hour
/// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
///     .retry_after_from_duration(down_time)
///     .build();
///
/// // Or with a comment
/// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
///     .retry_after_duration_with_comment(down_time, "Database maintenance")
///     .build();
/// ```
pub trait RetryAfterBuilderExt {
    /// Add a Retry-After header with a delay in seconds
    ///
    /// This method adds a Retry-After header with the specified delay in seconds.
    ///
    /// # Parameters
    ///
    /// * `seconds` - The number of seconds to wait before retrying
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Retry-After header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
    ///
    /// // Adding a Retry-After header with 180 seconds (3 minutes)
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .retry_after(180)
    ///     .build();
    ///
    /// // The response now contains a Retry-After: 180 header
    /// ```
    fn retry_after(self, seconds: u32) -> Self;
    
    /// Add a Retry-After header with a delay specified as a Duration
    ///
    /// This method adds a Retry-After header using a standard Rust Duration
    /// object to specify the delay.
    ///
    /// # Parameters
    ///
    /// * `duration` - The Duration to wait before retrying
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Retry-After header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
    /// use std::time::Duration;
    ///
    /// // Using a Duration object to specify a 5-minute retry delay
    /// let delay = Duration::from_secs(5 * 60);
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .retry_after_from_duration(delay)
    ///     .build();
    ///
    /// // The response contains Retry-After: 300
    /// ```
    fn retry_after_from_duration(self, duration: Duration) -> Self;
    
    /// Add a Retry-After header with a delay and a comment
    ///
    /// This method adds a Retry-After header with the specified delay and
    /// an explanatory comment.
    ///
    /// # Parameters
    ///
    /// * `seconds` - The number of seconds to wait before retrying
    /// * `comment` - A human-readable comment explaining the delay
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Retry-After header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
    ///
    /// // Adding a Retry-After with explanatory comment
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .retry_after_with_comment(3600, "Server maintenance")
    ///     .build();
    ///
    /// // The response contains Retry-After: 3600 (Server maintenance)
    /// ```
    fn retry_after_with_comment(self, seconds: u32, comment: &str) -> Self;
    
    /// Add a Retry-After header with a delay, duration parameter, and optional comment
    ///
    /// This method adds a Retry-After header with the specified retry delay,
    /// a duration parameter indicating how long the condition will last,
    /// and an optional comment.
    ///
    /// # Parameters
    ///
    /// * `seconds` - The number of seconds to wait before retrying
    /// * `duration` - How long (in seconds) the condition will last
    /// * `comment` - Optional human-readable comment explaining the delay
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Retry-After header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
    ///
    /// // Adding a Retry-After with duration parameter
    /// // (retry in 60s, condition lasts 3600s total, with comment)
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .retry_after_duration(60, 3600, Some("System upgrade"))
    ///     .build();
    ///
    /// // The response contains Retry-After: 60 (System upgrade);duration=3600
    /// ```
    fn retry_after_duration(self, seconds: u32, duration: u32, comment: Option<&str>) -> Self;
    
    /// Add a Retry-After header with a Duration and a comment
    ///
    /// This method adds a Retry-After header using a standard Rust Duration
    /// object to specify the delay, along with an explanatory comment.
    ///
    /// # Parameters
    ///
    /// * `duration` - The Duration to wait before retrying
    /// * `comment` - A human-readable comment explaining the delay
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Retry-After header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RetryAfterBuilderExt};
    /// use std::time::Duration;
    ///
    /// // Using a Duration object with comment
    /// let delay = Duration::from_secs(30 * 60); // 30 minutes
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .retry_after_duration_with_comment(delay, "Database maintenance")
    ///     .build();
    ///
    /// // The response contains Retry-After: 1800 (Database maintenance)
    /// ```
    fn retry_after_duration_with_comment(self, duration: Duration, comment: &str) -> Self;
}

impl<T> RetryAfterBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn retry_after(self, seconds: u32) -> Self {
        let retry_after = RetryAfter::new(seconds);
        self.set_header(retry_after)
    }
    
    fn retry_after_from_duration(self, duration: Duration) -> Self {
        let retry_after = RetryAfter::from_duration(duration);
        self.set_header(retry_after)
    }
    
    fn retry_after_with_comment(self, seconds: u32, comment: &str) -> Self {
        let retry_after = RetryAfter::new(seconds).with_comment(comment);
        self.set_header(retry_after)
    }
    
    fn retry_after_duration(self, seconds: u32, duration: u32, comment: Option<&str>) -> Self {
        let mut retry_after = RetryAfter::new(seconds).with_duration(duration);
        
        if let Some(cmt) = comment {
            retry_after = retry_after.with_comment(cmt);
        }
        
        self.set_header(retry_after)
    }
    
    fn retry_after_duration_with_comment(self, duration: Duration, comment: &str) -> Self {
        let retry_after = RetryAfter::from_duration(duration).with_comment(comment);
        self.set_header(retry_after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;
    use std::time::Duration;

    #[test]
    fn test_response_retry_after_simple() {
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after(60)
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 60);
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_retry_after_from_duration() {
        let duration = Duration::from_secs(120);
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after_from_duration(duration)
            .build();
            
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 120);
            assert_eq!(retry_after.as_duration(), duration);
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_retry_after_with_comment() {
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after_with_comment(300, "System maintenance")
            .build();
            
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 300);
            assert_eq!(retry_after.comment, Some("System maintenance".to_string()));
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_retry_after_duration() {
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after_duration(60, 3600, Some("Server update"))
            .build();
            
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 60);
            assert_eq!(retry_after.duration, Some(3600));
            assert_eq!(retry_after.comment, Some("Server update".to_string()));
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_retry_after_duration_no_comment() {
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after_duration(60, 3600, None)
            .build();
            
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 60);
            assert_eq!(retry_after.duration, Some(3600));
            assert_eq!(retry_after.comment, None);
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_retry_after_duration_with_comment() {
        let duration = Duration::from_secs(1800); // 30 minutes
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .retry_after_duration_with_comment(duration, "Database maintenance")
            .build();
            
        if let Some(TypedHeader::RetryAfter(retry_after)) = response.header(&HeaderName::RetryAfter) {
            assert_eq!(retry_after.delay, 1800);
            assert_eq!(retry_after.comment, Some("Database maintenance".to_string()));
        } else {
            panic!("RetryAfter header not found or has wrong type");
        }
    }
} 