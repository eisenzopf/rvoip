// Example code for Tutorial 09: Media Negotiation with SDP
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{RequestBuilder, ResponseBuilder};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::json::SipJsonExt;
use std::error::Error as StdError;
use std::str::FromStr;
use bytes::Bytes;

fn main() -> Result<()> {
    println!("Tutorial 09: Media Negotiation with SDP\n");
    
    // Example 1: Creating SDP with multiple media streams
    println!("Example 1: SDP with Multiple Media Streams\n");
    let multi_stream_sdp = create_multi_stream_sdp("alice", "alice.example.com")?;
    println!("{}\n", multi_stream_sdp);
    
    // Example 2: Codec preferences and selection
    println!("Example 2: SDP with Codec Preferences\n");
    let codec_pref_sdp = create_sdp_with_codec_preferences()?;
    println!("{}\n", codec_pref_sdp);
    
    // Parse the SDP to demonstrate codec selection
    let sdp_session = SdpSession::from_str(&codec_pref_sdp)?;
    let selected_codecs = select_preferred_codecs(&sdp_session);
    println!("Selected codecs: {:?}\n", selected_codecs);
    
    // Example 3: Hold and resume operations
    println!("Example 3: Media Hold and Resume\n");
    let hold_sdp = create_hold_sdp("alice", "alice.example.com")?;
    println!("SDP for placing call on hold:\n{}\n", hold_sdp);
    
    let resume_sdp = create_resume_sdp("alice", "alice.example.com")?;
    println!("SDP for resuming call:\n{}\n", resume_sdp);
    
    // Example 4: ICE candidates in SDP
    println!("Example 4: SDP with ICE Candidates\n");
    let ice_sdp = create_sdp_with_ice()?;
    println!("{}\n", ice_sdp);
    
    // Example 5: Complete advanced media negotiation flow
    println!("Example 5: Complete Advanced Media Negotiation\n");
    demonstrate_advanced_media_negotiation()?;
    
    Ok(())
}

