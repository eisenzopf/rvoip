/*! Session Description Protocol (SDP) implementation
 
This module provides a comprehensive implementation of the Session Description Protocol as defined in
[RFC 8866](https://tools.ietf.org/html/rfc8866) with additional support for WebRTC extensions.
SDP is commonly used in VoIP and WebRTC applications to describe multimedia sessions and negotiate
media capabilities.

# Overview

The SDP module consists of:

- **Parser**: A robust parser that converts SDP text to structured data
- **Generator**: Methods to convert SDP structures back to standard format
- **Types**: Data structures representing SDP sessions, media descriptions, and attributes
- **Builder**: Fluent builder API for creating SDP sessions programmatically
- **Macros**: Declarative macro-based syntax for creating SDP sessions

# Approaches to Creating SDP

This library provides two main approaches for creating SDP sessions:

## Builder Pattern

The [`SdpBuilder`] offers a fluent, type-safe API for creating SDP sessions programmatically.
This approach is best for dynamic SDP generation where values are determined at runtime.

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;

let session = SdpBuilder::new("My Session")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")  // Time 0-0 means permanent session
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()
    .expect("Valid SDP");
```

## Macro Pattern

The [`sdp!`] macro offers a concise, declarative syntax for defining SDP sessions.
This approach is best for static SDP configurations that are known at compile time.

```rust
use rvoip_sip_core::sdp;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
use rvoip_sip_core::sdp::attributes::MediaDirection;

let session = sdp! {
    origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
    session_name: "Audio Call",
    connection: ("IN", "IP4", "192.168.1.100"),
    time: ("0", "0"),
    media: {
        type: "audio",
        port: 49170,
        protocol: "RTP/AVP",
        formats: ["0", "8"],
        rtpmap: ("0", "PCMU/8000"),
        rtpmap: ("8", "PCMA/8000"),
        direction: "sendrecv"
    }
}.expect("Valid SDP");
```

# Supported SDP Message Types and Features

## Core SDP (RFC 8866)

| Feature | RFC Reference |
|---------|---------------|
| Version line (`v=`) | [RFC 8866 §5.1](https://tools.ietf.org/html/rfc8866#section-5.1) |
| Origin (`o=`) | [RFC 8866 §5.2](https://tools.ietf.org/html/rfc8866#section-5.2) |
| Session Name (`s=`) | [RFC 8866 §5.3](https://tools.ietf.org/html/rfc8866#section-5.3) |
| Information (`i=`) | [RFC 8866 §5.4](https://tools.ietf.org/html/rfc8866#section-5.4) |
| URI (`u=`) | [RFC 8866 §5.5](https://tools.ietf.org/html/rfc8866#section-5.5) |
| Email (`e=`) | [RFC 8866 §5.6](https://tools.ietf.org/html/rfc8866#section-5.6) |
| Phone (`p=`) | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Connection Data (`c=`) | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Bandwidth (`b=`) | [RFC 8866 §5.8](https://tools.ietf.org/html/rfc8866#section-5.8) |
| Time Description (`t=`) | [RFC 8866 §5.9](https://tools.ietf.org/html/rfc8866#section-5.9) |
| Repeat Times (`r=`) | [RFC 8866 §5.10](https://tools.ietf.org/html/rfc8866#section-5.10) |
| Time Zones (`z=`) | [RFC 8866 §5.11](https://tools.ietf.org/html/rfc8866#section-5.11) |
| Encryption Keys (`k=`) | [RFC 8866 §5.12](https://tools.ietf.org/html/rfc8866#section-5.12) |
| Attributes (`a=`) | [RFC 8866 §5.13](https://tools.ietf.org/html/rfc8866#section-5.13) |
| Media Descriptions (`m=`) | [RFC 8866 §5.14](https://tools.ietf.org/html/rfc8866#section-5.14) |

## Standard Attributes (RFC 8866)

| Attribute | RFC Reference |
|-----------|---------------|
| `rtpmap` | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `fmtp` | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `ptime` | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `maxptime` | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `recvonly`, `sendrecv`, `sendonly`, `inactive` | [RFC 8866 §6.7](https://tools.ietf.org/html/rfc8866#section-6.7) |

## WebRTC Extensions

| Feature | RFC Reference |
|---------|---------------|
| ICE Attributes | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| DTLS-SRTP | [RFC 8842](https://tools.ietf.org/html/rfc8842) |
| Media Stream Identification | [RFC 8830](https://tools.ietf.org/html/rfc8830) |
| BUNDLE Grouping | [RFC 8843](https://tools.ietf.org/html/rfc8843) |
| RTP Header Extensions | [RFC 8285](https://tools.ietf.org/html/rfc8285) |
| RID (Restricted ID) | [RFC 8851](https://tools.ietf.org/html/rfc8851) |
| Simulcast | [RFC 8853](https://tools.ietf.org/html/rfc8853) |
| RTCP Feedback | [RFC 4585](https://tools.ietf.org/html/rfc4585) |
| Data Channels | [RFC 8841](https://tools.ietf.org/html/rfc8841) |

# Detailed Examples

## Parsing SDP

```rust
use std::str::FromStr;
use rvoip_sip_core::types::sdp::SdpSession;

// Parse an SDP string
let sdp_str = "v=0\r\n\
    o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\n\
    s=SDP Seminar\r\n\
    c=IN IP4 224.2.17.12/127\r\n\
    t=0 0\r\n\
    m=audio 49170 RTP/AVP 0\r\n\
    a=rtpmap:0 PCMU/8000\r\n";

let session = SdpSession::from_str(sdp_str).expect("Valid SDP");

// Access session-level information
println!("Session name: {}", session.session_name);
if let Some(connection) = &session.connection_info {
    println!("Connection address: {}", connection.connection_address);
}

// Access media-level information
for media in &session.media_descriptions {
    println!("Media type: {}, port: {}", media.media, media.port);
}
```

## Creating WebRTC SDP with Builder Pattern

The following example creates an SDP offer for a WebRTC session with audio, video, and data channels:

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;

let sdp = SdpBuilder::new("WebRTC Session")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")
    .group("BUNDLE", &["audio", "video", "data"])
    .ice_ufrag("F7gI")
    .ice_pwd("x9cml/YzichV2+XlhiMu8g")
    .fingerprint("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24")
    // Audio media section
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
    // Video media section
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
        .done()
    // Data channel
    .media_application(9, "UDP/DTLS/SCTP")
        .formats(&["webrtc-datachannel"])
        .mid("data")
        .rtcp_mux()
        .attribute("sctp-port", Some("5000"))
        .setup("actpass")
        .ice_ufrag("F7gI")
        .ice_pwd("x9cml/YzichV2+XlhiMu8g")
        .done()
    .build()
    .expect("Valid WebRTC SDP");
```

## Creating SIP SDP with Macro Pattern

The following example creates an SDP for a SIP call with audio and video:

```rust
use rvoip_sip_core::sdp;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
use rvoip_sip_core::sdp::attributes::MediaDirection;

let session = sdp! {
    origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
    session_name: "SIP Call",
    connection: ("IN", "IP4", "192.168.1.100"),
    time: ("0", "0"),
    media: {
        type: "audio",
        port: 49170,
        protocol: "RTP/AVP",
        formats: ["0", "8"],
        rtpmap: ("0", "PCMU/8000"),
        rtpmap: ("8", "PCMA/8000"),
        direction: "sendrecv"
    },
    media: {
        type: "video",
        port: 51372,
        protocol: "RTP/AVP",
        formats: ["96"],
        rtpmap: ("96", "H264/90000"),
        fmtp: ("96", "profile-level-id=42e01f"),
        direction: "sendrecv"
    }
}.expect("Valid SIP SDP");
```

## Creating Multicast SDP with Builder Pattern

For multicast streaming applications:

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;

let sdp = SdpBuilder::new("Multicast Stream")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection_multicast("IN", "IP4", "224.2.36.42", 127, None)
    .time("0", "0")
    .bandwidth("AS", 256)  // 256 kbps Application-Specific bandwidth
    .media_video(5004, "RTP/AVP")
        .formats(&["96"])
        .rtpmap("96", "H264/90000")
        .direction(MediaDirection::SendOnly)
        .bandwidth("AS", 256)
        .done()
    .build()
    .expect("Valid multicast SDP");
```

## Advanced Media Attributes with Macro Pattern

Using advanced media attributes with the macro pattern:

```rust
use rvoip_sip_core::sdp;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
use rvoip_sip_core::sdp::attributes::MediaDirection;

let session = sdp! {
    origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
    session_name: "Advanced Media",
    connection: ("IN", "IP4", "192.168.1.100"),
    time: ("0", "0"),
    media: {
        type: "audio",
        port: 49170,
        protocol: "RTP/AVP",
        formats: ["0", "8", "96"],
        rtpmap: ("0", "PCMU/8000"),
        rtpmap: ("8", "PCMA/8000"),
        rtpmap: ("96", "opus/48000/2"),
        fmtp: ("96", "minptime=10;useinbandfec=1;stereo=1"),
        direction: "sendrecv"
    }
}.expect("Valid SDP with advanced media attributes");
```

# When to Use Each Approach

## Builder Pattern (`SdpBuilder`)

- **Advantages**:
  - Type safety and IDE autocompletion
  - Natural for dynamic SDP generation
  - Method chaining for a fluent API
  - Built-in validation

- **Use when**:
  - Building SDP programmatically at runtime
  - Working with dynamically determined values
  - Creating complex SDPs with multiple media sections
  - Implementing WebRTC or SIP signaling

## Macro Pattern (`sdp!`)

- **Advantages**:
  - Concise, declarative syntax
  - Visually resembles SDP structure
  - Less verbose for simple cases

- **Use when**:
  - Creating static SDP templates
  - Working with known values at compile time
  - Defining simple SDP configurations
  - Readability is a priority

For more information on SDP and related standards, see the [RFC references](#supported-sdp-message-types-and-features).
*/

pub mod parser;
pub mod session;
pub mod media;
pub mod attributes;
pub mod macros;
pub mod builder;
pub mod integration;  // New module for SIP/SDP integration helpers

#[cfg(test)]
mod tests;

pub use parser::parse_sdp;
pub use parser::validate_sdp;
pub use builder::SdpBuilder;
pub use crate::sdp; // Directly use the sdp macro

// For backward compatibility
pub mod media_parser {
    pub use crate::sdp::media::*;
}

// For backward compatibility
pub mod session_parser {
    pub use crate::sdp::session::*;
}

// For backward compatibility
pub mod time_parser {
    pub use crate::sdp::parser::time_parser::*;
}

// Re-exports
pub use attributes::MediaDirection;
pub use integration::*;  // Re-export integration helpers

/// Prelude module
///
/// Import common SDP types and functions.
pub mod prelude {
    pub use super::SdpBuilder;
    pub use super::parse_sdp;
    pub use super::attributes::MediaDirection;
    pub use crate::sdp; // Use the sdp macro from crate root
    pub use super::integration::*;  // Make integration helpers available in prelude
} 