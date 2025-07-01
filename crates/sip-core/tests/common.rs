// Common test utilities for sip-core
use std::str::FromStr;
use std::net::IpAddr;
use bytes::Bytes;
use ordered_float::NotNan;

// SIP Core imports using the rvoip_sip_core crate name
use rvoip_sip_core::types::uri::{Uri, Scheme, Host};
use rvoip_sip_core::types::{Address, Method, Param, StatusCode};
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::param::GenericValue;
use rvoip_sip_core::types::sip_message::{Message, Request, Response};
use rvoip_sip_core::{
    Error as SipError,
    Result as SipResult,
    parse_message, 
    types::header::{HeaderName, HeaderValue, TypedHeader}
};

// Use crate:: syntax as this will be part of the test crate
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::panic::{catch_unwind, AssertUnwindSafe};

// --- Type Construction Helpers ---

/// Parses a string into a Uri, panicking on failure.
pub fn uri(uri_str: &str) -> Uri {
    Uri::from_str(uri_str).unwrap_or_else(|e| {
        panic!("Failed to parse test URI '{}': {:?}", uri_str, e)
    })
}

/// Creates an Address struct.
pub fn addr(display_name: Option<&str>, uri_str: &str, params: Vec<Param>) -> Address {
     Address {
         display_name: display_name.map(String::from),
         uri: uri(uri_str), // Use uri helper
         params,
     }
}

// Param construction helpers
pub fn param_tag(val: &str) -> Param { Param::Tag(val.to_string()) }
pub fn param_branch(val: &str) -> Param { Param::Branch(val.to_string()) }
pub fn param_expires(val: u32) -> Param { Param::Expires(val) }
pub fn param_received(val: &str) -> Param { Param::Received(IpAddr::from_str(val).expect("Invalid IP address for received param")) }
pub fn param_maddr(val: &str) -> Param { Param::Maddr(val.to_string()) }
pub fn param_ttl(val: u8) -> Param { Param::Ttl(val) }
pub fn param_lr() -> Param { Param::Lr }
pub fn param_q(val: f32) -> Param { Param::Q(NotNan::new(val).expect("Q value cannot be NaN")) }
pub fn param_transport(val: &str) -> Param { Param::Transport(val.to_string()) }
pub fn param_user(val: &str) -> Param { Param::User(val.to_string()) }
pub fn param_method(val: &str) -> Param { Param::Method(val.to_string()) }
pub fn param_other(key: &str, value: Option<&str>) -> Param {
    Param::Other(
        key.to_string(), 
        value.map(|v| GenericValue::Token(v.to_string()))
    )
}

// --- Parser/FromStr Test Helpers ---

/// Asserts that parsing the input string with T::from_str results in the expected value.
pub fn assert_parses_ok<T>(input: &str, expected: T)
where
    T: FromStr<Err = SipError> + PartialEq + Debug,
{
    match T::from_str(input) {
        Ok(parsed) => assert_eq!(parsed, expected, "Input: '{}'", input),
        Err(e) => panic!("Expected Ok({:?}), got Err({:?}) for input: '{}'", expected, e, input),
    }
}

/// Asserts that parsing the input string with T::from_str results in an Err.
pub fn assert_parse_fails<T>(input: &str)
where
    T: FromStr<Err = SipError> + Debug, // Only need Debug for the panic message
{
     match T::from_str(input) {
        Ok(parsed) => panic!("Expected Err, got Ok({:?}) for input: '{}'", parsed, input),
        Err(_) => { /* Success */ } ,
    }
}

// --- Display Test Helper ---

/// Asserts that item.to_string() can be parsed back into an equivalent item.
pub fn assert_display_parses_back<T>(item: &T)
where
    T: Display + FromStr<Err = SipError> + PartialEq + Debug + Clone,
{
    let displayed = item.to_string();
    match T::from_str(&displayed) {
        Ok(parsed_back) => {
            // Compare original with parsed-back version
            assert_eq!(item, &parsed_back, 
                "\nDisplay->FromStr round trip failed!\n  Original: {:?}\n  Displayed: '{}'\n  Parsed Back: {:?}\n", 
                item, displayed, parsed_back);
        }
        Err(e) => panic!("Failed to parse back displayed string '{}': {:?}", displayed, e),
    }
}

