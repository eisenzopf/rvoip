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
    let uri = Uri::from_str("sip:user@example.com").unwrap();
    assert_eq!(uri.scheme, Scheme::Sip);
    assert_eq!(uri.user.as_deref(), Some("user"));
    assert_eq!(uri.host.to_string(), "example.com");
    assert_eq!(uri.port, None);
    assert!(uri.parameters.is_empty());
    assert!(uri.headers.is_empty());
    
    // Test with port
    let uri = Uri::from_str("sip:user@example.com:5060").unwrap();
    assert_eq!(uri.scheme, Scheme::Sip);
    assert_eq!(uri.port, Some(5060));
    
    // Test with parameters
    let uri = Uri::from_str("sip:user@example.com;transport=tcp;ttl=5").unwrap();
    assert_eq!(uri.scheme, Scheme::Sip);
    assert_eq!(uri.parameters.len(), 2);
    // Check parameters using pattern matching instead of fields
    assert!(uri.parameters.iter().any(|p| match p {
        Param::Transport(val) => val == "tcp",
        _ => false
    }));
    assert!(uri.parameters.iter().any(|p| match p {
        Param::Other(name, Some(val)) => name == "ttl" && val.as_str() == Some("5"),
        _ => false
    }));
    
    // Test with headers
    let uri = Uri::from_str("sip:user@example.com?subject=Meeting&priority=urgent").unwrap();
    assert_eq!(uri.headers.len(), 2);
    // Access headers using get instead of iter()
    assert_eq!(uri.headers.get("subject"), Some(&"Meeting".to_string()));
    assert_eq!(uri.headers.get("priority"), Some(&"urgent".to_string()));
    
    // Test without user part
    let uri = Uri::from_str("sip:example.com").unwrap();
    assert_eq!(uri.scheme, Scheme::Sip);
    assert_eq!(uri.user, None);
    assert_eq!(uri.host.to_string(), "example.com");
    
    // Test SIPS scheme
    let uri = Uri::from_str("sips:secure@example.com").unwrap();
    assert_eq!(uri.scheme, Scheme::Sips);
    
    // Test with IPv4 address
    let uri = Uri::from_str("sip:user@192.0.2.1").unwrap();
    assert_eq!(uri.host.to_string(), "192.0.2.1");
    
    // Test with IPv6 address
    let uri = Uri::from_str("sip:user@[2001:db8::1]").unwrap();
    assert_eq!(uri.host.to_string(), "[2001:db8::1]");
    
    // Test with password
    let uri = Uri::from_str("sip:user:password@example.com").unwrap();
    assert_eq!(uri.user.as_deref(), Some("user"));
    assert_eq!(uri.password.as_deref(), Some("password"));
    
    // Test complete complex URI
    let uri = Uri::from_str("sips:alice:secretword@example.com:5061;transport=tls?subject=Project%20X&priority=urgent").unwrap();
    assert_eq!(uri.scheme, Scheme::Sips);
    assert_eq!(uri.user.as_deref(), Some("alice"));
    assert_eq!(uri.password.as_deref(), Some("secretword"));
    assert_eq!(uri.host.to_string(), "example.com");
    assert_eq!(uri.port, Some(5061));
    assert!(uri.parameters.iter().any(|p| match p {
        Param::Transport(val) => val == "tls",
        _ => false
    }));
    assert_eq!(uri.headers.get("subject"), Some(&"Project%20X".to_string()));
    assert_eq!(uri.headers.get("priority"), Some(&"urgent".to_string()));
}

