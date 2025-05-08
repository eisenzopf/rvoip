use crate::types::{
    via::{Via, ViaHeader, SentProtocol},
    TypedHeader,
    Param,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding Via headers to SIP message builders.
///
/// This trait provides a standard way to add Via headers to both request and response builders
/// as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
///
/// ## Purpose and Importance
///
/// The Via header serves several critical functions in SIP:
///
/// - **Response Routing**: Enables responses to follow the same path as requests in reverse
/// - **Loop Detection**: Prevents messages from being processed more than once
/// - **Branch Identification**: Uniquely identifies transactions through the branch parameter
/// - **Protocol Information**: Specifies the transport protocol used at each hop (UDP, TCP, TLS, etc.)
/// - **NAT Traversal**: Assists with routing through NATs using the received and rport parameters
///
/// Each SIP element (UAC, UAS, proxy) involved in processing a request adds its own Via header
/// to the top of the list. When generating a response, all Via headers must be preserved in
/// the same order, and the response is routed by removing the topmost Via at each hop.
///
/// ## Branch Parameter
///
/// The branch parameter must be globally unique for each transaction and must start with the
/// magic cookie "z9hG4bK" for requests conforming to RFC 3261. This ensures transaction identification
/// across distributed systems.
///
/// ## Via Structure
///
/// A typical Via header has the format:
/// ```text
/// Via: SIP/2.0/UDP proxy.example.com:5060;branch=z9hG4bK776asdhds;received=192.0.2.1;rport=5060
/// ```
///
/// Which includes:
/// - Protocol version (SIP/2.0)
/// - Transport protocol (UDP)
/// - Host (proxy.example.com)
/// - Port (5060)
/// - Branch parameter (z9hG4bK776asdhds)
/// - Optional parameters (received, rport, etc.)
///
/// # Examples
///
/// ## Basic Via Header
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
///     .build();
/// ```
///
/// ## Multiple Via Headers (Request Through Proxies)
///
/// When a SIP message traverses multiple proxies, each one adds a Via header. 
/// The topmost (first) Via header represents the most recent hop.
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Simulate a request that went through two proxies
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     // First Via (most recent, added by last proxy)
///     .via("proxy2.example.com:5060", "UDP", Some("z9hG4bK33792"))
///     // Second Via (added by first proxy)
///     .via("proxy1.example.com:5060", "UDP", Some("z9hG4bK123e5"))
///     // Third Via (original client)
///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
///     .build();
/// ```
///
/// ## Complete SIP Dialog with Via Headers
///
/// This example shows how Via headers are used throughout a complete SIP dialog,
/// including how they're preserved in responses:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{ViaBuilderExt, FromBuilderExt, ToBuilderExt, CallIdBuilderExt};
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // 1. Initial INVITE from UAC through a proxy to UAS
/// let invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq(1)
///     // Proxy adds its Via first (top)
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     // UAC's original Via comes next
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 2. Response from UAS preserves both Via headers in same order
/// // The response will naturally be routed first to the proxy, then to the UAC
/// let response = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", Some("b84b23"))  // Response adds a To tag
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq_with_method(1, Method::Invite)
///     // Same Via headers in same order as received in the request
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
/// ```
///
/// ## Via Headers with NAT Traversal Support
///
/// This example demonstrates how received and rport parameters help with NAT traversal:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // 1. Client sends request indicating support for rport
/// let initial_request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
///
/// // Client's Via with empty rport parameter (requesting NAT handling)
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("rport", None::<String>)); // Empty rport parameter
/// 
/// let client_via = Via::new("SIP", "2.0", "UDP", "client.example.com", Some(5060), params).unwrap();
/// let request = initial_request.header(TypedHeader::Via(client_via)).build();
///
/// // 2. When server/proxy receives this request, it would see the client's IP and port
/// // and add them to the Via header before forwarding or responding
///
/// // 3. Server/proxy would generate a response like this:
/// let response = SimpleResponseBuilder::ok();
///
/// // Server adds received and rport parameters to help NAT traversal
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("rport", Some("12345"))); // The actual source port
/// params.push(Param::new("received", Some("203.0.113.1"))); // The actual source IP
/// 
/// let server_via = Via::new("SIP", "2.0", "UDP", "client.example.com", Some(5060), params).unwrap();
/// let response_with_via = response.header(TypedHeader::Via(server_via));
///
/// // Now the response will be routed to 203.0.113.1:12345 instead of client.example.com:5060
/// ```
///
/// ## Via Headers with Additional Parameters
///
/// Via headers can contain additional parameters like 'ttl' for multicast, 'maddr', etc.:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // Create a request with Via header containing special parameters
/// let mut request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
///
/// // Create Via header with multiple parameters
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("ttl", Some("16")));          // Time-to-live for multicast
/// params.push(Param::new("maddr", Some("224.2.0.1"))); // Multicast address
/// params.push(Param::new("hidden", None::<String>));   // Flag parameter (no value)
/// 
/// let via = Via::new("SIP", "2.0", "UDP", "proxy.example.com", Some(5060), params).unwrap();
/// let final_request = request.header(TypedHeader::Via(via)).build();
/// ```
///
/// ## Stateful Proxy Via Handling
///
/// This example shows how a stateful proxy handles Via headers:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // 1. Incoming request to the proxy with the UAC's Via header
/// let incoming_request = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 2. Proxy adds its own Via header and forwards request
/// let outgoing_request = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     // Proxy adds its Via first (will be top/first Via header)
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     // Original UAC's Via follows
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 3. Proxy receives response, processes top Via header (its own),
/// // then forwards response with that Via removed
/// // (In a real implementation, the proxy would extract Via headers from the response)
/// let incoming_response = SimpleResponseBuilder::ok()
///     // Response with both Via headers 
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 4. Proxy forwards response with its Via header removed
/// let outgoing_response = SimpleResponseBuilder::ok()
///     // Only the original UAC's Via remains
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
/// ```
pub trait ViaBuilderExt {
    /// Add a Via header with optional branch parameter
    ///
    /// Creates and adds a Via header as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
    /// The Via header indicates the path taken by the request so far and helps route responses back.
    ///
    /// # Parameters
    /// - `host`: The host or IP address (e.g., "192.168.1.1" or "example.com:5060")
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be prefixed with z9hG4bK per RFC 3261)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ## Basic Via Header
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ViaBuilderExt;
    ///
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .build();
    /// ```
    ///
    /// ## For a Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ViaBuilderExt;
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
    ///
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
    ///     .cseq_with_method(1, Method::Invite)
    ///     .build();
    /// ```
    ///
    /// # Note
    ///
    /// For more complex Via headers with additional parameters (received, rport, ttl, etc.),
    /// you should use the TypedHeader approach with Via constructor directly, as shown in
    /// the trait documentation examples.
    fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self;
}

