# Creating SIP Messages with the Builder Pattern

In this tutorial, we'll explore how to create SIP messages using the builder pattern provided by the `rvoip-sip-core` library. The builder pattern offers a fluent, easy-to-use interface for constructing complex SIP messages without having to worry about the underlying details of the protocol.

## Introduction to the Builder Pattern

The builder pattern is a design pattern that allows for the step-by-step construction of complex objects. It's particularly useful for creating objects with many optional parameters or when the construction process involves multiple steps.

In the context of SIP messages, the builder pattern helps us:

1. Create well-formed SIP messages with minimal code
2. Ensure all required headers are present
3. Validate the message structure as we build it
4. Provide a fluent, chainable API for better readability

## SIP Message Builders in rvoip-sip-core

The `rvoip-sip-core` library provides several builder classes for creating SIP messages:

1. **SimpleRequestBuilder**: For creating SIP request messages (INVITE, REGISTER, etc.)
2. **ResponseBuilder**: For creating SIP response messages (200 OK, 180 Ringing, etc.)
3. **SdpBuilder**: For creating SDP bodies for media negotiation

Let's explore each of these builders with practical examples.

## Creating SIP Requests

### Basic Request Structure

A SIP request typically contains:

- A request line with method, URI, and SIP version
- A set of headers (From, To, Call-ID, CSeq, Via, etc.)
- An optional message body

### Using SimpleRequestBuilder

The `SimpleRequestBuilder` provides a fluent interface for creating SIP requests:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::SimpleRequestBuilder;

// Create a basic INVITE request
let invite_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314159)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    .max_forwards(70)
    .contact("sip:alice@alice-pc.example.com", None)
    .build();

println!("{}", invite_request);
```

This produces a well-formed SIP INVITE request:

```
INVITE sip:bob@example.com SIP/2.0
From: Alice <sip:alice@example.com>;tag=1928301774
To: Bob <sip:bob@example.com>
Call-ID: a84b4c76e66710@alice-pc.example.com
CSeq: 314159 INVITE
Via: SIP/2.0/UDP alice-pc.example.com:5060;branch=z9hG4bK776asdhds
Max-Forwards: 70
Contact: <sip:alice@alice-pc.example.com>
```

### Method-Specific Builders

The `SimpleRequestBuilder` provides convenience constructors for common SIP methods:

```rust
// REGISTER request
let register_request = SimpleRequestBuilder::register("sip:registrar.example.com")?
    .from("User", "sip:user@example.com", Some("a73kszlfl"))
    .to("User", "sip:user@example.com", None)
    .call_id("1j9FpLxk3uxtm8tn@user-pc.example.com")
    .cseq(1)
    .via("user-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:user@user-pc.example.com", None)
    .expires_seconds(3600)
    .build();

// BYE request
let bye_request = SimpleRequestBuilder::bye("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", Some("8675309"))
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314160)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKasd123"))
    .max_forwards(70)
    .build();

// OPTIONS request
let options_request = SimpleRequestBuilder::options("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314161)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKasd456"))
    .max_forwards(70)
    .build();
```

Other available method-specific builders include:
- `SimpleRequestBuilder::ack()`
- `SimpleRequestBuilder::cancel()`
- `SimpleRequestBuilder::new()` (for any method)

## Creating SIP Responses

### Basic Response Structure

A SIP response typically contains:

- A status line with SIP version, status code, and reason phrase
- A set of headers (From, To, Call-ID, CSeq, Via, etc.)
- An optional message body

### Using ResponseBuilder

The `ResponseBuilder` provides a fluent interface for creating SIP responses:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::CSeqBuilderExt;

// Create a 200 OK response to an INVITE
let response = ResponseBuilder::new(StatusCode::Ok, None)
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", Some("8675309"))
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq_with_method(314159, Method::Invite)
    .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .contact("sip:bob@bob-pc.example.com", None)
    .build();

println!("{}", response);
```

This produces a well-formed SIP 200 OK response:

```
SIP/2.0 200 OK
From: Alice <sip:alice@example.com>;tag=1928301774
To: Bob <sip:bob@example.com>;tag=8675309
Call-ID: a84b4c76e66710@alice-pc.example.com
CSeq: 314159 INVITE
Via: SIP/2.0/UDP alice-pc.example.com;branch=z9hG4bK776asdhds
Contact: <sip:bob@bob-pc.example.com>
```

### Creating Different Response Types

You can create various response types by specifying different status codes:

```rust
// 100 Trying
let trying_response = ResponseBuilder::new(StatusCode::Trying, None)
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq_with_method(314159, Method::Invite)
    .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .build();

// 180 Ringing
let ringing_response = ResponseBuilder::new(StatusCode::Ringing, None)
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", Some("8675309"))
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq_with_method(314159, Method::Invite)
    .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .contact("sip:bob@bob-pc.example.com", None)
    .build();

// 404 Not Found
let not_found_response = ResponseBuilder::new(StatusCode::NotFound, None)
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", Some("8675309"))
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq_with_method(314159, Method::Invite)
    .via("alice-pc.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .build();
```

## Advanced Builder Features

### Working with Multiple Headers

Some SIP headers, like Via, can appear multiple times in a message. The builder pattern handles this elegantly:

