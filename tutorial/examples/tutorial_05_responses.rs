// Example code for Tutorial 05: SIP Responses in Depth
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::ResponseBuilder;
use rvoip_sip_core::builder::headers::*;
use rvoip_sip_core::types::{StatusCode, Refresher, Require};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use std::error::Error;

fn main() -> std::result::Result<(), Box<dyn Error>> {
    println!("Tutorial 05: SIP Responses in Depth\n");
    
    // Example 1: Provisional responses (1xx)
    let trying = create_trying_response()?;
    println!("=== 100 Trying Response ===");
    println!("{}\n", trying);
    
    let ringing = create_ringing_response()?;
    println!("=== 180 Ringing Response ===");
    println!("{}\n", ringing);
    
    let session_progress = create_session_progress_response()?;
    println!("=== 183 Session Progress Response ===");
    println!("{}\n", session_progress);
    
    // Example 2: Success responses (2xx)
    let ok_response = create_ok_response()?;
    println!("=== 200 OK Response ===");
    println!("{}\n", ok_response);
    
    let accepted = create_accepted_response()?;
    println!("=== 202 Accepted Response ===");
    println!("{}\n", accepted);
    
    // Example 3: Redirection responses (3xx)
    let moved_temporarily = create_moved_temporarily_response()?;
    println!("=== 302 Moved Temporarily Response ===");
    println!("{}\n", moved_temporarily);
    
    let use_proxy = create_use_proxy_response()?;
    println!("=== 305 Use Proxy Response ===");
    println!("{}\n", use_proxy);
    
    // Example 4: Client error responses (4xx)
    let bad_request = create_bad_request_response()?;
    println!("=== 400 Bad Request Response ===");
    println!("{}\n", bad_request);
    
    let unauthorized = create_unauthorized_response()?;
    println!("=== 401 Unauthorized Response ===");
    println!("{}\n", unauthorized);
    
    let forbidden = create_forbidden_response()?;
    println!("=== 403 Forbidden Response ===");
    println!("{}\n", forbidden);
    
    let not_found = create_not_found_response()?;
    println!("=== 404 Not Found Response ===");
    println!("{}\n", not_found);
    
    let not_acceptable = create_not_acceptable_response()?;
    println!("=== 406 Not Acceptable Response ===");
    println!("{}\n", not_acceptable);
    
    // Example 5: Server error responses (5xx)
    let server_error = create_server_error_response()?;
    println!("=== 500 Server Error Response ===");
    println!("{}\n", server_error);
    
    let service_unavailable = create_service_unavailable_response()?;
    println!("=== 503 Service Unavailable Response ===");
    println!("{}\n", service_unavailable);
    
    // Example 6: Global failure responses (6xx)
    let busy = create_busy_response()?;
    println!("=== 486 Busy Here Response ===");
    println!("{}\n", busy);
    
    let decline = create_decline_response()?;
    println!("=== 603 Decline Response ===");
    println!("{}\n", decline);
    
    Ok(())
}

// 1xx - Provisional Responses

// Create a 100 Trying response
fn create_trying_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .build();
    
    Ok(Message::Response(response))
}

// Create a 180 Ringing response
fn create_ringing_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Ringing, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@biloxi.example.com", None)
        .build();
    
    Ok(Message::Response(response))
}

