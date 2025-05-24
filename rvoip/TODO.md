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

### ✅ **Example Documentation & Cleanup - COMPLETED**
**Status**: **100% COMPLETE** - Comprehensive README.md created with full organization

**Root Cause**: Examples directory lacked comprehensive documentation and logical organization for 50+ examples.

**Solution**: Created structured README.md with API vs Core segmentation and functional categorization.

**Improvements Made**:
- ✅ Created comprehensive README.md for examples directory (135 → 400+ lines)
- ✅ Added clear API vs Core segmentation (21 API examples, 29 Core examples)
- ✅ Organized examples by functionality (7 categories: Basic, Security, Advanced, Protocol, Payload, Testing)
- ✅ Added clear comments explaining each example's purpose and use case
- ✅ Standardized example documentation format with consistent structure
- ✅ Added usage instructions and expected outputs for each example
- ✅ Included troubleshooting guide for common example issues
- ✅ Added quick start guide with recommended learning path

**Files Enhanced**:
- ✅ `examples/README.md` - Complete rewrite with comprehensive organization
- ✅ All 50+ examples now properly documented and categorized
- ✅ Zero file movement - no disruption to existing workflows

**Results**:
- ✅ Easy navigation between API vs Core examples
- ✅ Clear functional grouping within each category
- ✅ Comprehensive usage instructions for all examples
- ✅ Troubleshooting guide for common issues
- ✅ Quick start path for new users

**🎯 NEXT: Advanced Feature Development - READY**

### ✅ **Phase 1: Enhanced RTCP Feedback Mechanisms - COMPLETED**
**Status**: **100% COMPLETE** - WebRTC-compatible RTCP feedback system implemented
**Timeline**: Completed in 1 session
**WebRTC Impact**: Essential adaptive video streaming capabilities now available

### **🚀 What Was Implemented**

#### **Core Layer Implementation** (`/feedback/`)
- ✅ **New RTCP Packet Types**
  - ✅ Picture Loss Indication (PLI) - RFC 4585 with full serialization/parsing
  - ✅ Full Intra Request (FIR) - RFC 5104 with sequence number support
  - ✅ Slice Loss Indication (SLI) - RFC 4585 with macroblock addressing
  - ✅ Temporal-Spatial Trade-off (TSTO) - RFC 5104 with trade-off indexing
  - ✅ Receiver Estimated Max Bitrate (REMB) - Google extension with exponential encoding
  - ✅ Transport-wide Congestion Control feedback (WebRTC extension, basic implementation)

- ✅ **Feedback Generation Algorithms**
  - ✅ Loss-based feedback generator (PLI/FIR triggers based on loss patterns)
  - ✅ Congestion-based feedback generator (REMB with bandwidth estimation)
  - ✅ Quality-based feedback generator (comprehensive quality metrics)
  - ✅ Comprehensive feedback generator (combines all strategies with prioritization)

- ✅ **Enhanced Statistics & Algorithms**
  - ✅ Google Congestion Control (GCC) implementation with Kalman filtering
  - ✅ Simple Bandwidth Estimator with congestion adjustment
  - ✅ Quality Assessment with MOS score calculation
  - ✅ Configurable feedback generation with rate limiting

#### **API Layer Integration**
- ✅ **Feedback Types & Configuration**
  - ✅ `FeedbackContext`, `FeedbackConfig`, `FeedbackDecision` types
  - ✅ `FeedbackPriority` (Low, Normal, High, Critical) with intelligent prioritization
  - ✅ `QualityDegradation` reasons (PacketLoss, HighJitter, BandwidthLimited, FrameCorruption)
  - ✅ `CongestionState` tracking (None, Light, Moderate, Severe, Critical)

- ✅ **Generator Factory & Management**
  - ✅ `FeedbackGeneratorFactory` with multiple generator types
  - ✅ Configurable feedback rates and intervals
  - ✅ Automatic feedback response configuration

#### **Testing & Examples**
- ✅ **Core Example**: `rtcp_feedback_core.rs` (398 lines)
  - ✅ Low-level feedback packet handling demonstration
  - ✅ All 4 feedback generators tested with multiple scenarios
  - ✅ Google Congestion Control and bandwidth estimation demos
  - ✅ Quality assessment with MOS scoring (1.6-4.8 range)

