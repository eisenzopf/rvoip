use std::str::FromStr;
use rvoip_sip_core::types::uri::Uri;

#[test]
fn test_invalid_uris() {
    // Empty URI should fail
    let result = Uri::from_str("");
    assert!(result.is_err(), "Empty URI should be rejected");
    
    // URI with only a colon and digits should fail
    let result = Uri::from_str(":1234");
    assert!(result.is_err(), "URI without scheme should be rejected");
    
    // Invalid scheme should fail
    let result = Uri::from_str("invalid:test");
    assert!(result.is_err(), "URI with invalid scheme should be rejected");
    
    // Unclosed IPv6 bracket should fail
    let result = Uri::from_str("sip:user@[2001:db8::1");
    assert!(result.is_err(), "Unclosed IPv6 bracket should be rejected");
    
    // URI with missing host part should fail
    let result = Uri::from_str("sip:@");
    assert!(result.is_err(), "URI with missing host should be rejected");
    
    // URI with empty user part should fail
    let result = Uri::from_str("sip:@example.com");
    assert!(result.is_err(), "URI with empty user part should be rejected");
    
    // URI with invalid port should fail
    let result = Uri::from_str("sip:user@example.com:port");
    assert!(result.is_err(), "URI with non-numeric port should be rejected");
    
    // URI with port out of range should fail
    let result = Uri::from_str("sip:user@example.com:99999999");
    assert!(result.is_err(), "URI with port value out of range should be rejected");
}

#[test]
fn test_valid_uris() {
    // Basic SIP URI
    let result = Uri::from_str("sip:alice@example.com");
    assert!(result.is_ok(), "Valid basic SIP URI should be accepted");
    
    // SIP URI with port
    let result = Uri::from_str("sip:alice@example.com:5060");
    assert!(result.is_ok(), "Valid SIP URI with port should be accepted");
    
    // SIP URI with parameters
    let result = Uri::from_str("sip:alice@example.com;transport=tcp");
    assert!(result.is_ok(), "Valid SIP URI with parameters should be accepted");
    
    // SIP URI with headers
    let result = Uri::from_str("sip:alice@example.com?subject=meeting");
    assert!(result.is_ok(), "Valid SIP URI with headers should be accepted");
    
    // SIPS URI
    let result = Uri::from_str("sips:alice@example.com");
    assert!(result.is_ok(), "Valid SIPS URI should be accepted");
    
    // TEL URI
    let result = Uri::from_str("tel:+1-212-555-0101");
    assert!(result.is_ok(), "Valid TEL URI should be accepted");
    
    // URI with IPv4 host
    let result = Uri::from_str("sip:alice@192.168.1.1");
    assert!(result.is_ok(), "Valid SIP URI with IPv4 host should be accepted");
    
    // URI with IPv6 host
    let result = Uri::from_str("sip:alice@[2001:db8::1]");
    assert!(result.is_ok(), "Valid SIP URI with IPv6 host should be accepted");
} 