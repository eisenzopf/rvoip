// Message parser tests for SIP messages
// This file tests full SIP message parsing functionality

use std::str::FromStr;

// Import common test utilities
use crate::common::*;

// Import SIP Core types with specific imports
use rvoip_sip_core::{
    error::Error,
    parse_message,
    types::{
        Message,
        StatusCode,
        header::HeaderName
    },
};

#[test]
fn test_parse_unreason_response() {
    // This is the content of 3.1.1.12_unreason.sip from the RFC compliance tests
    // Test a response with non-ASCII characters in the reason phrase
    // Ensure all line endings are CRLF for RFC compliance
    let message = "SIP/2.0 200 = 2**3 * 5**2 но сто девяносто девять - простое\r\n\
Via: SIP/2.0/UDP 192.0.2.198;branch=z9hG4bK1324923\r\n\
Call-ID: unreason.1234ksdfak3j2erwedfsASdf\r\n\
CSeq: 35 INVITE\r\n\
From: sip:user@example.com;tag=11141343\r\n\
To: sip:user@example.edu;tag=2229\r\n\
Content-Length: 152\r\n\
Content-Type: application/sdp\r\n\
Contact: <sip:user@host198.example.com>\r\n\
\r\n\
v=0\r\n\
o=mhandley 29739 7272939 IN IP4 192.0.2.198\r\n\
s=-\r\n\
c=IN IP4 192.0.2.198\r\n\
t=0 0\r\n\
m=audio 49217 RTP/AVP 0 12\r\n\
m=video 3227 RTP/AVP 31\r\n\
a=rtpmap:31 LPC";

    // Try parsing each header individually to isolate the issue
    println!("--- Testing individual headers ---");
    
    // Status line
    println!("Status line: SIP/2.0 200 = 2**3 * 5**2 но сто девяносто девять - простое");
    
    // Via header
    println!("Via header: SIP/2.0/UDP 192.0.2.198;branch=z9hG4bK1324923");
    
    // Call-ID header
    println!("Call-ID header: unreason.1234ksdfak3j2erwedfsASdf");
    
    // CSeq header
    println!("CSeq header: 35 INVITE");
    
    // From header
    println!("From header: sip:user@example.com;tag=11141343");
    
    // To header
    println!("To header: sip:user@example.edu;tag=2229");
    
    // Now try parsing the whole message
    println!("\n--- Attempting to parse complete message ---");
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info on the parse result
    match &result {
        Ok(msg) => println!("Successfully parsed message: {:?}", msg),
        Err(e) => {
            println!("Parse error: {}", e);
            
            // Try parsing with byte-by-byte inspection to find the exact failure point
            println!("\n--- Message bytes (with line numbers) ---");
            for (i, line) in message.lines().enumerate() {
                println!("{:3}: {}", i+1, line);
            }
            
            // Try again with a simple message to see if basic parsing works
            let simple_message = "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP 192.0.2.198;branch=z9hG4bK1324923\r\nCall-ID: test\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";
            println!("\n--- Trying with a simplified message ---");
            match parse_message(simple_message.as_bytes()) {
                Ok(_) => println!("Simple message parsed successfully"),
                Err(e) => println!("Simple message parse error: {}", e)
            }
        }
    }
    
    // The test should pass - this message should be valid according to RFC 4475
    assert!(result.is_ok(), "Failed to parse wellformed message");
    
    // If it parsed successfully, verify the main components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Response(resp) => {
                assert_eq!(resp.status, StatusCode::Ok);
                // The reason phrase contains non-ASCII characters
                assert!(resp.reason.as_ref().unwrap().contains("но сто девяносто девять"));
                
                // Verify some headers exist
                assert!(resp.header(&HeaderName::Via).is_some());
                assert!(resp.header(&HeaderName::CallId).is_some());
                assert!(resp.header(&HeaderName::CSeq).is_some());
                
                // Check content-length and body
                let content_length = resp.header(&HeaderName::ContentLength)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0);
                assert_eq!(content_length, 152);
                assert!(!resp.body.is_empty());
            },
            _ => panic!("Expected Response but got Request")
        }
    }
}

