// Tests for From type
use crate::common::{addr, param_tag, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, From, Param};
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;

// Helper function (copied from address_test.rs)
fn basic_uri(user: &str, domain: &str) -> Uri {
    Uri { scheme: Scheme::Sip, user: Some(user.to_string()), password: None, host: Host::Domain(domain.to_string()), port: None, parameters: vec![], headers: Default::default() }
}

#[test]
fn test_from_display_parse_roundtrip() {
    let addr1 = addr(
        Some("Alice"), 
        "sip:alice@example.com", 
        vec![param_tag("123")]
    );
    let from_header1 = From(addr1);
    assert_display_parses_back(&from_header1);
    
    let addr2 = addr(None, "sip:anonymous@anonymous.invalid", vec![param_tag("456")]);
    let from_header2 = From(addr2);
    assert_display_parses_back(&from_header2);
    
    // Test FromStr directly
    assert_parses_ok(
        "Alice <sip:alice@example.com>;tag=123", 
        From(addr(Some("Alice"), "sip:alice@example.com", vec![param_tag("123")]))
    );
     assert_parses_ok(
        "<sip:anonymous@anonymous.invalid>;tag=456", 
        From(addr(None, "sip:anonymous@anonymous.invalid", vec![param_tag("456")]))
    );
    
    assert_parse_fails::<From>("sip:bob@host"); // Missing tag (usually required)
}

// TODO: Add tests for From-specific helpers if any are added 