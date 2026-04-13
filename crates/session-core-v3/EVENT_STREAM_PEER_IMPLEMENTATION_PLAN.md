# EventStreamPeer Implementation Plan

## Executive Summary

Create an **event stream API** (`event_stream.rs`) as an alternative to `simple.rs`, using Rust's async stream pattern (like tokio broadcast channels or futures::Stream).

**Key Features:**
- Subscribe to typed event streams
- Filter events by type or criteria
- Composable with async stream operators
- Explicit event handling (nothing automatic)
- Full user control

---

## Problem Statement

Current SimplePeer API has critical flaws:
- ❌ Events can be missed if not actively polled
- ❌ Blocking operations prevent event processing
- ❌ No way to handle asynchronous events without explicit waiting

**Solution:** Expose event streams that users explicitly consume and process.

---

## Core API Design

### EventStreamPeer Structure

```rust
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

/// Event stream-based SIP peer
pub struct EventStreamPeer {
    coordinator: Arc<UnifiedCoordinator>,
    event_broadcaster: broadcast::Sender<Event>,
    local_uri: String,
    
    // For monitoring
    stats: Arc<RwLock<EventStats>>,
}

impl EventStreamPeer {
    /// Create a new event stream peer
    pub async fn new(name: impl Into<String>) -> Result<Self> {
        let config = Config::default();
        Self::with_config(name, config).await
    }
    
    /// Create with custom configuration
    pub async fn with_config(name: impl Into<String>, mut config: Config) -> Result<Self> {
        let name = name.into();
        
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        let local_uri = config.local_uri.clone();
        
        // Create broadcast channel for events
        let (event_tx, _) = broadcast::channel(1000);
        let event_broadcaster = event_tx.clone();
        
        // Create coordinator with event forwarding
        let (internal_tx, mut internal_rx) = mpsc::channel(1000);
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, internal_tx).await?;
        
        let stats = Arc::new(RwLock::new(EventStats::default()));
        
        // Spawn forwarder to broadcast channel
        let event_tx_clone = event_tx.clone();
        let stats_clone = stats.clone();
        tokio::spawn(async move {
            while let Some(event) = internal_rx.recv().await {
                stats_clone.write().await.total_events += 1;
                
                // Broadcast to all subscribers
                let _ = event_tx_clone.send(event);
            }
        });
        
        Ok(Self {
            coordinator,
            event_broadcaster,
            local_uri,
            stats,
        })
    }
}
```

---

## Event Stream Methods

### Get All Events

```rust
impl EventStreamPeer {
    /// Get a stream of all events
    /// 
    /// Each call to this method creates a new independent subscriber.
    /// Events are broadcast to all subscribers.
    pub fn events(&self) -> BroadcastStream<Event> {
        BroadcastStream::new(self.event_broadcaster.subscribe())
    }
    
    /// Get a stream of events, dropping lagged events instead of erroring
    pub fn events_lossy(&self) -> impl Stream<Item = Event> {
        BroadcastStream::new(self.event_broadcaster.subscribe())
            .filter_map(|result| async move { result.ok() })
    }
}
```

### Get Filtered Event Streams

```rust
impl EventStreamPeer {
    /// Get stream of only incoming call events
    pub fn incoming_calls(&self) -> impl Stream<Item = IncomingCallInfo> {
        self.events_lossy()
            .filter_map(|event| async move {
                match event {
                    Event::IncomingCall { call_id, from, to, .. } => {
                        Some(IncomingCallInfo { call_id, from, to })
                    }
                    _ => None
                }
            })
    }
    
    /// Get stream of only transfer requests
    pub fn transfers(&self) -> impl Stream<Item = ReferRequest> {
        self.events_lossy()
            .filter_map(|event| async move {
                match event {
                    Event::ReferReceived { call_id, refer_to, transaction_id, transfer_type, .. } => {
                        Some(ReferRequest { call_id, refer_to, transaction_id, transfer_type })
                    }
                    _ => None
                }
            })
    }
    
    /// Get stream of call lifecycle events for a specific call
    pub fn call_events(&self, call_id: CallId) -> impl Stream<Item = CallEvent> {
        self.events_lossy()
            .filter_map(move |event| {
                let call_id = call_id.clone();
                async move {
                    match event {
                        Event::CallAnswered { call_id: id, .. } if id == call_id => {
                            Some(CallEvent::Answered)
                        }
                        Event::CallEnded { call_id: id, reason } if id == call_id => {
                            Some(CallEvent::Ended { reason })
                        }
                        Event::CallFailed { call_id: id, status_code, reason } if id == call_id => {
                            Some(CallEvent::Failed { status_code, reason })
                        }
                        _ => None
                    }
                }
            })
    }
    
    /// Get stream of DTMF digits
    pub fn dtmf_stream(&self) -> impl Stream<Item = (CallId, char)> {
        self.events_lossy()
            .filter_map(|event| async move {
                match event {
                    Event::DtmfReceived { call_id, digit } => Some((call_id, digit)),
                    _ => None
                }
            })
    }
}

/// Call lifecycle events
#[derive(Debug, Clone)]
pub enum CallEvent {
    Answered,
    Ended { reason: String },
    Failed { status_code: u16, reason: String },
    OnHold,
    Resumed,
}
```

