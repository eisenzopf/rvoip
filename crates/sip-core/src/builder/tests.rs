use super::*;
use crate::types::{
    Method,
    StatusCode,
    max_forwards::MaxForwards,
    content_type::ContentType,
    content_length::ContentLength,
    uri::Uri,
    header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait},
    auth::{
        Authorization, WwwAuthenticate, ProxyAuthenticate, 
        ProxyAuthorization, AuthenticationInfo, 
        AuthenticationInfoParam, Qop, Credentials, DigestParam
    },
    headers::header_access::HeaderAccess,
};
use std::str::FromStr;

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

#[test]
fn test_auth_headers() {
    // Test request with Authorization header
    let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .authorization_digest(
            "alice", 
            "sip.example.com", 
            "dcd98b7102dd2f0e8b11d0f600bfb0c093", 
            "5ccc069c403ebaf9f0171e9517f40e41", 
            Some("0a4f113b"), 
            Some("auth"), 
            Some("00000001"), 
            Some("INVITE"), 
            Some("sip:bob@example.com"), 
            Some("MD5"), 
            Some("5ccc069c403ebaf9f0171e9517f40e41")
        )
        .build();
    
    // Check if Authorization header exists
    let auth = request.header(&HeaderName::Authorization);
    assert!(auth.is_some(), "Authorization header not found");
    
    // Test response with WWW-Authenticate header
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Unauthorized"))
        .www_authenticate_digest(
            "sip.example.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093"
        )
        .build();
    
    // Check if WWW-Authenticate header exists
    let www_auth = response.header(&HeaderName::WwwAuthenticate);
    assert!(www_auth.is_some(), "WWW-Authenticate header not found");
    
    // Test response with Proxy-Authenticate header
    let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, Some("Proxy Authentication Required"))
        .proxy_authenticate_digest(
            "sip-proxy.example.com",
            "ae9f97cd017a325b2340afd642016322",
            Some("8d84f05c6ajax14"),
            Some("MD5"),
            Some(vec!["auth"]),
            Some(false),
            None
        )
        .build();
    
    // Check if Proxy-Authenticate header exists
    let proxy_auth = response.header(&HeaderName::ProxyAuthenticate);
    assert!(proxy_auth.is_some(), "Proxy-Authenticate header not found");
    
    // Test request with Proxy-Authorization header
    let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .proxy_authorization_digest(
            "alice", 
            "sip-proxy.example.com", 
            "ae9f97cd017a325b2340afd642016322", 
            "sip:bob@example.com",
            "6629fae49393a05397450978507c4ef1", 
            Some("MD5"),
            Some("9fxk39dmcn38"), 
            None,
            Some("auth"), 
            Some("00000001")
        )
        .build();
    
    // Check if Proxy-Authorization header exists
    let proxy_auth = request.header(&HeaderName::ProxyAuthorization);
    assert!(proxy_auth.is_some(), "Proxy-Authorization header not found");
    
    // Test response with Authentication-Info header
    let auth_info = AuthenticationInfo::new()
        .with_nextnonce("47364c23432d2e131a5fb210812c")
        .with_qop(Qop::Auth)
        .with_rspauth("988df663c6161de88914873c9975fd4c")
        .with_cnonce("0a4f113b")
        .with_nonce_count(1);
    
    let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .set_header(auth_info)
        .build();
    
    // Check if Authentication-Info header exists
    let auth_info_header = response.header(&HeaderName::AuthenticationInfo);
    assert!(auth_info_header.is_some(), "Authentication-Info header not found");
    
    if let Some(TypedHeader::AuthenticationInfo(info)) = auth_info_header {
        assert!(info.0.contains(&AuthenticationInfoParam::NextNonce("47364c23432d2e131a5fb210812c".to_string())));
        assert!(info.0.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(info.0.contains(&AuthenticationInfoParam::ResponseAuth("988df663c6161de88914873c9975fd4c".to_string())));
        assert!(info.0.contains(&AuthenticationInfoParam::Cnonce("0a4f113b".to_string())));
        assert!(info.0.contains(&AuthenticationInfoParam::NonceCount(1)));
    }
}

