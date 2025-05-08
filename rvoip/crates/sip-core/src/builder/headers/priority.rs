use crate::error::{Error, Result};
use crate::types::{
    priority::Priority,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Priority header builder
///
/// This module provides builder methods for the Priority header in SIP messages.
///
/// ## SIP Priority Header Overview
///
/// The Priority header is defined in [RFC 3261 Section 20.26](https://datatracker.ietf.org/doc/html/rfc3261#section-20.26)
/// as part of the core SIP protocol. It indicates the urgency or importance of a request as perceived
/// by the client. This helps receiving UAs or proxies to appropriately prioritize signaling operations.
///
/// ## Purpose of Priority Header
///
/// The Priority header serves several important purposes in SIP:
///
/// 1. It indicates the urgency of a request to user agents
/// 2. It guides proxy forwarding behavior for resource allocation
/// 3. It helps prioritize limited resources during processing
/// 4. It can influence queue processing in high-load situations
///
/// ## Standard Priority Values
///
/// SIP defines four standard priority values, in decreasing order of urgency:
///
/// - **emergency**: Emergency sessions that involve human safety
/// - **urgent**: Urgent sessions that must be answered immediately
/// - **normal**: Normal sessions with no particular urgency (default)
/// - **non-urgent**: Sessions that do not require immediate response
///
/// ## Extended Priority Values
///
/// Per RFC 3261, the Priority field can also accept:
///
/// - Numeric priority values (lower values indicate higher priority)
/// - Extension tokens for application-specific priority levels
///
/// ## Relationship with other headers
///
/// - **Priority** vs **Resource-Priority**: Priority is a standard SIP header for general urgency,
///   while Resource-Priority (RFC 4412) provides more detailed priority for specific resources
/// - **Priority** vs **Call-Info**: Priority indicates urgency, while Call-Info can provide
///   additional context about the call purpose
///
/// # Examples
///
/// ## Emergency Call
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
///
/// // Scenario: Emergency call to a service center
///
/// // Create an INVITE with emergency priority
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:emergency@service-center.example.com").unwrap()
///     .from("Caller", "sip:caller@example.com", Some("emerg123"))
///     .to("Emergency", "sip:emergency@service-center.example.com", None)
///     .contact("<sip:caller@192.0.2.1:5060>", None)
///     // Set emergency priority
///     .priority_emergency()
///     .build();
///
/// // The emergency service center will prioritize this call
/// ```
///
/// ## Different Priority Levels
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
///
/// // Create a MESSAGE with urgent priority
/// let urgent_message = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("msg1"))
///     .to("Support", "sip:support@example.com", None)
///     .priority_urgent()
///     .build();
///
/// // Create a MESSAGE with normal priority (default)
/// let normal_message = SimpleRequestBuilder::new(Method::Message, "sip:info@example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("msg2"))
///     .to("Info", "sip:info@example.com", None)
///     .priority_normal()
///     .build();
///
/// // Create a MESSAGE with non-urgent priority
/// let non_urgent_message = SimpleRequestBuilder::new(Method::Message, "sip:feedback@example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("msg3"))
///     .to("Feedback", "sip:feedback@example.com", None)
///     .priority_non_urgent()
///     .build();
/// ```
///
/// ## Custom Priority Values
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
///
/// // Create an INVITE with numeric priority
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("call1"))
///     .to("Bob", "sip:bob@example.com", None)
///     .priority_numeric(2) // Priority level 2 (equivalent to normal)
///     .build();
///
/// // Create a MESSAGE with custom token priority
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("msg4"))
///     .to("Support", "sip:support@example.com", None)
///     .priority_token("high-priority")
///     .build();
/// ```
pub trait PriorityBuilderExt {
    /// Add a Priority header with the specified priority value
    ///
    /// This method adds a Priority header with the specified priority value.
    /// It's the most general method that allows setting any priority type.
    ///
    /// # Parameters
    ///
    /// * `priority` - The Priority enum value to use
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create an INVITE with emergency priority
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:emergency@example.com").unwrap()
    ///     .from("Caller", "sip:caller@example.com", Some("emerg1"))
    ///     .to("Emergency", "sip:emergency@example.com", None)
    ///     .priority(Priority::Emergency)
    ///     .build();
    ///
    /// // The message now has Priority: emergency
    /// ```
    fn priority(self, priority: Priority) -> Self;
    
    /// Add a Priority header with emergency priority
    ///
    /// This convenience method adds a Priority header with the emergency value.
    /// Emergency priority indicates the request is related to human safety or
    /// has similar urgency requiring immediate handling.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to emergency
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create an emergency call
    /// let emergency_call = SimpleRequestBuilder::new(Method::Invite, "sip:911@emergency-services.example.com").unwrap()
    ///     .from("Caller", "sip:caller@example.com", Some("911call"))
    ///     .to("Emergency", "sip:911@emergency-services.example.com", None)
    ///     .priority_emergency()
    ///     .build();
    ///
    /// // The message now has Priority: emergency
    /// ```
    fn priority_emergency(self) -> Self;
    
    /// Add a Priority header with urgent priority
    ///
    /// This convenience method adds a Priority header with the urgent value.
    /// Urgent priority indicates the request requires immediate attention
    /// but is not related to an emergency situation.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to urgent
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create an urgent message to support
    /// let urgent_message = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("urgent1"))
    ///     .to("Support", "sip:support@example.com", None)
    ///     .priority_urgent()
    ///     .body("System is down, requires immediate attention!")
    ///     .build();
    ///
    /// // The message now has Priority: urgent
    /// ```
    fn priority_urgent(self) -> Self;
    
