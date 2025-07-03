use crate::error::{Error, Result};
use crate::types::{
    ProxyRequire,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Proxy-Require header builder
///
/// This module provides builder methods for the Proxy-Require header in SIP messages.
///
/// ## SIP Proxy-Require Header Overview
///
/// The Proxy-Require header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.29)
/// as part of the core SIP protocol. It identifies SIP extensions that must be supported by
/// all proxies in the request path. If any proxy in the path cannot support these extensions, it
/// MUST reject the request with a 420 (Bad Extension) response.
///
/// ## Purpose of Proxy-Require Header
///
/// The Proxy-Require header serves critical purposes in SIP:
///
/// 1. It ensures all proxies in the request path support mandatory extensions
/// 2. It allows clients to guarantee certain behaviors from all proxies
/// 3. It provides a mechanism for enforcing security or policy requirements
/// 4. It enables proper handling of extension features that proxies must process
///
/// ## Difference Between Proxy-Require and Require
///
/// - **Proxy-Require**: Targets SIP proxies in the request path
/// - **Require**: Targets the final recipient of the request (UAS)
///
/// ## Common Usage Scenarios
///
/// - **Security mechanisms**: Requiring secure signaling through all proxies via sec-agree
/// - **Resource priority**: Ensuring emergency or priority traffic handling by all proxies
/// - **Path routing**: Requiring proxies to support Path header processing
/// - **QoS preconditions**: Ensuring all proxies understand QoS requirements
/// - **Access control**: Enforcing specific extension support for enterprise networks
///
/// ## When to Use Proxy-Require
///
/// The Proxy-Require header should be used carefully since it will cause the request to fail if 
/// any proxy in the path doesn't support the required extensions. It's best used in controlled 
/// environments where you know the capabilities of all proxies, or when an extension is absolutely
/// necessary for proper operation.
///
/// ## Relationship with other headers
///
/// - **Proxy-Require** vs **Require**: Proxy-Require affects proxies; Require affects the endpoint
/// - **Proxy-Require** and **Unsupported**: When a proxy can't support a required extension, it responds with 420 and lists the extensions in Unsupported
/// - **Proxy-Require** vs **Supported**: Proxy-Require mandates support; Supported merely indicates capabilities
///
/// # Examples
///
/// ## Secure Enterprise SIP Signaling
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
///
/// // Scenario: Enterprise requiring security agreements for all proxy hops
///
/// // Create a REGISTER requiring security agreement support from all proxies
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:enterprise.example.com").unwrap()
///     .from("User", "sip:user@enterprise.example.com", Some("reg1"))
///     .to("User", "sip:user@enterprise.example.com", None)
///     .contact("<sip:user@10.0.0.1:5060;transport=tls>", None)
///     // Add Security-Client header for security mechanism negotiation
///     .header(TypedHeader::Other(
///         HeaderName::Other("Security-Client".to_string()),
///         HeaderValue::Raw("tls".as_bytes().to_vec())
///     ))
///     // Require sec-agree from all proxies
///     .proxy_require_sec_agree()
///     .build();
///
/// // If any proxy in the path doesn't support sec-agree, it will
/// // reject with 420 Bad Extension and the registration will fail
/// ```
///
/// ## Emergency Services Call Handling
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
///
/// // Scenario: Emergency call requiring priority handling by all proxies
///
/// // Create an emergency INVITE requiring resource priority handling
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:911@emergency-services.gov").unwrap()
///     .from("Caller", "sip:caller@example.com", Some("emerg"))
///     .to("911", "sip:911@emergency-services.gov", None)
///     .contact("<sip:caller@192.0.2.1:5060>", None)
///     // Add Resource-Priority header for emergency call priority
///     .header(TypedHeader::Other(
///         HeaderName::Other("Resource-Priority".to_string()),
///         HeaderValue::Raw("emergency.0".as_bytes().to_vec())
///     ))
///     // Require all proxies to support resource priority
///     .proxy_require_resource_priority()
///     .build();
///
/// // All proxies must honor the resource priority
/// // or the call will fail with 420 Bad Extension
/// ```
///
/// ## Quality of Service Preconditions
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
///
/// // Scenario: Video conferencing system requiring QoS support
///
/// // Create an INVITE with QoS preconditions for all proxies
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:conference@example.org").unwrap()
///     .from("Presenter", "sip:presenter@company.com", Some("vid1"))
///     .to("Conference", "sip:conference@example.org", None)
///     .contact("<sip:presenter@192.0.2.5:5060>", None)
///     // Require preconditions support from all proxies
///     .proxy_require_precondition()
///     .build();
///
/// // This ensures all proxies understand and can properly handle
/// // the QoS requirements for the video conference
/// ```
pub trait ProxyRequireBuilderExt {
    /// Add a Proxy-Require header with a single option tag
    ///
    /// This method adds a Proxy-Require header with a single SIP extension option tag
    /// that must be supported by all proxies in the request path. If any proxy cannot
    /// support the extension, it must reject the request with a 420 response.
    ///
    /// # Parameters
    ///
    /// * `option_tag` - The SIP extension option tag that all proxies must support
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Create a SIP request that requires all proxies to support a custom extension
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     // Require a custom extension for all proxies in the path
    ///     .proxy_require_tag("my-extension")
    ///     .build();
    ///
    /// // All proxies must understand and implement "my-extension"
    /// // or reject the request with 420 Bad Extension
    /// ```
    fn proxy_require_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Proxy-Require header with multiple option tags
    ///
    /// This method adds a Proxy-Require header with multiple SIP extension option tags
    /// that must all be supported by all proxies in the request path. If any proxy does
    /// not support any of these extensions, it must reject the request with a 420 response
    /// listing all unsupported extensions.
    ///
    /// # Parameters
    ///
    /// * `option_tags` - A vector of SIP extension option tags that all proxies must support
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header containing all specified tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Create a request requiring multiple extensions from all proxies
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("reg42"))
    ///     .to("User", "sip:user@example.com", None)
    ///     .contact("<sip:user@192.0.2.1:5060>", None)
    ///     // Require multiple extensions from all proxies
    ///     .proxy_require_tags(vec![
    ///         "sec-agree",          // Security mechanisms
    ///         "path",               // Path header support
    ///         "resource-priority"   // Priority handling
    ///     ])
    ///     .build();
    ///
    /// // If any proxy cannot support all three extensions,
    /// // it must reject with 420 and list unsupported extensions
    /// ```
    fn proxy_require_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Proxy-Require header for sec-agree (security agreement)
    ///
    /// This convenience method adds a Proxy-Require header with the sec-agree extension
    /// defined in RFC 3329. This requires all proxies in the path to support the SIP
    /// Security Agreement mechanism for ensuring secure SIP signaling.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header for sec-agree
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Scenario: Financial institution requiring secure signaling throughout the path
    ///
    /// // Create a secure REGISTER requiring security agreement from all proxies
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:bank.example.com").unwrap()
    ///     .from("Agent", "sip:agent@bank.example.com", Some("sec1"))
    ///     .to("Agent", "sip:agent@bank.example.com", None)
    ///     .contact("<sip:agent@10.0.0.5:5060;transport=tls>", None)
    ///     // Add Security-Client header with mechanism preferences
    ///     .header(TypedHeader::Other(
    ///         HeaderName::Other("Security-Client".to_string()),
    ///         HeaderValue::Raw("tls; q=1.0, digest; q=0.8".as_bytes().to_vec())
    ///     ))
    ///     // Require security agreement from all proxies
    ///     .proxy_require_sec_agree()
    ///     .build();
    ///
    /// // All proxies must support the security agreement mechanism
    /// // to properly negotiate security with this client
    /// ```
    fn proxy_require_sec_agree(self) -> Self;
    
    /// Add a Proxy-Require header for precondition
    ///
    /// This convenience method adds a Proxy-Require header with the precondition extension
    /// defined in RFC 3312. This requires all proxies to support the SIP Preconditions
    /// Framework for quality of service and resource reservation.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header for precondition
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Scenario: High-quality audio call requiring QoS guarantees
    ///
    /// // Create an INVITE with QoS preconditions
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:recording-studio@example.com").unwrap()
    ///     .from("Producer", "sip:producer@music.example.org", Some("hifi"))
    ///     .to("Studio", "sip:recording-studio@example.com", None)
    ///     .contact("<sip:producer@192.0.2.55:5060>", None)
    ///     // Require preconditions support from all proxies
    ///     .proxy_require_precondition()
    ///     .build();
    ///
    /// // All proxies must understand QoS preconditions
    /// // to properly provision network resources for the call
    /// ```
    fn proxy_require_precondition(self) -> Self;
    
    /// Add a Proxy-Require header for path
    ///
    /// This convenience method adds a Proxy-Require header with the path extension
    /// defined in RFC 3327. This requires all proxies to support the Path header
    /// mechanism for proper routing in registration scenarios.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header for path
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Scenario: Mobile client traversing multiple carrier network boundaries
    ///
    /// // Create a REGISTER requiring Path support from all proxies
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:home-network.example.com").unwrap()
    ///     .from("Mobile", "sip:user@home-network.example.com", Some("roam"))
    ///     .to("Mobile", "sip:user@home-network.example.com", None)
    ///     .contact("<sip:user@10.0.1.2:5060>", None)
    ///     // Require path header support from all proxies
    ///     .proxy_require_path()
    ///     .build();
    ///
    /// // All proxies in the visited and home networks must support
    /// // Path headers for this registration to work correctly
    /// ```
    fn proxy_require_path(self) -> Self;
    
    /// Add a Proxy-Require header for resource priority
    ///
    /// This convenience method adds a Proxy-Require header with the resource-priority extension
    /// defined in RFC 4412. This requires all proxies to support the Resource-Priority header
    /// for emergency or priority communications.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Proxy-Require header for resource-priority
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyRequireBuilderExt};
    ///
    /// // Scenario: Government emergency notification system
    ///
    /// // Create an INVITE for broadcasting emergency information
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:broadcast@emergency.gov").unwrap()
    ///     .from("EOC", "sip:emergency-ops@gov.example", Some("alert"))
    ///     .to("Broadcast", "sip:broadcast@emergency.gov", None)
    ///     .contact("<sip:emergency-ops@192.0.2.100:5060>", None)
    ///     // Add Resource-Priority header with emergency level
    ///     .header(TypedHeader::Other(
    ///         HeaderName::Other("Resource-Priority".to_string()),
    ///         HeaderValue::Raw("emergency.4".as_bytes().to_vec())
    ///     ))
    ///     // Require resource priority from all proxies
    ///     .proxy_require_resource_priority()
    ///     .build();
    ///
    /// // All proxies must support and honor the resource priority
    /// // to ensure this emergency call gets proper treatment
    /// ```
    fn proxy_require_resource_priority(self) -> Self;
}

