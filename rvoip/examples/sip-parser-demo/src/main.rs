use rvoip_sip_core::{
    parse_message,
    types::{
        Message, Param,
        header::{HeaderName, TypedHeader},
        contact::ContactValue,
    },
};

// Import our message builder module
mod message_builder;

fn main() {
    println!("SIP Parser Demo");
    println!("==============");

    // Part 1: Parsing examples
    println!("\n--- PART 1: PARSING EXAMPLES ---");

    // Example 1: Parse a basic SIP INVITE request
    println!("\nExample 1: Parsing a SIP INVITE request");
    parse_sip_request();

    // Example 2: Parse a SIP response
    println!("\nExample 2: Parsing a SIP response");
    parse_sip_response();

    // Example 3: Parse a complete SIP message (request or response)
    println!("\nExample 3: Parsing a complete SIP message");
    parse_full_message();

    // Example 4: Error handling
    println!("\nExample 4: Error handling for malformed messages");
    handle_parsing_errors();

    // Example 5: Working with headers
    println!("\nExample 5: Working with SIP headers");
    work_with_headers();

    // Part 2: Building examples
    println!("\n\n--- PART 2: BUILDING EXAMPLES ---");

    // Example 6: Building a SIP INVITE request
    println!("\nExample 6: Building a SIP INVITE request");
    build_and_parse_invite();

    // Example 7: Building a SIP response based on a request
    println!("\nExample 7: Building a SIP response based on a request");
    build_and_parse_response();

    // Example 8: Building a SIP REGISTER request
    println!("\nExample 8: Building a SIP REGISTER request");
    build_and_parse_register();
}

fn parse_sip_request() {
    // A simple SIP INVITE request
    let request_str = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 0\r\n\
\r\n";

    // Parse the request
    match parse_message(request_str.as_bytes()) {
        Ok(message) => {
            println!("  Successfully parsed a SIP request!");
            
            // Check if it's a request
            if let Message::Request(request) = &message {
                println!("  Method: {}", request.method);
                println!("  URI: {}", request.uri);
                println!("  Version: {}", request.version);
                println!("  Headers: {} headers found", request.headers.len());

                // Get specific headers
                if let Some(TypedHeader::From(from)) = request.header(&HeaderName::From) {
                    println!("  From: {}", from);
                }

                if let Some(TypedHeader::To(to)) = request.header(&HeaderName::To) {
                    println!("  To: {}", to);
                }

                if let Some(TypedHeader::Contact(contact)) = request.header(&HeaderName::Contact) {
                    println!("  Contact: {}", contact);
                }
            } else {
                println!("  Unexpected: Parsed as response instead of request!");
            }
        },
        Err(e) => {
            println!("  Failed to parse SIP request: {}", e);
        }
    }
}

fn parse_sip_response() {
    // A simple SIP 200 OK response
    let response_str = "\
SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP server10.biloxi.example.com;branch=z9hG4bK4b43c2ff8.1\r\n\
Via: SIP/2.0/UDP bigbox3.site3.atlanta.example.com;branch=z9hG4bK77ef4c2312983.1\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds;received=192.0.2.1\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=a6c85cf\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:bob@192.0.2.4>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 0\r\n\
\r\n";

    // Parse the response
    match parse_message(response_str.as_bytes()) {
        Ok(message) => {
            println!("  Successfully parsed a SIP response!");
            
            if let Message::Response(response) = &message {
                println!("  Status-Code: {}", response.status);
                println!("  Reason-Phrase: {}", response.reason.as_deref().unwrap_or(""));
                println!("  Version: {}", response.version);
                println!("  Headers: {} headers found", response.headers.len());
                
                // Show hop count using Via headers
                let via_headers = response.headers.iter()
                    .filter(|h| h.to_string().starts_with("Via:"))
                    .count();
                println!("  Number of hops (Via headers): {}", via_headers);
            } else {
                println!("  Unexpected: Parsed as request instead of response!");
            }
        },
        Err(e) => {
            println!("  Failed to parse SIP response: {}", e);
        }
    }
}

