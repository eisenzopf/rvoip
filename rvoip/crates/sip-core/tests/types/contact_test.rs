// Tests for Contact type
use crate::common::{addr, param_expires, param_q, param_other, param_tag, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, Contact, Param};
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;

// Helper function 
fn basic_uri(user: &str, domain: &str) -> Uri {
    Uri { scheme: Scheme::Sip, user: Some(user.to_string()), password: None, host: Host::Domain(domain.to_string()), port: None, parameters: vec![], headers: Default::default() }
}

#[test]
fn test_contact_display_parse_roundtrip() {
    let addr1 = addr(
        Some("Contact Name"), // Needs quoting
        "sip:contact@host.com", 
        vec![param_expires(60), param_other("instance", Some("abc"))]
    );
    let contact1 = Contact(addr1);
    // Check display manually due to potential param order variations if Address used HashMap
    let disp = contact1.to_string();
    assert!(disp.starts_with("\"Contact Name\" <sip:contact@host.com>"));
    assert!(disp.contains(";expires=60"));
    assert!(disp.contains(";instance=abc"));
    assert_display_parses_back(&contact1);

    let addr2 = addr(None, "sip:contact2@host.com", vec![param_q(0.5)]);
    let contact2 = Contact(addr2);
    assert_display_parses_back(&contact2);
    
    // Test FromStr directly
    assert_parses_ok(
        "\"Contact Name\" <sip:contact@host.com>;expires=60;instance=abc", 
        Contact(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        ))
    );
    
    assert_parse_fails::<Contact>(""); // Empty fails Address parsing
}

#[test]
fn test_contact_display() {
    let addr1 = addr(
        Some("Contact Name"), // Needs quoting
        "sip:contact@host.com", 
        vec![param_expires(60), param_other("instance", Some("abc"))]
    );
    let contact1 = Contact(addr1);
    // Check display manually due to potential param order variations if Address used HashMap
    let disp = contact1.to_string();
    assert!(disp.starts_with("\"Contact Name\" <sip:contact@host.com>"));
    assert!(disp.contains(";expires=60"));
    assert!(disp.contains(";instance=abc"));
    assert_display_parses_back(&contact1);

    let addr2 = addr(None, "sip:contact2@host.com", vec![param_q(0.5)]);
    let contact2 = Contact(addr2);
    assert_display_parses_back(&contact2);
    
    // Test FromStr directly
    assert_parses_ok(
        "\"Contact Name\" <sip:contact@host.com>;expires=60;instance=abc", 
        Contact(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        ))
    );
    
    assert_parse_fails::<Contact>(""); // Empty fails Address parsing
}

#[test]
fn test_contact_helpers() {
    let addr_no_params = addr(None, "sip:user@host", vec![]);
    let mut contact = Contact(addr_no_params);
    
    assert_eq!(contact.expires(), None);
    assert_eq!(contact.q(), None);
    assert_eq!(contact.tag(), None);
    
    contact.set_expires(3600);
    assert_eq!(contact.expires(), Some(3600));
    assert!(contact.0.params.contains(&Param::Expires(3600)));
    
    contact.set_q(0.7);
    assert_eq!(contact.q(), Some(0.7));
    assert!(contact.0.params.iter().any(|p| matches!(p, Param::Q(v) if (*v - 0.7).abs() < f32::EPSILON)));
    
    // Test replacement
    contact.set_expires(60);
    assert_eq!(contact.expires(), Some(60));
    assert_eq!(contact.0.params.iter().filter(|p| matches!(p, Param::Expires(_))).count(), 1);

    contact.set_q(1.1); // Test clamping
    assert_eq!(contact.q(), Some(1.0));
    assert!(contact.0.params.contains(&Param::Q(1.0)));
    
    contact.set_q(-0.5); // Test clamping
    assert_eq!(contact.q(), Some(0.0));
    assert!(contact.0.params.contains(&Param::Q(0.0)));

    // Test delegated methods
    contact.set_tag("newtag");
    assert_eq!(contact.tag(), Some("newtag"));
    assert!(contact.0.params.contains(&Param::Tag("newtag".to_string())));

}

// TODO: Add tests for Contact-specific helpers (expires, q) 