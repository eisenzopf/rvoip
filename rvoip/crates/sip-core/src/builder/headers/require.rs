use crate::error::{Error, Result};
use crate::types::{
    require::Require,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Require header builder
///
/// This module provides builder methods for the Require header in SIP messages.
///
/// ## SIP Require Header Overview
///
/// The Require header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.32)
/// as part of the core SIP protocol. It is used to indicate that particular SIP extensions are required
/// to process the request. If a server cannot support these extensions, it MUST reject the request
/// with a 420 (Bad Extension) response.
///
/// ## Purpose of Require Header
///
/// The Require header serves critical purposes in SIP:
///
/// 1. It mandates that specific extensions MUST be supported by the recipient
/// 2. It provides a mechanism for enforcing dependencies on protocol extensions
/// 3. It ensures consistent behavior between parties for critical features
/// 4. It allows features to be required rather than just advertised as supported
///
/// ## When to Use Require vs. Supported
///
/// - **Require**: Use when the request cannot function properly without the extension
/// - **Supported**: Use when the request can function without the extension but would benefit from it
///
/// ## Common Usage Scenarios
///
/// - **Reliable provisional responses**: Requiring 100rel for applications needing confirmed receipt of 1xx responses
/// - **Session timers**: Requiring timer extension for calls that must have session refresh
/// - **Path routing**: Requiring path for registrations that must use specific route paths
/// - **WebRTC signaling**: Requiring ice and related extensions for WebRTC interoperability
/// - **Call transfers**: Requiring replaces for attended transfer scenarios
///
/// ## Relationship with other headers
///
/// - **Require** vs **Supported**: Require mandates extensions, Supported merely indicates capabilities
/// - **Require** and **Unsupported**: When a server can't support a required extension, it responds with 420 and lists the extensions in Unsupported
/// - **Require** vs **Proxy-Require**: Require targets the recipient UAS, Proxy-Require targets proxies in the path
///
/// # Examples
///
/// ## INVITE with Reliable Provisional Responses
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
///
/// // Scenario: Contact center making a call that needs reliable ring indication
///
/// // Create an INVITE requiring reliable provisional responses
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:customer@example.com").unwrap()
///     .from("Agent", "sip:agent@contact-center.example.com", Some("a84b4c"))
///     .to("Customer", "sip:customer@example.com", None)
///     .contact("<sip:agent@192.0.2.5:5060;transport=tls>", None)
///     // Mandate reliable provisional responses (required for the application)
///     .require_100rel()
///     .build();
///
/// // If remote side doesn't support 100rel, it will respond with:
/// // SIP/2.0 420 Bad Extension
/// // Unsupported: 100rel
/// //
/// // Otherwise, it will include Require: 100rel in 18x responses
/// // with the RSeq/CSeq/RAck mechanism
/// ```
///
/// ## REGISTER with Path and outbound support
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
///
/// // Scenario: Mobile client that must have Path routing for proper operation
///
/// // Create a REGISTER requiring path support
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("reg1"))
///     .to("User", "sip:user@example.com", None)
///     .contact("<sip:user@192.0.2.1:5060;ob>", None)
///     // Mandate path support - necessary for NAT traversal in this case
///     .require_path()
///     .build();
///
/// // If the registrar doesn't support path, it must reject with 420
/// // The client would need to find another way to register or use a different server
/// ```
///
/// ## Handling 420 Bad Extension responses
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
///
/// // Original INVITE with multiple required extensions
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("xyz123"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@192.0.2.3:5060>", None)
///     // Require multiple extensions
///     .require_tags(vec!["100rel", "timer", "resource-priority"])
///     .build();
///
/// // When a 420 Bad Extension response is received, client can retry with fewer extensions
/// // The 420 would contain an Unsupported header listing the problematic extensions
/// // For example: Unsupported: resource-priority
///
/// // Client can then retry with just the extensions that are supported:
/// let retry_invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("xyz123"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@192.0.2.3:5060>", None)
///     // Only require extensions that are supported
///     .require_tags(vec!["100rel", "timer"])
///     .build();
/// ```
pub trait RequireBuilderExt {
    /// Add a Require header with a single option tag
    ///
    /// This method adds a Require header with a single SIP extension option tag
    /// that must be supported by the recipient for proper processing. If the receiver
    /// does not support the extension, it must reject the request with a 420 response.
    ///
    /// # Parameters
    ///
    /// * `option_tag` - The SIP extension option tag to require
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Create an INVITE requiring 'norefersub' extension (no implicit subscriptions in REFER)
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:receptionist@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Receptionist", "sip:receptionist@example.com", None)
    ///     // Require specific behavior for REFER handling
    ///     .require_tag("norefersub")
    ///     .build();
    ///
    /// // The recipient must understand and implement norefersub
    /// // behavior or reject the request with 420 Bad Extension
    /// ```
    fn require_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Require header with multiple option tags
    ///
    /// This method adds a Require header with multiple SIP extension option tags
    /// that must all be supported by the recipient. If the receiver does not support
    /// any of the extensions, it must reject the request with a 420 response listing
    /// all unsupported extensions.
    ///
    /// # Parameters
    ///
    /// * `option_tags` - A vector of SIP extension option tags to require
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header containing all specified tags
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Advanced call center making a call with several requirements
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:customer@example.com").unwrap()
    ///     .from("Agent", "sip:agent@contact-center.example.com", Some("cc123"))
    ///     .to("Customer", "sip:customer@example.com", None)
    ///     .contact("<sip:agent@10.0.1.2:5060>", None)
    ///     // Require multiple features essential for this deployment
    ///     .require_tags(vec![
    ///         "100rel",       // Reliable provisional responses
    ///         "timer",        // Session timer support
    ///         "replaces",     // Ability to replace dialogs (for transfers)
    ///         "gruu"          // Globally routable URI for mid-dialog targeting
    ///     ])
    ///     .build();
    ///
    /// // If the recipient does not support ALL required extensions,
    /// // it must reject with 420 and list the unsupported ones
    /// ```
    fn require_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Require header for 100rel (reliable provisional responses)
    ///
    /// This convenience method adds a Require header with the 100rel extension
    /// defined in RFC 3262. This requires the recipient to support reliable
    /// provisional responses, including the RSeq/RAck mechanism for 1xx responses.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header for 100rel
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Scenario: Video conferencing system initiating a call
    ///
    /// // Create an INVITE that requires reliable provisional responses
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:room@conference.example.org").unwrap()
    ///     .from("Participant", "sip:alice@example.com", Some("conf1"))
    ///     .to("Conference", "sip:room@conference.example.org", None)
    ///     .contact("<sip:alice@203.0.113.1:5060>", None)
    ///     // Require reliable provisional responses
    ///     .require_100rel()
    ///     .build();
    ///
    /// // The conference server must respond with 100rel in provisional responses
    /// // to confirm media is being negotiated reliably
    /// ```
    fn require_100rel(self) -> Self;
    
    /// Add a Require header for timer (session timers)
    ///
    /// This convenience method adds a Require header with the timer extension
    /// defined in RFC 4028. This requires the recipient to support session timers,
    /// allowing for periodic refresh of the session and detection of communication
    /// failures.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header for timer
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Scenario: Emergency services call that requires session monitoring
    ///
    /// // Create an INVITE for an emergency call with required session timers
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:911@emergency.example.gov").unwrap()
    ///     .from("User", "sip:user@example.net", Some("emerg"))
    ///     .to("Emergency", "sip:911@emergency.example.gov", None)
    ///     .contact("<sip:user@192.0.2.57:5060>", None)
    ///     // Add Session-Expires header for session refresh requirements
    ///     .header(TypedHeader::Other(
    ///         HeaderName::Other("Session-Expires".to_string()),
    ///         HeaderValue::Raw("600;refresher=uac".as_bytes().to_vec())
    ///     ))
    ///     // Require timer extension
    ///     .require_timer()
    ///     .build();
    ///
    /// // The emergency service must support session timers
    /// // to properly monitor the call state
    /// ```
    fn require_timer(self) -> Self;
    
    /// Add a Require header for path
    ///
    /// This convenience method adds a Require header with the path extension
    /// defined in RFC 3327. This requires the registrar to support the Path header
    /// mechanism for routing through intermediate proxies during registration.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header for path
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Scenario: Mobile client behind multiple NAT layers
    ///
    /// // Create a REGISTER requiring path support
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.org").unwrap()
    ///     .from("Mobile", "sip:user@example.org", Some("reg99"))
    ///     .to("Mobile", "sip:user@example.org", None)
    ///     .contact("<sip:user@10.20.30.40:5060;transport=udp>;reg-id=1;+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\"", None)
    ///     // Add path in Require header
    ///     .require_path()
    ///     .build();
    ///
    /// // The registrar must support Path header processing
    /// // for this registration to succeed
    /// ```
    fn require_path(self) -> Self;
    
    /// Add a Require header for ICE negotiation
    ///
    /// This convenience method adds a Require header with the ice extension.
    /// This requires the recipient to support Interactive Connectivity Establishment
    /// for NAT traversal in media negotiation, commonly used in WebRTC scenarios.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Require header for ice
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Scenario: WebRTC client making a call to a SIP endpoint
    ///
    /// // Create an INVITE requiring ICE support
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:sip-endpoint@example.com").unwrap()
    ///     .from("WebRTC", "sip:web-user@example.org", Some("web1"))
    ///     .to("SIP", "sip:sip-endpoint@example.com", None)
    ///     .contact("<sip:web-user@203.0.113.100:5060;transport=ws>", None)
    ///     // Require ICE for NAT traversal in the media session
    ///     .require_ice()
    ///     .build();
    ///
    /// // The recipient must support ICE negotiation
    /// // or reject the call with 420 Bad Extension
    /// ```
    fn require_ice(self) -> Self;
    
    /// Add a Require header with common WebRTC-related option tags
    ///
    /// This convenience method adds a Require header with the common WebRTC-related
    /// extensions: ice (for Interactive Connectivity Establishment) and replaces
    /// (for call transfer scenarios). These are often required for full WebRTC
    /// interoperability with SIP networks.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with WebRTC-related tags added to the Require header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RequireBuilderExt};
    ///
    /// // Scenario: Browser-based contact center requiring WebRTC support
    ///
    /// // Create an INVITE requiring WebRTC support
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:agent@contact-center.example.com").unwrap()
    ///     .from("Customer", "sip:web-customer@webrtc-gateway.example.org", Some("web-call"))
    ///     .to("Agent", "sip:agent@contact-center.example.com", None)
    ///     .contact("<sip:web-customer@203.0.113.45:5060;transport=ws>", None)
    ///     // Add all WebRTC-required extensions
    ///     .require_webrtc()
    ///     .build();
    ///
    /// // The contact center must support WebRTC-related extensions
    /// // or the call will fail with 420 Bad Extension
    /// ```
    fn require_webrtc(self) -> Self;
}

impl<T> RequireBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn require_tag(self, option_tag: impl Into<String>) -> Self {
        let require = Require::with_tag(option_tag);
        self.set_header(require)
    }
    
    fn require_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut tags = Vec::with_capacity(option_tags.len());
        for tag in option_tags {
            tags.push(tag.into());
        }
        let require = Require::new(tags);
        self.set_header(require)
    }
    
    fn require_100rel(self) -> Self {
        self.require_tag("100rel")
    }
    
    fn require_timer(self) -> Self {
        self.require_tag("timer")
    }
    
    fn require_path(self) -> Self {
        self.require_tag("path")
    }
    
    fn require_ice(self) -> Self {
        self.require_tag("ice")
    }
    
    fn require_webrtc(self) -> Self {
        self.require_tags(vec!["ice", "replaces"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_require_tag() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 1);
            assert!(require.requires("100rel"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_require_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .require_tags(vec!["100rel", "timer"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Require(require)) = response.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 2);
            assert!(require.requires("100rel"));
            assert!(require.requires("timer"));
            assert!(!require.requires("path"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_require_convenience_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_path()
            .build();
            
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 1);
            assert!(require.requires("path"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_require_webrtc() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_webrtc()
            .build();
            
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert!(require.requires("ice"));
            assert!(require.requires("replaces"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }
} 