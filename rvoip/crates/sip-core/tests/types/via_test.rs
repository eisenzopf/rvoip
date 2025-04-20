// Tests for the Via type

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back, param_branch, param_ttl, param_received, param_other, param_lr};
use rvoip_sip_core::types::{Via, Param};
use std::str::FromStr;
use std::net::IpAddr;
use rvoip_sip_core::SipError;

/*
#[test]
fn test_via_display_parse_roundtrip() {
    /// RFC 3261 Section 20.42 Via Header Field
    let mut via1 = Via::new("SIP", "2.0", "UDP", "pc33.example.com", Some(5060));
    via1.params.push(param_branch("z9hG4bK776asdhds"));
    via1.params.push(param_other("rport", None));
    via1.params.push(param_ttl(64));
    assert_display_parses_back(&via1);

    let mut via2 = Via::new("SIP", "2.0", "TCP", "client.biloxi.com", None);
    via2.set_branch("z9hG4bKnashds7"); // Test set_branch
    assert_display_parses_back(&via2);

    // Test FromStr directly using helpers
    let via_str = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds;ttl=64;rport";
    assert_parses_ok(via_str, via1); // via1 already has the expected structure
    
    assert_parse_fails::<Via>("SIP/2.0/UDP"); // Missing host
    assert_parse_fails::<Via>("SIP/BAD/UDP host.com"); // Invalid version
}
*/

#[test]
fn test_via_helpers() {
    let mut via = Via::new("SIP", "2.0", "UDP", "host", Some(1234));
    assert!(via.branch().is_none());
    assert!(via.received().is_none());
    assert!(via.maddr().is_none());
    assert!(via.ttl().is_none());
    assert!(!via.rport());
    assert!(!via.contains("received"));
    
    via.set_branch("branch1");
    assert_eq!(via.branch(), Some("branch1"));

    let ip = IpAddr::from_str("1.1.1.1").unwrap();
    via.set_received(ip);
    assert_eq!(via.received(), Some(ip));
    assert!(via.contains("received"));

    via.set_maddr("2.2.2.2");
    assert_eq!(via.maddr(), Some("2.2.2.2"));

    via.set_ttl(64);
    assert_eq!(via.ttl(), Some(64));

    via.set_rport(true);
    assert!(via.rport());
    assert!(via.contains("rport"));

    // Test replacement/removal
    via.set_received(IpAddr::from_str("3.3.3.3").unwrap());
    assert_eq!(via.received(), Some(IpAddr::from_str("3.3.3.3").unwrap()));
    assert_eq!(via.params.iter().filter(|p| matches!(p, Param::Received(_))).count(), 1);

    via.set_rport(false);
    assert!(!via.rport());
    assert!(!via.contains("rport"));

    // Test get() for these specific types (might be less precise due to get() current impl)
    assert!(via.get("received").is_some());
    assert!(via.get("maddr").is_some());
    assert!(via.get("ttl").is_some());
    assert_eq!(via.get("rport"), None); // Flag removed by set(false)
}

/*
#[test]
fn test_via_parsing_logic() {
    // ... existing test code ...
}
*/

// Removed old separate tests 