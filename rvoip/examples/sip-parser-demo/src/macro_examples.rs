use rvoip_sip_core::{
    sip_request,
    sip_response,
    types::{
        Message, Method, StatusCode,
        uri::Uri,
    },
    error::Error,
    parse_message,
};

/// Demonstrates creating various SIP requests using the macros
pub fn demonstrate_sip_request_macros() {
    println!("\n=== SIP Request Macro Examples ===");
    
    // Basic INVITE example
    println!("\nBasic INVITE request:");
    let invite = sip_request! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: 1,
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
        contact: "sip:alice@alice.example.com",
        max_forwards: 70,
        content_type: "application/sdp",
        body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
    };
    
    match Message::Request(invite) {
        Message::Request(req) => {
            println!("  Method: {}", req.method);
            println!("  URI: {}", req.uri);
            
            // Get headers by name
            let from_header = find_header(&req.headers, "From");
            let to_header = find_header(&req.headers, "To");
            
            println!("  From: {}", from_header);
            println!("  To: {}", to_header);
            println!("  Body length: {} bytes", req.body.len());
        },
        _ => println!("Error: Not a request")
    }
    
    // REGISTER with more parameters
    println!("\nREGISTER request with parameters:");
    let register = sip_request! {
        method: Method::Register,
        uri: "sip:registrar.example.com",
        from: ("Alice", "sip:alice@example.com", tag = "reg-tag"),
        to: ("Alice", "sip:alice@example.com"),
        call_id: "register-1234@example.com",
        cseq: 1,
        via: ("192.168.1.2:5060", "UDP", branch = "z9hG4bK-reg"),
        contact: "sip:alice@192.168.1.2:5060",
        max_forwards: 70
    };
    
    match Message::Request(register) {
        Message::Request(req) => {
            println!("  Method: {}", req.method);
            println!("  URI: {}", req.uri);
            
            // Get headers by name
            let from_header = find_header(&req.headers, "From");
            let via_header = find_header(&req.headers, "Via");
            
            println!("  From: {}", from_header);
            println!("  Via: {}", via_header);
        },
        _ => println!("Error: Not a request")
    }
    
    // OPTIONS request with custom headers
    println!("\nOPTIONS request with custom headers:");
    let custom_header_request = sip_request! {
        method: Method::Options,
        uri: "*",
        from: ("System", "sip:system@example.com", tag = "sys-tag"),
        to: ("Server", "sip:server@example.com"),
        call_id: "options-4321@example.com",
        cseq: 100,
        via: ("system.example.com:5060", "TCP", branch = "z9hG4bK-opts"),
        max_forwards: 70,
        accept: "application/sdp"
    };
    
    match Message::Request(custom_header_request) {
        Message::Request(req) => {
            println!("  Method: {}", req.method);
            println!("  URI: {}", req.uri);
            
            // Get headers by name
            let accept_header = find_header(&req.headers, "Accept");
            let via_header = find_header(&req.headers, "Via");
            
            println!("  Accept: {}", accept_header);
            println!("  Sent via TCP: {}", via_header.contains("TCP"));
        },
        _ => println!("Error: Not a request")
    }
}