```rust
// INVITE request with multiple Via headers
let multi_via_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314159)
    // First Via header (proxy)
    .via("proxy1.example.com:5060", "UDP", Some("z9hG4bK87asdks7"))
    // Second Via header (client)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@alice-pc.example.com", None)
    .build();
```

### Adding Custom Headers

You can add custom headers using the `header` method with `TypedHeader::Other`:

```rust
// INVITE request with custom headers
let custom_headers_request = SimpleRequestBuilder::invite("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314159)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    .max_forwards(70)
    .contact("sip:alice@alice-pc.example.com", None)
    // Add custom headers using TypedHeader::Other
    .header(TypedHeader::Other(HeaderName::Other("X-Custom-Header".to_string()), HeaderValue::text("Custom Value")))
    .header(TypedHeader::Other(HeaderName::Other("X-Priority".to_string()), HeaderValue::text("1 (Highest)")))
    .header(TypedHeader::Other(HeaderName::Other("X-Session-ID".to_string()), HeaderValue::text("abc123")))
    .build();
```

### Working with Complex URIs

The builder can handle complex SIP URIs with parameters:

```rust
// Parse a complex URI
let complex_uri_str = "sip:bob@example.com:5060;transport=tcp;lr";

// Create a request with this URI
let complex_uri_request = SimpleRequestBuilder::new(Method::Message, complex_uri_str)?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314159)
    .via("alice-pc.example.com:5060", "TCP", Some("z9hG4bK776asdhds"))
    .max_forwards(70)
    .content_type("text/plain")
    .body("Hello, Bob! This is a SIP MESSAGE.")
    .build();
```

## Building SDP Messages

SDP (Session Description Protocol) is commonly used with SIP for media negotiation. The `SdpBuilder` provides a fluent interface for creating SDP messages:

### Basic Audio SDP

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;

// Create a basic audio-only SDP
let basic_sdp = SdpBuilder::new("Audio Call")
    .origin("-", "1234567890", "1", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;

println!("Basic Audio SDP:\n{}", basic_sdp);
```

### Audio/Video SDP

```rust
// Create a more complex SDP with audio and video
let complex_sdp = SdpBuilder::new("Audio/Video Call")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .media_video(51372, "RTP/AVP")
        .formats(&["96", "97"])
        .rtpmap("96", "VP8/90000")
        .rtpmap("97", "H264/90000")
        .fmtp("97", "profile-level-id=42e01f;level-asymmetry-allowed=1")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;
```

### WebRTC SDP

```rust
// Create a WebRTC-style SDP with ICE and DTLS
let webrtc_sdp = SdpBuilder::new("WebRTC Session")
    .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")
    .group("BUNDLE", &["audio", "video"])
    .ice_ufrag("F7gI")
    .ice_pwd("x9cml/YzichV2+XlhiMu8g")
    .fingerprint("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24")
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
        .ice_candidate("1 1 UDP 2130706431 192.168.1.100 9 typ host")
        .done()
    .build()?;
```

## Combining SIP and SDP

To include an SDP body in a SIP message, you can use the `body` method:

```rust
// Create an SDP offer using the builder
let sdp_offer = SdpBuilder::new("Call Offer")
    .origin("-", "1234567890", "1", "IN", "IP4", "192.168.1.100")
    .connection("IN", "IP4", "192.168.1.100")
    .time("0", "0")
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;

// Use the SDP in an INVITE request
let invite_with_sdp = SimpleRequestBuilder::invite("sip:bob@example.com")?
    .from("Alice", "sip:alice@example.com", Some("1928301774"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("a84b4c76e66710@alice-pc.example.com")
    .cseq(314159)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    .max_forwards(70)
    .contact("sip:alice@alice-pc.example.com", None)
    .content_type("application/sdp")
    .body(sdp_offer.to_string())
    .build();
```

## Important Builder Methods

Here are some of the most commonly used builder methods:

### Request Builder Methods
- `from()`: Sets the From header
- `to()`: Sets the To header
- `call_id()`: Sets the Call-ID header
- `cseq()`: Sets the CSeq header
- `via()`: Adds a Via header
- `max_forwards()`: Sets the Max-Forwards header
- `contact()`: Sets the Contact header
- `content_type()`: Sets the Content-Type header
- `body()`: Sets the message body
- `header()`: Adds a custom header
- `build()`: Finalizes and returns the built message

### SDP Builder Methods
- `origin()`: Sets the origin (o=) field
- `connection()`: Sets the connection (c=) field
- `time()`: Sets the time (t=) field
- `media_audio()`: Starts building an audio media section
- `media_video()`: Starts building a video media section
- `formats()`: Sets the media formats
- `rtpmap()`: Adds an rtpmap attribute
- `fmtp()`: Adds an fmtp attribute
- `direction()`: Sets the media direction
- `done()`: Completes a media section
- `build()`: Finalizes and returns the built SDP

## Conclusion

The builder pattern provides a clean, intuitive way to create SIP and SDP messages. By chaining method calls, you can construct complex messages with minimal code and ensure that they adhere to the SIP and SDP specifications.

In the next tutorial, we'll dive deeper into SIP requests, exploring the different request methods, their specific requirements, and how to handle them properly.

## Exercise

1. Create a SIP REGISTER request with multiple Contact headers and an Expires header.
2. Create a SIP 302 Moved Temporarily response with a Contact header pointing to a new location.
3. Create an SDP message for a video call with H.264 and VP8 codecs, then include it in a SIP INVITE request. 