// Example code for Tutorial 07: Creating SDP Messages
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::sdp::attributes::rid::RidDirection;
use rvoip_sip_core::types::sdp::SdpSession;
use std::error::Error;
use std::str::FromStr;

fn main() -> Result<(), Box<dyn Error>> {
    println!("Tutorial 07: Creating SDP Messages\n");
    
    // Example 1: Multi-Stream SDP
    create_multi_stream_sdp()?;
    
    // Example 2: SDP with Bandwidth and QoS Parameters
    create_bandwidth_sdp()?;
    
    // Example 3: WebRTC-specific SDP
    create_webrtc_sdp()?;
    
    // Example 4: Simulcast SDP
    create_simulcast_sdp()?;
    
    // Example 5: Creating an SDP Answer from an Offer
    create_sdp_answer()?;
    
    // Example 6: Error Handling in SDP Creation
    demonstrate_error_handling()?;
    
    Ok(())
}

// Example 1: Creating an SDP with multiple audio and video streams
fn create_multi_stream_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 1: Multi-Stream SDP\n");
    
    let sdp = SdpBuilder::new("Multi-Stream Session")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        // Primary audio stream (high quality)
        .media_audio(49170, "RTP/AVP")
            .formats(&["109", "0", "8"])
            .rtpmap("109", "opus/48000/2")
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .fmtp("109", "maxplaybackrate=48000;stereo=1;maxaveragebitrate=256000")
            .ptime(20)
            .direction(MediaDirection::SendRecv)
            .done()
        // Secondary audio stream (backup narrowband)
        .media_audio(49172, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Main video stream (HD)
        .media_video(49174, "RTP/AVP")
            .formats(&["97", "98"])
            .rtpmap("97", "H264/90000")
            .rtpmap("98", "VP8/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .fmtp("98", "max-fr=30;max-fs=8160")
            .direction(MediaDirection::SendRecv)
            .done()
        // Secondary video stream (SD)
        .media_video(49176, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e00c;packetization-mode=1")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Multi-Stream SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 2: Creating an SDP with bandwidth and QoS parameters
fn create_bandwidth_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 2: SDP with Bandwidth and QoS Parameters\n");
    
    let sdp = SdpBuilder::new("Session with Bandwidth")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        // Session-level bandwidth constraint
        .bandwidth("AS", 1024)  // 1024 kbps total bandwidth
        .media_audio(49170, "RTP/AVP")
            .formats(&["109"])
            .rtpmap("109", "opus/48000/2")
            // Media-level bandwidth constraint
            .bandwidth("TIAS", 128000)  // 128 kbps for audio (Transport Independent Application Specific)
            .done()
        .media_video(49172, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            // Media-level bandwidth constraint
            .bandwidth("AS", 896)  // 896 kbps for video (Application Specific)
            .bandwidth("TIAS", 896000)  // Same in Transport Independent Application Specific units
            .done()
        .build()?;
    
    println!("SDP with Bandwidth Parameters:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 3: Creating a WebRTC-specific SDP with ICE, DTLS, and RTCP feedback
fn create_webrtc_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 3: WebRTC-specific SDP\n");
    
    let sdp = SdpBuilder::new("WebRTC Session")
        .origin("webrtc", "2890844527", "1", "IN", "IP4", "0.0.0.0")
        .connection("IN", "IP4", "0.0.0.0")
        .time("0", "0")
        // Session-level WebRTC attributes
        .group("BUNDLE", &["audio", "video"])  // Bundle audio and video on same transport
        .ice_ufrag("8hhY")
        .ice_pwd("asd88fgpdd777uzjYhagZg")
        .fingerprint("sha-256", "39:4A:09:1E:0E:27:00:19:5D:30:9A:34:3C:1A:EB:69:43:33:51:35:AE:8F:EC:56:4C:35:6A:A7:41:3A:14:3C")
        .media_audio(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["111", "103"])
            .rtpmap("111", "opus/48000/2")
            .rtpmap("103", "ISAC/16000")
            .fmtp("111", "minptime=10;useinbandfec=1")
            .mid("audio")
            .rtcp_mux()  // Multiplex RTP and RTCP on same port
            .rtcp_fb("111", "nack", None::<String>)  // NACK feedback for Opus
            .direction(MediaDirection::SendRecv)
            .setup("actpass")  // DTLS role
            .ice_ufrag("8hhY")
            .ice_pwd("asd88fgpdd777uzjYhagZg")
            .ice_candidate("1 1 UDP 2113937151 192.168.1.100 9 typ host")
            .extmap(1, None::<String>, "urn:ietf:params:rtp-hdrext:ssrc-audio-level", None::<String>)
            .done()
        .media_video(9, "UDP/TLS/RTP/SAVPF")
            .formats(&["96", "97"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "rtx/90000")
            .fmtp("97", "apt=96")  // RTX repair format for VP8
            .mid("video")
            .rtcp_mux()
            .rtcp_fb("96", "nack", None::<String>)  // Negative acknowledgment for VP8
            .rtcp_fb("96", "nack", Some("pli"))  // Picture loss indication for VP8
            .rtcp_fb("96", "ccm", Some("fir"))  // Full intra request for VP8
            .direction(MediaDirection::SendRecv)
            .setup("actpass")
            .ice_ufrag("8hhY")
            .ice_pwd("asd88fgpdd777uzjYhagZg")
            .ice_candidate("1 1 UDP 2113937151 192.168.1.100 9 typ host")
            .extmap(2, None::<String>, "urn:ietf:params:rtp-hdrext:toffset", None::<String>)
            .done()
        .build()?;
    
    println!("WebRTC SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 4: Creating an SDP with simulcast capabilities
fn create_simulcast_sdp() -> Result<(), Box<dyn Error>> {
    println!("Example 4: Simulcast SDP\n");
    
    let sdp = SdpBuilder::new("Simulcast Session")
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        .media_video(49174, "RTP/AVP")
            .formats(&["96", "97", "98"])
            .rtpmap("96", "VP8/90000")
            .rtpmap("97", "VP8/90000")
            .rtpmap("98", "VP8/90000")
            // RID (Restriction Identifiers) for simulcast streams
            .rid("high", RidDirection::Send, &["96"], &[("max-width", "1280"), ("max-height", "720")])
            .rid("medium", RidDirection::Send, &["97"], &[("max-width", "640"), ("max-height", "360")])
            .rid("low", RidDirection::Send, &["98"], &[("max-width", "320"), ("max-height", "180")])
            // Simulcast description
            .simulcast(vec!["high;medium;low".to_string()], Vec::<String>::new())
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Simulcast SDP:");
    println!("{}\n", sdp);
    
    Ok(())
}

// Example 5: Creating an SDP answer from an offer
fn create_sdp_answer() -> Result<(), Box<dyn Error>> {
    println!("Example 5: Creating an SDP Answer from an Offer\n");
    
    // First, let's create an offer SDP to work with
    let offer_sdp_str = "v=0\r\n\
                        o=alice 2890844526 2890844526 IN IP4 alice.example.com\r\n\
                        s=Offer Session\r\n\
                        c=IN IP4 alice.example.com\r\n\
                        t=0 0\r\n\
                        m=audio 49170 RTP/AVP 0 8 96\r\n\
                        a=rtpmap:0 PCMU/8000\r\n\
                        a=rtpmap:8 PCMA/8000\r\n\
                        a=rtpmap:96 opus/48000/2\r\n\
                        a=sendrecv\r\n\
                        m=video 49172 RTP/AVP 97 98\r\n\
                        a=rtpmap:97 H264/90000\r\n\
                        a=rtpmap:98 VP8/90000\r\n\
                        a=fmtp:97 profile-level-id=42e01f;packetization-mode=1\r\n\
                        a=sendrecv";
    
    // Parse the offer
    let offer = SdpSession::from_str(offer_sdp_str)?;
    
    println!("Original Offer SDP:");
    println!("{}\n", offer);
    
    // Create the answer based on the offer
    let answer = SdpBuilder::new("Answer Session")
        .origin("bob", "9876543210", "1", "IN", "IP4", "bob.example.com")
        .connection("IN", "IP4", "bob.example.com")
        .time("0", "0")
        // Audio media section - accept only PCMU codec
        .media_audio(49180, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Video media section - accept only H.264 codec
        .media_video(49182, "RTP/AVP")
            .formats(&["97"])
            .rtpmap("97", "H264/90000")
            .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    println!("Answer SDP:");
    println!("{}\n", answer);
    
    Ok(())
}

// Example 6: Demonstrating error handling during SDP creation
fn demonstrate_error_handling() -> Result<(), Box<dyn Error>> {
    println!("Example 6: Error Handling in SDP Creation\n");
    
    // Create an invalid SDP by omitting required fields
    let invalid_sdp_result = SdpBuilder::new("Invalid Session")
        // Missing origin
        // Missing connection information
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            // Missing formats
            .done()
        .build();
    
    match invalid_sdp_result {
        Ok(sdp) => {
            println!("Unexpectedly created valid SDP:");
            println!("{}", sdp);
        },
        Err(e) => {
            println!("Expected error detected:");
            println!("{}\n", e);
            
            // Now fix the issues and create a valid SDP
            let valid_sdp = SdpBuilder::new("Fixed Session")
                .origin("-", "12345", "1", "IN", "IP4", "example.com")
                .connection("IN", "IP4", "example.com")
                .time("0", "0")
                .media_audio(49170, "RTP/AVP")
                    .formats(&["0"])
                    .rtpmap("0", "PCMU/8000")
                    .done()
                .build()?;
            
            println!("Fixed Valid SDP:");
            println!("{}", valid_sdp);
        }
    }
    
    Ok(())
} 