use std::fs;
use rvoip_sip_core::{parse_message, error::Error};

// Single file testing for debugging
#[test]
fn debug_parse_longreq() {
    let filepath = "tests/rfc_compliance/wellformed/3.1.1.7_longreq.sip";
    let content = fs::read_to_string(filepath).expect("Failed to read file");
    
    println!("File content:\n{}", content);
    
    // Add some processing to ensure the content is properly formed
    let normalized = content.replace("\r\n", "\n").replace("\n", "\r\n");
    println!("\nNormalized content:\n{}", normalized);
    
    // Try to parse the message
    match parse_message(normalized.as_bytes()) {
        Ok(message) => {
            println!("\nSuccessfully parsed message:");
            println!("Type: {:?}", if message.is_request() { "Request" } else { "Response" });
            
            // Check headers
            match message {
                rvoip_sip_core::types::Message::Request(req) => {
                    println!("Method: {:?}", req.method);
                    println!("URI: {}", req.uri);
                    
                    // Print all headers
                    println!("\nHeaders:");
                    for header in &req.headers {
                        println!("  {}: {}", header.name(), header);
                    }
                    
                    // Check if specific headers exist
                    println!("\nRequired Headers:");
                    println!("  From: {}", if req.header(&rvoip_sip_core::types::header::HeaderName::From).is_some() { "Present" } else { "Missing" });
                    println!("  To: {}", if req.header(&rvoip_sip_core::types::header::HeaderName::To).is_some() { "Present" } else { "Missing" });
                    println!("  Call-ID: {}", if req.header(&rvoip_sip_core::types::header::HeaderName::CallId).is_some() { "Present" } else { "Missing" });
                    println!("  CSeq: {}", if req.header(&rvoip_sip_core::types::header::HeaderName::CSeq).is_some() { "Present" } else { "Missing" });
                },
                rvoip_sip_core::types::Message::Response(resp) => {
                    println!("Status Code: {:?}", resp.status);
                    println!("Reason: {}", resp.reason_phrase());
                    
                    // Print all headers
                    println!("\nHeaders:");
                    for header in &resp.headers {
                        println!("  {}: {}", header.name(), header);
                    }
                }
            }
        },
        Err(e) => {
            println!("\nParsing error: {:?}", e);
            
            // More detailed error information
            match e {
                Error::ParseError(msg) => {
                    println!("Parse error message: {}", msg);
                },
                Error::InvalidFormat(msg) => {
                    println!("Invalid format: {}", msg);
                },
                _ => {
                    println!("Other error type: {:?}", e);
                }
            }
        }
    }
} 