//! Creating SIP Messages Example
//! 
//! This example demonstrates how to create SIP requests and responses
//! using both the builder pattern and the more concise macro syntax.
//! It also shows how to create SDP content and integrate it with SIP messages.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::sdp::{SdpBuilder, attributes::MediaDirection};
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::builder::headers::ContentBuilderExt;
use rvoip_sip_core::{sip_request, sip_response, option_expr};
use std::str::FromStr;
use tracing::info;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Creating Messages Example");
    
    // Example 1: Creating a SIP request using the builder pattern
    create_request_with_builder();
    
    // Example 2: Creating a SIP response using the builder pattern
    create_response_with_builder();
    
    // Example 3: Using macros for concise message creation
    create_message_with_macros();
    
    // Example 4: Creating messages with complex bodies
    create_message_with_body();
    
    // Example 5: Using the SDP integration with the builder pattern
    create_message_with_sdp_integration();
    
    info!("All examples completed successfully!");
}

/// Example 1: Creating a SIP request using the builder pattern
fn create_request_with_builder() {
    info!("Example 1: Creating a SIP request using the builder pattern");
    
    // Build the INVITE request with SimpleRequestBuilder
    let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.com", None)
        .build();
    
    // Convert to string and display
    info!("Created SIP request:\n{}", request);
    
    // Demonstrate how to update parts of a request
    let updated_request = request
        .with_header(TypedHeader::Subject(Subject::new("Urgent call")))
        .with_header(TypedHeader::Priority(Priority::Urgent));
    
    info!("Added Subject and Priority headers");
    
    // Check if the new request has the headers we added
    if updated_request.typed_header::<Subject>().is_some() {
        info!("Subject header was successfully added");
    }
    
    if let Some(priority) = updated_request.typed_header::<Priority>() {
        info!("Priority was set to: {}", priority);
    }
}

/// Example 2: Creating a SIP response using the builder pattern
fn create_response_with_builder() {
    info!("Example 2: Creating a SIP response using the builder pattern");
    
    // Build a 200 OK response to an INVITE using SimpleResponseBuilder
    let response = SimpleResponseBuilder::ok()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@192.0.2.4", None)
        .build();
    
    // Convert to string and display
    info!("Created SIP response:\n{}", response);
    
    // Create other common response types
    
    // 180 Ringing - typical intermediate response
    let ringing_response = SimpleResponseBuilder::ringing()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    info!("Created 180 Ringing response");
    
    // 404 Not Found - error response
    let not_found_response = SimpleResponseBuilder::not_found()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    info!("Created 404 Not Found response");
    
    // Get the status code and reason phrase
    info!("Status: {} {}", not_found_response.status_code(), not_found_response.reason_phrase());
}

/// Example 3: Using macros for concise message creation
fn create_message_with_macros() {
    info!("Example 3: Using macros for concise message creation");
    
    // Create a SIP request with the sip_request! macro
    let request = sip_request! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            MaxForwards: "70",
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:alice@pc33.atlanta.com>",
            ContentLength: "0"
        }
    };
    
    // Display as string
    info!("Created SIP request using macro:\n{}", request);
    
    // Create a SIP response with the sip_response! macro
    let response = sip_response! {
        status: StatusCode::Ok,
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            To: "Bob <sip:bob@example.com>;tag=a6c85cf",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:bob@192.0.2.4>",
            ContentLength: "0"
        }
    };
    
    // Display as string
    info!("Created SIP response using macro:\n{}", response);
}

