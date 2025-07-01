# rvoip-sip-core

Core SIP protocol implementation for the rvoip VoIP stack.

## Overview

`rvoip-sip-core` provides a robust, RFC-compliant implementation of the Session Initiation Protocol (SIP) in Rust. This library offers a complete toolkit for building SIP-based applications such as VoIP clients, SIP proxies, SIP servers, and other SIP-enabled communications systems.

## Features

- Complete RFC-compliant SIP protocol implementation
- Efficient and robust message parsing and serialization
- Strongly-typed headers with validation
- Flexible URI handling with comprehensive parameter support
- SDP (Session Description Protocol) integration
- Multipart MIME body handling
- IPv6 support
- Strict and lenient parsing modes to handle both compliant and real-world SIP messages
- Fluent builder patterns for creating SIP messages
- Declarative macros for concise SIP/SDP creation
- Session validation with RFC-compliant IP address checking
- Extensive test suite with RFC torture tests

## Installation

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
rvoip-sip-core = "0.1.0"
```

## Quick Start

### Parsing SIP Messages

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;

// Parse a SIP message
let data = Bytes::from(
    "INVITE sip:bob@example.com SIP/2.0\r\n\
     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
     Max-Forwards: 70\r\n\
     To: Bob <sip:bob@example.com>\r\n\
     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
     CSeq: 314159 INVITE\r\n\
     Contact: <sip:alice@pc33.atlanta.com>\r\n\
     Content-Type: application/sdp\r\n\
     Content-Length: 0\r\n\r\n"
);

let message = parse_message(&data).expect("Valid SIP message");

// Access message components
if let Message::Request(request) = message {
    println!("Method: {}", request.method());
    println!("URI: {}", request.uri());
    
    let from = request.typed_header::<From>().expect("From header");
    println!("From: {}", from.address());
}
```

### Creating SIP Messages

#### Using the Builder Pattern (recommended for complex messages)

```rust
use rvoip_sip_core::prelude::*;

// Create a SIP request with the RequestBuilder
let bob_uri = "sip:bob@example.com".parse::<Uri>().unwrap();
let alice_uri = "sip:alice@atlanta.com".parse::<Uri>().unwrap();
let contact_uri = "sip:alice@pc33.atlanta.com".parse::<Uri>().unwrap();

let request = RequestBuilder::new(Method::Invite, &bob_uri.to_string())
    .unwrap()
    .header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", alice_uri.clone()))))
    .header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", bob_uri.clone()))))
    .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
    .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
    .header(TypedHeader::Via(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()))
    .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
    .header(TypedHeader::Contact(Contact::new(Address::new(contact_uri))))
    .header(TypedHeader::ContentLength(ContentLength::new(0)))
    .build();
```

#### Using the SIP Macros (recommended for simple messages)

```rust
use rvoip_sip_core::prelude::*;

// Create a SIP request with the sip! macro
let request = sip! {
    method: Method::Invite,
    uri: "sip:bob@example.com",
    headers: {
        Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
        MaxForwards: 70,
        To: "Bob <sip:bob@example.com>",
        From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
        CallId: "a84b4c76e66710@pc33.atlanta.com",
        CSeq: "314159 INVITE",
        Contact: "<sip:alice@pc33.atlanta.com>",
        ContentLength: 0
    }
};
```

### Creating SDP Messages

#### Using the SdpBuilder Pattern

```rust
use rvoip_sip_core::sdp_prelude::*;

// Create an SDP session with the SdpBuilder
let sdp = SdpBuilder::new("My Session")
    .origin("-", "1234567890", "2", "IN", "IP4", "127.0.0.1")
    .time("0", "0")  // Time 0-0 means permanent session
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .direction(MediaDirection::SendRecv)
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .done()
    .build();
```

#### Using the sdp! Macro (recommended for simple messages)

```rust
use rvoip_sip_core::sdp_prelude::*;

// Create an SDP session with the sdp! macro
let sdp = sdp! {
    origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
    session_name: "Audio Call",
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
```

## Core Components

### Message Types

The library provides three main message types:

- `Message`: An enum representing either a SIP request or response
- `Request`: Represents a SIP request (INVITE, BYE, etc.)
- `Response`: Represents a SIP response (200 OK, 404 Not Found, etc.)

### Headers

All standard SIP headers are implemented with strong typing:

- `From`, `To`: Address headers with tag parameters
- `Via`: Network routing information
- `CSeq`: Command sequence number
- `Call-ID`: Unique identifier for a dialog
- And many more...

### URI Handling

The library includes a comprehensive URI implementation:

- `Uri`: Main URI type with full parameter support
- `Scheme`: SIP URI schemes (sip, sips, tel, etc.)
- `Host`: Host representation (domain name or IP address)

### SDP Support

For handling multimedia session information:

- `SdpSession`: Full SDP session representation
- `MediaDescription`: Media type, port, and attributes
- `Connection`: Network connection information
- Complete support for WebRTC attributes and data channels

## Advanced Usage

### Parsing with Different Modes

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::parser::message::ParseMode;
use bytes::Bytes;

let data = Bytes::from("SIP message data...");

// Default parsing is lenient for robustness
let message = parse_message(&data).expect("Valid SIP message");

// Use strict mode for RFC compliance validation
let strict_message = parse_message_with_mode(&data, ParseMode::Strict);

// Use lenient mode explicitly to handle non-compliant messages
let lenient_message = parse_message_with_mode(&data, ParseMode::Lenient);
```

### Working with Multipart Bodies

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;

// Create a multipart body
let mut multipart = MultipartBody::new("mixed");
multipart.add_part(MimePart::new(
    "application/sdp",
    Bytes::from("v=0\r\no=- 123456 789012 IN IP4 192.168.1.1\r\ns=Call\r\nc=IN IP4 192.168.1.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\n")
));
multipart.add_part(MimePart::new(
    "application/xml",
    Bytes::from("<xml>Some XML content</xml>")
));

// Add to a request using the builder
let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com")
    .unwrap()
    // Add headers...
    .body(multipart.to_bytes())
    .build();
```

### Handling Authentication

```rust
use rvoip_sip_core::prelude::*;

// Create an Authorization header
let auth = Authorization::new_digest(
    "example.com",
    "alice",
    "password",
    "INVITE",
    "sip:bob@example.com",
    "nonce-value",
    "cnonce-value"
);

// Add to a request using the builder
let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com")
    .unwrap()
    .header(TypedHeader::Authorization(auth))
    // Add other headers...
    .build();
```

## Validation

The library includes comprehensive validation for SIP messages and SDP content:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::sdp_prelude::*;

// Validate an IP address
let is_valid = is_valid_ipv4("192.168.1.1"); // true
let is_valid = is_valid_ipv4("256.0.0.1");   // false (invalid IPv4)

// Validate an SDP session
let validation_result = sdp_session.validate();
if let Err(errors) = validation_result {
    for error in errors {
        println!("Validation error: {}", error);
    }
}
```

## Prelude Modules

The library provides convenient prelude modules to import common types:

```rust
// For SIP functionality
use rvoip_sip_core::prelude::*;

// For SDP functionality
use rvoip_sip_core::sdp_prelude::*;
```

## Feature Flags

- `lenient_parsing`: Enables more lenient parsing mode for torture tests and handling of non-compliant messages

## Testing

This crate includes a comprehensive test suite based on:

- RFC 4475 - SIP Torture Test Messages
- RFC 5118 - SIP IPv6 Torture Tests
- Custom torture cases for edge conditions

See the [test suite documentation](tests/README.md) for more details.

## License

MIT OR Apache-2.0