// Tests for URI parsing logic in parser/uri.rs

use crate::common::{uri, addr, param_tag, param_expires, param_q, param_transport, param_method, param_user, param_other, param_received, param_ttl, param_lr, assert_parses_ok, assert_parse_fails, param_branch, param_maddr};
use rvoip_sip_core::parser::uri::parse_uri; // Assuming this is the main entry point
// Need to make parameter_parser pub or test via parse_uri
// use rvoip_sip_core::parser::uri::{parameter_parser};
use rvoip_sip_core::types::{Param, Method};
use rvoip_sip_core::types::uri::{Uri, Scheme, Host}; // Updated URI path
use rvoip_sip_core::types::param::GenericValue; // Add GenericValue import
use std::str::FromStr;
use std::net::IpAddr;
use std::collections::HashMap;
// use rvoip_sip_core::error::SipError; // Commented out - likely not public

// Helper function to parse just parameters for focused tests
// Requires making parameter_parser pub or similar visibility adjustment.
/*
fn parse_just_param(input: &str) -> Param {
    // Need to strip leading ';' for parameter_parser
    if input.starts_with(';') {
        match parameter_parser(&input[1..]) {
            Ok((_, param)) => param,
            Err(e) => panic!("Failed to parse param '{}': {:?}", input, e),
        }
    } else {
        panic!("Invalid param input for test: {}", input);
    }
}
*/

// TODO: Adapt existing tests from src/parser/uri.rs
// TODO: Add specific tests for parameter_parser and parameters_parser

#[test]
fn test_uri_from_str() {
    /// Based on RFC 3261 Section 19.1.4 URI Parameters
    let input = "sip:alice@example.com;transport=tcp;method=REGISTER";
    let uri = Uri::from_str(input).expect("Failed to parse URI");

    assert_eq!(uri.scheme, Scheme::Sip);
    assert!(matches!(uri.host, Host::Domain(d) if d == "example.com"));
    assert_eq!(uri.user.as_deref(), Some("alice"));
    
    assert!(uri.parameters.contains(&Param::Transport("tcp".to_string())));
    assert!(uri.parameters.contains(&Param::Method("REGISTER".to_string())));
    assert_eq!(uri.parameters.len(), 2);
}

#[test]
fn test_uri_with_flag_param() {
    /// Based on RFC 3261 Section 20.42 Via Header Field (lr param example)
    let input = "sip:user@host.com;lr";
    let uri = Uri::from_str(input).expect("Failed to parse URI");
    assert!(uri.parameters.contains(&Param::Lr));
    assert_eq!(uri.parameters.len(), 1);
}

#[test]
fn test_uri_with_escaped_param() {
    /// Test parsing escaped characters in parameters
    let input = "sip:user@host;param=hello%20world";
    let uri = Uri::from_str(input).expect("Failed to parse URI");
    
    // Create the expected param with GenericValue
    let expected_param = Param::Other(
        "param".to_string(), 
        Some(GenericValue::Token("hello world".to_string()))
    );
    
    // Check if the param list contains a matching param
    let has_param = uri.parameters.iter().any(|p| {
        if let Param::Other(key, Some(GenericValue::Token(val))) = p {
            key == "param" && val == "hello world"
        } else {
            false
        }
    });
    
    assert!(has_param, "URI should contain param with escaped value");
}

// Add more tests for edge cases, different param types (q, ttl, expires, received, etc.) 

// --- Tests adapted from src/parser/uri.rs --- 

