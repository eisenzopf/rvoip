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

    // First, let's print the JSON structure to understand it better
    let json_str = response.to_json_string_pretty().unwrap_or_default();
    info!("Response JSON structure:\n{}", json_str);
    
    // Since get_path doesn't work properly in the current implementation,
    // we'll first convert to a SipValue and then analyze the structure directly
    let value = response.to_sip_value().unwrap_or_default();
    
    if let Some(headers_array) = value.as_object().and_then(|obj| obj.get("headers")).and_then(|h| h.as_array()) {
        // Iterate through headers to find the From header
        for header in headers_array {
            if let Some(from) = header.as_object().and_then(|obj| obj.get("From")) {
                // Now get the display_name from the From header
                if let Some(display_name) = from.as_object().and_then(|obj| obj.get("display_name")) {
                    if let Some(name) = display_name.as_str() {
                        info!("  From display name: {}", name);
                    } else {
                        info!("  From display name: Not found (not a string)");
                    }
                } else {
                    info!("  From display name: Not found (no display_name field)");
                }
                break;
            }
        }
        
        // Iterate to find the To header and get the tag
        for header in headers_array {
            if let Some(to) = header.as_object().and_then(|obj| obj.get("To")) {
                // Look for the tag in the params array
                if let Some(params) = to.as_object().and_then(|obj| obj.get("params")).and_then(|p| p.as_array()) {
                    let mut found_tag = false;
                    for param in params {
                        if let Some(tag) = param.as_object().and_then(|obj| obj.get("Tag")) {
                            if let Some(tag_str) = tag.as_str() {
                                info!("  To tag: {}", tag_str);
                                found_tag = true;
                                break;
                            }
                        }
                    }
                    if !found_tag {
                        info!("  To tag: Not found (no Tag in params)");
                    }
                } else {
                    info!("  To tag: Not found (no params array)");
                }
                break;
            }
        }
        
        // Find the Via header and get the first branch
        for header in headers_array {
            if let Some(via) = header.as_object().and_then(|obj| obj.get("Via")) {
                if let Some(via_entries) = via.as_array() {
                    if via_entries.is_empty() {
                        info!("  First Via branch: Not found (empty Via array)");
                    } else {
                        let first_via = &via_entries[0];
                        if let Some(params) = first_via.as_object().and_then(|obj| obj.get("params")).and_then(|p| p.as_array()) {
                            let mut found_branch = false;
                            for param in params {
                                if let Some(branch) = param.as_object().and_then(|obj| obj.get("Branch")) {
                                    if let Some(branch_str) = branch.as_str() {
                                        info!("  First Via branch: {}", branch_str);
                                        found_branch = true;
                                        break;
                                    }
                                }
                            }
                            if !found_branch {
                                info!("  First Via branch: Not found (no Branch in params)");
                            }
                        } else {
                            info!("  First Via branch: Not found (no params array)");
                        }
                    }
                } else {
                    info!("  First Via branch: Not found (Via is not an array)");
                }
                break;
            }
        }
    }
    
    // Get the status code
    if let Some(status_code) = value.as_object().and_then(|obj| obj.get("status_code")) {
        if let Some(code) = status_code.as_i64() {
            info!("  Status code: {}", code);
        } else {
            info!("  Status code: Not found (not a number)");
        }
    } else {
        info!("  Status code: Not found (no status_code field)");
    }
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
    
    // Print the JSON structure to understand it better
    let json_str = request.to_json_string_pretty().unwrap_or_default();
    info!("Request JSON structure for query operations:\n{}", json_str);
    
    // Query to get all Via branches - we need to adjust the query based on the JSON structure
    // The Branch value is inside each Via header's params array
    let branches = request.query("$..Branch");
    info!("All Via branches:");
    for (i, branch) in branches.iter().enumerate() {
        if let Some(branch_str) = branch.as_str() {
            info!("  Branch {}: {}", i+1, branch_str);
        }
    }
    
    // Query to get all display names using recursive descent
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
        // We're extracting just the key parts of the URI to display
        if let Some(scheme) = first.as_object().and_then(|obj| obj.get("scheme")).and_then(|s| s.as_str()) {
            if let Some(user) = first.as_object().and_then(|obj| obj.get("user")).and_then(|u| u.as_str()) {
                if let Some(host) = first.as_object().and_then(|obj| obj.get("host")) {
                    if let Some(domain) = host.as_object().and_then(|obj| obj.get("Domain")).and_then(|d| d.as_str()) {
                        info!("  Request URI: {}:{}@{}", scheme.to_lowercase(), user, domain);
                    }
                }
            }
        } else {
            info!("  Request URI: {}", first);
        }
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