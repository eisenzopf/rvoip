use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::headers::HeaderAccess;
use tracing::{info, Level};
use tracing_subscriber;

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();
    
    info!("Example 1: Creating a SIP message and converting to JSON");
    create_and_convert_to_json();
    
    info!("\nExample 2: Using path-based access to SIP message fields");
    path_based_access();
    
    info!("\nExample 3: Using query-based access for complex operations");
    query_based_access();
    
    info!("\nExample 4: JSON Round-trip conversion");
    json_round_trip();
}

/// Example 1: Creating a SIP message and converting to JSON
fn create_and_convert_to_json() {
    // Create a SIP request with multiple headers
    let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .max_forwards(70)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .via("proxy.atlanta.com", "TCP", Some("z9hG4bK776asdhds2"))
        .contact("sip:alice@pc33.atlanta.com", Some("Alice"))
        .build();
    
    // Convert to JSON and print
    match request.to_json_string_pretty() {
        Ok(json) => {
            info!("SIP request as JSON:\n{}", json);
        },
        Err(e) => {
            info!("Error converting to JSON: {:?}", e);
        }
    }
}

/// Example 2: Using path-based access to SIP message fields
fn path_based_access() {
    // Create a SIP response
    let response = ResponseBuilder::ok()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:alice@pc33.atlanta.com", Some("Alice"))
        .build();
    
    // Use path-based access to get specific fields
    info!("Accessing specific fields with path notation:");
    
    // Get the From display name
    let from_path = response.get_path("headers.from.display_name");
    let from_name = from_path.as_str().unwrap_or("Not found");
    info!("  From display name: {}", from_name);
    
    // Get the To tag
    let to_path = response.get_path("headers.to.tag");
    let to_tag = to_path.as_str().unwrap_or("Not found");
    info!("  To tag: {}", to_tag);
    
    // Get the first Via branch parameter
    let via_path = response.get_path("headers.via[0].branch");
    let via_branch = via_path.as_str().unwrap_or("Not found");
    info!("  First Via branch: {}", via_branch);
    
    // Get the status code
    let status_code = response.get_path("status_code").as_i64().unwrap_or(0);
    info!("  Status code: {}", status_code);
}

/// Example 3: Using query-based access for complex operations
fn query_based_access() {
    // Create a request with multiple Via headers
    let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .via("proxy.atlanta.com", "TCP", Some("z9hG4bK776asdhds2"))
        .via("edge.example.com", "TLS", Some("z9hG4bK776asdhds3"))
        .build();
    
    // Query to get all Via branches
    let branches = request.query("$.headers.via[*].branch");
    info!("All Via branches:");
    for (i, branch) in branches.iter().enumerate() {
        if let Some(branch_str) = branch.as_str() {
            info!("  Branch {}: {}", i+1, branch_str);
        }
    }
    
    // Query to get all display names (using recursive descent)
    let display_names = request.query("$..display_name");
    info!("All display names:");
    for (i, name) in display_names.iter().enumerate() {
        if let Some(name_str) = name.as_str() {
            info!("  Name {}: {}", i+1, name_str);
        }
    }
    
    // Query to get the URI of the request
    let uri = request.query("$.uri");
    if let Some(first) = uri.first() {
        info!("Request URI: {}", first);
    }
}

/// Example 4: JSON Round-trip conversion
fn json_round_trip() {
    // Create an original request
    let original_request = RequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:alice@pc33.atlanta.com", Some("Alice"))
        .build();
    
    // Convert to JSON string
    let json_str = match original_request.to_json_string() {
        Ok(json) => {
            info!("Original request converted to JSON string");
            json
        },
        Err(e) => {
            info!("Error converting to JSON: {:?}", e);
            return;
        }
    };
    
    // Create new request from JSON string
    let new_request = match Request::from_json_str(&json_str) {
        Ok(req) => {
            info!("Successfully created new request from JSON");
            req
        },
        Err(e) => {
            info!("Error creating from JSON: {:?}", e);
            return;
        }
    };
    
    // Verify that the round-trip worked by checking some fields
    let original_method = original_request.method().to_string();
    let new_method = new_request.method().to_string();
    
    let original_uri = original_request.uri().to_string();
    let new_uri = new_request.uri().to_string();
    
    let original_from = original_request.typed_header::<From>()
        .map(|f| f.to_string()).unwrap_or_default();
    let new_from = new_request.typed_header::<From>()
        .map(|f| f.to_string()).unwrap_or_default();
    
    info!("Comparing original and round-tripped request:");
    info!("  Original method: {}, New method: {}", original_method, new_method);
    info!("  Original URI: {}, New URI: {}", original_uri, new_uri);
    info!("  Original From: {}, New From: {}", original_from, new_from);
} 