// Create a 183 Session Progress response with SDP
fn create_session_progress_response() -> Result<Message> {
    // Create SDP for early media
    let sdp = SdpBuilder::new("Session Progress")
        .origin("bob", "2890844527", "2890844527", "IN", "IP4", "biloxi.example.com")
        .connection("IN", "IP4", "biloxi.example.com") 
        .time("0", "0")
        .media_audio(49172, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendOnly) // One-way early media
            .done()
        .build()?;

    let require = Require::with_tag("100rel");

    let response = ResponseBuilder::new(StatusCode::SessionProgress, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@biloxi.example.com", None)
        .content_type("application/sdp")
        .header(TypedHeader::Require(require)) // Requires reliable provisional responses
        .rseq(1) // Using the new RSeq builder
        .body(sdp.to_string())
        .build();
    
    Ok(Message::Response(response))
}

// 2xx - Success Responses

// Create a 200 OK response to an INVITE with SDP
fn create_ok_response() -> Result<Message> {
    // Create SDP answer
    let sdp = SdpBuilder::new("Call with Alice")
        .origin("bob", "2890844527", "2890844527", "IN", "IP4", "biloxi.example.com")
        .connection("IN", "IP4", "biloxi.example.com") 
        .time("0", "0")
        .media_audio(49172, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;

    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@biloxi.example.com", None)
        .content_type("application/sdp")
        .allow_methods(vec![
            Method::Invite,
            Method::Ack,
            Method::Cancel,
            Method::Bye,
            Method::Refer,
            Method::Notify,
            Method::Options
        ])
        .supported_tags(vec![
            "replaces".to_string(),
            "100rel".to_string()
        ])
        .session_expires(3600, Some(Refresher::Uas))
        .body(sdp.to_string())
        .build();
    
    Ok(Message::Response(response))
}

// Create a 202 Accepted response (for SUBSCRIBE)
fn create_accepted_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Accepted, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(1, Method::Subscribe)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@biloxi.example.com", None)
        .expires_seconds(3600) // How long the subscription is accepted for
        .build();
    
    Ok(Message::Response(response))
}

// 3xx - Redirection Responses

// Create a 302 Moved Temporarily response
fn create_moved_temporarily_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::MovedTemporarily, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@chicago.example.com", None) // New contact address
        .expires_seconds(1800) // For how long this redirection is valid
        .build();
    
    Ok(Message::Response(response))
}

// Create a 305 Use Proxy response
fn create_use_proxy_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::UseProxy, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:proxy.biloxi.example.com", None) // Proxy address
        .build();
    
    Ok(Message::Response(response))
}

// 4xx - Client Error Responses

// Create a 400 Bad Request response
fn create_bad_request_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::BadRequest, Some("Missing Required Header"))
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .build();
    
    Ok(Message::Response(response))
}

// Create a 401 Unauthorized response with authentication challenge
fn create_unauthorized_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .www_authenticate_digest(
            "biloxi.example.com",                  // realm
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",  // nonce
            Some("auth"),                          // qop
            Some("MD5"),                           // algorithm
            Some(vec!["5ccc069c403ebaf9f0171e9517f40e41"]), // opaque
            None,                                  // stale
            None                                   // domain
        )
        .build();
    
    Ok(Message::Response(response))
}

// Create a 403 Forbidden response
fn create_forbidden_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Forbidden, Some("User blocked"))
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .build();
    
    Ok(Message::Response(response))
}

// Create a 404 Not Found response
fn create_not_found_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::NotFound, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .build();
    
    Ok(Message::Response(response))
}

// Create a 406 Not Acceptable response
fn create_not_acceptable_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::NotAcceptable, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .accept("application/sdp", None) // Only accept SDP
        .build();
    
    Ok(Message::Response(response))
}

// 5xx - Server Error Responses

// Create a 500 Server Internal Error response
fn create_server_error_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::ServerInternalError, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .retry_after_duration(300, 0, Some("Server maintenance")) // Using the new RetryAfter builder
        .build();
    
    Ok(Message::Response(response))
}

// Create a 503 Service Unavailable response
fn create_service_unavailable_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .retry_after(120) // Using the new RetryAfter builder
        .build();
    
    Ok(Message::Response(response))
}

// 486 Busy Here (technically a 4xx but relevant to 6xx section)
fn create_busy_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::BusyHere, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .retry_after_with_comment(60, "User in another call") // Using the new RetryAfter builder
        .build();
    
    Ok(Message::Response(response))
}

// Create a 603 Decline response (global failure)
fn create_decline_response() -> Result<Message> {
    let response = ResponseBuilder::new(StatusCode::Decline, None)
        .from("Bob", "sip:bob@biloxi.example.com", None)
        .to("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .retry_after_with_comment(3600, "User unavailable") // Using the new RetryAfter builder
        .build();
    
    Ok(Message::Response(response))
} 