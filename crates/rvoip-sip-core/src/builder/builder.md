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

The builder supports the following SIP headers, organized by their defining RFC:

- **RFC 3261 (SIP: Session Initiation Protocol)**
  - [`From`][headers::from::FromBuilderExt] - Indicates the initiator of the request
  - [`To`][headers::to::ToBuilderExt] - Specifies the logical recipient of the request
  - [`Contact`][headers::contact::ContactBuilderExt] - Provides a URI for direct communication to a specific instance
  - [`Call-ID`][headers::call_id::CallIdBuilderExt] - Uniquely identifies a call or registration
  - [`CSeq`][headers::cseq::CSeqBuilderExt] - Contains a sequence number and request method for ordering requests
  - [`Via`][headers::via::ViaBuilderExt] - Indicates the path taken by the request and where responses should be sent
  - [`Max-Forwards`][headers::max_forwards::MaxForwardsBuilderExt] - Limits the number of hops a request can make
  - [`Content-Type`][headers::content_type::ContentTypeBuilderExt] - Indicates the media type of the message body
  - [`Content-Length`][headers::content_length::ContentLengthBuilderExt] - Indicates the size of the message body in bytes
  - [`Content-Encoding`][headers::content_encoding::ContentEncodingExt] - Indicates any encodings applied to the message body
  - [`Content-Language`][headers::content_language::ContentLanguageExt] - Indicates the language of the message body
  - [`Content-Disposition`][headers::content_disposition::ContentDispositionExt] - Describes how the message body should be handled
  - [`Accept`][headers::accept::AcceptExt] - Specifies acceptable media types for the response
  - [`Accept-Encoding`][headers::accept_encoding::AcceptEncodingExt] - Specifies acceptable encodings for the response
  - [`Accept-Language`][headers::accept_language::AcceptLanguageExt] - Specifies acceptable languages for the response
  - [`Alert-Info`][headers::alert_info::AlertInfoBuilderExt] - Specifies alternative ring or ringback tones for calls
  - [`Allow`][headers::allow::AllowBuilderExt] - Lists the methods supported by the UA
  - [`Supported`][headers::supported::SupportedBuilderExt] - Lists extensions supported by the UA
  - [`Unsupported`][headers::unsupported::UnsupportedBuilderExt] - Lists extensions not supported by the UA
  - [`Require`][headers::require::RequireBuilderExt] - Lists extensions that must be supported for the request to succeed
  - [`Proxy-Require`][headers::proxy_require::ProxyRequireBuilderExt] - Lists extensions that must be supported by proxies
  - [`Route`][headers::route::RouteBuilderExt] - Specifies a list of proxies that must be traversed by the request
  - [`Record-Route`][headers::record_route::RecordRouteBuilderExt] - Added by proxies to stay in the signaling path
  - [`User-Agent`][headers::user_agent::UserAgentBuilderExt] - Identifies the client software originating the request
  - [`Server`][headers::server::ServerBuilderExt] - Identifies the server software generating the response
  - [`Warning`][headers::warning::WarningBuilderExt] - Carries additional information about response status
  - [`Retry-After`][headers::retry_after::RetryAfterBuilderExt] - Indicates when to retry after certain error responses
  - [`MIME-Version`][headers::mime_version::MimeVersionBuilderExt] - Indicates the version of MIME protocol being used
  - [`Organization`][headers::organization::OrganizationBuilderExt] - Indicates the organization to which the entity belongs
  - [`Priority`][headers::priority::PriorityBuilderExt] - Indicates the urgency or importance of a request
  - [`Call-Info`][headers::call_info::CallInfoBuilderExt] - Provides additional information about the call
  - [`In-Reply-To`][headers::in_reply_to::InReplyToBuilderExt] - Identifies the Call-IDs that this call references
  - [`Reply-To`][headers::reply_to::ReplyToBuilderExt] - Specifies the address to use for replies outside SIP
  - [`Error-Info`][headers::error_info::ErrorInfoBuilderExt] - Provides pointers to additional error information
  - [`Expires`][headers::expires::ExpiresBuilderExt] - Specifies expiration time for registrations or subscriptions

- **RFC 3262 (Reliability of Provisional Responses in SIP)**
  - [`RSeq`][headers::rseq::RSeqBuilderExt] - Response sequence number for reliable provisional responses

- **RFC 3265 (SIP-Specific Event Notification)**
  - [`Event`][headers::event::EventBuilderExt] - Specifies the event type for subscription or notification

- **RFC 3515 (The SIP Refer Method)**
  - [`Refer-To`][headers::refer_to::ReferToExt] - Specifies the target resource for a referral

