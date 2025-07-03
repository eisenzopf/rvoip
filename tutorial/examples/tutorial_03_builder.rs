use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::ExpiresBuilderExt;
use rvoip_sip_core::builder::CSeqBuilderExt;
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
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

    println!("SIP Core Tutorial 3: Creating SIP Messages with the Builder Pattern\n");

    // Example 1: Creating a simple SIP INVITE request
    println!("Example 1: Creating a Simple SIP INVITE Request\n");
    
    // Create a basic INVITE request using the SimpleRequestBuilder
    let invite_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314159)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@alice-pc.example.com", None)
        .content_type("application/sdp")
        .body(r#"v=0
o=alice 2890844526 2890844526 IN IP4 alice-pc.example.com
s=Session SDP
c=IN IP4 alice-pc.example.com
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#)
        .build();
    
    // Display the formatted request
    println!("{}", invite_request);
    println!("\n------------------------------------\n");
    
    // Example 2: Creating a SIP response
    println!("Example 2: Creating a SIP Response\n");
    
    // Create a 200 OK response to the INVITE
    let response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("8675309"))
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq_with_method(314159, Method::Invite)
        .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@bob-pc.example.com", None)
        .content_type("application/sdp")
        .body(r#"v=0
o=bob 2890844527 2890844527 IN IP4 bob-pc.example.com
s=Answer to Alice
c=IN IP4 bob-pc.example.com
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#)
        .build();
    
    // Display the formatted response
    println!("{}", response);
    println!("\n------------------------------------\n");
    
    // Example 3: Creating different types of SIP requests
    println!("Example 3: Creating Different Types of SIP Requests\n");
    
    // REGISTER request
    let register_request = SimpleRequestBuilder::register("sip:registrar.example.com")?
        .from("User", "sip:user@example.com", Some("a73kszlfl"))
        .to("User", "sip:user@example.com", None)
        .call_id("1j9FpLxk3uxtm8tn@user-pc.example.com")
        .cseq(1)
        .via("user-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:user@user-pc.example.com", None)
        .expires_seconds(3600)
        .build();
    
    println!("REGISTER Request:\n{}", register_request);
    println!();
    
    // BYE request
    let bye_request = SimpleRequestBuilder::bye("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("8675309"))
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314160)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKasd123"))
        .max_forwards(70)
        .build();
    
    println!("BYE Request:\n{}", bye_request);
    println!();
    
    // OPTIONS request
    let options_request = SimpleRequestBuilder::options("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314161)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKasd456"))
        .max_forwards(70)
        // For Accept header, we need to create a TypedHeader
        .content_type("application/sdp")
        .build();
    
    println!("OPTIONS Request:\n{}", options_request);
    println!("\n------------------------------------\n");
    
    // Example 4: Creating different types of SIP responses
    println!("Example 4: Creating Different Types of SIP Responses\n");
    
    // 100 Trying
    let trying_response = ResponseBuilder::new(StatusCode::Trying, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq_with_method(314159, Method::Invite)
        .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    println!("100 Trying Response:\n{}", trying_response);
    println!();
    
    // 180 Ringing
    let ringing_response = ResponseBuilder::new(StatusCode::Ringing, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("8675309"))
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq_with_method(314159, Method::Invite)
        .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .contact("sip:bob@bob-pc.example.com", None)
        .build();
    
    println!("180 Ringing Response:\n{}", ringing_response);
    println!();
    
    // 404 Not Found
    let not_found_response = ResponseBuilder::new(StatusCode::NotFound, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", Some("8675309"))
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq_with_method(314159, Method::Invite)
        .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    println!("404 Not Found Response:\n{}", not_found_response);
    println!("\n------------------------------------\n");
    
    // Example 5: Creating a request with multiple headers of the same type
    println!("Example 5: Creating a Request with Multiple Headers\n");
    
    // INVITE request with multiple Via headers
    let multi_via_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314159)
        // First Via header (proxy)
        .via("proxy1.example.com:5060", "UDP", Some("z9hG4bK87asdks7"))
        // Second Via header (client)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@alice-pc.example.com", None)
        .build();
    
    println!("{}", multi_via_request);
    println!("\n------------------------------------\n");
    
    // Example 6: Creating a request with custom headers
    println!("Example 6: Creating a Request with Custom Headers\n");
    
    // INVITE request with custom headers
    let custom_headers_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314159)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@alice-pc.example.com", None)
        // Add custom headers using TypedHeader::Other
        .header(TypedHeader::Other(HeaderName::Other("X-Custom-Header".to_string()), HeaderValue::text("Custom Value")))
        .header(TypedHeader::Other(HeaderName::Other("X-Priority".to_string()), HeaderValue::text("1 (Highest)")))
        .header(TypedHeader::Other(HeaderName::Other("X-Session-ID".to_string()), HeaderValue::text("abc123")))
        .build();
    
    println!("{}", custom_headers_request);
    println!("\n------------------------------------\n");
    
    // Example 7: Creating a request with a different URI format
    println!("Example 7: Creating a Request with a Different URI Format\n");
    
    // Parse a complex URI
    let complex_uri_str = "sip:bob@example.com:5060;transport=tcp;lr";
    
    // Create a request with this URI
    let complex_uri_request = SimpleRequestBuilder::new(Method::Message, complex_uri_str)?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314159)
        .via("alice-pc.example.com:5060", "TCP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .content_type("text/plain")
        .body("Hello, Bob! This is a SIP MESSAGE.")
        .build();
    
    println!("{}", complex_uri_request);
    println!("\n------------------------------------\n");
    
    // Example 8: Creating SDP messages using the SdpBuilder
    println!("Example 8: Creating SDP Messages with SdpBuilder\n");
    
    // Create a basic audio-only SDP
    let basic_sdp = SdpBuilder::new("Audio Call")
        .origin("-", "1234567890", "1", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Basic Audio SDP:\n{}", basic_sdp);
    println!();
    
    // Create a more complex SDP with audio and video
    let complex_sdp = SdpBuilder::new("Audio/Video Call")
        .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .media_video(51372, "RTP/AVP")
            .formats(&["96", "97"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;level-asymmetry-allowed=1")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Audio/Video SDP:\n{}", complex_sdp);
    println!();
    
    // Create a WebRTC-style SDP with ICE and DTLS
    let webrtc_sdp = SdpBuilder::new("WebRTC Session")
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
            .ice_candidate("1 1 UDP 2130706431 192.168.1.100 9 typ host")
            .done()
        .media_video(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["96", "97"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "H264/90000")
            .rtcp_fb("96", "nack", Some("pli"))
            .rtcp_fb("96", "ccm", Some("fir"))
            .rtcp_mux()
            .mid("video")
            .direction(MediaDirection::SendRecv)
            .setup("actpass")
            .ice_ufrag("F7gI")
            .ice_pwd("x9cml/YzichV2+XlhiMu8g")
            .ice_candidate("1 1 UDP 2130706431 192.168.1.100 9 typ host")
            .done()
        .build()?;
    
    println!("WebRTC SDP:\n{}", webrtc_sdp);
    println!();
    
    // Example 9: Using SdpBuilder with SIP INVITE
    println!("Example 9: Using SdpBuilder with SIP INVITE\n");
    
    // Create an SDP offer using the builder
    let sdp_offer = SdpBuilder::new("Call Offer")
        .origin("-", "1234567890", "1", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    // Use the SDP in an INVITE request
    let invite_with_sdp = SimpleRequestBuilder::invite("sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("a84b4c76e66710@alice-pc.example.com")
        .cseq(314159)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
        .max_forwards(70)
        .contact("sip:alice@alice-pc.example.com", None)
        .content_type("application/sdp")
        .body(sdp_offer.to_string())
        .build();
    
    println!("INVITE with SDP built using SdpBuilder:\n{}", invite_with_sdp);
    
    Ok(())
} 