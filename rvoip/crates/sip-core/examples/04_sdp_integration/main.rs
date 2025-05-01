//! SDP Integration Example
//!
//! This example demonstrates how to work with Session Description Protocol (SDP)
//! for media negotiation within SIP sessions.

use bytes::Bytes;
use rvoip_sip_core::{prelude::*, sdp_prelude::*};
use tracing::{debug, info};

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core SDP Integration Example");
    
    // Example 1: Creating SDP with the builder pattern
    create_sdp_with_builder();
    
    // Example 2: Creating SDP with macros
    create_sdp_with_macros();
    
    // Example 3: Parsing SDP from raw data
    parse_sdp_from_data();
    
    // Example 4: Offer/Answer model
    sdp_offer_answer_example();
    
    info!("All examples completed successfully!");
}

/// Example 1: Creating SDP with the builder pattern
fn create_sdp_with_builder() {
    info!("Example 1: Creating SDP with builder pattern");
    
    // Create an SDP session with the builder pattern
    let sdp = SdpBuilder::new("Audio/Video Call")
        // Origin: username, session id, session version, network type, address type, address
        .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.168.1.100")
        // Connection information
        .connection("IN", "IP4", "192.168.1.100")
        // Time the session is active (0 0 means always active)
        .time("0", "0")
        // Add an audio media description
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"]) // PCMU and PCMA
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Add a video media description
        .media_video(49174, "RTP/AVP")
            .formats(&["31"])  // H.261
            .rtpmap("31", "H261/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build();
    
    // Convert to string and display
    let sdp_str = sdp.to_string();
    info!("Created SDP with builder:\n{}", sdp_str);
    
    // Demonstrate how to access parts of the SDP
    info!("Session name: {}", sdp.session_name());
    
    // Access media descriptions
    if sdp.media().len() >= 2 {
        info!("First media (audio): port={}, protocol={}", sdp.media()[0].port(), sdp.media()[0].protocol());
        info!("Second media (video): port={}, protocol={}", sdp.media()[1].port(), sdp.media()[1].protocol());
    }
}

/// Example 2: Creating SDP with macros
fn create_sdp_with_macros() {
    info!("Example 2: Creating SDP with macros");
    
    // Create an SDP session with the sdp! macro
    let sdp = sdp! {
        session_name: "Audio Call",
        origin: ("bob", "1234567890", "2", "IN", "IP4", "192.168.1.200"),
        connection: ("IN", "IP4", "192.168.1.200"),
        time: ("0", "0"),
        media: {
            type: "audio",
            port: 49180,
            protocol: "RTP/AVP",
            formats: ["0", "8", "101"],
            rtpmap: ("0", "PCMU/8000"),
            rtpmap: ("8", "PCMA/8000"),
            rtpmap: ("101", "telephone-event/8000"),
            fmtp: ("101", "0-16"),
            direction: "sendrecv"
        }
    };
    
    // Convert to string and display
    let sdp_str = sdp.to_string();
    info!("Created SDP with macro:\n{}", sdp_str);
    
    // Extract media formats
    if let Some(audio_media) = sdp.media().first() {
        info!("Audio formats: {:?}", audio_media.formats());
        
        // Get individual rtpmap entries
        for fmt in audio_media.formats() {
            if let Some(rtpmap) = audio_media.rtpmap(fmt) {
                info!("Format {} maps to: {}", fmt, rtpmap);
            }
        }
        
        // Check for telephone-event payload type
        if audio_media.has_format("101") {
            if let Some(fmtp) = audio_media.fmtp("101") {
                info!("Telephone events configuration: {}", fmtp);
            }
        }
    }
}

/// Example 3: Parsing SDP from raw data
fn parse_sdp_from_data() {
    info!("Example 3: Parsing SDP from raw data");
    
    // Raw SDP data as a string
    let sdp_data = "\
        v=0\r\n\
        o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\n\
        s=SDP Seminar\r\n\
        i=A Seminar on the session description protocol\r\n\
        u=http://www.example.com/seminars/sdp.pdf\r\n\
        e=j.doe@example.com (Jane Doe)\r\n\
        c=IN IP4 224.2.17.12/127\r\n\
        t=2873397496 2873404696\r\n\
        a=recvonly\r\n\
        m=audio 49170 RTP/AVP 0\r\n\
        a=rtpmap:0 PCMU/8000\r\n\
        m=video 51372 RTP/AVP 99\r\n\
        a=rtpmap:99 H264/90000\r\n\
        a=fmtp:99 profile-level-id=42e01f;packetization-mode=1\r\n";
    
    // Parse the SDP
    let sdp = parse_sdp(&Bytes::from(sdp_data)).expect("Failed to parse SDP");
    
    // Display information about the parsed SDP
    info!("Parsed SDP session name: {}", sdp.session_name());
    info!("Session info: {}", sdp.information().unwrap_or("None"));
    info!("Session time: start={}, end={}", sdp.time()[0].start_time(), sdp.time()[0].end_time());
    
    // Get information about each media stream
    for (i, media) in sdp.media().iter().enumerate() {
        info!("Media {}: {} on port {}", i+1, media.media_type(), media.port());
        
        for fmt in media.formats() {
            if let Some(rtpmap) = media.rtpmap(fmt) {
                info!("  Format {}: {}", fmt, rtpmap);
                
                // Check for fmtp parameters
                if let Some(fmtp) = media.fmtp(fmt) {
                    info!("    Parameters: {}", fmtp);
                }
            }
        }
    }
}

/// Example 4: Offer/Answer model
fn sdp_offer_answer_example() {
    info!("Example 4: SDP Offer/Answer model");
    
    // Scenario: Alice makes a call to Bob with audio and video
    // 1. Alice creates an SDP offer
    // 2. Bob receives the offer and creates an answer
    // 3. Alice receives the answer and establishes media
    
    // Step 1: Alice creates an SDP offer
    info!("Step 1: Alice creates an SDP offer");
    
    let alice_offer = sdp! {
        session_name: "Alice's Call",
        origin: ("alice", "1234567890", "1", "IN", "IP4", "192.168.1.100"),
        connection: ("IN", "IP4", "192.168.1.100"),
        time: ("0", "0"),
        media: {
            type: "audio",
            port: 49170,
            protocol: "RTP/AVP",
            formats: ["0", "8", "101"],
            rtpmap: ("0", "PCMU/8000"),
            rtpmap: ("8", "PCMA/8000"),
            rtpmap: ("101", "telephone-event/8000"),
            fmtp: ("101", "0-16"),
            direction: "sendrecv"
        },
        media: {
            type: "video",
            port: 49180,
            protocol: "RTP/AVP",
            formats: ["99", "100"],
            rtpmap: ("99", "H264/90000"),
            rtpmap: ("100", "VP8/90000"),
            fmtp: ("99", "profile-level-id=42e01f"),
            direction: "sendrecv"
        }
    };
    
    info!("Alice's SDP offer:\n{}", alice_offer.to_string());
    
    // Step 2: Bob receives the offer and creates an answer
    info!("Step 2: Bob creates an SDP answer");
    
    // Bob can only handle audio with PCMU and video with H264
    let bob_answer = sdp! {
        session_name: "Bob's Answer",
        origin: ("bob", "9876543210", "1", "IN", "IP4", "192.168.1.200"),
        connection: ("IN", "IP4", "192.168.1.200"),
        time: ("0", "0"),
        media: {
            type: "audio",
            port: 49180,
            protocol: "RTP/AVP",
            formats: ["0", "101"],  // Only supporting PCMU
            rtpmap: ("0", "PCMU/8000"),
            rtpmap: ("101", "telephone-event/8000"),
            fmtp: ("101", "0-16"),
            direction: "sendrecv"
        },
        media: {
            type: "video",
            port: 49190,
            protocol: "RTP/AVP",
            formats: ["99"],  // Only supporting H264
            rtpmap: ("99", "H264/90000"),
            fmtp: ("99", "profile-level-id=42e01f"),
            direction: "sendrecv"
        }
    };
    
    info!("Bob's SDP answer:\n{}", bob_answer.to_string());
    
    // Step 3: Analyze the negotiation result
    info!("Step 3: Analyzing the negotiation result");
    
    // Find the negotiated codecs for audio
    let alice_audio = alice_offer.media().iter().find(|m| m.media_type() == "audio").unwrap();
    let bob_audio = bob_answer.media().iter().find(|m| m.media_type() == "audio").unwrap();
    
    let common_audio_formats: Vec<&str> = alice_audio.formats().iter()
        .filter(|fmt| bob_audio.has_format(fmt))
        .copied()
        .collect();
    
    info!("Negotiated audio codecs: {:?}", common_audio_formats);
    
    // Find the negotiated codecs for video
    let alice_video = alice_offer.media().iter().find(|m| m.media_type() == "video").unwrap();
    let bob_video = bob_answer.media().iter().find(|m| m.media_type() == "video").unwrap();
    
    let common_video_formats: Vec<&str> = alice_video.formats().iter()
        .filter(|fmt| bob_video.has_format(fmt))
        .copied()
        .collect();
    
    info!("Negotiated video codecs: {:?}", common_video_formats);
    
    // Final negotiated session parameters
    info!("Final negotiated session:");
    info!("  Audio: PCMU on ports {} (Alice) and {} (Bob)",
         alice_audio.port(), bob_audio.port());
    info!("  Video: H264 on ports {} (Alice) and {} (Bob)",
         alice_video.port(), bob_video.port());
    
    // Check directions to ensure they're compatible
    if alice_audio.direction() == MediaDirection::SendRecv && 
       bob_audio.direction() == MediaDirection::SendRecv {
        info!("Audio is bidirectional");
    }
    
    if alice_video.direction() == MediaDirection::SendRecv && 
       bob_video.direction() == MediaDirection::SendRecv {
        info!("Video is bidirectional");
    }
} 