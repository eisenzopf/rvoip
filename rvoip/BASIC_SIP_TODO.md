# Basic SIP Server & Client TODO

**Goal**: Complete a functional basic SIP server and client implementation
**Scope**: Essential SIP functionality without advanced WebRTC features
**Target**: Production-ready basic SIP proxy, registrar, and UA functionality

---

## **🎯 Current Status Overview**

### **✅ COMPLETE - Ready for Basic SIP**
- ✅ **SIP Protocol Stack** (`sip-core`) - Message parsing, headers, URI handling
- ✅ **Transaction Layer** (`transaction-core`) - RFC 3261 client/server transactions  
- ✅ **Transport Layer** (`sip-transport`) - UDP, TCP, TLS, WebSocket
- ✅ **Basic RTP/RTCP** (`rtp-core`) - Packet handling, SSRC demux, basic feedback
- ✅ **Basic Security** - SRTP foundations, digest authentication ready

### **🔶 PARTIAL - Needs Completion for Basic SIP**
- 🔶 **Dialog Management** (`session-core`) - 70% complete, needs dialog state machines
- 🔶 **Call Routing** (`call-engine`) - 60% complete, needs basic server functionality  
- 🔶 **Media Relay** (`media-core`) - 50% complete, needs simple forwarding

### **❌ NOT NEEDED for Basic SIP**
- ❌ Advanced RTCP feedback (Phase 1 - already implemented but beyond basic needs)
- ❌ RTP Header Extensions (Phase 2)
- ❌ Adaptive Bitrate Control (Phase 3)  
- ❌ RTP Multiplexing (Phase 4)
- ❌ Simulcast/SVC (Phase 5)

---

## **📋 Priority 1: Complete Dialog Management (`session-core`)**

### **Critical Dialog Functionality**
- [ ] **Complete Dialog State Machine** (`/dialog/state_machine.rs`)
  - [ ] Implement `DialogState` enum (Early, Confirmed, Terminated)
  - [ ] Add state transition validation (Early → Confirmed → Terminated)
  - [ ] Handle dialog creation from INVITE transactions
  - [ ] Implement dialog termination on BYE/error

- [ ] **Dialog Matching & Routing** (`/dialog/matching.rs`)
  - [ ] Implement RFC 3261 Section 12.1.1 dialog identification
  - [ ] Match in-dialog requests using Call-ID, From/To tags
  - [ ] Route mid-dialog requests to correct dialog instance
  - [ ] Handle dialog forking scenarios

- [ ] **SDP Offer/Answer** (`/sdp/negotiation.rs`)
  - [ ] Basic SDP parsing and generation
  - [ ] Implement offer/answer model (RFC 3264)
  - [ ] Handle media format negotiation (audio: G.711, G.722)
  - [ ] Coordinate with media-core for codec selection

- [ ] **Dialog API Integration** (`/api/dialog_manager.rs`)
  - [ ] Create `DialogManager` for session lifecycle
  - [ ] Integrate with transaction-core for request/response handling
  - [ ] Expose dialog events to call-engine
  - [ ] Add dialog cleanup and resource management

### **Testing & Examples**
- [ ] **Dialog Tests**
  - [ ] Basic call flow test (INVITE → 180 → 200 → ACK → BYE)
  - [ ] Dialog forking scenarios
  - [ ] Mid-dialog request handling (re-INVITE, UPDATE)
- [ ] **Examples**
  - [ ] `basic_dialog_client.rs` - Simple UA dialog handling
  - [ ] `dialog_state_demo.rs` - State machine demonstration

---

## **📋 Priority 2: Basic Call Engine (`call-engine`)**

### **Registration Server**
- [ ] **User Registration** (`/registration/registrar.rs`)
  - [ ] Handle REGISTER requests with proper authentication
  - [ ] Maintain registration database (in-memory for basic version)
  - [ ] Implement registration expiration and refresh
  - [ ] Support contact header processing and binding updates

