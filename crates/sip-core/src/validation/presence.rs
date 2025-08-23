//! Validation for SIMPLE presence messages
//!
//! This module provides validation functions for PUBLISH, SUBSCRIBE, and NOTIFY
//! requests as defined in RFC 3903 and RFC 6665.

use crate::types::sip_request::Request;
use crate::types::Method;
use crate::types::headers::{HeaderName, TypedHeader};
use crate::{Error, Result};

/// Validates a PUBLISH request according to RFC 3903
///
/// # Requirements (RFC 3903)
///
/// - Event header MUST be present
/// - If body is present, Content-Type MUST be present
/// - For refresh/modify operations, SIP-If-Match SHOULD be present
/// - Expires header indicates publication lifetime (0 means remove)
///
/// # Parameters
///
/// - `request`: The PUBLISH request to validate
///
/// # Returns
///
/// - `Ok(())` if the request is valid
/// - `Err(Error)` describing the validation failure
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::validation::presence::validate_publish_request;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
/// 
/// let request = SimpleRequestBuilder::new(Method::Publish, "sip:alice@example.com")
///     .unwrap()
///     .event("presence")
///     .from("Alice", "sip:alice@example.com", None)
///     .to("Alice", "sip:alice@example.com", None)
///     .call_id("publish-test")
///     .cseq(1)
///     .build();
///     
/// assert!(validate_publish_request(&request).is_ok());
/// ```
pub fn validate_publish_request(request: &Request) -> Result<()> {
    // Check that this is actually a PUBLISH request
    if request.method != Method::Publish {
        return Err(Error::ValidationError(format!(
            "Expected PUBLISH method, got {:?}",
            request.method
        )));
    }
    
    // Event header MUST be present (RFC 3903 Section 4)
    let has_event = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::Event(_)) || h.name() == HeaderName::Event
    });
    
    if !has_event {
        return Err(Error::ValidationError(
            "PUBLISH request MUST contain Event header (RFC 3903 Section 4)".to_string()
        ));
    }
    
    // If body is present, Content-Type MUST be present
    if !request.body.is_empty() {
        let has_content_type = request.headers.iter().any(|h| {
            matches!(h, TypedHeader::ContentType(_)) || h.name() == HeaderName::ContentType
        });
        
        if !has_content_type {
            return Err(Error::ValidationError(
                "PUBLISH request with body MUST contain Content-Type header".to_string()
            ));
        }
    }
    
    // Check for conditional request (SIP-If-Match)
    let has_sip_if_match = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::SipIfMatch(_)) || h.name() == HeaderName::SipIfMatch
    });
    
    // If this is a refresh/modify (has SIP-If-Match), validate entity tag format
    if has_sip_if_match {
        // Entity tag validation could be enhanced here
        // For now, we just check that it exists
    }
    
    Ok(())
}

/// Validates a SUBSCRIBE request according to RFC 6665
///
/// # Requirements (RFC 6665)
///
/// - Event header MUST be present
/// - Expires header MUST be present (0 means unsubscribe)
/// - Accept header MAY be present to indicate acceptable NOTIFY body types
///
/// # Parameters
///
/// - `request`: The SUBSCRIBE request to validate
///
/// # Returns
///
/// - `Ok(())` if the request is valid
/// - `Err(Error)` describing the validation failure
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::validation::presence::validate_subscribe_request;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
/// 
/// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@example.com")
///     .unwrap()
///     .event("presence")
///     .expires(3600)
///     .from("Alice", "sip:alice@example.com", None)
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("subscribe-test")
///     .cseq(1)
///     .build();
///     
/// assert!(validate_subscribe_request(&request).is_ok());
/// ```
pub fn validate_subscribe_request(request: &Request) -> Result<()> {
    // Check that this is actually a SUBSCRIBE request
    if request.method != Method::Subscribe {
        return Err(Error::ValidationError(format!(
            "Expected SUBSCRIBE method, got {:?}",
            request.method
        )));
    }
    
    // Event header MUST be present (RFC 6665 Section 7.1)
    let has_event = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::Event(_)) || h.name() == HeaderName::Event
    });
    
    if !has_event {
        return Err(Error::ValidationError(
            "SUBSCRIBE request MUST contain Event header (RFC 6665 Section 7.1)".to_string()
        ));
    }
    
    // Expires header MUST be present (RFC 6665 Section 7.1)
    let has_expires = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::Expires(_)) || h.name() == HeaderName::Expires
    });
    
    if !has_expires {
        return Err(Error::ValidationError(
            "SUBSCRIBE request MUST contain Expires header (RFC 6665 Section 7.1)".to_string()
        ));
    }
    
    // Check if this is an initial SUBSCRIBE (no To tag)
    // Initial SUBSCRIBEs create dialogs, refreshes occur within dialogs
    let to_header = request.headers.iter().find(|h| {
        matches!(h, TypedHeader::To(_)) || h.name() == HeaderName::To
    });
    
    if let Some(TypedHeader::To(to)) = to_header {
        // If To has a tag, this is a refresh/unsubscribe within a dialog
        // Different validation rules may apply
    }
    
    Ok(())
}

