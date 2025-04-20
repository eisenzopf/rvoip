// Tests for Contact type
use crate::common::{addr, param_expires, param_q, param_other, param_tag, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, Contact, Param};
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;
use ordered_float::NotNan;

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
    
    // Test wildcard
    let wc_addr = Address::new(None::<String>, Uri::from_str("*").unwrap());
    let contact_wc = Contact::new(wc_addr);
    assert_eq!(contact_wc.to_string(), "*");
    assert_display_parses_back(&contact_wc);
    
    // Test FromStr directly
    assert_parses_ok(
        "\"Contact Name\" <sip:contact@host.com>;expires=60;instance=abc", 
        Contact(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        ))
    );
    
    assert_parses_ok("*", contact_wc);
    
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
    
    // Test wildcard
    let wc_addr = Address::new(None::<String>, Uri::from_str("*").unwrap());
    let contact_wc = Contact::new(wc_addr);
    assert_eq!(contact_wc.to_string(), "*");
    assert_display_parses_back(&contact_wc);
    
    // Test FromStr directly
    assert_parses_ok(
        "\"Contact Name\" <sip:contact@host.com>;expires=60;instance=abc", 
        Contact(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        ))
    );
    
    assert_parses_ok("*", contact_wc);
    
    assert_parse_fails::<Contact>(""); // Empty fails Address parsing
}

#[test]
fn test_contact_helpers() {
    let addr_no_params = addr(None, "sip:user@host", vec![]);
    let mut contact = Contact(addr_no_params);
    
    assert_eq!(contact.expires(), None);
    assert_eq!(contact.q(), None);
    assert_eq!(contact.tag(), None);
    assert!(!contact.is_wildcard());
    
    contact.set_expires(3600);
    assert_eq!(contact.expires(), Some(3600));
    assert!(contact.0.params.contains(&Param::Expires(3600)));
    
    contact.set_q(0.7);
    assert_eq!(contact.q(), Some(NotNan::new(0.7).unwrap()));
    assert!(contact.0.params.iter().any(|p| matches!(p, Param::Q(v) if (*v - 0.7).abs() < f32::EPSILON)));
    
    // Test replacement
    contact.set_expires(60);
    assert_eq!(contact.expires(), Some(60));
    assert_eq!(contact.0.params.iter().filter(|p| matches!(p, Param::Expires(_))).count(), 1);

    contact.set_q(1.1); // Test clamping
    assert_eq!(contact.q(), Some(NotNan::new(1.0).unwrap()));
    assert!(contact.0.params.contains(&Param::Q(NotNan::new(1.0).unwrap())));
    
    contact.set_q(-0.5); // Test clamping
    assert_eq!(contact.q(), Some(NotNan::new(0.0).unwrap()));
    assert!(contact.0.params.contains(&Param::Q(NotNan::new(0.0).unwrap())));

    // Test delegated methods
    contact.set_tag("newtag");
    assert_eq!(contact.tag(), Some("newtag"));
    assert!(contact.0.params.contains(&Param::Tag("newtag".to_string())));
    
    // Test wildcard check
    let wc_addr = Address::new(None::<String>, Uri::from_str("*").unwrap());
    let contact_wc = Contact::new(wc_addr);
    assert!(contact_wc.is_wildcard());

}

#[test]
fn test_contact_asterisk() {
    let wc_str = "*";
    let contact_wc = Contact::from_str(wc_str).unwrap();

    // Construct the expected Address part for '*' Contact
    let wc_addr = Address::new(None::<String>, Uri::from_str("*").unwrap());
    let expected_wc = Contact(wc_addr);

    assert_eq!(contact_wc, expected_wc);
}

#[test]
fn test_contact_list_parsing() {
    let input = "\"Alice Liddell\" <sip:alice@wonderland.lit>;tag=asdf, <sip:bob@biloxi.com>;q=0.5, *;expires=600";

    // Build expected results
    let alice_uri = Uri::from_str("sip:alice@wonderland.lit").unwrap();
    let mut alice_addr = Address::new(Some("Alice Liddell"), alice_uri);
    alice_addr.set_param("tag", Some("asdf"));
    let alice_contact = Contact(alice_addr);

    let bob_uri = Uri::from_str("sip:bob@biloxi.com").unwrap();
    let mut bob_addr = Address::new(None::<String>, bob_uri);
    bob_addr.set_q(0.5);
    let bob_contact = Contact(bob_addr);

    let mut wc_addr = Address::new(None::<String>, Uri::from_str("*").unwrap());
    wc_addr.set_expires(600);
    let wc_contact = Contact(wc_addr);

    // Parse the input and compare with expected results
    let parsed_contacts: Vec<Contact> = input.split(',').map(|s| s.trim().parse().unwrap()).collect();
    assert_eq!(parsed_contacts, vec![alice_contact, bob_contact, wc_contact]);
}

// TODO: Add tests for Contact-specific helpers (expires, q) 