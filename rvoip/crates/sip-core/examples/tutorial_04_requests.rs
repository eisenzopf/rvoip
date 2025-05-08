// Example code for Tutorial 04: SIP Requests in Depth
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::{SimpleRequestBuilder, CSeqBuilderExt};
use rvoip_sip_core::builder::headers::ExpiresBuilderExt;
use rvoip_sip_core::builder::headers::AllowBuilderExt;
use rvoip_sip_core::builder::headers::SupportedBuilderExt;
use rvoip_sip_core::builder::headers::AcceptExt;
use rvoip_sip_core::builder::headers::AcceptLanguageExt;
use rvoip_sip_core::builder::headers::AcceptEncodingExt;
use rvoip_sip_core::builder::headers::AuthorizationExt;
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::TypedHeader;
use std::error::Error;

fn main() -> std::result::Result<(), Box<dyn Error>> {
    println!("Tutorial 04: SIP Requests in Depth\n");
    
    // Example 1: INVITE with detailed headers
    let invite = create_detailed_invite()?;
    println!("=== Detailed INVITE Request ===");
    println!("{}\n", invite);
    
    // Example 2: REGISTER with authentication
    let register = create_register_with_auth()?;
    println!("=== REGISTER Request with Authentication ===");
    println!("{}\n", register);
    
    // Example 3: SUBSCRIBE for event notification
    let subscribe = create_subscribe()?;
    println!("=== SUBSCRIBE Request ===");
    println!("{}\n", subscribe);
    
    // Example 4: REFER for call transfer
    let refer = create_refer()?;
    println!("=== REFER Request ===");
    println!("{}\n", refer);
    
    // Example 5: MESSAGE for instant messaging
    let message = create_message()?;
    println!("=== MESSAGE Request ===");
    println!("{}\n", message);
    
    // Example 6: UPDATE for session modification
    let update = create_update()?;
    println!("=== UPDATE Request ===");
    println!("{}\n", update);
    
    // Example 7: OPTIONS for capability query
    let options = create_options()?;
    println!("=== OPTIONS Request ===");
    println!("{}\n", options);
    
    Ok(())
}

// Create a detailed INVITE request with many headers
fn create_detailed_invite() -> Result<Message> {
    // Create SDP using SdpBuilder
    let sdp = SdpBuilder::new("Call with Bob")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "atlanta.example.com")
        .connection("IN", "IP4", "atlanta.example.com") 
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8", "97"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("97", "iLBC/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;

    let invite_request = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        // Add standard but optional headers
        .content_type("application/sdp")
        .user_agent("SIPClient/1.0")
        .accept("application/sdp", None)
        .allow_methods(vec![
            Method::Invite, 
            Method::Ack, 
            Method::Cancel, 
            Method::Bye, 
            Method::Notify, 
            Method::Refer, 
            Method::Options
        ])
        .supported_tags(vec![
            "replaces".to_string(), 
            "100rel".to_string()
        ])
        // Session-specific headers (using TypedHeader for non-builder headers)
        .header(TypedHeader::Other(HeaderName::Other("Session-Expires".to_string()), 
                HeaderValue::text("3600;refresher=uac")))
        .header(TypedHeader::Other(HeaderName::Other("Min-SE".to_string()), 
                HeaderValue::text("90")))
        // Use the SDP we created
        .body(sdp.to_string())
        .build();
    
    Ok(Message::Request(invite_request))
}

// Create a REGISTER request with authentication
fn create_register_with_auth() -> Result<Message> {
    let register_request = SimpleRequestBuilder::register("sip:registrar.example.com")?
        .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("1j9FpLxk3uxtm8tn@alice-pc.example.com")
        .cseq(2)
        .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@alice-pc.example.com", None)
        .expires_seconds(3600)
        // Add authentication header using AuthorizationExt
        .authorization_digest(
            "alice@example.com",                // username
            "example.com",                      // realm
            "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
            "e6f99bf42fe01fc304d3d4eee7dddd44", // response
            Some("0a4f113b"),                  // cnonce
            Some("auth"),                      // qop
            Some("00000001"),                  // nc
            Some("REGISTER"),                  // method
            Some("sip:registrar.example.com"), // uri
            Some("MD5"),                       // algorithm
            Some("5ccc069c403ebaf9f0171e9517f40e41") // opaque
        )
        .build();
    
    Ok(Message::Request(register_request))
}

// Create a SUBSCRIBE request for event notification
fn create_subscribe() -> Result<Message> {
    let subscribe_request = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(1)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        // Event package and subscription details
        .header(TypedHeader::Other(HeaderName::Other("Event".to_string()), 
                HeaderValue::text("presence")))
        .accept("application/pidf+xml", Some(1.0))
        .expires_seconds(3600)
        .build();
    
    Ok(Message::Request(subscribe_request))
}

// Create a REFER request for call transfer
fn create_refer() -> Result<Message> {
    let refer_request = SimpleRequestBuilder::new(Method::Refer, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(101)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        // Refer-To header specifies transfer target
        .header(TypedHeader::Other(HeaderName::Other("Refer-To".to_string()), 
                HeaderValue::text("<sip:carol@chicago.example.com>")))
        .header(TypedHeader::Other(HeaderName::Other("Referred-By".to_string()), 
                HeaderValue::text("<sip:alice@atlanta.example.com>")))
        .build();
    
    Ok(Message::Request(refer_request))
}

// Create a MESSAGE request for instant messaging
fn create_message() -> Result<Message> {
    let message_request = SimpleRequestBuilder::new(Method::Message, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(1)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .content_type("text/plain")
        .body("Hello Bob, this is Alice. Can we meet at 2pm today?")
        .build();
    
    Ok(Message::Request(message_request))
}

// Create an UPDATE request for session modification
fn create_update() -> Result<Message> {
    // Create SDP for the update using SdpBuilder
    let sdp = SdpBuilder::new("Call with Bob")
        .origin("alice", "2890844526", "2890844527", "IN", "IP4", "atlanta.example.com")
        .connection("IN", "IP4", "atlanta.example.com") 
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;

    let update_request = SimpleRequestBuilder::new(Method::Update, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(2)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@atlanta.example.com", None)
        .content_type("application/sdp")
        // Session timer headers
        .header(TypedHeader::Other(HeaderName::Other("Session-Expires".to_string()), 
                HeaderValue::text("1800;refresher=uac")))
        .body(sdp.to_string())
        .build();
    
    Ok(Message::Request(update_request))
}

// Create an OPTIONS request for capability query
fn create_options() -> Result<Message> {
    let options_request = SimpleRequestBuilder::options("sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
        .cseq(1)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .accept("application/sdp", None)
        .accept_language("en", Some(1.0))
        .accept_encoding("identity", Some(1.0))
        .build();
    
    Ok(Message::Request(options_request))
} 