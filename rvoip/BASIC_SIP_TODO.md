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
- ✅ **Call Engine Structure** (`call-engine`) - Engine, Registry, PolicyEngine framework
- ✅ **Dialog Management** (`session-core`) - DialogManager, Dialog state machines, SDP basics
- ✅ **Session Management** (`session-core`) - SessionManager, Session lifecycle

### **🔶 NEEDS COMPLETION - Critical Missing Pieces**
- 🔶 **Authentication Validation** (`call-engine`) - 90% complete, missing digest validation
- 🔶 **Call Routing Logic** (`call-engine`) - 80% complete, missing proxy forwarding  
- 🔶 **Media Relay** (`media-core`) - 30% complete, needs RTP forwarding
- 🔶 **SIP Client Polish** (`sip-client`) - 70% complete, needs call handling

### **❌ NOT NEEDED for Basic SIP**
- ❌ Advanced RTCP feedback (Phase 1 - already implemented but beyond basic needs)
- ❌ RTP Header Extensions (Phase 2)
- ❌ Adaptive Bitrate Control (Phase 3)  
- ❌ RTP Multiplexing (Phase 4)
- ❌ Simulcast/SVC (Phase 5)

---

## **📋 Priority 1: Complete Authentication (`call-engine`)**
**Status**: 90% complete - challenges work, need digest validation

### **Critical Authentication Functionality**
- [ ] **Digest Authentication Validation** (`/auth/digest_validator.rs`)
  - [ ] Implement RFC 2617 digest response validation
  - [ ] Parse Authorization header from client responses
  - [ ] Validate digest response against stored credentials
  - [ ] Handle nonce tracking and replay protection
  - [ ] Add user credential database interface

- [ ] **User Credential Management** (`/auth/credential_store.rs`)
  - [ ] Create in-memory user credential store for basic version
  - [ ] Add methods for user creation, password validation
  - [ ] Support for different hashing algorithms (MD5, SHA-256)
  - [ ] User management API (add/remove/update users)

- [ ] **Integration with Policy Engine** (`/policy.rs` - enhancement)
  - [ ] Connect authentication validation to policy decisions
  - [ ] Handle authenticated vs. unauthenticated request routing
  - [ ] Add authentication bypass for testing/development

### **Testing & Examples**
- [ ] **Authentication Tests**
  - [ ] Digest authentication round-trip test
  - [ ] Invalid credential rejection test
  - [ ] Nonce replay protection test
- [ ] **Examples**
  - [ ] `authentication_server_demo.rs` - Complete auth flow
  - [ ] `user_management_demo.rs` - Credential management

---

## **📋 Priority 2: Complete Call Routing (`call-engine`)**
**Status**: 80% complete - registry lookup works, need call forwarding

### **Call Routing & Proxy Logic**
- [ ] **INVITE Request Routing** (`/routing/invite_router.rs`)
  - [ ] Route INVITE requests to registered users using Registry
  - [ ] Handle proxy responses and forward them back to caller
  - [ ] Implement proper Via header handling for proxying
  - [ ] Add Record-Route header for dialog establishment

- [ ] **Response Routing** (`/routing/response_router.rs`)
  - [ ] Route responses back to original requester
  - [ ] Handle multiple provisional responses (180, 183)
  - [ ] Forward final responses (200, 4xx, 5xx) correctly
  - [ ] Manage dialog state during call setup

- [ ] **Enhanced Router Integration** (`/routing/mod.rs` - enhancement)
  - [ ] Integrate with existing Router struct in call-engine
  - [ ] Add fallback routing for unregistered users
  - [ ] Support for call forwarding and redirection (3xx responses)
  - [ ] Load balancing for multiple registrations

### **Call State Coordination**
- [ ] **Call Manager Integration** (`/call/call_manager.rs` - enhancement)
  - [ ] Link call routing with session-core DialogManager
  - [ ] Track active calls and their routing state
  - [ ] Handle call termination (BYE) routing
  - [ ] Coordinate between SIP signaling and dialog state

### **Testing & Examples**
- [ ] **Call Routing Tests**
  - [ ] Registration → lookup → INVITE routing test
  - [ ] Multi-hop proxy routing test
  - [ ] Call termination routing test
- [ ] **Examples**
  - [ ] `basic_sip_proxy.rs` - Complete proxy server
  - [ ] `call_routing_demo.rs` - End-to-end call routing

---

## **📋 Priority 3: Complete SIP Client (`sip-client`)**
**Status**: 70% complete - registration works, need call handling

### **Call Management**
- [ ] **Outbound Call Handling** (`/call/outbound_call.rs`)
  - [ ] Enhanced `make_call()` with SDP offer generation
  - [ ] Handle call progress responses (180, 183)
  - [ ] Process final responses and establish dialog
  - [ ] Add call control (hold, transfer, hangup)

