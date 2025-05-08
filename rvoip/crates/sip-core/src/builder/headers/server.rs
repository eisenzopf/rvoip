use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use crate::types::server::ServerInfo;
use super::HeaderSetter;

/// Server header builder
///
/// This module provides builder methods for the Server header in SIP messages.
///
/// ## SIP Server Header Overview
///
/// The Server header is defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261#section-20.35)
/// as part of the core SIP protocol. It contains information about the software implementation
/// used by the UAS (User Agent Server) or server generating the response. This header helps
/// identify the server software for troubleshooting, statistics, and compatibility handling.
///
/// ## Purpose of Server Header
///
/// The Server header serves several important purposes in SIP:
///
/// 1. It identifies the server software responding to requests
/// 2. It helps with troubleshooting interoperability issues
/// 3. It provides information for network analytics and statistics
/// 4. It allows clients to implement workarounds for known server behaviors
///
/// ## Format and Conventions
///
/// The Server header typically follows these formatting conventions:
///
/// - Product/version identifiers (e.g., "SIP-Server/1.0")
/// - Platform information in parentheses (e.g., "(Linux x86_64)")
/// - Multiple components separated by spaces
/// - Order from general to specific (platform, server, modules)
///
/// A comprehensive Server header might look like:
/// `SIPProxy/5.2 (Debian; Linux x86_64) OpenSIPS/3.1`
///
/// ## Comparison with User-Agent Header
///
/// The Server header is analogous to the User-Agent header:
/// - **Server**: Used in responses to identify the responding server software
/// - **User-Agent**: Used in requests to identify the client software
///
/// ## Security Considerations
///
/// When implementing Server headers, consider:
///
/// - The header may reveal implementation details that could be exploited
/// - Version information might expose vulnerability windows
/// - Some deployments may choose to minimize information disclosure
/// - In secure environments, generic identifiers may be preferred over detailed information
/// 
/// # Examples
///
/// ## Basic SIP Proxy Identification
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ServerBuilderExt};
///
/// // Scenario: SIP proxy responding to an OPTIONS request
///
/// // Create a 200 OK response with Server identification
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("ProxyServer", "sip:proxy.example.com", None)
///     .to("Client", "sip:client@example.org", Some("abc123"))
///     // Identify the server software
///     .server("MyProxyServer/2.3")
///     .build();
///
/// // The client can identify which server implementation handled the request
/// // and potentially adjust behavior based on known server capabilities
/// ```
///
/// ## Detailed Server Information
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ServerBuilderExt};
///
/// // Scenario: SIP registrar with comprehensive server information
///
/// // Create a REGISTER 200 OK with detailed server information
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("User", "sip:user@example.com", Some("reg123"))
///     .to("User", "sip:user@example.com", None)
///     .contact("<sip:user@203.0.113.1:5060>", Some("7200"))
///     // Add detailed server information
///     .server_products(vec![
///         "ExampleSIPServer/4.2.1",    // Product name and version
///         "(Ubuntu 20.04 LTS)",        // OS information
///         "Kamailio/5.4.3",            // SIP server software
///         "TLS/1.3"                    // Security information
///     ])
///     .build();
///
/// // Administrators can use this detailed information for
/// // troubleshooting and interoperability analysis
/// ```
///
/// ## Enterprise SBC with Minimal Identification
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ServerBuilderExt};
///
/// // Scenario: Session Border Controller with security considerations
///
/// // Create a 403 Forbidden response with minimal server information
/// let response = SimpleResponseBuilder::new(StatusCode::Forbidden, Some("Authentication Failed"))
///     .from("SBC", "sip:edge.enterprise.example", None)
///     .to("External", "sip:external@example.org", Some("ext789"))
///     // Use generic identifier to avoid revealing implementation details
///     .server("EnterpriseSessionBorderController")
///     .build();
///
/// // Minimal identification that doesn't reveal version information
/// // which could be used by attackers to target known vulnerabilities
/// ```
pub trait ServerBuilderExt {
    /// Add a Server header with a single product token
    ///
    /// This method adds a Server header with a single product identifier,
    /// which is typically in the format "ProductName/VersionNumber".
    ///
    /// # Parameters
    ///
    /// * `product` - The product identifier string, typically in Product/Version format
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Server header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ServerBuilderExt};
    ///
    /// // Basic 200 OK response from a SIP registrar
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Registrar", "sip:registrar.example.com", None)
    ///     .to("User", "sip:user@example.org", Some("reg456"))
    ///     // Identify as version 3.1 of the registrar software
    ///     .server("SIPRegistrar/3.1")
    ///     .build();
    ///
    /// // The client knows this is version 3.1 of the SIP registrar
    /// ```
    fn server(self, product: impl Into<String>) -> Self;
    
    /// Add a Server header with multiple product tokens
    ///
    /// This method adds a Server header with multiple product identifiers,
    /// which allows for more detailed server information including product names,
    /// versions, platform details, and additional components.
    ///
    /// # Parameters
    ///
    /// * `products` - A vector of product identifier strings and other components
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Server header containing all components
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ServerBuilderExt};
    ///
    /// // Create a 486 Busy Here response with comprehensive server information
    /// let response = SimpleResponseBuilder::new(StatusCode::BusyHere, None)
    ///     .from("PBX", "sip:pbx.example.com", None)
    ///     .to("Caller", "sip:caller@example.org", Some("call123"))
    ///     // Add detailed server stack information
    ///     .server_products(vec![
    ///         "CompanyPBX/5.1.2",             // Main product
    ///         "(CentOS 7; x86_64)",           // OS details
    ///         "FreeSWITCH/1.10.7",            // SIP server platform
    ///         "mod_sofia/1.0.0",              // SIP module
    ///         "mod_conference/1.7.0"          // Conference module
    ///     ])
    ///     .build();
    ///
    /// // This provides comprehensive information for troubleshooting
    /// // and feature compatibility assessment
    /// ```
    fn server_products(self, products: Vec<impl Into<String>>) -> Self;
}

