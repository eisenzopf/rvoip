use super::*;
use crate::types::{
    Method,
    StatusCode,
    max_forwards::MaxForwards,
    content_type::ContentType,
    content_length::ContentLength,
};

#[test]
fn test_simple_request_builder() {
    let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .build();
        
    assert_eq!(request.method, Method::Invite);
    assert_eq!(request.uri.to_string(), "sip:bob@example.com");
    
    // Check From header
    let from = request.from().unwrap();
    assert_eq!(from.address().display_name(), Some("Alice"));
    assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
    assert_eq!(from.tag(), Some("1928301774"));
    
    // Check To header
    let to = request.to().unwrap();
    assert_eq!(to.address().display_name(), Some("Bob"));
    assert_eq!(to.address().uri.to_string(), "sip:bob@example.com");
    assert_eq!(to.tag(), None);
    
    // Check Call-ID header
    let call_id = request.call_id().unwrap();
    assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.com");
    
    // Check CSeq header
    let cseq = request.cseq().unwrap();
    assert_eq!(cseq.sequence(), 314159);
    assert_eq!(*cseq.method(), Method::Invite);
    
    // Check Via header
    let via = request.first_via().unwrap();
    assert_eq!(via.0[0].sent_protocol.transport, "UDP");
    assert_eq!(via.0[0].sent_by_host.to_string(), "pc33.atlanta.com");
    assert!(via.branch().is_some());
    assert_eq!(via.branch().unwrap(), "z9hG4bK776asdhds");
    
    // Check Max-Forwards header
    let max_forwards = request.typed_header::<MaxForwards>().unwrap();
    assert_eq!(max_forwards.0, 70);
}

#[test]
fn test_simple_response_builder() {
    let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(1, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
        
    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.reason, Some("OK".to_string()));
    
    // Check From header
    let from = response.from().unwrap();
    assert_eq!(from.address().display_name(), Some("Alice"));
    assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
    assert_eq!(from.tag(), Some("1928301774"));
    
    // Check To header
    let to = response.to().unwrap();
    assert_eq!(to.address().display_name(), Some("Bob"));
    assert_eq!(to.address().uri.to_string(), "sip:bob@example.com");
    assert_eq!(to.tag(), Some("a6c85cf"));
    
    // Check Call-ID header
    let call_id = response.call_id().unwrap();
    assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.com");
    
    // Check CSeq header
    let cseq = response.cseq().unwrap();
    assert_eq!(cseq.sequence(), 1);
    assert_eq!(*cseq.method(), Method::Invite);
    
    // Check Via header
    let via = response.first_via().unwrap();
    assert_eq!(via.0[0].sent_protocol.transport, "UDP");
    assert_eq!(via.0[0].sent_by_host.to_string(), "pc33.atlanta.com");
    assert!(via.branch().is_some());
    assert_eq!(via.branch().unwrap(), "z9hG4bK776asdhds");
}

#[test]
fn test_with_body_and_content_type() {
    let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .content_type("application/sdp")
        .body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n")
        .build();
        
    // Check Content-Type header
    let content_type = request.typed_header::<ContentType>().unwrap();
    assert_eq!(content_type.to_string(), "application/sdp");
    
    // Check body
    assert_eq!(
        String::from_utf8_lossy(&request.body),
        "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
    );
    
    // Check Content-Length header
    let content_length = request.typed_header::<ContentLength>().unwrap();
    assert_eq!(content_length.0 as usize, request.body.len());
}

#[test]
fn test_uri_parsing_error_handling() {
    // Test with invalid URI
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
        .from("Alice", "invalid-uri", Some("1928301774"))
        .to("Bob", "another-invalid-uri", None)
        .build();
        
    // The builder should still create headers with best effort parsing
    assert!(request.from().is_some());
    assert!(request.to().is_some());
} 