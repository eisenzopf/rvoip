use rvoip_sip_core::prelude::*;
use rvoip_sip_core::json::SipJsonExt;  // Import the JSON extension trait
use rvoip_sip_core::ResponseBuilder;  // Correct import path
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use bytes::Bytes;
use std::str::FromStr;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with a default filter level if RUST_LOG is set
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()))
            .init();
        
        tracing::info!("Logging enabled");
    }

    println!("SIP Core Tutorial 1: Introduction to SIP\n");

    // Example 1: Examining a SIP INVITE Request
    // SDP body with \n line endings
    let sdp_body = "v=0\n\
o=alice 2890844526 2890844526 IN IP4 alice-pc.example.com\n\
s=Session SDP\n\
c=IN IP4 alice-pc.example.com\n\
t=0 0\n\
m=audio 49170 RTP/AVP 0\n\
a=rtpmap:0 PCMU/8000";

    // Calculate the exact Content-Length
    let content_length = sdp_body.len();

    let invite_message = format!("INVITE sip:bob@example.com SIP/2.0\r\n\
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

    // For display purposes, convert \r\n to \n
    let display_message = invite_message.replace("\r\n", "\n");
    println!("Example 1: SIP INVITE Request\n");
    println!("{}", display_message);
    println!("\n------------------------------------\n");

    // Parse the INVITE request using the message parser
    let data = Bytes::from(invite_message);
    let message = match parse_message(&data) {
        Ok(msg) => msg,
        Err(e) => {
            println!("Failed to parse message: {:?}", e);
            return Ok(());
        }
    };
    
    // Check if it's a request (it should be!)
    if let Message::Request(request) = message {
        println!("Successfully parsed INVITE request using JSON path accessors:");
        
        // Using path_str_or() for direct string access with defaults
        println!("  Method: {}", request.path_str_or("method", "(unknown)"));
        println!("  URI: sip:{}@{}", 
            request.path_str_or("uri.user", "(unknown)"),
            request.path_str_or("uri.host.Domain", "(unknown)"));
        println!("  Version: {}", request.path_str_or("version", "(unknown)"));
        
        // Access headers using path accessors
        println!("\nHeader information:");
        println!("  From: {} <{}>; tag={}", 
            request.path_str_or("headers.From.display_name", "(unknown)"),
            request.path_str_or("headers.From.uri", "(unknown)"),
            request.path_str_or("headers.From.params[0].Tag", "(none)"));
            
        println!("  To: {}", 
            request.path_str_or("headers.To.display_name", "(unknown)"));
            
        println!("  Via: SIP/2.0/{} {}; branch={}", 
            request.path_str_or("headers.Via[0].sent_protocol.transport", "UDP"),
            request.path_str_or("headers.Via[0].sent_by_host.Domain", "unknown"),
            request.path_str_or("headers.Via[0].params[0].Branch", "unknown"));
            
        println!("  Call-ID: {}", request.path_str_or("headers.CallId", "(none)"));
        
        // For CSeq, we want to handle the numeric value properly
        let cseq_num = match request.path("headers.CSeq.seq") {
            Some(val) => val.as_i64().unwrap_or(0).to_string(),
            None => "0".to_string(),
        };
        let cseq_method = request.path_str_or("headers.CSeq.method", "(none)");
        println!("  CSeq: {} {}", cseq_num, cseq_method);
    } else {
        println!("Expected a request, got a response!");
    }
    println!("\n------------------------------------\n");

    // Example 2: Examining a SIP Response
    println!("Example 2: SIP 200 OK Response\n");

    // Create a 200 OK response to the INVITE
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774")) // Keep the same From as in request
        .to("Bob", "sip:bob@example.com", Some("8675309")) // To header with a tag
        .call_id("a84b4c76e66710@alice-pc.example.com") // Same Call-ID as request
        .cseq(314159, Method::Invite) // Same CSeq as request
        .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds")) // Via from request
        .contact("sip:bob@bob-pc.example.com", None) // Bob's contact
        .content_type("application/sdp") // Content-Type for SDP
        .body(r#"v=0
o=bob 2890844527 2890844527 IN IP4 bob-pc.example.com
s=Answer to Alice
c=IN IP4 bob-pc.example.com
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#)
        .build();

    // Print the formatted response
    println!("{}", response);
    println!("\n------------------------------------\n");

    // Example 3: SIP URI anatomy
    println!("Example 3: SIP URI Anatomy\n");
    
    // Create and examine a SIP URI
    let uri = Uri::from_str("sip:alice@example.com:5060;transport=udp;ttl=15?subject=Meeting&priority=urgent")?;
    
    println!("Full URI: {}", uri);
    println!("Scheme: {}", uri.scheme);
    println!("User: {}", uri.user.unwrap_or_default());
    println!("Host: {}", uri.host);
    if let Some(port) = uri.port {
        println!("Port: {}", port);
    }
    
    println!("URI Parameters:");
    for param in &uri.parameters {
        match param {
            Param::Other(name, Some(value)) => println!("  - {}: {}", name, value),
            Param::Other(name, None) => println!("  - {}", name),
            _ => println!("  - {:?}", param),
        }
    }
    
    println!("Header Parameters:");
    for (name, value) in &uri.headers {
        println!("  - {}: {}", name, value);
    }
    
    println!("\n------------------------------------\n");

    // Example 4: Basic SIP methods
    println!("Example 4: SIP Methods\n");
    
    println!("Common SIP methods:");
    let methods = vec![
        Method::Invite,
        Method::Ack,
        Method::Cancel,
        Method::Bye,
        Method::Register,
        Method::Options,
        Method::Subscribe,
        Method::Notify,
        Method::Refer,
        Method::Message,
    ];
    
    for method in methods {
        println!("  - {}", method);
    }

    println!("\n------------------------------------\n");

    // Example 5: Response Status Codes
    println!("Example 5: SIP Response Status Codes\n");
    
    println!("Response categories:");
    println!("  - 1xx Provisional: {}", StatusCode::Trying);
    println!("  - 2xx Success: {}", StatusCode::Ok);
    println!("  - 3xx Redirection: {}", StatusCode::MovedTemporarily);
    println!("  - 4xx Client Error: {}", StatusCode::BadRequest);
    println!("  - 5xx Server Error: {}", StatusCode::ServerInternalError);
    println!("  - 6xx Global Failure: {}", StatusCode::Decline);
    
    println!("\n------------------------------------\n");
    
    // Example 6: Creating SDP with the Builder Pattern
    println!("Example 6: Creating SDP with the Builder Pattern\n");
    
    // Create an SDP offer using the SdpBuilder
    let sdp_result = SdpBuilder::new("Audio Call")
        .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build();
    
    match sdp_result {
        Ok(sdp) => {
            println!("Successfully created SDP offer:");
            println!("{}", sdp);
        },
        Err(e) => println!("Failed to create SDP: {}", e),
    }
    
    // Create a WebRTC SDP offer
    let webrtc_sdp_result = SdpBuilder::new("WebRTC Session")
        .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .group("BUNDLE", &["audio", "video"])
        .ice_ufrag("F7gI")
        .ice_pwd("x9cml/YzichV2+XlhiMu8g")
        .fingerprint("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24")
        .media_audio(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["111", "103"])
            .rtpmap("111", "opus/48000/2")
            .rtpmap("103", "ISAC/16000")
            .fmtp("111", "minptime=10;useinbandfec=1")
            .rtcp_mux()
            .mid("audio")
            .direction(MediaDirection::SendRecv)
            .setup("actpass")
            .ice_ufrag("F7gI")
            .ice_pwd("x9cml/YzichV2+XlhiMu8g")
            .done()
        .build();
    
    match webrtc_sdp_result {
        Ok(sdp) => {
            println!("\nWebRTC SDP offer (truncated for brevity):");
            let sdp_str = sdp.to_string();
            let lines: Vec<&str> = sdp_str.lines().collect();
            // Print just the first few lines
            for line in lines.iter().take(10) {
                println!("{}", line);
            }
            println!("... (more lines) ...");
        },
        Err(e) => println!("Failed to create WebRTC SDP: {}", e),
    }
    
    Ok(())
} 