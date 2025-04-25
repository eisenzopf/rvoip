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
    let message = "SIP/2.0 200 = 2**3 * 5**2 но сто девяносто девять - простое
Via: SIP/2.0/UDP 192.0.2.198;branch=z9hG4bK1324923
Call-ID: unreason.1234ksdfak3j2erwedfsASdf
CSeq: 35 INVITE
From: sip:user@example.com;tag=11141343
To: sip:user@example.edu;tag=2229
Content-Length: 162
Content-Type: application/sdp
Contact: <sip:user@host198.example.com>

v=0
o=mhandley 29739 7272939 IN IP4 192.0.2.198
s=-
c=IN IP4 192.0.2.198
t=0 0
m=audio 49217 RTP/AVP 0 12
m=video 3227 RTP/AVP 31
a=rtpmap:31 LPC";

    // Use the parse_message function from the crate root
    let result = parse_message(message.as_bytes());
    
    // Print detailed debug info on the parse result
    match &result {
        Ok(msg) => println!("Successfully parsed message: {:?}", msg),
        Err(e) => println!("Parse error: {}", e)
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
                assert_eq!(content_length, 162);
                assert!(!resp.body.is_empty());
            },
            _ => panic!("Expected Response but got Request")
        }
    }
}

#[test]
fn test_parse_basic_invite() {
    // Test a basic INVITE request
    let message = "INVITE sip:bob@biloxi.com SIP/2.0
Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds
Max-Forwards: 70
To: Bob <sip:bob@biloxi.com>
From: Alice <sip:alice@atlanta.com>;tag=1928301774
Call-ID: a84b4c76e66710@pc33.atlanta.com
CSeq: 314159 INVITE
Contact: <sip:alice@pc33.atlanta.com>
Content-Type: application/sdp
Content-Length: 142

v=0
o=alice 2890844526 2890844526 IN IP4 pc33.atlanta.com
s=Session SDP
c=IN IP4 pc33.atlanta.com
t=0 0
m=audio 49172 RTP/AVP 0
a=rtpmap:0 PCMU/8000";

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
                assert_eq!(content_length, 142);
                assert!(!req.body.is_empty());
            },
            _ => panic!("Expected Request but got Response")
        }
    }
} 