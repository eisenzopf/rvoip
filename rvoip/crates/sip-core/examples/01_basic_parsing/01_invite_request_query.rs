//! Example: Parsing a SIP INVITE request (Query-based version)
//! 
//! This example demonstrates how to parse a SIP INVITE request
//! and extract fields using JSONPath-like queries.

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
    
    info!("Example: Parsing a SIP INVITE request (Query-based version)");
    
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
        // Get basic information about the request (not using query)
        info!("Method: {}", request.method());
        info!("URI: {}", request.uri());
        info!("SIP Version: {}", request.version());
        
        // ---------- Using JSONQuery queries ----------
        info!("\n---------- Using JSONQuery queries ----------");
        
        // Basic queries for single values
        if let Some(val) = request.query("$.method").first() {
            info!("Method (query): {}", val);
        }
        
        // Finding all display names
        info!("\n----- Finding all display names -----");
        let display_names = request.query("$..display_name");
        for (i, name) in display_names.iter().enumerate() {
            info!("Display name {}: {}", i+1, name);
        }
        
        // Finding URI information
        info!("\n----- Finding URI information -----");
        // Use direct queries for user and host fields
        let users = request.query("$..uri.user");
        let hosts = request.query("$..uri.host.Domain");
        
        // Print each URI by matching up the users and hosts
        for i in 0..users.len().min(hosts.len()) {
            if let (Some(user_str), Some(host_str)) = (users[i].as_str(), hosts[i].as_str()) {
                info!("URI {}: sip:{}@{}", i+1, user_str, host_str);
            }
        }
        
        // Finding tags
        info!("\n----- Finding all tags -----");
        let tags = request.query("$..Tag");
        for (i, tag) in tags.iter().enumerate() {
            info!("Tag {}: {}", i+1, tag);
        }
        
        // Finding branch parameters
        info!("\n----- Finding all branch parameters -----");
        let branches = request.query("$..Branch");
        for (i, branch) in branches.iter().enumerate() {
            info!("Branch {}: {}", i+1, branch);
        }
        
        // Finding headers by type
        info!("\n----- Finding specific headers -----");
        
        // Find the CallId header
        if let Some(call_id) = request.query("$..CallId").first() {
            info!("Call-ID: {}", call_id);
        }
        
        // Find the CSeq header information
        // Query directly for the sequence number and method
        let seq_values = request.query("$..CSeq.seq");
        let method_values = request.query("$..CSeq.method");
        
        if let (Some(seq), Some(method)) = (seq_values.first(), method_values.first()) {
            info!("CSeq: {} {}", seq, method);
        }
        
        // Find all header types
        info!("\n----- Finding all header types -----");
        let headers = request.query("$.headers[*]");
        for header in headers {
            // Get the first key in each header object
            if let Some(obj) = header.as_object() {
                for key in obj.keys() {
                    info!("Header type: {}", key);
                }
            }
        }
    } else {
        panic!("Expected a request, got a response!");
    }
} 