---

## Usage Examples

### Example 1: Simple Auto-Accept (NEW - Inspired by session-core)
```rust
use rvoip_session_core_v3::api::event_stream::EventStreamPeer;

// SIMPLEST: One line! (Like session-core's AutoAnswerHandler)
let peer = EventStreamPeer::with_auto_answer("agent").await?;

// That's it! All calls and transfers handled automatically
let call = peer.call("sip:customer@...").await?;
```

**Just 2 lines!** As simple as PolicyPeer!

### Example 1b: Builder Pattern (NEW)
```rust
// Configure behavior upfront (no manual spawning!)
let peer = EventStreamPeerBuilder::new("agent")
    .auto_accept_calls()
    .auto_accept_transfers()
    .enable_dtmf_commands()
    .enable_logging()
    .build().await?;

// Everything handled in background!
```

**Simple as PolicyPeer, but with stream power underneath!**

### Example 1c: Manual Event Loop (Original - For Advanced Users)
```rust
use rvoip_session_core_v3::api::event_stream::EventStreamPeer;
use tokio_stream::StreamExt;

let peer = EventStreamPeer::new("alice").await?;
let mut events = peer.events_lossy();

// Event loop
while let Some(event) = events.next().await {
    match event {
        Event::IncomingCall { call_id, from, .. } => {
            println!("📞 Call from {}", from);
            peer.accept(&call_id).await?;
        }
        Event::ReferReceived { call_id, refer_to, .. } => {
            println!("🔄 Transfer to {}", refer_to);
            peer.hangup(&call_id).await?;
            peer.call(&refer_to).await?;
        }
        Event::CallEnded { call_id, reason } => {
            println!("📴 Call {} ended: {}", call_id, reason);
        }
        _ => {}
    }
}
```

**Full control when needed!**

### Example 2: Custom Stream Processing (Helper Methods)
```rust
let peer = EventStreamPeer::new("alice").await?;

// Use helper methods instead of manual spawning!
peer.process_incoming_calls(|call, peer| Box::pin(async move {
    if call.from.contains("@trusted.com") {
        peer.accept(&call.call_id).await.ok();
    } else {
        peer.reject(&call.call_id, "Untrusted").await.ok();
    }
}));

peer.process_transfers(|refer, peer| Box::pin(async move {
    // Complete transfer
    peer.hangup(&refer.call_id).await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;
    peer.call(&refer.refer_to).await.ok();
}));

// Main thread can make outbound calls
let call = peer.call("sip:bob@...").await?;
```

**Helper methods hide the spawning complexity!**

### Example 2b: Manual Filtered Streams (Advanced Users)
```rust
let peer = EventStreamPeer::new("alice").await?;

// Manual spawning for full control
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            println!("Incoming: {}", call.from);
            peer.accept(&call.call_id).await.unwrap();
        }
    }
});

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut transfers = peer.transfers();
        while let Some(refer) = transfers.next().await {
            println!("Transfer to: {}", refer.refer_to);
            peer.hangup(&refer.call_id).await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            peer.call(&refer.refer_to).await.unwrap();
        }
    }
});

// Main thread can make outbound calls
let call = peer.call("sip:bob@...").await?;
```

### Example 3: Stream Combinators
```rust
use tokio_stream::StreamExt;

let peer = EventStreamPeer::new("alice").await?;

// Filter, map, and process
let mut important_calls = peer.incoming_calls()
    .filter(|call| async move { 
        call.from.contains("@important.com") 
    })
    .map(|call| async move {
        println!("Important call: {}", call.from);
        call
    });

while let Some(call) = important_calls.next().await {
    peer.accept(&call.call_id).await?;
}
```

### Example 4: Select Multiple Streams
```rust
use tokio::select;

let peer = EventStreamPeer::new("alice").await?;
let mut calls = peer.incoming_calls();
let mut transfers = peer.transfers();

loop {
    select! {
        Some(call) = calls.next() => {
            println!("Incoming call from {}", call.from);
            peer.accept(&call.call_id).await?;
        }
        Some(refer) = transfers.next() => {
            println!("Transfer to {}", refer.refer_to);
            // Handle transfer
        }
        else => break,
    }
}
```

