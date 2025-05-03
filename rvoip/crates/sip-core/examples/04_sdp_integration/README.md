# Example 4: SDP Integration

This example demonstrates how to use Session Description Protocol (SDP) with SIP for media negotiation. SDP is used to describe multimedia sessions, including audio and video codecs, network addresses, and other parameters needed for media exchange.

## What You'll Learn

- How to create SDP messages using both the builder pattern and macros
- How to parse and extract information from SDP messages
- How the SDP offer/answer model works for media negotiation
- How to analyze and determine the negotiated media parameters
- How to work with codecs, formats, and their parameters
- How to integrate SDP with SIP messages using the new integration helpers
- How to automatically generate SDP content from SIP message headers

## Running the Example

```bash
# Run the example
cargo run --example 04_sdp_integration

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 04_sdp_integration
```

## Code Walkthrough

The example is divided into six parts:

1. **Creating SDP with the Builder Pattern**
   - Demonstrates building an SDP session step-by-step
   - Shows how to add multiple media types (audio and video)
   - Illustrates setting codecs with rtpmap entries
   - Shows how to access information from the built SDP object

2. **Creating SDP with Macros**
   - Shows the more concise `sdp!` macro syntax for creating SDP
   - Demonstrates setting format-specific parameters (fmtp)
   - Shows how to handle telephone events (DTMF)
   - Illustrates how to extract and analyze the media formats

3. **Parsing SDP from Raw Data**
   - Demonstrates parsing a complete SDP message from raw text
   - Shows how to access various session attributes
   - Demonstrates iterating through media sections and their formats
   - Shows how to handle format parameters

4. **Offer/Answer Model**
   - Implements a complete SDP negotiation scenario
   - Shows how Alice creates an offer with multiple codecs
   - Demonstrates how Bob creates an answer supporting only some codecs
   - Illustrates how to analyze the negotiation results
   - Shows the algorithm for finding common codecs

5. **Integrated SIP/SDP Building**
   - Demonstrates how to use the SDP builder together with the SIP builder
   - Shows how to add an SDP body to SIP requests using the ContentBuilderExt trait
   - Illustrates creating a complete INVITE with SDP offer and 200 OK with answer
   - Shows how to extract and process SDP from SIP messages

6. **Advanced SIP/SDP Integration with Automatic Profiles**
   - Shows how to automatically generate SDP content from SIP message headers
   - Demonstrates creating audio-only, audio+video, and WebRTC SDPs
   - Illustrates how to use predefined media profiles for common configurations
   - Shows how the integration functions extract information from SIP to populate SDP

## Key Concepts

### SDP Structure

SDP (Session Description Protocol) is a format for describing multimedia communication sessions. Its key components include:

- **Session-level attributes**: Information about the whole session (origin, name, timing)
- **Media-level attributes**: Information about specific media streams (audio, video)
- **Codecs and formats**: Payload types and their mappings to actual codecs
- **Network information**: IP addresses, ports, and protocols for media exchange
- **Media direction**: Whether media flows in one direction or both

### SDP Offer/Answer Model

The offer/answer model is the basis for SIP media negotiation:

1. **Offer**: The initiator sends an SDP with all supported media and codecs
2. **Answer**: The recipient responds with an SDP indicating which options it accepts
3. **Negotiation**: Both sides determine the intersection of supported formats
4. **Media establishment**: Media flows using the negotiated parameters

### SIP-SDP Integration

The example demonstrates several approaches to integrating SDP with SIP:

1. **Manual Integration**: Manually creating both SIP and SDP and combining them
2. **Helper Methods**: Using the `sdp_body` method to attach an SDP session to a SIP message
3. **Automatic Profiles**: Using methods like `auto_sdp_audio_body` to generate SDP from SIP information
4. **WebRTC Integration**: Using special helpers for WebRTC-compatible SDP generation

### Common Media Types and Codecs

- **Audio**: PCMU (G.711 μ-law), PCMA (G.711 A-law), telephone-event (DTMF)
- **Video**: H.264, VP8, H.261
- **Application**: Real-time data channels

### Media Directions

- **sendrecv**: Media flows in both directions (default)
- **sendonly**: Sender will transmit but not receive
- **recvonly**: Sender will receive but not transmit
- **inactive**: No media flows in either direction

## Implementation Details

### Integration Module Highlights

The example showcases the new integration module that provides:

1. **Header Extraction**: Automatically extracts SIP headers to populate SDP fields
2. **Common Profiles**: Predefined audio, video, and WebRTC profiles
3. **Builder Extensions**: Convenience methods for SIP-SDP integration
4. **Auto-Generation**: Smart generation of SDP based on SIP message context

### ContentBuilderExt Trait Methods

The example uses several methods from the ContentBuilderExt trait:

- `sdp_body(sdp)`: Adds an SDP session to a SIP message
- `auto_sdp_audio_body(name, port, codecs)`: Generates an audio SDP and adds it
- `auto_sdp_av_body(name, audio_port, video_port, audio_codecs, video_codecs)`: Generates an audio+video SDP
- `auto_sdp_webrtc_body(name, ice_ufrag, ice_pwd, fingerprint, include_video)`: Generates a WebRTC SDP

## Next Steps

Now that you understand SDP integration, you can move on to Example 5 which focuses on authentication and security in SIP communications.