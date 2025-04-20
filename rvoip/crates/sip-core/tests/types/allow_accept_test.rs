// Tests for Allow and Accept types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Allow, Accept, Method, MediaType, ContentType};
use std::str::FromStr;
use std::collections::HashMap;

#[test]
fn test_allow_display_parse_roundtrip() {
    let allow1 = Allow(vec![Method::Invite, Method::Ack, Method::Options]);
    assert_display_parses_back(&allow1);
    
    let allow2 = Allow(vec![Method::Register]);
    assert_display_parses_back(&allow2);

    // Test with an extension method
    let allow3 = Allow(vec![Method::Invite, Method::Extension("CUSTOM".to_string())]);
    assert_display_parses_back(&allow3);
    
    // Test FromStr directly for edge cases
    assert_parses_ok(
        "INVITE, ACK, OPTIONS, CANCEL, BYE", 
        Allow(vec![Method::Invite, Method::Ack, Method::Options, Method::Cancel, Method::Bye])
    );
    // Parsing should succeed even with unknown methods
     assert_parses_ok("INVITE, BAD", Allow(vec![Method::Invite, Method::Extension("BAD".to_string())])); 
     assert_parses_ok(" CUSTOM , INVITE ", Allow(vec![Method::Extension("CUSTOM".to_string()), Method::Invite])); // Check trimming
    
    assert_parse_fails::<Allow>(""); // Empty fails
    assert_parse_fails::<Allow>("INVITE,"); // Trailing comma fails
    // Add test for invalid token if needed: assert_parse_fails::<Allow>("INVITE, BAD METHOD");
}

#[test]
fn test_accept_display_parse_roundtrip() {
    let mut params = HashMap::new();
    params.insert("level".to_string(), "1".to_string());
    let accept1 = Accept(vec![
        MediaType { type_: "application".to_string(), subtype: "sdp".to_string(), params: HashMap::new() },
        MediaType { type_: "application".to_string(), subtype: "json".to_string(), params: params.clone() }
    ]);
    assert_display_parses_back(&accept1);

    let accept2 = Accept(vec![
        MediaType { type_: "text".to_string(), subtype: "html".to_string(), params: HashMap::new() }
    ]);
    assert_display_parses_back(&accept2);

    // Test FromStr directly for edge cases
    assert_parses_ok("application/sdp, application/json;level=1", accept1);
    assert_parses_ok(" text/html ", accept2); // Check trimming

    assert_parse_fails::<Accept>("");
    assert_parse_fails::<Accept>("application/sdp,"); // Trailing comma fails
    assert_parse_fails::<Accept>("badtype");
} 