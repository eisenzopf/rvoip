//! Example 1: Parsing a SIP INVITE request
//! 
//! This example demonstrates how to parse a basic SIP INVITE request
//! and extract common header fields.

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
    
    info!("Example 1: Parsing a SIP INVITE request");
    
    // Raw SIP INVITE message as bytes
    let data = Bytes::from(
        "INVITE sip:bob@example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
         Max-Forwards: 70\r\n\
         To: Bob <sip:bob@example.com>\r\n\
         From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
         Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
         CSeq: 314159 INVITE\r\n\
         Contact: <sip:alice@pc33.atlanta.com>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: 0\r\n\r\n"
    );
    
    // Parse the message
    let message = parse_message(&data).expect("Failed to parse message");
    
    // Check if it's a request (it should be!)
    if let Message::Request(request) = message {
        // Get basic information about the request
        info!("Method: {}", request.method());
        info!("URI: {}", request.uri());
        info!("SIP Version: {}", request.version());
        
        // Access some headers using the typed header API
        let from = request.typed_header::<From>().expect("From header");
        let to = request.typed_header::<To>().expect("To header");
        let contact = request.typed_header::<Contact>().expect("Contact header");
        
        // Display information about these headers
        info!("From: {} <{}>", from.address().display_name().unwrap_or(""), from.address().uri());
        info!("To: {} <{}>", to.address().display_name().unwrap_or(""), to.address().uri());
        info!("Contact: {}", contact.address().map_or("none".to_string(), |addr| addr.uri().to_string()));
        
        // Get the tag parameter from the From header
        if let Some(tag) = from.tag() {
            info!("From tag: {}", tag);
        }
        
        // Get the branch parameter from the Via header
        let via = request.typed_header::<Via>().expect("Via header");
        if let Some(branch) = via.branch() {
            info!("Branch: {}", branch);
        }
    } else {
        panic!("Expected a request, got a response!");
    }
} 