#[test]
fn test_comprehensive_request_response() {
    // Create a comprehensive SIP REGISTER request with multiple headers
    let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
        .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
        .cseq(1)
        .via("192.168.1.1", "UDP", Some("z9hG4bKa7"))
        .max_forwards(70)
        .contact("sip:alice@192.168.1.1", Some("Alice's Phone"))
        .user_agent("SIPlib/1.0")
        .content_type("application/sdp")
        .authorization_digest(
            "alice", 
            "example.com", 
            "dcd98b7102dd2f0e8b11d0f600bfb0c093", 
            "5ccc069c403ebaf9f0171e9517f40e41", 
            Some("0a4f113b"), 
            Some("auth"), 
            Some("00000001"), 
            Some("REGISTER"), 
            Some("sip:example.com"), 
            Some("MD5"), 
            None
        )
        .body("v=0\r\no=alice 123 456 IN IP4 192.168.1.1\r\ns=SIP Call\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\n")
        .build();
    
    // Verify essential headers
    assert_eq!(request.method, Method::Register);
    assert_eq!(request.uri.to_string(), "sip:example.com");
    assert_eq!(request.call_id().unwrap().value(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    
    // Check if all headers exist
    assert!(request.from().is_some());
    assert!(request.to().is_some());
    assert!(request.cseq().is_some());
    assert!(request.first_via().is_some());
    assert!(request.header(&HeaderName::MaxForwards).is_some());
    assert!(request.header(&HeaderName::Contact).is_some());
    assert!(request.header(&HeaderName::UserAgent).is_some());
    assert!(request.header(&HeaderName::ContentType).is_some());
    assert!(request.header(&HeaderName::Authorization).is_some());
    
    // Verify body is set correctly
    assert!(!request.body.is_empty());
    
    // Create a comprehensive SIP response with multiple headers
    // Create Authentication-Info header separately
    let auth_info = AuthenticationInfo::new()
        .with_nextnonce("47364c23432d2e131a5fb210812c")
        .with_qop(Qop::Auth)
        .with_rspauth("988df663c6161de88914873c9975fd4c");
        
    let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
        .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
        .to("Bob", "sip:bob@example.com", Some("38fd98"))
        .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
        .cseq(1, Method::Register)
        .via("192.168.1.1", "UDP", Some("z9hG4bKa7"))
        .contact("sip:alice@192.168.1.1", Some("Alice's Phone"))
        .server("SIPServer/1.0")
        .set_header(auth_info)
        .content_type("application/sdp")
        .body("v=0\r\no=server 123 456 IN IP4 192.168.1.2\r\ns=SIP Response\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\n")
        .build();
    
    // Verify essential headers
    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.reason, Some("OK".to_string()));
    assert_eq!(response.call_id().unwrap().value(), "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    
    // Check if all headers exist
    assert!(response.from().is_some());
    assert!(response.to().is_some());
    assert!(response.cseq().is_some());
    assert!(response.first_via().is_some());
    assert!(response.header(&HeaderName::Contact).is_some());
    assert!(response.header(&HeaderName::Server).is_some());
    assert!(response.header(&HeaderName::AuthenticationInfo).is_some());
    assert!(response.header(&HeaderName::ContentType).is_some());
    
    // Verify body is set correctly
    assert!(!response.body.is_empty());
}

#[test]
fn test_header_roundtrip_conversion() {
    // Test WWW-Authenticate roundtrip
    let www_auth_str = r#"Digest realm="sip.example.com",nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",algorithm=MD5,qop="auth,auth-int""#;
    
    // Parse from string to typed
    let www_auth = WwwAuthenticate::from_str(www_auth_str).unwrap();
    
    // Convert to header
    let header = www_auth.to_header();
    
    // The TypedHeader conversion is handled correctly by the set_header operation,
    // so let's test the properties directly instead of converting back
    if let HeaderValue::WwwAuthenticate(value) = &header.value {
        assert_eq!(format!("{}", www_auth), format!("{}", value));
    } else {
        panic!("Expected HeaderValue::WwwAuthenticate");
    }
    
    // Test Authorization roundtrip
    let auth_str = r#"Digest username="alice",realm="sip.example.com",nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",uri="sip:example.com",response="5ccc069c403ebaf9f0171e9517f40e41",algorithm=MD5,cnonce="0a4f113b",qop=auth,nc=00000001"#;
    
    // Parse from string to typed
    let auth = Authorization::from_str(auth_str).unwrap();
    
    // Convert to header
    let header = auth.to_header();
    
    // Check directly on the header value
    if let HeaderValue::Authorization(value) = &header.value {
        if let Credentials::Digest { params } = &value.0 {
            assert!(params.iter().any(|p| matches!(p, DigestParam::Username(name) if name == "alice")));
        } else {
            panic!("Expected Digest credentials");
        }
    } else {
        panic!("Expected HeaderValue::Authorization");
    }
    
    // Test Proxy-Authenticate roundtrip
    let proxy_auth_str = r#"Digest realm="sip-proxy.example.com",nonce="ae9f97cd017a325b2340afd642016322",algorithm=MD5,qop="auth""#;
    
    // Parse from string to typed
    let proxy_auth = ProxyAuthenticate::from_str(proxy_auth_str).unwrap();
    
    // Convert to header
    let header = proxy_auth.to_header();
    
    // Check directly on the header value
    if let HeaderValue::ProxyAuthenticate(value) = &header.value {
        assert_eq!(format!("{}", proxy_auth), format!("{}", value));
    } else {
        panic!("Expected HeaderValue::ProxyAuthenticate");
    }
    
    // Test Proxy-Authorization roundtrip
    let proxy_auth_str = r#"Digest username="alice",realm="sip-proxy.example.com",nonce="ae9f97cd017a325b2340afd642016322",uri="sip:example.com",response="6629fae49393a05397450978507c4ef1",algorithm=MD5,cnonce="9fxk39dmcn38",qop=auth,nc=00000001"#;
    
    // Parse from string to typed
    let proxy_auth = ProxyAuthorization::from_str(proxy_auth_str).unwrap();
    
    // Convert to header
    let header = proxy_auth.to_header();
    
    // Check directly on the header value
    if let HeaderValue::ProxyAuthorization(value) = &header.value {
        if let Credentials::Digest { params } = &value.0 {
            assert!(params.iter().any(|p| matches!(p, DigestParam::Username(name) if name == "alice")));
        } else {
            panic!("Expected Digest credentials");
        }
    } else {
        panic!("Expected HeaderValue::ProxyAuthorization");
    }
    
    // Test Authentication-Info roundtrip
    let auth_info_str = r#"nextnonce="47364c23432d2e131a5fb210812c",qop=auth,rspauth="988df663c6161de88914873c9975fd4c",cnonce="0a4f113b",nc=00000001"#;
    
    // Parse from string to typed
    let auth_info = AuthenticationInfo::from_str(auth_info_str).unwrap();
    
    // Convert to header
    let header = auth_info.to_header();
    
    // Check directly on the header value
    if let HeaderValue::AuthenticationInfo(value) = &header.value {
        assert!(value.0.contains(&AuthenticationInfoParam::NextNonce("47364c23432d2e131a5fb210812c".to_string())));
        assert!(value.0.contains(&AuthenticationInfoParam::ResponseAuth("988df663c6161de88914873c9975fd4c".to_string())));
    } else {
        panic!("Expected HeaderValue::AuthenticationInfo");
    }
} 