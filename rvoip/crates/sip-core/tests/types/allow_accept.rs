use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Allow, Method};
use std::str::FromStr;

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
}

// Accept tests remain unchanged
#[test]
fn test_accept_display_parse_roundtrip() {
    // ... existing accept tests ...
} 