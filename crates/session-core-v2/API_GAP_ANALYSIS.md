# Session-Core-V2 API Gap Analysis

## The Problem

The session-core-v2 API is significantly worse than the original session-core API, despite the internal architecture being cleaner with the state table design.

## API Comparison

### Old API (session-core) - GOOD ✅
```rust
// Simple peer creation
let alice = SimplePeer::new("alice")
    .port(5060)
    .await?;

// Making a call - simple!
let call = alice.call("bob@127.0.0.1").await?;

// Receiving calls - simple!
if let Some(incoming) = alice.try_incoming() {
    let call = incoming.accept().await?;
}

// Send/receive audio
call.send_audio_frame(frame).await?;
let frame = call.recv_audio_frame().await?;
```

### New API (session-core-v2) - BROKEN ❌
```rust
// Complex setup
let config = Config { /* 6 fields to configure */ };
let coordinator = UnifiedCoordinator::new(config).await?;
let session = UnifiedSession::new(coordinator, Role::UAC).await?; // Why do I need to know UAC/UAS?

// Making a call - OK
session.make_call("sip:bob@127.0.0.1:5061").await?;

// Receiving calls - COMPLETELY BROKEN!
// You have to create a session BEFORE a call exists
// There's no way to listen for incoming calls
// The example resorts to SIMULATING an incoming call!

// Audio - exposed internals
session.send_audio_frame(frame).await?;
let mut rx = session.subscribe_to_audio_frames().await?; // Why subscribe?
```

## Critical Design Flaws

### 1. Session Lifecycle is Backwards
- **Old**: Coordinator creates sessions when calls arrive
- **New**: You create sessions before calls exist (makes no sense!)

### 2. No Incoming Call Handler
- **Old**: Simple callback `on_incoming_call` at coordinator level
- **New**: No mechanism to handle incoming calls at all

### 3. Exposed Complexity
- **Old**: Hides UAC/UAS distinction - you're just a "peer"
- **New**: Forces you to choose Role::UAC or Role::UAS upfront

### 4. No Automatic Session Management
- **Old**: Sessions created/destroyed automatically
- **New**: Manual session management

### 5. Complex Audio API
- **Old**: Simple send/recv on the call object
- **New**: Subscribe pattern with channels

## Root Cause

The fundamental design flaw is that session-core-v2 treats sessions as something you create manually, rather than something that gets created automatically when calls arrive or are made.

The state table design is good for internal state management, but it leaked into the public API, making it complex.

## What's Missing

### 1. SimplePeer API
A high-level API that:
- Hides the coordinator complexity
- Automatically creates sessions for incoming/outgoing calls
- Provides simple `call()` and `try_incoming()` methods
- Manages the session lifecycle

### 2. Incoming Call Handler
The coordinator needs:
- A callback mechanism for incoming calls
- Automatic session creation for UAS scenarios
- A way to defer call decisions

### 3. Unified Call Object
Instead of separate sessions with roles:
- A single Call object that works for both directions
- Simple audio send/recv methods
- Hide the underlying complexity

## Proposed Solution

### Step 1: Add SimplePeer to session-core-v2

```rust
// New simplified API
pub struct SimplePeer {
    coordinator: Arc<UnifiedCoordinator>,
    incoming_rx: mpsc::Receiver<IncomingCall>,
}

impl SimplePeer {
    pub async fn new(name: &str) -> Result<Self> { /* ... */ }
    pub async fn call(&self, target: &str) -> Result<Call> { /* ... */ }
    pub fn try_incoming(&mut self) -> Option<IncomingCall> { /* ... */ }
}
```

### Step 2: Fix Coordinator

Add incoming call handling to UnifiedCoordinator:
```rust
impl UnifiedCoordinator {
    pub fn on_incoming_call<F>(&self, handler: F) 
    where F: Fn(IncomingCall) -> CallDecision { /* ... */ }
}
```

### Step 3: Create Call Abstraction

```rust
pub struct Call {
    session: Arc<UnifiedSession>,
}

impl Call {
    pub async fn send_audio(&self, frame: AudioFrame) -> Result<()> { /* ... */ }
    pub async fn recv_audio(&self) -> Result<AudioFrame> { /* ... */ }
}
```

## Impact

Without these fixes:
- The API is unusable for real applications
- Examples resort to simulation instead of real functionality
- Users will stick with the old session-core
- The state table benefits are wasted

## Priority

This is **CRITICAL** - the library is fundamentally broken for receiving calls.