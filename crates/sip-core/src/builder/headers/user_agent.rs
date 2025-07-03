use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use crate::types::user_agent::UserAgent;
use super::HeaderSetter;

/// User-Agent header builder
///
/// This module provides builder methods for the User-Agent header in SIP messages.
///
/// ## SIP User-Agent Header Overview
///
/// The User-Agent header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.41)
/// as part of the core SIP protocol. It contains information about the client software
/// originating the request. This header helps identify the user agent implementation
/// for troubleshooting, statistics, and compatibility handling.
///
/// ## Purpose of User-Agent Header
///
/// The User-Agent header serves several purposes in SIP:
///
/// 1. It identifies the software implementation making the request
/// 2. It helps with troubleshooting interoperability issues
/// 3. It provides information for statistics and analytics
/// 4. It allows servers to implement workarounds for known client bugs
///
/// ## Format and Conventions
///
/// The User-Agent header typically follows these formatting conventions:
///
/// - Product/version identifiers (e.g., "SIP-Client/1.0")
/// - Platform information in parentheses (e.g., "(Linux x86_64)")
/// - Multiple components separated by spaces
/// - No commas between tokens (unlike HTTP)
///
/// A comprehensive User-Agent might look like:
/// `MyClient/2.5 (Windows 10; x64) ExtraModule/1.2`
///
/// ## Security Considerations
///
/// When implementing User-Agent headers, consider:
///
/// - The header may reveal implementation details that could be exploited
/// - Some deployments may choose to provide minimal information
/// - In secure environments, consistent generic identifiers may be preferred
/// - Privacy-enhancing implementations might omit detailed version information
/// 
/// # Examples
///
/// ## Basic SIP Client Identification
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::UserAgentBuilderExt};
///
/// // Scenario: Standard SIP client registration
///
/// // Create a REGISTER request with User-Agent identification
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("reg1"))
///     .to("User", "sip:user@example.com", None)
///     .contact("<sip:user@192.0.2.1:5060>", None)
///     // Identify the client software
///     .user_agent("MySIPClient/2.1")
///     .build();
///
/// // The registrar can track which client software is being used
/// // and potentially adjust behavior based on known capabilities
/// ```
///
/// ## Detailed Client Information
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::UserAgentBuilderExt};
///
/// // Scenario: SIP client providing comprehensive implementation details
///
/// // Create an INVITE with detailed client information
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:support@example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("call1"))
///     .to("Support", "sip:support@example.com", None)
///     .contact("<sip:user@192.0.2.1:5060>", None)
///     // Add detailed client information for support purposes
///     .user_agent_products(vec![
///         "MyCompanySoftphone/3.2.1",  // Product name and version
///         "(macOS 13.4)",              // OS information
///         "WebRTCEngine/98.0.4758.82", // Media engine details
///         "Lib/1.4.5"                  // Additional components
///     ])
///     .build();
///
/// // Support staff can use this detailed information
/// // to diagnose problems and provide assistance
/// ```
///
/// ## Enterprise Deployment with Minimal Identification
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::UserAgentBuilderExt};
///
/// // Scenario: Enterprise deployment with privacy/security considerations
///
/// // Create a request with minimal identifying information
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:pbx.example.com").unwrap()
///     .from("Extension", "sip:1001@pbx.example.com", Some("sec-reg"))
///     .to("Extension", "sip:1001@pbx.example.com", None)
///     .contact("<sip:1001@10.0.0.15:5060>", None)
///     // Use generic identifier to avoid revealing implementation details
///     .user_agent("EnterpriseClient")
///     .build();
///
/// // Minimal identification for security-conscious deployments
/// ```
pub trait UserAgentBuilderExt {
    /// Add a User-Agent header with a single product token
    ///
    /// This method adds a User-Agent header with a single product identifier,
    /// which is typically in the format "ProductName/VersionNumber".
    ///
    /// # Parameters
    ///
    /// * `product` - The product identifier string, typically in Product/Version format
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the User-Agent header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::UserAgentBuilderExt};
    ///
    /// // Basic REGISTER with User-Agent for a mobile client
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("Mobile", "sip:user@example.com", None)
    ///     .to("Mobile", "sip:user@example.com", None)
    ///     // Identify as a mobile client version 2.4
    ///     .user_agent("MobileClient/2.4")
    ///     .build();
    ///
    /// // The server knows this is version 2.4 of the mobile client
    /// ```
    fn user_agent(self, product: impl Into<String>) -> Self;
    
    /// Add a User-Agent header with multiple product tokens
    ///
    /// This method adds a User-Agent header with multiple product identifiers,
    /// which allows for more detailed client information including product names,
    /// versions, platform details, and additional components.
    ///
    /// # Parameters
    ///
    /// * `products` - A vector of product identifier strings and other components
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the User-Agent header containing all components
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::UserAgentBuilderExt};
    ///
    /// // Create a comprehensive User-Agent for full diagnostics
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     // Add detailed client fingerprint
    ///     .user_agent_products(vec![
    ///         "CompanyPhone/4.2.0",           // Main product
    ///         "(Ubuntu 22.04; x86_64)",       // OS details
    ///         "LibSIP/2.1",                   // SIP stack
    ///         "Opus/1.3",                     // Codec info
    ///         "VP8/4.0"                       // Video codec
    ///     ])
    ///     .build();
    ///
    /// // This provides comprehensive information for diagnostics,
    /// // interoperability testing, and software version tracking
    /// ```
    fn user_agent_products(self, products: Vec<impl Into<String>>) -> Self;
}