impl<T> ServerBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn server(self, product: impl Into<String>) -> Self {
        let server = ServerInfo::new()
            .with_product(&product.into(), None);
        self.set_header(server)
    }
    
    fn server_products(self, products: Vec<impl Into<String>>) -> Self {
        let mut server = ServerInfo::new();
        
        for product in products {
            let product_str = product.into();
            
            // Check if it's a comment (in parentheses)
            if product_str.starts_with('(') && product_str.ends_with(')') {
                let comment = &product_str[1..product_str.len()-1];
                server = server.with_comment(comment);
            } else if let Some(pos) = product_str.find('/') {
                // It's a product with version
                let (name, version) = product_str.split_at(pos);
                server = server.with_product(name, Some(&version[1..]));
            } else {
                // Just a product name without version
                server = server.with_product(&product_str, None);
            }
        }
        
        self.set_header(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_response_server() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server("Example-SIP-Server/1.0")
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
            assert_eq!(server.len(), 1);
            assert_eq!(server[0], "Example-SIP-Server/1.0");
        } else {
            panic!("Server header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_server_products() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server_products(vec!["Example-SIP-Server/1.0", "(Platform/OS Version)"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
            assert_eq!(server.len(), 2);
            assert_eq!(server[0], "Example-SIP-Server/1.0");
            assert_eq!(server[1], "(Platform/OS Version)");
        } else {
            panic!("Server header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_server_headers() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server("First-Server/1.0")
            .server("Second-Server/2.0")
            .build();
        
        // Get all Server headers
        let server_headers: Vec<_> = response.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::Server(s) => Some(s),
                _ => None
            })
            .collect();
        
        // Check header count - there might be 1 or 2 depending on implementation
        if server_headers.len() == 1 {
            // If there's only one header (replacement occurred), it should be the last one set
            assert_eq!(server_headers[0][0], "Second-Server/2.0");
        } else if server_headers.len() == 2 {
            // If there are two headers (append occurred), they should be in order of addition
            assert_eq!(server_headers[0][0], "First-Server/1.0");
            assert_eq!(server_headers[1][0], "Second-Server/2.0");
            
            // But the response.header() method should return the first matching header
            if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
                assert_eq!(server[0], "First-Server/1.0");
            } else {
                panic!("Server header not found or has wrong type");
            }
        } else {
            panic!("Unexpected number of Server headers: {}", server_headers.len());
        }
    }
} 