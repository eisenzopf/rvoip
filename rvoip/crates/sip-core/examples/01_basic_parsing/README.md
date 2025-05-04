# Example 1: Basic SIP Message Parsing

This example demonstrates how to parse raw SIP messages into structured types, how to access the various components of a SIP message, and how to create SIP messages with SDP content using the builder pattern.

## What You'll Learn

- How to parse raw SIP messages using the `parse_message` function
- How to determine if a message is a request or response
- How to access typed headers using the `typed_header` and `typed_headers` methods
- How to extract parameters from headers (e.g., tags, branch parameters)
- How to handle messages with multiple headers of the same type
- How to create SIP messages with SDP content using the builder pattern
- How to build and parse SDP sessions

## Running the Example

```bash
# Run the example
cargo run --example 01_basic_parsing

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 01_basic_parsing
```

## Code Walkthrough

The example is divided into four parts:

1. **Parsing a SIP INVITE Request**
   - Demonstrates parsing a basic SIP INVITE request
   - Shows how to access common headers like From, To, Contact, and Via
   - Extracts the tag from the From header and the branch parameter from Via

2. **Parsing a SIP Response**
   - Shows how to parse a 200 OK response
   - Demonstrates accessing status code and reason phrase
   - Handles multiple Via headers which are common in responses
   - Checks for the To tag which is important for dialog establishment

3. **Handling Multiple Headers**
   - Shows how to work with multiple headers of the same type (Record-Route)
   - Demonstrates iterating through headers and checking for parameters
   - Shows alternative ways to access headers (by name vs. by type)

4. **Creating SIP Messages with SDP Content**
   - Demonstrates how to use the builder pattern to create SIP messages
   - Shows how to build SDP sessions with the SdpBuilder
   - Illustrates how to attach SDP content to SIP messages
   - Shows how to parse SDP from a SIP message body

## Key Concepts

### Message Types

SIP defines two main message types:
- **Requests**: Used to initiate actions (INVITE, BYE, ACK, etc.)
- **Responses**: Used to reply to requests (200 OK, 404 Not Found, etc.)

### Important Headers

- **From**: Identifies the initiator of the request
- **To**: Identifies the intended recipient of the request
- **Via**: Shows the path taken by the request so far
- **CSeq**: Contains a sequence number and method name for request matching
- **Call-ID**: Unique identifier for the call
- **Contact**: Contains a URI for direct communication
- **Record-Route**: Used by proxies to stay in the signaling path
- **Content-Type**: Specifies the type of the message body (e.g., application/sdp)

### SDP Integration

SDP (Session Description Protocol) is commonly used with SIP for describing media sessions. Key concepts include:

- **SdpBuilder**: Fluent API for creating SDP sessions
- **SDP Session Structure**: v=, o=, s=, c=, t=, m= lines and their meanings
- **Media Descriptions**: Define the type, port, protocol, and formats for each media stream
- **Media Attributes**: Additional information about media (rtpmap, fmtp, direction, etc.)
- **Content Integration**: How to attach SDP to SIP messages using Content-Type headers

### Next Steps

Once you're comfortable with parsing SIP messages and creating them with SDP content, you can move on to more advanced examples that demonstrate dialog management, transactions, and full call flows. 