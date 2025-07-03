use std::str::FromStr;
use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
    path::Path,
    uri::Uri,
    Address
};
use super::HeaderSetter;
/// Path header builder
///
/// This module provides builder methods for the Path header in SIP messages.
/// 
/// ## SIP Path Header Overview
/// 
/// The Path header is defined in [RFC 3327](https://datatracker.ietf.org/doc/html/rfc3327) 
/// as an extension to the base SIP protocol. It solves the "SIP registration behind NAT" problem
/// by allowing registrations to traverse edge proxies while maintaining proper routing information.
/// 
/// ## Purpose of Path Header
/// 
/// The Path header serves as a "breadcrumb trail" for outbound proxies during registration:
/// 
/// 1. Edge proxies insert Path headers into REGISTER requests to indicate they must be traversed
///    for future requests toward the registered user agent
/// 2. The registrar collects these Path headers and associates them with the registered contact
/// 3. When someone sends a request to the registered user, the registrar adds the stored
///    Path URIs as Route headers, ensuring the request traverses the same edge proxies
/// 
/// ## Relationship with Service-Route and Record-Route
/// 
/// - **Path**: Used in REGISTER requests to indicate outbound proxies for future inbound requests
/// - **Service-Route**: Used by registrars in 200 OK responses to REGISTER to indicate service
///   proxies for future outbound requests from the client
/// - **Record-Route**: Used in dialog-forming requests to stay in path for future requests in that dialog
/// 
/// ## Common Use Cases
/// 
/// - **NAT traversal**: Edge proxies that maintain NAT bindings must stay in the signaling path
/// - **Topology hiding**: Core network elements remain hidden while edge proxies handle external communications
/// - **Network boundary traversal**: When SIP traffic must cross network boundaries through specific gateways
/// - **Load balancing**: Routing registrations through appropriate load balancing proxies
/// - **SIP trunking**: Provider edge proxies ensure proper routing between enterprises and service providers
/// 
/// # Examples
///
/// ## Complete Edge Proxy Registration Flow
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder, headers::PathBuilderExt};
/// use std::str::FromStr;
///
/// // Scenario: User behind a NAT registers through an edge proxy
///
/// // Step 1: User sends REGISTER (not shown)
///
/// // Step 2: Edge proxy receives REGISTER and adds Path header before forwarding
/// let original_register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     .build();
///
/// // Edge proxy adds Path header with its address (must have 'lr' parameter)
/// let edge_proxy_address = "sip:edge.example.com;lr;transport=tcp;maddr=203.0.113.1";
/// let forwarded_register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     // Edge proxy adds Path header with its routeable address
///     .path(edge_proxy_address).unwrap()
///     .build();
///
/// // Step 3: Registrar processes registration and stores Path with contact
/// // (Database storage not shown)
///
/// // Step 4: Registrar sends 200 OK with Path echoed back (acknowledgment)
/// let registration_ok = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     // Registrar echoes back Path header to acknowledge it was processed
///     .path(edge_proxy_address).unwrap()
///     .build();
///
/// // Step 5: Later, when a call arrives for Alice, registrar will add the stored
/// // Path URI as a Route header to ensure the request traverses the edge proxy
/// // (Not shown - would be Route header in INVITE)
/// ```
///
/// ## Multi-hop Registration Path
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PathBuilderExt};
/// use std::str::FromStr;
///
/// // Scenario: REGISTER traverses multiple edge proxies in an enterprise network
///
/// // Each proxy in the path adds its own Path header
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@office.example.com", Some("a6c85cf"))
///     .to("Alice", "sip:alice@office.example.com", None)
///     .contact("<sip:alice@10.0.0.123:5060;transport=tcp>", None)
///     // Department proxy (first proxy to receive REGISTER)
///     .path("sip:dept-proxy.office.example.com;lr").unwrap()
///     // Campus edge proxy (second proxy in path)
///     .path("sip:campus-edge.office.example.com;lr").unwrap()
///     // Enterprise session border controller (last proxy before registrar)
///     .path("sip:sbc.example.com;lr;x-session-id=a7bk2c").unwrap()
///     .build();
///
/// // The registrar will store all three Path entries in order
/// // Future requests to Alice will contain all three Routes in reverse order
/// ```
pub trait PathBuilderExt {
    /// Add a Path header with a single URI
    ///
    /// This method adds a Path header with a single URI to the SIP message. The Path header
    /// indicates a proxy that must be traversed by future requests to reach a registered user.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI to add as a Path entry, typically the address of an edge proxy.
    ///           It should contain the "lr" parameter for loose routing.
    ///
    /// # Returns
    ///
    /// * `Result<Self>` - The builder with the Path header added, or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PathBuilderExt};
    ///
    /// // Edge proxy adding its address to a REGISTER request
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     // Add Path with necessary parameters
    ///     .path("sip:edge.example.com;lr;transport=tcp;maddr=203.0.113.1").unwrap()
    ///     .build();
    /// ```
    fn path(self, uri: impl AsRef<str>) -> Result<Self> where Self: Sized;
    