- [ ] **Authentication** (`/auth/digest.rs`)
  - [ ] Implement HTTP Digest authentication (RFC 2617)
  - [ ] Generate and validate authentication challenges
  - [ ] Handle authentication failures and retry logic
  - [ ] Basic user credential storage interface

### **Call Routing & Proxy**
- [ ] **Basic Proxy Functionality** (`/proxy/basic_proxy.rs`)
  - [ ] Route INVITE requests to registered users
  - [ ] Handle proxy responses (100, 180, 200, 4xx, 5xx)
  - [ ] Implement basic Record-Route and Via header handling
  - [ ] Support call forwarding and redirection

- [ ] **Call State Management** (`/call/call_manager.rs`)
  - [ ] Track active calls and dialog associations
  - [ ] Handle call setup, progress, and termination events
  - [ ] Coordinate between SIP signaling and media sessions
  - [ ] Implement basic call transfer (REFER handling)

### **SIP Server Core**
- [ ] **Server Configuration** (`/config/server_config.rs`)
  - [ ] Define server listening addresses and ports
  - [ ] Configure authentication realms and user management
  - [ ] Set routing policies and proxy behavior
  - [ ] Add logging and monitoring configuration

- [ ] **Request Router** (`/routing/request_router.rs`)
  - [ ] Implement domain-based routing logic
  - [ ] Handle local vs. proxy routing decisions
  - [ ] Support multiple SIP domains and virtual hosting
  - [ ] Add load balancing for outbound routing

### **Testing & Examples**
- [ ] **Integration Tests**
  - [ ] Registration flow test (REGISTER → 200 OK)
  - [ ] Basic call test (Alice calls Bob through proxy)
  - [ ] Authentication challenge test
- [ ] **Examples**
  - [ ] `basic_sip_server.rs` - Complete SIP server
  - [ ] `sip_registrar_demo.rs` - Registration server
  - [ ] `sip_proxy_demo.rs` - Call routing proxy

---

## **📋 Priority 3: Simple Media Relay (`media-core`)**

### **Basic Media Pipeline**
- [ ] **RTP Relay** (`/relay/rtp_relay.rs`)
  - [ ] Simple RTP packet forwarding between endpoints
  - [ ] Handle SSRC rewriting for call routing
  - [ ] Basic jitter buffer for packet ordering
  - [ ] Support bidirectional media flow

- [ ] **Codec Negotiation** (`/codecs/negotiation.rs`)
  - [ ] Support basic audio codecs (G.711 μ-law/A-law, G.722)
  - [ ] Handle codec parameter negotiation
  - [ ] Basic transcoding capabilities (if different codecs)
  - [ ] Coordinate with session-core SDP negotiation

### **Media Session Management**
- [ ] **Session Coordination** (`/session/media_session.rs`)
  - [ ] Link SIP dialogs with media sessions
  - [ ] Handle media session setup and teardown
  - [ ] Coordinate RTP/RTCP port allocation
  - [ ] Support hold/resume functionality

- [ ] **Simple Media Bridge** (`/bridge/simple_bridge.rs`)
  - [ ] Connect two RTP streams for call bridging
  - [ ] Handle media mixing for conference scenarios (basic)
  - [ ] Support media recording hooks (basic file output)
  - [ ] Add basic statistics collection

### **Integration with RTP Core**
- [ ] **RTP Integration** (`/integration/rtp_integration.rs`)
  - [ ] Use rtp-core for packet processing
  - [ ] Handle SRTP when security is negotiated
  - [ ] Coordinate with rtp-core's SSRC demultiplexing
  - [ ] Leverage existing RTCP feedback (basic usage)

### **Testing & Examples**
- [ ] **Media Tests**
  - [ ] RTP relay test (packets flow A → B → A)
  - [ ] Codec negotiation test
  - [ ] Media session lifecycle test
- [ ] **Examples**
  - [ ] `simple_media_relay.rs` - Basic RTP forwarding
  - [ ] `codec_negotiation_demo.rs` - Audio codec handling

---

## **📋 Priority 4: SIP Client Library (`sip-client`)**

