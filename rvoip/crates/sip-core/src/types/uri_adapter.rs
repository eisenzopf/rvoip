use std::fmt;
use std::str::FromStr;
use fluent_uri::Uri as FluentUri;
use serde::{Serialize, Deserialize};
use std::net::{IpAddr, Ipv6Addr};

use crate::error::{Error, Result};
use crate::types::param::Param;
use crate::types::uri::{Uri, Scheme, Host};

/// Adapter struct to work with fluent-uri and our SIP URI types
#[derive(Debug, Clone)]
pub struct UriAdapter;

impl UriAdapter {
    /// Convert a string to a SIP Uri using fluent-uri for parsing
    pub fn parse_uri(uri_str: &str) -> Result<Uri> {
        // Check for recognized schemes first
        let scheme_end = uri_str.find(':').unwrap_or(0);
        let scheme = if scheme_end > 0 { &uri_str[0..scheme_end] } else { "" };
        
        match scheme.to_lowercase().as_str() {
            "sip" | "sips" | "tel" => {
                // Handle standard SIP schemes
                Self::parse_standard_uri(uri_str)
            },
            // For any other scheme, store as raw URI
            "" | _ => {
                if scheme.is_empty() {
                    // No scheme, assume SIP
                    Self::parse_standard_uri(uri_str)
                } else {
                    // Custom scheme, preserve as-is
                    Ok(Uri::custom(uri_str))
                }
            }
        }
    }
    
    /// Special handling for SIP URIs with IPv6 addresses
    /// This addresses a limitation in some URI parsers with IPv6 literals
    fn handle_sip_ipv6_uri(uri_str: &str) -> Option<Uri> {
        let sip_prefix = if uri_str.starts_with("sip:") {
            "sip:"
        } else if uri_str.starts_with("sips:") {
            "sips:"
        } else {
            return None;
        };
        
        // Check if we have an IPv6 address in the URI
        let ipv6_start = uri_str.find('[');
        let ipv6_end = uri_str.find(']');
        
        if ipv6_start.is_none() || ipv6_end.is_none() || ipv6_start.unwrap() >= ipv6_end.unwrap() {
            return None;
        }
        
        // Extract the IPv6 address
        let ipv6_start = ipv6_start.unwrap();
        let ipv6_end = ipv6_end.unwrap();
        let ipv6_addr_str = &uri_str[ipv6_start+1..ipv6_end];
        
        // Parse the IPv6 address
        let ipv6_addr = match Ipv6Addr::from_str(ipv6_addr_str) {
            Ok(addr) => addr,
            Err(_) => return None,
        };
        
        // Determine if this is a user@host format
        let has_userinfo = uri_str[sip_prefix.len()..ipv6_start].contains('@');
        let mut user = None;
        
        if has_userinfo {
            let user_part = &uri_str[sip_prefix.len()..ipv6_start];
            let at_pos = user_part.find('@').unwrap();
            user = Some(user_part[0..at_pos].to_string());
        }
        
        // Create the URI with the IPv6 host
        let mut uri = Uri::new(
            if sip_prefix == "sip:" { Scheme::Sip } else { Scheme::Sips },
            Host::Address(IpAddr::V6(ipv6_addr))
        );
        
        // Set the user if we have one
        if let Some(u) = user {
            uri.user = Some(u);
        }
        
        // Check for port
        if ipv6_end + 1 < uri_str.len() && uri_str.chars().nth(ipv6_end + 1) == Some(':') {
            let rest = &uri_str[ipv6_end+2..];
            let port_end = rest.find(|c| c == ';' || c == '?' || c == ' ').unwrap_or(rest.len());
            let port_str = &rest[0..port_end];
            
            if let Ok(port) = port_str.parse::<u16>() {
                if port > 0 {
                    uri.port = Some(port);
                }
            }
            
            // Check for parameters or headers
            if port_end < rest.len() {
                // For complex cases, store the original URI
                uri.raw_uri = Some(uri_str.to_string());
            }
        } else if ipv6_end + 1 < uri_str.len() {
            // For parameters, headers, etc., store the original URI
            uri.raw_uri = Some(uri_str.to_string());
        }
        
        Some(uri)
    }
    