    /// Add a Path header with multiple URIs
    ///
    /// This method adds a Path header with multiple URIs to the SIP message. This is used
    /// when a REGISTER request traverses multiple proxies that need to stay in the path.
    ///
    /// # Parameters
    ///
    /// * `uris` - A vector of URIs to add as Path entries. These should be in the order
    ///            they were added to the request, with the first proxy first.
    ///
    /// # Returns
    ///
    /// * `Result<Self>` - The builder with the Path header added, or an error if any URI is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PathBuilderExt};
    ///
    /// // Registrar creating a 200 OK to a REGISTER with multiple Path headers
    /// let register_ok = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .from("Bob", "sip:bob@branch.example.com", None)
    ///     .to("Bob", "sip:bob@branch.example.com", None)
    ///     // Echo back all Path headers from the REGISTER request
    ///     .path_addresses(vec![
    ///         "sip:branch-proxy.example.com;lr",
    ///         "sip:main-proxy.example.com;lr;transport=tcp",
    ///         "sip:edge.example.com;lr;maddr=203.0.113.7"
    ///     ]).unwrap()
    ///     .build();
    ///
    /// // When someone calls Bob, the INVITE will contain:
    /// // Route: <sip:edge.example.com;lr;maddr=203.0.113.7>
    /// // Route: <sip:main-proxy.example.com;lr;transport=tcp>
    /// // Route: <sip:branch-proxy.example.com;lr>
    /// ```
    fn path_addresses(self, uris: Vec<impl AsRef<str>>) -> Result<Self> where Self: Sized;
}

impl<T> PathBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn path(self, uri: impl AsRef<str>) -> Result<Self> {
        let uri = Uri::from_str(uri.as_ref())?;
        Ok(self.set_header(Path::with_uri(uri)))
    }
    
    fn path_addresses(self, uris: Vec<impl AsRef<str>>) -> Result<Self> {
        let mut path = Path::empty();
        
        for uri_str in uris {
            let uri = Uri::from_str(uri_str.as_ref())?;
            path.add_uri(uri);
        }
        
        Ok(self.set_header(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_path() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path("sip:proxy.example.com;lr").unwrap()
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Path(path)) = request.header(&HeaderName::Path) {
            assert_eq!(path.len(), 1);
            assert_eq!(path[0].0.uri.to_string(), "sip:proxy.example.com;lr");
        } else {
            panic!("Path header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_path_addresses() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path_addresses(vec!["sip:p1.example.com;lr", "sip:p2.example.com;lr"]).unwrap()
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Path(path)) = request.header(&HeaderName::Path) {
            assert_eq!(path.len(), 2);
            assert_eq!(path[0].0.uri.to_string(), "sip:p1.example.com;lr");
            assert_eq!(path[1].0.uri.to_string(), "sip:p2.example.com;lr");
        } else {
            panic!("Path header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_handling() {
        // Invalid URI
        let result = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path("invalid uri");
            
        assert!(result.is_err());
    }
} 