// Added another test with a simpler message to isolate the issue
#[test]
fn test_minimal_response() {
    // Create a minimal SIP response with just the required headers
    let message = "SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP 192.0.2.1;branch=z9hG4bK123\r\n\
Call-ID: test-call-id\r\n\
CSeq: 1 INVITE\r\n\
From: <sip:alice@example.com>;tag=abc\r\n\
To: <sip:bob@example.com>\r\n\
Content-Length: 0\r\n\
\r\n";

    // Parse the message
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info
    match &result {
        Ok(msg) => println!("Successfully parsed minimal response: {:?}", msg),
        Err(e) => println!("Parse error for minimal response: {}", e)
    }
    
    assert!(result.is_ok(), "Failed to parse minimal response");
}

#[test]
fn test_parse_basic_invite() {
    // Calculate the exact body length first
    let body = "v=0\r\n\
o=alice 2890844526 2890844526 IN IP4 pc33.atlanta.com\r\n\
s=Session SDP\r\n\
c=IN IP4 pc33.atlanta.com\r\n\
t=0 0\r\n\
m=audio 49172 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000";
    
    let body_len = body.len();
    println!("Exact body length for INVITE: {}", body_len);

    // Test a basic INVITE request
    let message = format!("INVITE sip:bob@biloxi.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.com>\r\n\
From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.com>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: {}\r\n\
\r\n\
{}", body_len, body);

    // Parse the message
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info
    match &result {
        Ok(msg) => println!("Successfully parsed INVITE: {:?}", msg),
        Err(e) => println!("Parse error for INVITE: {}", e)
    }
    
    assert!(result.is_ok(), "Failed to parse basic INVITE request");
    
    // If parsed successfully, verify the components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Request(req) => {
                assert_eq!(req.method.to_string(), "INVITE");
                assert_eq!(req.uri.to_string(), "sip:bob@biloxi.com");
                
                // Verify headers
                assert!(req.header(&HeaderName::Via).is_some());
                assert!(req.header(&HeaderName::From).is_some());
                assert!(req.header(&HeaderName::To).is_some());
                assert!(req.header(&HeaderName::CallId).is_some());
                assert!(req.header(&HeaderName::CSeq).is_some());
                
                // Check body
                let content_length = req.header(&HeaderName::ContentLength)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0);
                assert_eq!(content_length, body_len as u32);
                assert!(!req.body.is_empty());
            },
            _ => panic!("Expected Request but got Response")
        }
    }
}

// Add a test with a short content length
#[test]
fn test_parse_response_with_small_body() {
    // Test SIP response with a very small body to debug Content-Length issues
    let message = "SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP 192.0.2.1;branch=z9hG4bK123\r\n\
Call-ID: test-call-id\r\n\
CSeq: 1 INVITE\r\n\
From: <sip:alice@example.com>;tag=abc\r\n\
To: <sip:bob@example.com>\r\n\
Content-Length: 4\r\n\
\r\n\
Test";

    // Parse the message
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info
    match &result {
        Ok(msg) => println!("Successfully parsed small body response: {:?}", msg),
        Err(e) => println!("Parse error for small body response: {}", e)
    }
    
    assert!(result.is_ok(), "Failed to parse response with small body");
}