fn parse_full_message() {
    // Can parse either a request or response
    let message_str = "\
REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bK-74bf9\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@example.com>\r\n\
From: Bob <sip:bob@example.com>;tag=456248\r\n\
Call-ID: 843817637684230@998sdasdh09\r\n\
CSeq: 1826 REGISTER\r\n\
Contact: <sip:bob@192.0.2.1>\r\n\
Expires: 7200\r\n\
Content-Length: 0\r\n\
\r\n";

    // Parse as a generic SIP message
    match parse_message(message_str.as_bytes()) {
        Ok(message) => {
            println!("  Successfully parsed a SIP message!");
            
            // Check if it's a request or a response
            match &message {
                Message::Request(request) => {
                    println!("  Message type: Request");
                    println!("  Method: {}", request.method);
                    println!("  Request-URI: {}", request.uri);
                } 
                Message::Response(response) => {
                    println!("  Message type: Response");
                    println!("  Status-Code: {}", response.status);
                    println!("  Reason-Phrase: {}", response.reason.as_deref().unwrap_or(""));
                }
            }
            
            // Get Call-ID and CSeq using typed headers
            if let Message::Request(request) = &message {
                if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
                    println!("  Call-ID: {}", call_id);
                }
                
                if let Some(TypedHeader::CSeq(cseq)) = request.header(&HeaderName::CSeq) {
                    println!("  CSeq: {} {}", cseq.seq, cseq.method);
                }
                
                // Check for presence of extension headers
                let has_expires = request.headers.iter()
                    .any(|h| h.to_string().starts_with("Expires:"));
                println!("  Has Expires header: {}", has_expires);
            }
        },
        Err(e) => {
            println!("  Failed to parse SIP message: {}", e);
        }
    }
}

fn handle_parsing_errors() {
    // Example 1: Missing required headers
    let missing_headers = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Max-Forwards: 70\r\n\
Content-Length: 0\r\n\
\r\n";

    match parse_message(missing_headers.as_bytes()) {
        Ok(_) => println!("  Unexpectedly parsed an invalid message!"),
        Err(e) => println!("  Error handling example 1: {}", e)
    }

    // Example 2: Invalid SIP version
    let invalid_version = "\
INVITE sip:bob@biloxi.example.com SIP/3.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Content-Length: 0\r\n\
\r\n";

    match parse_message(invalid_version.as_bytes()) {
        Ok(_) => println!("  Parser accepts SIP/3.0 as a valid version"),
        Err(e) => println!("  Error handling example 2: {}", e)
    }

    // Example 3: Content-Length mismatch
    let content_length_mismatch = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Content-Length: 100\r\n\
\r\n\
This body is shorter than 100 bytes!";

    match parse_message(content_length_mismatch.as_bytes()) {
        Ok(_) => println!("  Parser accepts Content-Length mismatch in lenient mode"),
        Err(e) => println!("  Error handling example 3: {}", e)
    }
}

fn work_with_headers() {
    let message_str = "\
SIP/2.0 180 Ringing\r\n\
Via: SIP/2.0/UDP client.atlanta.example.com:5060;branch=z9hG4bK74bf9\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=9fxced76sl\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=8321234356\r\n\
Call-ID: 3848276298220188511@atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:bob@192.0.2.4>\r\n\
User-Agent: SoftServer/1.0\r\n\
Content-Length: 0\r\n\
\r\n";

    if let Ok(message) = parse_message(message_str.as_bytes()) {
        // Method 1: Work with raw headers as strings
        println!("  Method 1: Working with raw headers");
        match &message {
            Message::Request(req) => {
                for header in &req.headers {
                    println!("    {}", header);
                }
            },
            Message::Response(resp) => {
                for header in &resp.headers {
                    println!("    {}", header);
                }
            }
        }

        // Method 2: Get headers by name
        println!("\n  Method 2: Get headers by name");
        if let Message::Response(resp) = &message {
            if let Some(ua_header) = resp.header(&HeaderName::UserAgent) {
                println!("    User-Agent: {}", ua_header);
            }
        }

        // Method 3: Get typed headers
        println!("\n  Method 3: Get typed headers");
        if let Message::Response(resp) = &message {
            if let Some(TypedHeader::Contact(contact)) = resp.header(&HeaderName::Contact) {
                // Contact is a list of addresses
                if !contact.0.is_empty() {
                    if let ContactValue::Params(params) = &contact.0[0] {
                        println!("    Contact URI: {}", params[0].address.uri);
                    }
                }
            }

            if let Some(TypedHeader::From(from)) = resp.header(&HeaderName::From) {
                println!("    From URI: {}", from.0.uri);
                
                // Get tag parameter from params
                let tag = from.0.params.iter()
                    .find_map(|p| if let Param::Tag(tag) = p { Some(tag) } else { None });
                
                println!("    From tag: {}", tag.unwrap_or(&String::from("None")));
                
                if let Some(display_name) = &from.0.display_name {
                    println!("    From display name: {}", display_name);
                }
            }
        }
    }
}

