use crate::error::{Error, Result};
use crate::types::{
    allow::Allow,
    method::Method,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;
/// Allow header builder
///
/// This module provides builder methods for the Allow header in SIP messages.
///
/// ## SIP Allow Header Overview
///
/// The Allow header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.5)
/// as part of the core SIP protocol. It lists the set of methods supported by the User Agent or server
/// that generated the message.
///
/// ## Purpose of Allow Header
///
/// The Allow header serves several important purposes in SIP:
///
/// 1. It informs other entities about the methods a UA can process
/// 2. It's mandatory in 405 (Method Not Allowed) responses to indicate valid methods
/// 3. It's commonly included in responses to OPTIONS requests
/// 4. It enables capabilities advertisement and method negotiation
///
/// ## Common Use Cases
///
/// - **Basic UA capabilities**: Standard clients typically support INVITE, ACK, CANCEL, BYE, and OPTIONS
/// - **Advanced capabilities**: Full-featured UAs may also support MESSAGE, REFER, SUBSCRIBE, etc.
/// - **Method Not Allowed**: When rejecting requests with 405 responses
/// - **Session border controllers**: SBCs often filter methods based on policy
/// - **Proxy capabilities**: Proxies can advertise supported extension methods
///
/// ## Relationship with other headers
///
/// - **Allow** vs **Supported**: Allow lists methods, while Supported lists extension names
/// - **Allow** in 405 responses: Must be included when rejecting methods
/// - **Allow** in OPTIONS: Typically included for capabilities discovery
///
///
/// # Examples
///
/// ## 405 Method Not Allowed Response
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
///
/// // Scenario: Server receives a MESSAGE request but doesn't support it
///
/// // Create a 405 Method Not Allowed response with required Allow header
/// let response = SimpleResponseBuilder::new(StatusCode::MethodNotAllowed, Some("Method Not Allowed"))
///     // From header copied from the request
///     .from("Bob", "sip:bob@example.com", Some("a73kszlfl"))
///     // To header copied from the request
///     .to("Alice", "sip:alice@example.com", None)
///     // Allow header is mandatory in 405 responses
///     .allow_standard_methods()
///     .build();
///
/// // The response indicates to the client which methods are acceptable
/// // so it can retry with an appropriate method if needed
/// ```
///
/// ## OPTIONS Response with Complete Method List
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
///
/// // Scenario: Server responds to an OPTIONS request
///
/// // Create a response to OPTIONS with complete capabilities
/// let options_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("Server", "sip:pbx.example.com", Some("xyz123"))
///     .to("Client", "sip:client@example.org", Some("abc456"))
///     // Include all methods the server supports
///     .allow_all_methods()
///     // Additional capability headers would be included here
///     // (e.g., Supported, Accept, Accept-Language, etc.)
///     .build();
///
/// // The client can now understand the full capabilities of the server
/// // and make appropriate decisions about which methods to use
/// ```
///
/// ## SIP Trunk Configuration
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AllowBuilderExt};
///
/// // Scenario: Enterprise PBX registers with SIP trunk provider
///
/// // The initial REGISTER includes Allow to indicate supported methods
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:sip-trunk.example.com")
///     .unwrap()
///     .from("PBX", "sip:pbx.enterprise.com", Some("reg-1"))
///     .to("PBX", "sip:pbx.enterprise.com", None)
///     .contact("<sip:pbx.enterprise.com:5060;transport=tls>", None)
///     // Specify exactly which methods this PBX supports
///     .allow_methods(vec![
///         Method::Invite,
///         Method::Ack,
///         Method::Cancel,
///         Method::Bye,
///         Method::Options,
///         Method::Refer,  // Supports call transfers
///         Method::Update  // Supports session updates
///     ])
///     .build();
///
/// // The trunk provider can use this information for policy enforcement
/// // and to understand what features the enterprise PBX supports
/// ```
pub trait AllowBuilderExt {
    /// Add an Allow header with a single method
    ///
    /// This method adds an Allow header containing a single SIP method to the message.
    /// This is rarely used since entities typically support multiple methods, but
    /// can be useful for highly specialized SIP elements.
    ///
    /// # Parameters
    ///
    /// * `method` - The SIP method to include in the Allow header
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Allow header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
    ///
    /// // A specialized event notification server that only handles SUBSCRIBE/NOTIFY
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Event Server", "sip:events.example.com", None)
    ///     .to("Client", "sip:client@example.org", None)
    ///     .allow_method(Method::Subscribe)
    ///     .build();
    /// ```
    fn allow_method(self, method: Method) -> Self;
    
    /// Add an Allow header with multiple methods
    ///
    /// This method adds an Allow header containing multiple SIP methods to the message.
    /// It gives precise control over which methods are advertised as supported.
    ///
    /// # Parameters
    ///
    /// * `methods` - A vector of SIP methods to include in the Allow header
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Allow header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
    ///
    /// // A basic SIP gateway that supports calling features but not messaging
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Gateway", "sip:gateway.example.com", None)
    ///     .to("Client", "sip:client@example.org", None)
    ///     .allow_methods(vec![
    ///         Method::Invite,
    ///         Method::Ack,
    ///         Method::Cancel,
    ///         Method::Bye,
    ///         Method::Options,
    ///         Method::Update
    ///     ])
    ///     .build();
    /// ```
    fn allow_methods(self, methods: Vec<Method>) -> Self;
    
    /// Add an Allow header with standard methods for UA (User Agent) operations
    /// 
    /// This method adds an Allow header with the five core SIP methods that all
    /// compliant user agents must support according to RFC 3261:
    /// INVITE, ACK, CANCEL, BYE, and OPTIONS.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Allow header containing standard methods
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
    ///
    /// // A typical SIP phone responding to OPTIONS
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Phone", "sip:phone@example.com", None)
    ///     .to("Server", "sip:server.example.org", None)
    ///     // Standard methods supported by basic SIP phones
    ///     .allow_standard_methods()
    ///     .build();
    /// ```
    fn allow_standard_methods(self) -> Self;
    
    /// Add an Allow header with all common SIP methods
    /// 
    /// This method adds an Allow header with a comprehensive set of SIP methods,
    /// including both core methods and common extensions. This includes:
    /// INVITE, ACK, CANCEL, BYE, OPTIONS, REGISTER, INFO, MESSAGE, 
    /// SUBSCRIBE, NOTIFY, REFER, PUBLISH, and UPDATE.
    ///
    /// This is typically used by full-featured SIP user agents or servers
    /// that support all standard SIP extensions.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Allow header containing all common methods
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AllowBuilderExt};
    ///
    /// // A full-featured SIP softphone responding to OPTIONS
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Softphone", "sip:user@example.com", None)
    ///     .to("Server", "sip:server.example.org", None)
    ///     // Complete set of methods for a feature-rich client
    ///     .allow_all_methods()
    ///     .build();
    ///
    /// // The response indicates that this client supports all standard
    /// // SIP methods including messaging, presence, and call transfers
    /// ```
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