/// Validates a NOTIFY request according to RFC 6665
///
/// # Requirements (RFC 6665)
///
/// - Event header MUST be present
/// - Subscription-State header MUST be present
/// - Must be sent within a dialog (has To tag)
/// - Content-Type required if body is present
///
/// # Parameters
///
/// - `request`: The NOTIFY request to validate
///
/// # Returns
///
/// - `Ok(())` if the request is valid
/// - `Err(Error)` describing the validation failure
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::validation::presence::validate_notify_request;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
/// 
/// let request = SimpleRequestBuilder::new(Method::Notify, "sip:alice@192.168.1.10")
///     .unwrap()
///     .event("presence")
///     .subscription_state("active;expires=3599")
///     .from("Bob", "sip:bob@example.com", Some("tag123"))
///     .to("Alice", "sip:alice@example.com", Some("tag456"))  // Must have To tag for dialog
///     .call_id("notify-test")
///     .cseq(1)
///     .build();
///     
/// assert!(validate_notify_request(&request).is_ok());
/// ```
pub fn validate_notify_request(request: &Request) -> Result<()> {
    // Check that this is actually a NOTIFY request
    if request.method != Method::Notify {
        return Err(Error::ValidationError(format!(
            "Expected NOTIFY method, got {:?}",
            request.method
        )));
    }
    
    // Event header MUST be present (RFC 6665 Section 7.1)
    let has_event = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::Event(_)) || h.name() == HeaderName::Event
    });
    
    if !has_event {
        return Err(Error::ValidationError(
            "NOTIFY request MUST contain Event header (RFC 6665 Section 7.1)".to_string()
        ));
    }
    
    // Subscription-State header MUST be present (RFC 6665 Section 7.2.1)
    let has_subscription_state = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::SubscriptionState(_)) || h.name() == HeaderName::SubscriptionState
    });
    
    if !has_subscription_state {
        return Err(Error::ValidationError(
            "NOTIFY request MUST contain Subscription-State header (RFC 6665 Section 7.2.1)".to_string()
        ));
    }
    
    // NOTIFY must be sent within a dialog (should have To tag)
    let to_header = request.headers.iter().find(|h| {
        matches!(h, TypedHeader::To(_)) || h.name() == HeaderName::To
    });
    
    if let Some(TypedHeader::To(to)) = to_header {
        if to.tag().is_none() {
            return Err(Error::ValidationError(
                "NOTIFY request MUST be sent within a dialog (To header must have tag)".to_string()
            ));
        }
    }
    
    // If body is present, Content-Type MUST be present
    if !request.body.is_empty() {
        let has_content_type = request.headers.iter().any(|h| {
            matches!(h, TypedHeader::ContentType(_)) || h.name() == HeaderName::ContentType
        });
        
        if !has_content_type {
            return Err(Error::ValidationError(
                "NOTIFY request with body MUST contain Content-Type header".to_string()
            ));
        }
    }
    
    Ok(())
}

