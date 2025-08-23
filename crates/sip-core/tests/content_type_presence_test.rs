//! Tests for presence-related ContentType functionality

use rvoip_sip_core::types::content_type::ContentType;
use std::str::FromStr;

#[test]
fn test_pidf_content_type() {
    // Create using helper method
    let pidf = ContentType::pidf();
    assert_eq!(pidf.to_string(), "application/pidf+xml");
    assert!(pidf.is_pidf());
    assert!(!pidf.is_sdp());
    
    // Parse from string
    let parsed = ContentType::from_str("application/pidf+xml").unwrap();
    assert!(parsed.is_pidf());
    assert_eq!(parsed.to_string(), "application/pidf+xml");
    
    // With charset parameter
    let with_charset = ContentType::from_str("application/pidf+xml;charset=UTF-8").unwrap();
    assert!(with_charset.is_pidf());
}

#[test]
fn test_sdp_content_type() {
    let sdp = ContentType::sdp();
    assert_eq!(sdp.to_string(), "application/sdp");
    assert!(sdp.is_sdp());
    assert!(!sdp.is_pidf());
}

#[test]
fn test_message_summary_content_type() {
    let mwi = ContentType::message_summary();
    assert_eq!(mwi.to_string(), "application/simple-message-summary");
}

#[test]
fn test_text_plain_content_type() {
    let text = ContentType::text_plain();
    assert_eq!(text.to_string(), "text/plain");
}

#[test]
fn test_add_parameter() {
    let mut pidf = ContentType::pidf();
    pidf.add_parameter("charset", "UTF-8");
    assert_eq!(pidf.to_string(), "application/pidf+xml;charset=\"UTF-8\"");
    
    let mut text = ContentType::text_plain();
    text.add_parameter("charset", "ISO-8859-1");
    assert_eq!(text.to_string(), "text/plain;charset=\"ISO-8859-1\"");
}

#[test]
fn test_content_type_in_builder() {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::Method;
    
    // NOTIFY with PIDF body
    let notify = SimpleRequestBuilder::notify(
        "sip:alice@192.168.1.10:5060",
        "presence",
        "active;expires=3599"
    )
    .unwrap()
    .from("Bob", "sip:bob@example.com", Some("xyz987"))
    .to("Alice", "sip:alice@example.com", Some("1928301774"))
    .call_id("test-call-id")
    .cseq(1)
    .via("192.168.1.20:5060", "UDP", Some("z9hG4bK776branch"))
    .content_type("application/pidf+xml")
    .body("<presence entity=\"pres:bob@example.com\">...</presence>")
    .build();
    
    let notify_str = notify.to_string();
    assert!(notify_str.contains("Content-Type: application/pidf+xml"));
    
    // PUBLISH with PIDF body
    let publish = SimpleRequestBuilder::publish("sip:alice@example.com", "presence")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(1)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .content_type("application/pidf+xml")
        .body("<presence entity=\"pres:alice@example.com\">...</presence>")
        .build();
    
    let publish_str = publish.to_string();
    assert!(publish_str.contains("Content-Type: application/pidf+xml"));
}