### **Basic UA Functionality**
- [ ] **User Agent Core** (`/ua/user_agent.rs`)
  - [ ] Handle outbound call initiation (INVITE)
  - [ ] Process inbound calls (respond to INVITE)
  - [ ] Implement call control (hold, transfer, hangup)
  - [ ] Support registration with SIP servers

- [ ] **Client API** (`/api/client_api.rs`)
  - [ ] Simple call API (`make_call()`, `answer_call()`, `hangup()`)
  - [ ] Registration API (`register()`, `unregister()`)
  - [ ] Event callbacks for call state changes
  - [ ] Configuration for server settings and credentials

### **SIP Client Examples**
- [ ] **Client Examples**
  - [ ] `simple_sip_phone.rs` - Basic SIP phone functionality
  - [ ] `sip_registration_client.rs` - Registration example
  - [ ] `basic_call_client.rs` - Make and receive calls

### **Testing & Integration**
- [ ] **End-to-End Tests**
  - [ ] Client-to-client call through server
  - [ ] Registration and call routing test
  - [ ] Authentication flow test

---

## **📋 Priority 5: Integration & Polish**

### **Cross-Crate Integration**
- [ ] **Event Coordination** (`infra-common`)
  - [ ] Basic event bus for SIP events (call state, registration)
  - [ ] Coordinate between dialog, call, and media events
  - [ ] Add proper error propagation across crates

- [ ] **Configuration Management**
  - [ ] Unified configuration system for server and client
  - [ ] Environment-based configuration (dev, staging, prod)
  - [ ] Hot reload capabilities for server configuration

### **Documentation & Examples**
- [ ] **Integration Documentation**
  - [ ] Basic SIP server deployment guide
  - [ ] Client library usage documentation
  - [ ] Configuration reference guide

- [ ] **Comprehensive Examples**
  - [ ] `complete_sip_system.rs` - Server + multiple clients
  - [ ] `sip_interop_test.rs` - Test with other SIP implementations
  - [ ] `performance_test.rs` - Basic load testing

### **Production Readiness**
- [ ] **Monitoring & Logging**
  - [ ] Structured logging for SIP transactions
  - [ ] Basic metrics collection (calls/sec, registrations)
  - [ ] Health check endpoints for server

- [ ] **Error Handling & Recovery**
  - [ ] Graceful degradation on component failures
  - [ ] Proper resource cleanup on errors
  - [ ] Connection recovery and retry logic

---

## **🎯 Implementation Timeline**

### **Week 1-2: Dialog Management (Priority 1)**
- Complete dialog state machines and matching
- Implement basic SDP negotiation
- Add dialog integration with transaction layer

### **Week 3-4: Call Engine (Priority 2)**  
- Build registration server and authentication
- Implement basic proxy and routing functionality
- Add call state management

### **Week 5-6: Media Relay (Priority 3)**
- Create simple RTP relay and codec negotiation
- Implement media session coordination
- Add basic transcoding capabilities

### **Week 7-8: Client & Integration (Priority 4-5)**
- Complete SIP client library
- Add end-to-end testing and examples
- Polish integration and documentation

---

## **🌟 Success Criteria**

### **Functional Goals**
- ✅ Users can register with the SIP server
- ✅ Users can make calls through the server (proxy functionality)
- ✅ Audio flows bidirectionally through media relay
- ✅ Basic call features work (hold, transfer, hangup)
- ✅ System handles multiple concurrent calls

### **Technical Goals**
- ✅ RFC 3261 compliance for basic SIP operations
- ✅ Interoperability with standard SIP clients (like SIPp)
- ✅ Clean separation of concerns across crates
- ✅ Production-ready error handling and logging
- ✅ Documented APIs for integration

### **Performance Goals**
- ✅ Handle 100+ concurrent registrations
- ✅ Support 50+ concurrent calls
- ✅ Sub-100ms call setup time
- ✅ Stable media quality with <1% packet loss

---

**🚀 Target Outcome**: A complete, basic SIP server and client system that can handle real-world SIP communication scenarios without advanced WebRTC features. Perfect foundation for building more advanced capabilities later. 