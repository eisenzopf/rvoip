# Session-Core API Refactoring Plan

## Overview

This document outlines the refactoring of the session-core API to provide a more intuitive, developer-friendly interface that clearly separates P2P (peer-to-peer) and B2BUA (back-to-back user agent) use cases.

## Problem Statement

The current API structure exposes SIP protocol details (UAC/UAS) that developers shouldn't need to understand. It also creates an asymmetry where:
- UAC gets a `SimpleCall` handle with full control
- UAS is just a passive server with no call handles
- P2P scenarios require managing both UAC and UAS separately
- B2BUA patterns are difficult to implement

## Solution

Replace the UAC/UAS distinction with role-based APIs:
- **`SimplePeer`** - For P2P clients that make and receive calls
- **`SimpleB2BUA`** - For servers that bridge and control calls
- **`SimpleCall`** - Unified call handle for both sides
- **`CallBridge`** - N-party bridging capability

## New API Structure

```
crates/session-core/src/api/
├── mod.rs                 # Updated exports
├── types.rs               # Core types (enhanced)
├── peer.rs                # NEW - SimplePeer for P2P/client
├── b2bua.rs              # NEW - SimpleB2BUA for servers
├── call.rs               # NEW - Unified SimpleCall
├── bridge.rs             # Enhanced N-party bridging
├── common/               # Shared utilities
│   └── audio.rs
└── legacy/               # Old APIs (deprecated)
    ├── uac.rs
    └── uas.rs
```

## Implementation Details

### 1. SimplePeer (api/peer.rs)

A unified peer that combines UAC and UAS capabilities for P2P scenarios:

```rust
pub struct SimplePeer {
    identity: String,
    coordinator: Arc<SessionCoordinator>,
    incoming_calls: mpsc::Receiver<IncomingCall>,
    registrar: Option<String>,
    port: u16,
}

impl SimplePeer {
    // Create with automatic port selection
    pub async fn new(identity: &str) -> Result<Self>
    
    // Create on specific port
    pub async fn with_port(identity: &str, port: u16) -> Result<Self>
    
    // Register with SIP server (optional)
    pub async fn register(&mut self, server: &str) -> Result<()>
    
    // Make outgoing call
    pub async fn call(&self, target: &str) -> Result<SimpleCall>
    
    // Receive incoming calls
    pub async fn next_incoming(&mut self) -> Option<IncomingCall>
}
```

### 2. SimpleB2BUA (api/b2bua.rs)

Server-focused API for B2BUA scenarios with full capabilities:

```rust
pub struct SimpleB2BUA {
    coordinator: Arc<SessionCoordinator>,
    incoming_calls: mpsc::Receiver<IncomingCall>,
    active_bridges: Arc<RwLock<HashMap<String, CallBridge>>>,
    outbound_peer: Option<SimplePeer>,
}

impl SimpleB2BUA {
    // Create B2BUA with full capabilities (inbound and outbound)
    pub async fn new(bind_addr: &str, identity: &str) -> Result<Self>
    
    // Accept incoming calls
    pub async fn next_incoming(&mut self) -> Option<IncomingCall>
    
    // Make outbound calls
    pub async fn call(&self, target: &str) -> Result<SimpleCall>
    
    // Bridge management
    pub async fn create_bridge(&self, id: &str) -> CallBridge
    pub async fn get_bridge(&self, id: &str) -> Option<CallBridge>
    
    // Helper method for simple bridging
    pub async fn bridge_to(&self, inbound: SimpleCall, target: &str) -> Result<CallBridge>
}
```

### 3. SimpleCall (api/call.rs)

Unified call handle for both UAC and UAS sides:

```rust
pub struct SimpleCall {
    session_id: SessionId,
    coordinator: Arc<SessionCoordinator>,
    audio_tx: Option<mpsc::Sender<AudioFrame>>,
    audio_rx: Option<mpsc::Receiver<AudioFrame>>,
    remote_uri: String,
    state: Arc<RwLock<CallState>>,
}

impl SimpleCall {
    // Get audio channels (consumes - call once)
    pub fn audio_channels(&mut self) -> Result<(Sender, Receiver)>
    
    // Call control
    pub async fn hold(&self) -> Result<()>
    pub async fn resume(&self) -> Result<()>
    pub async fn mute(&self) -> Result<()>
    pub async fn unmute(&self) -> Result<()>
    pub async fn send_dtmf(&self, digits: &str) -> Result<()>
    pub async fn transfer(&self, target: &str) -> Result<()>
    pub async fn hangup(self) -> Result<()>
}
```