- [ ] **Inbound Call Handling** (`/call/inbound_call.rs`)
  - [ ] Process incoming INVITE requests
  - [ ] Generate appropriate responses (180, 200)
  - [ ] Handle SDP answer generation
  - [ ] Add call answer/reject functionality

- [ ] **Enhanced Client API** (`/api/client_api.rs` - enhancement)
  - [ ] Improve call state management and events
  - [ ] Add call progress callbacks
  - [ ] Better error handling and recovery
  - [ ] Enhanced configuration options

### **Testing & Examples**
- [ ] **Client Tests**
  - [ ] End-to-end client call test
  - [ ] Registration + call flow test
  - [ ] Call state machine test
- [ ] **Examples**
  - [ ] `complete_sip_phone.rs` - Full phone functionality
  - [ ] `client_server_demo.rs` - Client talking to our server

---

## **📋 Priority 4: Basic Media Relay (`media-core`)**
**Status**: 30% complete - needs RTP forwarding implementation

### **RTP Packet Forwarding**
- [ ] **Simple RTP Relay** (`/relay/rtp_relay.rs`)
  - [ ] Basic RTP packet forwarding between endpoints
  - [ ] Use existing rtp-core for packet processing
  - [ ] Handle bidirectional media flow
  - [ ] Basic SSRC rewriting for call routing

- [ ] **Media Session Integration** (`/session/media_session.rs`)
  - [ ] Link with session-core Dialog management
  - [ ] Coordinate RTP ports with SDP negotiation
  - [ ] Handle media session setup and teardown
  - [ ] Basic media statistics collection

### **Codec Support**
- [ ] **Basic Codec Handling** (`/codecs/basic_codecs.rs`)
  - [ ] Support G.711 μ-law/A-law passthrough
  - [ ] Basic codec parameter handling
  - [ ] Simple transcoding (if absolutely necessary)
  - [ ] Coordinate with SDP offer/answer

### **Testing & Examples**
- [ ] **Media Tests**
  - [ ] RTP packet relay test
  - [ ] End-to-end audio flow test
  - [ ] Media session lifecycle test
- [ ] **Examples**
  - [ ] `simple_media_bridge.rs` - Basic RTP forwarding
  - [ ] `audio_call_demo.rs` - Complete audio call

---

## **📋 Priority 5: Integration & Polish**

### **End-to-End Testing**
- [ ] **Complete System Tests**
  - [ ] Registration → authentication → call setup → media flow → call teardown
  - [ ] Multiple concurrent calls
  - [ ] Error recovery scenarios

- [ ] **Interoperability Testing**
  - [ ] Test with standard SIP clients (like SIPp)
  - [ ] Test with other SIP servers
  - [ ] Compliance testing for basic SIP scenarios

### **Production Readiness**
- [ ] **Monitoring & Logging**
  - [ ] Structured logging for call flows
  - [ ] Basic metrics (registrations, calls, success rates)
  - [ ] Health check endpoints

- [ ] **Documentation & Deployment**
  - [ ] Complete deployment guide
  - [ ] Configuration reference
  - [ ] Troubleshooting guide

---

## **🎯 Revised Implementation Timeline**

### **Week 1: Authentication Completion (Priority 1)**
- Complete digest authentication validation
- Add user credential management
- Test authentication flow end-to-end

### **Week 2: Call Routing Completion (Priority 2)**  
- Complete INVITE routing and response forwarding
- Integrate with existing Dialog management
- Test basic call setup flows

### **Week 3: SIP Client Enhancement (Priority 3)**
- Complete call handling in sip-client
- Add call control features
- Test client-to-client calls through our server

### **Week 4: Media Relay (Priority 4)**
- Implement basic RTP forwarding
- Integrate with call routing
- Test end-to-end audio calls

### **Week 5: Integration & Polish (Priority 5)**
- End-to-end testing and bug fixes
- Interoperability testing
- Documentation and deployment guides

---

## **🌟 Success Criteria**

### **Functional Goals**
- ✅ Users can register with digest authentication
- ✅ Users can make calls through the server with media
- ✅ Multiple concurrent calls work properly
- ✅ Standard SIP clients can interoperate
- ✅ System handles call failures gracefully

### **Technical Goals**
- ✅ RFC 3261 compliance for basic operations
- ✅ Proper digest authentication (RFC 2617)
- ✅ Clean integration between all components
- ✅ Production-ready error handling
- ✅ Documented and deployable system

**🚀 Target Outcome**: The fastest path to a working basic SIP server and client by building on the substantial foundation that already exists. 