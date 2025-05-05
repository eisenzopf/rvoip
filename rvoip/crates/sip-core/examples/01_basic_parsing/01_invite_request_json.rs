//! Example 1: Parsing a SIP INVITE request (JSON accessor version)
//! 
//! This example demonstrates how to parse a basic SIP INVITE request
//! and extract common header fields using JSON accessors.

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
    
    info!("Example 1: Parsing a SIP INVITE request (JSON accessor version)");
    
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
        
        // ---------- Using JSON accessors ----------
        info!("\n---------- Using JSON accessors ----------");
        
        // Extract basic information using the simpler path API
        let method = request.path("method")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        info!("Method (JSON): {}", method);
        
        // Explore the headers to find their structure
        if let Some(headers) = request.path("headers") {
            if let Some(headers_array) = headers.as_array() {
                info!("Found {} headers in the message", headers_array.len());
                
                // Print all header names to help understand the structure
                for (i, header) in headers_array.iter().enumerate() {
                    if let Some(obj) = header.as_object() {
                        let unknown = String::from("unknown");
                        let header_name = obj.keys().next().unwrap_or(&unknown);
                        info!("Header at index {}: {}", i, header_name);
                    }
                }
            }
        }

        // ---------- Direct header access (enhanced path parser) ----------
        info!("\n---------- Direct header access (enhanced path parser) ----------");
        
        // With the enhanced path parser, we can now use these simpler paths:
        
        // Extract From header information
        let from_display_name = request.path("headers.From.display_name")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let from_user = request.path("headers.From.uri.user")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let from_host = request.path("headers.From.uri.host.Domain")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let from_tag = request.path("headers.From.params[0].Tag")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("From (JSON): {} <sip:{}@{}>", from_display_name, from_user, from_host);
        info!("From tag (JSON): {}", from_tag);
        
        // Extract To header information
        let to_display_name = request.path("headers.To.display_name")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let to_user = request.path("headers.To.uri.user")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let to_host = request.path("headers.To.uri.host.Domain")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("To (JSON): {} <sip:{}@{}>", to_display_name, to_user, to_host);
        
        // Extract Via header information
        let via_host = request.path("headers.Via[0].sent_by_host.Domain")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let via_transport = request.path("headers.Via[0].sent_protocol.transport")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let via_branch = request.path("headers.Via[0].params[0].Branch")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("Via (JSON): {}; transport={}; branch={}", via_host, via_transport, via_branch);
        
        // Extract Contact header information
        let contact_user = request.path("headers.Contact[0].Params[0].address.uri.user")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
            
        let contact_host = request.path("headers.Contact[0].Params[0].address.uri.host.Domain")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("Contact (JSON): <sip:{}@{}>", contact_user, contact_host);
        
        // Extra: Example of accessing other headers directly
        let call_id = request.path("headers.CallId.value")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| String::from("(none)"));
        info!("Call-ID: {}", call_id);
        
        // ---------- JSON Query Example ----------
        info!("\n---------- JSON Query Example ----------");
        // Let's find all params with a Tag in them using JSONPath query
        info!("Using JSON query to find tags:");
        let tags = request.query("$..Tag");
        for tag in tags {
            info!("Found tag: {}", tag);
        }
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