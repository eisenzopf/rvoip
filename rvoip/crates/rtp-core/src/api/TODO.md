# API Examples - Error Analysis & TODO

Generated: 2025-05-23

## Summary

Tested all 13 examples starting with `api_*` for ERROR and WARN messages. Found both build-time warnings and runtime errors that need to be addressed.

---

# 🚀 NEW FEATURE PLAN: Non-DTLS SRTP & Authentication Schemes

**Priority:** HIGH  
**Target:** Add comprehensive SRTP and authentication support to API client/server libraries  
**Goal:** Support SIP-derived SRTP key exchange mechanisms (SDES, MIKEY, ZRTP) in addition to existing DTLS-SRTP

## Current State Analysis

### ✅ What We Have
- **Core SRTP Implementation** (`/src/srtp/`): Complete with encryption, authentication, key derivation
- **Security Key Exchange Protocols** (`/src/security/`):
  - ✅ SDES (RFC 4568) - SDP Security Descriptions 
  - ✅ MIKEY (RFC 3830) - Multimedia Internet KEYing
  - ✅ ZRTP (RFC 6189) - Media Path Key Agreement
- **API Infrastructure** (`/src/api/client/` & `/src/api/server/`): 
  - ✅ Security modules with basic SRTP structures
  - ✅ DTLS-SRTP support (WebRTC compatible)
  - ✅ Configuration builders and factories

### ❌ What We Need
- **Integration Layer**: Connect core security protocols to API clients/servers
- **Non-DTLS SRTP**: SIP-derived key exchange for SRTP without DTLS handshake
- **Unified Security API**: Consistent interface across all key exchange methods
- **Key Syndication**: Support for key distribution and management
- **Configuration Profiles**: Pre-built configs for common SIP/WebRTC scenarios

## Implementation Plan

### Phase 1: Core Integration Infrastructure (Week 1-2)

#### 1.1 Unified Security Context
**File:** `/src/api/common/security/unified_context.rs`
```rust
pub enum KeyExchangeMethod {
    DtlsSrtp,      // Existing WebRTC DTLS-SRTP
    Sdes,          // SDP Security Descriptions
    Mikey,         // Multimedia Internet KEYing  
    Zrtp,          // Z Real-time Transport Protocol
    PreSharedKey,  // Direct key configuration
}

pub struct UnifiedSecurityContext {
    method: KeyExchangeMethod,
    srtp_context: SrtpContext,
    key_exchange: Box<dyn SecurityKeyExchange>,
    state: SecurityState,
}
```

#### 1.2 Client Security Enhancement
**Files:** 
- `/src/api/client/security/sdes.rs` - SDES client implementation
- `/src/api/client/security/mikey.rs` - MIKEY client implementation  
- `/src/api/client/security/zrtp.rs` - ZRTP client implementation
- Update `/src/api/client/security/mod.rs` with new exports

#### 1.3 Server Security Enhancement  
**Files:**
- `/src/api/server/security/sdes.rs` - SDES server implementation
- `/src/api/server/security/mikey.rs` - MIKEY server implementation
- `/src/api/server/security/zrtp.rs` - ZRTP server implementation
- Update `/src/api/server/security/mod.rs` with new exports

### Phase 2: SIP-Derived SRTP Implementation (Week 3-4)

#### 2.1 SDES Integration (Highest Priority)
- **Use Case:** SIP/SDP signaling with in-band key exchange
- **Implementation:**
  ```rust
  // Client usage
  let client = ClientConfigBuilder::new()
      .with_sdes_srtp(sdes_config)
      .build();
  
  // Server usage  
  let server = ServerConfigBuilder::new()
      .with_sdes_srtp(sdes_config)
      .build();
  ```

#### 2.2 MIKEY Integration (Medium Priority)
- **Use Case:** Pre-arranged key management for enterprise SIP
- **Features:** 
  - PSK (Pre-Shared Key) mode
  - PKE (Public Key Exchange) mode
  - Key update/rotation support

