//! Utility functions and traits for dialog management
//!
//! This module provides helper utilities used throughout the dialog manager
//! implementation, including message extensions and address extraction.

use std::net::SocketAddr;
use rvoip_sip_core::{Request, Response};

/// Helper trait for Request message extensions
/// 
/// Provides convenient methods for extracting common information from SIP requests.
pub trait MessageExtensions {
    /// Extract body as string if present
    fn body_string(&self) -> Option<String>;
}

impl MessageExtensions for Request {
    fn body_string(&self) -> Option<String> {
        if self.body().is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(self.body()).to_string())
        }
    }
}

impl MessageExtensions for Response {
    fn body_string(&self) -> Option<String> {
        if self.body().is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(self.body()).to_string())
        }
    }
}

/// Utility for extracting source addresses from SIP messages
/// 
/// This provides fallback mechanisms when transport-layer source information
/// is not available, extracting address information from SIP headers.
pub struct SourceExtractor;

impl SourceExtractor {
    /// Extract source address from SIP request headers
    /// 
    /// This method attempts to extract the source address from SIP request headers.
    /// In a real implementation, this would typically come from the transport layer,
    /// but this provides a fallback for when that information isn't available.
    /// 
    /// The extraction follows this priority:
    /// 1. Via header host/port (most reliable for source)
    /// 2. Contact header URI
    /// 3. Default fallback address
    /// 
    /// # Arguments
    /// * `request` - The SIP request to extract source from
    /// 
    /// # Returns
    /// A SocketAddr representing the likely source of the request
    pub fn extract_from_request(request: &Request) -> SocketAddr {
        // Try to extract from Via header (most reliable for source)
        if let Some(via_header) = request.typed_header::<rvoip_sip_core::types::via::Via>() {
            if let Some(via) = via_header.0.first() {
                let host_str = via.host().to_string();
                if let Ok(host) = host_str.parse::<std::net::IpAddr>() {
                    let port = via.port().unwrap_or(5060);
                    return SocketAddr::new(host, port);
                }
                
                // Try to resolve hostname if it's not an IP
                if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&format!("{}:{}", host_str, via.port().unwrap_or(5060))) {
                    if let Some(addr) = addrs.into_iter().next() {
                        return addr;
                    }
                }
            }
        }
        