- **RFC 3892 (The SIP Referred-By Mechanism)**
  - [`Referred-By`][headers::referred_by::ReferredByExt] - Identifies the entity that initiated a referral

- **RFC 4028 (Session Timers in SIP)**
  - [`Session-Expires`][headers::session_expires::SessionExpiresExt] - Specifies session interval for periodic refreshes
  - [`Min-SE`][headers::min_se::MinSEBuilderExt] - Specifies minimum acceptable session timer interval

- **Content Features**
  - [`Content`][headers::content::ContentBuilderExt] - For message body handling with appropriate Content-Type headers
  - [`Multipart`][multipart::MultipartBuilder] - For creating multipart MIME bodies with multiple content types

- **RFC 3329 (Security Mechanism Agreement for SIP)**
  - [`Authorization`][headers::authorization::AuthorizationExt] - Contains authentication credentials for a user agent
  - [`WWW-Authenticate`][headers::www_authenticate::WwwAuthenticateExt] - Contains authentication challenge issued by a server
  - [`Proxy-Authenticate`][headers::proxy_authenticate::ProxyAuthenticateExt] - Contains authentication challenge issued by a proxy
  - [`Proxy-Authorization`][headers::proxy_authorization::ProxyAuthorizationExt] - Contains authentication credentials for a proxy
  - [`Authentication-Info`][headers::authentication_info::AuthenticationInfoExt] - Contains authentication information

- **RFC 3327 (Path Extension Header Field for SIP)**
  - [`Path`][headers::path::PathBuilderExt] - Specifies path to be used for registrations

- **RFC 3841 (Caller Preferences for SIP)**
  - [`Reason`][headers::reason::ReasonBuilderExt] - Provides information about why a request was generated

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
       .sdp_body(&sdp)  // Sets Content-Type to application/sdp and adds the SDP body
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
use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
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
    .body("v=0\r\no=alice...") // Raw body as string
    .build();
