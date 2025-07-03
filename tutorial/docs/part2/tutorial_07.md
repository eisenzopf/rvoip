# Creating SDP Messages

In the previous tutorial, we introduced SDP and its structure. Now, we'll dive deeper into creating more complex SDP messages using the `rvoip-sip-core` library's builder pattern.

## Advanced SDP Creation

The SdpBuilder API provides a fluent interface for creating SDP messages of varying complexity. In this tutorial, we'll explore:

1. Creating sophisticated multi-stream SDP messages
2. Setting up codec-specific parameters
3. Working with bandwidth and quality of service (QoS) parameters
4. Advanced attributes for WebRTC applications
5. Creating SDP answers based on offers

## Building Multi-Stream Sessions

Real-world SIP applications often need to establish sessions with multiple media streams, each with different characteristics. Let's look at how to create an SDP with multiple audio and video streams:

```rust
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
```

This example creates an SDP with four media streams: two audio and two video. Each has different characteristics:
- The primary audio stream offers Opus (high quality) with G.711 fallback
- The secondary audio stream only offers G.711 μ-law
- The main video stream offers both H.264 and VP8 in HD
- The secondary video stream offers H.264 in SD quality

## Working with Codec Parameters

The format parameters (fmtp) attribute allows for detailed configuration of codecs. Each codec has its own specific parameters:

### Opus Audio Parameters

```rust
.fmtp("109", "maxplaybackrate=48000;stereo=1;maxaveragebitrate=256000")
```

These parameters configure:
- Maximum playback rate (48kHz)
- Stereo audio (2 channels)
- Maximum bitrate (256 kbps)

### H.264 Video Parameters

```rust
.fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
```

These parameters specify:
- Profile level ID (Baseline Profile, Level 3.1)
- Packetization mode (1 = Non-Interleaved Mode)

### VP8 Video Parameters

```rust
.fmtp("98", "max-fr=30;max-fs=8160")
```

These parameters set:
- Maximum frame rate (30 fps)
- Maximum frame size (8160 macroblocks, roughly 720p)

## Bandwidth and QoS Parameters

SDP allows you to specify bandwidth constraints for the session or individual media streams:

```rust
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
```

Different bandwidth specifiers serve different purposes:
- **AS**: Application Specific - overall bandwidth for all media components
- **CT**: Conference Total - bandwidth shared by all conference participants
- **TIAS**: Transport Independent Application Specific - most precise way to specify bandwidth

## WebRTC-Specific SDP Features

WebRTC uses SDP for session negotiation but requires additional attributes for features like ICE, DTLS, and RTCP feedback:

```rust
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
```

Key WebRTC features include:
- **ICE candidates** for NAT traversal
- **DTLS fingerprints** for secure key exchange
- **RTCP feedback mechanisms** for congestion control and stream quality
- **RTP header extensions** for additional metadata
- **Bundling** for efficient use of network resources

## Working with Simulcast and SVC

For applications requiring scalable video, SDP can describe simulcast or scalable video coding (SVC):

```rust
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
        .rid("high", rvoip_sip_core::sdp::attributes::rid::RidDirection::Send, &["96"], &[("max-width", "1280"), ("max-height", "720")])
        .rid("medium", rvoip_sip_core::sdp::attributes::rid::RidDirection::Send, &["97"], &[("max-width", "640"), ("max-height", "360")])
        .rid("low", rvoip_sip_core::sdp::attributes::rid::RidDirection::Send, &["98"], &[("max-width", "320"), ("max-height", "180")])
        // Simulcast description
        .simulcast(vec!["high;medium;low".to_string()], Vec::<String>::new())
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;
```

This example creates an SDP that offers three simulcast streams (high, medium, and low resolution) of VP8 video.

## Creating SDP Answers

When responding to an SDP offer, you need to create an SDP answer that selects from the offered capabilities:

```rust
// Assuming we received an offer SDP
let offer = received_sdp;

// Create an answer based on the offer
let answer = offer.into_builder()
    // Update origin with our information
    .origin("bob", "9876543210", "1", "IN", "IP4", "bob.example.com")
    // Update connection with our address
    .connection("IN", "IP4", "bob.example.com")
    // Keep the offered media streams but potentially modify them
    .media_audio(49170, "RTP/AVP")
        // Choose only one of the offered codecs
        .formats(&["0"])
        .rtpmap("0", "PCMU/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .media_video(49172, "RTP/AVP")
        // Choose only H.264 from the offered codecs
        .formats(&["97"])
        .rtpmap("97", "H264/90000")
        .fmtp("97", "profile-level-id=42e01f;packetization-mode=1")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;
```

When creating an answer, it's important to:
1. Only include media types that were in the offer
2. Only select codecs that were offered
3. Ensure compatibility of parameters
4. Set appropriate connection information
5. Match the offered session structure

## SDP Validation and Error Handling

The `build()` method on SdpBuilder performs validation before returning the SdpSession:

```rust
match sdp_builder.build() {
    Ok(sdp) => {
        println!("Valid SDP created:");
        println!("{}", sdp);
    },
    Err(e) => {
        println!("Failed to create valid SDP: {}", e);
        // Handle the error - perhaps fix the issues in the builder
    }
}
```

Common validation failures include:
- Missing required fields (like origin, session name, time description)
- Invalid connection information
- Media sections without formats
- ICE candidates with invalid IP addresses
- Inconsistent media formats and rtpmap entries

## Best Practices for Creating SDP

1. **Always include required fields**: v=, o=, s=, t=, and either session-level or media-level c=
2. **Use specific format identifiers**: Prefer specific dynamic payload types for each codec
3. **Include rtpmap for all dynamic payload types**: Always map payload types >95 with rtpmap
4. **Set appropriate directions**: Be explicit about the media direction (sendrecv, sendonly, recvonly, inactive)
5. **Reuse session-level attributes**: When attributes apply to all media sections, define them at session level
6. **Be consistent with transport**: Ensure protocol compatibility across the entire session
7. **Handle time properly**: Use "0 0" for persistent sessions
8. **Validate before sending**: Always call `build()` to verify the SDP is valid

## Conclusion

Creating effective SDP messages is crucial for establishing compatible and efficient media sessions. The `rvoip-sip-core` SdpBuilder provides a robust API for creating SDP messages of varying complexity, from simple audio calls to sophisticated WebRTC applications with multiple streams, simulcast, and advanced media features.

In the next tutorial, we'll explore how to integrate SDP with SIP messages to establish complete multimedia sessions.
