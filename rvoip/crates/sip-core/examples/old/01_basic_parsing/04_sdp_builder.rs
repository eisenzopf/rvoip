//! Example 4: Creating SIP messages with SDP content using builder pattern
//! 
//! This example demonstrates how to create SIP messages with SDP (Session Description Protocol)
//! content using the builder pattern, and how to parse SDP from message bodies.

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::sdp::{SdpBuilder, attributes::MediaDirection};
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::builder::headers::ContentBuilderExt;
use std::str::FromStr;
use tracing::info;

fn main() {
    // Initialize logging with a default filter level
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();
    
    info!("Example 4: Creating SIP messages with SDP content using builder pattern");
    
    // Create a simple INVITE request
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
    
    // Add SDP to the INVITE request
    let invite_with_sdp = invite_req.content_type_sdp().sdp_body(&sdp).build();
    
    // Log the full message to see the result
    info!("INVITE with SDP: \n{}", invite_with_sdp);
    
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
    
    // Add SDP to the 200 OK response
    let ok_with_sdp = response.content_type_sdp().sdp_body(&sdp_answer).build();
    
    // Log the full message to see the result
    info!("200 OK with SDP: \n{}", ok_with_sdp);
    
    // Parse the SDP from the response body
    let body = ok_with_sdp.body();
    if let Ok(parsed_sdp) = SdpSession::from_str(std::str::from_utf8(&body).unwrap()) {
        info!("Parsed SDP session name: {}", parsed_sdp.session_name);
        info!("Media description count: {}", parsed_sdp.media_descriptions.len());
        
        if let Some(media) = parsed_sdp.media_descriptions.first() {
            info!("Media type: {}", media.media);
            info!("Media port: {}", media.port);
            info!("Media protocol: {}", media.protocol);
            info!("Media formats: {}", media.formats.join(", "));
        }
    }
} 