    /// Parse standard SIP URI schemes
    fn parse_standard_uri(uri_str: &str) -> Result<Uri> {
        // Special handling for SIP URIs with IPv6 addresses
        if let Some(uri) = Self::handle_sip_ipv6_uri(uri_str) {
            return Ok(uri);
        }
        
        // Pre-validate SIP URIs with userinfo part
        if uri_str.contains('@') {
            // Check for empty userinfo in the format "sip:@host" or "sips:@host"
            if uri_str.starts_with("sip:@") || uri_str.starts_with("sips:@") {
                return Err(Error::InvalidUri(format!(
                    "Invalid SIP URI: Empty userinfo with '@' separator is not allowed: {}", 
                    uri_str
                )));
            }
            
            // Check for @ at the end (missing host after @)
            if uri_str.ends_with('@') || uri_str.matches('@').count() > 1 {
                return Err(Error::InvalidUri(format!(
                    "Invalid SIP URI: Malformed userinfo part (missing host after @ or multiple @ symbols): {}", 
                    uri_str
                )));
            }
        }
        
        // Check for empty SIP URIs like "sip:" or "sips:" with no host
        if uri_str == "sip:" || uri_str == "sips:" || uri_str == "tel:" {
            return Err(Error::InvalidUri(format!(
                "Invalid URI: Missing host part: {}", uri_str
            )));
        }
        
        // Parse with fluent-uri first to validate and extract components
        let flu_uri = FluentUri::parse(uri_str)
            .map_err(|e| Error::InvalidUri(format!("Invalid URI: {}", e)))?;
        
        // Check if the URI has a scheme
        let scheme_opt = flu_uri.scheme();
        let scheme_str = scheme_opt.as_str();
        
        if scheme_str.is_empty() {
            // No scheme, create a simple URI
            let path = flu_uri.path().as_str();
            Ok(Uri::sip(path))
        } else {
            // Handle known schemes
            match scheme_str {
                "sip" => {
                    // Extract the host and other components
                    // For SIP URIs in Call-Info headers, the authority might not be parsed correctly
                    // by fluent-uri. In that case, fall back to a direct implementation.
                    if let Some(authority) = flu_uri.authority() {
                        let host_str = authority.host();
                        
                        // Additional validation: host part cannot be empty
                        if host_str.is_empty() {
                            return Err(Error::InvalidUri(format!("Invalid SIP URI: Empty host: {}", uri_str)));
                        }
                        
                        let host = Host::from_str(host_str)?;
                        
                        let mut uri = Uri::new(Scheme::Sip, host);
                        
                        // Extract userinfo if present
                        if let Some(userinfo) = authority.userinfo() {
                            // Additional validation: empty userinfo with @ is not allowed
                            if userinfo.is_empty() {
                                return Err(Error::InvalidUri(format!(
                                    "Invalid SIP URI: Empty userinfo: {}", uri_str
                                )));
                            }
                            uri.user = Some(userinfo.to_string());
                        }
                        
                        // Extract port if present
                        if let Some(port) = authority.port_to_u16().ok().flatten() {
                            // Don't set port if it's 0
                            if port > 0 {
                                uri.port = Some(port);
                            }
                        }
                        
                        // Extract any path or query components that might be present
                        if !flu_uri.path().as_str().is_empty() && flu_uri.path().as_str() != "/" {
                            // Store the full URI to preserve path components
                            uri.raw_uri = Some(uri_str.to_string());
                        }
                        
                        Ok(uri)
                    } else {
                        // Authority parse failed - for SIP URIs in Call-Info context,
                        // we'll preserve the full string to avoid data loss
                        let mut uri = Uri::custom(uri_str);
                        Ok(uri)
                    }
                },
                "sips" => {
                    // Similar to SIP URIs but with the Sips scheme
                    if let Some(authority) = flu_uri.authority() {
                        let host_str = authority.host();
                        
                        // Additional validation: host part cannot be empty
                        if host_str.is_empty() {
                            return Err(Error::InvalidUri(format!("Invalid SIPS URI: Empty host: {}", uri_str)));
                        }
                        
                        let host = Host::from_str(host_str)?;
                        
                        let mut uri = Uri::new(Scheme::Sips, host);
                        
                        // Extract userinfo if present
                        if let Some(userinfo) = authority.userinfo() {
                            // Additional validation: empty userinfo with @ is not allowed
                            if userinfo.is_empty() {
                                return Err(Error::InvalidUri(format!(
                                    "Invalid SIPS URI: Empty userinfo: {}", uri_str
                                )));
                            }
                            uri.user = Some(userinfo.to_string());
                        }
                        
                        // Extract port if present
                        if let Some(port) = authority.port_to_u16().ok().flatten() {
                            // Don't set port if it's 0
                            if port > 0 {
                                uri.port = Some(port);
                            }
                        }
                        
                        // Extract any path or query components
                        if !flu_uri.path().as_str().is_empty() && flu_uri.path().as_str() != "/" {
                            uri.raw_uri = Some(uri_str.to_string());
                        }
                        
                        Ok(uri)
                    } else {
                        // Fall back to custom URI for consistency
                        let uri = Uri::custom(uri_str);
                        Ok(uri)
                    }
                },
                "tel" => {
                    // Tel URI - just the number part
                    let path = flu_uri.path();
                    let number = path.as_str();
                    Ok(Uri::tel(number))
                },
                // For other schemes - preserve the original URI string
                _ => {
                    // For HTTP, HTTPS, etc. - preserve the original URI string
                    Ok(Uri::custom(uri_str))
                }
            }
        }
    }
    
