use rvoip_sip_core::prelude::*;
use rvoip_sip_core::json::SipJsonExt;  // Import the JSON extension trait
use rvoip_sip_core::json::ext::SipMessageJson;  // Import the SipMessageJson trait
use rvoip_sip_core::types::headers::HeaderAccess;  // Import the HeaderAccess trait
use bytes::Bytes;
use std::str::FromStr;
use tracing::info;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with a default filter level if RUST_LOG is set
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()))
            .init();
        
        info!("Logging enabled for tutorial");
    }

    println!("SIP Core Tutorial 2: Parsing SIP Messages\n");

    // Example 1: Parsing a SIP Request
    println!("Example 1: Parsing a SIP REGISTER Request\n");
    
    // Raw SIP REGISTER message as bytes
    let register_message = "REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP user-pc.example.com:5060;branch=z9hG4bKnashds7\r\n\
Max-Forwards: 70\r\n\
To: User <sip:user@example.com>\r\n\
From: User <sip:user@example.com>;tag=a73kszlfl\r\n\
Call-ID: 1j9FpLxk3uxtm8tn@user-pc.example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:user@user-pc.example.com>\r\n\
Expires: 3600\r\n\
Content-Length: 0\r\n\
\r\n";

    // Display the raw message
    println!("{}", register_message.replace("\r\n", "\n"));
    println!("\n------------------------------------\n");

    // Parse the message using the message parser
    let data = Bytes::from(register_message);
    let message = parse_message(&data)?;
    
    // Check if it's a request (it should be!)
    if let Message::Request(request) = message {
        println!("Successfully parsed REGISTER request\n");
        
        // 1. Using Path Accessors (JSON path style)
        println!("1. Using Path Accessors (JSON path style):");
        println!("  Method: {}", request.path_str_or("method", "(unknown)"));
        println!("  URI: {}", request.path_str_or("uri", "(unknown)"));
        println!("  Version: {}", request.path_str_or("version", "(unknown)"));
        
        // Accessing headers with path accessors
        println!("\n  Header information:");
        println!("    From: {} <{}>; tag={}", 
            request.path_str_or("headers.From.display_name", "(unknown)"),
            request.path_str_or("headers.From.uri", "(unknown)"),
            request.path_str_or("headers.From.Params.Tag", "(none)"));
            
        println!("    To: {} <{}>", 
            request.path_str_or("headers.To.display_name", "(unknown)"),
            request.path_str_or("headers.To.Uri", "(unknown)"));
            
        // Test both implicit and explicit first-element Via header access
        println!("    Via (implicit): SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via.sent_protocol.transport", "(unknown)"),
            request.path_str_or("headers.Via.sent_by_host.Domain", "(unknown)"),
            request.path_str_or("headers.Via.params.Branch", "(unknown)"));
            
        println!("    Via (explicit): SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via.sent_protocol.transport", "(unknown)"),
            request.path_str_or("headers.Via.sent_by_host.Domain", "(unknown)"),
            request.path_str_or("headers.Via.params.Branch", "(unknown)"));
            
        println!("    Call-ID: {}", request.path_str_or("headers.CallId", "(none)"));
        println!("    CSeq: {} {}", 
            request.path_str_or("headers.CSeq.seq", "(unknown)"),
            request.path_str_or("headers.CSeq.method", "(unknown)"));
        println!("    Contact: <{}>", 
            request.path_str_or("headers.Contact.Params.Address.uri", "(unknown)"));
        println!("    Expires: {}", request.path_str_or("headers.Expires", "(unknown)"));
        
        // 2. Using Native Methods
        println!("\n2. Using Native Methods:");
        println!("  Method: {}", request.method());
        println!("  URI: {}", request.uri());
        println!("  Version: {}", request.version());
        
        // Accessing headers with native methods
        println!("\n  Header information:");
        if let Some(from) = request.from() {
            println!("    From: {}", from);
        }
        
        if let Some(to) = request.to() {
            println!("    To: {}", to);
        }
        
        if let Some(via) = request.first_via() {
            println!("    Via: {}", via);
        }
        
        if let Some(call_id) = request.call_id() {
            println!("    Call-ID: {}", call_id);
        }
        
        if let Some(cseq) = request.cseq() {
            println!("    CSeq: {} {}", cseq.seq, cseq.method);
        }
        
        // Using contact_uri() from SipMessageJson trait
        if let Some(contact_uri) = request.contact_uri() {
            println!("    Contact: <{}>", contact_uri);
        }
        
        // For Expires, use header() with HeaderName
        if let Some(header) = request.header(&HeaderName::Expires) {
            println!("    Expires: {}", header);
        }
    } else {
        println!("Expected a request, got a response!");
    }
    
    println!("\n------------------------------------\n");
    
    // Example 2: Parsing a SIP Response
    println!("Example 2: Parsing a SIP Response\n");
    
    // Raw SIP 200 OK response to REGISTER
    let response_message = "SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP user-pc.example.com:5060;branch=z9hG4bKnashds7;received=192.168.1.100\r\n\
To: User <sip:user@example.com>;tag=37GkEhwl6\r\n\
From: User <sip:user@example.com>;tag=a73kszlfl\r\n\
Call-ID: 1j9FpLxk3uxtm8tn@user-pc.example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:user@user-pc.example.com>;expires=3600\r\n\
Date: Wed, 10 Nov 2023 16:40:30 GMT\r\n\
Content-Length: 0\r\n\
\r\n";

    // Display the raw message
    println!("{}", response_message.replace("\r\n", "\n"));
    println!("\n------------------------------------\n");

    // Parse the message
    let data = Bytes::from(response_message);
    let message = parse_message(&data)?;
    
    // Check if it's a response (it should be!)
    if let Message::Response(response) = message {
        println!("Successfully parsed 200 OK response\n");
        
        // 1. Using Path Accessors
        println!("1. Using Path Accessors:");
        println!("  Status Code: {}", response.path_str_or("status", "(unknown)"));
        println!("  Reason: {}", response.path_str_or("reason", "(unknown)"));
        println!("  Version: {}", response.path_str_or("version", "(unknown)"));
        
        // Accessing headers with path accessors
        println!("\n  Header information:");
        println!("    From: {} <{}>; tag={}", 
            response.path_str_or("headers.From.display_name", "(unknown)"),
            response.path_str_or("headers.From.uri", "(unknown)"),
            response.path_str_or("headers.From.params[0].Tag", "(none)"));
            
        println!("    To: {} <{}>; tag={}", 
            response.path_str_or("headers.To.display_name", "(unknown)"),
            response.path_str_or("headers.To.uri", "(unknown)"),
            response.path_str_or("headers.To.params[0].Tag", "(none)"));
            
        println!("    Via: SIP/2.0/{} {}; branch={}; received={}", 
            response.path_str_or("headers.Via[0].sent_protocol.transport", "(unknown)"),
            response.path_str_or("headers.Via[0].sent_by_host.Domain", "(unknown)"),
            response.path_str_or("headers.Via[0].params.Branch", "(unknown)"),
            response.path_str_or("headers.Via[0].params[1].Received", "(unknown)"));
            
        println!("    Contact: <{}>; expires={}", 
            response.path_str_or("headers.Contact[0].Params[0].address.uri", "(unknown)"),
            response.path_str_or("headers.Contact[0].Params[0].address.params[0].Expires", "(unknown)"));
        
        // 2. Using Native Methods
        println!("\n2. Using Native Methods:");
        println!("  Status Code: {}", response.status_code());
        println!("  Reason: {}", response.reason_phrase());
        println!("  Version: {}", response.version());
        
        // Accessing headers with native methods
        println!("\n  Header information:");
        if let Some(from) = response.from() {
            println!("    From: {}", from);
        }
        
        if let Some(to) = response.to() {
            println!("    To: {}", to);
        }
        
        if let Some(via) = response.first_via() {
            println!("    Via: {}", via);
        }
        
        // Using contact_uri() from SipMessageJson trait
        if let Some(contact_uri) = response.contact_uri() {
            println!("    Contact: <{}>", contact_uri);
        }
        
        // For Date, use header() with HeaderName
        if let Some(header) = response.header(&HeaderName::Date) {
            println!("    Date: {}", header);
        }
    } else {
        println!("Expected a response, got a request!");
    }
    
    println!("\n------------------------------------\n");
    
    // Example 3: Handling Multiple Headers
    println!("Example 3: Handling Multiple Headers\n");
    
    // SIP message with multiple headers of the same type
    let multi_header_message = "INVITE sip:bob@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bK87asdks7\r\n\
Via: SIP/2.0/UDP user-pc.example.com:5060;branch=z9hG4bKnashds7\r\n\
Record-Route: <sip:proxy1.example.com;lr>\r\n\
Record-Route: <sip:proxy2.example.com;lr>\r\n\
To: Bob <sip:bob@example.com>\r\n\
From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@user-pc.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@user-pc.example.com>\r\n\
Content-Length: 0\r\n\
\r\n";

    // Display the raw message
    println!("{}", multi_header_message.replace("\r\n", "\n"));
    println!("\n------------------------------------\n");

    // Parse the message
    let data = Bytes::from(multi_header_message);
    let message = parse_message(&data)?;
    
    // Check if it's a request
    if let Message::Request(request) = message {
        println!("Successfully parsed request with multiple headers\n");
        
        // 1. Accessing multiple headers with path accessors
        println!("1. Accessing multiple headers with path accessors:");
        
        // Via headers - test both implicit and explicit access
        println!("\n  Via headers:");
        println!("    First Via (implicit index): SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via.sent_protocol.transport", "(unknown)"),
            request.path_str_or("headers.Via.sent_by_host.Domain", "(unknown)"),
            request.path_str_or("headers.Via.params.Branch", "(unknown)"));
            
        println!("    First Via (explicit index): SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via[0].sent_protocol.transport", "(unknown)"),
            request.path_str_or("headers.Via[0].sent_by_host.Domain", "(unknown)"),
            request.path_str_or("headers.Via[0].params.Branch", "(unknown)"));
            
        println!("    Second Via: SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via[1].sent_protocol.transport", "(unknown)"),
            request.path_str_or("headers.Via[1].sent_by_host.Domain", "(unknown)"),
            request.path_str_or("headers.Via[1].params.Branch", "(unknown)"));
        
        // Record-Route headers
        // Note: With the enhanced get_path, headers.RecordRoute accesses the first instance,
        // and headers.RecordRoute[1] accesses the second instance.
        // The .uri access will implicitly take the first element if the RecordRoute value is an array.
        println!("\n  Record-Route headers:");
        println!("    First Record-Route: <{}>", 
            request.path_str_or("headers.RecordRoute.uri", "(unknown)"));
            
        println!("    Second Record-Route: <{}>", 
            request.path_str_or("headers.RecordRoute[1].uri", "(unknown)"));
        
        // 2. Accessing multiple headers with native methods
        println!("\n2. Accessing multiple headers with native methods:");
        
        // Via headers
        println!("\n  Via headers:");
        let via_headers = request.via_headers();
        for (i, via) in via_headers.iter().enumerate() {
            println!("    Via #{}: {}", i+1, via);
        }
        
        // Record-Route headers
        println!("\n  Record-Route headers:");
        let record_route_headers = request.headers_by_name("Record-Route");
        for (i, rr) in record_route_headers.iter().enumerate() {
            println!("    Record-Route #{}: {}", i+1, rr);
        }
        
        // 3. Checking header count
        println!("\n3. Checking header count:");
        println!("    Via headers: {}", request.via_headers().len());
        println!("    Record-Route headers: {}", request.headers_by_name("Record-Route").len());
    } else {
        println!("Expected a request, got a response!");
    }
    
    println!("\n------------------------------------\n");
    
    // Example 4: Handling SIP URIs
    println!("Example 4: Handling SIP URIs\n");
    
    // Parse a complex SIP URI
    let uri_str = "sip:user:password@example.com:5060;transport=tcp;ttl=3?subject=Meeting&priority=urgent";
    println!("URI string: {}\n", uri_str);
    
    match Uri::from_str(uri_str) {
        Ok(uri) => {
            println!("Successfully parsed URI\n");
            
            // Access URI components
            println!("URI components:");
            println!("  Scheme: {}", uri.scheme);
            println!("  User: {}", uri.user.unwrap_or_default());
            
            if let Some(password) = uri.password {
                println!("  Password: {}", password);
            }
            
            println!("  Host: {}", uri.host);
            
            if let Some(port) = uri.port {
                println!("  Port: {}", port);
            }
            
            // URI parameters
            println!("\nURI parameters:");
            for param in &uri.parameters {
                match param {
                    Param::Transport(transport) => println!("  Transport: {}", transport),
                    Param::Ttl(ttl) => println!("  TTL: {}", ttl),
                    Param::Other(name, Some(value)) => println!("  {}: {}", name, value),
                    Param::Other(name, None) => println!("  {}", name),
                    _ => println!("  {:?}", param),
                }
            }
            
            // URI headers
            println!("\nURI headers:");
            for (name, value) in &uri.headers {
                println!("  {}: {}", name, value);
            }
        },
        Err(e) => {
            println!("Failed to parse URI: {}", e);
        }
    }
    
    println!("\n------------------------------------\n");
    
    // Example 5: Working with Message Bodies
    println!("Example 5: Working with Message Bodies\n");
    
    // SIP message with SDP body
    let sdp_body = "v=0\r\n\
o=alice 2890844526 2890844526 IN IP4 alice-pc.example.com\r\n\
s=Session SDP\r\n\
c=IN IP4 alice-pc.example.com\r\n\
t=0 0\r\n\
m=audio 49170 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n";
    
    let content_length = sdp_body.len();
    
    let message_with_body = format!("INVITE sip:bob@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP alice-pc.example.com:5060;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@example.com>\r\n\
From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@alice-pc.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@alice-pc.example.com>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: {}\r\n\
\r\n\
{}", content_length, sdp_body);

    // Display the raw message
    println!("{}", message_with_body.replace("\r\n", "\n"));
    println!("\n------------------------------------\n");

    // Parse the message
    let data = Bytes::from(message_with_body);
    let message = parse_message(&data)?;
    
    // Check if it's a request
    if let Message::Request(request) = message {
        println!("Successfully parsed request with body\n");
        
        // 1. Accessing body with path accessors
        println!("1. Accessing body with path accessors:");
        println!("  Content-Type: {}", request.path_str_or("headers.ContentType", "(none)"));
        println!("  Content-Length: {}", request.path_str_or("headers.ContentLength", "0"));
        println!("  Body: {} bytes", request.body().len());
        
        // 2. Accessing body with native methods
        println!("\n2. Accessing body with native methods:");
        
        if let Some(header) = request.header(&HeaderName::ContentType) {
            println!("  Content-Type: {}", header);
        }
        
        if let Some(header) = request.header(&HeaderName::ContentLength) {
            println!("  Content-Length: {}", header);
        }
        
        println!("  Body: {} bytes", request.body().len());
        println!("  Body (as string):\n{}", std::str::from_utf8(request.body())?);
    } else {
        println!("Expected a request, got a response!");
    }
    
    Ok(())
} 