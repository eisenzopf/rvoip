//! Example 1: Parsing a SIP INVITE request (JSON path accessor version)
//! 
//! This example demonstrates how to parse a basic SIP INVITE request
//! and extract common header fields using JSON path accessors.
//!
//! There are two main path accessor methods:
//! 
//! 1. path() - Returns Option<SipValue>
//!    - Preserves the original type (number, bool, object, etc.)
//!    - You must manually handle the Option and conversion
//!    - Use when you need the original type (not just a string)
//!    - Use when you need to check if a path exists
//!
//! 2. path_str_or() - Returns String
//!    - Automatically converts to string
//!    - Takes a default value to use if path not found
//!    - More concise for simple string values (most common case)
//!    - Use for simple string access with default values
//!    - Works with all value types (strings, numbers, booleans, etc.)

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
        // Get basic information about the request using native methods
        info!("Method: {}", request.method());
        info!("URI: {}", request.uri());
        info!("SIP Version: {}", request.version());
        
        // ---------- Comparing path() and path_str_or() ----------
        info!("\n---------- Comparing path() and path_str_or() ----------");
        
        // Using path() - Returns Option<SipValue>, preserves type
        info!("\n----- Using path() -----");
        
        // Method
        match request.path("method") {
            Some(val) => info!("Method (path): {}", val),
            None => info!("Method not found"),
        }
        
        // From display name
        match request.path("headers.From.display_name") {
            Some(val) => info!("From display name (path): {}", val),
            None => info!("From display name not found"),
        }
        
        // CSeq (numeric value)
        match request.path("headers.CSeq.seq") {
            Some(val) => {
                // Debug: Print the type of the value
                info!("CSeq number debug: is_number={}, is_string={}, raw={:?}", 
                      val.is_number(), val.is_string(), val);
                
                // Convert to number and handle type appropriately
                if let Some(num) = val.as_i64() {
                    info!("CSeq number (path): {} (numeric value)", num);
                } else {
                    info!("CSeq number found but not a number: {}", val);
                }
            },
            None => info!("CSeq number not found"),
        }
        
        // Using path_str_or() - Returns String directly with default
        info!("\n----- Using path_str_or() -----");
        
        // Method - Same field as above but with path_str_or
        let method = request.path_str_or("method", "(none)");
        info!("Method (path_str_or): {}", method);
        
        // From display name - Same field as above but with path_str_or
        let from_display = request.path_str_or("headers.From.display_name", "(unknown)");
        info!("From display name (path_str_or): {}", from_display);
        
        // CSeq - Same field as above but with path_str_or
        // path_str_or handles all value types including numbers
        let cseq_num_str = request.path_str_or("headers.CSeq.seq", "0");
        info!("CSeq number (path_str_or): {} (converted to string)", cseq_num_str);
        
        // Alternative approach for numeric values - use path() when you need the original numeric type
        let cseq_num_as_number = match request.path("headers.CSeq.seq") {
            Some(val) => val.as_i64().unwrap_or(0),
            None => 0,
        };
        info!("CSeq number (as numeric type): {} (can perform arithmetic)", cseq_num_as_number);
        
        // ---------- Full SIP message example with path_str_or ----------
        info!("\n---------- Full SIP message example (one-liners) ----------");
        
        // From header - using path_str_or for direct, concise access
        let from_display = request.path_str_or("headers.From.display_name", "(unknown)");
        let from_uri = format!("sip:{}@{}", 
            request.path_str_or("headers.From.uri.user", "unknown"),
            request.path_str_or("headers.From.uri.host.Domain", "unknown"));
        let from_tag = request.path_str_or("headers.From.params[0].Tag", "(none)");
        
        info!("From: {} <{}>; tag={}", from_display, from_uri, from_tag);
        
        // To header
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
        
        // Call-ID
        info!("Call-ID: {}", request.path_str_or("headers.CallId", "(none)"));
        
        // CSeq - Special case with type conversion
        // For CSeq, we want to handle the numeric value properly
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