#[test]
fn test_parse_simple_uri_adapted() {
    /// RFC 3261 Section 19.1: SIP and SIPS URIs
    let uri = Uri {
        scheme: Scheme::Sip,
        user: None,
        password: None,
        host: Host::domain("example.com"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sip:example.com".to_string()),
    };
    assert_parses_ok("sip:example.com", uri);
    
    let uri = Uri {
        scheme: Scheme::Sips,
        user: None,
        password: None,
        host: Host::domain("secure.example.com"),
        port: Some(5061),
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sips:secure.example.com:5061".to_string()),
    };
    assert_parses_ok("sips:secure.example.com:5061", uri);
}

#[test]
fn test_parse_complex_uri_adapted() {
    /// RFC 3261 Section 19.1: SIP and SIPS URIs (with params/headers)
    let expected_uri = Uri {
        scheme: Scheme::Sip,
        user: Some("alice".to_string()),
        password: None,
        host: Host::domain("example.com"),
        port: None,
        parameters: vec![param_transport("tcp"), param_lr()],
        headers: {
            let mut map = std::collections::HashMap::new();
            map.insert("subject".to_string(), "Meeting".to_string());
            map
        },
        raw_uri: Some("sip:alice@example.com;transport=tcp;lr?subject=Meeting".to_string()),
    };
    assert_parses_ok("sip:alice@example.com;transport=tcp;lr?subject=Meeting", expected_uri);
}

#[test]
fn test_tel_uri_adapted() {
    /// RFC 3966: The tel URI for Telephone Numbers
    let uri = Uri {
        scheme: Scheme::Tel,
        user: None,
        password: None,
        host: Host::domain("+1-212-555-0123"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("tel:+1-212-555-0123".to_string()),
    };
    assert_parses_ok("tel:+1-212-555-0123", uri);
}

#[test]
fn test_escaped_uri_adapted() {
    /// RFC 3261 Section 19.1.1: Escaping
    let expected = Uri {
        scheme: Scheme::Sip, 
        user: Some("user with spaces".to_string()), 
        password: None,
        host: Host::domain("example.com"),
        port: None,
        parameters: vec![param_other("param", Some("value with spaces"))],
        headers: HashMap::new(),
        raw_uri: Some("sip:user%20with%20spaces@example.com;param=value%20with%20spaces".to_string()),
    };
    assert_parses_ok("sip:user%20with%20spaces@example.com;param=value%20with%20spaces", expected);
}

// Note: is_valid_ipv4 and is_valid_ipv6 tests are not moved as they test private helper functions.
// Consider making them pub and testing directly if needed.

// --- New specific tests for parameter parsing --- 

#[test]
fn test_parse_param_types() {
    /// RFC 3261 various sections for specific parameters
    let input = "sip:host;branch=1;tag=abc;expires=60;ttl=128;q=0.5;received=1.2.3.4;maddr=4.3.2.1;user=phone;transport=tls;method=INVITE;foo=bar;baz";
    let result = Uri::from_str(input);
    assert!(result.is_ok());
    let uri = result.unwrap();
    
    assert!(uri.parameters.contains(&param_branch("1")));
    assert!(uri.parameters.contains(&param_tag("abc")));
    assert!(uri.parameters.contains(&param_expires(60)));
    assert!(uri.parameters.contains(&param_ttl(128)));
    assert!(uri.parameters.iter().any(|p| matches!(p, Param::Q(v) if (*v - 0.5).abs() < f32::EPSILON )));
    assert!(uri.parameters.contains(&param_received("1.2.3.4")));
    assert!(uri.parameters.contains(&param_maddr("4.3.2.1")));
    assert!(uri.parameters.contains(&param_user("phone")));
    assert!(uri.parameters.contains(&param_transport("tls")));
    assert!(uri.parameters.contains(&param_method("INVITE")));
    assert!(uri.parameters.contains(&param_other("foo", Some("bar"))));
    assert!(uri.parameters.contains(&param_other("baz", None))); // Flag param parsed as Other 
}

#[test]
fn test_parse_param_lr_flag() {
    /// RFC 3261 Section 20.42 Via Header Field (lr param example)
    let uri = Uri { 
        scheme: Scheme::Sip, 
        user: Some("user".to_string()),
        password: None,
        host: Host::domain("host.com"),
        port: None,
        parameters: vec![param_lr()],
        headers: HashMap::new(),
        raw_uri: Some("sip:user@host.com;lr".to_string()),
    };
    assert_parses_ok("sip:user@host.com;lr", uri);
}

#[test]
fn test_parse_param_case_insensitive() {
    /// Parameter names are case-insensitive (RFC 3261 Section 19.1.4)
    let uri = Uri { 
        scheme: Scheme::Sip, 
        user: None,
        password: None,
        host: Host::domain("host"),
        port: None,
        parameters: vec![param_branch("xyz"), param_transport("UDP")],
        headers: HashMap::new(),
        raw_uri: Some("sip:host;BRANCH=xyz;Transport=UDP".to_string()),
    };
    assert_parses_ok("sip:host;BRANCH=xyz;Transport=UDP", uri);
}

#[test]
fn test_parse_param_invalid_values() {
    /// Test that invalid values for known params fall back to Other
    let uri = Uri { 
        scheme: Scheme::Sip, 
        user: None,
        password: None,
        host: Host::domain("host"),
        port: None,
        parameters: vec![
            param_other("expires", Some("bad")),
            param_other("ttl", Some("999")),
            param_other("q", Some("xyz")),
            param_other("received", Some("invalid-ip"))
        ],
        headers: HashMap::new(),
        raw_uri: Some("sip:host;expires=bad;ttl=999;q=xyz;received=invalid-ip".to_string()),
    };
    assert_parses_ok("sip:host;expires=bad;ttl=999;q=xyz;received=invalid-ip", uri);
}

#[test]
fn test_parse_uri_edge_cases() {
    // No user, no params, no headers
    let uri = Uri {
        scheme: Scheme::Sip,
        user: None,
        password: None,
        host: Host::domain("example.com"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sip:example.com".to_string()),
    };
    assert_parses_ok("sip:example.com", uri);
    
    // User only
    let uri = Uri {
        scheme: Scheme::Sip,
        user: Some("alice".to_string()),
        password: None,
        host: Host::domain("atlanta.com"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sip:alice@atlanta.com".to_string()),
    };
    assert_parses_ok("sip:alice@atlanta.com", uri);
    
    // User and password (discouraged)
    let uri = Uri {
        scheme: Scheme::Sip,
        user: Some("alice".to_string()),
        password: Some("secret".to_string()),
        host: Host::domain("atlanta.com"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sip:alice:secret@atlanta.com".to_string()),
    };
    assert_parses_ok("sip:alice:secret@atlanta.com", uri);
    
    // Host is IPv4
    let uri = Uri {
        scheme: Scheme::Sip,
        user: None,
        password: None,
        host: Host::ipv4("192.168.0.1"),
        port: None,
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sip:192.168.0.1".to_string()),
    };
    assert_parses_ok("sip:192.168.0.1", uri);
    
    // Host is IPv6
    let uri = Uri {
        scheme: Scheme::Sips,
        user: None,
        password: None,
        host: Host::ipv6("2001:db8::1"),
        port: Some(5061),
        parameters: vec![],
        headers: HashMap::new(),
        raw_uri: Some("sips:[2001:db8::1]:5061".to_string()),
    };
    assert_parses_ok("sips:[2001:db8::1]:5061", uri);
    
    // Params but no user
    let uri = Uri {
        scheme: Scheme::Sip,
        user: None,
        password: None,
        host: Host::domain("host.com"),
        port: None,
        parameters: vec![param_transport("udp")],
        headers: HashMap::new(),
        raw_uri: Some("sip:host.com;transport=udp".to_string()),
    };
    assert_parses_ok("sip:host.com;transport=udp", uri);
    
    // Headers but no params
    let uri = Uri { 
        scheme: Scheme::Sip, 
        user: None,
        password: None,
        host: Host::domain("host.com"), 
        port: None,
        parameters: vec![],
        headers: { let mut h=HashMap::new(); h.insert("Subject".to_string(), "Hello".to_string()); h },
        raw_uri: Some("sip:host.com?Subject=Hello".to_string()),
    };
    assert_parses_ok("sip:host.com?Subject=Hello", uri);
    
    // Empty parameter value
    let uri = Uri {
        scheme: Scheme::Sip,
        user: None,
        password: None,
        host: Host::domain("host.com"),
        port: None,
        parameters: vec![param_other("foo", Some(""))],
        headers: HashMap::new(),
        raw_uri: Some("sip:host.com;foo=".to_string()),
    };
    assert_parses_ok("sip:host.com;foo=", uri);
}

#[test]
fn test_parse_uri_invalid() {
    /// Invalid formats based on RFC 3261 Section 25 ABNF
    assert_parse_fails::<Uri>("sip:"); // Missing host
    assert_parse_fails::<Uri>("sip:user@"); // Missing host
    assert_parse_fails::<Uri>("sip:user@:5060"); // Missing host
    assert_parse_fails::<Uri>("example.com"); // Missing scheme
    assert_parse_fails::<Uri>("sip:host;=value"); // Missing param name
    assert_parse_fails::<Uri>("sip:host?=value"); // Missing header name
    assert_parse_fails::<Uri>("sip:host?name"); // Missing header value
    assert_parse_fails::<Uri>("sip:user name@host"); // Space in user
    assert_parse_fails::<Uri>("sip:[::1]:badport"); // Invalid port
} 