# Layered Architecture: session-core-v3 → b2bua → call-center

## Design

Three separate crates form a layered stack:

```
┌─────────────────────────────────────────┐
│      rvoip-call-center (crate)          │  ← Queues, agents, ACD, supervisor
│  - CallQueue, AgentManager, ACD         │
│  - Supervisor features                  │
│  - Dashboard/monitoring                 │
├─────────────────────────────────────────┤
│         rvoip-b2bua (crate)             │  ← Two-leg bridging, routing
│  - SimpleB2bua, LegCoordinator          │
│  - Audio bridging, transcoding          │
│  - Routing logic                        │
├─────────────────────────────────────────┤
│      rvoip-session-core-v3              │  ← Single session management
│  - StreamPeer, CallbackPeer             │
│  - SessionHandle, AudioStream           │
│  - State machine, adapters              │
└─────────────────────────────────────────┘
```

---

## Key Constraint

**session-core-v3 handles ONE session at a time.** Multi-session orchestration
(B2BUA, call queues, bridges) belongs in higher-layer crates.

---

## session-core-v3 Interfaces

### Two Peer Types

| Peer | Use Case | Style |
|------|----------|-------|
| `StreamPeer` | Clients, tests, sequential scripts | Event loop / sequential `wait_for_X()` |
| `CallbackPeer<H>` | Proxy servers, IVR, reactive apps | Trait-based handlers |

`UnifiedCoordinator` stays `pub` for power users building custom peer types.

---

### Core Types

#### `SessionHandle` — Active call control

```rust
#[derive(Clone)]
pub struct SessionHandle { ... }  // cheap Arc clone, Send + Sync

impl SessionHandle {
    pub fn id(&self) -> &CallId;

    // Call control
    pub async fn hangup(&self) -> Result<()>;
    pub async fn hold(&self) -> Result<()>;
    pub async fn resume(&self) -> Result<()>;
    pub async fn mute(&self) -> Result<()>;
    pub async fn unmute(&self) -> Result<()>;

    // Transfer
    pub async fn transfer_blind(&self, target: &str) -> Result<()>;

    // DTMF
    pub async fn send_dtmf(&self, digit: char) -> Result<()>;

    // Audio — split duplex stream
    pub async fn audio(&self) -> Result<AudioStream>;

    // State
    pub async fn state(&self) -> Result<CallState>;
    pub async fn info(&self) -> Result<SessionInfo>;

    // Per-session events (broadcast, multi-subscriber)
    pub fn events(&self) -> EventReceiver;
}
```

#### `AudioStream` — Duplex audio, caller-owned loop

```rust
pub struct AudioStream {
    pub sender: AudioSender,
    pub receiver: AudioReceiver,
}

#[derive(Clone)]
pub struct AudioSender { ... }
impl AudioSender {
    pub async fn send(&self, frame: AudioFrame) -> Result<()>;
}

pub struct AudioReceiver { ... }
impl AudioReceiver {
    pub async fn recv(&mut self) -> Option<AudioFrame>;
    pub fn try_recv(&mut self) -> Option<AudioFrame>;
}
```

b2bua bridges two sessions by splitting both AudioStreams and cross-wiring them.

#### `IncomingCall` — Three resolution paths

```rust
pub struct IncomingCall {
    pub call_id: CallId,
    pub from: String,
    pub to: String,
    pub sdp: Option<String>,
    pub headers: HashMap<String, String>,
    ...
}

impl IncomingCall {
    pub async fn accept(self) -> Result<SessionHandle>;    // softphone: immediate
    pub fn reject(self, status: u16, reason: &str);       // proxy: immediate reject
    pub fn reject_busy(self);
    pub fn reject_decline(self);
    pub fn redirect(self, target: &str);                   // proxy: 3xx redirect
    pub fn defer(self, timeout: Duration) -> IncomingCallGuard; // call center: hold in ringing
}
```

#### `IncomingCallGuard` — Deferred decision

```rust
pub struct IncomingCallGuard { ... }  // Drop without resolving → auto-reject

impl IncomingCallGuard {
    pub async fn accept(self) -> Result<SessionHandle>;
    pub fn reject(self, status: u16, reason: &str);
    pub fn call_id(&self) -> &CallId;
    pub fn deadline(&self) -> Instant;
}
```

#### `EventReceiver` — Filtered event stream

```rust
pub struct EventReceiver { ... }  // wraps broadcast::Receiver<Event>

impl EventReceiver {
    pub async fn next(&mut self) -> Option<Event>;
    pub fn try_next(&mut self) -> Option<Event>;
}
```

`handle.events()` returns a receiver pre-filtered to that session's `CallId`.

---

### `StreamPeer` — Sequential / event-stream API

```rust
pub struct StreamPeer { ... }

impl StreamPeer {
    pub async fn new(name: &str) -> Result<Self>;
    pub async fn with_config(config: Config) -> Result<Self>;

    // Split: move EventReceiver to a task, keep PeerControl
    pub fn split(self) -> (PeerControl, EventReceiver);

    // Control half (also available via .control())
    pub fn control(&self) -> &PeerControl;

    // Sequential helpers — drive events internally
    pub async fn call(&mut self, target: &str) -> Result<SessionHandle>;
    pub async fn wait_for_incoming(&mut self) -> Result<IncomingCall>;
    pub async fn wait_for_answered(&mut self, id: &CallId) -> Result<SessionHandle>;
    pub async fn wait_for_ended(&mut self, id: &CallId) -> Result<String>;
    pub async fn register(&mut self, params: RegistrationParams) -> Result<RegistrationHandle>;
    pub async fn shutdown(self) -> Result<()>;

    // Direct event access
    pub async fn next_event(&mut self) -> Option<Event>;
}

#[derive(Clone)]
pub struct PeerControl {
    coordinator: Arc<UnifiedCoordinator>,
    local_uri: String,
}

impl PeerControl {
    pub async fn call(&self, target: &str) -> Result<SessionHandle>;
    pub async fn accept(&self, id: &CallId) -> Result<SessionHandle>;
    pub async fn reject(&self, id: &CallId, status: u16, reason: &str) -> Result<()>;
    pub async fn register(&self, params: RegistrationParams) -> Result<RegistrationHandle>;
    pub fn subscribe_events(&self) -> EventReceiver;
}
```

