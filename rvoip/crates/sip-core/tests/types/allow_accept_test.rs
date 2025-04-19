// Tests for Allow and Accept types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Allow, Accept, Method, MediaType, ContentType};
use std::str::FromStr;
use std::collections::HashMap;

#[test]
fn test_allow_display_parse_roundtrip() {
    let allow1 = Allow(vec![Method::Invite, Method::Ack, Method::Options]);
    assert_display_parses_back(&allow1);
    
    assert_parse_fails::<Allow>("");
    assert_parse_fails::<Allow>("INVITE, BAD");
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

    // Test FromStr directly
    assert_parses_ok(
        "application/sdp, application/json;level=1", 
        accept1 // Use the same constructed object
    );
    
    assert_parse_fails::<Accept>("");
    assert_parse_fails::<Accept>("application/sdp,"); // Trailing comma
    assert_parse_fails::<Accept>("badtype");
} 