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
                            uri.port = Some(port);
                        }
                        
                        // We could extract parameters here as well
                        // For now just returning the basic URI
                        
                        Ok(uri)
                    } else {
                        // SIP URI without authority
                        Err(Error::InvalidUri("SIP URI requires authority".to_string()))
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
                            uri.port = Some(port);
                        }
                        
                        Ok(uri)
                    } else {
                        Err(Error::InvalidUri("SIPS URI requires authority".to_string()))
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
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), http_uri);
        
        // Test SIP URI
        let sip_uri = "sip:alice@example.com";
        let result = UriAdapter::parse_uri(sip_uri);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), sip_uri);
        
        // Test TEL URI
        let tel_uri = "tel:+1-212-555-1234";
        let result = UriAdapter::parse_uri(tel_uri);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.to_string(), tel_uri);
    }
} 