// --- Message Test Helpers ---

/// Helper to parse a SIP message using bytes
pub fn parse_sip_message_bytes(data: &[u8]) -> SipResult<Message> {
    parse_message(data)
}

/// Helper to parse a SIP message from a string
pub fn parse_sip_message(msg: &str) -> SipResult<Message> {
    parse_message(msg.as_bytes())
}

/// Helper to expect a parsing error
pub fn expect_parse_error(msg: &str, expected_substring: Option<&str>) -> bool {
    let result = parse_message(msg.as_bytes());
    match result {
        Ok(_) => false,
        Err(e) => {
            if let Some(sub) = expected_substring {
                e.to_string().contains(sub)
            } else {
                true // Any error is acceptable if no substring specified
            }
        }
    }
}

/// Create a SIP request for testing
pub fn create_test_request(method: Method, uri_str: &str) -> Request {
    let uri = Uri::from_str(uri_str).expect("Invalid URI for test");
    Request::new(method, uri)
}

/// Create a SIP response for testing
pub fn create_test_response(status: StatusCode) -> Response {
    Response::new(status)
}

/// Create a basic SIP message for testing (common test case)
pub fn create_basic_message(method: Method, uri_str: &str) -> Message {
    // Clone method since it doesn't implement Copy trait
    let mut request = create_test_request(method.clone(), uri_str);
    
    // Import the specific types needed for each header
    use rvoip_sip_core::types::to::To;
    use rvoip_sip_core::types::from::From;
    use rvoip_sip_core::types::call_id::CallId;
    use rvoip_sip_core::types::cseq::CSeq;
    
    // Create properly typed headers
    let to_header = To(addr(Some("Bob"), "sip:bob@example.com", vec![]));
    let from_header = From(addr(Some("Alice"), "sip:alice@example.com", vec![param_tag("1234")]));
    let call_id = CallId("test-call-id".to_string());
    let cseq = CSeq::new(1, method);
    
    request = request
        .with_header(TypedHeader::To(to_header))
        .with_header(TypedHeader::From(from_header))
        .with_header(TypedHeader::CallId(call_id))
        .with_header(TypedHeader::CSeq(cseq))
        .with_header(TypedHeader::Via(Via::new(
            "SIP", "2.0", "UDP", "example.com", None, vec![param_branch("z9hG4bK-test")]
        ).expect("Failed to create Via header")));
    
    Message::Request(request)
}

/// Validate that a message parses successfully and contains expected headers
pub fn validate_message(
    data: &str, 
    expected_method: Option<Method>,
    expected_uri: Option<&str>,
    expected_headers: &[(&str, &str)] // Check if value *contains* expected
) -> bool {
    match parse_sip_message(data) {
        Ok(message) => {
            // Check method and URI for requests
            if let Some(method) = expected_method {
                if let Message::Request(request) = &message {
                    if request.method != method {
                        println!("Expected method {:?} but got {:?}", method, request.method);
                        return false;
                    }
                } else {
                    println!("Expected request but got response");
                    return false;
                }
            }
            
            if let Some(uri_str) = expected_uri {
                if let Message::Request(request) = &message {
                    if request.uri.to_string() != uri_str {
                        println!("Expected URI {} but got {}", uri_str, request.uri);
                        return false;
                    }
                } else {
                    println!("Expected request but got response");
                    return false;
                }
            }
            
            // Check headers
            let headers = message.headers();
            for (name, value) in expected_headers {
                let found = headers.iter().any(|h| {
                    h.name().as_str() == *name && h.to_string().contains(value)
                });
                
                if !found {
                    println!("Header '{}' with value containing '{}' not found", name, value);
                    return false;
                }
            }
            
            true
        },
        Err(error) => {
            println!("Expected successful parse but got error: {:?}", error);
            false
        }
    }
}

// Helper to create a Via header
pub fn via(host: Host, port: Option<u16>, transport: &str) -> Via {
    Via::new(
        "SIP",
        "2.0",
        transport.to_string(),
        host.to_string(),
        port,
        vec![]
    ).expect("Failed to create Via header")
}