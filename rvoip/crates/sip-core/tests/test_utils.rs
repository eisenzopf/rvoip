use rvoip_sip_core::{Error, Message, Request, Response, Method, StatusCode, parse_message, Uri};
use bytes::Bytes;
use std::collections::HashMap;

/// Helper to parse a SIP message
pub fn parse_sip_message(data: &str) -> Result<Message, Error> {
    parse_message(&Bytes::from(data.to_string()))
}

/// Helper to expect a parse error
pub fn expect_parse_error(data: &str, _expected_error: Option<&str>) -> bool {
    match parse_sip_message(data) {
        Ok(message) => {
            println!("Expected error but got successful parse: {:?}", message);
            false
        },
        Err(error) => {
            println!("Got expected error: {:?}", error);
            true
        }
    }
}

/// Validate that a message parses successfully and contains expected fields
pub fn validate_message(
    data: &str, 
    expected_method: Option<Method>,
    expected_uri: Option<&str>,
    expected_headers: &[(&str, &str)]
) -> bool {
    match parse_sip_message(data) {
        Ok(message) => {
            // Check method if expected
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
            
            // Check URI if expected
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
            
            // Check headers if expected
            for (name, value) in expected_headers {
                let found = message.headers().iter().any(|h| {
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