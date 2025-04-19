// Common test utilities for sip-core

// Use crate:: syntax as this will be part of the test crate
use rvoip_sip_core::error::{Error, Result};
use rvoip_sip_core::message::{Message, Request, Response};
// use rvoip_sip_core::method::Method; // Now in types
// use rvoip_sip_core::message::StatusCode; // Now in types
use rvoip_sip_core::parser::message::parse_message; // Use the parser's function
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use rvoip_sip_core::types::{Address, Method, Param, StatusCode, Via}; // Import types
use rvoip_sip_core::types::{HeaderName, HeaderValue}; // Added Message, Request, Response imports
use rvoip_sip_core::types::sip_message::{Message, Request, Response};
use rvoip_sip_core::{
    Error as SipError,
    Result as SipResult,
    parse_message, // Use the main parse function
    // Message, Request, Response // Already imported from types?
};
use ordered_float::NotNan; // Add import for NotNan

use bytes::Bytes;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::str::FromStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::net::IpAddr;

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
pub fn param_received(val: &str) -> Param { Param::Received(val.to_string()) }
pub fn param_maddr(val: &str) -> Param { Param::Maddr(val.to_string()) }
pub fn param_ttl(val: u8) -> Param { Param::Ttl(val) }
pub fn param_lr() -> Param { Param::Lr }
pub fn param_q(val: f32) -> Param { Param::Q(NotNan::new(val).expect("Q value cannot be NaN")) }
pub fn param_transport(val: &str) -> Param { Param::Transport(val.to_string()) }
pub fn param_user(val: &str) -> Param { Param::User(val.to_string()) }
pub fn param_method(val: &str) -> Param { Param::Method(val.to_string()) }
pub fn param_other(key: &str, value: Option<&str>) -> Param {
    Param::Other(key.to_string(), value.map(String::from))
}


// --- Parser/FromStr Test Helpers ---

/// Asserts that parsing the input string with T::from_str results in the expected value.
pub fn assert_parses_ok<T>(input: &str, expected: T)
where
    T: FromStr<Err = Error> + PartialEq + Debug,
{
    match T::from_str(input) {
        Ok(parsed) => assert_eq!(parsed, expected, "Input: '{}'", input),
        Err(e) => panic!("Expected Ok({:?}), got Err({:?}) for input: '{}'", expected, e, input),
    }
}

/// Asserts that parsing the input string with T::from_str results in an Err.
pub fn assert_parse_fails<T>(input: &str)
where
    T: FromStr<Err = Error> + Debug, // Only need Debug for the panic message
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
    T: Display + FromStr<Err = Error> + PartialEq + Debug + Clone,
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


// --- Message Test Helpers (Keep existing for now) ---

/// Helper to parse a SIP message using bytes
pub fn parse_sip_message_bytes(data: &[u8]) -> Result<Message> {
    parse_message(data) // Direct call to parser
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

/// Validate that a message parses successfully and contains expected fields (basic)
/// Note: This uses raw string comparisons for headers, needs update for typed headers.
pub fn validate_message_basic(
    data: &str, 
    expected_method: Option<Method>,
    expected_uri: Option<&str>,
    expected_headers: &[(&str, &str)] // Check if value *contains* expected
) -> bool {
    match parse_sip_message(data) {
        Ok(message) => {
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
            
            let headers = message.headers();
            for (name, value) in expected_headers {
                let found = headers.iter().any(|h| {
                    h.name.as_str() == *name && h.value.to_string().contains(value)
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

// TODO: Add typed header validation helpers later 