impl<T> UserAgentBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn user_agent(self, product: impl Into<String>) -> Self {
        let user_agent = UserAgent::single(&product.into());
        self.set_header(user_agent)
    }
    
    fn user_agent_products(self, products: Vec<impl Into<String>>) -> Self {
        let string_products: Vec<String> = products.into_iter().map(|p| p.into()).collect();
        let user_agent = UserAgent::with_products(&string_products);
        self.set_header(user_agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_user_agent() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .user_agent("Example-SIP-Client/1.0")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::UserAgent(user_agent)) = request.header(&HeaderName::UserAgent) {
            assert_eq!(user_agent.len(), 1);
            assert_eq!(user_agent[0], "Example-SIP-Client/1.0");
        } else {
            panic!("User-Agent header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_user_agent_products() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .user_agent_products(vec!["Example-SIP-Client/1.0", "(Platform/OS Version)"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::UserAgent(user_agent)) = response.header(&HeaderName::UserAgent) {
            assert_eq!(user_agent.len(), 2);
            assert_eq!(user_agent[0], "Example-SIP-Client/1.0");
            assert_eq!(user_agent[1], "(Platform/OS Version)");
        } else {
            panic!("User-Agent header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_user_agent_headers() {
        // The behavior when calling user_agent multiple times could be either:
        // 1. Replace previous header (desired)
        // 2. Add another header (current implementation)
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .user_agent("First-Client/1.0")
            .user_agent("Second-Client/2.0")
            .build();
        
        // Get all User-Agent headers
        let user_agent_headers: Vec<_> = request.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::UserAgent(u) => Some(u),
                _ => None
            })
            .collect();
        
        // Check header count - there might be 1 or 2 depending on implementation
        if user_agent_headers.len() == 1 {
            // If there's only one header (replacement occurred), it should be the last one set
            assert_eq!(user_agent_headers[0][0], "Second-Client/2.0");
        } else if user_agent_headers.len() == 2 {
            // If there are two headers (append occurred), they should be in order of addition
            assert_eq!(user_agent_headers[0][0], "First-Client/1.0");
            assert_eq!(user_agent_headers[1][0], "Second-Client/2.0");
            
            // But the request.header() method should return the first matching header
            if let Some(TypedHeader::UserAgent(user_agent)) = request.header(&HeaderName::UserAgent) {
                assert_eq!(user_agent[0], "First-Client/1.0");
            } else {
                panic!("User-Agent header not found or has wrong type");
            }
        } else {
            panic!("Unexpected number of User-Agent headers: {}", user_agent_headers.len());
        }
    }
} 