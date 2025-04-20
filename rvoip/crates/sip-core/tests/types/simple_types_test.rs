// Tests for simple wrapper types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{ContentLength, Expires, MaxForwards, CallId};
use std::str::FromStr;

#[test]
fn test_content_length_display_parse_roundtrip() {
    let cl = ContentLength(123);
    assert_display_parses_back(&cl);
    assert_parses_ok(" 123 ", cl); // Test FromStr with trimming
    assert_parse_fails::<ContentLength>("bad");
    assert_parse_fails::<ContentLength>("-10");
}

#[test]
fn test_expires_display_parse_roundtrip() {
    let exp = Expires(3600);
    assert_display_parses_back(&exp);
    assert_parses_ok(" 3600\t", exp); // Test FromStr with trimming
    assert_parse_fails::<Expires>("never");
    assert_parse_fails::<Expires>("-1");
}

#[test]
fn test_max_forwards_display_parse_roundtrip() {
    let mf = MaxForwards(70);
    assert_display_parses_back(&mf);
    assert_parses_ok(" 70 ", mf); // Test FromStr with trimming
    assert_parse_fails::<MaxForwards>("256");
    assert_parse_fails::<MaxForwards>("-5");
}

#[test]
fn test_call_id_display_parse_roundtrip() {
    let call_id_str = "abc-123@example.com";
    let cid = CallId(call_id_str.to_string());
    assert_display_parses_back(&cid);
    assert_parses_ok("  spaced id  ", CallId("spaced id".to_string())); 
}

#[test]
fn test_call_id() {
    let s = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
    let cid = CallId::from_str(s).unwrap();
    assert_eq!(cid.to_string(), s);
    assert_display_parses_back(&cid);

    /*
    let cid1 = CallId::new_random();
    let cid2 = CallId::new_random();
    assert_ne!(cid1.0, cid2.0);
    */
} 