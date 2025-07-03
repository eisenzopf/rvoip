# SIP Core Interactive Tutorial

This interactive tutorial series introduces you to building and parsing SIP/SDP messages using the `rvoip-sip-core` library. The tutorials are designed to progressively build your knowledge from basic concepts to advanced real-world scenarios.

## Using the Interactive Tutorial

This tutorial is built with [mdBook](https://rust-lang.github.io/mdBook/), allowing you to:
- Read explanations and code examples
- Run code experiments directly in your browser
- Modify examples to see how they work
- Follow a structured learning path

### Getting Started

```bash
# Install mdBook if you don't have it
cargo install mdbook

# Serve the tutorial locally
cd tutorial
mdbook serve --open
```

## Tutorial Structure

### Part 1: SIP Fundamentals

#### Tutorial 1: Introduction to SIP
- What is SIP and how it works
- SIP message structure
- Understanding SIP URIs
- Basic SIP terminology

#### Tutorial 2: Parsing Your First SIP Message
- Using the parser to decode SIP messages
- Accessing message components
- Understanding headers and body
- Using the `json` module to inspect message structure

#### Tutorial 3: Creating SIP Messages with the Builder Pattern
- Introduction to the `builder` module
- Creating a simple SIP request
- Adding headers and content
- Validating SIP messages

#### Tutorial 4: SIP Requests in Depth
- Common request methods (INVITE, REGISTER, BYE, etc.)
- Method-specific builders
- Required headers for different request types
- Request validation

#### Tutorial 5: SIP Responses in Depth
- Response status codes and their meanings
- Creating responses with the builder
- Matching responses to requests
- Response headers and body

### Part 2: SDP and Media Negotiation

#### Tutorial 6: Introduction to SDP
- SDP structure and purpose
- Media descriptions and attributes
- Connection information
- Time descriptions

#### Tutorial 7: Creating SDP Messages
- Building SDP sessions
- Adding media streams
- Setting codec parameters
- Time and connection information

#### Tutorial 8: Integrating SDP with SIP
- SDP as a SIP message body
- Content-Type headers
- Offer/Answer model basics
- Parsing SDP from SIP messages

#### Tutorial 9: Media Negotiation with SDP
- Implementing the Offer/Answer model
- Codec negotiation
- Media capability advertisement
- Handling multiple media streams

### Part 3: SIP Dialogs and Transactions

#### Tutorial 10: SIP Transactions
- Client and server transactions
- Transaction identifiers
- Transaction state machines
- Handling retransmissions

#### Tutorial 11: SIP Dialogs
- Dialog creation and identification
- Dialog state management
- Route sets and dialog routing
- Dialog termination

#### Tutorial 12: Complete Call Flow
- INVITE-200-ACK flow
- Mid-dialog requests
- Call termination with BYE
- Error handling during calls

### Part 4: Advanced SIP Features

#### Tutorial 13: Authentication
- Digest authentication
- Creating authentication headers
- Handling authentication challenges
- Secure credential management

#### Tutorial 14: SIP Registration
- Client registration process
- Expiration and refreshing
- Multiple contacts
- Registration state management

#### Tutorial 15: SIP Proxying and Routing
- Via headers and routing
- Record-Route and Route headers
- Proxy behavior
- Forwarding requests and responses

#### Tutorial 16: Event Notification Framework
- SUBSCRIBE and NOTIFY methods
- Event packages
- Subscription state management
- Notification bodies

### Part 5: Real-World Applications

#### Tutorial 17: Building a SIP Client
- Client architecture
- Transport layer integration
- User interface considerations
- Registration and call handling

#### Tutorial 18: WebRTC Integration
- SDP for WebRTC
- ICE candidates in SDP
- SIP signaling for WebRTC media
- Browser interoperability

#### Tutorial 19: SIP Troubleshooting
- Common SIP issues and solutions
- Debugging SIP messages
- Tracing SIP flows
- Performance considerations

#### Tutorial 20: Advanced Use Cases
- Multi-device forking
- Conference calling
- Call transfer
- Presence and messaging

## Running Example Code

Each tutorial includes runnable code examples. You can:

```bash
# Run a specific tutorial's example code
cargo run --example tutorial_01_parsing

# Run with logging enabled
RUST_LOG=debug cargo run --example tutorial_10_dialogs
```

## Additional Resources

- [SIP Core API Documentation](https://docs.rs/rvoip-sip-core)
- [SIP RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261)
- [SDP RFC 4566](https://datatracker.ietf.org/doc/html/rfc4566)
- [WebRTC and SIP Integration](https://datatracker.ietf.org/doc/html/rfc7118) 