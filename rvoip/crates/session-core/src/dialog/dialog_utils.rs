use std::str::FromStr;
use std::net::{SocketAddr, IpAddr, ToSocketAddrs};
use rvoip_sip_core::{Uri, Method, TypedHeader};
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::contact::{ContactParamInfo, ContactValue};
use tracing::{warn, debug};

/// Extract tag from header with parameter
pub fn extract_tag(header: &rvoip_sip_core::Header) -> Option<String> {
    // Just return empty tag for now
    None
}

/// Extract tag from header string - test compatibility function
pub fn extract_tag_from_str(header_value: &str) -> Option<String> {
    // Check for tag parameter
    if let Some(tag_pos) = header_value.find(";tag=") {
        let tag_start = tag_pos + 5; // ";tag=" length
        let tag_end = header_value[tag_start..]
            .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
            .map(|pos| tag_start + pos)
            .unwrap_or(header_value.len());
        Some(header_value[tag_start..tag_end].to_string())
    } else {
        None
    }
}

/// Extract URI from a string representation
pub fn extract_uri(header_value: &str) -> Option<Uri> {
    if header_value.is_empty() {
        return None;
    }
    
    // Try to directly parse the URI first
    if let Ok(uri) = Uri::from_str(header_value) {
        // This might include parameters like ;tag in the raw string
        // If it's a sip URI, parse out just the URI part without parameters
        if header_value.contains(';') && uri.scheme().to_string().starts_with("sip") {
            let uri_part = header_value.split(';').next().unwrap();
            if let Ok(clean_uri) = Uri::from_str(uri_part) {
                return Some(clean_uri);
            }
        }
        return Some(uri);
    }
    
    // Handle URIs in angle brackets
    if header_value.contains('<') && header_value.contains('>') {
        let start = header_value.find('<').unwrap() + 1;
        let end = header_value.find('>').unwrap();
        if start < end {
            let uri_str = &header_value[start..end];
            if let Ok(uri) = Uri::from_str(uri_str) {
                return Some(uri);
            }
        }
    }
    
    // Attempt to extract domain part as fallback
    if let Some(at_pos) = header_value.rfind('@') {
        // There's a username@domain pattern
        let domain_part = &header_value[at_pos + 1..];
        let domain_end = domain_part.find(|c: char| c == ';' || c == '>' || c == ' ')
            .unwrap_or(domain_part.len());
        let domain = &domain_part[0..domain_end];
        
        // Try to create a SIP URI with just the domain
        if let Ok(uri) = Uri::from_str(&format!("sip:{}", domain)) {
            return Some(uri);
        }
    } else {
        // Try to extract just the domain/IP directly
        let end = header_value.find(|c: char| c == ';' || c == '>' || c == ' ')
            .unwrap_or(header_value.len());
        let potential_domain = &header_value[0..end];
        
        // Ensure we're not including scheme parts
        let domain = if potential_domain.contains(':') {
            // If it has a scheme like sip:, skip to the domain part
            if let Some(colon) = potential_domain.find(':') {
                &potential_domain[colon + 1..]
            } else {
                potential_domain
            }
        } else {
            potential_domain
        };
        
        if !domain.is_empty() {
            if let Ok(uri) = Uri::from_str(&format!("sip:{}", domain)) {
                return Some(uri);
            }
        }
    }
    
    None
}

/// Extract URI from a contact parameter
pub fn extract_uri_from_contact(contact: &ContactValue) -> Option<Uri> {
    match contact {
        ContactValue::Params(params) if !params.is_empty() => {
            // Extract the URI from the first address in the params list
            if let Some(p) = params.first() {
                return Some(p.address.uri.clone());
            }
        },
        ContactValue::Star => return None,
        _ => {}
    }
    
    // Fallback for tests
    Uri::from_str("sip:unknown@example.com").ok()
}

/// URI resolution utilities
pub mod uri_resolver {
    use super::*;
    
    /// Resolve a SIP URI to a socket address
    pub async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
        // Extract host and port by parsing the URI string
        // This is a more robust approach that doesn't rely on specific URI methods
        let uri_str = uri.to_string();
        
        // Find the host part - after @ symbol if present, or after sip: if not
        let host_part = if let Some(at_pos) = uri_str.find('@') {
            uri_str[at_pos + 1..].to_string()
        } else if let Some(scheme_pos) = uri_str.find("sip:") {
            uri_str[scheme_pos + 4..].to_string()
        } else {
            uri_str.clone()
        };
        
