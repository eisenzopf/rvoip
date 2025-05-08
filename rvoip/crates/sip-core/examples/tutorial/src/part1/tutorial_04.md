# SIP Requests in Depth

In this tutorial, we'll dive deeper into SIP requests, exploring the different request methods, their specific requirements, and how to implement them using the `rvoip-sip-core` library. Building on what we learned about the builder pattern in the previous tutorial, we'll now focus on the semantics and usage of each request type.

## SIP Request Methods

The SIP protocol defines several request methods, each serving a specific purpose in establishing, managing, or terminating sessions. The core methods defined in [RFC 3261](https://tools.ietf.org/html/rfc3261) are:

1. **INVITE**: Initiates a session
2. **ACK**: Confirms receipt of a final response to INVITE
3. **BYE**: Terminates a session
4. **CANCEL**: Cancels a pending request
5. **REGISTER**: Registers contact information
6. **OPTIONS**: Queries capabilities

Additional methods defined in extensions include:

7. **SUBSCRIBE**: Requests notification of an event
8. **NOTIFY**: Sends a notification of an event
9. **MESSAGE**: Sends an instant message
10. **REFER**: Asks recipient to issue a request
11. **INFO**: Sends mid-session information
12. **UPDATE**: Modifies session parameters

Let's explore each of these methods in detail.

## INVITE Requests

The INVITE method is used to establish a session between user agents. It typically contains an SDP body describing the media capabilities of the caller.

### Detailed INVITE Example

```rust
let invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id("3848276298220188511@atlanta.example.com")
    .cseq(314159)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@atlanta.example.com", None)
    // Add standard but optional headers
    .content_type("application/sdp")
    .content_length(None)  // Will be calculated automatically
    .user_agent("SIPClient/1.0")
    .accept("application/sdp")
    .allow(&["INVITE", "ACK", "CANCEL", "BYE", "NOTIFY", "REFER", "OPTIONS"])
    .supported(&["replaces", "100rel"])
    .header(TypedHeader::Other(HeaderName::Other("Session-Expires".to_string()), 
            HeaderValue::text("3600;refresher=uac")))
    .header(TypedHeader::Other(HeaderName::Other("Min-SE".to_string()), 
            HeaderValue::text("90")))
    .body("v=0\r\n\
           o=alice 2890844526 2890844526 IN IP4 atlanta.example.com\r\n\
           s=Call with Bob\r\n\
           c=IN IP4 atlanta.example.com\r\n\
           t=0 0\r\n\
           m=audio 49170 RTP/AVP 0 8 97\r\n\
           a=rtpmap:0 PCMU/8000\r\n\
           a=rtpmap:8 PCMA/8000\r\n\
           a=rtpmap:97 iLBC/8000\r\n\
           a=sendrecv\r\n")
    .build();
```

### Important INVITE Headers

- **Contact**: Specifies where subsequent requests should be sent
- **Allow**: Lists methods supported by the UAC
- **Supported**: Lists SIP extensions supported by the UAC
- **Session-Expires**: Specifies session refresh interval
- **Min-SE**: Minimum session expiration
- **Content-Type**: Usually "application/sdp" for INVITE requests

## REGISTER Requests

The REGISTER method is used to register contact information for a user with a SIP registrar server. It associates a SIP URI (Address-of-Record) with one or more contact URIs where the user can be reached.

### REGISTER with Authentication

```rust
let register = SimpleRequestBuilder::register("sip:registrar.example.com")?
    .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    .to("Alice", "sip:alice@example.com", None)
    .call_id("1j9FpLxk3uxtm8tn@alice-pc.example.com")
    .cseq(2)
    .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@alice-pc.example.com", None)
    .expires_seconds(3600)
    // Add authentication header
    .header(TypedHeader::Other(
        HeaderName::Other("Authorization".to_string()),
        HeaderValue::text("Digest username=\"alice@example.com\", \
                          realm=\"example.com\", \
                          nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", \
                          uri=\"sip:registrar.example.com\", \
                          response=\"e6f99bf42fe01fc304d3d4eee7dddd44\", \
                          algorithm=MD5, \
                          cnonce=\"0a4f113b\", \
                          opaque=\"5ccc069c403ebaf9f0171e9517f40e41\", \
                          qop=auth, \
                          nc=00000001")
    ))
    .build();
```

### Important REGISTER Headers

- **Contact**: The URI where the user can be reached
- **Expires**: How long the registration should be valid (in seconds)
- **Authorization**: Credentials for authentication
- **To/From**: Both typically contain the Address-of-Record being registered

## SUBSCRIBE and NOTIFY Requests

The SUBSCRIBE method is used to request notification of an event or set of events at a later time. The NOTIFY method is used to inform subscribers of events.

### SUBSCRIBE Example

```rust
let subscribe = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
    .cseq(1)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@atlanta.example.com", None)
    // Event package and subscription details
    .header(TypedHeader::Other(HeaderName::Other("Event".to_string()), 
            HeaderValue::text("presence")))
    .header(TypedHeader::Other(HeaderName::Other("Accept".to_string()), 
            HeaderValue::text("application/pidf+xml")))
    .expires_seconds(3600)
    .build();
```

### Important SUBSCRIBE Headers

- **Event**: Specifies the event package being subscribed to
- **Accept**: Specifies acceptable body formats for NOTIFY requests
- **Expires**: How long the subscription should be valid

## REFER Requests

The REFER method is used to ask the recipient to issue a request. It's commonly used for call transfer scenarios.

### REFER Example

```rust
let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
    .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
    .cseq(101)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@atlanta.example.com", None)
    // Refer-To header specifies transfer target
    .header(TypedHeader::Other(HeaderName::Other("Refer-To".to_string()), 
            HeaderValue::text("<sip:carol@chicago.example.com>")))
    .header(TypedHeader::Other(HeaderName::Other("Referred-By".to_string()), 
            HeaderValue::text("<sip:alice@atlanta.example.com>")))
    .build();
```

### Important REFER Headers

- **Refer-To**: Specifies the URI that the recipient should send a request to
- **Referred-By**: Indicates the identity of the referring party

## MESSAGE Requests

The MESSAGE method is used for instant messaging functionality within SIP. It carries the message content directly in the request body.

### MESSAGE Example

```rust
let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
    .cseq(1)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .content_type("text/plain")
    .body("Hello Bob, this is Alice. Can we meet at 2pm today?")
    .build();
```

### Important MESSAGE Headers

- **Content-Type**: Specifies the format of the message body (often "text/plain")
- **Content-Length**: Size of the message body in bytes

## UPDATE Requests

The UPDATE method is used to modify the state of a session without changing the state of the dialog. It's often used for session timer refreshes or to update media parameters.

### UPDATE Example

```rust
let update = SimpleRequestBuilder::new(Method::Update, "sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
    .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
    .cseq(2)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@atlanta.example.com", None)
    .content_type("application/sdp")
    // Session timer headers
    .header(TypedHeader::Other(HeaderName::Other("Session-Expires".to_string()), 
            HeaderValue::text("1800;refresher=uac")))
    .body("v=0\r\n\
           o=alice 2890844526 2890844527 IN IP4 atlanta.example.com\r\n\
           s=Call with Bob\r\n\
           c=IN IP4 atlanta.example.com\r\n\
           t=0 0\r\n\
           m=audio 49170 RTP/AVP 0\r\n\
           a=rtpmap:0 PCMU/8000\r\n\
           a=sendrecv\r\n")
    .build();
```

### Important UPDATE Headers

- **Session-Expires**: Updates the session refresh interval
- **Content-Type**: Usually "application/sdp" when updating media parameters

## OPTIONS Requests

The OPTIONS method is used to query the capabilities of a server or another user agent. The response typically includes Allow, Accept, and other capability-related headers.

### OPTIONS Example

```rust
let options = SimpleRequestBuilder::options("sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id("7a9f2f899ndf98f7a8fd9f890as87f9a")
    .cseq(1)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .accept("application/sdp")
    .header(TypedHeader::Other(HeaderName::Other("Accept-Language".to_string()), 
            HeaderValue::text("en")))
    .header(TypedHeader::Other(HeaderName::Other("Accept-Encoding".to_string()), 
            HeaderValue::text("identity")))
    .build();
```

### Important OPTIONS Headers

- **Accept**: Specifies acceptable body formats
- **Accept-Language**: Specifies acceptable languages
- **Accept-Encoding**: Specifies acceptable encodings

## ACK Requests

The ACK method is used to acknowledge receipt of a final response to an INVITE request. It's part of the three-way handshake for establishing a session.

### ACK Example

```rust
let ack = SimpleRequestBuilder::ack("sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
    .call_id("3848276298220188511@atlanta.example.com")
    .cseq(314159)  // Must match the INVITE CSeq number
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnewbranch"))
    .max_forwards(70)
    .build();
```

### Important ACK Headers

- **CSeq**: Must match the CSeq number of the INVITE being acknowledged
- **Via**: Typically contains a new branch parameter

## CANCEL Requests

The CANCEL method is used to cancel a pending request. It's commonly used to cancel an INVITE that hasn't received a final response yet.

### CANCEL Example

```rust
let cancel = SimpleRequestBuilder::cancel("sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id("3848276298220188511@atlanta.example.com")
    .cseq(314159)  // Must match the request being canceled
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))  // Must match the request being canceled
    .max_forwards(70)
    .build();
```

### Important CANCEL Headers

- **Call-ID**: Must match the Call-ID of the request being canceled
- **To/From/CSeq**: Must match the request being canceled
- **Via**: Must match the topmost Via header of the request being canceled

## BYE Requests

The BYE method is used to terminate an established session.

### BYE Example

```rust
let bye = SimpleRequestBuilder::bye("sip:bob@biloxi.example.com")?
    .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
    .to("Bob", "sip:bob@biloxi.example.com", Some("314159"))
    .call_id("3848276298220188511@atlanta.example.com")
    .cseq(2)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds8"))
    .max_forwards(70)
    .build();
```

## Common Request Headers

All SIP requests share a common set of headers:

1. **From**: Identifies the initiator of the request
2. **To**: Identifies the intended recipient of the request
3. **Call-ID**: Unique identifier for the call
4. **CSeq**: Command sequence number
5. **Via**: Indicates the path taken by the request
6. **Max-Forwards**: Limits the number of hops to the destination

Additional common headers include:

7. **Contact**: Where subsequent requests should be sent
8. **Content-Type**: Format of the message body
9. **Content-Length**: Size of the message body in bytes
10. **User-Agent**: Information about the client software

## Request URIs

The Request-URI is a SIP or SIPS URI that identifies the resource to which the request is being addressed. It appears in the start-line of the request and determines where the request is sent.

```
INVITE sip:bob@biloxi.example.com SIP/2.0
```

Here, `sip:bob@biloxi.example.com` is the Request-URI.

The Request-URI can contain various parameters:

```
sip:bob@biloxi.example.com:5060;transport=tcp;lr
```

This URI specifies:
- User: bob
- Domain: biloxi.example.com
- Port: 5060
- Transport: TCP
- Parameter: lr (loose routing)

## Conclusion

SIP requests are the building blocks of SIP-based communication. Each request type serves a specific purpose in the lifecycle of a SIP session, from establishment to termination. The `rvoip-sip-core` library provides a flexible and intuitive builder pattern for creating these requests with all their required and optional headers.

In the next tutorial, we'll explore SIP responses in depth, learning how to create and handle the various response types that correspond to these requests.

## Exercises

1. Create an INVITE request with a custom SDP body that offers both audio and video media.
2. Create a REGISTER request for multiple contacts with different expiration times.
3. Create a REFER request for attended transfer (with a Replaces header).
4. Create a MESSAGE request with a multipart/mixed body containing both text and an image.
5. Create an OPTIONS request that queries for specific capabilities (e.g., support for specific SIP extensions). 