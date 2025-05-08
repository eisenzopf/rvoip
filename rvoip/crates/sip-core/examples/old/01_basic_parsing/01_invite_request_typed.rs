//! # Example 1: Parsing a SIP INVITE request (Using Typed Accessors)
//! 
//! This example demonstrates how to parse a basic SIP INVITE request
//! and extract common header fields using the typed header accessor API.
//!
//! ## Typed Accessor Approach
//! 
//! The typed accessor approach uses Rust's type system to provide the safest
//! and most robust method for working with SIP message headers:
//!
//! ### Advantages:
//! 
//! - **Type Safety**: Compile-time checks ensure you're working with the correct header types
//! - **IDE Integration**: Autocomplete and tooltips in your IDE help discover available methods
//! - **Error Handling**: Type-safe results prevent runtime type errors
//! - **Discoverability**: Method names clearly indicate what you're accessing
//! - **Maintainability**: Refactoring tools work better with typed code
//!
//! ### When To Use Typed Accessors:
//!
//! - For production code where stability is critical
//! - When you need compile-time guarantees about header types
//! - When you want to leverage Rust's type system for SIP message handling
//! - For most performance-critical code paths
//!
//! ### Usage Pattern:
//!
//! ```
//! let from = request.typed_header::<From>().expect("From header");
//! let display_name = from.address().display_name();
//! let tag = from.tag();
//! ```
//!
//! This approach may be more verbose, but it provides the strongest guarantees.

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
    
    info!("Example 1: Parsing a SIP INVITE request (Typed Headers)");
    
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
        // This approach provides type safety and better IDE integration
        let from = request.typed_header::<From>().expect("From header");
        let to = request.typed_header::<To>().expect("To header");
        let contact = request.typed_header::<Contact>().expect("Contact header");
        
        // Display information about these headers
        // The typed API provides clear accessors for common fields
        info!("From: {} <{}>", from.address().display_name().unwrap_or(""), from.address().uri());
        info!("To: {} <{}>", to.address().display_name().unwrap_or(""), to.address().uri());
        info!("Contact: {}", contact.address().map_or("none".to_string(), |addr| addr.uri().to_string()));
        
        // Get the tag parameter from the From header
        // Type-safe accessors make it clear what you're getting
        if let Some(tag) = from.tag() {
            info!("From tag: {}", tag);
        }
        
        // Get the branch parameter from the Via header
        let via = request.typed_header::<Via>().expect("Via header");
        if let Some(branch) = via.branch() {
            info!("Branch: {}", branch);
        }
        
        // Other fields can be accessed in a similar way
        // For example, getting the CSeq header:
        if let Some(cseq) = request.typed_header::<CSeq>() {
            info!("CSeq: {} {}", cseq.sequence(), cseq.method());
        }
    } else {
        panic!("Expected a request, got a response!");
    }
} 