// Example 1: Create SDP with multiple media streams
fn create_multi_stream_sdp(username: &str, domain: &str) -> Result<String> {
    let sdp = SdpBuilder::new("Multi-Stream Session")
        .origin(username, "2890844526", "2890844526", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0")
        
        // Audio stream
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8", "96"])  // PCMU, PCMA, telephone-event
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("96", "telephone-event/8000")
            .fmtp("96", "0-15")  // DTMF events
            .direction(MediaDirection::SendRecv)
            .done()
            
        // Video stream
        .media_video(49174, "RTP/AVP")
            .formats(&["97", "98"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .rtpmap("98", "VP8/90000")
            .direction(MediaDirection::SendRecv)
            .done()
            
        // Application data (for example, BFCP for floor control)
        .media("application", 50000, "TCP/BFCP")
            .formats(&["*"])  // Add formats separately
            .connection("IN", "IP4", domain)  // Per-media connection info
            .attribute("setup", Some("actpass"))
            .attribute("connection", Some("new"))
            .attribute("floorctrl", Some("c-s"))
            .attribute("confid", Some("4321"))
            .attribute("userid", Some("1234"))
            .done()
            
        .build()?;
    
    Ok(sdp.to_string())
}

// Example 2: Creating SDP with ordered codec preferences
fn create_sdp_with_codec_preferences() -> Result<String> {
    let sdp = SdpBuilder::new("Audio with Codec Preferences")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        
        // Audio stream with ordered codec preferences
        .media_audio(49170, "RTP/AVP")
            // Most preferred codec first
            .formats(&["96", "97", "98", "99", "0", "8"])
            .rtpmap("96", "opus/48000/2")     // Wideband, high quality
            .rtpmap("97", "AMR-WB/16000/1")   // Wideband
            .rtpmap("98", "EVS/32000/1")      // Super-wideband
            .rtpmap("99", "telephone-event/8000")
            .rtpmap("0", "PCMU/8000")         // Fallback
            .rtpmap("8", "PCMA/8000")         // Last fallback
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// Function to select codecs from an offer based on preferences
fn select_preferred_codecs(offer_sdp: &SdpSession) -> Vec<String> {
    let mut selected_codecs = Vec::new();
    
    // Our codec preferences in order
    let codec_preferences = ["opus", "AMR-WB", "EVS", "PCMU", "PCMA"];
    
    // Find the audio media description
    if let Some(audio_media) = offer_sdp.media_descriptions.iter().find(|m| m.media == "audio") {
        // Get all rtpmap attributes to map payload types to codec names
        let mut codec_map = std::collections::HashMap::new();
        
        // Access the rtpmap attributes directly 
        for rtpmap in audio_media.rtpmaps() {
            let pt = rtpmap.payload_type.to_string();
            let codec_name = rtpmap.encoding_name.to_lowercase();
            codec_map.insert(codec_name, pt);
        }
        
        // Add codecs in our preferred order if they're in the offer
        for preferred_codec in &codec_preferences {
            if let Some(pt) = codec_map.get(*preferred_codec) {
                selected_codecs.push(pt.clone());
            }
        }
    }
    
    selected_codecs
}

// Example 3: SDP for placing a call on hold
fn create_hold_sdp(username: &str, domain: &str) -> Result<String> {
    // Note the "sendonly" direction - this indicates hold
    let sdp = SdpBuilder::new("Call On Hold")
        .origin(username, "2890844526", "2890844527", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendOnly)  // SendOnly for hold
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// SDP for resuming a call that was on hold
fn create_resume_sdp(username: &str, domain: &str) -> Result<String> {
    // Back to sendrecv for resuming the call
    let sdp = SdpBuilder::new("Call Resumed")
        .origin(username, "2890844526", "2890844528", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)  // SendRecv for resuming
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// Example 4: Adding ICE candidates to SDP
fn create_sdp_with_ice() -> Result<String> {
    let sdp = SdpBuilder::new("ICE Session")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "0.0.0.0")  // IP is determined by ICE
        .time("0", "0")
        
        // Session level ICE attributes
        .attribute("ice-pwd", Some("asd88fgpdd777uzjYhagZg"))
        .attribute("ice-ufrag", Some("8hhY"))
        .attribute("ice-options", Some("trickle"))
        
        // Audio stream with ICE candidates
        .media_audio(9, "RTP/AVP")  // Port 9 is a placeholder
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            
            // ICE candidates
            .attribute("candidate", Some("1 1 UDP 2130706431 10.0.1.1 8998 typ host"))
            .attribute("candidate", Some("1 2 UDP 2130706430 10.0.1.1 8999 typ host"))
            .attribute("candidate", Some("2 1 UDP 1694498815 192.0.2.3 45664 typ srflx raddr 10.0.1.1 rport 8998"))
            .attribute("candidate", Some("2 2 UDP 1694498814 192.0.2.3 45665 typ srflx raddr 10.0.1.1 rport 8999"))
            
            // RTCP multiplexing
            .attribute("rtcp-mux", Some(""))
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// Example 5: Complete advanced media negotiation flow
fn demonstrate_advanced_media_negotiation() -> Result<()> {
    // Initial parameters
    let call_id = "adv-media-call-1@example.com";
    let from_tag = "alice-tag-1";
    let cseq = 100;
    
    // Step 1: Alice sends INVITE with complex SDP offer
    println!("Step 1: Starting the VoIP call with advanced media");
    
    let alice_sdp = SdpBuilder::new("Advanced Media Call")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        
        // Session-level attributes
        .attribute("ice-options", Some("trickle"))
        .attribute("ice-pwd", Some("alice-ice-password-1"))
        .attribute("ice-ufrag", Some("alice-ice-1"))
        
        // High-quality audio
        .media_audio(49170, "RTP/AVP")
            .formats(&["96", "0", "8"])
            .rtpmap("96", "opus/48000/2")  // Preferred: Opus
            .rtpmap("0", "PCMU/8000")      // Fallback: PCMU
            .rtpmap("8", "PCMA/8000")      // Last fallback: PCMA
            .direction(MediaDirection::SendRecv)
            .attribute("rtcp-mux", Some(""))
            .attribute("candidate", Some("1 1 UDP 2130706431 192.168.1.1 49170 typ host"))
            .done()
            
        // Video
        .media_video(49174, "RTP/AVP")
            .formats(&["97", "98"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .rtpmap("98", "VP8/90000")
            .direction(MediaDirection::SendRecv)
            .attribute("rtcp-mux", Some(""))
            .attribute("candidate", Some("1 1 UDP 2130706431 192.168.1.1 49174 typ host"))
            .done()
        
        .build()?;
    
    let alice_sdp_str = alice_sdp.to_string();
    
    let invite = RequestBuilder::new(Method::Invite, "sip:bob@example.com")?
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(cseq)
        .via("example.com", "UDP", Some("z9hG4bKabc123"))
        .max_forwards(70)
        .contact("sip:alice@192.168.1.1", None)
        .content_type("application/sdp")
        .body(Bytes::from(alice_sdp_str))
        .build();
    
    println!("INVITE with advanced media offer:\n{}\n", Message::Request(invite.clone()));
    
    // Step 2: Bob processes the offer and creates an answer
    println!("Step 2: Bob answers the call with codec selection");
    
    // Parse the SDP offer
    let alice_sdp_text = std::str::from_utf8(&invite.body()[..]).unwrap();
    let alice_sdp_parsed = SdpSession::from_str(alice_sdp_text)?;
    
    // Bob's preferences and capabilities:
    // - Supports Opus for audio, prefers it
    // - Only supports VP8 for video (not H.264)
    // - Has own ICE credentials
    
    let bob_sdp = SdpBuilder::new("Advanced Media Response")
        .origin("bob", "3890844527", "3890844527", "IN", "IP4", "bob.example.com")
        .connection("IN", "IP4", "bob.example.com")
        .time("0", "0")
        
        // Session-level attributes
        .attribute("ice-options", Some("trickle"))
        .attribute("ice-pwd", Some("bob-ice-password-1"))
        .attribute("ice-ufrag", Some("bob-ice-1"))
        
        // Audio - select Opus as preferred
        .media_audio(59170, "RTP/AVP")
            .formats(&["96"])  // Just Opus, which we prefer
            .rtpmap("96", "opus/48000/2")
            .direction(MediaDirection::SendRecv)
            .attribute("rtcp-mux", Some(""))
            .attribute("candidate", Some("1 1 UDP 2130706431 192.168.1.2 59170 typ host"))
            .done()
            
        // Video - select VP8 only (no H.264 support)
        .media_video(59174, "RTP/AVP")
            .formats(&["98"])  // Just VP8
            .rtpmap("98", "VP8/90000")
            .direction(MediaDirection::SendRecv)
            .attribute("rtcp-mux", Some(""))
            .attribute("candidate", Some("1 1 UDP 2130706431 192.168.1.2 59174 typ host"))
            .done()
        
        .build()?;
    
    let bob_sdp_str = bob_sdp.to_string();
    
    let to_tag = "bob-tag-1";
    let ok_response = ResponseBuilder::dialog_response(
        &invite,
        StatusCode::Ok,
        None
    )
    .to("Bob", "sip:bob@example.com", Some(to_tag))
    .contact("sip:bob@192.168.1.2", None)
    .content_type("application/sdp")
    .body(Bytes::from(bob_sdp_str))
    .build();
    
    println!("200 OK with advanced media answer:\n{}\n", Message::Response(ok_response.clone()));
    
    // Step 3: Alice sends ACK to establish the session
    println!("Step 3: Alice acknowledges the call");
    
    let ack = RequestBuilder::new(Method::Ack, "sip:bob@192.168.1.2")?
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq)
        .via("example.com", "UDP", Some("z9hG4bKdef456"))
        .max_forwards(70)
        .build();
    
    println!("ACK to establish session:\n{}\n", Message::Request(ack));
    
    // Step 4: Alice sends additional ICE candidates via INFO (Trickle ICE)
    println!("Step 4: Alice sends additional ICE candidates");
    
    let info = RequestBuilder::new(Method::Info, "sip:bob@192.168.1.2")?
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 1)
        .via("example.com", "UDP", Some("z9hG4bKghi789"))
        .max_forwards(70)
        .content_type("application/trickle-ice-sdpfrag")
        .body(Bytes::from(
            "a=ice-pwd:alice-ice-password-1\r\n\
             a=ice-ufrag:alice-ice-1\r\n\
             a=candidate:2 1 UDP 1694498815 203.0.113.3 56789 typ srflx raddr 192.168.1.1 rport 49170\r\n"
        ))
        .build();
    
    println!("INFO with additional ICE candidates:\n{}\n", Message::Request(info));
    
    // Step 5: Later, Alice sends re-INVITE to change to hold
    println!("Step 5: Alice puts the call on hold");
    
    let alice_hold_sdp = SdpBuilder::new("Call On Hold")
        .origin("alice", "2890844526", "2890844527", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        
        // Audio on hold (sendonly)
        .media_audio(49170, "RTP/AVP")
            .formats(&["96"])
            .rtpmap("96", "opus/48000/2")
            .direction(MediaDirection::SendOnly)  // Hold
            .attribute("rtcp-mux", Some(""))
            .done()
            
        // Video on hold too
        .media_video(49174, "RTP/AVP")
            .formats(&["98"])
            .rtpmap("98", "VP8/90000")
            .direction(MediaDirection::SendOnly)  // Hold
            .attribute("rtcp-mux", Some(""))
            .done()
        
        .build()?;
    
    let alice_hold_sdp_str = alice_hold_sdp.to_string();
    
    let reinvite = RequestBuilder::new(Method::Invite, "sip:bob@192.168.1.2")?
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 2)
        .via("example.com", "UDP", Some("z9hG4bKjkl012"))
        .max_forwards(70)
        .contact("sip:alice@192.168.1.1", None)
        .content_type("application/sdp")
        .body(Bytes::from(alice_hold_sdp_str))
        .build();
    
    println!("re-INVITE to put call on hold:\n{}\n", Message::Request(reinvite));
    
    println!("All examples completed successfully!");
    
    Ok(())
} 