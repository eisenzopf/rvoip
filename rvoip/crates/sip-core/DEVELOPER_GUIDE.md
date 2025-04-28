# rvoip-sip-core Developer Guide

This guide provides detailed examples and best practices for working with the `rvoip-sip-core` library.

## Table of Contents

- [Getting Started](#getting-started)
- [Parsing SIP Messages](#parsing-sip-messages)
- [Creating SIP Messages](#creating-sip-messages)
- [Working with Headers](#working-with-headers)
- [URI Manipulation](#uri-manipulation)
- [SDP Integration](#sdp-integration)
- [Authentication](#authentication)
- [Common Patterns](#common-patterns)
- [Error Handling](#error-handling)
- [Testing](#testing)

## Getting Started

To use `rvoip-sip-core`, add it to your `Cargo.toml`:

```toml
[dependencies]
rvoip-sip-core = "0.1.0"
bytes = "1.4"  # Needed for handling raw message data
```

Import the prelude to get access to all common types:

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;
use std::str::FromStr;
```

## Parsing SIP Messages

### Basic Parsing

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;

// Raw SIP message
let data = Bytes::from(
    "INVITE sip:bob@example.com SIP/2.0\r\n\
     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
     Max-Forwards: 70\r\n\
     To: Bob <sip:bob@example.com>\r\n\
     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
     CSeq: 314159 INVITE\r\n\
     Contact: <sip:alice@pc33.atlanta.com>\r\n\
     Content-Length: 0\r\n\r\n"
);

// Parse the message
match parse_message(&data) {
    Ok(message) => {
        // Process the parsed message
        match message {
            Message::Request(request) => {
                println!("Received request: {} {}", request.method(), request.uri());
            }
            Message::Response(response) => {
                println!("Received response: {}", response.status);
            }
        }
    }
    Err(err) => {
        eprintln!("Failed to parse SIP message: {}", err);
    }
}
```

### Advanced Parsing Options

For specialized parsing needs, you can use the `parse_message_with_mode` function with different parsing modes:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::parser::message::ParseMode;
use bytes::Bytes;

let data = Bytes::from("SIP message data...");

// Use strict mode for RFC compliance validation
let strict_result = parse_message_with_mode(&data, ParseMode::Strict);

// Use lenient mode for handling real-world SIP messages with minor issues
let lenient_result = parse_message_with_mode(&data, ParseMode::Lenient);
```

#### Lenient vs. Strict Parsing

- **Lenient Mode (Default):**
  - Handles Content-Length mismatches gracefully
  - Accepts messages with missing or extra body data
  - Processes malformed headers as raw headers instead of failing
  - Suitable for real-world SIP traffic

- **Strict Mode:**
  - Enforces RFC 3261 compliance
  - Rejects messages with Content-Length mismatches
  - Validates all required headers
  - Useful for testing and validation

## Creating SIP Messages

### Creating SIP Requests

```rust
use rvoip_sip_core::prelude::*;
use std::str::FromStr;

// Create a basic INVITE request
let invite = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap())))
    .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap())))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(MaxForwards::new(70))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
    .with_header(Contact::new(Address::new(Uri::from_str("sip:alice@pc33.atlanta.com").unwrap())))
    .with_header(ContentLength::new(0))
    .build();

// Convert to bytes for transmission
let bytes = invite.to_bytes();
```

### Creating SIP Responses

```rust
use rvoip_sip_core::prelude::*;
use std::str::FromStr;

// Create a 200 OK response
let ok_response = ResponseBuilder::new(StatusCode::Ok)
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap()).with_tag("1928301774")))
    .with_header(To::new(Address::new_with_display_name("Bob", Uri::from_str("sip:bob@example.com").unwrap()).with_tag("a6c85cf")))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
    .with_header(Contact::new(Address::new(Uri::from_str("sip:bob@192.168.1.2").unwrap())))
    .with_header(ContentLength::new(0))
    .build();

// Create a 404 Not Found response
let not_found = ResponseBuilder::new(StatusCode::NotFound)
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap()).with_tag("1928301774")))
    .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap())))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
    .with_reason("User Not Available")
    .build();
