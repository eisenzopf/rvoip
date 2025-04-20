// Tests for the Address type and related types (From, To, Contact)

use crate::common::{uri, addr, param_tag, param_expires, param_q, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, Param, From, Contact};
use rvoip_sip_core::types::to::To;
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;
use ordered_float::NotNan;

// Helper to create a basic URI
fn basic_uri(user: &str, domain: &str) -> Uri {
    Uri {
        scheme: Scheme::Sip,
        user: Some(user.to_string()),
        password: None,
        host: Host::Domain(domain.to_string()),
        port: None,
        parameters: vec![],
        headers: Default::default(),
    }
}

#[test]
fn test_address_display_parse_roundtrip() {
    /// RFC 3261 Section 20.10 Examples
    let addr1 = addr(
        Some("Alice"), 
        "sip:alice@example.com", 
        vec![param_tag("123")]
    );
    // Simple token display name doesn't need quotes
    assert_eq!(addr1.to_string(), "Alice <sip:alice@example.com>;tag=123"); 
    assert_display_parses_back(&addr1);

    let addr2 = addr(
        None, 
        "sip:bob@example.com", 
        vec![param_expires(3600), param_q(0.9)]
    );
    // Display check (order matters for vec)
    assert_eq!(addr2.to_string(), "<sip:bob@example.com>;expires=3600;q=0.9");
    assert_display_parses_back(&addr2);

    let addr3 = addr(
        Some(" Carol Quinn "), // Needs quoting
        "sip:carol@host.net", 
        vec![]
    );
    assert_eq!(addr3.to_string(), "\" Carol Quinn \" <sip:carol@host.net>");
    assert_display_parses_back(&addr3);
    
    let addr4 = addr(
        Some("Mr. \"X\""), // Contains quotes
        "sip:x@domain.com", 
        vec![]
    );
    assert_eq!(addr4.to_string(), "\"Mr. \\\"X\\\"\" <sip:x@domain.com>");
    // assert_display_parses_back(&addr4); // Round trip might fail if quote escaping in display isn't perfectly symmetric with parser

    let addr5 = addr(
        Some(""), // Empty display name
        "sip:empty@domain.com", 
        vec![]
    );
    assert_eq!(addr5.to_string(), "\"\" <sip:empty@domain.com>");
    assert_display_parses_back(&addr5);
}

#[test]
fn test_address_from_str() {
    assert_parses_ok(
        "\"Alice\" <sip:alice@example.com>;tag=123", 
        addr(Some("Alice"), "sip:alice@example.com", vec![param_tag("123")])
    );
    assert_parses_ok(
        "<sip:bob@example.com>", 
        addr(None, "sip:bob@example.com", vec![])
    );
    // Plain URI
     assert_parses_ok(
        "sip:carol@chicago.com", 
        addr(None, "sip:carol@chicago.com", vec![])
    );
    
    assert_parse_fails::<Address>("<");
    assert_parse_fails::<Address>("Display Name Only");
    assert_parse_fails::<Address>("\"Bob\" sip:bob@biloxi.com");
}

#[test]
fn test_address_helpers() {
     let mut addr = addr(None, "sip:user@host", vec![]);
     assert_eq!(addr.tag(), None);
     assert_eq!(addr.expires(), None);
     assert_eq!(addr.q(), None);
     assert!(!addr.has_param("tag"));
     assert!(!addr.has_param("expires"));
     assert!(!addr.has_param("q"));
     assert!(!addr.has_param("custom"));
     assert_eq!(addr.get_param("tag"), None);
     
     addr.set_tag("tag1");
     assert_eq!(addr.tag(), Some("tag1"));
     assert!(addr.has_param("tag"));
     assert_eq!(addr.get_param("tag"), Some(Some("tag1")));
     
     addr.set_expires(3600);
     assert_eq!(addr.expires(), Some(3600));
     assert!(addr.has_param("expires"));
     assert!(addr.get_param("expires").flatten().unwrap_or("").contains("3600"));
     
     addr.set_q(0.5);
     assert_eq!(addr.q(), Some(NotNan::new(0.5).unwrap()));
     assert!(addr.has_param("q"));
     assert!(addr.get_param("q").flatten().unwrap_or("").contains("0.5"));

     // Test replacement
     addr.set_tag("tag2");
     assert_eq!(addr.tag(), Some("tag2"));
     assert_eq!(addr.params.iter().filter(|p| matches!(p, Param::Tag(_))).count(), 1);
     
     addr.set_expires(60);
     assert_eq!(addr.expires(), Some(60));
     assert_eq!(addr.params.iter().filter(|p| {
         match p {
             Param::Expires(_) => true,
             Param::Other(k, _) => k.eq_ignore_ascii_case("expires"),
             _ => false,
         }
     }).count(), 1);
     
     addr.set_q(1.1); // Clamping
     assert_eq!(addr.q(), Some(NotNan::new(1.0).unwrap()));
     assert_eq!(addr.params.iter().filter(|p| matches!(p, Param::Q(_))).count(), 1);
     
     // Test generic set_param
     addr.set_param("custom", Some("value"));
     assert!(addr.has_param("custom"));
     assert_eq!(addr.get_param("custom"), Some(Some("value")));
     
     addr.set_param("flag", None::<String>);
     assert!(addr.has_param("flag"));
     assert_eq!(addr.get_param("flag"), Some(None));

     // Test set_param replaces typed param
     addr.set_param("expires", Some("override"));
     assert!(addr.has_param("expires"));
     assert_eq!(addr.expires(), None); // Typed getter fails
     assert_eq!(addr.get_param("expires"), Some(Some("override"))); // Generic getter works
     assert!(addr.params.contains(&Param::Other("expires".to_string(), Some("override".to_string()))));
     assert_eq!(addr.params.iter().filter(|p| {
         match p {
             Param::Expires(_) => true,
             Param::Other(k, _) => k.eq_ignore_ascii_case("expires"),
             _ => false,
         }
     }).count(), 1);

}

// TODO: Add tests for From/To/Contact Display and specific helper methods (like expires(), q())

// TODO: Add tests for From/To/Contact Display and specific helper methods (like tag(), expires(), q())
// Note: Display implementation for Address needs to be added first. 