    /// Add a Priority header with normal priority
    ///
    /// This convenience method adds a Priority header with the normal value.
    /// Normal priority is the default level for standard communications
    /// with no special urgency requirements.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to normal
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create a standard call with normal priority
    /// let call = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("call1"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .priority_normal()
    ///     .build();
    ///
    /// // The message now has Priority: normal
    /// ```
    fn priority_normal(self) -> Self;
    
    /// Add a Priority header with non-urgent priority
    ///
    /// This convenience method adds a Priority header with the non-urgent value.
    /// Non-urgent priority indicates the request does not require immediate
    /// attention and can be handled when resources are available.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to non-urgent
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create a non-urgent feedback message
    /// let feedback = SimpleRequestBuilder::new(Method::Message, "sip:feedback@example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("fb1"))
    ///     .to("Feedback", "sip:feedback@example.com", None)
    ///     .priority_non_urgent()
    ///     .body("I have some suggestions for improving your service...")
    ///     .build();
    ///
    /// // The message now has Priority: non-urgent
    /// ```
    fn priority_non_urgent(self) -> Self;
    
    /// Add a Priority header with a numeric priority value
    ///
    /// This method adds a Priority header with a numeric value.
    /// Lower numbers indicate higher priority in SIP.
    ///
    /// # Parameters
    ///
    /// * `value` - The numeric priority value (lower is higher priority)
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to the numeric value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create a message with custom numeric priority
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("msg1"))
    ///     .to("Support", "sip:support@example.com", None)
    ///     .priority_numeric(1) // Higher priority than normal (2)
    ///     .body("This is important but not quite urgent")
    ///     .build();
    ///
    /// // The message now has Priority: 1
    /// ```
    fn priority_numeric(self, value: u8) -> Self;
    
    /// Add a Priority header with a custom token value
    ///
    /// This method adds a Priority header with a custom token value.
    /// This allows for application-specific priority schemes.
    ///
    /// # Parameters
    ///
    /// * `token` - The custom priority token
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Priority header set to the token value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PriorityBuilderExt};
    ///
    /// // Create a message with custom token priority
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("msg1"))
    ///     .to("Support", "sip:support@example.com", None)
    ///     .priority_token("high-priority")
    ///     .body("This uses a custom priority scheme")
    ///     .build();
    ///
    /// // The message now has Priority: high-priority
    /// ```
    fn priority_token(self, token: impl Into<String>) -> Self;
}

impl<T> PriorityBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn priority(self, priority: Priority) -> Self {
        self.set_header(priority)
    }
    
    fn priority_emergency(self) -> Self {
        self.priority(Priority::Emergency)
    }
    
    fn priority_urgent(self) -> Self {
        self.priority(Priority::Urgent)
    }
    
    fn priority_normal(self) -> Self {
        self.priority(Priority::Normal)
    }
    
    fn priority_non_urgent(self) -> Self {
        self.priority(Priority::NonUrgent)
    }
    
    fn priority_numeric(self, value: u8) -> Self {
        self.priority(Priority::Other(value))
    }
    
    fn priority_token(self, token: impl Into<String>) -> Self {
        self.priority(Priority::Token(token.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_priority_standard_values() {
        // Test emergency priority
        let request = RequestBuilder::new(Method::Invite, "sip:emergency@example.com").unwrap()
            .priority_emergency()
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Emergency);
        } else {
            panic!("Priority header not found or has wrong type");
        }
        
        // Test urgent priority
        let request = RequestBuilder::new(Method::Invite, "sip:urgent@example.com").unwrap()
            .priority_urgent()
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Urgent);
        } else {
            panic!("Priority header not found or has wrong type");
        }
        
        // Test normal priority
        let request = RequestBuilder::new(Method::Invite, "sip:normal@example.com").unwrap()
            .priority_normal()
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Normal);
        } else {
            panic!("Priority header not found or has wrong type");
        }
        
        // Test non-urgent priority
        let request = RequestBuilder::new(Method::Invite, "sip:non-urgent@example.com").unwrap()
            .priority_non_urgent()
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::NonUrgent);
        } else {
            panic!("Priority header not found or has wrong type");
        }
    }

    #[test]
    fn test_priority_custom_values() {
        // Test numeric priority
        let request = RequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
            .priority_numeric(5)
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Other(5));
        } else {
            panic!("Priority header not found or has wrong type");
        }
        
        // Test token priority
        let request = RequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
            .priority_token("high-priority")
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Token("high-priority".to_string()));
        } else {
            panic!("Priority header not found or has wrong type");
        }
    }

    #[test]
    fn test_priority_in_response() {
        // Test priority in response
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .priority_urgent()
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = response.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Urgent);
        } else {
            panic!("Priority header not found or has wrong type");
        }
    }

    #[test]
    fn test_priority_direct() {
        // Test direct priority setting
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .priority(Priority::Emergency)
            .build();
            
        if let Some(TypedHeader::Priority(priority)) = request.header(&HeaderName::Priority) {
            assert_eq!(priority.clone(), Priority::Emergency);
        } else {
            panic!("Priority header not found or has wrong type");
        }
    }
} 