```

### Common Response Shortcuts

```rust
use rvoip_sip_core::prelude::*;
use std::str::FromStr;

// Use convenience methods for common responses
let trying = Response::trying()
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap()).with_tag("1928301774")))
    .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap())))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap());

let ringing = Response::ringing()
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap()).with_tag("1928301774")))
    .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap()).with_tag("a6c85cf")))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap());

let ok = Response::ok()
    .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap()).with_tag("1928301774")))
    .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap()).with_tag("a6c85cf")))
    .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
    .with_header(CSeq::new(314159, Method::Invite))
    .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap());
```

## Working with Headers

### Accessing Headers

```rust
// Get a specific header by its type
if let Some(from) = request.typed_header::<From>() {
    println!("From: {}", from.address());
    
    // Check for a tag parameter
    if let Some(tag) = from.address().tag() {
        println!("Tag: {}", tag);
    }
}

// Get a header by name
if let Some(header) = request.header(&HeaderName::CallId) {
    println!("Call-ID: {}", header);
}

// Access common headers through helper methods
if let Some(call_id) = request.call_id() {
    println!("Call-ID: {}", call_id.value());
}

// Get multiple instances of the same header type
let via_headers = request.via_headers();
println!("Via headers: {}", via_headers.len());
```

### Creating Headers

```rust
use rvoip_sip_core::prelude::*;
use std::str::FromStr;

// Create a From header
let from = From::new(Address::new(Uri::from_str("sip:alice@atlanta.com").unwrap()));

// Create a From header with display name
let from_with_name = From::new(
    Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap())
);

// Add a tag parameter
let from_with_tag = From::new(
    Address::new(Uri::from_str("sip:alice@atlanta.com").unwrap())
        .with_tag("1928301774")
);

// Create a Call-ID header
let call_id = CallId::new("a84b4c76e66710@pc33.atlanta.com");

// Create a CSeq header
let cseq = CSeq::new(314159, Method::Invite);

// Create a Via header with parameters
let via = Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap();
```

## URI Manipulation

### Creating URIs

```rust
use std::str::FromStr;
use rvoip_sip_core::prelude::*;

// Parse a URI from a string
let uri = Uri::from_str("sip:alice@atlanta.com:5060;transport=tcp").unwrap();

// Create a URI with a domain host
let uri = Uri::sip("example.com")
    .with_user("bob")
    .with_port(5060);

// Create a URI with an IPv4 address
let uri = Uri::sip_ipv4("192.168.1.1")
    .with_parameter(Param::transport("udp"));

// Create a URI with an IPv6 address
let uri = Uri::sip_ipv6("2001:db8::1")
    .with_port(5060);

// Create a secure SIP URI
let uri = Uri::sips("example.com")
    .with_user("alice");

// Create a TEL URI
let uri = Uri::tel("+12125551212");
```

### Accessing URI Components

```rust
// Get the scheme
println!("Scheme: {}", uri.scheme);

// Get the username, if present
if let Some(username) = uri.username() {
    println!("Username: {}", username);
}

// Get the host
match &uri.host {
    Host::Domain(domain) => println!("Domain: {}", domain),
    Host::Address(addr) => println!("IP address: {}", addr),
}

// Get the port, if present
if let Some(port) = uri.port {
    println!("Port: {}", port);
}

// Check for specific parameters
if let Some(transport) = uri.transport() {
    println!("Transport: {}", transport);
}

if uri.is_phone_number() {
    println!("This is a phone number URI");
}
```

## SDP Integration

### Creating an SDP Session

```rust
// Create an SDP session
let sdp = SdpSession::new(
    Origin::new("alice", 123456, 789012, "IN", "IP4", "192.168.1.1"),
    "Call with Bob"
)
.with_connection(ConnectionData::new("IN", "IP4", "192.168.1.1"))
.with_time(TimeDescription::new(0, 0))
.with_media(
    MediaDescription::new("audio", 49170, "RTP/AVP", &[0, 8])
        .with_attribute("rtpmap", "0 PCMU/8000")
        .with_attribute("rtpmap", "8 PCMA/8000")
        .with_attribute("ptime", "20")
        .with_direction(MediaDirection::SendRecv)
);