fn build_and_parse_invite() {
    match message_builder::build_invite_request() {
        Ok(message) => {
            // Convert to wire format
            let message_str = message_builder::message_to_string(&message);
            
            // Print out the wire format message
            println!("  Generated INVITE request:");
            println!("  -----------------------");
            
            // Print with line numbers for clarity
            for (i, line) in message_str.lines().enumerate() {
                println!("  {:2}: {}", i+1, line);
            }
            
            // Now parse it back to verify it's valid
            match parse_message(message_str.as_bytes()) {
                Ok(parsed_message) => {
                    println!("\n  Successfully parsed back the generated message!");
                    
                    if let Message::Request(req) = &parsed_message {
                        println!("  Method: {}", req.method);
                        println!("  URI: {}", req.uri);
                        println!("  Body length: {} bytes", req.body.len());
                    }
                },
                Err(e) => {
                    println!("\n  Failed to parse the generated message: {}", e);
                }
            }
        },
        Err(e) => {
            println!("  Failed to build INVITE request: {}", e);
        }
    }
}

fn build_and_parse_response() {
    // First build an INVITE request
    match message_builder::build_invite_request() {
        Ok(invite) => {
            // Now build a 200 OK response to it
            match message_builder::build_200_ok_response(&invite) {
                Ok(response) => {
                    // Convert to wire format
                    let response_str = message_builder::message_to_string(&response);
                    
                    // Print out the wire format message
                    println!("  Generated 200 OK response:");
                    println!("  ------------------------");
                    
                    // Print with line numbers for clarity
                    for (i, line) in response_str.lines().enumerate() {
                        println!("  {:2}: {}", i+1, line);
                    }
                    
                    // Now parse it back to verify it's valid
                    match parse_message(response_str.as_bytes()) {
                        Ok(parsed_message) => {
                            println!("\n  Successfully parsed back the generated response!");
                            
                            if let Message::Response(resp) = &parsed_message {
                                // Format status code appropriately
                                let status_code = match resp.status {
                                    rvoip_sip_core::types::StatusCode::Ok => 200,
                                    _ => 0,
                                };
                                
                                println!("  Status: {} {}", 
                                    status_code,
                                    resp.reason.as_deref().unwrap_or(""));
                                println!("  Headers: {} headers found", resp.headers.len());
                                println!("  Body length: {} bytes", resp.body.len());
                            }
                        },
                        Err(e) => {
                            println!("\n  Failed to parse the generated response: {}", e);
                        }
                    }
                },
                Err(e) => {
                    println!("  Failed to build 200 OK response: {}", e);
                }
            }
        },
        Err(e) => {
            println!("  Failed to build INVITE request: {}", e);
        }
    }
}

fn build_and_parse_register() {
    match message_builder::build_register_request() {
        Ok(message) => {
            // Convert to wire format
            let message_str = message_builder::message_to_string(&message);
            
            // Print out the wire format message
            println!("  Generated REGISTER request:");
            println!("  --------------------------");
            
            // Print with line numbers for clarity
            for (i, line) in message_str.lines().enumerate() {
                println!("  {:2}: {}", i+1, line);
            }
            
            // Now parse it back to verify it's valid
            match parse_message(message_str.as_bytes()) {
                Ok(parsed_message) => {
                    println!("\n  Successfully parsed back the generated message!");
                    
                    if let Message::Request(req) = &parsed_message {
                        println!("  Method: {}", req.method);
                        println!("  URI: {}", req.uri);
                        
                        // Get the Expires header value
                        let expires_value = req.headers.iter()
                            .find(|h| h.to_string().starts_with("Expires:"))
                            .map_or("None".to_string(), |h| {
                                let header_str = h.to_string();
                                let parts: Vec<&str> = header_str.splitn(2, ": ").collect();
                                if parts.len() >= 2 {
                                    parts[1].to_string()
                                } else {
                                    "None".to_string()
                                }
                            });
                            
                        println!("  Expires: {}", expires_value);
                    }
                },
                Err(e) => {
                    println!("\n  Failed to parse the generated message: {}", e);
                }
            }
        },
        Err(e) => {
            println!("  Failed to build REGISTER request: {}", e);
        }
    }
} 