// Add a test with an exact content length
#[test]
fn test_parse_unreason_response_exact_length() {
    // This is the content of 3.1.1.12_unreason.sip with an EXACTLY matching content length
    // For debugging the "Incomplete message: Needed Size(10)" error
    
    // Calculate the exact body length first
    let body = "v=0\r\n\
o=mhandley 29739 7272939 IN IP4 192.0.2.198\r\n\
s=-\r\n\
c=IN IP4 192.0.2.198\r\n\
t=0 0\r\n\
m=audio 49217 RTP/AVP 0 12\r\n\
m=video 3227 RTP/AVP 31\r\n\
a=rtpmap:31 LPC";
    
    let body_len = body.len();
    println!("Exact body length: {}", body_len);
    
    // Create the message with the actual body length
    let message = format!("SIP/2.0 200 = 2**3 * 5**2 но сто девяносто девять - простое\r\n\
Via: SIP/2.0/UDP 192.0.2.198;branch=z9hG4bK1324923\r\n\
Call-ID: unreason.1234ksdfak3j2erwedfsASdf\r\n\
CSeq: 35 INVITE\r\n\
From: sip:user@example.com;tag=11141343\r\n\
To: sip:user@example.edu;tag=2229\r\n\
Content-Length: {}\r\n\
Content-Type: application/sdp\r\n\
Contact: <sip:user@host198.example.com>\r\n\
\r\n\
{}", body_len, body);

    // Parse the message
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info
    match &result {
        Ok(msg) => println!("Successfully parsed with exact length: {:?}", msg),
        Err(e) => {
            println!("Parse error with exact length: {}", e);
            
            // Try parsing with various content lengths
            for test_len in [body_len-1, body_len, body_len+1, body_len+10] {
                let test_message = format!("SIP/2.0 200 OK\r\n\
Content-Length: {}\r\n\
\r\n\
{}", test_len, body);
                
                match parse_message(test_message.as_bytes()) {
                    Ok(_) => println!("Content-Length {} worked!", test_len),
                    Err(e) => println!("Content-Length {} failed: {}", test_len, e)
                }
            }
        }
    }
    
    assert!(result.is_ok(), "Failed to parse wellformed message with exact length");
}

// Add a test for one of the failing wellformed messages from the torture tests
#[test]
fn test_parse_noreason_response() {
    // This is the content of 3.1.1.13_noreason.sip from the RFC compliance tests
    // Test a response with empty reason phrase
    // NOTE: While this specific test passes, the SIP torture tests from RFC 4475 are currently failing
    // due to issues with the URI parser handling non-standard schemes, unusual IPV6 addresses,
    // and various character encodings. A more comprehensive fix would require more extensive changes.
    let message = "SIP/2.0 100 \r\n\
Via: SIP/2.0/UDP 192.0.2.105;branch=z9hG4bK2398ndaoe\r\n\
Call-ID: noreason.asndj203insdf99223ndf\r\n\
CSeq: 35 INVITE\r\n\
From: <sip:user@example.com>;tag=39ansfi3\r\n\
To: <sip:user@example.edu>;tag=902jndnke3\r\n\
Content-Length: 0\r\n\
Contact: <sip:user@host105.example.com>\r\n\
\r\n";

    // Try parsing each header individually to isolate the issue
    println!("--- Testing individual headers ---");
    
    // Status line
    println!("Status line: SIP/2.0 100 ");
    
    // Via header
    println!("Via header: SIP/2.0/UDP 192.0.2.105;branch=z9hG4bK2398ndaoe");
    
    // Call-ID header
    println!("Call-ID header: noreason.asndj203insdf99223ndf");
    
    // CSeq header
    println!("CSeq header: 35 INVITE");
    
    // From header
    println!("From header: <sip:user@example.com>;tag=39ansfi3");
    
    // To header
    println!("To header: <sip:user@example.edu>;tag=902jndnke3");
    
    // Now try parsing the whole message
    println!("\n--- Attempting to parse complete message ---");
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info on the parse result
    match &result {
        Ok(msg) => println!("Successfully parsed message: {:?}", msg),
        Err(e) => {
            println!("Parse error: {}", e);
            
            // Try parsing with byte-by-byte inspection to find the exact failure point
            println!("\n--- Message bytes (with line numbers) ---");
            for (i, line) in message.lines().enumerate() {
                println!("{:3}: {}", i+1, line);
            }
        }
    }
    
    // The test should pass - this message should be valid according to RFC 3261
    assert!(result.is_ok(), "Failed to parse wellformed message");
    
    // If it parsed successfully, verify the main components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Response(resp) => {
                assert_eq!(resp.status, StatusCode::Trying);
                // The reason phrase is empty
                assert_eq!(resp.reason.as_deref(), Some(""));
                
                // Verify some headers exist
                assert!(resp.header(&HeaderName::Via).is_some());
                assert!(resp.header(&HeaderName::CallId).is_some());
                assert!(resp.header(&HeaderName::CSeq).is_some());
                
                // Check content-length
                let content_length = resp.header(&HeaderName::ContentLength)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0);
                assert_eq!(content_length, 0);
                assert!(resp.body.is_empty());
            },
            _ => panic!("Expected Response but got Request")
        }
    }
} 