/// Example 4: Creating messages with complex bodies
fn create_message_with_body() {
    info!("Example 4: Creating messages with bodies");
    
    // Create a simple SDP body
    let sdp_body = 
        "v=0\r\n\
         o=alice 2890844526 2890844526 IN IP4 pc33.atlanta.com\r\n\
         s=Session SDP\r\n\
         c=IN IP4 pc33.atlanta.com\r\n\
         t=0 0\r\n\
         m=audio 49172 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n";
    
    // Create a SIP INVITE with SDP body
    let invite_with_sdp = sip_request! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            MaxForwards: "70",
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:alice@pc33.atlanta.com>",
            ContentType: "application/sdp",
            ContentLength: format!("{}", sdp_body.len())
        },
        body: sdp_body
    };
    
    // Display as string
    info!("Created SIP INVITE with SDP body:\n{}", invite_with_sdp);
    
    // Alternative way to set the body using builder
    let invite_with_builder = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.com", None)
        .content_type("application/sdp")
        .body(sdp_body)
        .build();
    
    info!("Created SIP INVITE with SDP body using builder");
    
    // Parse the SIP message and extract the body
    let serialized = invite_with_builder.to_string();
    let bytes = Bytes::from(serialized);
    let parsed_message = parse_message(&bytes).unwrap();
    
    if let Message::Request(request) = parsed_message {
        let body = request.body();
        if !body.is_empty() {
            info!("Extracted body from parsed message:\n{}", std::str::from_utf8(body).unwrap());
            
            // Check content type
            if let Some(content_type) = request.typed_header::<ContentType>() {
                info!("Content-Type: {}", content_type);
            }
        } else {
            info!("No body found in parsed message");
        }
    }
}

/// Example 5: Using the SDP integration with the builder pattern
fn create_message_with_sdp_integration() {
    info!("Example 5: Using the SDP integration with the builder pattern");
    
    // Create a simple INVITE request using SimpleRequestBuilder
    let invite_req = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.com", None);
    
    // Create an SDP session describing an audio call
    let sdp = SdpBuilder::new("Audio Call")
        .origin("alice", "12345", "12345", "IN", "IP4", "pc33.atlanta.com")
        .connection("IN", "IP4", "pc33.atlanta.com")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"]) // PCMU and PCMA
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Valid SDP");
    
    // Add SDP to the INVITE request using the ContentBuilderExt trait
    let invite_with_sdp = invite_req.content_type_sdp().sdp_body(&sdp).build();
    
    // Log the full message to see the result
    info!("INVITE with SDP using ContentBuilderExt: \n{}", invite_with_sdp);
    
    // Now create a 200 OK response with SDP answer
    let response = SimpleResponseBuilder::ok()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
        .call_id("a84b4c76e66710@pc33.atlanta.com")
        .cseq(314159, Method::Invite)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@192.0.2.4", None);
    
    // Create an SDP answer
    let sdp_answer = SdpBuilder::new("Audio Answer")
        .origin("bob", "54321", "54321", "IN", "IP4", "192.0.2.4")
        .connection("IN", "IP4", "192.0.2.4")
        .time("0", "0")
        .media_audio(51372, "RTP/AVP")
            .formats(&["0"]) // Just PCMU
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Valid SDP answer");
    
    // Add SDP to the 200 OK response using the ContentBuilderExt trait
    let ok_with_sdp = response.content_type_sdp().sdp_body(&sdp_answer).build();
    
    // Log the full message to see the result
    info!("200 OK with SDP using ContentBuilderExt: \n{}", ok_with_sdp);
    
    // Parse the SDP from the response body to demonstrate extraction
    let body = ok_with_sdp.body();
    if let Ok(parsed_sdp) = SdpSession::from_str(std::str::from_utf8(body).unwrap()) {
        info!("Parsed SDP session name: {}", parsed_sdp.session_name);
        info!("Media description count: {}", parsed_sdp.media_descriptions.len());
        
        if let Some(media) = parsed_sdp.media_descriptions.first() {
            info!("Media type: {}", media.media);
            info!("Media port: {}", media.port);
            info!("Media protocol: {}", media.protocol);
            info!("Media formats: {}", media.formats.join(", "));
        }
    }
    
    // Demonstrate adding different media types
    
    // Create an audio+video SDP
    let av_sdp = SdpBuilder::new("Audio+Video Call")
        .origin("alice", "12345", "12345", "IN", "IP4", "pc33.atlanta.com")
        .connection("IN", "IP4", "pc33.atlanta.com")
        .time("0", "0")
        // Add audio media
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"]) // PCMU and PCMA
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Add video media
        .media_video(49180, "RTP/AVP")
            .formats(&["96", "97"]) // VP8 and H264
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Valid A/V SDP");
    
    // Create an INVITE with the A/V SDP
    let av_invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
        .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66711@pc33.atlanta.com")
        .cseq(314160)
        .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.com", None)
        .content_type_sdp()  // Set Content-Type to application/sdp
        .sdp_body(&av_sdp)   // Add the SDP body
        .build();
    
    info!("INVITE with audio+video SDP: \n{}", av_invite);
} 