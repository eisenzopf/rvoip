# rvoip Architecture TODO

This document outlines architectural recommendations and improvements for the rvoip project, focusing on proper layering and component responsibilities according to SIP RFCs and best practices.

## Recently Completed Major Issues (HIGH PRIORITY)

### ✅ **Timeout Error Reduction - COMPLETED** 
**Status**: **100% COMPLETE** - All timeout errors eliminated across the codebase

**Root Cause**: Broadcast channel anti-pattern where `receive_frame()` method was creating new subscribers on each call, causing frame loss and timeout errors.

**Solution**: Implemented persistent frame receiver pattern using `get_frame_receiver()` method for long-lived subscribers.

**Files Fixed**:
- ✅ `examples/api_srtp.rs` - Fixed timeout errors in SRTP example
- ✅ `examples/media_api_usage.rs` - Fixed timeout errors in media API usage
- ✅ `examples/minimal_connection_test.rs` - Preventive fix for consistency
- ✅ `examples/api_ssrc_demultiplexing_basic.rs` - Previously fixed
- ✅ `examples/api_ssrc_demultiplexing_advanced.rs` - Previously fixed

**Testing Results**: All examples now complete successfully with zero timeout errors.

### ✅ **SSRC Demultiplexing Issues - COMPLETED**
**Status**: **100% COMPLETE** - Perfect SSRC separation achieved

**Issues Fixed**:
1. ✅ Server configuration bug (hardcoded `ssrc_demultiplexing_enabled = false`)
2. ✅ Missing SSRC field in `RtpEvent::MediaReceived` events
3. ✅ Broadcast channel timeout issues (covered above)

**Results**: 
- Perfect SSRC separation: "SSRC=1234a001: 1 frames", "SSRC=5678b001: 1 frames"
- Zero timeout errors
- Complete frame processing

### ✅ **RTCP Multiplexing Compilation Fix - COMPLETED**
**Status**: **100% COMPLETE** - rtcp_mux example now working perfectly

**Root Cause**: Missing `ssrc` field in `RtpEvent::MediaReceived` pattern matches after SSRC demultiplexing improvements.

**Solution**: Updated all pattern matches to include the `ssrc` field and enhanced logging.

**Files Fixed**:
- ✅ `examples/rtcp_mux.rs` - Fixed compilation errors and enhanced SSRC logging

**Testing Results**: 
- ✅ RFC 5761 RTCP Multiplexing working perfectly
- ✅ Bidirectional RTP/RTCP communication successful
- ✅ Proper SSRC tracking and display (`SSRC=87654321`)
- ✅ RTCP packet parsing (SenderReport & Goodbye) functional

### ✅ **Payload Parsing Refinement - COMPLETED**
**Status**: **100% COMPLETE** - RFC 3551-compliant payload type registry implemented

**Root Cause**: Hardcoded, duplicated, and incorrect payload type logic scattered across multiple files violating RFC 3551 standards.

**Solution**: Implemented comprehensive payload type registry with RFC 3551 compliance and dynamic payload support.

**Improvements Made**:
- ✅ Created centralized `PayloadTypeRegistry` with RFC 3551 compliance
- ✅ Added support for all standard audio payload types (PCMU, PCMA, G722, G729, etc.)
- ✅ Added support for all standard video payload types (H261, H263, JPEG, etc.)  
- ✅ Implemented dynamic payload type support (96-127) for H264, VP8, VP9, Opus
- ✅ Replaced hardcoded logic in `default.rs` (3 instances) and `connection.rs` (2 instances)
- ✅ Added proper fallback behavior for unregistered payload types
- ✅ Created comprehensive test suite and demo example

**Files Enhanced**:
- ✅ `src/payload/registry.rs` - New comprehensive payload type registry
- ✅ `src/api/server/transport/default.rs` - Replaced hardcoded logic with registry calls
- ✅ `src/api/server/transport/core/connection.rs` - Replaced hardcoded logic with registry calls
- ✅ `examples/payload_type_demo.rs` - New demo showcasing enhanced capabilities

**Testing Results**: 
- ✅ All examples work perfectly with enhanced payload type handling
- ✅ RFC 3551 compliance verified for all standard payload types
- ✅ Dynamic payload types (H264, VP8, VP9, Opus) properly supported
- ✅ Proper media frame type detection (Audio/Video/Data)
- ✅ Zero compilation errors or runtime issues

## Current Next Priorities (MEDIUM PRIORITY)

### ✅ **Duplicate Example Consolidation - COMPLETED**
**Status**: **100% COMPLETE** - Redundant example removed

**Issue**: `api_ssrc_demux.rs` and `api_ssrc_demultiplexing.rs` were 99% identical (only 2-line diff in security config style)

**Action Taken**:
- ✅ Removed `api_ssrc_demux.rs` (duplicate)
- ✅ Kept `api_ssrc_demultiplexing.rs` (more descriptive name)
- ✅ Verified both examples worked identically before removal

**Result**: Cleaner example codebase with no redundant functionality

### **🎯 NEXT: Example Documentation & Cleanup - READY**
**Status**: **CURRENT PRIORITY** - Core functionality stable, ready for enhancement

**Focus Areas**:
- [ ] Create comprehensive README.md for examples directory
- [ ] Add clear comments explaining each example's purpose and use case
- [ ] Standardize example structure and patterns across all examples
- [ ] Ensure consistent error handling and logging patterns
- [ ] Add usage instructions and expected outputs for each example
- [ ] Group examples by functionality (basic, security, advanced features)
- [ ] Add troubleshooting guide for common example issues

### **3. Advanced Feature Development - READY**
**Status**: **FUTURE PRIORITY** - Ready for new feature development

