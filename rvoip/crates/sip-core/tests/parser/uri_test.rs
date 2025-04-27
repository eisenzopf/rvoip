use std::str::FromStr;
use rvoip_sip_core::types::uri::Uri;

#[test]
fn test_invalid_uris() {
    // Missing scheme should fail
    assert!(Uri::from_str("").is_err(), "Empty URI should be rejected");
    assert!(Uri::from_str(":1234").is_err(), "URI without scheme should be rejected");
    
    // Invalid scheme should fail
    assert!(Uri::from_str("invalid:test").is_err(), "Invalid scheme should be rejected");
    
    // Unclosed IPv6 bracket should fail
    // BUG NOTICE: The IPv6 parser was previously allowing unclosed brackets.
    // This test ensures that URIs with unmatched brackets are properly rejected.
    assert!(Uri::from_str("sip:user@[2001:db8::1").is_err(), "Unclosed IPv6 bracket should be rejected");
    
    // Add more invalid URI cases here
} 