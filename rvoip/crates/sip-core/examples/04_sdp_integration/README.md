# Example 4: SDP Integration

This example demonstrates how to use Session Description Protocol (SDP) with SIP for media negotiation. SDP is used to describe multimedia sessions, including audio and video codecs, network addresses, and other parameters needed for media exchange.

## What You'll Learn

- How to create SDP messages using both the builder pattern and macros
- How to parse and extract information from SDP messages
- How the SDP offer/answer model works for media negotiation
- How to analyze and determine the negotiated media parameters
- How to work with codecs, formats, and their parameters
- How to interpret media directions (sendrecv, recvonly, etc.)

## Running the Example

```bash
# Run the example
cargo run --example 04_sdp_integration

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 04_sdp_integration
```

## Code Walkthrough

The example is divided into four parts:

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

### Common Media Types and Codecs

- **Audio**: PCMU (G.711 μ-law), PCMA (G.711 A-law), telephone-event (DTMF)
- **Video**: H.264, VP8, H.261
- **Application**: Real-time data channels

### Media Directions

- **sendrecv**: Media flows in both directions (default)
- **sendonly**: Sender will transmit but not receive
- **recvonly**: Sender will receive but not transmit
- **inactive**: No media flows in either direction

## Next Steps

Now that you understand SDP integration, you can move on to Example 5 which focuses on authentication and security in SIP communications.