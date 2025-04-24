use std::fmt;
use std::str::FromStr;
use fluent_uri::Uri as FluentUri;
use serde::{Serialize, Deserialize};

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
    
    /// Parse standard SIP URI schemes
    fn parse_standard_uri(uri_str: &str) -> Result<Uri> {
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
                        let host = Host::from_str(host_str)?;
                        
                        let mut uri = Uri::new(Scheme::Sip, host);
                        
                        // Extract userinfo if present
                        if let Some(userinfo) = authority.userinfo() {
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
                        let host = Host::from_str(host_str)?;
                        
                        let mut uri = Uri::new(Scheme::Sips, host);
                        
                        // Extract userinfo if present
                        if let Some(userinfo) = authority.userinfo() {
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
} 