### Example 5: Collect DTMF Digits
```rust
use tokio_stream::StreamExt;

let peer = EventStreamPeer::new("ivr").await?;
let call = peer.call("sip:customer@...").await?;

// Collect DTMF until '#' pressed
let digits: Vec<char> = peer.dtmf_stream()
    .filter(|(id, _)| async move { *id == call })  // Only this call
    .map(|(_, digit)| digit)
    .take_while(|digit| async move { *digit != '#' })
    .collect()
    .await;

println!("User entered: {:?}", digits);
```

### Example 6: Per-Call Event Monitoring
```rust
let peer = EventStreamPeer::new("alice").await?;
let call = peer.call("sip:bob@...").await?;

// Monitor just this call
let mut call_events = peer.call_events(call.clone());

while let Some(event) = call_events.next().await {
    match event {
        CallEvent::Answered => println!("Call connected!"),
        CallEvent::Ended { reason } => {
            println!("Call ended: {}", reason);
            break;
        }
        CallEvent::Failed { status_code, reason } => {
            println!("Call failed: {} {}", status_code, reason);
            break;
        }
        _ => {}
    }
}
```

---

## Call Operations (Same as Others)

```rust
impl EventStreamPeer {
    pub async fn call(&self, target: &str) -> Result<CallId> { ... }
    pub async fn accept(&self, call_id: &CallId) -> Result<()> { ... }
    pub async fn reject(&self, call_id: &CallId, reason: &str) -> Result<()> { ... }
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> { ... }
    pub async fn hold(&self, call_id: &CallId) -> Result<()> { ... }
    pub async fn resume(&self, call_id: &CallId) -> Result<()> { ... }
    pub async fn send_refer(&self, call_id: &CallId, refer_to: &str) -> Result<()> { ... }
    pub async fn send_audio(&self, call_id: &CallId, frame: AudioFrame) -> Result<()> { ... }
    pub async fn subscribe_to_audio(&self, call_id: &CallId) -> Result<AudioFrameSubscriber> { ... }
    pub async fn stats(&self) -> EventStats { ... }
}
```

---

## Advantages

### 1. **Explicit Control**
- User decides exactly which events to handle
- No magic behavior
- Full visibility into what's happening

### 2. **Composable**
- Use standard stream operators (filter, map, take, etc.)
- Combine streams with select!
- Chain operations

### 3. **Rust Idiomatic**
- Uses tokio::Stream trait
- Works with tokio-stream crate
- Familiar to Rust developers

### 4. **Per-Call Granularity**
- Can subscribe to specific call's events
- Isolate concerns per call
- Perfect for multi-call scenarios

### 5. **No Hidden State**
- All events are explicit
- No automatic behaviors
- Clear data flow

---

## Disadvantages

### 1. **Requires Active Consumption**
- Must spawn tasks to consume streams
- Streams can lag if not drained
- More boilerplate than callbacks or policies

### 2. **No Safe Defaults**
- Incoming calls ignored if stream not consumed
- Transfers ignored if stream not consumed
- Easy to miss events

### 3. **Complexity**
- Need to understand async streams
- Need to understand select!, spawn, etc.
- Higher learning curve for beginners

### 4. **Broadcast Lag**
- Slow consumers can lag behind
- Need to handle RecvError::Lagged
- Or use lossy streams

---

## Implementation Size (With Helpers)

### Code Breakdown
- **EventStreamPeer struct:** ~20 lines
- **Constructor:** ~40 lines
- **Event stream methods:** ~150 lines
  - events() / events_lossy()
  - incoming_calls()
  - transfers()
  - call_events()
  - dtmf_stream()
  - Other filtered streams
- **Helper methods (NEW):** ~120 lines
  - auto_accept_calls()
  - auto_reject_calls()
  - auto_accept_transfers()
  - auto_reject_transfers()
  - enable_dtmf_commands()
  - enable_event_logging()
  - process_incoming_calls()
  - process_transfers()
- **Pre-configured constructors (NEW):** ~60 lines
  - with_auto_answer()
  - with_reject_all()
  - with_logging()
  - for_call_center()
- **Enhanced builder:** ~80 lines (with auto-spawn options)
- **Call operations:** ~80 lines
- **Helper types:** ~40 lines
- **Event forwarder:** ~30 lines

**Total: ~620 lines**

