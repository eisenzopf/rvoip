# SIP Parser Demo

This example demonstrates how to use the SIP parser from the `rvoip-sip-core` library to parse and work with SIP messages.

## Features Demonstrated

1. Parsing SIP requests (INVITE, REGISTER, etc.)
2. Parsing SIP responses (200 OK, 180 Ringing, etc.)
3. Parsing complete SIP messages (automatically detecting if they're requests or responses)
4. Error handling for malformed messages
5. Working with header fields (raw, typed, and convenience methods)

## Running the Example

From the project root:

```bash
cargo run --example sip-parser-demo
```

Or from within this directory:

```bash
cargo run
```

## Example Code Structure

The code is organized into several functions that demonstrate different aspects of the SIP parser:

- `parse_sip_request()`: Shows how to parse a SIP request and access its components
- `parse_sip_response()`: Shows how to parse a SIP response and access its components
- `parse_full_message()`: Demonstrates the generic parser that can handle either requests or responses
- `handle_parsing_errors()`: Shows how the parser handles various error conditions
- `work_with_headers()`: Demonstrates different ways to access and work with SIP headers

## Key APIs Used

- `parse_request()`: Parses a SIP request message
- `parse_response()`: Parses a SIP response message
- `parse_message()`: Parses either a request or response message
- Header access methods: `message.get_header()`, `message.from()`, `message.to()`, etc.
- Typed headers via the `TypedHeader` trait

## Related Documentation

For more information on the SIP protocol:
- [RFC 3261](https://www.rfc-editor.org/rfc/rfc3261.html) - SIP: Session Initiation Protocol 