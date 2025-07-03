# Parsing Your First SIP Message

In this tutorial, we'll explore how to parse SIP messages using the `rvoip-sip-core` library. We'll focus on different approaches to accessing message components, with special emphasis on the JSON path accessor approach.

## SIP Message Structure Recap

Before diving into parsing, let's recall the basic structure of a SIP message:

1. **Start Line**:
   - For requests: `METHOD URI SIP/VERSION`
   - For responses: `SIP/VERSION STATUS_CODE REASON_PHRASE`

2. **Headers**: Multiple `Name: Value` pairs

3. **Empty Line**: Separates headers from body

4. **Body** (optional): Message content (e.g., SDP for media negotiation)

## Parsing Approaches

The `rvoip-sip-core` library provides several ways to parse and access SIP message components:

1. **JSON Path Accessors**: Flexible string-based access to any part of the message
2. **Native Methods**: Type-safe, direct access to common fields
3. **Header Access Traits**: For working with multiple headers of the same type

Let's explore each approach with examples.

## Parsing a SIP Message

To parse a SIP message, we use the `parse_message` function:

```rust
use rvoip_sip_core::prelude::*;
use bytes::Bytes;

// Raw SIP message as bytes
let data = Bytes::from(message_string);

// Parse the message
match parse_message(&data) {
    Ok(message) => {
        // Work with the parsed message
    },
    Err(e) => {
        println!("Failed to parse message: {:?}", e);
    }
}
```

The `parse_message` function returns a `Result<Message>`, where `Message` is an enum with variants for `Request` and `Response`.

## JSON Path Accessors

The JSON path accessor approach provides a flexible way to access any part of a SIP message using dot notation paths. This is especially useful for quick prototyping or when you need to access deeply nested fields.

To use JSON path accessors, import the `SipJsonExt` trait:

```rust
use rvoip_sip_core::json::SipJsonExt;
```

### Path Accessor Methods

There are two main path accessor methods:

1. **path()**: Returns `Option<SipValue>`, preserving the original type
2. **path_str_or()**: Returns `String`, with a default value if the path is not found

### Example: Accessing Request Fields

```rust
if let Message::Request(request) = message {
    // Basic request information
    println!("Method: {}", request.path_str_or("method", "(unknown)"));
    println!("URI: {}", request.path_str_or("uri", "(unknown)"));
    println!("Version: {}", request.path_str_or("version", "(unknown)"));
    
    // Headers
    println!("From: {}", 
        request.path_str_or("headers.From.display_name", "(unknown)"));
    println!("To: {}", 
        request.path_str_or("headers.To.display_name", "(unknown)"));
    println!("Call-ID: {}", 
        request.path_str_or("headers.CallId", "(none)"));
}
```

### Example: Accessing Response Fields

```rust
if let Message::Response(response) = message {
    // Basic response information
    println!("Status Code: {}", response.path_str_or("status_code", "(unknown)"));
    println!("Reason: {}", response.path_str_or("reason", "(unknown)"));
    
    // Headers (same as for requests)
    println!("From: {}", 
        response.path_str_or("headers.From.display_name", "(unknown)"));
}
```

### Working with Numeric Values

For numeric values, you might want to preserve the type. Here's how to do it with `path()`:

```rust
match request.path("headers.CSeq.seq") {
    Some(val) => {
        if let Some(num) = val.as_i64() {
            println!("CSeq number: {} (numeric value)", num);
            // Now you can use num in arithmetic operations
        }
    },
    None => println!("CSeq number not found"),
}
```

## Native Methods

The library also provides native methods for accessing common fields in a type-safe way:

```rust
// Basic request information
println!("Method: {}", request.method());
println!("URI: {}", request.uri());
println!("Version: {}", request.version());

// Common headers
if let Some(from) = request.from() {
    println!("From: {}", from);
}

if let Some(to) = request.to() {
    println!("To: {}", to);
}

if let Some(via) = request.first_via() {
    println!("Via: {}", via);
}

if let Some(call_id) = request.call_id() {
    println!("Call-ID: {}", call_id);
}

if let Some(cseq) = request.cseq() {
    println!("CSeq: {} {}", cseq.seq, cseq.method);
}
```

