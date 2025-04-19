// Tests for MediaType and ContentType types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{MediaType, ContentType};
use std::str::FromStr;
use std::collections::HashMap;

#[test]
fn test_media_type_display_parse_roundtrip() {
    let mut params1 = HashMap::new();
    params1.insert("charset".to_string(), "utf-8".to_string());
    let mt1 = MediaType {
        type_: "application".to_string(),
        subtype: "sdp".to_string(),
        params: params1.clone(),
    };
    assert_display_parses_back(&mt1);

    let mt2 = MediaType {
        type_: "text".to_string(),
        subtype: "html".to_string(),
        params: HashMap::new(),
    };
    assert_display_parses_back(&mt2);
    
    // Test FromStr directly
    assert_parses_ok("application/sdp;charset=utf-8", mt1);
    assert_parses_ok(" text/html ", mt2); // Trimming handled by parser
    
    assert_parse_fails::<MediaType>("application"); // Missing subtype
    assert_parse_fails::<MediaType>("application/");
    assert_parse_fails::<MediaType>("/sdp");
}

#[test]
fn test_content_type_display_parse_roundtrip() {
    let mt = MediaType {
        type_: "application".to_string(),
        subtype: "sdp".to_string(),
        params: HashMap::new(),
    };
    let ct = ContentType(mt.clone());
    assert_display_parses_back(&ct);

    // Test FromStr directly
     assert_parses_ok("application/sdp", ct);
} 