# CallbackPeer Implementation Plan (Trait-Based)

## Executive Summary

Create a **trait-based callback API** (`callback.rs`) as an alternative to `simple.rs`, using the trait pattern from session-core.

**Key Features:**
- Implement `PeerHandler` trait with callback methods
- Background processor triggers callbacks automatically
- Clean trait syntax (no Box::pin boilerplate)
- Compatible with session-core patterns
- Flexible and familiar to session-core users

---

## Problem Statement

Current SimplePeer API has critical flaws:
- ❌ Events can be missed if not actively polled
- ❌ Blocking operations prevent event processing
- ❌ No way to handle asynchronous events without explicit waiting

**Solution:** Implement a trait with callback methods (like session-core's CallHandler).

---

## Core API Design

### PeerHandler Trait (Similar to session-core's CallHandler)

```rust
use async_trait::async_trait;

/// Event handler for CallbackPeer
/// 
/// Implement this trait to handle call events. All methods have default
/// implementations, so you only need to override what you care about.
#[async_trait]
pub trait PeerHandler: Send + Sync + std::fmt::Debug {
    /// Handle incoming calls - must return a decision
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        CallDecision::Reject  // Safe default
    }
    
    /// Handle when a call is answered (200 OK received)
    async fn on_call_answered(&self, call_id: CallId) {
        tracing::info!("Call answered: {}", call_id);
    }
    
    /// Handle when a call ends
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        tracing::info!("Call ended: {} ({})", call_id, reason);
    }
    
    /// Handle when a call fails
    async fn on_call_failed(&self, call_id: CallId, status_code: u16, reason: String) {
        tracing::warn!("Call failed: {} - {} {}", call_id, status_code, reason);
    }
    
    /// Handle transfer requests - must return a decision
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        TransferDecision::Reject  // Safe default
    }
    
    /// Handle DTMF digits
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        tracing::debug!("DTMF {} on call {}", digit, call_id);
    }
    
    /// Handle hold requests
    async fn on_hold(&self, call_id: CallId) {
        tracing::info!("Call {} put on hold", call_id);
    }
    
    /// Handle resume requests
    async fn on_resume(&self, call_id: CallId) {
        tracing::info!("Call {} resumed", call_id);
    }
}
```

### CallbackPeer Structure

```rust
/// Trait-based callback SIP peer
pub struct CallbackPeer {
    coordinator: Arc<UnifiedCoordinator>,
    handler: Arc<dyn PeerHandler>,
    event_processor_handle: JoinHandle<()>,
    local_uri: String,
    
    // For monitoring
    stats: Arc<RwLock<EventStats>>,
}
```

---

## Constructor and Builder

```rust
impl CallbackPeer {
    /// Create a new callback peer with handler
    pub async fn new<H>(name: impl Into<String>, handler: Arc<H>) -> Result<Self>
    where H: PeerHandler + 'static
    {
        let config = Config::default();
        Self::with_config(name, config, handler).await
    }
    
    /// Create with custom configuration
    pub async fn with_config<H>(
        name: impl Into<String>, 
        mut config: Config,
        handler: Arc<H>
    ) -> Result<Self>
    where H: PeerHandler + 'static
    {
        let name = name.into();
        
        // Set local URI
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        let local_uri = config.local_uri.clone();
        
        // Create coordinator
        let (event_tx, event_rx) = mpsc::channel(1000);
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, event_tx).await?;
        
        let stats = Arc::new(RwLock::new(EventStats::default()));
        
        // Spawn background event processor
        let event_processor_handle = Self::spawn_event_processor(
            event_rx,
            coordinator.clone(),
            handler.clone() as Arc<dyn PeerHandler>,
            stats.clone(),
        );
        
        Ok(Self {
            coordinator,
            handler: handler as Arc<dyn PeerHandler>,
            event_processor_handle,
            local_uri,
            stats,
        })
    }
}

/// Builder pattern for CallbackPeer
pub struct CallbackPeerBuilder<H: PeerHandler> {
    name: String,
    config: Config,
    handler: Arc<H>,
}

impl<H: PeerHandler + 'static> CallbackPeerBuilder<H> {
    pub fn new(name: impl Into<String>, handler: Arc<H>) -> Self {
        Self {
            name: name.into(),
            config: Config::default(),
            handler,
        }
    }
    
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }
    
    pub async fn build(self) -> Result<CallbackPeer> {
        CallbackPeer::with_config(self.name, self.config, self.handler).await
    }
}
```

---

## Built-in Handler Implementations

### AutoAnswerHandler

Automatically accept all incoming calls and transfers.

```rust
/// Handler that accepts all calls
#[derive(Debug)]
pub struct AutoAnswerHandler;

#[async_trait]
impl PeerHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        tracing::info!("Auto-accepting call from {}", call.from);
        CallDecision::Accept
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        tracing::info!("Auto-accepting transfer to {}", refer.refer_to);
        TransferDecision::Accept
    }
}

// Usage:
let peer = CallbackPeer::new("agent", Arc::new(AutoAnswerHandler)).await?;
```

### RejectAllHandler

Reject all incoming calls and transfers (safe default).

```rust
/// Handler that rejects all calls (secure default)
#[derive(Debug)]
pub struct RejectAllHandler;

#[async_trait]
impl PeerHandler for RejectAllHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        tracing::warn!("Rejecting call from {}", call.from);
        CallDecision::Reject
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        tracing::warn!("Rejecting transfer to {}", refer.refer_to);
        TransferDecision::Reject
    }
}

// Usage:
let peer = CallbackPeer::new("secure", Arc::new(RejectAllHandler)).await?;
```

### Custom Handler Implementation

Clean trait implementation without Box::pin!
    
```rust
#[derive(Debug)]
struct MyHandler {
    db: Arc<Database>,
    allowed_domains: Vec<String>,
}

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Clean async code, no Box::pin!
        
        // Check whitelist
        let domain = call.from.split('@').nth(1).unwrap_or("");
        if !self.allowed_domains.contains(&domain.to_string()) {
            return CallDecision::Reject;
        }
        
        // Check database
        if let Ok(caller_info) = self.db.lookup(&call.from).await {
            if caller_info.is_blocked {
                return CallDecision::Reject;
            }
        }
        
        CallDecision::Accept
    }
    
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        // Log to database
        self.db.log_call_end(&call_id, &reason).await.ok();
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        // Check if transfer target is allowed
        if refer.refer_to.contains("@trusted.com") {
            TransferDecision::Accept
        } else {
            TransferDecision::Reject
        }
    }
    
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        println!("DTMF {} on call {}", digit, call_id);
    }
}

// Usage:
let handler = Arc::new(MyHandler {
    db: Arc::new(Database::new()),
    allowed_domains: vec!["company.com".to_string()],
});

let peer = CallbackPeer::new("alice", handler).await?;
```

### Composite Handlers (Advanced)

Chain multiple handlers together:

```rust
/// Chains multiple handlers, trying each in order
#[derive(Debug)]
pub struct CompositeHandler {
    handlers: Vec<Arc<dyn PeerHandler>>,
}

impl CompositeHandler {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }
    
    pub fn add(mut self, handler: Arc<dyn PeerHandler>) -> Self {
        self.handlers.push(handler);
        self
    }
}

#[async_trait]
impl PeerHandler for CompositeHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Try each handler until one makes a decision
        for handler in &self.handlers {
            let decision = handler.on_incoming_call(call.clone()).await;
            if decision != CallDecision::Continue {
                return decision;
            }
        }
        CallDecision::Reject  // Default if all defer
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        // Similar pattern for transfers
        for handler in &self.handlers {
            let decision = handler.on_transfer(refer.clone()).await;
            // Could add Continue variant if needed
            return decision;
        }
        TransferDecision::Reject
    }
    
    // Other events call all handlers
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        for handler in &self.handlers {
            handler.on_call_ended(call_id.clone(), reason.clone()).await;
        }
    }
}

// Usage:
let handler = CompositeHandler::new()
    .add(Arc::new(LoggingHandler))
    .add(Arc::new(WhitelistHandler))
    .add(Arc::new(AutoAnswerHandler));

let peer = CallbackPeer::new("alice", Arc::new(handler)).await?;
```

---

```rust
#[derive(Debug)]
struct ConditionalHandler {
    allowed_domains: Vec<String>,
}

#[async_trait]
impl PeerHandler for ConditionalHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        let domain = call.from.split('@').nth(1).unwrap_or("");
        
        if self.allowed_domains.iter().any(|d| domain.contains(d)) {
            tracing::info!("Accepting call from trusted domain: {}", domain);
            CallDecision::Accept
        } else {
            tracing::warn!("Rejecting call from untrusted domain: {}", domain);
            CallDecision::Reject
        }
    }
    
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        tracing::info!("Call {} ended: {}", call_id, reason);
    }
}
```

---

## Background Event Processor

```rust
impl CallbackPeer {
    fn spawn_event_processor(
        mut event_rx: mpsc::Receiver<Event>,
        coordinator: Arc<UnifiedCoordinator>,
        handler: Arc<dyn PeerHandler>,
        stats: Arc<RwLock<EventStats>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // Update stats
                stats.write().await.total_events += 1;
                
                // Route event to appropriate handler method
                match event {
                    Event::IncomingCall { call_id, from, to, .. } => {
                        let info = IncomingCallInfo { call_id: call_id.clone(), from, to };
                        let decision = handler.on_incoming_call(info).await;
                        
                        match decision {
                            CallDecision::Accept => {
                                let _ = coordinator.accept_call(&call_id).await;
                            }
                            CallDecision::Reject => {
                                let _ = coordinator.reject_call(&call_id, "Declined").await;
                            }
                            CallDecision::RejectBusy => {
                                let _ = coordinator.reject_call(&call_id, "Busy Here").await;
                            }
                            _ => {}
                        }
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::CallAnswered { call_id, .. } => {
                        handler.on_call_answered(call_id).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::ReferReceived { call_id, refer_to, transaction_id, transfer_type, .. } => {
                        let refer = ReferRequest { call_id: call_id.clone(), refer_to, transaction_id, transfer_type };
                        let decision = handler.on_transfer(refer.clone()).await;
                        
                        match decision {
                            TransferDecision::Accept => {
                                // Complete transfer in background
                                let coordinator = coordinator.clone();
                                tokio::spawn(async move {
                                    let _ = coordinator.hangup(&refer.call_id).await;
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    // Make new call to refer.refer_to
                                });
                            }
                            TransferDecision::Reject => {
                                // Automatic rejection
                            }
                        }
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::DtmfReceived { call_id, digit } => {
                        handler.on_dtmf(call_id, digit).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::CallEnded { call_id, reason } => {
                        handler.on_call_ended(call_id, reason).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::CallFailed { call_id, status_code, reason } => {
                        handler.on_call_failed(call_id, status_code, reason).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::CallOnHold { call_id } => {
                        handler.on_hold(call_id).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    Event::CallResumed { call_id } => {
                        handler.on_resume(call_id).await;
                        stats.write().await.handled_by_trait += 1;
                    }
                    
                    _ => {
                        tracing::debug!("Unhandled event: {:?}", event);
                        stats.write().await.unhandled += 1;
                    }
                }
            }
            
            tracing::info!("Event processor stopped");
        })
    }
}
```

---

## Usage Examples

### Example 1: Call Center Agent (Auto-Accept)
```rust
use rvoip_session_core_v3::api::callback::{CallbackPeer, AutoAnswerHandler};

// Just use the built-in handler!
let peer = CallbackPeer::new("agent", Arc::new(AutoAnswerHandler)).await?;

// That's it! All calls and transfers auto-accepted
let call = peer.call("sip:customer@...").await?;
```

**Clean! Just 2 lines!**

### Example 2: Softphone with UI (Interactive)
```rust
#[derive(Debug)]
struct UiHandler {
    ui: Arc<UserInterface>,
}

#[async_trait]
impl PeerHandler for UiHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Show UI dialog (clean async code!)
        let decision = self.ui.show_incoming_call_dialog(&call.from).await;
        
        if decision == UiDecision::Accept {
            CallDecision::Accept
        } else {
            CallDecision::Reject
        }
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        // Show confirmation
        let approved = self.ui.show_transfer_dialog(&refer.refer_to).await;
        
        if approved {
            TransferDecision::Accept
        } else {
            TransferDecision::Reject
        }
    }
    
    async fn on_call_answered(&self, call_id: CallId) {
        self.ui.update_status(&call_id, "Connected");
    }
    
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        self.ui.update_status(&call_id, &format!("Ended: {}", reason));
    }
}

// Usage:
let handler = Arc::new(UiHandler {
    ui: Arc::new(UserInterface::new()),
});

let peer = CallbackPeer::new("alice", handler).await?;
```

**Much cleaner! No Box::pin, no closures, just clean async methods.**

### Example 3: Monitoring/Logging
```rust
#[derive(Debug)]
struct LoggingHandler {
    log_file: Arc<Mutex<File>>,
}

#[async_trait]
impl PeerHandler for LoggingHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        self.log(&format!("Incoming call from {}", call.from)).await;
        CallDecision::Reject  // Monitor only
    }
    
    async fn on_call_answered(&self, call_id: CallId) {
        self.log(&format!("Call {} answered", call_id)).await;
    }
    
    async fn on_call_ended(&self, call_id: CallId, reason: String) {
        self.log(&format!("Call {} ended: {}", call_id, reason)).await;
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        self.log(&format!("Transfer to {}", refer.refer_to)).await;
        TransferDecision::Reject
    }
}

impl LoggingHandler {
    async fn log(&self, message: &str) {
        let mut file = self.log_file.lock().await;
        writeln!(file, "[{}] {}", chrono::Utc::now(), message).ok();
    }
}
```

### Example 4: Chaining Handlers (Composite)
```rust
// Chain multiple handlers
let handler = CompositeHandler::new()
    .add(Arc::new(LoggingHandler::new()))
    .add(Arc::new(WhitelistHandler::new()))
    .add(Arc::new(AutoAnswerHandler));

let peer = CallbackPeer::new("alice", Arc::new(handler)).await?;

// Handlers called in order:
// 1. LoggingHandler logs the call
// 2. WhitelistHandler checks whitelist (returns decision if matched)
// 3. AutoAnswerHandler accepts (if whitelist returned Continue)
```

### Example 5: DTMF Commands
```rust
#[derive(Debug)]
struct DtmfCommandHandler {
    peer: Arc<CallbackPeer>,  // Can store peer ref for call operations
}

#[async_trait]
impl PeerHandler for DtmfCommandHandler {
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        match digit {
            '1' => { self.peer.hold(&call_id).await.ok(); }
            '2' => { self.peer.resume(&call_id).await.ok(); }
            '3' => { self.peer.hangup(&call_id).await.ok(); }
            _ => { tracing::debug!("Unknown DTMF: {}", digit); }
        }
    }
}

// Note: Peer stores handler, handler stores peer - careful with circular refs!
// Better approach: Use coordinator directly or pass operations via channels
```

### Example 5b: DTMF Commands (Better Pattern)
```rust
#[derive(Debug)]
struct DtmfHandler {
    command_tx: mpsc::Sender<(CallId, char)>,
}

#[async_trait]
impl PeerHandler for DtmfHandler {
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        // Send to command processor
        let _ = self.command_tx.send((call_id, digit)).await;
    }
}

// Separate command processor
async fn process_dtmf_commands(
    mut rx: mpsc::Receiver<(CallId, char)>,
    peer: Arc<CallbackPeer>
) {
    while let Some((call_id, digit)) = rx.recv().await {
        match digit {
            '1' => peer.hold(&call_id).await.ok(),
            '2' => peer.resume(&call_id).await.ok(),
            '3' => peer.hangup(&call_id).await.ok(),
            _ => None,
        };
    }
}
```

---

## Call Operations (Same as PolicyPeer)

```rust
impl CallbackPeer {
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<CallId> {
        self.coordinator.make_call(&self.local_uri, target).await
    }
    
    /// Accept a call (usually called from callback)
    pub async fn accept(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Reject a call (usually called from callback)
    pub async fn reject(&self, call_id: &CallId, reason: &str) -> Result<()> {
        self.coordinator.reject_call(call_id, reason).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Put call on hold
    pub async fn hold(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hold(call_id).await
    }
    
    /// Resume call from hold
    pub async fn resume(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.resume(call_id).await
    }
    
    /// Send REFER
    pub async fn send_refer(&self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, refer_to).await
    }
    
    /// Send audio
    pub async fn send_audio(&self, call_id: &CallId, frame: AudioFrame) -> Result<()> {
        self.coordinator.send_audio(call_id, frame).await
    }
    
    /// Subscribe to audio
    pub async fn subscribe_to_audio(&self, call_id: &CallId) -> Result<AudioFrameSubscriber> {
        self.coordinator.subscribe_to_audio(call_id).await
    }
    
    /// Get event statistics
    pub async fn stats(&self) -> EventStats {
        self.stats.read().await.clone()
    }
}
```

---

## Callback Decisions

```rust
/// Decision about how to handle an incoming call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDecision {
    /// Accept the call
    Accept,
    
    /// Reject the call with 603 Decline
    Reject,
    
    /// Reject with 486 Busy Here
    RejectBusy,
    
    /// Continue to next callback (don't decide yet)
    Continue,
}

/// Decision about how to handle a transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDecision {
    /// Accept and complete the transfer
    Accept,
    
    /// Reject the transfer
    Reject,
}
```

---

## Advantages (Trait-Based)

### 1. **Clean Code**
- No Box::pin boilerplate
- No closure syntax
- Clean async methods
- Just like normal Rust code

### 2. **Familiar to session-core Users**
- Same pattern as session-core's CallHandler
- Easy migration from session-core
- Same trait patterns

### 3. **Type Safe**
- Trait bounds enforced by compiler
- Return types checked
- Default implementations provided

### 4. **Structured**
- One struct implementing trait
- All related logic together
- Easy to test (mock the trait)

### 5. **Composable**
- CompositeHandler chains multiple handlers
- Each handler focused on one concern
- Separation of concerns

---

## Disadvantages (Trait-Based)

### 1. **Must Implement Trait**
Need to create a struct and implement trait:
```rust
#[derive(Debug)]
struct MyHandler;

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        CallDecision::Accept
    }
    // ... other methods
}

let peer = CallbackPeer::new("alice", Arc::new(MyHandler)).await?;
```

vs PolicyPeer:
```rust
let peer = PolicyPeerBuilder::new("alice")
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;
```

**But much cleaner than closure-based callbacks!**

### 2. **Build-Time Handler**
- Handler set at construction
- Can't change at runtime (without rebuilding peer)
- Less flexible than closure registration

### 3. **Default Implementations May Be Overlooked**
- Easy to forget to override a method
- Calls fall through to default (which just logs)
- But safer than closures (at least has a default)

---

## Implementation Size (Trait-Based)

### Code Breakdown
- **PeerHandler trait:** ~80 lines (8 methods with defaults)
- **CallbackPeer struct:** ~20 lines
- **Constructor:** ~40 lines
- **Background processor:** ~120 lines (simpler - just call trait methods)
- **Call operations:** ~80 lines
- **Built-in handlers:** ~120 lines (AutoAnswer, RejectAll, Composite)
- **Helper types:** ~40 lines

**Total: ~500 lines**

### Files
1. ✅ `src/api/callback.rs` - New file (500 lines)
2. ✅ `src/api/mod.rs` - Add export (2 lines)
3. ✅ `examples/callback_demo/main.rs` - Example (100 lines, simpler!)

**Total new code: ~600 lines**

**Comparison to closure-based:**
- Slightly more code (~20 lines)
- Much cleaner user code (no Box::pin)
- Built-in handlers included
- Compatible with session-core patterns

---

## Comparison with PolicyPeer

| Aspect | PolicyPeer | CallbackPeer (Trait) |
|--------|-----------|---------------------|
| **Configuration** | Policies set upfront | Trait implementation |
| **Safe Defaults** | ✅ Yes (Reject all) | ✅ Yes (trait defaults) |
| **Verbosity** | Low (3 policy setters) | Medium (trait impl) |
| **Flexibility** | Medium (3 options) | High (arbitrary logic) |
| **Familiarity** | New pattern | session-core pattern |
| **Type Safety** | ✅ Compile-time | ✅ Compile-time |
| **Runtime Changes** | ❌ No | ❌ No (build-time) |
| **Code Complexity** | Low | Low |
| **Learning Curve** | Learn policies | Know traits |
| **Implementation** | ~380 lines | ~500 lines |
| **User Code Clarity** | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ (cleanest) |
| **Built-in Handlers** | ❌ No | ✅ Yes (3+) |

---

## Testing Strategy

### Unit Tests
```rust
#[tokio::test]
async fn test_incoming_call_callback() {
    let peer = CallbackPeer::new("test").await.unwrap();
    
    let accepted = Arc::new(AtomicBool::new(false));
    let accepted_clone = accepted.clone();
    
    peer.on_incoming_call(move |_| {
        let accepted = accepted_clone.clone();
        Box::pin(async move {
            accepted.store(true, Ordering::SeqCst);
            Ok(CallDecision::Accept)
        })
    }).await;
    
    // Simulate incoming call
    // Verify callback was called
    // Verify call was accepted
}

#[tokio::test]
async fn test_multiple_callbacks_called_in_order() {
    let peer = CallbackPeer::new("test").await.unwrap();
    
    let order = Arc::new(Mutex::new(Vec::new()));
    
    // First callback
    peer.on_incoming_call({
        let order = order.clone();
        move |_| {
            let order = order.clone();
            Box::pin(async move {
                order.lock().await.push(1);
                Ok(CallDecision::Continue)
            })
        }
    }).await;
    
    // Second callback
    peer.on_incoming_call({
        let order = order.clone();
        move |_| {
            let order = order.clone();
            Box::pin(async move {
                order.lock().await.push(2);
                Ok(CallDecision::Accept)
            })
        }
    }).await;
    
    // Verify order is [1, 2]
}
```

---

## Migration from SimplePeer

### Before (SimplePeer)
```rust
let mut peer = SimplePeer::new("alice").await?;

// Make call
let call = peer.call("sip:bob@...").await?;

// Wait for answer
peer.wait_for_answered(&call).await?;

// Wait for transfer
if let Some(refer) = peer.wait_for_refer().await? {
    let new_call = peer.call(&refer.refer_to).await?;
}
```

### After (CallbackPeer with Trait)
```rust
#[derive(Debug)]
struct MyHandler;

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        println!("Transfer to {}", refer.refer_to);
        TransferDecision::Accept
    }
}

let peer = CallbackPeer::new("alice", Arc::new(MyHandler)).await?;

// Make call - transfers handled automatically!
let call = peer.call("sip:bob@...").await?;

// No wait_for_* needed - trait methods called automatically
```

**Much cleaner than closure-based callbacks!**

---

## Advanced Patterns

### Decorator Pattern (Add Logging to Any Handler)

```rust
#[derive(Debug)]
struct LoggingDecorator<H: PeerHandler> {
    inner: Arc<H>,
}

#[async_trait]
impl<H: PeerHandler + 'static> PeerHandler for LoggingDecorator<H> {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        tracing::info!(">>> Incoming call from {}", call.from);
        let decision = self.inner.on_incoming_call(call).await;
        tracing::info!("<<< Decision: {:?}", decision);
        decision
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        tracing::info!(">>> Transfer to {}", refer.refer_to);
        let decision = self.inner.on_transfer(refer).await;
        tracing::info!("<<< Decision: {:?}", decision);
        decision
    }
    
    // Wrap other methods similarly
}

// Usage:
let handler = Arc::new(MyHandler::new());
let logged_handler = Arc::new(LoggingDecorator { inner: handler });
let peer = CallbackPeer::new("alice", logged_handler).await?;
```

### State Machine Handler (Complex Logic)

```rust
#[derive(Debug)]
struct StateMachineHandler {
    state: Arc<RwLock<HandlerState>>,
}

enum HandlerState {
    AcceptingCalls,
    Busy,
    DoNotDisturb,
}

#[async_trait]
impl PeerHandler for StateMachineHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        match *self.state.read().await {
            HandlerState::AcceptingCalls => CallDecision::Accept,
            HandlerState::Busy => CallDecision::RejectBusy,
            HandlerState::DoNotDisturb => CallDecision::Reject,
        }
    }
    
    async fn on_call_answered(&self, _call_id: CallId) {
        // Change state when on a call
        *self.state.write().await = HandlerState::Busy;
    }
    
    async fn on_call_ended(&self, _call_id: CallId, _reason: String) {
        // Back to accepting
        *self.state.write().await = HandlerState::AcceptingCalls;
    }
}
```

---

## Implementation Timeline

### Phase 1: Core Structure (2 hours)
- Define CallbackPeer struct
- Define CallbackRegistry
- Define callback type aliases
- Define helper types (CallDecision, etc.)

### Phase 2: Built-in Handlers (2 hours)
- Implement AutoAnswerHandler
- Implement RejectAllHandler
- Implement CompositeHandler

### Phase 3: Background Processor (2 hours)
- Implement spawn_event_processor()
- Route events to trait methods
- Apply decisions automatically
- (Simpler than closure version - just call trait methods!)

### Phase 4: Call Operations (1 hour)
- Wrap UnifiedCoordinator methods
- Add stats tracking

### Phase 5: Testing (2 hours)
- Unit tests for each callback type
- Integration tests for background processing
- Example program

**Total: 9 hours (~500 lines)**

---

## Pros and Cons (Trait-Based)

### ✅ Advantages
1. **Clean code** - No Box::pin, no closures
2. **Familiar to session-core users** - Same trait pattern
3. **Type safe** - Compiler enforces trait bounds
4. **Structured** - All logic in one struct
5. **Testable** - Easy to mock traits
6. **Built-in handlers** - AutoAnswer, RejectAll, Composite
7. **Default methods** - Safe fallbacks

### ❌ Disadvantages
1. **Must implement trait** - More boilerplate than policies
2. **Build-time only** - Can't change handler at runtime
3. **More code** - 500 lines vs 380 for PolicyPeer
4. **Less flexible** - Can't add/remove handlers dynamically

---

## When to Use CallbackPeer (Trait-Based)

**Good for:**
- ✅ **Session-core users** (same pattern!)
- ✅ Complex event logic (full async methods)
- ✅ Structured applications (trait keeps logic organized)
- ✅ Testing (easy to mock traits)
- ✅ Developers who like OOP patterns

**Not good for:**
- ❌ Simple scripts (PolicyPeer is simpler)
- ❌ Quick prototypes (more boilerplate)
- ❌ Need runtime reconfiguration

---

## Conclusion

**Trait-based CallbackPeer** provides the **cleanest, most structured API** using session-core's proven pattern.

**Tradeoffs vs PolicyPeer:**
- ✅ Cleaner code (no Box::pin)
- ✅ More flexible (arbitrary logic)
- ✅ Familiar to session-core users
- ✅ Built-in handlers included
- ❌ More verbose (trait impl vs 3 policies)
- ❌ More code to maintain (500 vs 380 lines)

**Tradeoffs vs Closure-based:**
- ✅ Much cleaner user code
- ✅ Better structured
- ✅ Built-in handlers
- ❌ Build-time only (vs runtime registration)
- ❌ Slightly more library code

**Best for:** 
- Developers migrating from session-core
- Applications with complex event logic
- Structured/enterprise applications
- Anyone who values code clarity

---

## Next Steps

1. ⏳ Review this plan
2. ⏳ Compare with PolicyPeer plan
3. ⏳ Compare with EventStream plan
4. ⏳ Choose one approach to implement
5. ⏳ Keep SimplePeer as-is for backward compatibility