**Tasks**:
- [ ] Implement additional RTP header extensions
- [ ] Add support for RTP multiplexing
- [ ] Enhance RTCP feedback mechanisms
- [ ] Implement adaptive bitrate control
- [ ] Add support for simulcast and SVC

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

## Transaction Core Major Issues

- [x] Fix trait object safety issue: async methods in Transaction trait (original_request, last_response, send_command) can't be used in trait objects
- [ ] Transaction structs and TransactionData field mismatches (timer_manager, cmd_rx fields)
- [ ] TransactionEvent enum variant mismatches (Response, Timeout, Terminated)
- [x] Implement proper TypedHeader access for Request/Response methods (via, header, etc.)
- [x] Fix TransactionKey::new implementation to match the expected parameters
- [x] Address error propagation issues in client.rs handle_transport_message function
- [ ] Fix the AtomicTransactionState usage in ClientNonInviteTransaction
- [ ] Fix RequestBuilder.build() handling - it should return a Result<Request, Error>

## Transaction Core Improvements

- [x] Create comprehensive documentation in README.md explaining architecture and usage
- [x] Implement RFC 3261 compliant timer management system
- [x] Add proper support for both Send and Sync in Transaction trait
- [x] Migrate from std::sync::Mutex to tokio::sync::Mutex for better async support
- [x] Fix ClientNonInviteTransaction implementation
- [x] Add utils.rs with create_ack_from_invite helper function
- [x] Fix Error enum to use struct variants consistently
- [x] Update Transaction trait interface with async original_request and last_response methods
- [x] Fix transaction references to avoid borrowing issues with boxed trait objects
- [x] Fix TransportEvent handling to match the current API
- [ ] Redesign the trait hierarchy to avoid async methods in trait objects
- [ ] Add proper client transaction test for full transaction lifecycle
- [ ] Add proper server transaction test for full transaction lifecycle
- [ ] Fix bug with ACK handling in InviteServerTransaction after 2xx response
- [ ] Improve transaction reference handling in manager.rs (use Arc<RwLock> for transaction storage)
- [ ] Add metrics and telemetry for monitoring transaction states
- [ ] Add support for transaction termination and cleanup in the manager

## Transaction Core Missing Features

- [ ] Implement CANCEL method support with proper handling and matching
- [ ] Add support for reliability extensions (RFC 3262/PRACK)
- [ ] Implement forking support for handling multiple responses
- [ ] Improve transport failure handling and recovery
- [ ] Add dialog integration points for transaction layer
- [ ] Implement UPDATE method support (RFC 3311)
- [ ] Add error recovery and resilience mechanisms
- [ ] Provide operational metrics for transaction states
- [ ] Fix server transaction creation issues evident in integration tests
- [ ] Add performance benchmarks and optimizations

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
- [ ] Create visual state machine diagrams for all transaction types
- [ ] Document transaction timer behavior and configuration options
- [ ] Add examples of common transaction scenarios and patterns
- [ ] Create troubleshooting guides for transaction-related issues
- [ ] Document transaction manager's API contract

## Performance Considerations

- [ ] Benchmark transaction processing capacity
- [ ] Monitor and optimize memory usage, particularly in long-running transactions
- [ ] Ensure proper connection pooling at transport layer
- [ ] Consider scale-out strategies for high volume deployments
- [ ] Analyze and optimize transaction timer overhead
- [ ] Measure and reduce lock contention in transaction hot paths
- [ ] Implement efficient transaction lookup with optimized data structures
- [ ] Consider sharded transaction storage for better parallelism
- [ ] Add performance testing framework with reproducible load tests
- [ ] Implement load shedding mechanisms for overload protection

## General Architecture

- [ ] Define clear module boundaries and public interfaces (API separation)
- [ ] Create diagrams for key data flow paths
- [ ] Document communication patterns between components
- [ ] Research WebRTC integration options
- [ ] Design persistent storage for call history, registration status
- [ ] Identify performance bottlenecks under high volume
- [ ] Implement consistent logging strategy across crates
- [ ] Add metrics collection and reporting
- [ ] Implement graceful shutdown throughout the stack
- [ ] Add thorough error handling with context
- [ ] Standardize configuration approach
- [ ] Add comprehensive integration tests

## SIP Core 

- [ ] Split parser into smaller, more focused components
- [ ] Benchmark and optimize header parsing
- [ ] Implement proper Via header handling
- [ ] Add support for additional extensions (Replaces, etc.)
- [ ] Create connection-oriented transport abstractions
- [ ] Optimize memory usage for message parsing/serialization
- [ ] Add validation for header values

## Transport Layer

- [ ] Implement connection pooling for TCP
- [ ] Add TLS support with proper certificate handling
- [x] Create WebSocket transport for WebRTC signaling
- [ ] Implement proper DNS SRV resolution
- [ ] Create NAT traversal strategy (using STUN/ICE)
- [ ] Add IPv6 support
- [ ] Implement keep-alive mechanisms for persistent connections
- [x] Successfully integrate sip-transport with transaction-core

## Dialog Layer
- [ ] Design core dialog state management
- [ ] Implement dialog creation, modification, termination
- [ ] Create dialog matching for in-dialog requests
- [ ] Design proper Route/Record-Route handling
- [ ] Implement target refresh handling

## Control Layer / User Agent
- [ ] Define API for application integration
- [ ] Implement registration handling
- [ ] Create call control interface
- [ ] Add proxy authentication support
- [ ] Implement re-INVITE support for media changes
- [ ] Create subscription/notification framework

---

These recommendations aim to strengthen the current architectural approach while ensuring adherence to SIP standards and scalability requirements. 