```

#### Using ContentBuilderExt for SDP Bodies

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::headers::ContentBuilderExt;
use rvoip_sip_core::types::sdp::SdpSession;
use std::str::FromStr;

// Parse an SDP session
let sdp = SdpSession::from_str("\
v=0\r
o=alice 2890844526 2890844526 IN IP4 alice.example.org\r
s=SIP Call\r
c=IN IP4 alice.example.org\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
").unwrap();

// Create a request with SDP body
// This automatically sets Content-Type: application/sdp and the body
let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@example.com", Some("tag1234"))
    .to("Bob", "sip:bob@example.com", None)
    .call_id("sdp-body-example")
    .sdp_body(&sdp)  // This handles both Content-Type and body
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
    
    // Add SDP body with one method
    .sdp_body(&sdp)  // Sets Content-Type and body together
    
    // Content-Encoding header
    .content_encoding("identity")
    
    // Content-Language header
    .content_language("en")
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
let response_401 = WwwAuthenticateExt::www_authenticate_digest(
    SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("registrar", "sip:registrar.example.com", Some("reg-tag"))
        .to("Alice", "sip:alice@example.com", Some("tag5678"))
        .call_id("auth-test-123")
        .cseq(2, Method::Register),
    
    // Add WWW-Authenticate challenge
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
let response_407 = ProxyAuthenticateExt::proxy_authenticate_digest(
    SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
        .from("proxy", "sip:proxy.example.com", Some("proxy-tag"))
        .to("Alice", "sip:alice@example.com", Some("tag5678"))
        .call_id("auth-test-123") 
        .cseq(2, Method::Register),
    
    // Add Proxy-Authenticate challenge
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

[headers::from::FromBuilderExt]: crate::builder::headers::from::FromBuilderExt
[headers::to::ToBuilderExt]: crate::builder::headers::to::ToBuilderExt
[headers::contact::ContactBuilderExt]: crate::builder::headers::contact::ContactBuilderExt
[headers::call_id::CallIdBuilderExt]: crate::builder::headers::call_id::CallIdBuilderExt
[headers::cseq::CSeqBuilderExt]: crate::builder::headers::cseq::CSeqBuilderExt
[headers::via::ViaBuilderExt]: crate::builder::headers::via::ViaBuilderExt
[headers::max_forwards::MaxForwardsBuilderExt]: crate::builder::headers::max_forwards::MaxForwardsBuilderExt
[headers::content::ContentBuilderExt]: crate::builder::headers::content::ContentBuilderExt
[headers::content_type::ContentTypeBuilderExt]: crate::builder::headers::content_type::ContentTypeBuilderExt
[headers::content_length::ContentLengthBuilderExt]: crate::builder::headers::content_length::ContentLengthBuilderExt
[headers::content_encoding::ContentEncodingExt]: crate::builder::headers::content_encoding::ContentEncodingExt
[headers::content_language::ContentLanguageExt]: crate::builder::headers::content_language::ContentLanguageExt
[headers::content_disposition::ContentDispositionExt]: crate::builder::headers::content_disposition::ContentDispositionExt
[headers::accept::AcceptExt]: crate::builder::headers::accept::AcceptExt
[headers::accept_encoding::AcceptEncodingExt]: crate::builder::headers::accept_encoding::AcceptEncodingExt
[headers::accept_language::AcceptLanguageExt]: crate::builder::headers::accept_language::AcceptLanguageExt
[headers::authorization::AuthorizationExt]: crate::builder::headers::authorization::AuthorizationExt
[headers::www_authenticate::WwwAuthenticateExt]: crate::builder::headers::www_authenticate::WwwAuthenticateExt
[headers::proxy_authenticate::ProxyAuthenticateExt]: crate::builder::headers::proxy_authenticate::ProxyAuthenticateExt
[headers::proxy_authorization::ProxyAuthorizationExt]: crate::builder::headers::proxy_authorization::ProxyAuthorizationExt
[headers::authentication_info::AuthenticationInfoExt]: crate::builder::headers::authentication_info::AuthenticationInfoExt
[headers::route::RouteBuilderExt]: crate::builder::headers::route::RouteBuilderExt
[headers::record_route::RecordRouteBuilderExt]: crate::builder::headers::record_route::RecordRouteBuilderExt
[headers::path::PathBuilderExt]: crate::builder::headers::path::PathBuilderExt
[headers::allow::AllowBuilderExt]: crate::builder::headers::allow::AllowBuilderExt
[headers::supported::SupportedBuilderExt]: crate::builder::headers::supported::SupportedBuilderExt
[headers::unsupported::UnsupportedBuilderExt]: crate::builder::headers::unsupported::UnsupportedBuilderExt
[headers::require::RequireBuilderExt]: crate::builder::headers::require::RequireBuilderExt
[headers::proxy_require::ProxyRequireBuilderExt]: crate::builder::headers::proxy_require::ProxyRequireBuilderExt
[headers::user_agent::UserAgentBuilderExt]: crate::builder::headers::user_agent::UserAgentBuilderExt
[headers::server::ServerBuilderExt]: crate::builder::headers::server::ServerBuilderExt
[headers::call_info::CallInfoBuilderExt]: crate::builder::headers::call_info::CallInfoBuilderExt
[headers::in_reply_to::InReplyToBuilderExt]: crate::builder::headers::in_reply_to::InReplyToBuilderExt
[headers::reply_to::ReplyToBuilderExt]: crate::builder::headers::reply_to::ReplyToBuilderExt
[headers::mime_version::MimeVersionBuilderExt]: crate::builder::headers::mime_version::MimeVersionBuilderExt
[multipart::MultipartBuilder]: crate::builder::multipart::MultipartBuilder
[headers::warning::WarningBuilderExt]: crate::builder::headers::warning::WarningBuilderExt
[headers::error_info::ErrorInfoBuilderExt]: crate::builder::headers::error_info::ErrorInfoBuilderExt
[headers::reason::ReasonBuilderExt]: crate::builder::headers::reason::ReasonBuilderExt
[headers::expires::ExpiresBuilderExt]: crate::builder::headers::expires::ExpiresBuilderExt
[headers::organization::OrganizationBuilderExt]: crate::builder::headers::organization::OrganizationBuilderExt
[headers::priority::PriorityBuilderExt]: crate::builder::headers::priority::PriorityBuilderExt
[headers::refer_to::ReferToExt]: crate::builder::headers::refer_to::ReferToExt
[headers::retry_after::RetryAfterBuilderExt]: crate::builder::headers::retry_after::RetryAfterBuilderExt
[headers::rseq::RSeqBuilderExt]: crate::builder::headers::rseq::RSeqBuilderExt
[headers::event::EventBuilderExt]: crate::builder::headers::event::EventBuilderExt
[headers::session_expires::SessionExpiresExt]: crate::builder::headers::session_expires::SessionExpiresExt
[headers::min_se::MinSEBuilderExt]: crate::builder::headers::min_se::MinSEBuilderExt
[headers::referred_by::ReferredByExt]: crate::builder::headers::referred_by::ReferredByExt
[headers::alert_info::AlertInfoBuilderExt]: crate::builder::headers::alert_info::AlertInfoBuilderExt 