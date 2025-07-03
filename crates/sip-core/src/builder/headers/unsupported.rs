use crate::error::{Error, Result};
use crate::types::{
    unsupported::Unsupported,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Unsupported header builder
///
/// This module provides builder methods for the Unsupported header in SIP messages.
///
/// ## SIP Unsupported Header Overview
///
/// The Unsupported header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.40)
/// as part of the core SIP protocol. It is used in 420 (Bad Extension) responses to indicate
/// which SIP extensions requested in a Require or Proxy-Require header cannot be supported
/// by the server.
///
/// ## Purpose of Unsupported Header
///
/// The Unsupported header serves several important purposes in SIP:
///
/// 1. It informs clients which requested extensions cannot be supported by a server
/// 2. It is mandatory in 420 (Bad Extension) responses to explain the failure reason
/// 3. It helps clients make informed decisions about fallback behavior
/// 4. It allows servers to explicitly reject features they don't implement
///
/// ## Common Usage Scenarios
///
/// - **Rejecting calls with required extensions**: When a UAS can't support a feature in a Require header
/// - **Proxy rejection**: When a proxy can't support features in a Proxy-Require header
/// - **Graceful degradation**: Informing clients which features to avoid in future requests
/// - **Compatibility management**: Allowing older servers to interoperate with newer clients
/// - **Security boundaries**: Rejecting potentially dangerous or unauthorized extensions
///
/// ## Relationship with other headers
///
/// - **Unsupported** vs **Supported**: Unsupported lists rejected features, Supported lists implemented features
/// - **Unsupported** and **Require**: Unsupported lists rejected option tags from the Require header
/// - **Unsupported** in 420 responses: Required when rejecting requests with unsupported extensions
///
/// # Examples
///
/// ## Basic 420 Bad Extension Response
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
///
/// // Scenario: Server cannot support required 100rel extension
///
/// // Original request (not shown) contained:
/// // Require: 100rel
///
/// // Create a 420 Bad Extension response
/// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("Extension Not Supported"))
///     // From header copied from the request
///     .from("Bob", "sip:bob@example.com", Some("a73kszlfl"))
///     // To header copied from the request, no tag as no dialog is created
///     .to("Alice", "sip:alice@example.com", None)
///     // Mandatory Unsupported header listing the rejected extensions
///     .unsupported_100rel()
///     .build();
///
/// // The client will understand which feature caused the rejection
/// // and may retry without requiring that feature
/// ```
///
/// ## Rejecting Multiple Extensions
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
///
/// // Scenario: Basic SIP gateway rejecting advanced features
///
/// // Original request (not shown) contained:
/// // Require: 100rel, timer, resource-priority
///
/// // Create a 420 response for multiple unsupported extensions
/// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("Multiple Extensions Not Supported"))
///     .from("Gateway", "sip:gateway.example.com", None)
///     .to("User", "sip:user@example.org", None)
///     // List all extensions that cannot be supported
///     .unsupported_tags(vec!["100rel", "timer", "resource-priority"])
///     .build();
///
/// // The client knows all three extensions caused problems
/// // and might retry with a simpler feature set
/// ```
///
/// ## SIP Interoperability Testing
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
///
/// // Scenario: SIP testing tool checking extension support
///
/// // Test extension support by creating various 420 responses
/// // based on probe requests with different Require headers
///
/// // Create a response for an older device that doesn't support timers
/// let timer_response = SimpleResponseBuilder::new(StatusCode::BadExtension, None)
///     .from("TestServer", "sip:test.example.com", None)
///     .to("TestClient", "sip:client.example.org", None)
///     .unsupported_timer()
///     .build();
///
/// // Create a response for a device that doesn't support Path header
/// let path_response = SimpleResponseBuilder::new(StatusCode::BadExtension, None)
///     .from("TestServer", "sip:test.example.com", None)
///     .to("TestClient", "sip:client.example.org", None)
///     .unsupported_path()
///     .build();
///
/// // The testing tool can now determine which extensions
/// // are supported by the device under test
/// ```
pub trait UnsupportedBuilderExt {
    /// Add an Unsupported header with a single option tag
    ///
    /// This method adds an Unsupported header with a single SIP extension option tag
    /// that cannot be supported by the server. This is typically used in 420 (Bad Extension)
    /// responses to reject requests containing a Require or Proxy-Require header with
    /// extensions that cannot be supported.
    ///
    /// # Parameters
    ///
    /// * `option_tag` - The SIP extension option tag that cannot be supported
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Unsupported header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
    ///
    /// // Simple proxy rejecting a request with Require: gruu
    /// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("GRUU Not Supported"))
    ///     .from("Proxy", "sip:proxy.example.com", None)
    ///     .to("User", "sip:user@example.org", None)
    ///     // Indicate GRUU (globally routable user agent URIs) is not supported
    ///     .unsupported_tag("gruu")
    ///     .build();
    ///
    /// // The client must retry without requiring GRUU if it wants the request to succeed
    /// ```
    fn unsupported_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add an Unsupported header with multiple option tags
    ///
    /// This method adds an Unsupported header with multiple SIP extension option tags
    /// that cannot be supported by the server. This is used when rejecting requests
    /// that require multiple extensions that cannot be supported.
    ///
    /// # Parameters
    ///
    /// * `option_tags` - A vector of SIP extension option tags that cannot be supported
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Unsupported header containing all specified tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
    ///
    /// // Legacy SIP server rejecting WebRTC features
    /// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("WebRTC Features Not Supported"))
    ///     .from("Server", "sip:oldserver.example.com", None)
    ///     .to("WebClient", "sip:webclient@example.org", None)
    ///     // List all WebRTC-related features that cannot be supported
    ///     .unsupported_tags(vec![
    ///         "ice",          // Interactive Connectivity Establishment
    ///         "replaces",     // Call transfer with Replaces
    ///         "outbound",     // Connection reuse
    ///         "gruu"          // Globally routable URIs
    ///     ])
    ///     .build();
    ///
    /// // The WebRTC client knows it must use a gateway that can
    /// // translate between WebRTC and traditional SIP
    /// ```
    fn unsupported_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add an Unsupported header for 100rel (reliable provisional responses)
    ///
    /// This convenience method adds an Unsupported header indicating that the
    /// 100rel extension (reliable provisional responses) defined in RFC 3262
    /// cannot be supported. This is commonly used when a UAS or proxy cannot
    /// implement the reliability mechanisms for 1xx responses.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with 100rel added to the Unsupported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
    ///
    /// // Scenario: Basic SIP phone that can't handle reliable provisional responses
    ///
    /// // Create a 420 response rejecting the 100rel extension
    /// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("Reliable Provisionals Not Supported"))
    ///     .from("Phone", "sip:phone@example.com", None)
    ///     .to("Caller", "sip:caller@example.net", None)
    ///     // Indicate reliable provisional responses are not supported
    ///     .unsupported_100rel()
    ///     .build();
    ///
    /// // The caller should retry without Require: 100rel
    /// // or may decide to use a different calling method
    /// ```
    fn unsupported_100rel(self) -> Self;
    
    /// Add an Unsupported header for timer
    ///
    /// This convenience method adds an Unsupported header indicating that the
    /// timer extension (session timers) defined in RFC 4028 cannot be supported.
    /// This is used when a UAS or proxy cannot implement session timers for
    /// dialog keepalive and expiration.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with timer added to the Unsupported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
    ///
    /// // Scenario: Basic B2BUA that doesn't implement session timers
    ///
    /// // Create a 420 response rejecting the timer extension
    /// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("Session Timers Not Supported"))
    ///     .from("B2BUA", "sip:b2bua.example.com", None)
    ///     .to("Caller", "sip:caller@example.org", None)
    ///     // Indicate session timers are not supported
    ///     .unsupported_timer()
    ///     .build();
    ///
    /// // The caller should retry without Require: timer
    /// // or may need to handle dialog refreshes via other means
    /// ```
    fn unsupported_timer(self) -> Self;
    
    /// Add an Unsupported header for path
    ///
    /// This convenience method adds an Unsupported header indicating that the
    /// path extension defined in RFC 3327 cannot be supported. This is used when
    /// a registrar or proxy cannot implement the Path header mechanism for
    /// routing through NATs or firewalls.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with path added to the Unsupported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::UnsupportedBuilderExt};
    ///
    /// // Scenario: Legacy registrar that doesn't understand Path headers
    ///
    /// // Create a 420 response rejecting the path extension
    /// let response = SimpleResponseBuilder::new(StatusCode::BadExtension, Some("Path Not Supported"))
    ///     .from("Registrar", "sip:registrar.example.com", None)
    ///     .to("User", "sip:user@example.net", None)
    ///     // Indicate Path header is not supported
    ///     .unsupported_path()
    ///     .build();
    ///
    /// // The client may need to use a different registrar
    /// // or find an alternative way to handle NAT traversal
    /// ```
    fn unsupported_path(self) -> Self;
}

impl<T> UnsupportedBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn unsupported_tag(self, option_tag: impl Into<String>) -> Self {
        let mut unsupported = Unsupported::new();
        let tag_str = option_tag.into();
        unsupported.add_option_tag(&tag_str);
        self.set_header(unsupported)
    }
    
    fn unsupported_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut unsupported = Unsupported::new();
        for tag_impl in option_tags {
            let tag = tag_impl.into();
            unsupported.add_option_tag(&tag);
        }
        self.set_header(unsupported)
    }
    
    fn unsupported_100rel(self) -> Self {
        self.unsupported_tag("100rel")
    }
    
    fn unsupported_timer(self) -> Self {
        self.unsupported_tag("timer")
    }
    
    fn unsupported_path(self) -> Self {
        self.unsupported_tag("path")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_unsupported_tag() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("100rel"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_unsupported_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .unsupported_tags(vec!["100rel", "path"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Unsupported(unsupported)) = response.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 2);
            assert!(unsupported.has_option_tag("100rel"));
            assert!(unsupported.has_option_tag("path"));
            assert!(!unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_unsupported_convenience_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_timer()
            .build();
            
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_unsupported_multiple_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_timer()
            .unsupported_100rel()
            .build();
        
        // When adding multiple headers with the same name, they get added as separate headers
        // rather than being merged. The header() method returns the first one it finds.
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
        
        // Verify that there are actually two Unsupported headers
        let unsupported_headers: Vec<_> = request.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::Unsupported(u) => Some(u),
                _ => None
            })
            .collect();
        
        assert_eq!(unsupported_headers.len(), 2);
        assert!(unsupported_headers[0].has_option_tag("timer"));
        assert!(unsupported_headers[1].has_option_tag("100rel"));
    }
} 