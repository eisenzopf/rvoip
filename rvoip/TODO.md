# rvoip Architecture TODO

This document outlines architectural recommendations and improvements for the rvoip project, focusing on proper layering and component responsibilities according to SIP RFCs and best practices.

## Layering Architecture

The current layering strategy is sound and follows RFC recommendations, but can be enhanced:

```
+--------------------------+       +--------------------------+
|     sip-client           |       |     call-engine          |
| (Client-side TU Logic)   |       | (Server-side TU Logic)   |
+--------------------------+       +--------------------------+
              |                                |
              v                                v
+--------------------------------------------------+
|                  session-core                    |
|            (Core TU Functionality)               |
+--------------------------------------------------+
        |               |                |
        v               v                v
+-------------+  +-------------+  +-------------+
| transaction |  |    media    |  |     ice     |
|    core     |  |    core     |  |    core     |
+-------------+  +-------------+  +-------------+
        |               |                |
        v               v                v
+-------------+  +-------------+  +-------------+
|  sip-core   |  |  rtp-core   |  |     ...     |
+-------------+  +-------------+  +-------------+
        |               |                |
        v               v                v
+--------------------------------------------------+
|                  sip-transport                   |
+--------------------------------------------------+
```

## General Recommendations

- [ ] Document clear layer boundaries and responsibilities
- [ ] Enforce unidirectional dependencies (lower layers shouldn't depend on higher layers)
- [ ] Create interface diagrams showing the interaction between components
- [ ] Establish consistent error handling patterns across all layers
- [ ] Add metrics/telemetry at key transition points between layers

## Transaction User (TU) Layer

The Transaction User (TU) functionality should be properly distributed:

- [ ] **Session Core (Core TU Functionality)**
  - [ ] Implement dialog management according to RFC 3261 Section 12
  - [ ] Handle core call state transitions
  - [ ] Manage dialog matching and routing
  - [ ] Implement mid-dialog request/response handling
  - [ ] Document APIs for upper layers to extend behavior

- [ ] **Client/Server Split**
  - [ ] Move client-specific TU logic to `sip-client`
  - [ ] Move server-specific TU logic to `call-engine`
  - [ ] Ensure both use common interfaces from `session-core`

## Layer-Specific Improvements

### Transport Layer (`sip-transport`)

- [ ] Ensure proper connection management/recycling
- [ ] Implement robust error handling and recovery
- [ ] Add support for all required transport protocols (UDP, TCP, TLS, WebSocket)
- [ ] Provide clear connection lifecycle events to higher layers

### Message Layer (`sip-core`)

- [ ] Validate message format compliance with RFC 3261
- [ ] Enhance header validation and normalization
- [ ] Add extensive test coverage for edge cases
- [ ] Ensure proper handling of compact header forms

### Transaction Layer (`transaction-core`)

- [ ] Verify all transaction timers match RFC specifications
- [ ] Implement proper transaction matching
- [ ] Ensure transaction termination events are properly propagated
- [ ] Add detailed logging for transaction state transitions

### Session Layer (`session-core`)

- [ ] Implement complete dialog state machines
- [ ] Add support for dialog forking
- [ ] Handle multiple concurrent dialogs properly
- [ ] Implement RFC 6665 event subscription/notification framework

### Media Stack

- [ ] Ensure proper synchronization between SIP signaling and media setup
- [ ] Implement fallback mechanisms for ICE failures
- [ ] Support multiple media types and codec negotiation
- [ ] Add proper SRTP keying and security

## Testing Strategy

- [ ] Create integration tests spanning multiple layers
- [ ] Implement conformance tests against RFC requirements
- [ ] Add interoperability tests with common SIP implementations
- [ ] Create scenario-based tests for common call flows

## Documentation Needs

- [ ] Document layer boundaries and responsibilities
- [ ] Create architectural diagrams
- [ ] Document key extension points for customization
- [ ] Provide usage examples for each layer

## Performance Considerations

- [ ] Benchmark transaction processing capacity
- [ ] Monitor and optimize memory usage, particularly in long-running transactions
- [ ] Ensure proper connection pooling at transport layer
- [ ] Consider scale-out strategies for high volume deployments

---

These recommendations aim to strengthen the current architectural approach while ensuring adherence to SIP standards and scalability requirements. 