### Files
1. ✅ `src/api/event_stream.rs` - New file (620 lines)
2. ✅ `src/api/mod.rs` - Add export (2 lines)
3. ✅ `examples/event_stream_demo/main.rs` - Example (80 lines, simpler!)

**Total new code: ~700 lines**

**Comparison:**
- Original plan: 360 lines (complex for users)
- With helpers: 620 lines (simple for users!)
- **+260 lines makes it as easy as PolicyPeer**

---

## Stream Combinators

### Built-in Helpers

```rust
impl EventStreamPeer {
    /// Get stream of events matching a predicate
    pub fn events_where<F>(&self, predicate: F) -> impl Stream<Item = Event>
    where F: Fn(&Event) -> bool + Send + 'static
    {
        self.events_lossy()
            .filter(move |event| async move { predicate(event) })
    }
    
    /// Get stream of events for a specific call
    pub fn events_for_call(&self, call_id: CallId) -> impl Stream<Item = Event> {
        self.events_lossy()
            .filter(move |event| {
                let call_id = call_id.clone();
                async move { 
                    event.call_id().map(|id| *id == call_id).unwrap_or(false)
                }
            })
    }
    
    /// Get stream of call state changes
    pub fn state_changes(&self) -> impl Stream<Item = (CallId, CallState)> {
        self.events_lossy()
            .filter_map(|event| async move {
                // Would need StateChanged event
                None
            })
    }
}
```

---

## Advanced Usage Patterns

### Pattern 1: Multiple Consumers
```rust
let peer = EventStreamPeer::new("alice").await?;

// Consumer 1: Handle incoming calls
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut stream = peer.incoming_calls();
        while let Some(call) = stream.next().await {
            peer.accept(&call.call_id).await.unwrap();
        }
    }
});

// Consumer 2: Handle transfers
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut stream = peer.transfers();
        while let Some(refer) = stream.next().await {
            // Handle transfer
        }
    }
});

// Consumer 3: Log all events
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut stream = peer.events_lossy();
        while let Some(event) = stream.next().await {
            log_event(event);
        }
    }
});

// Main task: Make outbound calls
let call = peer.call("sip:bob@...").await?;
```

### Pattern 2: Stream Processing Pipeline
```rust
use tokio_stream::StreamExt;

let peer = EventStreamPeer::new("alice").await?;

let mut processed_calls = peer.incoming_calls()
    .filter(|call| async move {
        // Only from trusted domains
        call.from.ends_with("@trusted.com")
    })
    .map(|call| async move {
        // Add metadata
        (call, chrono::Utc::now())
    })
    .take(10);  // Only handle first 10 calls

while let Some((call, timestamp)) = processed_calls.next().await {
    println!("[{}] Call from {}", timestamp, call.from);
    peer.accept(&call.call_id).await?;
}
```

### Pattern 3: Race Multiple Events
```rust
use tokio::select;

let peer = EventStreamPeer::new("alice").await?;
let call = peer.call("sip:bob@...").await?;

let mut events = peer.call_events(call.clone());

select! {
    Some(CallEvent::Answered) = events.next() => {
        println!("Call answered!");
    }
    _ = tokio::time::sleep(Duration::from_secs(30)) => {
        println!("Timeout!");
        peer.hangup(&call).await?;
    }
}
```

### Pattern 4: Merge Multiple Streams
```rust
use tokio_stream::StreamExt;

let peer = EventStreamPeer::new("alice").await?;

// Merge different event types into one stream
let mut combined = tokio_stream::StreamExt::merge(
    peer.incoming_calls().map(CombinedEvent::IncomingCall),
    peer.transfers().map(CombinedEvent::Transfer),
);

while let Some(event) = combined.next().await {
    match event {
        CombinedEvent::IncomingCall(info) => { /* ... */ }
        CombinedEvent::Transfer(refer) => { /* ... */ }
    }
}
```

---

## Helper Methods (Inspired by session-core)

### Auto-Spawn Stream Consumers

Make it easy to set up common patterns without manual spawning:

```rust
impl EventStreamPeer {
    /// Auto-accept all incoming calls (spawns background task)
    pub fn auto_accept_calls(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut calls = peer.incoming_calls();
            while let Some(call) = calls.next().await {
                tracing::info!("Auto-accepting call from {}", call.from);
                let _ = peer.accept(&call.call_id).await;
            }
        });
    }
    
    /// Auto-reject all incoming calls (spawns background task)
    pub fn auto_reject_calls(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut calls = peer.incoming_calls();
            while let Some(call) = calls.next().await {
                tracing::info!("Auto-rejecting call from {}", call.from);
                let _ = peer.reject(&call.call_id, "Declined").await;
            }
        });
    }
    
    /// Auto-complete blind transfers (spawns background task)
    pub fn auto_accept_transfers(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut transfers = peer.transfers();
            while let Some(refer) = transfers.next().await {
                tracing::info!("Auto-accepting transfer to {}", refer.refer_to);
                let _ = peer.hangup(&refer.call_id).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                let _ = peer.call(&refer.refer_to).await;
            }
        });
    }
    
    /// Auto-reject all transfers (spawns background task)
    pub fn auto_reject_transfers(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut transfers = peer.transfers();
            while let Some(refer) = transfers.next().await {
                tracing::info!("Auto-rejecting transfer to {}", refer.refer_to);
                // Rejection happens automatically in dialog-core
            }
        });
    }
    
    /// Process DTMF as commands (spawns background task)
    pub fn enable_dtmf_commands(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut dtmf = peer.dtmf_stream();
            while let Some((call_id, digit)) = dtmf.next().await {
                match digit {
                    '1' => { peer.hold(&call_id).await.ok(); }
                    '2' => { peer.resume(&call_id).await.ok(); }
                    '9' => { peer.hangup(&call_id).await.ok(); }
                    _ => {}
                }
            }
        });
    }
    
    /// Log all events to tracing (spawns background task)
    pub fn enable_event_logging(&self) {
        let peer = self.clone();
        tokio::spawn(async move {
            let mut events = peer.events_lossy();
            while let Some(event) = events.next().await {
                tracing::info!("Event: {:?}", event);
            }
        });
    }
}
```

### Pre-Configured Constructors (Like session-core's built-in handlers)

```rust
impl EventStreamPeer {
    /// Create peer with auto-answer behavior (like session-core's AutoAnswerHandler)
    pub async fn with_auto_answer(name: impl Into<String>) -> Result<Self> {
        let peer = Self::new(name).await?;
        peer.auto_accept_calls();
        peer.auto_accept_transfers();
        Ok(peer)
    }
    
    /// Create peer that rejects everything (secure default)
    pub async fn with_reject_all(name: impl Into<String>) -> Result<Self> {
        let peer = Self::new(name).await?;
        peer.auto_reject_calls();
        peer.auto_reject_transfers();
        Ok(peer)
    }
    
    /// Create peer with full logging enabled
    pub async fn with_logging(name: impl Into<String>) -> Result<Self> {
        let peer = Self::new(name).await?;
        peer.enable_event_logging();
        Ok(peer)
    }
    
    /// Create call center peer (accept all, auto-transfer)
    pub async fn for_call_center(name: impl Into<String>) -> Result<Self> {
        let peer = Self::new(name).await?;
        peer.auto_accept_calls();
        peer.auto_accept_transfers();
        peer.enable_dtmf_commands();
        peer.enable_event_logging();
        Ok(peer)
    }
}
```

### Simplified Usage

Now developers can use EventStreamPeer simply:

```rust
// SIMPLE: Just one line for auto-answer!
let peer = EventStreamPeer::with_auto_answer("agent").await?;

// That's it! Incoming calls and transfers handled automatically
let call = peer.call("sip:customer@...").await?;
```

Or with manual control:

```rust
// ADVANCED: Full control when needed
let peer = EventStreamPeer::new("alice").await?;

// Set up custom stream processing
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls()
            .filter(|call| async move { call.from.contains("@trusted.com") });
        
        while let Some(call) = calls.next().await {
            peer.accept(&call.call_id).await.unwrap();
        }
    }
});
```

---

## Builder Pattern (Enhanced)