impl<T> ProxyRequireBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn proxy_require_tag(self, option_tag: impl Into<String>) -> Self {
        let proxy_require = ProxyRequire::single(&option_tag.into());
        self.set_header(proxy_require)
    }
    
    fn proxy_require_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let tags: Vec<String> = option_tags.into_iter().map(Into::into).collect();
        let proxy_require = ProxyRequire::with_options(&tags);
        self.set_header(proxy_require)
    }
    
    fn proxy_require_sec_agree(self) -> Self {
        self.proxy_require_tag("sec-agree")
    }
    
    fn proxy_require_precondition(self) -> Self {
        self.proxy_require_tag("precondition")
    }
    
    fn proxy_require_path(self) -> Self {
        self.proxy_require_tag("path")
    }
    
    fn proxy_require_resource_priority(self) -> Self {
        self.proxy_require_tag("resource-priority")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_proxy_require_tag() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .proxy_require_tag("sec-agree")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = request.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 1);
            assert!(proxy_require.has_option("sec-agree"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_proxy_require_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .proxy_require_tags(vec!["sec-agree", "precondition"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = response.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 2);
            assert!(proxy_require.has_option("sec-agree"));
            assert!(proxy_require.has_option("precondition"));
            assert!(!proxy_require.has_option("path"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_proxy_require_convenience_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .proxy_require_path()
            .build();
            
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = request.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 1);
            assert!(proxy_require.has_option("path"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }
} 