# SDP Parser and Generator

This module provides a complete implementation for parsing, manipulating, and generating Session Description Protocol (SDP) messages according to [RFC 8866](https://tools.ietf.org/html/rfc8866) (which obsoletes [RFC 4566](https://tools.ietf.org/html/rfc4566)) and various extensions.

## Overview

The SDP module consists of:

- **Parser**: A robust parser that converts SDP text to structured data
- **Types**: Data structures representing SDP sessions, media descriptions, and attributes
- **Attributes**: Parsers for various standard and extended SDP attributes
- **Generator**: Methods to convert SDP structures back to standard format

## Compliance

### Core SDP (RFC 8866)

| Feature | Status | RFC Reference |
|---------|--------|---------------|
| Version line (`v=`) | ✅ Supported | [RFC 8866 §5.1](https://tools.ietf.org/html/rfc8866#section-5.1) |
| Origin (`o=`) | ✅ Supported | [RFC 8866 §5.2](https://tools.ietf.org/html/rfc8866#section-5.2) |
| Session Name (`s=`) | ✅ Supported | [RFC 8866 §5.3](https://tools.ietf.org/html/rfc8866#section-5.3) |
| Information (`i=`) | ✅ Supported | [RFC 8866 §5.4](https://tools.ietf.org/html/rfc8866#section-5.4) |
| URI (`u=`) | ✅ Supported | [RFC 8866 §5.5](https://tools.ietf.org/html/rfc8866#section-5.5) |
| Email (`e=`) | ✅ Supported | [RFC 8866 §5.6](https://tools.ietf.org/html/rfc8866#section-5.6) |
| Phone (`p=`) | ✅ Supported | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Connection Data (`c=`) | ✅ Supported | [RFC 8866 §5.7](https://tools.ietf.org/html/rfc8866#section-5.7) |
| Bandwidth (`b=`) | ✅ Supported | [RFC 8866 §5.8](https://tools.ietf.org/html/rfc8866#section-5.8) |
| Time Description (`t=`) | ✅ Supported | [RFC 8866 §5.9](https://tools.ietf.org/html/rfc8866#section-5.9) |
| Repeat Times (`r=`) | ✅ Supported | [RFC 8866 §5.10](https://tools.ietf.org/html/rfc8866#section-5.10) |
| Time Zones (`z=`) | ✅ Basic Support | [RFC 8866 §5.11](https://tools.ietf.org/html/rfc8866#section-5.11) |
| Encryption Keys (`k=`) | ✅ Supported | [RFC 8866 §5.12](https://tools.ietf.org/html/rfc8866#section-5.12) |
| Attributes (`a=`) | ✅ Supported | [RFC 8866 §5.13](https://tools.ietf.org/html/rfc8866#section-5.13) |
| Media Descriptions (`m=`) | ✅ Supported | [RFC 8866 §5.14](https://tools.ietf.org/html/rfc8866#section-5.14) |

### Media Formats (RFC 8866, 3264, 4566)

| Media Type | Status | Notes |
|------------|--------|-------|
| Audio | ✅ Supported | Full support for audio media types and formats |
| Video | ✅ Supported | Full support for video media types and formats |
| Application | ✅ Supported | Includes data channels |
| Text | ✅ Supported | |
| Message | ✅ Supported | |
| Non-standard types | ✅ Supported | Validated as tokens |

### Standard Attributes (RFC 8866)

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `rtpmap` | ✅ Supported | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `fmtp` | ✅ Supported | [RFC 8866 §6.6](https://tools.ietf.org/html/rfc8866#section-6.6) |
| `ptime` | ✅ Supported | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `maxptime` | ✅ Supported | [RFC 8866 §6.4](https://tools.ietf.org/html/rfc8866#section-6.4) |
| `recvonly`, `sendrecv`, `sendonly`, `inactive` | ✅ Supported | [RFC 8866 §6.7](https://tools.ietf.org/html/rfc8866#section-6.7) |

### WebRTC Extensions

| Feature | Status | RFC Reference |
|---------|--------|---------------|
| ICE Attributes | ✅ Supported | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| DTLS-SRTP | ✅ Supported | [RFC 8842](https://tools.ietf.org/html/rfc8842) |
| Media Stream Identification | ✅ Supported | [RFC 8830](https://tools.ietf.org/html/rfc8830) |
| BUNDLE Grouping | ✅ Supported | [RFC 8843](https://tools.ietf.org/html/rfc8843) |
| RTP Header Extensions | ✅ Supported | [RFC 8285](https://tools.ietf.org/html/rfc8285) |

### WebRTC Attributes

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `candidate` | ✅ Supported | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-ufrag` | ✅ Supported | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-pwd` | ✅ Supported | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `ice-options` | ✅ Supported | [RFC 8839](https://tools.ietf.org/html/rfc8839) |
| `fingerprint` | ✅ Supported | [RFC 8122](https://tools.ietf.org/html/rfc8122) |
| `setup` | ✅ Supported | [RFC 4145](https://tools.ietf.org/html/rfc4145) |
| `mid` | ✅ Supported | [RFC 8843](https://tools.ietf.org/html/rfc8843) |
| `group` | ✅ Supported | [RFC 5888](https://tools.ietf.org/html/rfc5888) |
| `msid` | ✅ Supported | [RFC 8830](https://tools.ietf.org/html/rfc8830) |
| `rtcp-fb` | ✅ Supported | [RFC 4585](https://tools.ietf.org/html/rfc4585) |
| `rtcp-mux` | ✅ Supported | [RFC 5761](https://tools.ietf.org/html/rfc5761) |
| `extmap` | ✅ Supported | [RFC 8285](https://tools.ietf.org/html/rfc8285) |
| `rid` | ✅ Supported | [RFC 8851](https://tools.ietf.org/html/rfc8851) |
| `simulcast` | ✅ Supported | [RFC 8853](https://tools.ietf.org/html/rfc8853) |
| `ssrc` | ✅ Supported | [RFC 5576](https://tools.ietf.org/html/rfc5576) |
| `end-of-candidates` | ✅ Supported | [RFC 8840](https://tools.ietf.org/html/rfc8840) |

### Data Channel Support (WebRTC)

| Attribute | Status | RFC Reference |
|-----------|--------|---------------|
| `sctp-port` | ✅ Supported | [RFC 8841](https://tools.ietf.org/html/rfc8841) |
| `max-message-size` | ✅ Supported | [RFC 8841](https://tools.ietf.org/html/rfc8841) |
| `sctpmap` | ✅ Supported | [RFC 4960](https://tools.ietf.org/html/rfc4960) |

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
    if let Some(rtpmap) = media.get_rtpmap(0) {
        println!("Codec: {}/{}", rtpmap.encoding_name, rtpmap.clock_rate);
    }
}
```

### Creating and Modifying SDP

```rust
use rvoip_sip_core::types::sdp::{SdpSession, Origin, MediaDescription, ConnectionData};

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

// Add a media description
let audio_media = MediaDescription::new(
    "audio", 
    49170, 
    "RTP/AVP", 
    vec!["0".to_string(), "8".to_string()]
);
session.add_media(audio_media);
```

## Known Limitations

1. **Partial Support for SIP-specific Extensions**: While the core SDP features required for SIP signaling are supported, some SIP-specific extensions may not be fully implemented.

2. **No Direct SRTP Parameters Handling**: The library parses `crypto` attributes but doesn't provide specialized methods for SRTP parameter manipulation.

3. **Limited SDP Munging Helpers**: Advanced WebRTC operations that require SDP manipulation (munging) don't have dedicated helper methods, though all the necessary parsing and structure is in place.

4. **Minimal SCTP Data Channel Helpers**: While data channel attributes are parsed, there are limited helper methods for data channel manipulation.

## Extending the Library

The library follows an extensible pattern that makes it straightforward to add support for new attributes:

1. Define any necessary structured types in `types/sdp.rs`
2. Add parser function in `attributes.rs`
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