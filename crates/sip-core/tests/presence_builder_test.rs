//! Integration tests for presence-related builder methods

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{Method, StatusCode};
use rvoip_sip_core::builder::SimpleResponseBuilder;

#[test]
fn test_publish_builder() {
    // Initial PUBLISH
    let request = SimpleRequestBuilder::publish("sip:alice@example.com", "presence")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(1)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .expires(3600)
        .build();
    
    assert_eq!(request.method, Method::Publish);
    let request_str = request.to_string();
    assert!(request_str.contains("PUBLISH"));
    assert!(request_str.contains("Event: presence"));
    assert!(request_str.contains("Expires: 3600"));
    
    // Refresh PUBLISH with SIP-If-Match
    let refresh = SimpleRequestBuilder::publish("sip:alice@example.com", "presence")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(2)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .sip_if_match("abc123xyz")
        .expires(3600)
        .build();
    
    let refresh_str = refresh.to_string();
    assert!(refresh_str.contains("SIP-If-Match: abc123xyz"));
}

#[test]
fn test_subscribe_builder() {
    // Initial SUBSCRIBE
    let request = SimpleRequestBuilder::subscribe("sip:bob@example.com", "presence", 3600)
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:alice@192.168.1.10:5060", None)
        .build();
    
    assert_eq!(request.method, Method::Subscribe);
    let request_str = request.to_string();
    assert!(request_str.contains("SUBSCRIBE"));
    assert!(request_str.contains("Event: presence"));
    assert!(request_str.contains("Expires: 3600"));
    
    // Unsubscribe
    let unsub = SimpleRequestBuilder::subscribe("sip:bob@example.com", "presence", 0)
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("xyz987"))
        .call_id("test-call-id")
        .cseq(2)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    let unsub_str = unsub.to_string();
    assert!(unsub_str.contains("Expires: 0"));
}

#[test]
fn test_notify_builder() {
    // Active NOTIFY
    let request = SimpleRequestBuilder::notify(
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
    .contact("sip:bob@192.168.1.20:5060", None)
    .build();
    
    assert_eq!(request.method, Method::Notify);
    let request_str = request.to_string();
    println!("NOTIFY request:\n{}", request_str);
    assert!(request_str.contains("NOTIFY"));
    assert!(request_str.contains("Event: presence"));
    // The header value is stored as-is, no extra formatting
    assert!(request_str.contains("Subscription-State: active;expires=3599"));
    
    // Terminated NOTIFY
    let terminated = SimpleRequestBuilder::notify(
        "sip:alice@192.168.1.10:5060",
        "presence",
        "terminated;reason=timeout"
    )
    .unwrap()
    .from("Bob", "sip:bob@example.com", Some("xyz987"))
    .to("Alice", "sip:alice@example.com", Some("1928301774"))
    .call_id("test-call-id")
    .cseq(2)
    .via("192.168.1.20:5060", "UDP", Some("z9hG4bK776branch"))
    .build();
    
    let terminated_str = terminated.to_string();
    assert!(terminated_str.contains("Subscription-State: terminated;reason=timeout"));
}

#[test]
fn test_response_builder_presence_headers() {
    // PUBLISH response with SIP-ETag
    let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Publish)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .sip_etag("abc123xyz")
        .expires(3600)
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("SIP-ETag: abc123xyz"));
    assert!(response_str.contains("Expires: 3600"));
    
    // OPTIONS response with Allow-Events
    let options_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
        .from("Bob", "sip:bob@example.com", None)
        .to("Alice", "sip:alice@example.com", Some("1928301774"))
        .call_id("test-call-id")
        .cseq(1, Method::Options)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .allow_events(&["presence", "dialog", "message-summary"])
        .build();
    
    let options_str = options_response.to_string();
    assert!(options_str.contains("Allow-Events: presence, dialog, message-summary"));
    
    // 423 Interval Too Brief with Min-Expires
    let error_response = SimpleResponseBuilder::new(StatusCode::IntervalTooBrief, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .min_expires(3600)
        .build();
    
    let error_str = error_response.to_string();
    assert!(error_str.contains("423 Interval Too Brief"));
    assert!(error_str.contains("Min-Expires: 3600"));
}

#[test]
fn test_pidf_body_integration() {
    use rvoip_sip_core::types::pidf::{PidfDocument, Tuple, Status};
    
    // Create a PIDF document
    let pidf = PidfDocument::new("pres:alice@example.com")
        .add_tuple(
            Tuple::new("t1", Status::open())
                .with_contact("sip:alice@192.168.1.10")
        )
        .add_note("Available for calls");
    
    let pidf_xml = pidf.to_xml();
    
    // Use it in a NOTIFY
    let notify = SimpleRequestBuilder::notify(
        "sip:bob@192.168.1.20:5060",
        "presence",
        "active;expires=3599"
    )
    .unwrap()
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", Some("xyz987"))
    .call_id("test-call-id")
    .cseq(1)
    .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776branch"))
    .content_type("application/pidf+xml")
    .body(pidf_xml.clone())
    .build();
    
    let notify_str = notify.to_string();
    assert!(notify_str.contains("Content-Type: application/pidf+xml"));
    assert!(notify_str.contains("<presence"));
    assert!(notify_str.contains("<basic>open</basic>"));
}