#### 2.3 ZRTP Integration (Medium Priority)  
- **Use Case:** Peer-to-peer secure calling without PKI
- **Features:**
  - In-media key exchange
  - Short authentication strings (SAS)
  - Perfect forward secrecy

### Phase 3: Key Syndication & Management (Week 5)

#### 3.1 Key Rotation & Lifecycle Management
**File:** `/src/api/common/security/key_management.rs`
```rust
pub struct KeyManager {
    rotation_policy: KeyRotationPolicy,
    key_store: KeyStore,
    syndication: KeySyndication,
}

pub enum KeyRotationPolicy {
    TimeInterval(Duration),
    PacketCount(u64),
    Manual,
    Never,
}
```

#### 3.2 Multi-Stream Key Syndication
- **Use Case:** Single negotiation for multiple media streams
- **Implementation:** Derive audio/video keys from master key material
- **Standards:** Follow RFC recommendations for key derivation

### Phase 4: Configuration & API Enhancement (Week 6)

#### 4.1 Security Profile System
**File:** `/src/api/common/config/security_profiles.rs`
```rust
impl SecurityConfig {
    // SIP scenarios
    pub fn sip_enterprise() -> Self;     // MIKEY with PSK
    pub fn sip_operator() -> Self;       // SDES with operator keys
    pub fn sip_peer_to_peer() -> Self;   // ZRTP for P2P calls
    
    // Hybrid scenarios  
    pub fn sip_webrtc_bridge() -> Self;  // SDES<->DTLS-SRTP bridge
    pub fn multi_protocol() -> Self;     // Support multiple methods
}
```

#### 4.2 Enhanced Configuration Builders
```rust
// Client configurations
impl ClientConfigBuilder {
    pub fn with_sdes_inline_keys(self, sdp_crypto_lines: Vec<String>) -> Self;
    pub fn with_mikey_psk(self, psk: Vec<u8>, identity: String) -> Self;
    pub fn with_zrtp_hello(self, zrtp_config: ZrtpConfig) -> Self;
    pub fn with_security_fallback(self, methods: Vec<KeyExchangeMethod>) -> Self;
}

// Server configurations  
impl ServerConfigBuilder {
    pub fn with_sdes_offer(self, crypto_suites: Vec<SrtpCryptoSuite>) -> Self;
    pub fn with_mikey_responder(self, key_store: KeyStore) -> Self;
    pub fn with_zrtp_responder(self, zrtp_config: ZrtpConfig) -> Self;
}
```

### Phase 5: Examples & Documentation (Week 7)

#### 5.1 New API Examples
- [ ] `api_srtp_sdes.rs` - SDES-based SRTP with SDP exchange
- [ ] `api_srtp_mikey.rs` - MIKEY-based enterprise SRTP  
- [ ] `api_srtp_zrtp.rs` - ZRTP peer-to-peer secure media
- [ ] `api_srtp_multi_method.rs` - Multiple key exchange support
- [ ] `api_sip_webrtc_bridge.rs` - SIP<->WebRTC security bridge

#### 5.2 Integration Tests
- [ ] SDES offer/answer negotiation
- [ ] MIKEY key exchange scenarios
- [ ] ZRTP handshake validation
- [ ] Cross-protocol compatibility
- [ ] Key rotation testing

## Technical Specifications

### Security Method Support Matrix

| Method | Client | Server | Key Exchange | Use Case |
|--------|--------|--------|--------------|----------|
| DTLS-SRTP | ✅ | ✅ | In-band | WebRTC |
| SDES | 🚧 | 🚧 | SDP signaling | SIP/SDP |
| MIKEY | 🚧 | 🚧 | Separate protocol | Enterprise |
| ZRTP | 🚧 | 🚧 | In-media | P2P calling |
| PSK | 🚧 | 🚧 | Pre-configured | Testing/Simple |

Legend: ✅ Complete, 🚧 To implement, ❌ Not planned