// Convert to string for inclusion in a SIP message
let sdp_string = sdp.to_string();

// Add as a body to a SIP request
let invite = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    // Add headers...
    .with_header(ContentType::new(MediaType::parse("application/sdp").unwrap()))
    .with_header(ContentLength::new(sdp_string.len() as u32))
    .with_body(Bytes::from(sdp_string))
    .build();
```

### Parsing an SDP Body

```rust
// Parse an SDP body from a SIP message
if message.is_request() && 
   message.method() == Some(Method::Invite) {
    // Check for SDP content type
    if let Some(content_type) = message.typed_header::<ContentType>() {
        if content_type.media_type().to_string() == "application/sdp" {
            // Parse the SDP body
            let body_str = std::str::from_utf8(message.body()).unwrap();
            match SdpSession::parse(body_str) {
                Ok(sdp) => {
                    println!("Session: {}", sdp.session_name());
                    
                    // Process media descriptions
                    for media in sdp.media() {
                        println!("Media: {} {}", media.media_type(), media.port());
                        
                        // Get codec information
                        for format in media.formats() {
                            println!("Format: {}", format);
                        }
                        
                        // Check media direction
                        match media.direction() {
                            Some(MediaDirection::SendRecv) => println!("Direction: sendrecv"),
                            Some(MediaDirection::SendOnly) => println!("Direction: sendonly"),
                            Some(MediaDirection::RecvOnly) => println!("Direction: recvonly"),
                            Some(MediaDirection::Inactive) => println!("Direction: inactive"),
                            None => println!("Direction: not specified"),
                        }
                    }
                },
                Err(err) => {
                    eprintln!("Failed to parse SDP: {}", err);
                }
            }
        }
    }
}
```

## Authentication

### Creating Authentication Headers

```rust
// WWW-Authenticate challenge from server
let www_auth = WwwAuthenticate::new_digest("example.com")
    .with_nonce("raNdOm-NoNcE-vAlUe")
    .with_algorithm("MD5")
    .with_qop("auth");

// Authorization header for client response
let auth = Authorization::new_digest(
    "example.com",    // realm
    "alice",          // username
    "secret-password", // password
    "INVITE",         // method
    "sip:bob@example.com", // URI
    "raNdOm-NoNcE-vAlUe", // nonce (from WWW-Authenticate)
    "cNoNcE-vAlUe"    // cnonce (client-generated)
)
.with_algorithm("MD5")
.with_qop("auth")
.with_nc(1);

// Add to a request
let request = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
    .with_header(auth)
    // Add other headers...
    .build();
```

## Common Patterns

### Creating a Transaction ID

```rust
// Generate a branch parameter for the Via header (RFC 3261 magic cookie + random string)
use uuid::Uuid;
let branch = format!("z9hG4bK-{}", Uuid::new_v4().simple());

// Create a Via header with this branch
let via = Via::parse(&format!("SIP/2.0/UDP 192.168.1.1:5060;branch={}", branch)).unwrap();
```

### Handling a Dialog

```rust
// Store dialog information
struct Dialog {
    call_id: String,
    local_tag: String,
    remote_tag: Option<String>,
    local_seq: u32,
    remote_seq: u32,
    local_uri: Uri,
    remote_uri: Uri,
    route_set: Vec<String>,
}

