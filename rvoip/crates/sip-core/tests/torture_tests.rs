use rvoip_sip_core::{
    parse_message, Message, Result, Error,
    Header, HeaderName, HeaderValue,
    Method, Uri, Version, Request, Response, StatusCode,
};
use bytes::Bytes;
use std::str::FromStr;

pub mod rfc4475;
pub mod rfc5118;
pub mod custom_torture;

/// Helper function to parse a SIP message and return the result
pub fn parse_sip_message(message: &str) -> Result<Message> {
    let bytes = Bytes::copy_from_slice(message.as_bytes());
    parse_message(&bytes)
}

/// Helper function to check if a message parsing returns the expected error
pub fn expect_parse_error(message: &str, expected_error: Option<&str>) -> bool {
    let result = parse_sip_message(message);
    
    match (result, expected_error) {
        (Err(e), Some(expected)) => {
            let err_string = e.to_string();
            if !err_string.contains(expected) {
                println!("Expected error containing '{}', got: '{}'", expected, err_string);
                false
            } else {
                true
            }
        },
        (Err(_), None) => true, // Expected any error
        (Ok(_), Some(expected)) => {
            println!("Expected error containing '{}', but parsing succeeded", expected);
            false
        },
        (Ok(_), None) => {
            println!("Expected parsing to fail, but it succeeded");
            false
        },
    }
}

/// Helper function to validate a parsed SIP message
pub fn validate_message(
    result: Result<Message>,
    expected_method: Option<Method>,
    expected_uri: Option<&str>,
    expected_headers: &[(&str, &str)],
) -> bool {
    match result {
        Ok(Message::Request(req)) => {
            // Validate method
            if let Some(expected) = expected_method {
                if req.method != expected {
                    println!("Expected method {:?}, got {:?}", expected, req.method);
                    return false;
                }
            }
            
            // Validate URI
            if let Some(expected) = expected_uri {
                if req.uri.to_string() != expected {
                    println!("Expected URI {}, got {}", expected, req.uri);
                    return false;
                }
            }
            
            // Validate headers
            for (name, value) in expected_headers {
                let header_name = HeaderName::from_str(name).unwrap();
                // Find any header that matches the name and value
                let found = req.headers.iter().any(|h| {
                    h.name == header_name && h.value.to_string() == *value
                });
                
                if !found {
                    println!("Header {} with value '{}' not found", name, value);
                    return false;
                }
            }
            
            true
        },
        Ok(Message::Response(resp)) => {
            // For responses, we don't check method/URI but we can check status code and headers
            
            // Validate headers
            for (name, value) in expected_headers {
                let header_name = HeaderName::from_str(name).unwrap();
                // Find any header that matches the name and value
                let found = resp.headers.iter().any(|h| {
                    h.name == header_name && h.value.to_string() == *value
                });
                
                if !found {
                    println!("Header {} with value '{}' not found", name, value);
                    return false;
                }
            }
            
            true
        },
        Err(e) => {
            println!("Expected successful parse, got error: {}", e);
            false
        }
    }
}

#[test]
fn test_valid_basic_messages() {
    // A very basic INVITE request
    let basic_invite = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r
Max-Forwards: 70\r
To: Bob <sip:bob@biloxi.example.com>\r
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r
CSeq: 314159 INVITE\r
Contact: <sip:alice@pc33.atlanta.example.com>\r
Content-Type: application/sdp\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(basic_invite);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:bob@biloxi.example.com"),
        &[
            ("Via", "SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds"),
            ("From", "Alice <sip:alice@atlanta.example.com>;tag=1928301774"),
            ("To", "Bob <sip:bob@biloxi.example.com>"),
        ]
    ));

    // A basic 200 OK response
    let basic_response = "\
SIP/2.0 200 OK\r
Via: SIP/2.0/UDP server10.biloxi.example.com;branch=z9hG4bK4b43c2ff8.1\r
Via: SIP/2.0/UDP bigbox3.site3.atlanta.example.com;branch=z9hG4bK77ef4c2312983.1\r
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r
To: Bob <sip:bob@biloxi.example.com>;tag=a6c85cf\r
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r
CSeq: 314159 INVITE\r
Contact: <sip:bob@biloxi.example.com>\r
Content-Type: application/sdp\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(basic_response);
    assert!(result.is_ok());
    if let Ok(Message::Response(resp)) = result {
        assert_eq!(resp.status, StatusCode::Ok);
    } else {
        panic!("Expected Response, got something else");
    }
} 