```rust
pub struct EventStreamPeerBuilder {
    name: String,
    config: Config,
    buffer_size: usize,
    
    // Auto-enable common behaviors
    auto_accept_calls: bool,
    auto_reject_calls: bool,
    auto_accept_transfers: bool,
    auto_reject_transfers: bool,
    enable_dtmf_commands: bool,
    enable_logging: bool,
}

impl EventStreamPeerBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config: Config::default(),
            buffer_size: 1000,
            auto_accept_calls: false,
            auto_reject_calls: false,
            auto_accept_transfers: false,
            auto_reject_transfers: false,
            enable_dtmf_commands: false,
            enable_logging: false,
        }
    }
    
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }
    
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
    
    /// Auto-accept all incoming calls
    pub fn auto_accept_calls(mut self) -> Self {
        self.auto_accept_calls = true;
        self
    }
    
    /// Auto-reject all incoming calls
    pub fn auto_reject_calls(mut self) -> Self {
        self.auto_reject_calls = true;
        self
    }
    
    /// Auto-accept all transfers
    pub fn auto_accept_transfers(mut self) -> Self {
        self.auto_accept_transfers = true;
        self
    }
    
    /// Auto-reject all transfers
    pub fn auto_reject_transfers(mut self) -> Self {
        self.auto_reject_transfers = true;
        self
    }
    
    /// Enable DTMF commands (*1=hold, *2=resume, *9=hangup)
    pub fn enable_dtmf_commands(mut self) -> Self {
        self.enable_dtmf_commands = true;
        self
    }
    
    /// Enable event logging
    pub fn enable_logging(mut self) -> Self {
        self.enable_logging = true;
        self
    }
    
    pub async fn build(self) -> Result<EventStreamPeer> {
        let peer = EventStreamPeer::with_config(self.name, self.config).await?;
        
        // Auto-spawn based on configuration
        if self.auto_accept_calls {
            peer.auto_accept_calls();
        }
        if self.auto_reject_calls {
            peer.auto_reject_calls();
        }
        if self.auto_accept_transfers {
            peer.auto_accept_transfers();
        }
        if self.auto_reject_transfers {
            peer.auto_reject_transfers();
        }
        if self.enable_dtmf_commands {
            peer.enable_dtmf_commands();
        }
        if self.enable_logging {
            peer.enable_event_logging();
        }
        
        Ok(peer)
    }
}
```

### Simplified Builder Usage

```rust
// Call center agent - just configure!
let peer = EventStreamPeerBuilder::new("agent")
    .auto_accept_calls()
    .auto_accept_transfers()
    .enable_dtmf_commands()
    .build().await?;

// Now it's as simple as PolicyPeer!
// No manual stream spawning needed
```

---

## Complete Example: Call Center

```rust
use rvoip_session_core_v3::api::event_stream::EventStreamPeer;
use tokio_stream::StreamExt;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    let peer = EventStreamPeer::new("agent").await?;
    
    // Track active calls
    let active_calls = Arc::new(Mutex::new(HashMap::new()));
    
    // Task 1: Handle incoming calls
    tokio::spawn({
        let peer = peer.clone();
        let active_calls = active_calls.clone();
        async move {
            let mut incoming = peer.incoming_calls();
            while let Some(call) = incoming.next().await {
                println!("📞 Incoming: {}", call.from);
                
                // Accept call
                peer.accept(&call.call_id).await.unwrap();
                
                // Track it
                active_calls.lock().await.insert(call.call_id.clone(), call.from);
            }
        }
    });
    
    // Task 2: Handle transfers
    tokio::spawn({
        let peer = peer.clone();
        let active_calls = active_calls.clone();
        async move {
            let mut transfers = peer.transfers();
            while let Some(refer) = transfers.next().await {
                println!("🔄 Transfer to {}", refer.refer_to);
                
                // Complete transfer
                peer.hangup(&refer.call_id).await.unwrap();
                tokio::time::sleep(Duration::from_millis(500)).await;
                let new_call = peer.call(&refer.refer_to).await.unwrap();
                
                // Update tracking
                let mut calls = active_calls.lock().await;
                calls.remove(&refer.call_id);
                calls.insert(new_call, refer.refer_to);
            }
        }
    });
    
    // Task 3: Handle DTMF
    tokio::spawn({
        let peer = peer.clone();
        async move {
            let mut dtmf = peer.dtmf_stream();
            while let Some((call_id, digit)) = dtmf.next().await {
                println!("🔢 DTMF {} on {}", digit, call_id);
                
                // Execute commands
                match digit {
                    '1' => peer.hold(&call_id).await.unwrap(),
                    '2' => peer.resume(&call_id).await.unwrap(),
                    '9' => peer.hangup(&call_id).await.unwrap(),
                    _ => {}
                }
            }
        }
    });
    
    // Main loop: Status monitoring
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let calls = active_calls.lock().await;
        println!("📊 Active calls: {}", calls.len());
    }
}
```

---

## Comparison: SimplePeer vs CallbackPeer vs EventStreamPeer

| Feature | SimplePeer | CallbackPeer | EventStreamPeer |
|---------|-----------|--------------|-----------------|
| **Pattern** | Sequential | Callback | Stream |
| **API Style** | Blocking | Event-driven | Reactive |
| **Configuration** | None | Callbacks | Stream subscriptions |
| **Defaults** | N/A | None | None |
| **Flexibility** | Low | High | Very High |
| **Boilerplate** | Low | Medium | Medium-High |
| **Learning Curve** | Low | Low | Medium-High |
| **Lost Events** | ⚠️ Yes | ✅ No | ✅ No |
| **Multi-Call** | ⚠️ Limited | ✅ Yes | ✅ Yes (best) |
| **Composability** | ❌ No | ⚠️ Limited | ✅ Excellent |
| **Type Safety** | ✅ Yes | ✅ Yes | ✅ Yes |
| **Runtime Changes** | N/A | ✅ Yes | ✅ Yes |
| **Code Size** | ~200 lines | ~480 lines | ~360 lines |
| **Rust Idiomatic** | ⚠️ No | ⚠️ Sort of | ✅ Very |

