# SDP Parser and Generator

This module provides a complete implementation for parsing, manipulating, and generating Session Description Protocol (SDP) messages according to [RFC 8866](https://tools.ietf.org/html/rfc8866) (which obsoletes [RFC 4566](https://tools.ietf.org/html/rfc4566)) and various extensions.

## Overview

The SDP module consists of:

- **Parser**: A robust parser that converts SDP text to structured data
- **Types**: Data structures representing SDP sessions, media descriptions, and attributes
- **Attributes**: Parsers for various standard and extended SDP attributes
- **Builder**: Fluent builder API for creating SDP sessions programmatically
- **Macros**: Declarative macro-based syntax for creating SDP sessions
- **Generator**: Methods to convert SDP structures back to standard format

## Compliance

### Core SDP (RFC 8866)

| Feature | Status | RFC Reference |
|---------|--------|---------------|
| Version line (`v=`) | ✅ Fully Compliant | [RFC 8866 §5.1](https://tools.ietf.org/html/rfc8866#section-5.1) |
| Origin (`o=`) | ✅ Fully Compliant | [RFC 8866 §5.2](https://tools.ietf.org/html/rfc8866#section-5.2) |
| Session Name (`s=`) | ✅ Fully Compliant | [RFC 8866 §5.3](https://tools.ietf.org/html/rfc8866#section-5.3) |
| Information (`i=`) | ✅ Fully Compliant | [RFC 8866 §5.4](https://tools.ietf.org/html/rfc8866#section-5.4) |
| URI (`u=`) | ✅ Fully Compliant | [RFC 8866 §5.5](https://tools.ietf.org/html/rfc8866#section-5.5) |
| Email (`e=`) | ✅ Fully Compliant | [RFC 8866 §5.6](https://tools.ietf.org/html/rfc8866#section-5.6) |
| Phone (`p=`) | ✅ Fully Compliant | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Connection Data (`c=`) | ✅ Fully Compliant | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Bandwidth (`b=`) | ✅ Fully Compliant | [RFC 8866 §5.8](https://tools.ietf.org/html/rfc8866#section-5.8) |
| Time Description (`t=`) | ✅ Fully Compliant | [RFC 8866 §5.9](https://tools.ietf.org/html/rfc8866#section-5.9) |
| Repeat Times (`r=`) | ✅ Fully Compliant | [RFC 8866 §5.10](https://tools.ietf.org/html/rfc8866#section-5.10) |
| Time Zones (`z=`) | ✅ Basic Support | [RFC 8866 §5.11](https://tools.ietf.org/html/rfc8866#section-5.11) |
| Encryption Keys (`k=`) | ✅ Supported | [RFC 8866 §5.12](https://tools.ietf.org/html/rfc8866#section-5.12) |
| Attributes (`a=`) | ✅ Fully Compliant | [RFC 8866 §5.13](https://tools.ietf.org/html/rfc8866#section-5.13) |
| Media Descriptions (`m=`) | ✅ Fully Compliant | [RFC 8866 §5.14](https://tools.ietf.org/html/rfc8866#section-5.14) |

### Media Formats (RFC 8866, 3264, 4566)

| Media Type | Status | Notes |
|------------|--------|-------|
| Audio | ✅ Fully Supported | Complete support for audio media types and formats |
| Video | ✅ Fully Supported | Complete support for video media types and formats |
| Application | ✅ Fully Supported | Includes data channels with WebRTC extensions |
| Text | ✅ Supported | Basic support |
| Message | ✅ Supported | Basic support |
| Non-standard types | ✅ Supported | Validated as tokens |

### Standard Attributes (RFC 8866)

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `rtpmap` | ✅ Fully Compliant | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `fmtp` | ✅ Fully Compliant | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `ptime` | ✅ Fully Compliant | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `maxptime` | ✅ Fully Compliant | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `recvonly`, `sendrecv`, `sendonly`, `inactive` | ✅ Fully Compliant | [RFC 8866 §6.7](https://tools.ietf.org/html/rfc8866#section-6.7) |

### WebRTC Extensions

| Feature | Status | RFC Reference |
|---------|--------|---------------|
| ICE Attributes | ✅ Fully Compliant | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| DTLS-SRTP | ✅ Fully Compliant | [RFC 8842](https://tools.ietf.org/html/rfc8842) |
| Media Stream Identification | ✅ Fully Compliant | [RFC 8830](https://tools.ietf.org/html/rfc8830) |
| BUNDLE Grouping | ✅ Fully Compliant | [RFC 8843](https://tools.ietf.org/html/rfc8843) |
| RTP Header Extensions | ✅ Fully Compliant | [RFC 8285](https://tools.ietf.org/html/rfc8285) |
| RID (Restricted ID) | ✅ Fully Compliant | [RFC 8851](https://tools.ietf.org/html/rfc8851) |
| Simulcast | ✅ Fully Compliant | [RFC 8853](https://tools.ietf.org/html/rfc8853) |
| RTCP Feedback | ✅ Fully Compliant | [RFC 4585](https://tools.ietf.org/html/rfc4585) |

### WebRTC Attributes

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `candidate` | ✅ Fully Compliant | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-ufrag` | ✅ Fully Compliant | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-pwd` | ✅ Fully Compliant | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-options` | ✅ Fully Compliant | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `fingerprint` | ✅ Fully Compliant | [RFC 8122](https://tools.ietf.org/html/rfc8122) |
| `setup` | ✅ Fully Compliant | [RFC 4145](https://tools.ietf.org/html/rfc4145) |
| `mid` | ✅ Fully Compliant | [RFC 8843](https://tools.ietf.org/html/rfc8843) |
| `group` | ✅ Fully Compliant | [RFC 5888](https://tools.ietf.org/html/rfc5888) |
| `msid` | ✅ Fully Compliant | [RFC 8830](https://tools.ietf.org/html/rfc8830) |
| `rtcp-fb` | ✅ Fully Compliant | [RFC 4585](https://tools.ietf.org/html/rfc4585) |
| `rtcp-mux` | ✅ Fully Compliant | [RFC 5761](https://tools.ietf.org/html/rfc5761) |
| `extmap` | ✅ Fully Compliant | [RFC 8285](https://tools.ietf.org/html/rfc8285) |
| `rid` | ✅ Fully Compliant | [RFC 8851](https://tools.ietf.org/html/rfc8851) |
| `simulcast` | ✅ Fully Compliant | [RFC 8853](https://tools.ietf.org/html/rfc8853) |
| `ssrc` | ✅ Fully Compliant | [RFC 5576](https://tools.ietf.org/html/rfc5576) |
| `end-of-candidates` | ✅ Fully Compliant | [RFC 8840](https://tools.ietf.org/html/rfc8840) |

### Data Channel Support (WebRTC)

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `sctp-port` | ✅ Fully Compliant | [RFC 8841](https://tools.ietf.org/html/rfc8841) |
| `max-message-size` | ✅ Fully Compliant | [RFC 8841](https://tools.ietf.org/html/rfc8841) |
| `sctpmap` | ✅ Fully Compliant | [RFC 4960](https://tools.ietf.org/html/rfc4960) (deprecated by RFC 8841) |

## Usage Examples

### Parsing SDP

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
```

### Accessing SDP Information

```rust
// Access session-level information
println!("Session name: {}", session.session_name);
if let Some(connection) = &session.connection_info {
    println!("Connection address: {}", connection.connection_address);
}

// Access media-level information
for media in &session.media_descriptions {
    println!("Media type: {}, port: {}", media.media, media.port);
    
    // Get specific attributes
    if let Some(rtpmap) = media.get_rtpmap("0") {
        println!("Codec: {}/{}", rtpmap.encoding_name, rtpmap.clock_rate);
    }
}
```

### Creating SDP with the Builder Pattern (Recommended)

The SdpBuilder provides a fluent interface for creating SDP sessions programmatically:

```rust
use rvoip_sip_core::sdp_prelude::*;

// Create an SDP session with the builder
let sdp = SdpBuilder::new("My Session")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")  // Time 0-0 means permanent session
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .direction(MediaDirection::SendRecv)
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .done()
    .build()
    .expect("Valid SDP");
```

### Creating SDP with the Declarative Macro

The `sdp!` macro offers a concise, declarative way to create SDP sessions:

```rust
use rvoip_sip_core::sdp;
use rvoip_sip_core::sdp_prelude::*;

// Create an SDP session with the sdp! macro
let sdp_result = sdp! {
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
};

let sdp = sdp_result.expect("Valid SDP");
```

### Creating SDP Manually (Low-level API)

```rust
use rvoip_sip_core::types::sdp::{SdpSession, Origin, MediaDescription, ConnectionData};
use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute};
use rvoip_sip_core::sdp::attributes::MediaDirection;

// Create a basic session
let origin = Origin {
    username: "-".to_string(),
    sess_id: "1234567890".to_string(),
    sess_version: "2".to_string(),
    net_type: "IN".to_string(),
    addr_type: "IP4".to_string(),
    unicast_address: "192.168.1.100".to_string(),
};

let mut session = SdpSession::new(origin, "VoIP Call");

// Add connection information
let conn = ConnectionData {
    net_type: "IN".to_string(),
    addr_type: "IP4".to_string(),
    connection_address: "192.168.1.100".to_string(),
    ttl: None,
    multicast_count: None,
};
session = session.with_connection_data(conn);

// Add a time description
session.time_descriptions.push(TimeDescription {
    start_time: "0".to_string(),
    stop_time: "0".to_string(),
    repeat_times: vec![],
});

// Add a media description
let mut audio_media = MediaDescription::new(
    "audio".to_string(), 
    49170, 
    "RTP/AVP".to_string(), 
    vec!["0".to_string(), "8".to_string()]
);

// Add media attributes
audio_media.generic_attributes.push(ParsedAttribute::RtpMap(RtpMapAttribute {
    payload_type: 0,
    encoding_name: "PCMU".to_string(),
    clock_rate: 8000,
    encoding_params: None,
}));

// Set media direction
audio_media.direction = Some(MediaDirection::SendRecv);

session.add_media(audio_media);
```

## Choosing the Right Approach

For creating SDP messages, we provide three approaches:

1. **Builder Pattern** (Recommended): Provides a fluent, type-safe API for creating complex SDP sessions programmatically. Best for dynamic SDP generation where values are determined at runtime.

2. **Declarative Macro**: Offers a concise, declarative syntax for defining SDP sessions with minimal boilerplate. Best for static SDP configurations known at compile time.

3. **Manual Construction**: Gives complete control over the SDP structure. Most verbose but allows full customization for advanced use cases.

## Compliance Testing

The implementation has thorough test coverage (over 1,300 tests) ensuring RFC compliance including:

- Complete session-level and media-level attribute parsing
- Validation of required fields and field ordering
- Support for IPv4, IPv6, and multicast addresses
- WebRTC-specific attributes and extensions
- Handling of malformed SDP messages with appropriate errors
- Round-trip testing (parse → serialize → parse)

## Known Limitations

1. **Strict Validation in Some Areas**: The parser implements strict validation for certain fields like version number, which must be "0" according to RFC 8866.

2. **IP Address Validation**: While the parser handles IPv4, IPv6, and multicast addresses, its validation is more lenient than strict RFC requirements in some cases, accepting addresses that might not be technically valid.

3. **Limited SDP Munging Helpers**: Advanced WebRTC operations that require SDP manipulation (munging) don't have dedicated helper methods, though all the necessary parsing and structure is in place.

4. **Minimal Documentation for Custom Attributes**: While the library supports parsing unknown/custom attributes, there's limited guidance on how to extend for custom attributes.

5. **Performance Considerations**: For extremely large SDP messages or high-frequency parsing scenarios, the library prioritizes correctness over maximum performance.

## Extending the Library

The library follows an extensible pattern that makes it straightforward to add support for new attributes:

1. Define any necessary structured types in the appropriate module
2. Add parser function in the relevant attribute module
3. Add a variant to the `ParsedAttribute` enum
4. Update the `parse_attribute` function in `parser.rs`
5. Add display formatting in the `Display` implementation

## References

- [RFC 8866: Session Description Protocol (SDP)](https://tools.ietf.org/html/rfc8866)
- [RFC 3264: Offer/Answer Model with SDP](https://tools.ietf.org/html/rfc3264)
- [RFC 8839: ICE for SDP](https://tools.ietf.org/html/rfc8839)
- [RFC 8840: Trickle ICE for SDP](https://tools.ietf.org/html/rfc8840)
- [RFC 8841: SDP-based Data Channel Negotiation](https://tools.ietf.org/html/rfc8841)
- [RFC 8842: DTLS-SRTP for SDP](https://tools.ietf.org/html/rfc8842)
- [RFC 8843: BUNDLE for SDP](https://tools.ietf.org/html/rfc8843)
- [RFC 8851: RID for SDP](https://tools.ietf.org/html/rfc8851)
- [RFC 8853: Simulcast for SDP](https://tools.ietf.org/html/rfc8853)
- [RFC 5576: Source-Specific Media Attributes in SDP](https://tools.ietf.org/html/rfc5576)
- [RFC 4585: Extended RTP Profile for RTCP-Based Feedback](https://tools.ietf.org/html/rfc4585)
- [RFC 5761: Multiplexing RTP and RTCP](https://tools.ietf.org/html/rfc5761) 