### 4. CallBridge (api/bridge.rs)

N-party bridging for conferences and complex scenarios:

```rust
pub struct CallBridge {
    calls: Vec<SimpleCall>,
    bridge_type: BridgeType,
}

pub enum BridgeType {
    Full,        // Everyone connected
    Linear,      // Chain: A <-> B <-> C
    Selective(Vec<(usize, usize)>), // Custom topology
}

impl CallBridge {
    pub async fn add(&mut self, call: SimpleCall) -> usize
    pub async fn remove(&mut self, index: usize) -> Option<SimpleCall>
    pub async fn connect(&self) -> Result<()>
    pub async fn hold(&self, index: usize) -> Result<()>
}
```

### 5. IncomingCall Enhancement (api/types.rs)

Add methods to IncomingCall for symmetric API:

```rust
impl IncomingCall {
    // Accept and get SimpleCall handle
    pub async fn accept(self) -> Result<SimpleCall>
    
    // Reject with reason
    pub async fn reject(self, reason: &str) -> Result<()>
    
    // Forward to another destination
    pub async fn forward(self, target: &str) -> Result<()>
}
```

## Usage Examples

### P2P Direct Call

```rust
use rvoip_session_core::api::prelude::*;

// Create peers
let mut alice = SimplePeer::new("alice").await?;
let mut bob = SimplePeer::new("bob").await?;

// Alice calls Bob
let alice_call = alice.call(&format!("bob@localhost:{}", bob.port())).await?;

// Bob accepts
let incoming = bob.next_incoming().await.unwrap();
let bob_call = incoming.accept().await?;

// Both have SimpleCall - fully symmetric!
```

### Server with Registration

```rust
// Peers register with server
let mut alice = SimplePeer::new("alice").await?;
alice.register("sip.example.com").await?;

// Now can call by name
let call = alice.call("bob").await?;  // Server routes
```

### B2BUA Bridge

```rust
// B2BUA server with full capabilities
let mut b2bua = SimpleB2BUA::new("0.0.0.0:5060", "pbx").await?;

// Accept and route
let incoming = b2bua.next_incoming().await.unwrap();
let inbound = incoming.accept().await?;
let outbound = b2bua.call("support@agents.local").await?;

// Bridge calls
let bridge = b2bua.create_bridge("call_123").await;
bridge.add(inbound).await;
bridge.add(outbound).await;
bridge.connect().await?;

// Or use the helper method
let incoming = b2bua.next_incoming().await.unwrap();
let inbound = incoming.accept().await?;
let bridge = b2bua.bridge_to(inbound, "support@agents.local").await?;
```

### Multi-Party Conference

```rust
let mut conference = SimpleB2BUA::new("0.0.0.0:5060", "conf").await?;
let bridge = conference.create_bridge("room_123").await;

// Add multiple participants
for participant in ["alice", "bob", "charlie"] {
    let call = conference.call(&format!("{}@host.com", participant)).await?;
    bridge.add(call).await;
}

// Connect everyone
bridge.set_type(BridgeType::Full).await;
bridge.connect().await?;
```

## Migration Path

### Phase 1: Add New APIs
1. Create `api/peer.rs`
2. Create `api/b2bua.rs`  
3. Create `api/call.rs`
4. Update `api/bridge.rs`

### Phase 2: Update Existing
1. Enhance `api/types.rs` with IncomingCall methods
2. Update `api/mod.rs` with new exports
3. Create `api/legacy/` directory
4. Move old UAC/UAS to legacy

### Phase 3: Update Tests
1. Convert tests to use SimplePeer
2. Add CallBridge tests
3. Add B2BUA tests

## Benefits

1. **Intuitive** - Developers think in terms of peers and servers, not UAC/UAS
2. **Symmetric** - Both sides get SimpleCall with same capabilities
3. **Scalable** - CallBridge handles 2 to N parties
4. **Compatible** - No changes to core session-core internals
5. **Progressive** - Advanced users can still use SessionControl/MediaControl

## Timeline

- Phase 1: Core implementation (1-2 days)
- Phase 2: Testing and refinement (1 day)
- Phase 3: Documentation and examples (1 day)

## Backwards Compatibility

Old APIs will be moved to `api/legacy/` and marked deprecated. They will continue to work but users will be encouraged to migrate to the new APIs.

## Success Criteria

1. P2P calls work with single SimplePeer object
2. B2BUA can bridge multiple calls
3. All tests pass with new API
4. Examples demonstrate common use cases
5. API is more intuitive than current UAC/UAS split