#[test]
fn test_parse_tel_uri() {
    // Test basic telephone URI
    let uri = Uri::from_str("tel:+1-212-555-0101").unwrap();
    assert_eq!(uri.scheme, Scheme::Tel);
    assert_eq!(uri.user.as_deref(), Some("+1-212-555-0101"));
    assert_eq!(uri.host.to_string(), "");
    
    // Test with parameters
    let uri = Uri::from_str("tel:+1-212-555-0101;phone-context=example.com").unwrap();
    assert_eq!(uri.scheme, Scheme::Tel);
    assert_eq!(uri.user.as_deref(), Some("+1-212-555-0101"));
    assert!(uri.parameters.iter().any(|p| match p {
        Param::Other(name, Some(val)) => 
            name == "phone-context" && val.as_str() == Some("example.com"),
        _ => false
    }));
    
    // Test global number
    let uri = Uri::from_str("tel:+44-20-7946-0123").unwrap();
    assert_eq!(uri.scheme, Scheme::Tel);
    assert_eq!(uri.user.as_deref(), Some("+44-20-7946-0123"));
}

#[test]
fn test_uri_to_string() {
    // Test round-trip for SIP URI
    let uri_str = "sip:alice@atlanta.com:5060;transport=tcp";
    let uri = Uri::from_str(uri_str).unwrap();
    assert_eq!(uri.to_string(), uri_str);
    
    // Test round-trip for SIPS URI with parameters and headers
    let uri_str = "sips:bob@biloxi.com:5061;transport=tls?subject=Meeting";
    let uri = Uri::from_str(uri_str).unwrap();
    assert_eq!(uri.to_string(), uri_str);
    
    // Test round-trip for URI with IPv6
    let uri_str = "sip:carol@[2001:db8::1]:5060";
    let uri = Uri::from_str(uri_str).unwrap();
    assert_eq!(uri.to_string(), uri_str);
}

#[test]
fn test_invalid_uris() {
    // Test missing scheme
    assert!(Uri::from_str("user@example.com").is_err());
    
    // Test invalid scheme
    assert!(Uri::from_str("invalid:user@example.com").is_err());
    
    // Test malformed URI
    assert!(Uri::from_str("sip:@example.com").is_err());
    
    // Test missing host
    assert!(Uri::from_str("sip:user@").is_err());
    
    // Test invalid port
    assert!(Uri::from_str("sip:user@example.com:abcd").is_err());
    
    // Test unmatched brackets for IPv6
    assert!(Uri::from_str("sip:user@[2001:db8::1").is_err());
}

#[test]
fn test_uri_parameters() {
    let uri = Uri::from_str("sip:user@example.com;transport=tcp;maddr=239.255.255.1;ttl=15").unwrap();
    
    // Test individual parameter access using find_param
    let transport_param = uri.parameters.iter().find(|p| match p {
        Param::Transport(_) => true,
        _ => false
    });
    assert!(transport_param.is_some());
    if let Some(Param::Transport(val)) = transport_param {
        assert_eq!(val, "tcp");
    }
    
    // Check maddr parameter
    let maddr_param = uri.parameters.iter().find(|p| match p {
        Param::Maddr(_) => true,
        _ => false
    });
    assert!(maddr_param.is_some());
    if let Some(Param::Maddr(val)) = maddr_param {
        assert_eq!(val, "239.255.255.1");
    }
    
    // Check ttl parameter
    let ttl_param = uri.parameters.iter().find(|p| match p {
        Param::Ttl(_) => true,
        _ => false
    });
    assert!(ttl_param.is_some());
    if let Some(Param::Ttl(val)) = ttl_param {
        assert_eq!(*val, 15);
    }
    
    // Test parameter case insensitivity with find_param_ignore_case
    let uri = Uri::from_str("sip:user@example.com;TRANSPORT=TCP;MADDR=239.255.255.1").unwrap();
    
    // Access Transport parameter (case insensitive)
    let transport_param = uri.parameters.iter().find(|p| match p {
        Param::Transport(_) => true,
        _ => false
    });
    assert!(transport_param.is_some());
    if let Some(Param::Transport(val)) = transport_param {
        assert_eq!(val.to_uppercase(), "TCP");
    }
    
    // Test valueless parameters
    let uri = Uri::from_str("sip:user@example.com;lr").unwrap();
    let lr_param = uri.parameters.iter().find(|p| match p {
        Param::Lr => true,
        _ => false
    });
    assert!(lr_param.is_some());
} 