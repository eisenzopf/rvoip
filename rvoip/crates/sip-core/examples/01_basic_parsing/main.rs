//! Basic SIP Message Parsing Example
//! 
//! This example demonstrates how to parse raw SIP messages into structured types
//! and how to access the various components of a SIP message.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::headers::HeaderAccess;
use tracing::info;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Basic Parsing Example");
    
    // Example 1: Parse a simple SIP INVITE request
    parse_invite_request();
    
    // Example 2: Parse a SIP response
    parse_sip_response();
    
    // Example 3: Parse a message with multiple headers of the same type
    parse_multiple_headers();
    
    info!("All examples completed successfully!");
}

/// Example 1: Parse a SIP INVITE request
fn parse_invite_request() {
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

/// Example 2: Parse a SIP response
fn parse_sip_response() {
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

/// Example 3: Parse a message with multiple headers of the same type
fn parse_multiple_headers() {
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