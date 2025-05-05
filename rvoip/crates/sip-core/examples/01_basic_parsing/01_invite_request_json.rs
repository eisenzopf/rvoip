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
        
        // Getting the method - store the value directly
        let method = request.path()
            .field("method")
            .value()
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        info!("Method (JSON): {}", method);
        
        // Getting From header values using specialized accessors
        // Notice we don't need to know the position - the from() method finds it for us
        let value = request.path().headers().from().display_name().value();
        let from_display_name = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().from().uri().field("user").value();
        let from_user = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().from().uri().field("host").field("Domain").value();
        let from_host = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().from().params().index(0).field("Tag").value();
        let from_tag = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("From (JSON): {} <sip:{}@{}>", from_display_name, from_user, from_host);
        info!("From tag (JSON): {}", from_tag);
        
        // Alternatively, we could access the tag directly using the tag() helper method:
        let value = request.path().headers().from().tag().value();
        let alt_from_tag = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        info!("From tag (alternative): {}", alt_from_tag);
        
        // Getting To header values using specialized accessors
        let value = request.path().headers().to().display_name().value();
        let to_display_name = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().to().uri().field("user").value();
        let to_user = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().to().uri().field("host").field("Domain").value();
        let to_host = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("To (JSON): {} <sip:{}@{}>", to_display_name, to_user, to_host);
        
        // Getting Via header values using specialized accessors
        let value = request.path().headers().via().index(0).field("sent_by_host").field("Domain").value();
        let via_host = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().via().index(0).field("sent_protocol").field("transport").value();
        let via_transport = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        // Using branch() helper method instead of manual path traversal
        let value = request.path().headers().via().index(0).branch().value();
        let via_branch = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("Via (JSON): {}; transport={}; branch={}", via_host, via_transport, via_branch);
        
        // Getting Contact header value using specialized accessors
        // Note: The Params casing might need adjustment based on your codebase structure
        let value = request.path().headers().field("Contact").index(0).field("Params").index(0).field("address").uri().field("user").value();
        let contact_user = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
            
        let value = request.path().headers().field("Contact").index(0).field("Params").index(0).field("address").uri().field("host").field("Domain").value();
        let contact_host = value.as_str()
            .map(String::from)
            .unwrap_or_else(|| String::from("(none)"));
        
        info!("Contact (JSON): <sip:{}@{}>", contact_user, contact_host);
        
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