    /// Convert our SIP URI to a fluent-uri Uri
    pub fn to_fluent_uri(uri: &Uri) -> Result<FluentUri<String>> {
        // If it's a custom URI, use the raw string
        if let Some(raw_uri) = &uri.raw_uri {
            return FluentUri::parse(raw_uri.to_string())
                .map_err(|e| Error::InvalidUri(format!("Could not convert to fluent-uri: {}", e)));
        }
        
        // Otherwise use the standard format
        let uri_str = uri.to_string();
        FluentUri::parse(uri_str)
            .map_err(|e| Error::InvalidUri(format!("Could not convert to fluent-uri: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_uri_adapter_basic() {
        // Test HTTP URI
        let http_uri = "http://www.example.com/alice/photo.jpg";
        let result = UriAdapter::parse_uri(http_uri);
        assert!(result.is_ok(), "Failed to parse HTTP URI");
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), http_uri);
        assert!(parsed.is_custom(), "HTTP URI should be marked as custom");
        
        // Test SIP URI
        let sip_uri = "sip:alice@example.com";
        let result = UriAdapter::parse_uri(sip_uri);
        assert!(result.is_ok(), "Failed to parse SIP URI");
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), sip_uri);
        assert!(!parsed.is_custom(), "SIP URI should not be marked as custom");
        
        // Test TEL URI
        let tel_uri = "tel:+1-212-555-1234";
        let result = UriAdapter::parse_uri(tel_uri);
        assert!(result.is_ok(), "Failed to parse TEL URI");
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), tel_uri);
        assert!(!parsed.is_custom(), "TEL URI should not be marked as custom");
    }
    
    #[test]
    fn test_uri_adapter_complex() {
        // Test SIP URI with path components
        let sip_uri = "sip:alice@example.com/path/to/resource";
        let result = UriAdapter::parse_uri(sip_uri);
        assert!(result.is_ok(), "Failed to parse SIP URI with path");
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), sip_uri);
        
        // Test SIP URI with parameters
        let sip_uri_params = "sip:alice@example.com;transport=tcp;user=phone";
        let result = UriAdapter::parse_uri(sip_uri_params);
        assert!(result.is_ok(), "Failed to parse SIP URI with parameters");
        
        // Test SIP URI with query string
        let sip_uri_query = "sip:alice@example.com?subject=meeting&priority=urgent";
        let result = UriAdapter::parse_uri(sip_uri_query);
        assert!(result.is_ok(), "Failed to parse SIP URI with query string");
        
        // Test URI with encoded characters
        let uri_encoded = "http://example.com/path%20with%20spaces";
        let result = UriAdapter::parse_uri(uri_encoded);
        assert!(result.is_ok(), "Failed to parse URI with encoded characters");
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), uri_encoded);
    }
    
    #[test]
    fn test_uri_adapter_round_trip() {
        let uris = [
            "http://www.example.com/alice/photo.jpg",
            "https://secure.example.com/alice/photo.jpg?param=value",
            "sip:alice@example.com",
            "sips:bob@secure.example.com:5061",
            "tel:+1-212-555-1234",
            "sip:conference@example.com;transport=tcp",
            "mailto:user@example.com"
        ];
        
        for uri_str in uris.iter() {
            let parsed = UriAdapter::parse_uri(uri_str).unwrap();
            assert_eq!(parsed.to_string(), *uri_str, "URI was not preserved in round-trip: {}", uri_str);
        }
    }
    
    #[test]
    fn test_invalid_sip_uris() {
        // Test cases for invalid SIP URIs
        let invalid_uris = [
            "sip:@example.com",  // Missing user part (empty user before @)
            "sip:",              // Missing host part
            "sip:example.com@",  // User but no host after @
            "sip:@",             // Just @ without user or host
            "sip@example.com",   // Missing colon after scheme
        ];
        
        for uri_str in invalid_uris.iter() {
            let result = UriAdapter::parse_uri(uri_str);
            assert!(result.is_err(), "Invalid URI '{}' should have been rejected", uri_str);
        }
    }
    
    #[test]
    fn test_fluent_uri_parsing() {
        // Direct test with fluent-uri to see how it handles empty userinfo
        let uri_str = "sip:@example.com";
        let flu_result = fluent_uri::Uri::parse(uri_str);
        
        // This is a debugging test to see how fluent-uri handles this case
        if let Ok(flu_uri) = flu_result {
            if let Some(authority) = flu_uri.authority() {
                let userinfo = authority.userinfo();
                println!("Fluent-URI parsed 'sip:@example.com' with userinfo: {:?}", userinfo);
                // Check if userinfo is empty string or None
                assert!(userinfo.is_none() || userinfo.unwrap().is_empty(), 
                   "UserInfo should be None or empty for 'sip:@example.com'");
            }
        }
    }
} 