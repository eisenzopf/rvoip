# rvoip-sip-core

Core SIP protocol implementation for the rvoip VoIP stack.

## Features

- Complete implementation of SIP message parsing and serialization
- Support for all standard SIP methods and headers
- RFC-compliant URI parsing
- Robust error handling
- Multipart MIME body support
- IPv6 support

## Usage

```rust
use rvoip_sip_core::{
    parse_message, Message, Request, Response, 
    Method, Uri, StatusCode, Header, HeaderName, HeaderValue
};
use bytes::Bytes;

// Parse a SIP message
let data = Bytes::from("INVITE sip:bob@example.com SIP/2.0\r\n...");
let message = parse_message(&data).unwrap();

// Create a SIP request
let request = Request::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    .with_header(Header::text(HeaderName::From, "<sip:alice@example.com>"))
    .with_header(Header::text(HeaderName::To, "<sip:bob@example.com>"));

// Create a SIP response
let response = Response::new(StatusCode::Ok)
    .with_header(Header::text(HeaderName::From, "<sip:alice@example.com>"))
    .with_header(Header::text(HeaderName::To, "<sip:bob@example.com>"));
```

## Testing

This crate includes a comprehensive torture test suite based on:

- RFC 4475 - SIP Torture Test Messages
- RFC 5118 - SIP IPv6 Torture Tests
- Custom torture cases for edge conditions

See the [test suite documentation](tests/README.md) for more details.

## License

MIT OR Apache-2.0