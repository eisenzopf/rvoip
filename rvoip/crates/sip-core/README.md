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
- Extensive test suite with RFC torture tests

## Installation

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
rvoip-sip-core = "0.1.0"
```

## Quick Start

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;
use std::str::FromStr;

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

// Create a SIP request
let request = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap())))
    .with_header(To::new(Address::new_with_display_name("Bob", Uri::from_str("sip:bob@example.com").unwrap())))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
    .with_header(MaxForwards::new(70))
    .with_header(Contact::new(Address::new(Uri::from_str("sip:alice@pc33.atlanta.com").unwrap())))
    .with_header(ContentLength::new(0))
    .build();

// Create a SIP response
let response = ResponseBuilder::new(StatusCode::Ok)
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap())))
    .with_header(To::new(Address::new_with_display_name("Bob", Uri::from_str("sip:bob@example.com").unwrap())))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
    .with_header(ContentLength::new(0))
    .build();
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

// Add to a request
let request = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    // Add headers...
    .with_body(multipart.to_bytes())
    .build();
```

### Handling Authentication

```rust
use rvoip_sip_core::prelude::*;
use std::str::FromStr;

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

let request = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    .with_header(auth)
    // Add other headers...
    .build();
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