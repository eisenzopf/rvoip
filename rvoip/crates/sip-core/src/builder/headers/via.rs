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
/// The Via header indicates the transport used for the transaction and identifies the location where the response should be sent.
///
/// # Examples
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