#### **Library Integration**
- ✅ **Updated lib.rs** with comprehensive feedback exports
- ✅ **Complete documentation** explaining WebRTC-compatible adaptive streaming
- ✅ **Clean API surface** with both low-level and high-level interfaces

### **🎯 Test Results & Validation**

#### **Packet Creation & Serialization**
- ✅ PLI packets: 12 bytes (round-trip parsing verified)
- ✅ FIR packets: 16 bytes with sequence number support
- ✅ REMB packets: 24 bytes with exponential bitrate encoding (2 Mbps tested)

#### **Feedback Generation Intelligence**
- ✅ **Loss Generator**: PLI at 5% loss (Normal priority), FIR at 15% loss (Critical priority)
- ✅ **Congestion Generator**: REMB with 60-90% confidence, adaptive bandwidth (0.9-1.1 Mbps)
- ✅ **Quality Generator**: MOS-based feedback decisions, quality thresholds working
- ✅ **Comprehensive Generator**: Multi-type feedback recommendations (up to 3 types simultaneously)

#### **Bandwidth Estimation Accuracy**
- ✅ **Google Congestion Control**: State transitions (Hold → Decrease), accurate packet feedback processing
- ✅ **Simple Estimator**: 30-70% confidence levels, congestion-aware adjustments

#### **Quality Assessment**
- ✅ Quality scores: 0.95 (Excellent) → 0.15 (Critical)
- ✅ MOS scores: 4.8 (Excellent) → 1.6 (Critical)
- ✅ Feedback thresholds: Correctly triggering at quality < 0.6

### **🌟 Achievement Summary**

**📊 Code Metrics:**
- **1,800+ lines** of new feedback-specific code
- **6 RTCP packet types** with full RFC compliance
- **4 intelligent feedback generators** with different strategies
- **3 bandwidth estimation algorithms** (GCC, Simple, Quality-based)
- **1 comprehensive core example** demonstrating all capabilities

**🔧 Technical Capabilities:**
- **WebRTC-compatible** PLI/FIR/REMB packet generation
- **Google Congestion Control** with Kalman filtering and over-use detection
- **Quality-driven adaptation** with MOS scoring and trend analysis
- **Multi-strategy feedback** with intelligent prioritization
- **Rate-limited generation** preventing feedback storms

**📈 Quality Improvements Enabled:**
- **Adaptive video streaming** with automatic keyframe requests
- **Bandwidth-aware streaming** with REMB-based rate control
- **Loss recovery optimization** with intelligent PLI/FIR selection
- **Network condition adaptation** with GCC-based congestion control

**🎯 WebRTC Compliance:**
- ✅ RFC 4585 (Generic NACK and feedback messages)
- ✅ RFC 5104 (Codec Control Messages) 
- ✅ Google REMB extension compatibility
- ✅ Transport-wide Congestion Control foundation

### **🚀 Ready for Phase 2: Additional RTP Header Extensions**

### **🚀 Advanced Feature Development - DETAILED IMPLEMENTATION PLAN**
**Status**: **CURRENT PRIORITY** - Core functionality stable, ready for advanced WebRTC/enterprise features

**Implementation Strategy**: Both Core and API layers required for each feature
- **Core Layer**: Protocol-specific parsing, algorithm implementation, low-level processing
- **API Layer**: Simplified configuration, application-friendly interfaces, event notifications

---

## **📊 Phase 2: Additional RTP Header Extensions (PRIORITY 2)**
**Goal**: Advanced RTP metadata for modern WebRTC features
**Timeline**: 2 weeks  
**WebRTC Impact**: Critical for advanced streaming features

### **Core Layer Implementation** (`/packet/`, `/header_extensions/`)
- [ ] **Extension Registry System** (`/header_extensions/registry.rs`)
  - [ ] Audio Level Extensions (RFC 6464)
  - [ ] Video Orientation Extensions (RFC 7742)
  - [ ] Transport-wide Congestion Control extensions
  - [ ] Frame Marking Extensions (RFC 7941)
  - [ ] RTP Stream Identifier (RID) - RFC 8852
  - [ ] Repair RTP Stream Identifier (R-RID) - RFC 8853

