// Modern parser tests for SIP URIs
// This file tests the current implementation of the SIP URI parsers

use std::str::FromStr;

// Import SIP Core types with specific imports
use rvoip_sip_core::{
    types::uri::{Uri, Scheme},
    types::Param,
    error::Error,
};

#[test]
fn test_parse_sip_uri() {
    // Test basic SIP URI
    let uri_str = "sip:user@example.com";
    let uri = Uri::from_str(uri_str).expect("Failed to parse basic SIP URI");
    assert_eq!(uri.scheme, Scheme::Sip, "URI scheme should be SIP");
    assert_eq!(uri.user.as_deref(), Some("user"), "User part should be 'user'");
    assert_eq!(uri.host.to_string(), "example.com", "Host should be 'example.com'");
    assert!(uri.port.is_none() || uri.port == Some(0), "Port should be None or 0");
    assert!(uri.parameters.is_empty(), "URI should have no parameters");
    assert!(uri.headers.is_empty(), "URI should have no headers");
    
    // Test URI with port
    let uri_str = "sip:user@example.com:5060";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with port");
    assert_eq!(uri.scheme, Scheme::Sip, "URI scheme should be SIP");
    assert_eq!(uri.port, Some(5060), "Port should be 5060");
    
    // Test URI with parameters
    let uri_str = "sip:user@example.com;transport=tcp;ttl=5";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with parameters");
    assert_eq!(uri.scheme, Scheme::Sip, "URI scheme should be SIP");
    assert_eq!(uri.user.as_deref(), Some("user"), "User part should be 'user'");
    assert_eq!(uri.host.to_string(), "example.com", "Host should be 'example.com'");
    
    // Test URI with headers
    let uri_str = "sip:user@example.com?subject=Meeting&priority=urgent";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with headers");
    assert_eq!(uri.headers.len(), 2, "URI should have 2 headers");
    assert_eq!(uri.headers.get("subject"), Some(&"Meeting".to_string()), "Subject header should be 'Meeting'");
    assert_eq!(uri.headers.get("priority"), Some(&"urgent".to_string()), "Priority header should be 'urgent'");
    
    // Test URI without user part
    let uri_str = "sip:example.com";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI without user part");
    assert_eq!(uri.scheme, Scheme::Sip, "URI scheme should be SIP");
    assert_eq!(uri.user, None, "User part should be None");
    assert_eq!(uri.host.to_string(), "example.com", "Host should be 'example.com'");
    
    // Test SIPS scheme
    let uri_str = "sips:secure@example.com";
    let uri = Uri::from_str(uri_str).expect("Failed to parse SIPS URI");
    assert_eq!(uri.scheme, Scheme::Sips, "URI scheme should be SIPS");
    
    // Test URI with IPv4 address
    let uri_str = "sip:user@192.0.2.1";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with IPv4 address");
    assert_eq!(uri.host.to_string(), "192.0.2.1", "Host should be the IPv4 address '192.0.2.1'");
    
    // Test URI with IPv6 address
    let uri_str = "sip:user@[2001:db8::1]";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with IPv6 address");
    let host_str = uri.host.to_string();
    assert!(
        host_str == "[2001:db8::1]" || 
        host_str == "2001:db8::1", 
        "IPv6 host '{}' doesn't match expected format", host_str
    );
    
    // Test URI with password
    let uri_str = "sip:user:password@example.com";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with password");
    assert_eq!(uri.user.as_deref(), Some("user"), "User part should be 'user'");
    assert_eq!(uri.password.as_deref(), Some("password"), "Password should be 'password'");
    
    // Test complex URI
    let uri_str = "sips:alice:secretword@example.com:5061;transport=tls?subject=Project%20X&priority=urgent";
    let uri = Uri::from_str(uri_str).expect("Failed to parse complex URI");
    assert_eq!(uri.scheme, Scheme::Sips, "URI scheme should be SIPS");
    assert_eq!(uri.user.as_deref(), Some("alice"), "User part should be 'alice'");
    assert_eq!(uri.password.as_deref(), Some("secretword"), "Password should be 'secretword'");
    assert_eq!(uri.host.to_string(), "example.com", "Host should be 'example.com'");
    assert_eq!(uri.port, Some(5061), "Port should be 5061");
}