---

### `CallbackPeer<H>` — Trait-based reactive API

```rust
#[async_trait]
pub trait CallHandler: Send + Sync + 'static {
    // Required: decide what to do with incoming call
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision;

    // Optional lifecycle hooks
    async fn on_call_established(&self, handle: SessionHandle) {}
    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {}
    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {}
    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool { false }
}

pub enum CallHandlerDecision {
    Accept,
    AcceptWithSdp(String),
    Reject { status: u16, reason: String },
    Redirect(String),
    Defer(IncomingCallGuard),
}

#[derive(Debug, Clone)]
pub enum EndReason {
    Normal,
    Rejected,
    Timeout,
    NetworkError,
    Other(String),
}

pub struct CallbackPeer<H: CallHandler> { ... }

impl<H: CallHandler> CallbackPeer<H> {
    pub async fn new(handler: H, config: Config) -> Result<Self>;
    pub async fn run(self) -> Result<()>;       // event loop; returns on shutdown
    pub async fn shutdown(&self);
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator>;  // for proactive outgoing calls
}
```

**Proxy example:**
```rust
struct Router { table: HashMap<String, String> }

#[async_trait]
impl CallHandler for Router {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        match self.table.get(&call.to) {
            Some(dest) => CallHandlerDecision::Redirect(dest.clone()),
            None => CallHandlerDecision::Reject { status: 404, reason: "Not Found".into() },
        }
    }
}
```

**IVR / queue example:**
```rust
struct QueueHandler { queue: Arc<CallQueue> }

#[async_trait]
impl CallHandler for QueueHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let guard = call.defer(Duration::from_secs(120));
        self.queue.enqueue(guard.call_id().clone(), guard).await;
        // queue resolves the guard when an agent is available
        CallHandlerDecision::Accept  // temporary accept while queuing
    }
}
```

---

### Registration

Registration is independent of calls. Both peer types access it the same way:

```rust
pub struct RegistrationParams {
    pub registrar_uri: String,
    pub from_uri: String,
    pub contact_uri: String,
    pub credentials: Credentials,
    pub expires: u32,
    pub auto_refresh: bool,
}

pub struct RegistrationHandle { ... }

impl RegistrationHandle {
    pub async fn is_registered(&self) -> Result<bool>;
    pub async fn unregister(self) -> Result<()>;
    pub async fn refresh(&self) -> Result<()>;
    pub fn events(&self) -> EventReceiver;  // RegistrationSuccess / RegistrationFailed
}
```

---

### Public API Surface

```
// Peer types
StreamPeer, PeerControl
CallbackPeer<H>, trait CallHandler

// Core types
SessionHandle
IncomingCall, IncomingCallGuard
CallHandlerDecision, EndReason
AudioStream, AudioSender, AudioReceiver, AudioFrame
EventReceiver, Event

// Registration
RegistrationHandle, RegistrationParams

// Configuration / errors
Config, SessionBuilder
SessionError, Result

// Power user (unstable surface)
UnifiedCoordinator   ← pub, for custom peer type implementations
```

---

## How b2bua Uses session-core-v3

The b2bua crate (to be created) creates TWO sessions and bridges their audio:

```rust
// In b2bua: one call leg
pub struct CallLeg {
    handle: SessionHandle,
    audio: AudioStream,
}

// Bridge two legs
let a_audio = leg_a.handle.audio().await?;
let b_audio = leg_b.handle.audio().await?;

// Cross-wire audio in background tasks
tokio::spawn(async move { /* a_audio.receiver → b_audio.sender */ });
tokio::spawn(async move { /* b_audio.receiver → a_audio.sender */ });
```

For attended transfer at the b2bua level, the layer holds three legs and
drops/bridges them based on transfer progress — all using `SessionHandle` methods.

---

## Dependency Flow

```
call-center
    ↓ uses
b2bua
    ↓ uses
session-core-v3 (StreamPeer / CallbackPeer / SessionHandle)
    ↓ uses
dialog-core + media-core
```

Each layer only depends on the layer below.

---

## Implementation Status

- [x] `UnifiedCoordinator` (core infrastructure)
- [x] `SimplePeer` → renamed/refactored to `StreamPeer`; `SimplePeer` kept as alias
- [x] `SessionHandle` — new
- [x] `AudioStream` / `AudioSender` / `AudioReceiver` — new
- [x] `IncomingCall` (with accept/reject/defer) — new
- [x] `IncomingCallGuard` — new
- [x] `EventReceiver` (broadcast-based) — new
- [x] `PeerControl` — new
- [x] `CallbackPeer<H>` + `CallHandler` trait — new
- [x] `RegistrationParams` — new
- [ ] `redirect()` on IncomingCall (3xx support) — planned
- [ ] Registration auto-refresh — planned
- [ ] `CallbackPeer` outgoing call initiation — planned