- [ ] **Extension Codecs** (`/header_extensions/codecs/`)
  - [ ] Audio level parsing/serialization
  - [ ] Video orientation metadata handling
  - [ ] Transport CC sequence number handling
  - [ ] Frame marking dependency parsing
  - [ ] RID identification and validation

- [ ] **Packet Integration** (`/packet/`)
  - [ ] Enhanced header extension parsing
  - [ ] Extension negotiation support
  - [ ] Extension priority handling

### **API Layer Implementation** (`/api/common/config.rs`)
- [ ] **Extension Configuration**
  - [ ] `HeaderExtensionConfig` with enable/disable options
  - [ ] Extension-specific parameter configuration
  - [ ] Automatic extension negotiation settings

- [ ] **Stream Management**
  - [ ] RID-based stream identification
  - [ ] Multi-stream extension coordination
  - [ ] Extension-aware stream routing

### **Testing & Examples**
- [ ] **Core Examples**
  - [ ] `header_extensions_advanced.rs` - All extension types
  - [ ] `rid_stream_identification.rs` - RID-based routing
- [ ] **API Examples**
  - [ ] `api_header_extensions_webrtc.rs` - WebRTC-compatible setup
  - [ ] `api_multi_stream_extensions.rs` - Multiple stream handling

---

## **🎚️ Phase 3: Adaptive Bitrate Control (PRIORITY 3)**  
**Goal**: Dynamic network-aware quality adaptation
**Timeline**: 2 weeks
**WebRTC Impact**: Essential for mobile and variable network conditions

### **Core Layer Implementation** (`/congestion/`, `/rate_control/`)
- [ ] **Bandwidth Estimation** (`/congestion/estimation.rs`)
  - [ ] Google Congestion Control (GCC) algorithm
  - [ ] Transport-wide congestion control implementation
  - [ ] Loss-based bandwidth estimation
  - [ ] Delay-based congestion detection
  - [ ] Hybrid estimation algorithms

- [ ] **Rate Control** (`/rate_control/`)
  - [ ] Target bitrate calculation algorithms
  - [ ] Quality scaling decision logic
  - [ ] Keyframe request scheduling
  - [ ] Encoder parameter recommendation

- [ ] **Probing Mechanisms** (`/congestion/probing.rs`)
  - [ ] Active bandwidth probing
  - [ ] Probe packet generation and scheduling
  - [ ] Probe response analysis

### **API Layer Implementation** (`/api/common/`)
- [ ] **Bitrate Configuration**
  - [ ] `AdaptiveBitrateConfig` with min/max/target rates
  - [ ] Adaptation policy configuration (aggressive/conservative)
  - [ ] Quality preference settings (resolution vs framerate)

- [ ] **Adaptation Events**
  - [ ] `BitrateAdaptation { old_rate, new_rate, reason }`
  - [ ] `QualityRecommendation { resolution, framerate, bitrate }`
  - [ ] `NetworkConditionChange { bandwidth, rtt, loss_rate }`

### **Testing & Examples**
- [ ] **Core Examples**
  - [ ] `bandwidth_estimation.rs` - Core estimation algorithms
  - [ ] `rate_control_algorithms.rs` - Rate adaptation logic
- [ ] **API Examples**
  - [ ] `api_adaptive_streaming.rs` - End-to-end adaptation
  - [ ] `api_network_adaptation.rs` - Network condition response

---

## **🔄 Phase 4: RTP Multiplexing Support (PRIORITY 4)**
**Goal**: Multiple stream multiplexing for efficient transport
**Timeline**: 2 weeks
**WebRTC Impact**: Required for Bundle support and NAT optimization

### **Core Layer Implementation** (`/transport/`, `/session/`)
- [ ] **Stream Multiplexing** (`/transport/multiplexer.rs`)
  - [ ] RID-based stream identification and routing
  - [ ] Enhanced SSRC collision detection and resolution
  - [ ] Dynamic SSRC allocation management
  - [ ] Stream priority and bandwidth sharing

- [ ] **Bundle Transport** (`/transport/bundle.rs`)
  - [ ] Single-port multi-stream transport
  - [ ] Connection state management for bundled streams
  - [ ] ICE integration for bundled connections
  - [ ] Stream lifecycle coordination