// Create a dialog from an INVITE and its 200 OK response
fn create_dialog(invite: &Request, ok: &Response) -> Dialog {
    let call_id = ok.typed_header::<CallId>().unwrap().value().to_string();
    
    let from = ok.typed_header::<From>().unwrap();
    let to = ok.typed_header::<To>().unwrap();
    
    let local_tag = from.address().tag().unwrap().to_string();
    let remote_tag = to.address().tag().map(|s| s.to_string());
    
    let cseq = ok.typed_header::<CSeq>().unwrap();
    
    Dialog {
        call_id,
        local_tag,
        remote_tag,
        local_seq: cseq.sequence(),
        remote_seq: 0, // Will be set when we receive a request
        local_uri: from.address().uri().clone(),
        remote_uri: to.address().uri().clone(),
        route_set: Vec::new(), // Populate from Record-Route headers
    }
}
```

### Creating a BYE Request in a Dialog

```rust
// Create a BYE request within an existing dialog
fn create_bye(dialog: &Dialog, local_contact: &Contact) -> Request {
    RequestBuilder::new(Method::Bye, dialog.remote_uri.clone())
        .with_header(From::new(
            Address::new(dialog.local_uri.clone()).with_tag(&dialog.local_tag)
        ))
        .with_header(To::new(
            match &dialog.remote_tag {
                Some(tag) => Address::new(dialog.remote_uri.clone()).with_tag(tag),
                None => Address::new(dialog.remote_uri.clone()),
            }
        ))
        .with_header(CallId::new(&dialog.call_id))
        .with_header(CSeq::new(dialog.local_seq + 1, Method::Bye))
        .with_header(MaxForwards::new(70))
        .with_header(local_contact.clone())
        .with_header(ContentLength::new(0))
        .build()
}
```

## Error Handling

```rust
// Handle parsing errors
match parse_message(&data) {
    Ok(message) => {
        // Process message
    },
    Err(err) => match err {
        Error::ParseError(msg) => {
            eprintln!("Parse error: {}", msg);
        },
        Error::InvalidUri(msg) => {
            eprintln!("Invalid URI: {}", msg);
        },
        Error::MalformedHeader(name, msg) => {
            eprintln!("Malformed header {}: {}", name, msg);
        },
        _ => {
            eprintln!("Other error: {}", err);
        }
    }
}
```

## Testing

### Using the Test Suite

The library comes with a comprehensive test suite that you can use to validate your SIP implementation:

```bash
# Run all tests
cargo test -p rvoip-sip-core

# Run specific test suites
cargo test -p rvoip-sip-core --test torture_tests
cargo test -p rvoip-sip-core --test parser_tests
```

The test suite includes special tests for RFC compliance:

```bash
# Run with lenient parsing for real-world compatibility
cargo test -p rvoip-sip-core --features="lenient_parsing"

# Run strict RFC compliance tests 
cargo test -p rvoip-sip-core --test torture_tests::test_malformed_messages --features="lenient_parsing"
```

### Writing Your Own Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::prelude::*;
    use bytes::Bytes;
    use std::str::FromStr;
    
    #[test]
    fn test_invite_creation() {
        let invite = RequestBuilder::new(Method::Invite, Uri::from_str("sip:bob@example.com").unwrap())
            .with_header(From::new(Address::new_with_display_name("Alice", Uri::from_str("sip:alice@atlanta.com").unwrap())))
            .with_header(To::new(Address::new(Uri::from_str("sip:bob@example.com").unwrap())))
            .with_header(CallId::new("test-call-id"))
            .with_header(CSeq::new(1, Method::Invite))
            .build();
            
        assert_eq!(invite.method(), Method::Invite);
        assert_eq!(invite.uri().to_string(), "sip:bob@example.com");
        
        let from = invite.typed_header::<From>().unwrap();
        assert_eq!(from.address().uri().to_string(), "sip:alice@atlanta.com");
        assert_eq!(from.address().display_name(), Some("Alice"));
    }
    
    #[test]
    fn test_response_parsing() {
        let data = Bytes::from(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
             To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
             From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
             Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
             CSeq: 314159 INVITE\r\n\
             Contact: <sip:bob@192.168.1.2>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        
        let message = parse_message(&data).unwrap();
        assert!(message.is_response());
        
        if let Message::Response(response) = message {
            assert_eq!(response.status, StatusCode::Ok);
            
            let to = response.typed_header::<To>().unwrap();
            assert_eq!(to.address().uri().to_string(), "sip:bob@example.com");
            assert_eq!(to.address().tag(), Some("a6c85cf"));
        } else {
            panic!("Expected Response, got Request");
        }
    }
} 