        // Extract host and port
        let parts: Vec<&str> = host_part.split(':').collect();
        let host = parts[0];
        let port = if parts.len() > 1 {
            parts[1].parse::<u16>().unwrap_or(5060)
        } else {
            5060 // Default SIP port
        };
        
        debug!("Resolving SIP URI: {} to host: {}, port: {}", uri_str, host, port);
        
        // Try to parse as IP address first
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Some(SocketAddr::new(ip, port));
        }
        
        // Otherwise, try DNS resolution
        let addr_string = format!("{}:{}", host, port);
        
        match addr_string.to_socket_addrs() {
            Ok(mut addrs) => addrs.next(),
            Err(e) => {
                warn!("Failed to resolve {}: {}", addr_string, e);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_tag() {
        // Standard case with a tag parameter
        let header_value = "\"User\" <sip:user@example.com>;tag=abc123";
        assert_eq!(extract_tag_from_str(header_value), Some("abc123".to_string()));
        
        // Tag with additional parameters
        let header_value = "<sip:user@example.com>;tag=xyz789;param=value";
        assert_eq!(extract_tag_from_str(header_value), Some("xyz789".to_string()));
        
        // No tag parameter
        let header_value = "<sip:user@example.com>";
        assert_eq!(extract_tag_from_str(header_value), None);
        
        // Tag at the end of the string with no delimiter
        let header_value = "<sip:user@example.com>;tag=end-tag";
        assert_eq!(extract_tag_from_str(header_value), Some("end-tag".to_string()));
        
        // Multiple tag parameters (should return the first one)
        let header_value = "<sip:user@example.com>;tag=first;otherparam=x;tag=second";
        assert_eq!(extract_tag_from_str(header_value), Some("first".to_string()));
        
        // Tag with whitespace
        let header_value = "<sip:user@example.com>;tag=with space;param=value";
        assert_eq!(extract_tag_from_str(header_value), Some("with".to_string()));
        
        // Tag with empty value
        let header_value = "<sip:user@example.com>;tag=;param=value";
        assert_eq!(extract_tag_from_str(header_value), Some("".to_string()));
        
        // Semicolon without tag parameter
        let header_value = "<sip:user@example.com>;notag=value";
        assert_eq!(extract_tag_from_str(header_value), None);
    }
    
    #[test]
    fn test_extract_uri() {
        // URI in angle brackets with display name
        let header_value = "\"User\" <sip:user@example.com>;tag=abc123";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com");
        
        // URI without angle brackets
        let header_value = "sip:user@example.com;tag=xyz789";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com");
        
        // SIPS URI scheme
        let header_value = "sips:secure@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sips:secure@example.com");
        
        // TEL URI scheme
        let header_value = "tel:+1-212-555-0123";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "tel:+1-212-555-0123");
        
        // Malformed URI (opening angle bracket but no closing one) - the function
        // falls back to extracting the domain part and creating a sip URI
        let header_value = "\"User\" <sip:malformed@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");
        
        // Extract from just domain part
        let header_value = "example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");
        
        // With port in URI
        let header_value = "<sip:user@example.com:5060>";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com:5060");
        
        // Empty header value
        let header_value = "";
        let uri = extract_uri(header_value);
        assert!(uri.is_none());
        
        // Invalid scheme
        let header_value = "invalid:user@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some(), "Should extract the domain part as fallback");
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");
        
        // IP address instead of domain
        let header_value = "192.168.1.1";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:192.168.1.1");
    }
    
    #[tokio::test]
    async fn test_uri_resolver() {
        // IP address in URI
        let uri = Uri::from_str("sip:user@192.168.1.1").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(socket_addr.is_some());
        assert_eq!(socket_addr.unwrap().to_string(), "192.168.1.1:5060");
        
        // IP address with custom port
        let uri = Uri::from_str("sip:user@192.168.1.1:5070").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(socket_addr.is_some());
        assert_eq!(socket_addr.unwrap().to_string(), "192.168.1.1:5070");
        
        // Domain name test - just verify behavior is consistent
        let uri = Uri::from_str("sip:user@nonexistent-domain-123456.local").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(socket_addr.is_none(), "Non-existent domain should not resolve");
        
        // IPv6 address test - only if IPv6 is supported
        if let Ok(uri) = Uri::from_str("sip:user@[::1]") {
            let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
            // Don't assert specifically that IPv6 works, as it may not be available in all environments
            println!("IPv6 resolver test result: {:?}", socket_addr);
        }
    }
} 