---

## Advantages

### 1. **Rust Idiomatic**
- Uses standard Stream trait
- Works with tokio-stream operators
- Familiar to experienced Rust developers

### 2. **Extremely Flexible**
- Filter, map, merge, select streams
- Build complex event processing pipelines
- Compose streams in powerful ways

### 3. **Explicit and Clear**
- All event handling is visible
- No hidden behavior
- Clear data flow

### 4. **Perfect for Multi-Call**
- Each call can have its own event stream
- Easy to track per-call state
- No cross-call pollution

### 5. **Testable**
- Easy to mock event streams
- Can test stream processing logic in isolation
- Deterministic behavior

---

## Disadvantages

### 1. **Higher Learning Curve**
- Need to understand async streams
- Need to understand stream operators
- Need to understand spawn/select patterns

### 2. **More Boilerplate**
- Must spawn tasks for each stream
- Must handle stream errors
- More setup code

### 3. **No Safe Defaults**
- Incoming calls dropped if not consumed
- Transfers ignored if not consumed
- Everything is explicit (pro and con)

### 4. **Broadcast Complexity**
- Need to handle lagged consumers
- Need to choose buffered vs lossy
- Can be confusing for beginners

---

## Implementation Timeline (With Helpers)

### Phase 1: Core Structure (2 hours)
- EventStreamPeer struct
- Constructor with broadcast channel
- Event forwarder task

### Phase 2: Stream Methods (3 hours)
- events() / events_lossy()
- incoming_calls()
- transfers()
- call_events()
- dtmf_stream()
- Other filtered streams

### Phase 3: Helper Methods (3 hours) - NEW
- auto_accept_calls()
- auto_reject_calls()
- auto_accept_transfers()
- auto_reject_transfers()
- enable_dtmf_commands()
- enable_event_logging()
- process_incoming_calls()
- process_transfers()

### Phase 4: Pre-Configured Constructors (1 hour) - NEW
- with_auto_answer()
- with_reject_all()
- with_logging()
- for_call_center()

### Phase 5: Enhanced Builder (1 hour) - NEW
- Builder with auto-spawn options
- Conditional spawning logic

### Phase 6: Call Operations (1 hour)
- Wrap UnifiedCoordinator methods
- Add stats tracking

### Phase 7: Testing (2 hours)
- Unit tests for helpers
- Unit tests for stream filtering
- Integration tests with multiple consumers
- Example programs (simpler now!)

**Total: 13 hours (~620 lines)**

**Worth it?** Yes! +5 hours makes EventStreamPeer as easy to use as PolicyPeer while keeping stream power for advanced users.

---

## When to Use EventStreamPeer

**Good for (with helpers):**
- ✅ **Anyone!** (helpers make it as easy as PolicyPeer)
- ✅ Simple apps (use `with_auto_answer()`)
- ✅ Complex pipelines (use manual streams)
- ✅ Multi-call applications (best per-call isolation)
- ✅ Reactive programming fans

**Advanced features for:**
- ✅ Custom stream processing
- ✅ Complex event pipelines
- ✅ Per-call event monitoring

**Trade-off:**
- ✅ Simple for common cases (via helpers)
- ✅ Powerful for complex cases (via streams)
- ❌ More library code to maintain (+260 lines)

---

## Comparison with PolicyPeer (With Helpers)

| Aspect | PolicyPeer | EventStreamPeer (With Helpers) |
|--------|-----------|-------------------------------|
| **Defaults** | ✅ Safe defaults | ✅ Via helpers! |
| **Simple Usage** | ✅ 3 policies | ✅ Pre-configured constructors |
| **Advanced Usage** | ❌ Limited | ✅ Full stream power |
| **Verbosity (Simple)** | Low (3 policies) | Low (use helpers) |
| **Verbosity (Advanced)** | N/A | Medium (manual streams) |
| **Flexibility** | Medium (3 options) | Very High (streams + helpers) |
| **Learning Curve** | Low | Low (helpers) → High (streams) |
| **Familiarity** | New pattern | Rust streams (optional) |
| **Composability** | ❌ No | ✅ Excellent |
| **Per-Call Control** | ❌ No | ✅ Yes |
| **Code Size** | 380 lines | 620 lines |
| **Best For** | Simple configs | Everything! |