        // Try Contact header as fallback
        if let Some(contact_header) = request.typed_header::<rvoip_sip_core::types::contact::Contact>() {
            if let Some(contact) = contact_header.0.first() {
                // Extract host from contact URI
                if let rvoip_sip_core::types::contact::ContactValue::Params(params) = contact {
                    if let Some(param) = params.first() {
                        let uri_str = param.address.uri.to_string();
                        if let Ok(_uri) = uri_str.parse::<rvoip_sip_core::Uri>() {
                            // Extract host and port from URI string parsing
                            // Since URI methods don't exist, we'll parse the string
                            if let Ok(parsed_uri) = uri_str.parse::<http::Uri>() {
                                if let Some(host) = parsed_uri.host() {
                                    if let Ok(host_ip) = host.parse::<std::net::IpAddr>() {
                                        let port = parsed_uri.port_u16().unwrap_or(5060);
                                        return SocketAddr::new(host_ip, port);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Fallback to default
        std::net::SocketAddr::from(([127, 0, 0, 1], 5060))
    }
    
    /// Extract source address from Response headers
    /// 
    /// Similar to request extraction but adapted for response messages.
    /// 
    /// # Arguments
    /// * `response` - The SIP response to extract source from
    /// 
    /// # Returns
    /// A SocketAddr representing the likely source of the response
    pub fn extract_from_response(response: &Response) -> SocketAddr {
        // For responses, we primarily look at Via headers
        if let Some(via_header) = response.typed_header::<rvoip_sip_core::types::via::Via>() {
            if let Some(via) = via_header.0.first() {
                let host_str = via.host().to_string();
                if let Ok(host) = host_str.parse::<std::net::IpAddr>() {
                    let port = via.port().unwrap_or(5060);
                    return SocketAddr::new(host, port);
                }
            }
        }
        
        // Fallback to default
        std::net::SocketAddr::from(([127, 0, 0, 1], 5060))
    }
}

/// Utility functions for dialog identification and matching
pub struct DialogUtils;

impl DialogUtils {
    /// Generate a dialog lookup key from Call-ID and tags
    /// 
    /// Creates a unique key for dialog lookup based on RFC 3261 dialog identification.
    /// The key format is "call-id:local-tag:remote-tag".
    /// 
    /// # Arguments
    /// * `call_id` - The Call-ID header value
    /// * `local_tag` - The local tag
    /// * `remote_tag` - The remote tag
    /// 
    /// # Returns
    /// A string key for dialog lookup
    pub fn create_lookup_key(call_id: &str, local_tag: &str, remote_tag: &str) -> String {
        format!("{}:{}:{}", call_id, local_tag, remote_tag)
    }
    
    /// Generate multiple lookup keys for bidirectional dialog matching
    /// 
    /// Creates both UAC and UAS perspective keys for robust dialog matching.
    /// 
    /// # Arguments
    /// * `call_id` - The Call-ID header value
    /// * `tag1` - First tag (could be local or remote)
    /// * `tag2` - Second tag (could be remote or local)
    /// 
    /// # Returns
    /// A tuple of (key1, key2) for bidirectional lookup
    pub fn create_bidirectional_keys(call_id: &str, tag1: &str, tag2: &str) -> (String, String) {
        (
            Self::create_lookup_key(call_id, tag1, tag2),
            Self::create_lookup_key(call_id, tag2, tag1),
        )
    }
    
    /// Extract dialog identification tuple from a SIP request
    /// 
    /// Extracts the essential dialog identification information according to RFC 3261.
    /// 
    /// # Arguments
    /// * `request` - The SIP request to extract from
    /// 
    /// # Returns
    /// Optional tuple of (call_id, from_tag, to_tag)
    pub fn extract_dialog_info(request: &Request) -> Option<(String, Option<String>, Option<String>)> {
        let call_id = request.call_id()?.to_string();
        let from_tag = request.from().and_then(|f| f.tag()).map(|t| t.to_string());
        let to_tag = request.to().and_then(|t| t.tag()).map(|t| t.to_string());
        
        Some((call_id, from_tag, to_tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::{Method, Uri};
    
    #[test]
    fn test_dialog_lookup_key_creation() {
        let key = DialogUtils::create_lookup_key("call-123", "tag-local", "tag-remote");
        assert_eq!(key, "call-123:tag-local:tag-remote");
    }
    
    #[test]
    fn test_bidirectional_keys() {
        let (key1, key2) = DialogUtils::create_bidirectional_keys("call-123", "tag-a", "tag-b");
        assert_eq!(key1, "call-123:tag-a:tag-b");
        assert_eq!(key2, "call-123:tag-b:tag-a");
    }
    
    #[test]
    fn test_source_extraction_fallback() {
        // Test fallback behavior when no headers are present
        let uri = Uri::sip("test@example.com");
        let request = Request::new(Method::Invite, uri);
        
        let source = SourceExtractor::extract_from_request(&request);
        assert_eq!(source, SocketAddr::from(([127, 0, 0, 1], 5060)));
    }
    
    #[test]
    fn test_message_extensions_request() {
        let uri = Uri::sip("test@example.com");
        let request_empty = Request::new(Method::Invite, uri.clone());
        let request_with_body = Request::new(Method::Invite, uri).with_body(b"test content".to_vec());
        
        // Test empty body
        assert_eq!(request_empty.body_string(), None);
        
        // Test body with content
        assert_eq!(request_with_body.body_string(), Some("test content".to_string()));
    }
    
    #[test]
    fn test_dialog_info_extraction_empty() {
        let uri = Uri::sip("test@example.com");
        let request = Request::new(Method::Invite, uri);
        
        // Test extraction when no Call-ID header is present
        let result = DialogUtils::extract_dialog_info(&request);
        assert!(result.is_none()); // Should be None due to missing Call-ID
    }
} 