#[test]
fn test_parse_tel_uri() {
    // Test basic telephone URI
    let uri_str = "tel:+1-212-555-0101";
    let uri = Uri::from_str(uri_str).expect("Failed to parse basic TEL URI");
    assert_eq!(uri.scheme, Scheme::Tel, "URI scheme should be TEL");
    
    // Test TEL URI with parameters
    let uri_str = "tel:+1-212-555-0101;phone-context=example.com";
    let uri = Uri::from_str(uri_str).expect("Failed to parse TEL URI with parameters");
    assert_eq!(uri.scheme, Scheme::Tel, "URI scheme should be TEL");
    assert!(!uri.parameters.is_empty(), "URI should have parameters");
    
    // Test global number format
    let uri_str = "tel:+44-20-7946-0123";
    let uri = Uri::from_str(uri_str).expect("Failed to parse TEL URI with global number");
    assert_eq!(uri.scheme, Scheme::Tel, "URI scheme should be TEL");
}

#[test]
fn test_uri_to_string() {
    // Test round-trip for SIP URI
    let uri_str = "sip:alice@atlanta.com:5060;transport=tcp";
    let uri = Uri::from_str(uri_str).expect("Failed to parse SIP URI for round-trip test");
    assert_eq!(uri.to_string(), uri_str, "Round-trip conversion failed for SIP URI");
    
    // Test round-trip for SIPS URI with parameters and headers
    let uri_str = "sips:bob@biloxi.com:5061;transport=tls?subject=Meeting";
    let uri = Uri::from_str(uri_str).expect("Failed to parse SIPS URI for round-trip test");
    assert_eq!(uri.to_string(), uri_str, "Round-trip conversion failed for SIPS URI");
    
    // Test round-trip for URI with IPv6
    let uri_str = "sip:carol@[2001:db8::1]:5060";
    let uri = Uri::from_str(uri_str).expect("Failed to parse IPv6 URI for round-trip test");
    assert_eq!(uri.to_string(), uri_str, "Round-trip conversion failed for IPv6 URI");
}

#[test]
fn test_invalid_uris() {
    // Test missing scheme
    let result = Uri::from_str("user@example.com");
    assert!(result.is_err(), "URI without scheme should be rejected");
    
    // Test invalid scheme
    let result = Uri::from_str("invalid:user@example.com");
    assert!(result.is_err(), "URI with invalid scheme should be rejected");
    
    // Test malformed IPv6 address (missing closing bracket)
    let result = Uri::from_str("sip:user@[2001:db8::1");
    assert!(result.is_err(), "URI with unclosed IPv6 bracket should be rejected");
}

#[test]
fn test_uri_parameters() {
    // Test URI with transport parameter
    let uri_str = "sip:user@example.com;transport=tcp";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with transport parameter");
    assert_eq!(uri.scheme, Scheme::Sip, "URI scheme should be SIP");
    assert_eq!(uri.user.as_deref(), Some("user"), "User part should be 'user'");
    assert_eq!(uri.host.to_string(), "example.com", "Host should be 'example.com'");
    assert!(!uri.parameters.is_empty(), "URI should have parameters");
    
    // Check if transport parameter exists and has correct value
    assert!(uri.parameters.iter().any(|p| p.key() == "transport" && p.value().as_deref() == Some("tcp")),
            "URI should have transport=tcp parameter");
    
    // Test URI with multiple parameters
    let uri_str = "sip:user@example.com;transport=tls;ttl=5;method=INVITE";
    let uri = Uri::from_str(uri_str).expect("Failed to parse URI with multiple parameters");
    assert_eq!(uri.parameters.len(), 3, "URI should have 3 parameters");
    
    // Check each parameter individually
    assert!(uri.parameters.iter().any(|p| p.key() == "transport" && p.value().as_deref() == Some("tls")),
            "URI should have transport=tls parameter");
    assert!(uri.parameters.iter().any(|p| p.key() == "ttl" && p.value().as_deref() == Some("5")),
            "URI should have ttl=5 parameter");
    assert!(uri.parameters.iter().any(|p| p.key() == "method" && p.value().as_deref() == Some("INVITE")),
            "URI should have method=INVITE parameter");
} 