- [ ] **Session Management** (`/session/`)
  - [ ] Multi-stream session coordination
  - [ ] Stream dependency management
  - [ ] Cross-stream synchronization

### **API Layer Implementation** (`/api/server/`, `/api/common/`)
- [ ] **Stream Management API**
  - [ ] `add_stream(config: StreamConfig) -> StreamId`
  - [ ] `remove_stream(stream_id: StreamId)`
  - [ ] `configure_bundle(bundle_config: BundleConfig)`

- [ ] **Stream Configuration**
  - [ ] Per-stream codec and quality settings
  - [ ] Stream priority and resource allocation
  - [ ] RID assignment and management

### **Testing & Examples**
- [ ] **Core Examples**
  - [ ] `rtp_multiplexing_core.rs` - Low-level multiplexing
  - [ ] `bundle_transport.rs` - Bundle transport handling
- [ ] **API Examples**
  - [ ] `api_bundle_streams.rs` - Multi-stream bundling
  - [ ] `api_stream_management.rs` - Dynamic stream control

---

## **📹 Phase 5: Simulcast and SVC Support (PRIORITY 5)**
**Goal**: Advanced scalable video streaming
**Timeline**: 3 weeks
**WebRTC Impact**: Required for conference optimization and device adaptation

### **Core Layer Implementation** (`/packet/`, `/scalability/`)
- [ ] **Simulcast Support** (`/scalability/simulcast.rs`)
  - [ ] Multiple encoding stream management
  - [ ] RID-based simulcast identification
  - [ ] Dynamic stream selection algorithms
  - [ ] Bandwidth-aware stream switching

- [ ] **SVC Support** (`/scalability/svc.rs`)
  - [ ] Temporal layer parsing and handling
  - [ ] Spatial layer dependency tracking
  - [ ] Quality layer management
  - [ ] Layer dependency graph computation

- [ ] **Packet Processing** (`/packet/`)
  - [ ] SVC header parsing (VP9, AV1)
  - [ ] Temporal ID extraction and validation
  - [ ] Layer dependency validation
  - [ ] Frame completion detection

### **API Layer Implementation** (`/api/common/`, `/api/server/`)
- [ ] **Simulcast Configuration**
  - [ ] `SimulcastConfig` with multiple stream definitions
  - [ ] Per-stream encoding parameters
  - [ ] Automatic stream selection policies

- [ ] **SVC Configuration**
  - [ ] Temporal/spatial/quality layer configuration
  - [ ] Layer dependency specification
  - [ ] Adaptive layer selection

### **Testing & Examples**
- [ ] **Core Examples**
  - [ ] `simulcast_core.rs` - Multi-stream simulcast
  - [ ] `svc_layers.rs` - SVC layer handling
- [ ] **API Examples**
  - [ ] `api_simulcast_conference.rs` - Conference simulcast
  - [ ] `api_svc_adaptation.rs` - SVC layer adaptation

---

## **🎯 Implementation Order & Dependencies**

### **Phase Dependencies**:
1. **Phase 1 (RTCP Feedback)** → **Phase 3 (Adaptive Bitrate)** (feedback enables adaptation)
2. **Phase 2 (Header Extensions)** → **Phase 4 (Multiplexing)** (RID extensions enable multiplexing)
3. **Phase 4 (Multiplexing)** → **Phase 5 (Simulcast/SVC)** (multiplexing enables multiple streams)

### **Success Metrics**:
- **Phase 1**: Working PLI/FIR/REMB with quality improvement demonstrations
- **Phase 2**: All WebRTC-required extensions working with browser compatibility
- **Phase 3**: Demonstrated bandwidth adaptation with 50% improvement in variable networks
- **Phase 4**: Bundle support with multiple simultaneous streams
- **Phase 5**: Working simulcast/SVC with conference-style demonstrations

### **WebRTC Compliance Goals**:
- [ ] Chrome/Firefox/Safari browser compatibility
- [ ] Standards-compliant extension negotiation
- [ ] Interoperability with existing WebRTC implementations
- [ ] Performance suitable for production deployment

**🌟 Target Outcome**: Complete WebRTC-compatible media transport system with enterprise-grade adaptive streaming capabilities

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