/// Demonstrates creating various SIP responses using the macros
pub fn demonstrate_sip_response_macros() {
    println!("\n=== SIP Response Macro Examples ===");
    
    // Basic 200 OK response
    println!("\nBasic 200 OK response:");
    let ok_response = sip_response! {
        status: StatusCode::Ok,
        reason: "OK",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: (1, Method::Invite),
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
        contact: "sip:bob@192.168.1.2",
        content_type: "application/sdp",
        body: "v=0\r\no=bob 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
    };
    
    match Message::Response(ok_response) {
        Message::Response(resp) => {
            println!("  Status: {} {}", 
                     match resp.status {
                         StatusCode::Ok => 200,
                         _ => 0,
                     },
                     resp.reason.as_deref().unwrap_or(""));
            
            // Get headers by name
            let from_header = find_header(&resp.headers, "From");
            let to_header = find_header(&resp.headers, "To");
            
            println!("  From: {}", from_header);
            println!("  To (with tag): {}", to_header);
            println!("  Body length: {} bytes", resp.body.len());
        },
        _ => println!("Error: Not a response")
    }
    
    // 180 Ringing response
    println!("\n180 Ringing response:");
    let ringing_response = sip_response! {
        status: StatusCode::Ringing,
        reason: "Ringing",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com", tag = "early-tag"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: (1, Method::Invite),
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds", received = "192.168.1.1"),
        contact: "sip:bob@192.168.1.2"
    };
    
    match Message::Response(ringing_response) {
        Message::Response(resp) => {
            println!("  Status: {} {}", 
                     match resp.status {
                         StatusCode::Ringing => 180,
                         _ => 0,
                     },
                     resp.reason.as_deref().unwrap_or(""));
            
            // Get headers by name
            let via_header = find_header(&resp.headers, "Via");
            
            println!("  Via (with received): {}", via_header);
        },
        _ => println!("Error: Not a response")
    }
    
    // 4xx Error response with custom headers
    println!("\n400 Bad Request response with custom headers:");
    let error_response = sip_response! {
        status: StatusCode::BadRequest,
        reason: "Bad Request",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com"),
        call_id: "error-123@example.com",
        cseq: (42, Method::Message),
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
        warning: "399 example.com \"Malformed Content-Type\""
    };
    
    match Message::Response(error_response) {
        Message::Response(resp) => {
            println!("  Status: {} {}", 
                     match resp.status {
                         StatusCode::BadRequest => 400,
                         _ => 0,
                     },
                     resp.reason.as_deref().unwrap_or(""));
            
            // Get headers by name
            let warning_header = find_header(&resp.headers, "Warning");
            
            println!("  Warning: {}", warning_header);
        },
        _ => println!("Error: Not a response")
    }
}

/// Tests if the various SIP macros produce valid parseable messages
pub fn test_macro_generated_messages() {
    println!("\n=== Testing Macro-Generated Messages ===");
    
    // Create a message with the sip_request macro
    let invite = sip_request! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: 1,
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
        contact: "sip:alice@alice.example.com",
        max_forwards: 70,
        content_type: "application/sdp",
        body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
    };
    
    // Convert to wire format (using a simple implementation)
    let message_str = message_to_string(&Message::Request(invite));
    
    // Try to parse it back
    match parse_message(message_str.as_bytes()) {
        Ok(_) => println!("✓ INVITE request successfully parsed back"),
        Err(e) => println!("✗ INVITE parsing error: {}", e)
    }
    
    // Test a SIP response
    let ok_response = sip_response! {
        status: StatusCode::Ok,
        reason: "OK",
        from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
        to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: (1, Method::Invite),
        via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
        contact: "sip:bob@192.168.1.2",
        content_type: "application/sdp",
        body: "v=0\r\no=bob 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
    };
    
    // Convert to wire format
    let response_str = message_to_string(&Message::Response(ok_response));
    
    // Try to parse it back
    match parse_message(response_str.as_bytes()) {
        Ok(_) => println!("✓ 200 OK response successfully parsed back"),
        Err(e) => println!("✗ 200 OK parsing error: {}", e)
    }
}

/// Helper function to find a header in a header list by name
fn find_header(headers: &[rvoip_sip_core::types::TypedHeader], header_name: &str) -> String {
    headers.iter()
        .find(|h| h.to_string().starts_with(&format!("{}:", header_name)))
        .map(|h| h.to_string())
        .unwrap_or_else(|| format!("{}: <not found>", header_name))
}

/// Simple wire format conversion (for demo/test purposes only)
fn message_to_string(message: &Message) -> String {
    match message {
        Message::Request(req) => {
            let mut result = format!("{} {} {}\r\n", req.method, req.uri, req.version);
            
            for header in &req.headers {
                result.push_str(&format!("{}\r\n", header));
            }
            
            result.push_str(&format!("Content-Length: {}\r\n\r\n", req.body.len()));
            
            if !req.body.is_empty() {
                result.push_str(&String::from_utf8_lossy(&req.body));
            }
            
            result
        },
        Message::Response(resp) => {
            let status_code = match resp.status {
                StatusCode::Ok => 200,
                StatusCode::Ringing => 180,
                StatusCode::BadRequest => 400,
                StatusCode::Trying => 100,
                _ => 200, // default
            };
            
            let mut result = format!("{} {} {}\r\n", 
                                   resp.version,
                                   status_code,
                                   resp.reason.as_deref().unwrap_or(""));
            
            for header in &resp.headers {
                result.push_str(&format!("{}\r\n", header));
            }
            
            result.push_str(&format!("Content-Length: {}\r\n\r\n", resp.body.len()));
            
            if !resp.body.is_empty() {
                result.push_str(&String::from_utf8_lossy(&resp.body));
            }
            
            result
        }
    }
} 