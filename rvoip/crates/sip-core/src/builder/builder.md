# SIP Builders

This module provides builder patterns for creating SIP messages with a fluent, chainable API.
The builders ([`SimpleRequestBuilder`] and [`SimpleResponseBuilder`]) support all common SIP headers defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261)
and various extensions.

## Overview

The builder pattern in this crate enables the construction of complex SIP messages through a 
series of method calls that return `self`, allowing for a fluent API. This approach simplifies 
the creation of valid SIP messages by handling many of the validation requirements and header 
formatting internally.

## Type System

The builder system is built on a strong type system with several key components:

- **TypedHeader**: An enum representing all supported header types with proper typing
- **HeaderTrait**: A trait implemented by all header types, enabling conversion to raw headers
- **HeaderSetter**: A trait implemented by builders for setting typed headers
- **Extension Traits**: Header-specific extension traits that add convenient builder methods

This type system ensures that headers are properly formatted and validates input at compile time
when possible, with runtime validation for more complex scenarios.

## Supported Headers

The builder supports the following SIP headers:

- **Core Headers**
  - [`From`][headers::from], [`To`][headers::to], [`Contact`][headers::contact], [`Call-ID`][headers::call_id], [`CSeq`][headers::cseq], [`Via`][headers::via], [`Max-Forwards`][headers::max_forwards]

- **Content-Related Headers**
  - [`Content-Type`][headers::content], [`Content-Length`][headers::content], [`Content-Encoding`][headers::content_encoding], [`Content-Language`][headers::content_language], [`Content-Disposition`][headers::content_disposition]
  - [`Accept`][headers::accept], [`Accept-Encoding`][headers::accept_encoding], [`Accept-Language`][headers::accept_language]

- **Authentication Headers**
  - [`Authorization`][headers::authorization], [`WWW-Authenticate`][headers::www_authenticate], [`Proxy-Authenticate`][headers::proxy_authenticate], [`Proxy-Authorization`][headers::proxy_authorization], [`Authentication-Info`][headers::authentication_info]

- **Routing Headers**
  - [`Route`][headers::route], [`Record-Route`][headers::record_route], [`Path`][headers::path]

- **Feature Headers**
  - [`Allow`][headers::allow], [`Supported`][headers::supported], [`Unsupported`][headers::unsupported], [`Require`][headers::require], [`Proxy-Require`][headers::proxy_require]

- **Miscellaneous Headers**
  - [`User-Agent`][headers::user_agent], [`Server`][headers::server], [`Call-Info`][headers::call_info], [`In-Reply-To`][headers::in_reply_to], [`Reply-To`][headers::reply_to]

## Recommendations

1. **Start with method-specific constructors** for common SIP methods:
   ```rust
   use rvoip_sip_core::builder::SimpleRequestBuilder;
   
   let builder = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
   ```

2. **Chain method calls** to build your message:
   ```rust
   use rvoip_sip_core::builder::SimpleRequestBuilder;
   
   let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
       .from("Alice", "sip:alice@example.com", Some("tag1234"))
       .to("Bob", "sip:bob@example.com", None)
       .call_id("a84b4c76e66710")
       .build();
   ```

3. **Use header extension traits** for specialized headers:
   ```rust
   use rvoip_sip_core::builder::SimpleRequestBuilder;
   use rvoip_sip_core::builder::headers::ContentBuilderExt;
   use std::str::FromStr;
   use rvoip_sip_core::types::sdp::SdpSession;
   
   // Create an SDP session with proper connection info
   let sdp = SdpSession::from_str("v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=Call\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\n").unwrap();
   
   let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
       .content_type_sdp()
       .sdp_body(&sdp)
       .build();
   ```

4. **Build on existing messages** when creating related requests:
   ```rust
   use rvoip_sip_core::builder::SimpleRequestBuilder;
   use rvoip_sip_core::types::{Method, Uri, Request};
   use std::str::FromStr;
   
   // Create or get a request from somewhere
   let uri = Uri::from_str("sip:bob@example.com").unwrap();
   let existing_request = Request::new(Method::Invite, uri);
   
   let new_builder = SimpleRequestBuilder::from_request(existing_request);
   ```

## Limitations

