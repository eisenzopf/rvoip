# Media Negotiation with SDP

In the previous tutorial, we explored the basics of integrating SDP with SIP for establishing multimedia sessions. Now, we'll dive deeper into the complexities of media negotiation using SDP, covering advanced scenarios like multiple media streams, codec preferences, and handling special media operations.

## Advanced Media Stream Negotiation

Real-world VoIP and multimedia applications often involve multiple types of media streams and complex negotiation scenarios. Let's explore these in detail.

### Multiple Media Types (Audio, Video, Application)

SDP allows for the negotiation of multiple media streams of different types in a single session:

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use bytes::Bytes;

// Create SDP with audio, video, and application data streams
fn create_multi_stream_sdp(username: &str, domain: &str) -> Result<String, Error> {
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
```

When handling multiple media streams, it's important to:

1. Maintain the same media streams in the same order in the answer
2. Process each media stream independently in terms of codec selection
3. Include connection information (c=) for each media when they differ

### Codec Preferences and Ordering

In SDP, the order of codecs in the format list indicates preference. The first codec is the most preferred:

```rust
// Creating SDP with ordered codec preferences
fn create_sdp_with_codec_preferences() -> Result<String, Error> {
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
```

When responding to an offer, your answer should:

1. Only include codecs that were in the original offer
2. Order them according to your preferences
3. You may use a subset of the offered codecs, but cannot add new ones

```rust
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
```

### Bandwidth and Quality Considerations

SDP allows for specifying bandwidth requirements for each media stream:

```rust
// Adding bandwidth parameters to SDP
fn create_sdp_with_bandwidth() -> Result<String, Error> {
    let sdp = SdpBuilder::new("High Quality Video")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        
        // High-def video with bandwidth limit
        .media_video(49174, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .attribute("bandwidth", Some("AS:2000"))  // Application Specific: 2 Mbps
            .attribute("bandwidth", Some("TIAS:2000000"))  // Transport Independent: 2 Mbps
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}
```

## Handling Special Media Operations

### Media Hold and Resume

Placing a call on hold is a common operation that's handled through SDP renegotiation:

```rust
// SDP for placing a call on hold
fn create_hold_sdp(username: &str, domain: &str) -> Result<String, Error> {
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
fn create_resume_sdp(username: &str, domain: &str) -> Result<String, Error> {
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
```

Key points about hold/resume operations:
- For hold, change direction to `SendOnly` or `Inactive`
- For resume, change direction back to `SendRecv`
- Always increment the SDP version in the origin line
- Send a re-INVITE with the updated SDP

### Codec Switching

Sometimes you need to switch or update codecs mid-call:

```rust
// SDP for switching to a different codec
fn create_codec_switch_sdp(username: &str, domain: &str, use_high_quality: bool) -> Result<String, Error> {
    let sdp_builder = SdpBuilder::new("Codec Update")
        .origin(username, "2890844526", "2890844529", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0");
    
    // Choose codecs based on quality preference
    let (formats, audio_builder) = if use_high_quality {
        // High quality: Opus wideband
        let formats = &["96"];
        let audio_builder = sdp_builder.media_audio(49170, "RTP/AVP")
            .formats(formats)
            .rtpmap("96", "opus/48000/2")
            .fmtp("96", "stereo=1;sprop-stereo=1;maxplaybackrate=48000");
        (formats, audio_builder)
    } else {
        // Low quality: PCMU narrowband
        let formats = &["0"];
        let audio_builder = sdp_builder.media_audio(49170, "RTP/AVP")
            .formats(formats)
            .rtpmap("0", "PCMU/8000");
        (formats, audio_builder)
    };
    
    // Complete and build the SDP
    let sdp = audio_builder
        .direction(MediaDirection::SendRecv)
        .done()
        .build()?;
    
    Ok(sdp.to_string())
}
```

## ICE and NAT Traversal

Interactive Connectivity Establishment (ICE) is a protocol for NAT traversal that's commonly used with SIP/SDP:

```rust
// Adding ICE candidates to SDP
fn create_sdp_with_ice() -> Result<String, Error> {
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
```

## Implementing Trickle ICE

Trickle ICE is an extension that allows candidates to be sent incrementally:

```rust
// Initial SDP for Trickle ICE
fn create_initial_trickle_ice_sdp() -> Result<String, Error> {
    let sdp = SdpBuilder::new("Trickle ICE Session")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "0.0.0.0")
        .time("0", "0")
        
        // Session-level ICE attributes
        .attribute("ice-options", Some("trickle"))
        .attribute("ice-pwd", Some("asd88fgpdd777uzjYhagZg"))
        .attribute("ice-ufrag", Some("8hhY"))
        
        // Media section with minimal ICE
        .media_audio(9, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            
            // Just include host candidates initially
            .attribute("candidate", Some("1 1 UDP 2130706431 10.0.1.1 8998 typ host"))
            .attribute("candidate", Some("1 2 UDP 2130706430 10.0.1.1 8999 typ host"))
            
            // Mark that more candidates may come
            .attribute("end-of-candidates", Some(""))  // Not yet the end
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// Additional ICE candidates sent via INFO messages or other means
fn send_additional_ice_candidate(sdp_mid: &str, m_line_index: u32, candidate: &str) {
    // In a real application, this would be sent as an INFO message
    // with application/trickle-ice+sdpfrag content type
    println!("Send Trickle ICE candidate:");
    println!("a=mid:{}", sdp_mid);
    println!("a=ice-ufrag:8hhY");
    println!("a=ice-pwd:asd88fgpdd777uzjYhagZg");
    println!("a=candidate:{}", candidate);
}
```

## Complete Media Negotiation Example

Here's a complete example that handles a more complex SDP negotiation scenario:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{RequestBuilder, ResponseBuilder};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::json::SipJsonExt;
use bytes::Bytes;
use std::str::FromStr;

fn advanced_media_negotiation_example() -> Result<(), Error> {
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
    
    Ok(())
}
```

## Best Practices for SDP Media Negotiation

1. **Codec Compatibility**: Ensure any codecs included in your answer were in the original offer
2. **Media Stream Preservation**: Maintain the same number and type of media streams in the answer as in the offer
3. **Direction Management**: Choose appropriate directions (sendrecv, sendonly, recvonly, inactive) for each media stream
4. **Bandwidth Control**: Use bandwidth attributes to manage quality and network usage
5. **ICE Integration**: Support ICE for NAT traversal; consider Trickle ICE for faster connection establishment
6. **Version Management**: Increment the SDP version in the o= line for each new SDP in the same session
7. **Quality Fallback**: Order codecs to gracefully handle varying bandwidth conditions

## Conclusion

Advanced media negotiation with SDP is critical for creating robust real-time communication applications. With the `rvoip-sip-core` library, you can:

- Negotiate complex media scenarios with multiple streams
- Implement codec priorities and preferences
- Handle special operations like hold/resume and codec switching
- Integrate ICE for NAT traversal

In the next tutorial, we'll move into Part 3 of our series, exploring SIP Transactions and the state machines that govern SIP message exchanges.
