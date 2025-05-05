# Basic SIP Message Parsing Examples

This directory contains a collection of basic examples demonstrating how to parse and work with SIP (Session Initiation Protocol) messages using the RVOIP SIP Core library.

The examples are organized into modular components, each focusing on a specific aspect of SIP message handling:

## Examples

### 1. Parsing a SIP INVITE Request (`01_invite_request.rs`)

This example demonstrates how to:
- Parse a raw SIP INVITE request message
- Access basic request information (method, URI, version)
- Extract and work with typed headers (From, To, Contact)
- Access header parameters (tags, branch identifiers)

### 2. Parsing a SIP Response (`02_sip_response.rs`)

This example shows how to:
- Parse a SIP 200 OK response
- Access response status code and reason phrase
- Determine which request the response is for using CSeq
- Handle multiple Via headers in responses
- Extract tag parameters from headers

### 3. Parsing Messages with Multiple Headers (`03_multiple_headers.rs`)

This example covers:
- Handling messages with multiple instances of the same header (Record-Route)
- Iterating through header collections
- Working with header parameters (loose routing)
- Different ways to access headers (by type or by name)

### 4. Working with SDP Content (`04_sdp_builder.rs`)

This example demonstrates:
- Creating SIP messages with SDP content using the builder pattern
- Building SDP sessions with various attributes
- Generating INVITE requests and responses with SDP bodies
- Parsing SDP content from message bodies

## Running the Examples

You can run these examples individually or collectively using the interactive menu:

```
cargo run --example 01_basic_parsing
```

This will present a menu that allows you to:
1. Run any individual example
2. Run all examples sequentially
3. Exit the program

To run a specific example directly, use:

```
cargo run --example <example_name>
```

For instance:

```
cargo run --example 01_invite_request
```

### Tracing Output

All examples are set up to display informational tracing messages by default. You can adjust the tracing level by setting the `RUST_LOG` environment variable:

```
# Display all debug messages (more verbose)
RUST_LOG=debug cargo run --example 01_invite_request

# Display only warnings and errors
RUST_LOG=warn cargo run --example 01_invite_request
```

Available logging levels from most to least verbose are: `trace`, `debug`, `info`, `warn`, `error`.

## Learning Path

These examples are designed to be followed sequentially, introducing SIP concepts gradually:

1. Start with basic request parsing to understand the fundamentals
2. Move to response parsing to see how SIP transactions work
3. Learn how to handle multiple headers for more complex scenarios
4. Finally, explore SDP integration for media session negotiation

Each example is heavily commented to explain what's happening at each step. 