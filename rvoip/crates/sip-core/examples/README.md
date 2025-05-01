# SIP Core Tutorial Examples

This directory contains a series of examples that progressively introduce the features and capabilities of the `rvoip-sip-core` library. These examples are designed to be used as a tutorial, starting with the simplest concepts and building up to more complex applications.

## How to Run the Examples

All examples can be run using Cargo:

```bash
# Run a specific example
cargo run --example 01_basic_parsing

# Run with logs enabled
RUST_LOG=debug cargo run --example 01_basic_parsing
```

## Examples Overview

### 1. Basic SIP Message Parsing
**Directory**: `01_basic_parsing/`

Learn how to parse SIP messages and access their components. This example demonstrates:
- Parsing raw SIP messages into structured types
- Accessing headers and message components
- Understanding the basic structure of SIP messages

### 2. Creating SIP Messages
**Directory**: `02_creating_messages/`

Learn how to create SIP requests and responses. This example demonstrates:
- Building SIP messages using the builder pattern
- Creating messages with the concise macro syntax
- Handling different header types
- Proper URI construction

### 3. SIP Dialog Example
**Directory**: `03_sip_dialog/`

Implement a basic call flow with dialog state management. This example demonstrates:
- Complete call flow (INVITE, 200 OK, ACK, BYE)
- Dialog state tracking
- Transaction handling
- End-to-end message flow

### 4. SDP Integration
**Directory**: `04_sdp_integration/`

Learn how to work with Session Description Protocol (SDP) for media negotiation. This example demonstrates:
- Creating and parsing SDP messages
- Integrating SDP bodies with SIP messages
- Basic media negotiation concepts
- Offer/answer model implementation

### 5. Authentication and Security
**Directory**: `05_authentication/`

Handle authentication in SIP communications. This example demonstrates:
- Implementing digest authentication
- Creating and validating authentication headers
- Nonce handling and security best practices
- Registrar authentication flows

### 6. Advanced Routing
**Directory**: `06_advanced_routing/`

Learn SIP routing mechanisms for proxies and servers. This example demonstrates:
- Via header handling for routing
- Implementing basic proxy functionality
- Record-route and route header processing
- Multi-hop communication

### 7. Multipart Message Handling
**Directory**: `07_multipart_messages/`

Work with multipart MIME bodies in SIP messages. This example demonstrates:
- Creating and parsing multipart bodies
- Handling mixed content types
- Real-world use cases for multipart messages
- Content-type header handling

### 8. Complete SIP Client
**Directory**: `08_sip_client/`

Build a functional SIP client application. This example demonstrates:
- Registration with a SIP server
- Making and receiving calls
- Integration with transport layer
- Handling retransmissions and timeouts
- Full application structure

### 9. WebRTC Integration
**Directory**: `09_webrtc_integration/`

Connect SIP signaling with WebRTC media. This example demonstrates:
- Using SDP for WebRTC negotiation
- ICE candidate handling in SDP
- Creating a SIP-to-WebRTC bridge
- Modern communication applications

## Progression Path

The examples are designed to be followed in order, as later examples build upon concepts introduced in earlier ones. However, each example is self-contained and can be run independently.

For developers new to SIP, we recommend starting with Example 1 and working through the series sequentially.

## Additional Resources

- [SIP Core Documentation](../README.md)
- [SIP RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261)
- [SDP RFC 4566](https://datatracker.ietf.org/doc/html/rfc4566) 