**Key Insight:** With helpers, EventStreamPeer becomes **both simple AND powerful**!

---

## Hybrid Approach: Best of Both?

```rust
// Can mix PolicyPeer config with event streams
let peer = PolicyPeerBuilder::new("alice")
    .incoming_call_policy(IncomingCallPolicy::Accept)  // Default
    .build().await?;

// But also expose event streams for monitoring
impl PolicyPeer {
    pub fn event_stream(&self) -> impl Stream<Item = Event> {
        // Expose underlying events even with policies
    }
}

// Use defaults for most things, stream for special cases
let mut transfers = peer.event_stream()
    .filter_map(|e| match e {
        Event::ReferReceived { .. } => Some(e),
        _ => None
    });
```

---

## Testing Strategy

### Unit Tests
```rust
#[tokio::test]
async fn test_incoming_call_stream() {
    let peer = EventStreamPeer::new("test").await.unwrap();
    let mut stream = peer.incoming_calls();
    
    // Simulate incoming call
    // Verify stream receives it
}

#[tokio::test]
async fn test_stream_filtering() {
    let peer = EventStreamPeer::new("test").await.unwrap();
    
    let mut filtered = peer.incoming_calls()
        .filter(|call| async move { call.from.contains("@test.com") });
    
    // Simulate calls from different domains
    // Verify only @test.com calls come through
}

#[tokio::test]
async fn test_broadcast_to_multiple_consumers() {
    let peer = EventStreamPeer::new("test").await.unwrap();
    
    let mut stream1 = peer.events_lossy();
    let mut stream2 = peer.events_lossy();
    
    // Both should receive the same events
}
```

---

## Migration from SimplePeer

### Before (SimplePeer)
```rust
let mut peer = SimplePeer::new("alice").await?;

loop {
    // Poll for events manually
    if let Some(refer) = peer.wait_for_refer().await? {
        // Handle transfer
    }
}
```

### After (EventStreamPeer)
```rust
let peer = EventStreamPeer::new("alice").await?;

// Spawn background task for transfers
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut transfers = peer.transfers();
        while let Some(refer) = transfers.next().await {
            // Handle transfer automatically
        }
    }
});

// Main code is cleaner
let call = peer.call("sip:bob@...").await?;
```

---

## Pros and Cons Summary

### ✅ Advantages
1. **Rust idiomatic** - Uses Stream trait
2. **Composable** - Rich operators available
3. **Explicit** - All event handling visible
4. **Powerful** - Can build complex pipelines
5. **Testable** - Easy to test stream logic

### ❌ Disadvantages
1. **Complex** - Higher learning curve
2. **Verbose** - More boilerplate
3. **No defaults** - Everything explicit
4. **Broadcast lag** - Need to handle lagging
5. **Beginner unfriendly** - Requires async expertise

---

## When to Choose EventStreamPeer

### Choose EventStreamPeer if:
- ✅ You're comfortable with Rust async/streams
- ✅ You need complex event processing pipelines
- ✅ You want maximum flexibility
- ✅ You're building a reactive system
- ✅ You want per-call event isolation

### Choose PolicyPeer if:
- ✅ You want safe defaults
- ✅ You want simple configuration
- ✅ You want less boilerplate
- ✅ You're building a standard softphone/call center

### Choose CallbackPeer if:
- ✅ You're familiar with callback patterns
- ✅ You want runtime flexibility
- ✅ You're coming from JavaScript/Node.js

### Choose SimplePeer if:
- ✅ You're writing tests/examples
- ✅ You have simple, deterministic flows
- ✅ You don't care about lost events

---

## Conclusion

EventStreamPeer provides the **most powerful and flexible** API using Rust's async stream pattern. It's perfect for advanced users who need complex event processing, but has a steeper learning curve than the alternatives.

**Tradeoffs:**
- ✅ Most flexible (stream operators)
- ✅ Most Rust idiomatic
- ❌ Most complex (async streams)
- ❌ Most verbose (spawn tasks)
- ✅ Best for multi-call scenarios
- ❌ No safe defaults

**Best for:** Advanced Rust developers building complex reactive systems.

---

## Next Steps

1. ⏳ Review all three plans (Policy, Callback, EventStream)
2. ⏳ Compare tradeoffs
3. ⏳ Choose one approach (or implement all three!)
4. ⏳ Implement chosen approach
5. ⏳ Keep SimplePeer for backward compatibility