- **Header Validation**: While many headers are validated, some complex validation scenarios may not be caught until message transmission
- **Custom Headers**: Custom headers require use of the generic `header()` method with appropriate type conversions
- **Performance**: The builder pattern creates intermediate objects, which may have performance implications for high-volume scenarios
- **Mutability**: Each builder method consumes and returns a new builder, requiring careful handling in complex logic

## Examples By Header Category

### Core Headers

#### Request Example with Core Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::Method;

let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    // From header with display name, URI, and tag
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    // To header with display name, URI, and no tag (common for initial requests)
    .to("Bob", "sip:bob@example.com", None)
    // Call-ID header uniquely identifying this dialog
    .call_id("a84b4c76e66710")
    // CSeq header with sequence number (method is added automatically)
    .cseq(1)
    // Via header with sending host, transport protocol, and branch parameter
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    // Contact header specifying where to send responses
    .contact("sip:alice@192.168.0.2:5060", Some("Alice"))
    // Max-Forwards header to limit routing hops
    .max_forwards(70)
    .build();
```

#### Response Example with Core Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::types::{StatusCode, Method};

let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    // From header copied from request, but with added tag
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    // To header copied from request, with added tag in responses
    .to("Alice", "sip:alice@example.com", Some("tag1234"))
    // Call-ID copied from request
    .call_id("a84b4c76e66710")
    // CSeq with sequence number and method from request
    .cseq(1, Method::Invite)
    // Via headers copied from request
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    // Contact header for where subsequent requests should be sent
    .contact("sip:bob@192.168.1.3:5060", Some("Bob"))
    .build();
```

### Content-Related Headers

#### Request Example with Content Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::ContentBuilderExt;
use rvoip_sip_core::builder::headers::ContentEncodingExt;
use rvoip_sip_core::builder::headers::ContentLanguageExt;
use rvoip_sip_core::builder::headers::AcceptExt;
use rvoip_sip_core::builder::headers::AcceptEncodingExt;
use rvoip_sip_core::builder::headers::AcceptLanguageExt;
use rvoip_sip_core::types::sdp::SdpSession;
use std::str::FromStr;

// Create a simple SDP message with proper connection info
let sdp_text = "\
v=0\r
o=alice 2890844526 2890844526 IN IP4 alice.example.org\r
s=SIP Call\r
c=IN IP4 alice.example.org\r
t=0 0\r
m=audio 49170 RTP/AVP 0 8 97\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:97 iLBC/8000\r
";
let sdp = SdpSession::from_str(sdp_text).unwrap();

// Create message with Content headers
let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("content-test-123")
    
    // Content-Type set via convenient extension method
    .content_type_sdp()
    
    // Content-Encoding header
    .content_encoding("identity")
    
    // Content-Language header
    .content_language("en")
    
    // Accept header - what content types the UAC accepts in response
    .accept("application/sdp", None)
    .accept("text/plain", None)
    
    // Accept-Encoding header
    .accept_encoding("gzip", None)
    .accept_encoding("identity", None)
    
    // Accept-Language header
    .accept_language("en", None)
    .accept_language("fr", None)
    
    // Set body and Content-Length is managed automatically
    .sdp_body(&sdp)
    .build();
```

#### Response Example with Content Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::headers::ContentBuilderExt;
use rvoip_sip_core::builder::headers::ContentEncodingExt;
use rvoip_sip_core::builder::headers::ContentLanguageExt;
use rvoip_sip_core::types::{StatusCode, Method};
use rvoip_sip_core::types::sdp::SdpSession;
use std::str::FromStr;

// Create an SDP answer with proper connection info
let sdp_text = "\
v=0\r
o=bob 2890844527 2890844527 IN IP4 bob.example.org\r
s=SIP Call\r
c=IN IP4 bob.example.org\r
t=0 0\r
m=audio 49180 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";
let sdp = SdpSession::from_str(sdp_text).unwrap();

// Create response with Content headers
let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", Some("tag1234")) 
    .call_id("content-test-123")
    .cseq(1, Method::Invite)
    
    // Content-Type set via convenient extension method
    .content_type_sdp()
    
    // Content-Encoding header
    .content_encoding("identity")
    
    // Content-Language header
    .content_language("en")
    
    // Set body and Content-Length is managed automatically
    .sdp_body(&sdp)
    .build();
```

