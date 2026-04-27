use bytes::Bytes;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_core::types::rack::RAck;
use rvoip_sip_core::types::{ContentLength, HeaderName, Method, StatusCode, TypedHeader};
use rvoip_sip_core::validation::{validate_generated_request, validate_generated_response};

fn base_request(method: Method) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(method, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", Some("bob-tag"))
        .call_id("call-1")
        .cseq(1)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-generated"))
        .max_forwards(70)
        .contact("sip:alice@127.0.0.1:5060", None)
}

fn invite_request() -> rvoip_sip_core::Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("invite-call-1")
        .cseq(42)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-invite"))
        .max_forwards(70)
        .contact("sip:alice@127.0.0.1:5060", None)
        .content_type("application/sdp")
        .body("v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=-\r\n")
        .build()
}

#[test]
fn generated_message_compliance_request_method_matrix_roundtrips() {
    let cases = vec![
        SimpleRequestBuilder::new(Method::Options, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("options-call")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-options"))
            .max_forwards(70)
            .build(),
        SimpleRequestBuilder::register("sip:registrar.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("reg-tag"))
            .to("Alice", "sip:alice@example.com", None)
            .call_id("register-call")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-register"))
            .max_forwards(70)
            .contact("sip:alice@127.0.0.1:5060", None)
            .expires(3600)
            .build(),
        invite_request(),
        base_request(Method::Bye).build(),
        base_request(Method::Cancel).build(),
        base_request(Method::Ack).build(),
        base_request(Method::Message)
            .content_type("text/plain")
            .body("hello")
            .build(),
        SimpleRequestBuilder::subscribe("sip:bob@example.com", "presence", 3600)
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("sub-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("subscribe-call")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-subscribe"))
            .max_forwards(70)
            .contact("sip:alice@127.0.0.1:5060", None)
            .build(),
        SimpleRequestBuilder::notify("sip:bob@example.com", "presence", "active;expires=3600")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("notify-from"))
            .to("Bob", "sip:bob@example.com", Some("notify-to"))
            .call_id("notify-call")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-notify"))
            .max_forwards(70)
            .content_type("application/pidf+xml")
            .body("<presence/>")
            .build(),
        base_request(Method::Update)
            .content_type("application/sdp")
            .body("v=0\r\n")
            .build(),
        base_request(Method::Info)
            .content_type("application/info")
            .body("signal=on")
            .build(),
        base_request(Method::Refer)
            .header(TypedHeader::Other(
                HeaderName::ReferTo,
                HeaderValue::Raw(b"<sip:carol@example.com>".to_vec()),
            ))
            .build(),
        base_request(Method::Prack)
            .header(TypedHeader::RAck(RAck::new(1, 42, Method::Invite)))
            .build(),
    ];

    for request in cases {
        let parsed = validate_generated_request(&request)
            .unwrap_or_else(|e| panic!("{} failed generated validation: {}", request.method(), e));
        assert_eq!(parsed.method(), request.method());
        assert_eq!(parsed.cseq().unwrap().method, request.method());
    }
}

#[test]
fn generated_message_compliance_response_status_matrix_roundtrips() {
    let request = invite_request();
    let statuses = [
        StatusCode::Trying,
        StatusCode::Ringing,
        StatusCode::Ok,
        StatusCode::Accepted,
        StatusCode::MovedTemporarily,
        StatusCode::BadRequest,
        StatusCode::Unauthorized,
        StatusCode::ProxyAuthenticationRequired,
        StatusCode::SessionIntervalTooSmall,
        StatusCode::IntervalTooBrief,
        StatusCode::BusyHere,
        StatusCode::RequestTerminated,
        StatusCode::ServerInternalError,
    ];

    for status in statuses {
        let mut response =
            SimpleResponseBuilder::response_from_request(&request, status, None).build();
        if status == StatusCode::Ok {
            response = SimpleResponseBuilder::from_response(response)
                .contact("sip:bob@127.0.0.1:5062", None)
                .content_type("application/sdp")
                .body("v=0\r\n")
                .build();
        }
        let parsed = validate_generated_response(&response)
            .unwrap_or_else(|e| panic!("{} failed generated validation: {}", status, e));
        assert_eq!(parsed.status_code(), status.as_u16());
    }
}

#[test]
fn generated_message_compliance_builders_normalize_content_length() {
    let request = base_request(Method::Message)
        .header(TypedHeader::ContentLength(ContentLength::new(999)))
        .content_type("text/plain")
        .body("hello")
        .build();
    assert_eq!(request.typed_header::<ContentLength>().unwrap().0, 5);
    validate_generated_request(&request).unwrap();

    let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", Some("bob-tag"))
        .call_id("response-call")
        .cseq(1, Method::Message)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-response"))
        .header(TypedHeader::ContentLength(ContentLength::new(999)))
        .content_type("text/plain")
        .body("ok")
        .build();
    assert_eq!(response.typed_header::<ContentLength>().unwrap().0, 2);
    validate_generated_response(&response).unwrap();
}

#[test]
fn generated_message_compliance_empty_messages_have_content_length_zero() {
    let request = base_request(Method::Options).build();
    assert_eq!(request.typed_header::<ContentLength>().unwrap().0, 0);
    validate_generated_request(&request).unwrap();

    let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", Some("bob-tag"))
        .call_id("empty-response")
        .cseq(1, Method::Options)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-empty-response"))
        .build();
    assert_eq!(response.typed_header::<ContentLength>().unwrap().0, 0);
    validate_generated_response(&response).unwrap();
}

#[test]
fn generated_message_compliance_rejects_malformed_messages() {
    let mut missing_call_id = base_request(Method::Options).build();
    missing_call_id
        .headers
        .retain(|h| h.name() != HeaderName::CallId);
    assert!(validate_generated_request(&missing_call_id).is_err());

    let mut duplicate_content_length = base_request(Method::Options).build();
    duplicate_content_length
        .headers
        .push(TypedHeader::ContentLength(ContentLength::new(0)));
    assert!(validate_generated_request(&duplicate_content_length)
        .unwrap_err()
        .to_string()
        .contains("duplicate singleton Content-Length"));

    let mut body_without_content_type = base_request(Method::Message).build();
    body_without_content_type.body = Bytes::from_static(b"hello");
    assert!(validate_generated_request(&body_without_content_type)
        .unwrap_err()
        .to_string()
        .contains("mismatch"));
}
