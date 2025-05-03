//! Allow header builder
//!
//! This module provides builder methods for the Allow header.

use crate::error::{Error, Result};
use crate::types::{
    allow::Allow,
    method::Method,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Allow header building capabilities to request and response builders
pub trait AllowBuilderExt {
    /// Add an Allow header with a single method
    fn allow_method(self, method: Method) -> Self;
    
    /// Add an Allow header with multiple methods
    fn allow_methods(self, methods: Vec<Method>) -> Self;
    
    /// Add an Allow header with standard methods for UA (User Agent) operations
    /// 
    /// This includes INVITE, ACK, CANCEL, BYE, and OPTIONS.
    fn allow_standard_methods(self) -> Self;
    
    /// Add an Allow header with all common SIP methods
    /// 
    /// This includes INVITE, ACK, CANCEL, BYE, OPTIONS, REGISTER, INFO, MESSAGE, 
    /// SUBSCRIBE, NOTIFY, REFER, PUBLISH, and UPDATE.
    fn allow_all_methods(self) -> Self;
}

impl<T> AllowBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn allow_method(self, method: Method) -> Self {
        let mut allow = Allow::new();
        allow.add_method(method);
        self.set_header(allow)
    }
    
    fn allow_methods(self, methods: Vec<Method>) -> Self {
        let mut allow = Allow::new();
        for method in methods {
            allow.add_method(method);
        }
        self.set_header(allow)
    }
    
    fn allow_standard_methods(self) -> Self {
        self.allow_methods(vec![
            Method::Invite,
            Method::Ack,
            Method::Cancel,
            Method::Bye,
            Method::Options,
        ])
    }
    
    fn allow_all_methods(self) -> Self {
        self.allow_methods(vec![
            Method::Invite,
            Method::Ack,
            Method::Cancel,
            Method::Bye,
            Method::Options,
            Method::Register,
            Method::Info,
            Method::Message,
            Method::Subscribe,
            Method::Notify,
            Method::Refer,
            Method::Publish,
            Method::Update,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_allow_method() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .allow_method(Method::Invite)
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Allow(allow)) = request.header(&HeaderName::Allow) {
            assert_eq!(allow.0.len(), 1);
            assert!(allow.allows(&Method::Invite));
        } else {
            panic!("Allow header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_allow_methods() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .allow_methods(vec![Method::Invite, Method::Bye])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Allow(allow)) = response.header(&HeaderName::Allow) {
            assert_eq!(allow.0.len(), 2);
            assert!(allow.allows(&Method::Invite));
            assert!(allow.allows(&Method::Bye));
            assert!(!allow.allows(&Method::Cancel));
        } else {
            panic!("Allow header not found or has wrong type");
        }
    }

    #[test]
    fn test_allow_standard_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .allow_standard_methods()
            .build();
            
        if let Some(TypedHeader::Allow(allow)) = request.header(&HeaderName::Allow) {
            assert_eq!(allow.0.len(), 5);
            assert!(allow.allows(&Method::Invite));
            assert!(allow.allows(&Method::Ack));
            assert!(allow.allows(&Method::Cancel));
            assert!(allow.allows(&Method::Bye));
            assert!(allow.allows(&Method::Options));
            assert!(!allow.allows(&Method::Refer));
        } else {
            panic!("Allow header not found or has wrong type");
        }
    }

    #[test]
    fn test_allow_all_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .allow_all_methods()
            .build();
            
        if let Some(TypedHeader::Allow(allow)) = request.header(&HeaderName::Allow) {
            assert_eq!(allow.0.len(), 13);
            // Check a few methods
            assert!(allow.allows(&Method::Invite));
            assert!(allow.allows(&Method::Register));
            assert!(allow.allows(&Method::Subscribe));
            assert!(allow.allows(&Method::Publish));
        } else {
            panic!("Allow header not found or has wrong type");
        }
    }
} 