### Authentication Headers

#### Request Example with Authentication Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::AuthorizationExt;
use rvoip_sip_core::builder::headers::ProxyAuthorizationExt;

// Create a request with Authorization header
let request = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", None)
    .call_id("auth-test-123")
    .cseq(2)
    
    // Use extension trait for digest authentication
    .authorization_digest(
        "alice",                     // username
        "registrar.example.com",     // realm
        "dcd98b7102dd2f0e",          // nonce from server
        "5ccc069c403ebaf9f0171e9517f40e41", // response
        Some("0a4f113b"),            // cnonce
        Some("auth"),                // qop
        Some("00000001"),            // nc
        Some("REGISTER"),            // method
        Some("sip:registrar.example.com"), // URI
        Some("MD5"),                 // algorithm
        Some("opaque-data")          // opaque
    )
    
    // Proxy-Authorization for traversing proxies that require auth
    .proxy_authorization_digest(
        "alice",                     // username
        "proxy.example.com",         // realm
        "6944656576e46aa3",          // nonce from proxy
        "sip:registrar.example.com", // URI
        "5ccc069c403ebaf9f0171e9517f40e41", // response
        Some("MD5"),                 // algorithm
        Some("0a4f113b"),            // cnonce
        Some("opaque-data"),         // opaque
        Some("auth"),                // qop
        Some("00000001")             // nc
    )
    .build();
```

#### Response Example with Authentication Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::headers::WwwAuthenticateExt;
use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
use rvoip_sip_core::types::{StatusCode, Method};

// Create 401 Unauthorized response with authentication challenge
let response_401 = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
    .from("registrar", "sip:registrar.example.com", Some("reg-tag"))
    .to("Alice", "sip:alice@example.com", Some("tag5678"))
    .call_id("auth-test-123")
    .cseq(2, Method::Register)
    
    // Add WWW-Authenticate challenge
    .www_authenticate_digest(
        "registrar.example.com",   // realm
        "dcd98b7102dd2f0e",        // nonce
        None,                      // opaque
        Some("MD5"),               // algorithm
        Some(vec!["auth"]),        // qop options
        Some(false),               // stale
        None                       // domain
    )
    .build();

// Create 407 Proxy Authentication Required response
let response_407 = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    .from("proxy", "sip:proxy.example.com", Some("proxy-tag"))
    .to("Alice", "sip:alice@example.com", Some("tag5678"))
    .call_id("auth-test-123") 
    .cseq(2, Method::Register)
    
    // Add Proxy-Authenticate challenge
    .proxy_authenticate_digest(
        "proxy.example.com",       // realm
        "6944656576e46aa3",        // nonce
        None,                      // opaque
        Some("MD5"),               // algorithm
        Some(vec!["auth"]),        // qop
        Some(false),               // stale
        None                       // domain
    )
    .build();
```

### Routing Headers

#### Request Example with Routing Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::{RouteBuilderExt, PathBuilderExt};
use rvoip_sip_core::types::{Method, Uri};
use std::str::FromStr;

// Create request with routing headers
let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();

let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("routing-test-123")
    
    // Route header to force specific routing path
    .route_uri(uri1.clone())
    .route_uri(uri2.clone())
    
    // Path header often used in REGISTER
    .path("sip:edge.example.com;lr").unwrap()
    .build();
```

#### Response Example with Routing Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::headers::{RecordRouteBuilderExt, PathBuilderExt};
use rvoip_sip_core::types::{StatusCode, Method, Uri};
use std::str::FromStr;

// Create URIs for record route
let rr_uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
let rr_uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();

// Create response with Record-Route headers
let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", Some("tag1234"))
    .call_id("routing-test-123")
    .cseq(1, Method::Invite)
    
    // Record-Route headers added by proxies to stay in the routing path
    .record_route_uri(rr_uri1.clone())
    .record_route_uri(rr_uri2.clone())
    
    // For a REGISTER 200 OK with Path header
    .path("sip:edge.example.com;lr").unwrap()
    .build();
```

### Feature Headers

#### Request Example with Feature Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::{AllowBuilderExt, SupportedBuilderExt, RequireBuilderExt, UnsupportedBuilderExt, ProxyRequireBuilderExt};
use rvoip_sip_core::types::Method;