### Key Exchange Flow Examples

#### SDES Flow (SIP/SDP)
```
Client                           Server
  |                                |
  |  SDP Offer (a=crypto lines)    |
  |------------------------------->|
  |                                |
  |  SDP Answer (selected crypto)  |
  |<-------------------------------|
  |                                |
  |     SRTP Media Exchange        |
  |<=============================>|
```

#### MIKEY Flow (Enterprise)
```
Client                           Server  
  |                                |
  |     MIKEY-INIT message         |
  |------------------------------->|
  |                                |
  |    MIKEY-RESP message          |
  |<-------------------------------|
  |                                |
  |     SRTP Media Exchange        |
  |<=============================>|
```

## Implementation Checklist

### Phase 1: Infrastructure
- [ ] Create `UnifiedSecurityContext` 
- [ ] Implement `SecurityContextManager`
- [ ] Add security method enumeration
- [ ] Create base traits and interfaces
- [ ] Update client/server security modules

### Phase 2: Protocol Integration
- [ ] **SDES Client Implementation**
  - [ ] SDP crypto attribute parsing
  - [ ] Key extraction and validation
  - [ ] SRTP context setup
- [ ] **SDES Server Implementation**  
  - [ ] Crypto offer generation
  - [ ] Answer processing
  - [ ] Key confirmation
- [ ] **MIKEY Integration** (PSK mode first)
- [ ] **ZRTP Integration** (basic handshake first)

### Phase 3: Advanced Features
- [ ] Key rotation mechanisms
- [ ] Multi-stream syndication  
- [ ] Error recovery and fallback
- [ ] Security policy enforcement

### Phase 4: Testing & Examples
- [ ] Unit tests for each protocol
- [ ] Integration test scenarios
- [ ] Performance benchmarking
- [ ] API example programs
- [ ] Documentation updates

## Success Criteria

1. **Functional:** All three key exchange methods (SDES, MIKEY, ZRTP) work end-to-end
2. **Compatible:** Existing DTLS-SRTP functionality remains unchanged
3. **Configurable:** Simple API for common scenarios, flexible for advanced use
4. **Performant:** No significant overhead compared to current DTLS-SRTP
5. **Tested:** Comprehensive test coverage including cross-protocol scenarios

## Risk Mitigation

- **Complexity:** Implement SDES first (simplest), then MIKEY, then ZRTP
- **Breaking Changes:** Keep existing APIs intact, add new functionality alongside
- **Testing:** Create isolated test environments for each protocol
- **Standards Compliance:** Validate against RFC test vectors where available

---

## Build-Time Issues

### 🔴 HIGH PRIORITY - Cargo.toml Configuration Issues

**Affects:** ALL examples  
**Issue:** Unused manifest keys in sip-core crate  
**Messages:**
```
warning: /Users/jonathan/Documents/Work/Rudeless Ventures/rvoip/rvoip/crates/sip-core/Cargo.toml: unused manifest key: dependencies.serde.version
warning: /Users/jonathan/Documents/Work/Rudeless Ventures/rvoip/rvoip/crates/sip-core/Cargo.toml: unused manifest key: dependencies.serde_json.version  
warning: /Users/jonathan/Documents/Work/Rudeless Ventures/rvoip/rvoip/crates/sip-core/Cargo.toml: unused manifest key: dependencies.uuid.version
```

**Action Required:**
- [ ] Review and clean up `crates/sip-core/Cargo.toml` dependencies section
- [ ] Remove unused version keys or fix dependency specification format

## Runtime Issues by Example

### 🔴 HIGH PRIORITY - Connection & Transport Issues

#### `api_srtp` - SRTP Security Example
**Status:** ❌ Multiple Errors  
**Issues:**
- `ERROR`: Client connection timed out after 2 seconds
- `WARN`: Failed to send frame 0: Transport not connected  
- `ERROR`: Server receive error: Timeout error: No frame received within timeout period
- `WARN`: Failed to send frames 1-4: Transport not connected

