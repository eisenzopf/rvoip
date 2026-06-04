# SDP Parser and Generator

This module provides practical parsing, manipulation, and generation support for Session Description Protocol (SDP) messages according to [RFC 8866](https://tools.ietf.org/html/rfc8866) (which obsoletes [RFC 4566](https://tools.ietf.org/html/rfc4566)) and common SIP/WebRTC extensions. The parser/model accepts and round-trips the core RFC 8866 line set and the WebRTC-facing attributes used by SIP today and `rvoip-webrtc` going forward; use the checklist below to track remaining conformance, validation, and fixture gaps.

## Overview

The SDP module consists of:

- **Parser**: A robust parser that converts SDP text to structured data
- **Types**: Data structures representing SDP sessions, media descriptions, and attributes
- **Attributes**: Parsers for various standard and extended SDP attributes
- **Builder**: Fluent builder API for creating SDP sessions programmatically
- **Macros**: Declarative macro-based syntax for creating SDP sessions
- **Generator**: Methods to convert SDP structures back to standard format

## Parser and Builder Coverage Checklist

Last audited: 2026-06-03.

Status legend:

- `Typed`: parsed into a dedicated field or `ParsedAttribute` variant and displayed in SDP syntax.
- `Generic`: accepted and preserved as a generic `a=` attribute, but no typed validation/accessor exists.
- `Partial`: accepted for common cases, but known RFC grammar, semantic, builder-helper, or validation gaps remain.
- `Missing`: not accepted, dropped, or serialized incorrectly.

### Core SDP Lines (RFC 8866)

| RFC field | Parser status | Builder/display status | Known gaps / next checks |
|-----------|---------------|------------------------|--------------------------|
| `v=` protocol version | Typed | Typed | Only version `0` accepted, as required. |
| `o=` origin | Typed | Typed | Add more RFC 8866 ABNF edge-case fixtures. |
| `s=` session name | Partial | Typed | Parser requires UTF-8 input and does not implement `a=charset` decoding for non-UTF-8 text fields. |
| `i=` information | Typed | Typed | Session-level and media-level `i=` are preserved; charset decoding remains future work. |
| `u=` URI | Typed | Typed | Stored as raw text; no URI syntax validation. |
| `e=` email | Typed | Typed | Repeatable `e=` lines are preserved; legacy `email` mirrors the first entry. |
| `p=` phone | Typed | Typed | Repeatable `p=` lines are preserved; legacy `phone` mirrors the first entry. |
| `c=` connection | Typed | Typed | Session-level `c=` and repeatable media-level `c=` are preserved; deeper address grammar validation remains limited. |
| `b=` bandwidth | Typed | Typed | Session/media multiple `b=` lines are accepted as `ParsedAttribute::Bandwidth`. |
| `t=` active time | Typed | Typed | Multiple `t=` lines supported. Add ABNF edge-case fixtures. |
| `r=` repeat time | Typed | Typed | Attached to the previous `t=` line. Add RFC duration-unit fixtures. |
| `z=` time-zone adjustment | Typed | Typed | Raw `z=` syntax is preserved; structured adjustment-pair validation is still minimal. |
| `k=` encryption key | Typed | Typed | Raw deprecated `k=` syntax is preserved; no key-type-specific validation is applied. |
| `a=` attributes | Partial | Partial | Unknown attributes are accepted generically in lenient mode. Strict mode rejects malformed known attributes. |
| `m=` media description | Typed | Typed | Media, port, optional port count, protocol, and formats are preserved. |
| Field ordering | Typed | N/A | `parse_sdp_strict` enforces RFC 8866 ordering; default lenient parsing keeps SIP/WebRTC interoperability behavior. |

### RFC 8866 Standard Attributes

| Attribute | Parser status | Builder/display status | Known gaps / next checks |
|-----------|---------------|------------------------|--------------------------|
| `cat` | Typed | Typed | Stored as a non-empty string; add full ABNF fixtures. |
| `keywds` | Typed | Typed | Stored as a non-empty string; add full ABNF fixtures. |
| `tool` | Typed | Typed | Stored as a non-empty string; add full ABNF fixtures. |
| `ptime` | Typed | Typed | Stored in a dedicated media field when media-level. |
| `maxptime` | Typed | Typed | No dedicated accessor; stored as a generic typed attribute. |
| `rtpmap` | Typed | Typed | Add ABNF fixtures for static/dynamic payload type edge cases and trailing data rejection. |
| `sendrecv`, `sendonly`, `recvonly`, `inactive` | Typed | Typed | Session/media level supported. |
| `orient` | Typed | Typed | Stored as text; no enum validation for allowed orientation values yet. |
| `type` | Typed | Typed | Stored as text; no enum validation for RFC conference-type values yet. |
| `charset` | Typed | Typed | Attribute is preserved; parser does not decode non-UTF-8 SDP bodies. |
| `sdplang` | Typed | Typed | Preserved; no RFC 5646 language-tag validation yet. |
| `lang` | Typed | Typed | Preserved; no RFC 5646 language-tag validation yet. |
| `framerate` | Typed | Typed | Numeric syntax is checked and original text is preserved. |
| `quality` | Typed | Typed | Parsed as `0..=10`. |
| `fmtp` | Typed | Typed | Parameters are preserved as raw text. |

### WebRTC-Relevant Extensions

| Area / attribute | Parser status | Builder/display status | Known gaps / next checks |
|------------------|---------------|------------------------|--------------------------|
| ICE: `candidate` | Typed | Typed | UDP/TCP candidates and extension parameters are preserved. Add broader RFC 8839 and Trickle ICE corpus coverage. |
| ICE: `ice-ufrag`, `ice-pwd`, `ice-options`, `ice-lite`, `remote-candidates`, `end-of-candidates` | Typed | Typed | Syntax accepted; add more RFC length/token fixtures and level/semantic checks where required. |
| DTLS: `fingerprint`, `setup`, `tls-id` | Typed | Typed | Syntax parsed; semantic validator checks DTLS media fingerprint presence, but full hash/role policy remains caller-defined. |
| BUNDLE: `group`, `mid`, `bundle-only` | Typed | Typed | `validate_sdp_semantics` checks unique mids and BUNDLE mids; add more browser and RFC example fixtures. |
| MSID: `msid`, `msid-semantic` | Typed | Typed | Basic stream/track/semantic parse. Add RFC 8830 edge-case fixtures. |
| RTP feedback: `rtcp`, `rtcp-fb`, `rtcp-mux`, `rtcp-rsize` | Typed | Typed | Common forms parse; detailed feedback parameter semantics remain limited. |
| RTP header extensions: `extmap`, `extmap-allow-mixed` | Typed | Typed | `http`, `https`, and URN extension URIs parse; add more ID/direction boundary fixtures. |
| Source attributes: `ssrc`, `ssrc-group` | Typed | Typed | Basic RFC 5576 parse; source-level semantic validation remains limited. |
| RID: `rid` | Typed | Typed | RFC-style RID parses and simulcast references are checked by semantic validation. Add more parameter grammar fixtures. |
| Simulcast: `simulcast` | Typed | Typed | Structured model preserves send/recv directions, alternatives, and paused state. |
| Data channels: `sctp-port`, `max-message-size`, `sctpmap`, `dcmap`, `dcsa` | Typed | Typed | Modern, legacy, and RFC 8864 forms parse; add broader browser/data-channel fixture coverage. |
| SDES-SRTP: `crypto` | Partial | Typed | Known suites parse into `Crypto`; unknown suites are preserved as generic values for compatibility. Add lifetime/MKI round-trip fixtures. |
| Semantic checks | Partial | N/A | `validate_sdp_semantics` covers unique mids, BUNDLE mids, ICE credential pairing, simulcast RID references, DTLS fingerprint presence, and SCTP requirements. |

### Gap-Filling Checklist

| Priority | Work item | Status | Why it matters / next check |
|----------|-----------|--------|-----------------------------|
| P0 | Accept and preserve RFC 8866 core line types, including `b=`, `z=`, `k=`, media `i=`, repeatable `e=`/`p=`, media-level `c=`, and `m=` port counts. | Done | Prevents repeats of the `b=` dispatcher miss and avoids lossy parse/display cycles. |
| P0 | Split strict RFC ordering from lenient SIP/WebRTC interoperability parsing. | Done | `parse_sdp_strict` enforces fixed RFC line order; `parse_sdp` remains lenient. |
| P0 | Add typed RFC 8866 standard attributes. | Done | `cat`, `keywds`, `tool`, `orient`, `type`, `charset`, `sdplang`, `lang`, `framerate`, and `quality` now have typed variants. |
| P0 | Add typed WebRTC-relevant attributes. | Done | ICE, Trickle ICE, DTLS, BUNDLE, MSID, RTCP, extmap, RID, simulcast, SCTP, RFC 8864 data-channel, and SDES crypto forms are parser-visible. |
| P1 | Add semantic validation helpers separately from syntax parsing. | Done | `validate_sdp_semantics` keeps policy checks opt-in and parse preservation lenient. |
| P1 | Expand RFC 8866 ABNF fixtures for every core line and strict malformed permutation. | Open | Validates boundaries beyond the initial implementation tests. |
| P1 | Expand WebRTC fixture corpus from browser offers/answers and RFC examples. | Open | WebRTC integration depends on combinations of ICE, DTLS, BUNDLE, RTP, RID, simulcast, and data-channel attributes. |
| P1 | Add fuzz coverage for valid/invalid SDP line permutations and WebRTC attribute corpora. | Done | Existing SDP fuzz target now exercises lenient/strict modes, RFC-shaped core SDP, mixed line permutations, and a WebRTC corpus. Keep growing the seed corpus as new browser fixtures land. |
| P2 | Add higher-level builder/accessor ergonomics for newer typed fields. | Open | Parser/model support exists; fluent builder helpers are still uneven across newer attributes. |
| P2 | Decide charset decoding policy for non-UTF-8 SDP text fields. | Open | `a=charset` is preserved, but parser input is still UTF-8. |

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

## Coverage Testing

The implementation has broad test coverage for currently supported SDP behavior, including:

- Common session-level and media-level line parsing
- Validation of required fields and selected ordering constraints
- Support for IPv4, IPv6, and multicast connection addresses
- Common SIP/WebRTC attributes and extensions
- Handling of malformed SDP messages with appropriate errors
- Round-trip tests for supported fields

The checklist above is the source of truth for remaining RFC grammar, semantic, data model, and round-trip gaps.

## Known Limitations

1. **Strict Validation in Some Areas**: The parser implements strict validation for certain fields like version number, which must be "0" according to RFC 8866.

2. **IP Address Validation**: While the parser handles IPv4, IPv6, and multicast addresses, its validation is more lenient than strict RFC requirements in some cases, accepting addresses that might not be technically valid.

3. **Limited SDP Munging Helpers**: Advanced WebRTC operations that require SDP manipulation (munging) don't have dedicated helper methods. Some WebRTC-related SDP structures are still represented generically or partially.

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