/// Validates SIP-If-Match conditional requests
///
/// According to RFC 3903, SIP-If-Match is used for conditional PUBLISH
/// requests to ensure the client has the current state before modifying it.
///
/// # Parameters
///
/// - `request`: The request to check for conditional requirements
///
/// # Returns
///
/// - `Ok(true)` if this is a conditional request with valid SIP-If-Match
/// - `Ok(false)` if this is not a conditional request
/// - `Err(Error)` if the conditional request is malformed
pub fn validate_conditional_request(request: &Request) -> Result<bool> {
    // Only PUBLISH uses SIP-If-Match
    if request.method != Method::Publish {
        return Ok(false);
    }
    
    let has_sip_if_match = request.headers.iter().any(|h| {
        matches!(h, TypedHeader::SipIfMatch(_)) || h.name() == HeaderName::SipIfMatch
    });
    
    if has_sip_if_match {
        // This is a conditional request
        // Could add additional validation of the entity tag format here
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::event::{Event, EventType};
    use crate::types::expires::Expires;
    use crate::types::subscription_state::SubscriptionState;
    use crate::types::from::From;
    use crate::types::to::To;
    use crate::types::call_id::CallId;
    use crate::types::cseq::CSeq;
    use crate::types::max_forwards::MaxForwards;
    use crate::types::via::Via;
    
    #[test]
    fn test_validate_publish_request_valid() {
        let mut request = Request::new(Method::Publish, "sip:alice@example.com".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        
        assert!(validate_publish_request(&request).is_ok());
    }
    
    #[test]
    fn test_validate_publish_request_missing_event() {
        let request = Request::new(Method::Publish, "sip:alice@example.com".parse().unwrap());
        
        let result = validate_publish_request(&request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Event header"));
    }
    
    #[test]
    fn test_validate_publish_request_with_body_missing_content_type() {
        let mut request = Request::new(Method::Publish, "sip:alice@example.com".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        request.body = b"<presence/>".to_vec().into();
        
        let result = validate_publish_request(&request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Content-Type"));
    }
    
    #[test]
    fn test_validate_subscribe_request_valid() {
        let mut request = Request::new(Method::Subscribe, "sip:bob@example.com".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        request.headers.push(TypedHeader::Expires(Expires::new(3600)));
        
        assert!(validate_subscribe_request(&request).is_ok());
    }
    
    #[test]
    fn test_validate_subscribe_request_missing_expires() {
        let mut request = Request::new(Method::Subscribe, "sip:bob@example.com".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        
        let result = validate_subscribe_request(&request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expires header"));
    }
    
    #[test]
    fn test_validate_notify_request_valid() {
        let mut request = Request::new(Method::Notify, "sip:alice@192.168.1.10".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        request.headers.push(TypedHeader::SubscriptionState(
            SubscriptionState::active(3599).to_string()
        ));
        
        // NOTIFY must be in a dialog, so To must have a tag
        let to = To::new("sip:alice@example.com".parse().unwrap())
            .with_tag("xyz123");
        request.headers.push(TypedHeader::To(to));
        
        assert!(validate_notify_request(&request).is_ok());
    }
    
    #[test]
    fn test_validate_notify_request_missing_subscription_state() {
        let mut request = Request::new(Method::Notify, "sip:alice@192.168.1.10".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        
        let result = validate_notify_request(&request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Subscription-State"));
    }
    
    #[test]
    fn test_validate_notify_request_no_dialog() {
        let mut request = Request::new(Method::Notify, "sip:alice@192.168.1.10".parse().unwrap());
        request.headers.push(TypedHeader::Event(Event::new(EventType::Token("presence".to_string()))));
        request.headers.push(TypedHeader::SubscriptionState(
            SubscriptionState::active(3599).to_string()
        ));
        
        // To without tag means no dialog
        let to = To::new("sip:alice@example.com".parse().unwrap());
        request.headers.push(TypedHeader::To(to));
        
        let result = validate_notify_request(&request);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("within a dialog"));
    }
}