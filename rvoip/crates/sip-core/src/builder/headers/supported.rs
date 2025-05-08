use crate::error::{Error, Result};
use crate::types::{
    supported::Supported,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Supported header builder
///
/// This module provides builder methods for the Supported header in SIP messages.
///
/// ## SIP Supported Header Overview
///
/// The Supported header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.37)
/// as part of the core SIP protocol. It lists SIP extensions and features that are supported by the
/// User Agent or server. These are identified by option tags defined in various SIP RFCs.
///
/// ## Purpose of Supported Header
///
/// The Supported header serves several important purposes in SIP:
///
/// 1. It indicates which SIP extensions a UA can support during a transaction or dialog
/// 2. It allows capability discovery and feature negotiation between SIP entities
/// 3. It enables graceful fallback when communicating with less capable implementations
/// 4. It works with the Require and Proxy-Require headers for mandatory feature negotiation
///
/// ## Common Option Tags
///
/// - **100rel**: Support for reliable provisional responses ([RFC 3262](https://datatracker.ietf.org/doc/html/rfc3262))
/// - **path**: Support for the Path header mechanism ([RFC 3327](https://datatracker.ietf.org/doc/html/rfc3327))
/// - **timer**: Support for session timers ([RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028))
/// - **replaces**: Support for the Replaces header for dialog replacement ([RFC 3891](https://datatracker.ietf.org/doc/html/rfc3891))
/// - **outbound**: Support for managing NAT/firewall connections ([RFC 5626](https://datatracker.ietf.org/doc/html/rfc5626))
/// - **gruu**: Support for Globally Routable User Agent URIs ([RFC 5627](https://datatracker.ietf.org/doc/html/rfc5627))
/// - **ice**: Support for Interactive Connectivity Establishment (common in WebRTC)
///
/// ## Relationship with other headers
///
/// - **Supported** vs **Allow**: Supported lists extension features, while Allow lists methods
/// - **Supported** vs **Require**: Supported indicates capabilities, Require mandates support
/// - **Supported** vs **Unsupported**: Unsupported indicates features that cannot be supported
///
/// # Examples
///
/// ## REGISTER with Path and Outbound Support
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
///
/// // Scenario: UA registering through a NAT with Path and Outbound support
///
/// // Create a REGISTER request with Path and Outbound support
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060;ob>", None) // 'ob' parameter indicates outbound
///     // Indicate support for both Path header and Outbound connection reuse
///     .supported_tags(vec!["path", "outbound"])
///     .build();
///
/// // The registrar can now use Path headers and will understand Outbound connection management
/// ```
///
/// ## INVITE with Reliable Provisional Responses
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
///
/// // Scenario: Making a call with support for reliable provisional responses
///
/// // Create an INVITE with 100rel support
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("inv-1"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@203.0.113.5:5060>", None)
///     // Support reliable provisional responses (like 180 Ringing)
///     .supported_100rel()
///     .build();
///
/// // When the remote side sends 180 Ringing, it can mark it as reliable
/// // by adding a Require: 100rel header, and the response will include a RSeq header
/// ```
///
/// ## WebRTC SIP Client
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
///
/// // Scenario: WebRTC client registering with a SIP server
///
/// // Create a REGISTER request with WebRTC-specific features
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:sip-ws.example.com").unwrap()
///     .from("WebUser", "sip:webuser@example.com", Some("web-reg"))
///     .to("WebUser", "sip:webuser@example.com", None)
///     .contact("<sip:webuser@192.0.2.4:5060;transport=ws>;+sip.ice;reg-id=1", None)
///     // Include all WebRTC-related capabilities
///     .supported_webrtc()
///     .build();
///
/// // The SIP server now knows this is a WebRTC client with ICE, GRUU,
/// // and other capabilities needed for WebRTC interoperability
/// ```
pub trait SupportedBuilderExt {
    /// Add a Supported header with a single option tag
    ///
    /// This method adds a Supported header with a single SIP extension option tag.
    /// Option tags are standardized identifiers for SIP extensions defined in various RFCs.
    ///
    /// # Parameters
    ///
    /// * `option_tag` - The SIP extension option tag to include (e.g., "100rel", "timer")
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Supported header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Adding support for the Replaces header (for call transfers)
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .supported_tag("replaces")
    ///     .build();
    ///
    /// // This indicates the UA can handle Replaces headers in REFER requests
    /// // for attended transfer scenarios
    /// ```
    fn supported_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Supported header with multiple option tags
    ///
    /// This method adds a Supported header with multiple SIP extension option tags.
    /// This is the most common way to indicate support for several extensions at once.
    ///
    /// # Parameters
    ///
    /// * `option_tags` - A vector of SIP extension option tags to include
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Supported header containing all specified tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Create an enterprise PBX REGISTER with multiple capabilities
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("PBX", "sip:pbx@company.com", Some("pbx-reg"))
    ///     .to("PBX", "sip:pbx@company.com", None)
    ///     .contact("<sip:pbx@10.0.0.1:5060;transport=tls>", None)
    ///     .supported_tags(vec![
    ///         "path",         // Support for Path header
    ///         "outbound",     // Support for connection reuse
    ///         "timer",        // Support for session timers
    ///         "replaces",     // Support for call transfers
    ///         "gruu"          // Support for globally routable URIs
    ///     ])
    ///     .build();
    ///
    /// // The registrar now knows this is a full-featured PBX
    /// // capable of handling advanced SIP features
    /// ```
    fn supported_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Supported header for 100rel (reliable provisional responses)
    ///
    /// This convenience method adds support for the 100rel extension defined in RFC 3262.
    /// When included, it indicates that the UA can handle reliable provisional responses
    /// (1xx responses) with reliability mechanisms like RSeq/RAck.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with 100rel added to the Supported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::SupportedBuilderExt};
    ///
    /// // Scenario: A gateway responding to an INVITE and supporting reliable provisionals
    ///
    /// // Create a 180 Ringing response that can be sent reliably
    /// let ringing = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
    ///     .from("Gateway", "sip:gateway.example.com", None)
    ///     .to("Alice", "sip:alice@example.com", Some("xyz123"))
    ///     .supported_100rel()
    ///     .build();
    ///
    /// // If the original INVITE contained Supported: 100rel or Require: 100rel,
    /// // the UAC will know it can request reliability by adding Require: 100rel
    /// // to this provisional response
    /// ```
    fn supported_100rel(self) -> Self;
    
    /// Add a Supported header for path
    ///
    /// This convenience method adds support for the Path extension defined in RFC 3327.
    /// When included in a REGISTER request, it indicates that the UA understands and
    /// supports the Path header mechanism for traversing NATs and firewalls.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with path added to the Supported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Scenario: Mobile client registering through carrier network
    ///
    /// // Create a REGISTER request that supports Path headers
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("Mobile", "sip:user@example.com", Some("mo-reg"))
    ///     .to("Mobile", "sip:user@example.com", None)
    ///     .contact("<sip:user@10.0.0.1:5060>", None)
    ///     .supported_path()
    ///     .build();
    ///
    /// // The registrar and proxies know this client understands Path routing
    /// // and can properly handle requests routed via Path-specified proxies
    /// ```
    fn supported_path(self) -> Self;
    
    /// Add a Supported header for timer
    ///
    /// This convenience method adds support for the timer extension defined in RFC 4028.
    /// It indicates the UA supports session timers, which prevent hanging dialogs and
    /// ensure both parties know when sessions are still active.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with timer added to the Supported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Scenario: Making a call with session timer support
    ///
    /// // Create an INVITE with session timer support
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("call-1"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .contact("<sip:alice@192.0.2.1:5060>", None)
    ///     // Indicate support for session timers
    ///     .supported_timer()
    ///     // Session-Expires header would typically be added as well
    ///     // .header("Session-Expires", "1800;refresher=uac")
    ///     .build();
    ///
    /// // The remote party knows this client supports session timers
    /// // and can negotiate the refresh interval and refresher role
    /// ```
    fn supported_timer(self) -> Self;
    
    /// Add a Supported header with common WebRTC-related option tags
    ///
    /// This convenience method adds support for option tags commonly needed for
    /// WebRTC interoperability with SIP networks. This includes ICE, replaces,
    /// outbound, and GRUU capabilities.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with WebRTC-related tags added to the Supported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Scenario: WebRTC browser client making a call through a SIP gateway
    ///
    /// // Create an INVITE for a WebRTC-to-SIP call
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:+1-212-555-1234@sip-gateway.example.com")
    ///     .unwrap()
    ///     .from("WebUser", "sip:web-user@example.com", Some("web-call"))
    ///     .to("PSTN", "sip:+1-212-555-1234@sip-gateway.example.com", None)
    ///     .contact("<sip:web-user@192.0.2.4:5060;transport=ws>", None)
    ///     // Add WebRTC-specific capabilities
    ///     .supported_webrtc()
    ///     .build();
    ///
    /// // The SIP gateway can now understand this is a WebRTC client
    /// // and can handle media negotiation appropriately
    /// ```
    fn supported_webrtc(self) -> Self;
    
    /// Add a Supported header with standard option tags used by UAs
    ///
    /// This convenience method adds support for the most common standard SIP
    /// extensions used by full-featured User Agents: 100rel, path, timer, and replaces.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with standard option tags added to the Supported header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::SupportedBuilderExt};
    ///
    /// // Scenario: Full-featured SIP phone initiating a call
    ///
    /// // Create an INVITE with standard extensions
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:receptionist@example.com").unwrap()
    ///     .from("Executive", "sip:executive@example.com", Some("ex-call"))
    ///     .to("Receptionist", "sip:receptionist@example.com", None)
    ///     .contact("<sip:executive@192.0.2.10:5060;transport=tls>", None)
    ///     // Include standard SIP extensions
    ///     .supported_standard()
    ///     .build();
    ///
    /// // The callee knows this client supports reliable provisional responses,
    /// // session timers, call transfers, and other standard capabilities
    /// ```
    fn supported_standard(self) -> Self;
}

impl<T> SupportedBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn supported_tag(self, option_tag: impl Into<String>) -> Self {
        let supported = Supported::with_tag(option_tag);
        self.set_header(supported)
    }
    
    fn supported_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut tags = Vec::with_capacity(option_tags.len());
        for tag in option_tags {
            tags.push(tag.into());
        }
        let supported = Supported::new(tags);
        self.set_header(supported)
    }
    
    fn supported_100rel(self) -> Self {
        self.supported_tag("100rel")
    }
    
    fn supported_path(self) -> Self {
        self.supported_tag("path")
    }
    
    fn supported_timer(self) -> Self {
        self.supported_tag("timer")
    }
    
    fn supported_webrtc(self) -> Self {
        self.supported_tags(vec!["ice", "replaces", "outbound", "gruu"])
    }
    
    fn supported_standard(self) -> Self {
        self.supported_tags(vec!["100rel", "path", "timer", "replaces"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_supported_tag() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 1);
            assert!(supported.supports("100rel"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_supported_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .supported_tags(vec!["100rel", "path"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Supported(supported)) = response.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 2);
            assert!(supported.supports("100rel"));
            assert!(supported.supports("path"));
            assert!(!supported.supports("timer"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_supported_convenience_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_timer()
            .build();
            
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 1);
            assert!(supported.supports("timer"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_supported_webrtc() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_webrtc()
            .build();
            
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert!(supported.supports("ice"));
            assert!(supported.supports("replaces"));
            assert!(supported.supports("outbound"));
            assert!(supported.supports("gruu"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }
} 