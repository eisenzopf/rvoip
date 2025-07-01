use crate::error::{Error, Result};
use crate::types::{
    expires::Expires,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;
use std::time::Duration;

/// Expires header builder
///
/// This module provides builder methods for the Expires header in SIP messages.
///
/// ## SIP Expires Header Overview
///
/// The Expires header is defined in [RFC 3261 Section 20.19](https://datatracker.ietf.org/doc/html/rfc3261#section-20.19)
/// as part of the core SIP protocol. It specifies the relative time after which the message or content expires.
///
/// ## Format
///
/// ```text
/// Expires: 3600
/// ```
///
/// The value is a decimal integer number of seconds.
///
/// ## Purpose of Expires Header
///
/// The Expires header serves several important purposes in SIP:
///
/// 1. Limiting the validity duration of registrations (REGISTER requests)
/// 2. Setting the subscription duration (SUBSCRIBE requests)
/// 3. Setting the validity period of event state (NOTIFY requests)
/// 4. Limiting the validity of a SIP message (any request/response)
///
/// ## Common Values
///
/// - **0**: Indicates immediate expiration (often used to remove registrations or terminate subscriptions)
/// - **300-600**: Common for short-lived registrations or subscriptions (5-10 minutes)
/// - **3600**: Common for hourly registrations
/// - **86400**: Daily registrations (24 hours)
///
/// # Examples
///
/// ## REGISTER with One-Hour Registration
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
///
/// // Scenario: Client registering with a one-hour expiration
///
/// // Create a REGISTER request with a one-hour expiration
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     // Set registration to expire in one hour
///     .expires_seconds(3600)
///     .build();
///
/// // The registrar will remove this binding after one hour unless refreshed
/// ```
///
/// ## De-registration Using Zero Expiration
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
///
/// // Scenario: Client de-registering (removing registration)
///
/// // Create a REGISTER request to remove a registration
/// let deregister = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     // Set expiration to zero to remove the registration
///     .expires_zero()
///     .build();
///
/// // The registrar will immediately remove this binding
/// ```
///
/// ## SUBSCRIBE with Duration
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
/// use std::time::Duration;
///
/// // Scenario: Creating a presence subscription
///
/// // Create a SUBSCRIBE request for presence with a 30-minute duration
/// let subscribe = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("sub-1"))
///     .to("Bob", "sip:bob@example.com", None)
///     // Set subscription to expire in 30 minutes
///     .expires_duration(Duration::from_secs(30 * 60))
///     .build();
///
/// // The subscription will be active for 30 minutes unless refreshed
/// ```
pub trait ExpiresBuilderExt {
    /// Add an Expires header with a specified number of seconds
    ///
    /// This method adds an Expires header with the given number of seconds until expiration.
    /// This is the most common way to specify expiration time in SIP.
    ///
    /// # Parameters
    ///
    /// * `seconds` - The number of seconds until expiration
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Expires header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
    ///
    /// // Create a REGISTER request with a 1-hour registration
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("reg-1"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .expires_seconds(3600)
    ///     .build();
    ///
    /// // The registration will expire in one hour (3600 seconds)
    /// ```
    fn expires_seconds(self, seconds: u32) -> Self;
    
    /// Add an Expires header with a specified Duration
    ///
    /// This method adds an Expires header with expiration time specified as a std::time::Duration.
    /// The Duration is converted to seconds for the header.
    ///
    /// # Parameters
    ///
    /// * `duration` - The duration until expiration
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Expires header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
    /// use std::time::Duration;
    ///
    /// // Create a SUBSCRIBE request for dialog events that expires in 15 minutes
    /// let subscribe = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("sub-1"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .expires_duration(Duration::from_secs(15 * 60))
    ///     .build();
    ///
    /// // The subscription will expire in 15 minutes (900 seconds)
    /// ```
    fn expires_duration(self, duration: Duration) -> Self;
    
    /// Add an Expires header with zero value for immediate expiration
    ///
    /// This convenience method adds an Expires header with a value of 0, which
    /// indicates immediate expiration. This is commonly used for de-registration
    /// or for terminating subscriptions.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Expires header set to 0
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
    ///
    /// // Create a REGISTER request to remove a registration
    /// let deregister = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("reg-2"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .contact("<sip:alice@192.168.1.2:5060>", None)
    ///     .expires_zero()
    ///     .build();
    ///
    /// // The registration will be removed immediately
    /// ```
    fn expires_zero(self) -> Self;
    
    /// Add an Expires header with a standard one-hour expiration
    ///
    /// This convenience method adds an Expires header with a value of 3600 seconds (1 hour),
    /// which is a common standard duration for registrations and subscriptions.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Expires header set to 3600 seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
    ///
    /// // Create a REGISTER request with standard one-hour expiration
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("reg-3"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .expires_one_hour()
    ///     .build();
    ///
    /// // The registration will expire in one hour (3600 seconds)
    /// ```
    fn expires_one_hour(self) -> Self;
    
    /// Add an Expires header with a standard one-day expiration
    ///
    /// This convenience method adds an Expires header with a value of 86400 seconds (24 hours),
    /// which is often used for long-lasting registrations.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Expires header set to 86400 seconds
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ExpiresBuilderExt};
    ///
    /// // Create a REGISTER request with one-day expiration
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("reg-4"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .expires_one_day()
    ///     .build();
    ///
    /// // The registration will expire in 24 hours (86400 seconds)
    /// ```
    fn expires_one_day(self) -> Self;
}

impl<T> ExpiresBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn expires_seconds(self, seconds: u32) -> Self {
        let expires = Expires::new(seconds);
        self.set_header(expires)
    }
    
    fn expires_duration(self, duration: Duration) -> Self {
        // Convert Duration to seconds, capping at u32::MAX if necessary
        let seconds = duration.as_secs().min(u32::MAX as u64) as u32;
        self.expires_seconds(seconds)
    }
    
    fn expires_zero(self) -> Self {
        self.expires_seconds(0)
    }
    
    fn expires_one_hour(self) -> Self {
        self.expires_seconds(3600)
    }
    
    fn expires_one_day(self) -> Self {
        self.expires_seconds(86400)
    }
}

pub trait ExpiresExt {
    fn expires(self, delta_seconds: u32) -> Self;
}

impl<T: HeaderSetter> ExpiresExt for T {
    fn expires(self, delta_seconds: u32) -> Self {
        self.set_header(Expires(delta_seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;
    use std::time::Duration;
    use crate::builder::request::SimpleRequestBuilder;
    use crate::types::expires::Expires;
    use crate::types::headers::HeaderName;
    use crate::types::headers::header_access::HeaderAccess;

    #[test]
    fn test_request_expires_seconds() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .expires_seconds(3600)
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 3600);
        } else {
            panic!("Expires header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_expires_seconds() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .expires_seconds(1800)
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Expires(expires)) = response.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 1800);
        } else {
            panic!("Expires header not found or has wrong type");
        }
    }

    #[test]
    fn test_expires_duration() {
        // Test with a duration of 30 minutes
        let request = RequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
            .expires_duration(Duration::from_secs(30 * 60))
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 1800); // 30 minutes in seconds
        } else {
            panic!("Expires header not found or has wrong type");
        }
        
        // Test with a very large duration (should cap at u32::MAX)
        let large_duration = Duration::from_secs(u64::MAX);
        let request = RequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
            .expires_duration(large_duration)
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, u32::MAX);
        } else {
            panic!("Expires header not found or has wrong type");
        }
    }

    #[test]
    fn test_expires_convenience_methods() {
        // Test expires_zero
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .expires_zero()
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 0);
        } else {
            panic!("Expires header not found or has wrong type");
        }
        
        // Test expires_one_hour
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .expires_one_hour()
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 3600);
        } else {
            panic!("Expires header not found or has wrong type");
        }
        
        // Test expires_one_day
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .expires_one_day()
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.0, 86400);
        } else {
            panic!("Expires header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_expires_display() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .expires_seconds(7200)
            .build();
            
        if let Some(TypedHeader::Expires(expires)) = request.header(&HeaderName::Expires) {
            assert_eq!(expires.to_string(), "7200");
        } else {
            panic!("Expires header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_multiple_expires() {
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@biloxi.com")
            .unwrap()
            .expires(3600)
            .expires(0)
            .build();

        let header = request.typed_header::<Expires>();
        assert!(header.is_some(), "Expires header should be present");
        assert_eq!(header.unwrap().0, 0, "Expires header should be 0 after second set");
    }
} 