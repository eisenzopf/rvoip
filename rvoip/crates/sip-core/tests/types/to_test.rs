// Tests for To type
use crate::common::{addr, param_tag, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, To, Param};
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;

// Helper function 
fn basic_uri(user: &str, domain: &str) -> Uri {
    Uri { scheme: Scheme::Sip, user: Some(user.to_string()), password: None, host: Host::Domain(domain.to_string()), port: None, parameters: vec![], headers: Default::default() }
}

#[test]
fn test_to_display_parse_roundtrip() {
    let addr1 = addr(
        None,
        "sip:bob@example.com",
        vec![param_tag("456")]
    );
    let to_header1 = To(addr1);
    assert_display_parses_back(&to_header1);

    let addr2 = addr(Some("Receiver"), "sip:recv@host", vec![]);
    let to_header2 = To(addr2);
    assert_display_parses_back(&to_header2);

    // Test FromStr directly
    assert_parses_ok(
        "<sip:bob@example.com>;tag=456", 
        To(addr(None, "sip:bob@example.com", vec![param_tag("456")]))
    );
    assert_parses_ok(
        "Receiver <sip:recv@host>", 
        To(addr(Some("Receiver"), "sip:recv@host", vec![]))
    );

    // To header doesn't strictly require a tag on incoming requests
    assert_parses_ok(
        "<sip:bob@example.com>", 
        To(addr(None, "sip:bob@example.com", vec![]))
    );

    assert_parse_fails::<To>("invalid");
}

// TODO: Add tests for To-specific helpers 