// Create OPTIONS request with feature headers
let request = SimpleRequestBuilder::options("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("feature-test-123")
    
    // Allow header to indicate supported methods
    .allow_method(Method::Invite)
    .allow_method(Method::Ack)
    .allow_method(Method::Cancel)
    .allow_method(Method::Bye)
    .allow_method(Method::Options)
    
    // Or use convenient method to add standard methods at once
    // .allow_standard_methods()
    
    // Supported header for supported extensions
    .supported_tag("path")
    .supported_tag("outbound")
    .supported_tag("gruu")
    
    // Require header for required extensions
    .require_tag("100rel")
    .require_tag("replaces")
    
    // Unsupported header to explicitly indicate unsupported extensions
    .unsupported_tag("join")
    
    // Proxy-Require header for features required by proxies
    .proxy_require_tag("p-record-route")
    .build();
```

#### Response Example with Feature Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::headers::{AllowBuilderExt, SupportedBuilderExt, UnsupportedBuilderExt};
use rvoip_sip_core::types::{StatusCode, Method};

// Create 200 OK for OPTIONS with feature headers
let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", Some("tag1234"))
    .call_id("feature-test-123")
    .cseq(1, Method::Options)
    
    // Allow header showing supported methods
    .allow_method(Method::Invite)
    .allow_method(Method::Ack)
    .allow_method(Method::Cancel)
    .allow_method(Method::Bye)
    .allow_method(Method::Options)
    .allow_method(Method::Message)
    
    // Supported header showing supported extensions
    .supported_tag("path")
    .supported_tag("outbound")
    .supported_tag("timer")
    
    // Unsupported header explicitly stating unsupported extensions
    .unsupported_tag("join")
    .unsupported_tag("100rel")
    .build();

// Create 420 Bad Extension for unsupported Require
let response_420 = SimpleResponseBuilder::new(StatusCode::BadExtension, None)
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", Some("tag1234"))
    .call_id("feature-test-123")
    .cseq(1, Method::Invite)
    
    // Unsupported header listing the unsupported extensions
    .unsupported_tag("join")
    .unsupported_tag("100rel")
    .build();
```

### Miscellaneous Headers

#### Request Example with Miscellaneous Headers

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::UserAgentBuilderExt;
use rvoip_sip_core::types::Method;

// Create INVITE with miscellaneous headers
let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("misc-test-123")
    
    // User-Agent header to identify your SIP client
    .user_agent("MyCompany SIP Client 1.0")
    
    .build();
```

#### Response Example with Miscellaneous Headers

```rust
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::headers::UserAgentBuilderExt;
use rvoip_sip_core::types::{StatusCode, Method};

// Create 200 OK with Server header
let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    .from("Bob", "sip:bob@example.com", Some("tag5678"))
    .to("Alice", "sip:alice@example.com", Some("tag1234"))
    .call_id("misc-test-123")
    .cseq(1, Method::Invite)
    
    // Server header to identify the UAS (uses same trait as User-Agent)
    .user_agent("MyCompany SIP Server 2.1")
    .build();
```

### Complete SIP Dialogs with SDP

The following example demonstrates a complete SIP dialog with SDP, showing how both 
requests and responses are constructed through a typical call flow:

```rust
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::builder::headers::{ContentBuilderExt, UserAgentBuilderExt};
use rvoip_sip_core::types::{StatusCode, Method, sdp::SdpSession};
use std::str::FromStr;

// 1. Create INVITE with SDP offer
let sdp_offer = "\
v=0\r
o=alice 2890844526 2890844526 IN IP4 alice.example.org\r
s=SIP Call\r
c=IN IP4 alice.example.org\r
t=0 0\r
m=audio 49170 RTP/AVP 0 8 97\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:97 iLBC/8000\r
";
let alice_sdp = SdpSession::from_str(sdp_offer).unwrap();

let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("call-xyz-789")
    .cseq(1)
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .contact("sip:alice@192.168.0.2:5060", None)
    .user_agent("Alice SIP Client 1.0")
    .sdp_body(&alice_sdp)
    .build();

// 2. Create 100 Trying provisional response
let trying = SimpleResponseBuilder::trying()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("call-xyz-789")
    .cseq(1, Method::Invite)
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .user_agent("SIP Proxy 1.0")
    .build();

// 3. Create 180 Ringing provisional response
let ringing = SimpleResponseBuilder::ringing()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", Some("tag-bob-456"))
    .call_id("call-xyz-789")
    .cseq(1, Method::Invite)
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .contact("sip:bob@192.168.1.3:5060", None)
    .user_agent("Bob SIP Client 2.0")
    .build();

// 4. Create 200 OK with SDP answer
let sdp_answer = "\
v=0\r
o=bob 2890844527 2890844527 IN IP4 bob.example.org\r
s=SIP Call\r
c=IN IP4 bob.example.org\r
t=0 0\r
m=audio 49180 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";
let bob_sdp = SdpSession::from_str(sdp_answer).unwrap();

let ok = SimpleResponseBuilder::ok()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", Some("tag-bob-456"))
    .call_id("call-xyz-789")
    .cseq(1, Method::Invite)
    .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    .contact("sip:bob@192.168.1.3:5060", None)
    .user_agent("Bob SIP Client 2.0")
    .sdp_body(&bob_sdp)
    .build();

// 5. Create ACK request
let ack = SimpleRequestBuilder::ack("sip:bob@192.168.1.3:5060").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", Some("tag-bob-456"))
    .call_id("call-xyz-789")
    .cseq(1) // ACK has same CSeq as INVITE
    .via("alice.example.com", "UDP", Some("z9hG4bK887asdhds")) // new branch
    .user_agent("Alice SIP Client 1.0")
    .build();

// 6. Later, create BYE request
let bye = SimpleRequestBuilder::bye("sip:bob@192.168.1.3:5060").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", Some("tag-bob-456"))
    .call_id("call-xyz-789")
    .cseq(2) // BYE gets new CSeq
    .via("alice.example.com", "UDP", Some("z9hG4bK776ztuvw"))
    .user_agent("Alice SIP Client 1.0")
    .build();

// 7. Create 200 OK for BYE
let ok_bye = SimpleResponseBuilder::ok()
    .from("Alice", "sip:alice@example.com", Some("tag-alice-123"))
    .to("Bob", "sip:bob@example.com", Some("tag-bob-456"))
    .call_id("call-xyz-789")
    .cseq(2, Method::Bye)
    .via("alice.example.com", "UDP", Some("z9hG4bK776ztuvw"))
    .user_agent("Bob SIP Client 2.0")
    .build();
```

[headers::from]: crate::builder::headers
[headers::to]: crate::builder::headers
[headers::contact]: crate::builder::headers
[headers::call_id]: crate::builder::headers::call_id
[headers::cseq]: crate::builder::headers
[headers::via]: crate::builder::headers
[headers::max_forwards]: crate::builder::headers
[headers::content]: crate::builder::headers::content
[headers::content_encoding]: crate::builder::headers::content_encoding
[headers::content_language]: crate::builder::headers::content_language
[headers::content_disposition]: crate::builder::headers::content_disposition
[headers::accept]: crate::builder::headers::accept
[headers::accept_encoding]: crate::builder::headers::accept_encoding
[headers::accept_language]: crate::builder::headers::accept_language
[headers::authorization]: crate::builder::headers::authorization
[headers::www_authenticate]: crate::builder::headers::www_authenticate
[headers::proxy_authenticate]: crate::builder::headers::proxy_authenticate
[headers::proxy_authorization]: crate::builder::headers::proxy_authorization
[headers::authentication_info]: crate::builder::headers::authentication_info
[headers::route]: crate::builder::headers::route
[headers::record_route]: crate::builder::headers::record_route
[headers::path]: crate::builder::headers::path
[headers::allow]: crate::builder::headers::allow
[headers::supported]: crate::builder::headers::supported
[headers::unsupported]: crate::builder::headers::unsupported
[headers::require]: crate::builder::headers::require
[headers::proxy_require]: crate::builder::headers::proxy_require
[headers::user_agent]: crate::builder::headers::user_agent
[headers::server]: crate::builder::headers::server
[headers::call_info]: crate::builder::headers
[headers::in_reply_to]: crate::builder::headers::in_reply_to
[headers::reply_to]: crate::builder::headers::reply_to 