// Example code for Tutorial 06: Introduction to SDP
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::sdp::SdpSession;
use std::error::Error;
use std::str::FromStr;

fn main() -> Result<(), Box<dyn Error>> {
    println!("Tutorial 06: Introduction to SDP\n");
    
    // Example 1: Creating a basic audio-only SDP
    create_audio_sdp()?;
    
    // Example 2: Creating an audio and video SDP
    create_audio_video_sdp()?;
    
    // Example 3: Creating a more complex WebRTC-style SDP
    create_webrtc_sdp()?;
    
    // Example 4: Parsing an SDP string
    parse_sdp_string()?;
    
    // Example 5: Modifying an existing SDP
    modify_existing_sdp()?;
    
    Ok(())
}

// Example 1: Creating a basic audio-only SDP
fn create_audio_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 1: Basic Audio-only SDP\n");
    
    let sdp = SdpBuilder::new("Audio Call")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0") // "0 0" indicates the session is permanent
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8", "96"]) // G.711 μ-law, G.711 A-law, telephone-event
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("96", "telephone-event/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Audio-only SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 2: Creating an audio and video SDP
fn create_audio_video_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 2: Audio and Video SDP\n");
    
    let sdp = SdpBuilder::new("Audio/Video Call")
        .origin("bob", "2890844527", "2890844527", "IN", "IP4", "bob.example.com")
        .connection("IN", "IP4", "bob.example.com")
        .time("0", "0")
        // Audio stream
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Video stream
        .media_video(51372, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Audio/Video SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 3: Creating a more complex WebRTC-style SDP
fn create_webrtc_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 3: WebRTC-style SDP\n");
    
    let sdp = SdpBuilder::new("WebRTC Session")
        .origin("webrtc", "2890844527", "2", "IN", "IP4", "0.0.0.0")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .group("BUNDLE", &["audio", "video"])
        .ice_ufrag("F7gI")
        .ice_pwd("x9cml/YzichV2+XlhiMu8g")
        .fingerprint("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24:2C:C2:A2:C0:3E:FD:34:8E:5E:EA:6F:AF:52:CE:E6:0F")
        .media_audio(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["111", "103", "104"])
            .rtpmap("111", "opus/48000/2")
            .rtpmap("103", "ISAC/16000")
            .rtpmap("104", "ISAC/32000")
            .fmtp("111", "minptime=10;useinbandfec=1")
            .rtcp_mux()
            .mid("audio")
            .direction(MediaDirection::SendRecv)
            .setup("actpass")
            .ice_ufrag("F7gI")
            .ice_pwd("x9cml/YzichV2+XlhiMu8g")
            .ice_candidate("1 1 UDP 2113937151 192.168.1.100 9 typ host")
            .ice_candidate("2 1 UDP 1845501695 203.0.113.100 9 typ srflx raddr 192.168.1.100 rport 9")
            .done()
        .media_video(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["96", "97", "98"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "H264/90000")
            .rtpmap("98", "VP9/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .rtcp_fb("96", "nack", Some("pli"))
            .rtcp_fb("96", "ccm", Some("fir"))
            .rtcp_mux()
            .mid("video")
            .direction(MediaDirection::SendRecv)
            .setup("actpass")
            .ice_ufrag("F7gI")
            .ice_pwd("x9cml/YzichV2+XlhiMu8g")
            .ice_candidate("1 1 UDP 2113937151 192.168.1.100 9 typ host")
            .ice_candidate("2 1 UDP 1845501695 203.0.113.100 9 typ srflx raddr 192.168.1.100 rport 9")
            .done()
        .build()?;
    
    println!("WebRTC SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 4: Parsing an SDP string
fn parse_sdp_string() -> Result<(), Box<dyn Error>> {
    println!("Example 4: Parsing an SDP string\n");
    
    let sdp_str = "v=0\r\n\
                  o=alice 2890844526 2890844526 IN IP4 alice.example.com\r\n\
                  s=Audio Call\r\n\
                  c=IN IP4 alice.example.com\r\n\
                  t=0 0\r\n\
                  m=audio 49170 RTP/AVP 0 8 96\r\n\
                  a=rtpmap:0 PCMU/8000\r\n\
                  a=rtpmap:8 PCMA/8000\r\n\
                  a=rtpmap:96 telephone-event/8000\r\n\
                  a=sendrecv";
    
    // Parse the SDP string
    let session = SdpSession::from_str(sdp_str)?;
    
    println!("Parsed SDP Session:");
    println!("Session name: {}", session.session_name);
    println!("Origin: {}", session.origin);
    println!("Number of media streams: {}", session.media_descriptions.len());
    
    if !session.media_descriptions.is_empty() {
        let media = &session.media_descriptions[0];
        println!("Media type: {}", media.media);
        println!("Media port: {}", media.port);
        println!("Media protocol: {}", media.protocol);
        println!("Media formats: {:?}", media.formats);
    }
    
    println!("\nReconstructed SDP:");
    println!("{}\n", session);
    
    Ok(())
}

// Example 5: Modifying an existing SDP
fn modify_existing_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 5: Modifying an existing SDP\n");
    
    // Create a basic SDP
    let sdp = SdpBuilder::new("Original Session")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .done()
        .build()?;
    
    println!("Original SDP:");
    println!("{}\n", sdp);
    
    // Modify the SDP using into_builder()
    let modified_sdp = sdp.into_builder()
        // Change the IP address
        .connection("IN", "IP4", "192.168.1.200")
        // Update the existing audio media section (the builder will handle this by appending media)
        .media_audio(49180, "RTP/AVP")
            .formats(&["0", "8", "96"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("96", "telephone-event/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Add a new video section
        .media_video(51372, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Modified SDP:");
    println!("{}", modified_sdp);
    
    Ok(())
} 