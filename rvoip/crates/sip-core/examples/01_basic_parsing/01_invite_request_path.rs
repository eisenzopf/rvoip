//! Example 1: Parsing a SIP INVITE request (JSON path accessor version)
//! 
//! This example demonstrates how to parse a basic SIP INVITE request
//! and extract common header fields using JSON path accessors.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::json::SipJsonExt;  // Import the JSON extension trait
use tracing::info;

fn main() {
    // Initialize logging with a default filter level
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();
    
    info!("Example 1: Parsing a SIP INVITE request (JSON path accessor version)");
    
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
        
        // ---------- Using JSON path accessors ----------
        info!("\n---------- Using JSON path accessors ----------");
        
        // Extract basic information with our simpler API
        let method = request.path_str_or("method", "(none)");
        info!("Method (path): {}", method);
        
        // The most basic approach (using path_str_or directly):
        info!("\n----- Path-based access (one-liners) -----");
        
        // From header - using path_str_or for direct, concise access
        let from_display = request.path_str_or("headers.From.display_name", "(unknown)");
        let from_uri = format!("sip:{}@{}", 
            request.path_str_or("headers.From.uri.user", "unknown"),
            request.path_str_or("headers.From.uri.host.Domain", "unknown"));
        let from_tag = request.path_str_or("headers.From.params[0].Tag", "(none)");
        
        info!("From: {} <{}>; tag={}", from_display, from_uri, from_tag);
        
        // To header - even more concise
        info!("To: {} <sip:{}@{}>", 
            request.path_str_or("headers.To.display_name", "(unknown)"),
            request.path_str_or("headers.To.uri.user", "unknown"),
            request.path_str_or("headers.To.uri.host.Domain", "unknown"));
        
        // Via header
        info!("Via: SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via[0].sent_protocol.transport", "UDP"),
            request.path_str_or("headers.Via[0].sent_by_host.Domain", "unknown"),
            request.path_str_or("headers.Via[0].params[0].Branch", "unknown"));
        
        // Contact header
        info!("Contact: <sip:{}@{}>", 
            request.path_str_or("headers.Contact[0].Params[0].address.uri.user", "unknown"),
            request.path_str_or("headers.Contact[0].Params[0].address.uri.host.Domain", "unknown"));
        
        // Call-ID - fix the path to match the actual structure (CallId directly contains the value)
        info!("Call-ID: {}", request.path_str_or("headers.CallId", "(none)"));
        
        // CSeq - fix to match actual structure (seq instead of sequence_number)
        // Try to access as integer directly
        let cseq_num = match request.path("headers.CSeq.seq") {
            Some(val) => val.as_i64().unwrap_or(0).to_string(),
            None => "0".to_string(),
        };
        let cseq_method = request.path_str_or("headers.CSeq.method", "(none)");
        info!("CSeq: {} {}", cseq_num, cseq_method);
    } else {
        panic!("Expected a request, got a response!");
    }
}

// Note: With the enhanced path parser, we no longer need this helper function
// since we can access headers directly by name
//
// fn find_header_index<T: SipJsonExt>(sip_object: &T, header_name: &str) -> Option<usize> {
//     if let Some(headers) = sip_object.path("headers") {
//         if let Some(headers_array) = headers.as_array() {
//             for (i, header) in headers_array.iter().enumerate() {
//                 if let Some(obj) = header.as_object() {
//                     if obj.contains_key(header_name) {
//                         return Some(i);
//                     }
//                 }
//             }
//         }
//     }
//     None
// } 