**Root Cause:** DTLS handshake or SRTP setup failures  
**Action Required:**
- [ ] Investigate DTLS handshake timeout issues
- [ ] Review SRTP key exchange implementation
- [ ] Add better error handling and retry logic
- [ ] Consider adding mock/test mode for examples

#### `api_rtcp_app_bye_xr` - RTCP Extended Reports Example  
**Status:** ⚠️ Expected Warnings
**Issues:**
- `WARN`: No clients connected, cannot send APP packet
- `WARN`: No clients connected, cannot send XR packet

**Root Cause:** Example tries to send RTCP packets without active clients  
**Action Required:**
- [ ] Add mock client connection for demonstration
- [ ] Update example to show both server and client sides
- [ ] Add connection establishment before RTCP operations

#### `api_media_sync` - Media Synchronization Example
**Status:** ⚠️ Expected Warnings  
**Issues:**
- `WARN`: No synchronization info available for audio stream
- `WARN`: No synchronization info available for video stream  
- `WARN`: Failed to convert audio timestamp to video timestamp

**Root Cause:** Example attempts sync operations without established media streams  
**Action Required:**
- [ ] Add proper stream establishment before sync operations
- [ ] Include sample RTP packets with timing information
- [ ] Demonstrate sync setup process step-by-step

### 🟢 WORKING CORRECTLY - No Runtime Issues

These examples run without errors or warnings:

#### Core Functionality
- [x] `api_basic` - Basic RTP transport ✅
- [x] `api_high_performance_buffers` - Buffer management ✅
- [x] `api_rtcp_mux` - RTCP multiplexing ✅
- [x] `api_rtcp_reports` - RTCP reporting ✅

#### SSRC & Demultiplexing  
- [x] `api_ssrc_demultiplexing` - SSRC demux ✅
- [x] `api_ssrc_demux` - SSRC demux alternative ✅
- [x] `api_ssrc_demux_test` - SSRC demux testing ✅

#### Extension & Management
- [x] `api_csrc_management_test` - CSRC management ✅
- [x] `api_header_extensions` - RTP header extensions ✅
- [x] `api_header_extensions_simple` - Simple extensions ✅

## Action Plan Priority

### Phase 1: Build Issues (Immediate)
1. **Fix Cargo.toml warnings** - Clean up sip-core dependencies
   - Impact: All builds
   - Effort: Low
   - Priority: High

### Phase 2: Connection Issues (Critical)
2. **Fix api_srtp connection timeouts**
   - Impact: SRTP functionality demonstration
   - Effort: Medium
   - Priority: High
   - Investigate: DTLS handshake, key exchange, timeout handling

### Phase 3: Example Enhancement (Important)
3. **Enhance api_rtcp_app_bye_xr** 
   - Add client connection setup
   - Impact: RTCP demonstration
   - Effort: Low
   - Priority: Medium

4. **Enhance api_media_sync**
   - Add proper stream setup
   - Impact: Media sync demonstration  
   - Effort: Medium
   - Priority: Medium

## Testing Verification

After fixes, verify with:
```bash
# Test all api examples for errors
for example in $(find ./crates/rtp-core/examples -name "api_*.rs" | sed 's/.*\///g' | sed 's/\.rs$//g'); do
    echo "Testing $example..."
    cargo run --example $example 2>&1 | grep -E "(ERROR|WARN|error|warn)" || echo "✅ Clean"
done
```

## Notes

- Runtime warnings in `api_rtcp_app_bye_xr` and `api_media_sync` may be acceptable if they demonstrate expected behavior when connections/streams aren't established
- The SSRC demultiplexing examples are working correctly after the recent refactoring
- Core RTP transport functionality appears stable
- Security-related examples (`api_srtp`) need the most attention

---

**Last Updated:** 2025-05-23  
**Tested Examples:** 13/13  
**Critical Issues:** 2  
**Enhancement Opportunities:** 2 