impl ViaBuilderExt for SimpleRequestBuilder {
    fn via(mut self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        let mut params = Vec::new();
        
        // Add branch parameter if provided
        if let Some(branch_value) = branch {
            params.push(Param::branch(branch_value));
        }
        
        // Parse host to separate hostname and port
        let (hostname, port) = if host.contains(':') {
            let parts: Vec<&str> = host.split(':').collect();
            if parts.len() == 2 {
                if let Ok(port_num) = parts[1].parse::<u16>() {
                    (parts[0].to_string(), Some(port_num))
                } else {
                    (host.to_string(), None)
                }
            } else {
                (host.to_string(), None)
            }
        } else {
            (host.to_string(), None)
        };
        
        // Create Via header
        if let Ok(via) = Via::new("SIP", "2.0", transport, &hostname, port, params) {
            self.header(TypedHeader::Via(via))
        } else {
            self
        }
    }
}

impl ViaBuilderExt for SimpleResponseBuilder {
    fn via(mut self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        let mut params = Vec::new();
        
        // Add branch parameter if provided
        if let Some(branch_value) = branch {
            params.push(Param::branch(branch_value));
        }
        
        // Parse host to separate hostname and port
        let (hostname, port) = if host.contains(':') {
            let parts: Vec<&str> = host.split(':').collect();
            if parts.len() == 2 {
                if let Ok(port_num) = parts[1].parse::<u16>() {
                    (parts[0].to_string(), Some(port_num))
                } else {
                    (host.to_string(), None)
                }
            } else {
                (host.to_string(), None)
            }
        } else {
            (host.to_string(), None)
        };
        
        // Create Via header
        if let Ok(via) = Via::new("SIP", "2.0", transport, &hostname, port, params) {
            self.header(TypedHeader::Via(via))
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    
    #[test]
    fn test_request_via_header() {
        // Test with hostname and port
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .via("example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
            .build();
            
        let via_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Via(v) = h { Some(v) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), Some(5060));
        assert_eq!(header.transport(), "UDP");
        assert_eq!(header.branch(), Some("z9hG4bK776asdhds"));
        
        // Test with hostname only
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .via("example.com", "TCP", None)
            .build();
            
        let via_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Via(v) = h { Some(v) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), None);
        assert_eq!(header.transport(), "TCP");
        assert_eq!(header.branch(), None);
        
        // Test with invalid port format (should use host as is)
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .via("example.com:invalid", "UDP", Some("z9hG4bK776asdhds"))
            .build();
            
        let via_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Via(v) = h { Some(v) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com:invalid");
        assert_eq!(header.port(), None);
    }
    
    #[test]
    fn test_response_via_header() {
        // Test with hostname and port
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .via("example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
            .build();
            
        let via_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Via(v) = h { Some(v) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), Some(5060));
        assert_eq!(header.transport(), "UDP");
        assert_eq!(header.branch(), Some("z9hG4bK776asdhds"));
        
        // Test with IP address
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .via("192.168.1.1", "TCP", None)
            .build();
            
        let via_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Via(v) = h { Some(v) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "192.168.1.1");
        assert_eq!(header.port(), None);
        assert_eq!(header.transport(), "TCP");
        assert_eq!(header.branch(), None);
    }
} 