For responses, similar methods are available:

```rust
// Basic response information
println!("Status Code: {}", response.status_code());
println!("Reason: {}", response.reason_phrase());
println!("Version: {}", response.version());

// Headers (same as for requests)
```

## Working with Multiple Headers

SIP messages can contain multiple headers of the same type (e.g., multiple Via headers). To access them:

### Using Path Accessors

```rust
// Access first Via header
println!("First Via: {}", 
    request.path_str_or("headers.Via[0].sent_protocol.transport", "(unknown)"));

// Access second Via header
println!("Second Via: {}", 
    request.path_str_or("headers.Via[1].sent_protocol.transport", "(unknown)"));
```

### Using Native Methods

To work with multiple headers, use the `HeaderAccess` trait:

```rust
use rvoip_sip_core::types::headers::HeaderAccess;

// Get all Via headers
let via_headers = request.via_headers();
for (i, via) in via_headers.iter().enumerate() {
    println!("Via #{}: {}", i+1, via);
}

// Get all headers of a specific name
let record_route_headers = request.headers_by_name("Record-Route");
for (i, rr) in record_route_headers.iter().enumerate() {
    println!("Record-Route #{}: {}", i+1, rr);
}
```

## Working with SIP URIs

SIP URIs are a fundamental part of SIP messages. Here's how to parse and work with them:

```rust
use std::str::FromStr;

let uri_str = "sip:user:password@example.com:5060;transport=tcp?subject=Meeting";
match Uri::from_str(uri_str) {
    Ok(uri) => {
        println!("Scheme: {}", uri.scheme);
        println!("User: {}", uri.user.unwrap_or_default());
        
        if let Some(password) = uri.password {
            println!("Password: {}", password);
        }
        
        println!("Host: {}", uri.host);
        
        if let Some(port) = uri.port {
            println!("Port: {}", port);
        }
        
        // URI parameters
        for param in &uri.parameters {
            match param {
                Param::Transport(transport) => println!("Transport: {}", transport),
                Param::Ttl(ttl) => println!("TTL: {}", ttl),
                Param::Other(name, Some(value)) => println!("{}: {}", name, value),
                Param::Other(name, None) => println!("{}", name),
                _ => println!("{:?}", param),
            }
        }
        
        // URI headers
        for (name, value) in &uri.headers {
            println!("{}: {}", name, value);
        }
    },
    Err(e) => {
        println!("Failed to parse URI: {}", e);
    }
}
```

## Working with Message Bodies

SIP messages can contain bodies, such as SDP for media negotiation:

```rust
// Access Content-Type and Content-Length headers
if let Some(header) = request.header(&HeaderName::ContentType) {
    println!("Content-Type: {}", header);
}

if let Some(header) = request.header(&HeaderName::ContentLength) {
    println!("Content-Length: {}", header);
}

// Access the body
let body = request.body();
println!("Body: {} bytes", body.len());

// Convert body to string (if it's text-based)
if let Ok(body_str) = std::str::from_utf8(body) {
    println!("Body (as string):\n{}", body_str);
}
```

## When to Use Each Approach

- **JSON Path Accessors**:
  - For quick prototyping
  - When accessing deeply nested fields
  - When working with custom or non-standard headers
  - When you need string representations

- **Native Methods**:
  - For type safety
  - For common headers and fields
  - When you need the full power of the typed objects

- **Header Access Traits**:
  - When working with multiple headers of the same type
  - For advanced header manipulation

## Conclusion

In this tutorial, we've explored different ways to parse and access SIP message components using the `rvoip-sip-core` library. The JSON path accessor approach provides flexibility, while native methods offer type safety. Choose the approach that best suits your needs.

In the next tutorial, we'll learn how to create SIP messages using the builder pattern.

## Exercise

1. Parse a SIP INVITE request and extract all Via headers.
2. Parse a SIP 200 OK response and extract the To header with tag parameter.
3. Parse a SIP URI with multiple parameters and headers.

## References

- [RFC 3261: SIP: Session Initiation Protocol](https://datatracker.ietf.org/doc/html/rfc3261)
- [RFC 3986: URI Generic Syntax](https://datatracker.ietf.org/doc/html/rfc3986) 