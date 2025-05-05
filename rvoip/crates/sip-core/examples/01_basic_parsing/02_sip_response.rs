//! Example 2: Parsing a SIP response
//! 
//! This example demonstrates how to parse a SIP response message
//! and access its status code, reason phrase, and headers.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::headers::HeaderAccess;
use tracing::info;

fn main() {
    // Initialize logging with a default filter level
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();
    
    info!("Example 2: Parsing a SIP response");
    
    // Raw SIP 200 OK response as bytes
    let data = Bytes::from(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP server10.example.com;branch=z9hG4bK4442ba5c;received=192.0.2.3\r\n\
         Via: SIP/2.0/UDP bigbox3.example.com;branch=z9hG4bK77ef4c2312983.1;received=192.0.2.2\r\n\
         Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds;received=192.0.2.1\r\n\
         To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
         From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
         Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
         CSeq: 314159 INVITE\r\n\
         Contact: <sip:bob@192.0.2.4>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: 0\r\n\r\n"
    );
    
    // Parse the message
    let message = parse_message(&data).expect("Failed to parse message");
    
    // Check if it's a response (it should be!)
    if let Message::Response(response) = message {
        // Get basic information about the response
        info!("Status Code: {}", response.status_code());
        info!("Reason Phrase: {}", response.reason_phrase());
        info!("SIP Version: {}", response.version());
        
        // Access the CSeq header to see what method this is responding to
        let cseq = response.typed_header::<CSeq>().expect("CSeq header");
        info!("Response to method: {}", cseq.method());
        
        // Get all Via headers (responses typically have multiple)
        let via_headers = response.typed_headers::<Via>();
        info!("Number of Via headers: {}", via_headers.len());
        
        // Display information about the top Via header
        if let Some(top_via) = via_headers.first() {
            info!("Top Via: {}", top_via);
            if let Some(received) = top_via.received() {
                info!("  Received parameter: {}", received);
            }
        }
        
        // Check if the To header has a tag (it should in a 200 OK)
        let to = response.typed_header::<To>().expect("To header");
        if let Some(tag) = to.tag() {
            info!("To tag: {}", tag);
        } else {
            info!("No tag in To header!");
        }
    } else {
        panic!("Expected a response, got a request!");
    }
} 