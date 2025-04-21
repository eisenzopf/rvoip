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
    // Simple address contact
    let addr1 = addr(Some("Alice"), "sip:alice@example.com", vec![param_tag("123")]);
    let contact1 = Contact::new_address(addr1.clone()); // Clone addr1 if needed elsewhere
    assert_eq!(contact1.to_string(), "Alice <sip:alice@example.com>;tag=123");
    assert_display_parses_back(&contact1);

    // Address contact with expires and q
    let addr2 = addr(None, "sip:bob@example.com", vec![param_expires(3600), param_q(0.9)]);
    let contact2 = Contact::new_address(addr2.clone()); // Clone addr2
    assert_eq!(contact2.to_string(), "<sip:bob@example.com>;expires=3600;q=0.9");
    assert_display_parses_back(&contact2);

    // Wildcard contact
    let contact_wc = Contact::new_wildcard();
    assert_eq!(contact_wc.to_string(), "*");
    assert_display_parses_back(&contact_wc);
    
    // Contact with quoted display name and other params
     assert_display_parses_back(&Contact::new_address(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        )));
}

#[test]
fn test_contact_from_str() {
    // Parsing valid contacts
    assert_parses_ok("\"Alice\" <sip:alice@example.com>;tag=123", 
                     Contact::new_address(addr(Some("Alice"), "sip:alice@example.com", vec![param_tag("123")])));
    assert_parses_ok("<sip:bob@example.com>", 
                     Contact::new_address(addr(None, "sip:bob@example.com", vec![])));
    assert_parses_ok("sip:carol@chicago.com", 
                     Contact::new_address(addr(None, "sip:carol@chicago.com", vec![])));
    assert_parses_ok("*", Contact::new_wildcard());
    assert_parses_ok("  *  ", Contact::new_wildcard()); // With whitespace

    // Parsing failures
    assert_parse_fails::<Contact>("<");
    assert_parse_fails::<Contact>("Display Name Only");
    assert_parse_fails::<Contact>("\"Bob\" sip:bob@biloxi.com"); // Missing <> for display name
    assert_parse_fails::<Contact>("*;tag=123"); 
}

#[test]
fn test_contact_display() {
    // Display for address contact
    let addr1 = addr(Some("Display"), "sip:user@host", vec![param_tag("abc")]);
    let contact1 = Contact::new_address(addr1);
    assert_eq!(contact1.to_string(), "Display <sip:user@host>;tag=abc");

    // Display for address contact with no display name
    let addr2 = addr(None, "sip:user@host", vec![]);
    let contact2 = Contact::new_address(addr2);
    assert_eq!(contact2.to_string(), "<sip:user@host>");

    // Display for wildcard contact
    let contact_wc = Contact::new_wildcard();
    assert_eq!(contact_wc.to_string(), "*");
    
     // Check quoted display
    assert_eq!(
        Contact::new_address(addr(
            Some("Contact Name"), 
            "sip:contact@host.com", 
            vec![param_expires(60), param_other("instance", Some("abc"))]
        )).to_string(),
        "\"Contact Name\" <sip:contact@host.com>;expires=60;instance=abc"
    );
}

#[test]
fn test_contact_helpers() {
    let addr_no_params = addr(None, "sip:test@test.com", vec![]);
    let mut contact = Contact::new_address(addr_no_params);
    
    assert_eq!(contact.expires(), None);
    assert_eq!(contact.q(), None);
    assert_eq!(contact.tag(), None);
    assert!(!contact.is_wildcard());
    assert!(contact.address().is_some());

    contact.set_expires(3600);
    assert_eq!(contact.expires(), Some(3600));
    assert!(contact.address().unwrap().params.contains(&Param::Expires(3600)));

    contact.set_q(0.7);
    assert_eq!(contact.q(), Some(NotNan::new(0.7).unwrap()));
    assert!(contact.address().unwrap().params.iter().any(|p| matches!(p, Param::Q(v) if (*v - 0.7).abs() < f32::EPSILON)));

    // Test replacement
    contact.set_expires(120);
    assert_eq!(contact.expires(), Some(120));
    assert_eq!(contact.address().unwrap().params.iter().filter(|p| matches!(p, Param::Expires(_))).count(), 1);

    contact.set_q(1.1); // Clamping
    assert_eq!(contact.q(), Some(NotNan::new(1.0).unwrap()));
    assert!(contact.address().unwrap().params.contains(&Param::Q(NotNan::new(1.0).unwrap())));

    contact.set_q(-0.5); // Clamping
    assert_eq!(contact.q(), Some(NotNan::new(0.0).unwrap()));
     assert!(contact.address().unwrap().params.contains(&Param::Q(NotNan::new(0.0).unwrap())));

    contact.set_tag("newtag");
    assert_eq!(contact.tag(), Some("newtag"));
    assert!(contact.address().unwrap().params.contains(&Param::Tag("newtag".to_string())));

    // Test wildcard contact
    let contact_wc = Contact::new_wildcard();
    assert!(contact_wc.is_wildcard());
    assert_eq!(contact_wc.expires(), None);
    assert_eq!(contact_wc.q(), None);
    assert_eq!(contact_wc.tag(), None);
    assert!(contact_wc.address().is_none());
}

#[test]
fn test_contact_asterisk() { 
    let contact = Contact::new_wildcard();
    assert!(contact.is_wildcard());
    assert_eq!(contact.to_string(), "*");

    // Test parsing
    let parsed = Contact::from_str("*").expect("Failed to parse wildcard contact");
    assert!(parsed.is_wildcard());
    assert_eq!(parsed, contact);
}

#[test]
#[should_panic]
fn test_contact_set_expires_on_wildcard() {
    let mut contact = Contact::new_wildcard();
    contact.set_expires(100); // Should panic
}

#[test]
#[should_panic]
fn test_contact_set_q_on_wildcard() {
    let mut contact = Contact::new_wildcard();
    contact.set_q(0.5); // Should panic
}

#[test]
#[should_panic]
fn test_contact_set_tag_on_wildcard() {
    let mut contact = Contact::new_wildcard();
    contact.set_tag("tag"); // Should panic
}

#[test]
fn test_contact_list_parsing() {
    // Test parsing comma-separated contacts 
    // NOTE: This test assumes a future parser function `parse_contact_list` exists.
    // For now, we test FromStr on individual valid parts.
    
    let alice_addr = addr(None, "sip:alice@example.com", vec![param_tag("1")]);
    let bob_addr = addr(Some("Bob"), "sips:bob@example.com", vec![param_q(0.5)]);
    
    let contact1_str = "<sip:alice@example.com>;tag=1";
    let contact2_str = "\"Bob\" <sips:bob@example.com>;q=0.5";

    let alice_contact = Contact::from_str(contact1_str).unwrap();
    let bob_contact = Contact::from_str(contact2_str).unwrap();
    
    assert_eq!(alice_contact, Contact::new_address(alice_addr));
    assert_eq!(bob_contact, Contact::new_address(bob_addr));

    // Test parsing includes wildcard (as a single item)
    let wc_contact = Contact::from_str("*").unwrap();
    assert!(wc_contact.is_wildcard());

    // Example of how list parsing *could* be tested later:
    // let input_wc = "*, <sip:alice@example.com>;tag=1"; 
    // let contacts_wc = parse_contact_list(input_wc).unwrap(); // Assumes parse_contact_list exists
    // assert_eq!(contacts_wc.len(), 2);
    // assert!(contacts_wc[0].is_wildcard());
    // assert_eq!(contacts_wc[1], alice_contact);
}

// TODO: Add tests for Contact-specific helpers (expires, q) 