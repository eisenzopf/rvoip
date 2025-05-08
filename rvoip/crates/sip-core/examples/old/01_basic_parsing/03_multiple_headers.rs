//! Example 3: Parsing a message with multiple headers of the same type
//! 
//! This example demonstrates how to parse and access a SIP message
//! that contains multiple instances of the same header type.

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
    
    info!("Example 3: Parsing a message with multiple headers");
    
    // Raw SIP message with multiple Record-Route headers
    let data = Bytes::from(
        "INVITE sip:bob@example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
         Max-Forwards: 70\r\n\
         To: Bob <sip:bob@example.com>\r\n\
         From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
         Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
         CSeq: 314159 INVITE\r\n\
         Record-Route: <sip:proxy1.example.com;lr>\r\n\
         Record-Route: <sip:proxy2.example.com;lr>\r\n\
         Record-Route: <sip:proxy3.example.com;lr>\r\n\
         Contact: <sip:alice@pc33.atlanta.com>\r\n\
         Content-Length: 0\r\n\r\n"
    );
    
    // Parse the message
    let message = parse_message(&data).expect("Failed to parse message");
    
    if let Message::Request(request) = message {
        // Get all Record-Route headers
        let record_routes = request.typed_headers::<RecordRoute>();
        info!("Number of Record-Route headers: {}", record_routes.len());
        
        // Display each Record-Route header
        for (i, rr) in record_routes.iter().enumerate() {
            // For each RecordRouteEntry in the RecordRoute
            for (j, entry) in rr.iter().enumerate() {
                info!("Record-Route {}.{}: {}", i + 1, j + 1, entry.uri());
                
                // Check for the 'lr' parameter (loose routing)
                if entry.is_loose_routing() {
                    info!("  Uses loose routing (has 'lr' parameter)");
                }
            }
        }
        
        // Demonstrate how to access headers by name (string) instead of type
        let headers = request.headers_by_name("record-route");
        info!("Found {} Record-Route headers by name", headers.len());
        
        // Access headers using the raw API
        let raw_headers = request.all_headers();
        info!("Total number of headers: {}", raw_headers.len());
    }
} 