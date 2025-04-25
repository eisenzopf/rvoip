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
        Method,
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

// Add a test for a wellformed SIP message with IPv6 addresses
#[test]
fn test_parse_ipv6_request() {
    // This is the content of 4.1_ipv6-good.sip from the RFC compliance tests
    // Test a request with IPv6 addresses
    // NOTE: This test may fail due to issues with the URI parser handling IPv6 addresses.
    // A more comprehensive fix would require more extensive changes to the URI parser.
    let message = "REGISTER sip:[2001:db8::10] SIP/2.0\r\n\
To: sip:user@example.com\r\n\
From: sip:user@example.com;tag=81x2\r\n\
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3-111\r\n\
Call-ID: SSG9559905523997077@hlau_4100\r\n\
Max-Forwards: 70\r\n\
Contact: \"Caller\" <sip:caller@[2001:db8::1]>\r\n\
CSeq: 98176 REGISTER\r\n\
Content-Length: 0\r\n\
\r\n";

    // Try parsing each header individually to isolate the issue
    println!("--- Testing individual headers ---");
    
    // Request line
    println!("Request line: REGISTER sip:[2001:db8::10] SIP/2.0");
    
    // To header
    println!("To header: sip:user@example.com");
    
    // From header
    println!("From header: sip:user@example.com;tag=81x2");
    
    // Via header
    println!("Via header: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3-111");
    
    // Call-ID header
    println!("Call-ID header: SSG9559905523997077@hlau_4100");
    
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
    
    // Comment out the assertion if we know it will fail due to URI parser limitations
    assert!(result.is_ok(), "Failed to parse wellformed message with IPv6 addresses");
    
    // If it parsed successfully, verify the main components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Request(req) => {
                assert_eq!(req.method, Method::Register);
                assert_eq!(req.uri.to_string(), "sip:[2001:db8::10]");
                
                // Verify some headers exist
                assert!(req.header(&HeaderName::Via).is_some());
                assert!(req.header(&HeaderName::From).is_some());
                assert!(req.header(&HeaderName::To).is_some());
                assert!(req.header(&HeaderName::CallId).is_some());
                assert!(req.header(&HeaderName::CSeq).is_some());
                
                // Check content-length
                let content_length = req.header(&HeaderName::ContentLength)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0);
                assert_eq!(content_length, 0);
                assert!(req.body.is_empty());
            },
            _ => panic!("Expected Request but got Response")
        }
    }
}

// Add a test for a SIP message with multiple Via headers and transport types
#[test]
fn test_parse_multiple_transports() {
    // This is the content of 3.1.1.10_transports.sip from the RFC compliance tests
    // Test a request with multiple Via headers with different transport types
    let message = "OPTIONS sip:user@example.com SIP/2.0\r\n\
To: sip:user@example.com\r\n\
From: <sip:caller@example.com>;tag=323\r\n\
Max-Forwards: 70\r\n\
Call-ID:  transports.kijh4akdnaqjkwendsasfdj\r\n\
Accept: application/sdp\r\n\
CSeq: 60 OPTIONS\r\n\
Via: SIP/2.0/UDP t1.example.com;branch=z9hG4bKkdjuw\r\n\
Via: SIP/2.0/SCTP t2.example.com;branch=z9hG4bKklasjdhf\r\n\
Via: SIP/2.0/TLS t3.example.com;branch=z9hG4bK2980unddj\r\n\
Via: SIP/2.0/UNKNOWN t4.example.com;branch=z9hG4bKasd0f3en\r\n\
Via: SIP/2.0/TCP t5.example.com;branch=z9hG4bK0a9idfnee\r\n\
l: 0\r\n\
\r\n";

    // Now try parsing the whole message
    println!("\n--- Attempting to parse message with multiple transports ---");
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
    
    assert!(result.is_ok(), "Failed to parse wellformed message with multiple transports");
    
    // If it parsed successfully, verify the main components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Request(req) => {
                assert_eq!(req.method, Method::Options);
                assert_eq!(req.uri.to_string(), "sip:user@example.com");
                
                // Check Via headers - we should have 5 of them with different transports
                let via_headers = req.via_headers();
                assert_eq!(via_headers.len(), 5, "Should have 5 Via headers");
                
                // Verify the transports from the Via headers
                if !via_headers.is_empty() {
                    let transports: Vec<&str> = via_headers.iter()
                        .flat_map(|v| v.0.iter())
                        .map(|vh| vh.sent_protocol.transport.as_str())
                        .collect();
                    
                    println!("Found transports: {:?}", transports);
                    
                    // Verify we have all the expected transports
                    assert!(transports.contains(&"UDP"), "Missing UDP transport");
                    assert!(transports.contains(&"SCTP"), "Missing SCTP transport");
                    assert!(transports.contains(&"TLS"), "Missing TLS transport");
                    assert!(transports.contains(&"UNKNOWN"), "Missing UNKNOWN transport");
                    assert!(transports.contains(&"TCP"), "Missing TCP transport");
                }
                
                // Check compact header name ('l' for Content-Length)
                let content_length = req.header(&HeaderName::ContentLength)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(999); // Default to a non-zero value to detect issues
                assert_eq!(content_length, 0, "Content-Length should be 0");
            },
            _ => panic!("Expected Request but got Response")
        }
    }
}

// Add a test for a SIP message with compact header forms
#[test]
fn test_parse_compact_headers() {
    // This is the content of 3.1.1.8_dblreq.sip from the RFC compliance tests
    // Test a request with compact header forms (I for Call-ID)
    let message = "REGISTER sip:example.com SIP/2.0\r\n\
To: sip:j.user@example.com\r\n\
From: sip:j.user@example.com;tag=43251j3j324\r\n\
Max-Forwards: 8\r\n\
I: dblreq.0ha0isndaksdj99sdfafnl3lk233412\r\n\
Contact: sip:j.user@host.example.com\r\n\
CSeq: 8 REGISTER\r\n\
Via: SIP/2.0/UDP 192.0.2.125;branch=z9hG4bKkdjuw23492\r\n\
Content-Length: 0\r\n\
\r\n";

    // Now try parsing the whole message
    println!("\n--- Attempting to parse message with compact headers ---");
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
    
    assert!(result.is_ok(), "Failed to parse wellformed message with compact headers");
    
    // If it parsed successfully, verify the main components
    if let Ok(parsed_msg) = result {
        match parsed_msg {
            Message::Request(req) => {
                assert_eq!(req.method, Method::Register);
                assert_eq!(req.uri.to_string(), "sip:example.com");
                
                // Check the Call-ID header (compact form 'I')
                let call_id = req.header(&HeaderName::CallId)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::CallId(cid) = h { Some(cid) } else { None });
                
                assert!(call_id.is_some(), "Call-ID header not found despite using compact form 'I'");
                if let Some(cid) = call_id {
                    println!("Found Call-ID: {}", cid.0);
                    assert_eq!(cid.0, "dblreq.0ha0isndaksdj99sdfafnl3lk233412");
                }
                
                // Check Max-Forwards
                let max_forwards = req.header(&HeaderName::MaxForwards)
                    .and_then(|h| if let rvoip_sip_core::types::TypedHeader::MaxForwards(mf) = h { Some(mf.0) } else { None })
                    .unwrap_or(0);
                
                assert_eq!(max_forwards, 8, "Max-Forwards should be 8");
            },
            _ => panic!("Expected Request but got Response")
        }
    }
} 