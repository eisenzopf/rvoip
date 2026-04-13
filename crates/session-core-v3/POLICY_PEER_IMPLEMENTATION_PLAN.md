# PolicyPeer Implementation Plan

## Executive Summary

Create a **new** policy-based API (`policy.rs`) as an alternative to `simple.rs`, allowing both APIs to coexist:
- **SimplePeer** - Keep as-is for simple, deterministic, test scenarios
- **PolicyPeer** - New policy-based API for production, event-driven scenarios

**Benefits:**
- ✅ No breaking changes to SimplePeer
- ✅ Clean separation of concerns
- ✅ Users choose the API that fits their use case
- ✅ Can deprecate SimplePeer later if desired
- ✅ Both APIs use the same underlying UnifiedCoordinator

---

## File Structure

```
src/api/
├── simple.rs          # Existing - Keep as-is
├── policy.rs          # NEW - Policy-based API
├── unified.rs         # Existing - Shared coordinator
├── events.rs          # Existing - Shared events
├── builder.rs         # Existing - Session builder
├── types.rs           # Existing - Shared types
└── mod.rs             # Update to export PolicyPeer
```

---

## API Comparison

### SimplePeer (Keep Unchanged)
```rust
// Simple, sequential, blocking-style
let mut peer = SimplePeer::new("alice").await?;
let call = peer.call("sip:bob@...").await?;
peer.wait_for_answered(&call).await?;
peer.exchange_audio(&call, duration, generator).await?;
peer.hangup(&call).await?;
```

**Good for:**
- Tests
- Examples  
- Simple scripts
- Deterministic flows

**Problems:**
- Events can be lost
- Transfers require explicit waiting
- No multi-call support

---

### PolicyPeer (New)
```rust
// Policy-based, event-driven, non-blocking
let peer = PolicyPeerBuilder::new("alice")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;

// Events handled automatically in background
let call = peer.call("sip:bob@...").await?;
peer.exchange_audio(&call, duration, generator).await?;
// Transfers handled automatically during audio exchange!
```

**Good for:**
- Production applications
- Call centers
- Softphones with UI
- Multi-call scenarios
- Unpredictable event flows

**Benefits:**
- No lost events
- Automatic transfer handling
- Non-blocking operations
- Multi-call ready

---

## PolicyPeer API Design

### Core Structure

```rust
/// Policy-based SIP peer with background event handling
pub struct PolicyPeer {
    coordinator: Arc<UnifiedCoordinator>,
    policies: Arc<EventPolicies>,
    event_processor_handle: JoinHandle<()>,
    user_handlers: Arc<RwLock<UserHandlers>>,
    
    // For monitoring/debugging
    stats: Arc<RwLock<EventStats>>,
}

/// Event handling policies
#[derive(Debug, Clone)]
pub struct EventPolicies {
    pub incoming_call: IncomingCallPolicy,
    pub transfer: TransferPolicy,
    pub unhandled: UnhandledEventPolicy,
}

/// User-registered event handlers
struct UserHandlers {
    on_incoming_call: Option<Box<dyn Fn(IncomingCallInfo) -> BoxFuture<'static, CallDecision> + Send + Sync>>,
    on_transfer: Option<Box<dyn Fn(ReferRequest) -> BoxFuture<'static, TransferDecision> + Send + Sync>>,
    on_dtmf: Option<Box<dyn Fn(char, CallId) -> BoxFuture<'static, ()> + Send + Sync>>,
}

/// Statistics about event processing
#[derive(Debug, Default, Clone)]
pub struct EventStats {
    pub total_events: u64,
    pub handled_by_user: u64,
    pub handled_by_policy: u64,
    pub unhandled: u64,
    pub errors: u64,
}
```

---

## Policy Enums (Same as Simplified Plan)

### 1. IncomingCallPolicy

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingCallPolicy {
    /// Reject all incoming calls (603 Decline)
    /// ✅ SAFE DEFAULT
    Reject,
    
    /// Accept all incoming calls
    /// ⚠️ Security risk
    Accept,
    
    /// Require manual handling via callback
    /// Error if no callback registered
    RequireManual,
    
    /// Queue for async processing (like session-core's QueueHandler)
    /// Sends incoming calls to channel for background processing
    Queue(mpsc::Sender<IncomingCallInfo>),
    
    /// Route based on patterns (like session-core's RoutingHandler)
    Route {
        rules: Vec<RoutingRule>,
        default_action: Box<IncomingCallPolicy>,
    },
}

/// Routing rule for pattern matching
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingRule {
    pub pattern: String,        // e.g., "support@", "@company.com"
    pub matches: MatchType,     // Contains, StartsWith, EndsWith, Exact
    pub action: IncomingCallAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    Contains,
    StartsWith,
    EndsWith,
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingCallAction {
    Accept,
    Reject,
    RejectBusy,
    Forward(String),
}
```

**Inspired by session-core's built-in handlers!**

### 2. TransferPolicy

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPolicy {
    /// Reject all transfers (603 Decline)
    /// ✅ SAFE DEFAULT
    Reject,
    
    /// Accept and complete blind transfers automatically
    AcceptBlind,
    
    /// Require manual handling via callback
    RequireManual,
    
    /// Accept only from trusted domains (inspired by session-core filtering)
    AcceptTrusted {
        trusted_domains: &'static [&'static str],
    },
}
```

### 3. UnhandledEventPolicy
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnhandledEventPolicy {
    /// Log at WARN level
    /// ✅ SAFE DEFAULT
    Log,
    
    /// Return error (strict mode)
    Error,
    
    /// Ignore silently
    Ignore,
}
```

---

## PolicyPeerBuilder

```rust
pub struct PolicyPeerBuilder {
    name: String,
    config: Config,
    policies: EventPolicies,
}

impl PolicyPeerBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config: Config::default(),
            policies: EventPolicies::default(),
        }
    }
    
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }
    
    pub fn incoming_call_policy(mut self, policy: IncomingCallPolicy) -> Self {
        self.policies.incoming_call = policy;
        self
    }
    
    pub fn transfer_policy(mut self, policy: TransferPolicy) -> Self {
        self.policies.transfer = policy;
        self
    }
    
    pub fn unhandled_event_policy(mut self, policy: UnhandledEventPolicy) -> Self {
        self.policies.unhandled = policy;
        self
    }
    
    pub async fn build(self) -> Result<PolicyPeer> {
        PolicyPeer::with_policies(self.name, self.config, self.policies).await
    }
}

impl Default for EventPolicies {
    fn default() -> Self {
        Self {
            incoming_call: IncomingCallPolicy::Reject,    // Safe
            transfer: TransferPolicy::Reject,              // Safe
            unhandled: UnhandledEventPolicy::Log,          // Safe
        }
    }
}
```

---

## PolicyPeer Implementation

### Constructor

```rust
impl PolicyPeer {
    pub async fn new(name: impl Into<String>) -> Result<Self> {
        PolicyPeerBuilder::new(name).build().await
    }
    
    pub async fn with_policies(
        name: impl Into<String>,
        config: Config,
        policies: EventPolicies,
    ) -> Result<Self> {
        let name = name.into();
        let mut config = config;
        
        // Set local URI if not provided
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        
        // Create coordinator
        let (event_tx, event_rx) = mpsc::channel(1000);
        let coordinator = UnifiedCoordinator::with_simple_peer_events(config, event_tx).await?;
        
        let user_handlers = Arc::new(RwLock::new(UserHandlers::default()));
        let stats = Arc::new(RwLock::new(EventStats::default()));
        
        // Spawn background event processor
        let event_processor_handle = Self::spawn_event_processor(
            event_rx,
            coordinator.clone(),
            policies.clone(),
            user_handlers.clone(),
            stats.clone(),
        );
        
        Ok(Self {
            coordinator,
            policies: Arc::new(policies),
            event_processor_handle,
            user_handlers,
            stats,
        })
    }
}
```

### Background Event Processor

```rust
impl PolicyPeer {
    fn spawn_event_processor(
        mut event_rx: mpsc::Receiver<Event>,
        coordinator: Arc<UnifiedCoordinator>,
        policies: EventPolicies,
        user_handlers: Arc<RwLock<UserHandlers>>,
        stats: Arc<RwLock<EventStats>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // Update stats
                stats.write().await.total_events += 1;
                
                // Try user handler first
                let handled = Self::try_user_handler(&event, &user_handlers, &stats).await;
                
                if !handled {
                    // Fall back to policy-based handler
                    Self::handle_with_policy(
                        event, 
                        &coordinator, 
                        &policies, 
                        &stats
                    ).await;
                }
            }
            
            tracing::info!("Event processor stopped");
        })
    }
    
    async fn try_user_handler(
        event: &Event,
        user_handlers: &Arc<RwLock<UserHandlers>>,
        stats: &Arc<RwLock<EventStats>>,
    ) -> bool {
        let handlers = user_handlers.read().await;
        
        match event {
            Event::IncomingCall { call_id, from, to, .. } => {
                if let Some(handler) = &handlers.on_incoming_call {
                    let info = IncomingCallInfo {
                        call_id: call_id.clone(),
                        from: from.clone(),
                        to: to.clone(),
                    };
                    
                    // Execute user handler
                    handler(info).await;
                    stats.write().await.handled_by_user += 1;
                    return true;
                }
            }
            Event::ReferReceived { call_id, refer_to, transaction_id, transfer_type, .. } => {
                if let Some(handler) = &handlers.on_transfer {
                    let refer = ReferRequest {
                        call_id: call_id.clone(),
                        refer_to: refer_to.clone(),
                        transaction_id: transaction_id.clone(),
                        transfer_type: transfer_type.clone(),
                    };
                    
                    handler(refer).await;
                    stats.write().await.handled_by_user += 1;
                    return true;
                }
            }
            Event::DtmfReceived { call_id, digit, .. } => {
                if let Some(handler) = &handlers.on_dtmf {
                    handler(*digit, call_id.clone()).await;
                    stats.write().await.handled_by_user += 1;
                    return true;
                }
            }
            _ => {}
        }
        
        false
    }
    
    async fn handle_with_policy(
        event: Event,
        coordinator: &Arc<UnifiedCoordinator>,
        policies: &EventPolicies,
        stats: &Arc<RwLock<EventStats>>,
    ) {
        match event {
            Event::IncomingCall { call_id, from, .. } => {
                match policies.incoming_call {
                    IncomingCallPolicy::Reject => {
                        tracing::info!("Auto-rejecting incoming call from {} (policy: Reject)", from);
                        let _ = coordinator.reject_call(&call_id, "Declined").await;
                    }
                    IncomingCallPolicy::Accept => {
                        tracing::info!("Auto-accepting incoming call from {} (policy: Accept)", from);
                        let _ = coordinator.accept_call(&call_id).await;
                    }
                    IncomingCallPolicy::RequireManual => {
                        tracing::error!("Incoming call but no handler (policy: RequireManual)");
                        stats.write().await.errors += 1;
                    }
                }
                stats.write().await.handled_by_policy += 1;
            }
            
            Event::ReferReceived { call_id, refer_to, .. } => {
                match policies.transfer {
                    TransferPolicy::Reject => {
                        tracing::info!("Auto-rejecting transfer to {} (policy: Reject)", refer_to);
                        // Rejection happens automatically in dialog-core
                    }
                    TransferPolicy::AcceptBlind => {
                        tracing::info!("Auto-accepting blind transfer to {} (policy: AcceptBlind)", refer_to);
                        
                        // Complete transfer in background
                        let coordinator = coordinator.clone();
                        let call_id = call_id.clone();
                        let refer_to = refer_to.clone();
                        
                        tokio::spawn(async move {
                            // 1. Hangup current call
                            let _ = coordinator.hangup(&call_id).await;
                            
                            // 2. Wait briefly for cleanup
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            
                            // 3. Call transfer target
                            // Note: Need to get local_uri from somewhere
                            // This is a limitation - might need to store in coordinator
                            tracing::info!("Transfer call would be made to {} here", refer_to);
                        });
                    }
                    TransferPolicy::RequireManual => {
                        tracing::error!("Transfer request but no handler (policy: RequireManual)");
                        stats.write().await.errors += 1;
                    }
                }
                stats.write().await.handled_by_policy += 1;
            }
            
            _ => {
                // Unhandled event
                match policies.unhandled {
                    UnhandledEventPolicy::Log => {
                        tracing::warn!("Unhandled event: {:?}", event);
                    }
                    UnhandledEventPolicy::Error => {
                        tracing::error!("Unhandled event (policy: Error): {:?}", event);
                        stats.write().await.errors += 1;
                    }
                    UnhandledEventPolicy::Ignore => {
                        // Silent
                    }
                }
                stats.write().await.unhandled += 1;
            }
        }
    }
}
```

### User Override Methods

```rust
impl PolicyPeer {
    /// Register callback for incoming calls (overrides policy)
    pub async fn on_incoming_call<F>(&self, handler: F)
    where 
        F: Fn(IncomingCallInfo) -> BoxFuture<'static, CallDecision> + Send + Sync + 'static
    {
        self.user_handlers.write().await.on_incoming_call = Some(Box::new(handler));
    }
    
    /// Register callback for transfers (overrides policy)
    pub async fn on_transfer<F>(&self, handler: F)
    where 
        F: Fn(ReferRequest) -> BoxFuture<'static, TransferDecision> + Send + Sync + 'static
    {
        self.user_handlers.write().await.on_transfer = Some(Box::new(handler));
    }
    
    /// Register callback for DTMF
    pub async fn on_dtmf<F>(&self, handler: F)
    where 
        F: Fn(char, CallId) -> BoxFuture<'static, ()> + Send + Sync + 'static
    {
        self.user_handlers.write().await.on_dtmf = Some(Box::new(handler));
    }
}
```

---

## Call Operations (Same as SimplePeer but Non-Blocking)

```rust
impl PolicyPeer {
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<CallId> {
        // Get local_uri from config (stored during construction)
        self.coordinator.make_call(&self.local_uri, target).await
    }
    
    /// Accept a call
    pub async fn accept(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Reject a call
    pub async fn reject(&self, call_id: &CallId, reason: &str) -> Result<()> {
        self.coordinator.reject_call(call_id, reason).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Send audio frame
    pub async fn send_audio(&self, call_id: &CallId, frame: AudioFrame) -> Result<()> {
        self.coordinator.send_audio(call_id, frame).await
    }
    
    /// Subscribe to audio frames
    pub async fn subscribe_to_audio(&self, call_id: &CallId) -> Result<AudioFrameSubscriber> {
        self.coordinator.subscribe_to_audio(call_id).await
    }
    
    /// Send REFER for transfer
    pub async fn send_refer(&self, call_id: &CallId, refer_to: &str) -> Result<()> {
        self.coordinator.send_refer(call_id, refer_to).await
    }
    
    /// Get event statistics
    pub async fn stats(&self) -> EventStats {
        self.stats.read().await.clone()
    }
}
```

**Notice:** 
- No `&mut self` - concurrent operations supported!
- No `wait_for_*` methods - events handled in background
- Same underlying operations as SimplePeer

---

## Helper Types

```rust
/// Information about an incoming call
#[derive(Debug, Clone)]
pub struct IncomingCallInfo {
    pub call_id: CallId,
    pub from: String,
    pub to: String,
}

/// Decision about how to handle an incoming call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDecision {
    Accept,
    Reject,
    RejectBusy,
    Forward(String),    // NEW: Forward to another destination (session-core has this)
    Defer,              // NEW: Defer to next policy (session-core's pattern)
}

/// Decision about how to handle a transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDecision {
    Accept,
    Reject,
    Defer,              // NEW: Let next handler decide
}

/// Re-export ReferRequest from events
pub use crate::api::events::ReferRequest;  // Already defined in Phase 1
```

---

## Pre-Configured Constructors (Inspired by session-core)

Like session-core's built-in handlers, add convenience constructors:

```rust
impl PolicyPeer {
    /// Create with auto-answer (like session-core's AutoAnswerHandler)
    pub async fn with_auto_answer(name: impl Into<String>) -> Result<Self> {
        PolicyPeerBuilder::new(name)
            .incoming_call_policy(IncomingCallPolicy::Accept)
            .transfer_policy(TransferPolicy::AcceptBlind)
            .build().await
    }
    
    /// Create with reject-all (secure default)
    pub async fn with_reject_all(name: impl Into<String>) -> Result<Self> {
        PolicyPeerBuilder::new(name)
            .incoming_call_policy(IncomingCallPolicy::Reject)
            .transfer_policy(TransferPolicy::Reject)
            .build().await
    }
    
    /// Create with queue (like session-core's QueueHandler)
    pub async fn with_queue(
        name: impl Into<String>,
        queue_size: usize
    ) -> Result<(Self, mpsc::Receiver<IncomingCallInfo>)> {
        let (tx, rx) = mpsc::channel(queue_size);
        
        let peer = PolicyPeerBuilder::new(name)
            .incoming_call_policy(IncomingCallPolicy::Queue(tx))
            .build().await?;
        
        Ok((peer, rx))
    }
    
    /// Create for call center (accept all + auto-transfer)
    pub async fn for_call_center(name: impl Into<String>) -> Result<Self> {
        Self::with_auto_answer(name).await
    }
}
```

---

## Usage Examples

### Example 1: Call Center (Auto-Everything) - SIMPLIFIED
```rust
use rvoip_session_core_v3::api::policy::PolicyPeer;

// ONE LINE! (Like session-core's AutoAnswerHandler)
let peer = PolicyPeer::with_auto_answer("agent").await?;

// That's it! Everything handled automatically
let call = peer.call("sip:customer@...").await?;
```

**Just 2 lines! Simpler than session-core!**

### Example 1b: Call Center (With Builder)
```rust
use rvoip_session_core_v3::api::policy::{
    PolicyPeerBuilder,
    IncomingCallPolicy,
    TransferPolicy,
};

let peer = PolicyPeerBuilder::new("agent")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;

// The peer will:
// - Auto-answer all incoming calls
// - Auto-complete all transfers
// - Handle everything in background

let call = peer.call("sip:customer@...").await?;
```

### Example 2: Queue Pattern (Like session-core's QueueHandler)
```rust
// Create with queue support
let (peer, mut queue_rx) = PolicyPeer::with_queue("agent", 100).await?;

// Process queue in background (like session-core pattern!)
tokio::spawn({
    let peer = peer.clone();
    async move {
        while let Some(call) = queue_rx.recv().await {
            // Async database lookup
            if let Ok(allowed) = check_database(&call.from).await {
                if allowed {
                    peer.accept(&call.call_id).await.ok();
                } else {
                    peer.reject(&call.call_id, "Not authorized").await.ok();
                }
            }
        }
    }
});

// Main code - make calls
let call = peer.call("sip:customer@...").await?;
```

**Exactly like session-core's QueueHandler!**

### Example 3: Routing Pattern (Like session-core's RoutingHandler)
```rust
use rvoip_session_core_v3::api::policy::{
    PolicyPeerBuilder,
    IncomingCallPolicy,
    RoutingRule,
    MatchType,
    IncomingCallAction,
};

let peer = PolicyPeerBuilder::new("pbx")
    .incoming_call_policy(IncomingCallPolicy::Route {
        rules: vec![
            RoutingRule {
                pattern: "support@".to_string(),
                matches: MatchType::Contains,
                action: IncomingCallAction::Forward("sip:queue@support.internal".to_string()),
            },
            RoutingRule {
                pattern: "sales@".to_string(),
                matches: MatchType::Contains,
                action: IncomingCallAction::Forward("sip:queue@sales.internal".to_string()),
            },
            RoutingRule {
                pattern: "@company.com".to_string(),
                matches: MatchType::EndsWith,
                action: IncomingCallAction::Accept,
            },
        ],
        default_action: Box::new(IncomingCallPolicy::Reject),
    })
    .build().await?;
```

**Routing built into policy! Like session-core's RoutingHandler!**

### Example 4: Softphone (Manual Control)
```rust
let peer = PolicyPeerBuilder::new("alice")
    .incoming_call_policy(IncomingCallPolicy::RequireManual)
    .transfer_policy(TransferPolicy::RequireManual)
    .build().await?;

// Register handlers
peer.on_incoming_call(|call_info| Box::pin(async move {
    // Show UI notification
    println!("Incoming call from {}", call_info.from);
    
    // User clicks accept/reject
    if user_clicked_accept() {
        Ok(CallDecision::Accept)
    } else {
        Ok(CallDecision::Reject)
    }
})).await;

peer.on_transfer(|refer| Box::pin(async move {
    // Show UI dialog
    println!("Transfer to {}?", refer.refer_to);
    
    if user_approves() {
        Ok(TransferDecision::Accept)
    } else {
        Ok(TransferDecision::Reject)
    }
})).await;

// Now just use the peer normally
let call = peer.call("sip:bob@...").await?;
```

### Example 5: Secure Application (Reject All)
```rust
// Use helper constructor - just one line!
let peer = PolicyPeer::with_reject_all("secure_app").await?;

// Or use defaults (same thing)
let peer = PolicyPeer::new("secure_app").await?;

// Only outbound calls allowed
let call = peer.call("sip:trusted@...").await?;
```

### Example 6: Trusted Domains Only (NEW - Session-core inspired)
```rust
let peer = PolicyPeerBuilder::new("enterprise")
    .transfer_policy(TransferPolicy::AcceptTrusted {
        trusted_domains: &["company.com", "partner.com"],
    })
    .build().await?;

// Transfers from company.com or partner.com: auto-accepted
// Transfers from other domains: auto-rejected
```

### Example 7: Development/Testing (Strict)
```rust
let peer = PolicyPeerBuilder::new("test")
    .incoming_call_policy(IncomingCallPolicy::RequireManual)
    .transfer_policy(TransferPolicy::RequireManual)
    .unhandled_event_policy(UnhandledEventPolicy::Error)  // Fail fast
    .build().await?;

// Must register handlers or get errors
peer.on_incoming_call(|info| Box::pin(async move {
    Ok(CallDecision::Accept)
})).await;

peer.on_transfer(|refer| Box::pin(async move {
    Ok(TransferDecision::Accept)
})).await;
```

---

## Coexistence Strategy

### Module Exports

```rust
// In src/api/mod.rs
pub mod simple;     // Existing
pub mod policy;     // NEW
pub mod unified;    // Existing
pub mod events;     // Existing
pub mod types;      // Existing
pub mod builder;    // Existing

// Re-exports
pub use simple::SimplePeer;           // Existing
pub use policy::{PolicyPeer, PolicyPeerBuilder};  // NEW
pub use unified::{UnifiedCoordinator, Config};
```

### Documentation

```rust
//! # Session Core v3 API
//!
//! This crate provides two high-level APIs for SIP session management:
//!
//! ## SimplePeer - Sequential API
//!
//! Best for:
//! - Testing and examples
//! - Simple scripts
//! - Deterministic call flows
//! - Learning SIP
//!
//! ```rust
//! use rvoip_session_core_v3::SimplePeer;
//!
//! let mut peer = SimplePeer::new("alice").await?;
//! let call = peer.call("sip:bob@...").await?;
//! peer.wait_for_answered(&call).await?;
//! peer.hangup(&call).await?;
//! ```
//!
//! **Limitations:**
//! - Events can be lost during blocking operations
//! - Transfers require explicit `wait_for_refer()`
//! - Single-call focus
//!
//! ## PolicyPeer - Event-Driven API
//!
//! Best for:
//! - Production applications
//! - Softphones
//! - Call centers
//! - Multi-call scenarios
//! - Unpredictable event flows
//!
//! ```rust
//! use rvoip_session_core_v3::api::policy::{PolicyPeerBuilder, TransferPolicy};
//!
//! let peer = PolicyPeerBuilder::new("alice")
//!     .transfer_policy(TransferPolicy::AcceptBlind)
//!     .build().await?;
//!
//! let call = peer.call("sip:bob@...").await?;
//! // Transfers handled automatically in background!
//! ```
//!
//! **Benefits:**
//! - No lost events (background processor)
//! - Automatic transfer handling
//! - Multi-call support
//! - Production ready
```

---

## Migration Path

### Existing Code (No Changes Required)
```rust
// SimplePeer code continues to work
let mut peer = SimplePeer::new("alice").await?;
// ... existing code ...
```

### New Code (Opt-In to PolicyPeer)
```rust
// New applications can use PolicyPeer
let peer = PolicyPeer::new("alice").await?;
// ... new code ...
```

### Gradual Migration
```rust
// Step 1: Replace SimplePeer with PolicyPeer
// - let mut peer = SimplePeer::new("alice").await?;
+ let peer = PolicyPeer::new("alice").await?;

// Step 2: Remove &mut and wait_for_* calls
// - peer.wait_for_answered(&call).await?;
+ // Handled automatically!

// Step 3: Add handlers for RequireManual policies
+ peer.on_transfer(|refer| Box::pin(async move {
+     Ok(TransferDecision::Accept)
+ })).await;
```

---

## Implementation Breakdown (Enhanced with session-core patterns)

### File 1: `src/api/policy.rs` (~550 lines)

**Sections:**
1. Policy enums (150 lines) - ENHANCED
   - IncomingCallPolicy (5 variants: Reject, Accept, RequireManual, Queue, Route)
   - TransferPolicy (4 variants: Reject, AcceptBlind, RequireManual, AcceptTrusted)
   - UnhandledEventPolicy (3 variants)
   - RoutingRule, MatchType, IncomingCallAction helpers

2. Helper types (80 lines) - ENHANCED
   - IncomingCallInfo
   - CallDecision (5 variants: Accept, Reject, RejectBusy, Forward, Defer)
   - TransferDecision (3 variants: Accept, Reject, Defer)
   - EventStats
   - UserHandlers (internal)

3. PolicyPeerBuilder (50 lines)
   - Constructor
   - Builder methods
   - Default impl

4. PolicyPeer struct and constructors (80 lines) - ENHANCED
   - Basic constructors
   - with_auto_answer() - NEW
   - with_reject_all() - NEW
   - with_queue() - NEW
   - for_call_center() - NEW

5. Background event processor (140 lines) - ENHANCED
   - spawn_event_processor()
   - try_user_handler()
   - handle_with_policy()
   - handle_queue_policy() - NEW
   - handle_route_policy() - NEW
   - handle_trusted_policy() - NEW

6. Call operations (50 lines)
   - call(), accept(), reject(), hangup()
   - send_audio(), subscribe_to_audio()
   - send_refer()
   - stats()

**Total: ~550 lines**

**Growth from original:** +170 lines
**Benefit:** Session-core compatibility + built-in patterns

### File 2: Update `src/api/mod.rs` (~5 lines)
```rust
pub mod policy;  // NEW

pub use policy::{PolicyPeer, PolicyPeerBuilder};  // NEW
```

### File 3: Example `examples/policy_peer_demo/main.rs` (~100 lines)
Demonstrate all 3 policies with different configurations.

---

## Testing Strategy

### Unit Tests
```rust
#[tokio::test]
async fn test_incoming_call_policy_reject() {
    let peer = PolicyPeerBuilder::new("test")
        .incoming_call_policy(IncomingCallPolicy::Reject)
        .build().await.unwrap();
    
    // Simulate incoming call
    // Verify 603 Decline sent
}

#[tokio::test]
async fn test_transfer_policy_accept_blind() {
    let peer = PolicyPeerBuilder::new("test")
        .transfer_policy(TransferPolicy::AcceptBlind)
        .build().await.unwrap();
    
    // Simulate REFER
    // Verify transfer completed
}

#[tokio::test]
async fn test_user_handler_overrides_policy() {
    let peer = PolicyPeerBuilder::new("test")
        .incoming_call_policy(IncomingCallPolicy::Reject)
        .build().await.unwrap();
    
    // Register handler (overrides Reject policy)
    peer.on_incoming_call(|_| Box::pin(async { 
        Ok(CallDecision::Accept) 
    })).await;
    
    // Simulate incoming call
    // Verify call ACCEPTED (handler wins over policy)
}
```

### Integration Tests
```rust
#[tokio::test]
async fn test_background_processor_handles_events_during_audio() {
    let peer = PolicyPeerBuilder::new("test")
        .transfer_policy(TransferPolicy::AcceptBlind)
        .build().await.unwrap();
    
    let call = peer.call("sip:target@...").await.unwrap();
    
    // Exchange audio (blocking operation)
    tokio::spawn(async move {
        peer.exchange_audio(&call, Duration::from_secs(10), generator).await
    });
    
    // Simulate REFER during audio
    // Verify transfer completed even though exchange_audio is running
}
```

---

## Comparison Table

| Feature | SimplePeer | PolicyPeer |
|---------|-----------|-----------|
| **API Style** | Sequential, blocking | Event-driven, non-blocking |
| **Mutability** | `&mut self` required | `&self` only |
| **Event Handling** | Manual with wait_for_* | Automatic via policies |
| **Background Processing** | ❌ No | ✅ Yes |
| **Lost Events** | ⚠️ Possible | ✅ Never |
| **Transfer Support** | ⚠️ Explicit wait | ✅ Automatic |
| **Multi-Call** | ⚠️ Limited | ✅ Full support |
| **Concurrent Ops** | ❌ No | ✅ Yes |
| **Configuration** | Simple (2 params) | Policies (3 params) |
| **Code Complexity** | Low | Medium |
| **Use Case** | Tests, examples | Production |
| **Breaking Changes** | None | N/A (new API) |

---

## Implementation Timeline (Enhanced)

### Phase 1: Core Structure (3 hours) - ENHANCED
- Create `src/api/policy.rs`
- Define policy enums (3 types, but more variants)
  - IncomingCallPolicy (5 variants: includes Queue, Route)
  - TransferPolicy (4 variants: includes AcceptTrusted)
  - UnhandledEventPolicy (3 variants)
- Define helper types (CallDecision with Forward/Defer, etc.)
- Define RoutingRule and related types

### Phase 2: Background Processor (4 hours) - ENHANCED
- Implement `spawn_event_processor()`
- Implement `try_user_handler()`
- Implement `handle_with_policy()` for basic policies
- Implement `handle_queue_policy()` - NEW
- Implement `handle_route_policy()` - NEW
- Implement `handle_trusted_policy()` - NEW
- Handle Forward and Defer decisions

### Phase 3: Pre-Configured Constructors (1 hour) - NEW
- with_auto_answer() - Like session-core's AutoAnswerHandler
- with_reject_all() - Secure default
- with_queue() - Like session-core's QueueHandler
- for_call_center() - Common pattern

### Phase 4: Call Operations (1 hour)
- Implement call operations (call, accept, reject, hangup)
- Implement audio operations (send_audio, subscribe_to_audio)
- Implement stats()

### Phase 5: Testing (2 hours)
- Unit tests for each policy
- Unit tests for Queue and Route policies
- Integration tests for background processing
- Example program

**Total: 11 hours (~550 lines)**

**Worth it?** Yes! +3 hours adds session-core's best patterns

---

## File Changes Required (Enhanced)

### New Files
1. ✅ `src/api/policy.rs` - PolicyPeer implementation (550 lines)
2. ✅ `examples/policy_peer_demo/main.rs` - Example usage (120 lines)
   - Demo of auto_answer
   - Demo of queue pattern
   - Demo of routing pattern
   - Demo of manual handlers

### Modified Files
1. ✅ `src/api/mod.rs` - Add policy module export (2 lines)
2. ✅ `src/lib.rs` - Re-export PolicyPeer (1 line)

### Unchanged Files
- ✅ `src/api/simple.rs` - **NO CHANGES**
- ✅ `src/api/unified.rs` - **NO CHANGES**
- ✅ All existing examples - **NO CHANGES**

**Total new code: ~670 lines** (+190 from original)
**Changed existing code: ~3 lines**
**Risk of breaking existing code: Near zero**

**What the +190 lines buys:**
- ✅ Queue policy (like session-core's QueueHandler)
- ✅ Routing policy (like session-core's RoutingHandler)
- ✅ Trusted domains filtering
- ✅ Forward and Defer decisions
- ✅ Pre-configured constructors (with_auto_answer, etc.)
- ✅ Session-core compatibility and familiarity

---

## Advantages of This Approach (Enhanced)

### 1. **Zero Breaking Changes**
- SimplePeer untouched
- Existing examples continue to work
- Existing tests continue to pass

### 2. **Clean Separation**
- SimplePeer = simple, sequential
- PolicyPeer = complex, event-driven
- Each API is internally consistent

### 3. **User Choice**
- Developers pick the API that fits their needs
- Can use both in same application (different peers)
- Clear upgrade path

### 4. **Session-Core Compatibility** - NEW
- Queue pattern (like QueueHandler)
- Routing pattern (like RoutingHandler)
- Pre-configured constructors (like AutoAnswerHandler)
- Defer pattern (like CallDecision::Defer)
- Easy migration from session-core

### 5. **Maintainability**
- Two focused APIs vs one confused API
- Easier to document (two separate docs)
- Easier to test (separate test suites)
- Proven patterns from session-core

### 6. **Future Flexibility**
- Can deprecate SimplePeer later if desired
- Can add PolicyPeer v2 without affecting SimplePeer
- Can add more policies incrementally
- Can add CompositePolicy (chain policies like session-core's CompositeHandler)

---

## Rollout Strategy

### Phase 1: Soft Launch
1. Implement PolicyPeer
2. Document in separate README
3. Add example programs
4. Mark as "experimental" initially

### Phase 2: Validation
1. Use in internal projects
2. Gather feedback
3. Fix issues
4. Add missing policies if needed

### Phase 3: Promote
1. Mark as "stable"
2. Update main README to recommend PolicyPeer for production
3. Note SimplePeer limitations clearly

### Phase 4: Deprecation (Optional, Later)
1. Mark SimplePeer as deprecated
2. Provide migration guide
3. Keep SimplePeer for backward compatibility

---

## Open Questions

1. ✅ Should PolicyPeer store local_uri? (Yes, for call())
2. ✅ Should stats be queryable? (Yes, for monitoring)
3. ⏳ Should we support policy changes at runtime? (Probably not - rebuild peer)
4. ⏳ Should we support per-call policies? (No - one policy per peer)
5. ⏳ Should event history be queryable? (Later, if needed)

---

## Success Criteria

### Implementation Complete When:
- ✅ PolicyPeer API fully implemented
- ✅ All 3 policies working
- ✅ Background processor prevents lost events
- ✅ User handlers override policies
- ✅ Unit tests pass
- ✅ Integration tests pass
- ✅ Example program works
- ✅ No breaking changes to SimplePeer

### Production Ready When:
- ✅ Used in real application
- ✅ Handles transfers correctly
- ✅ No memory leaks
- ✅ No deadlocks
- ✅ Documented

---

## Conclusion (Enhanced)

Creating `policy.rs` as a **separate API with session-core patterns** is the best approach because:

1. ✅ **No risk** - SimplePeer untouched
2. ✅ **Clean design** - Each API does one thing well
3. ✅ **Reasonable implementation** - 550 lines in new file
4. ✅ **Session-core compatible** - Proven patterns from production library
5. ✅ **User choice** - Pick the right tool for the job
6. ✅ **Maintainable** - Clear separation of concerns
7. ✅ **Feature-rich** - Queue, Routing, Trusted domains built-in

**Key Improvements from session-core:**
- ✅ Queue policy (QueueHandler equivalent)
- ✅ Routing policy (RoutingHandler equivalent)  
- ✅ Pre-configured constructors (AutoAnswerHandler equivalent)
- ✅ Forward and Defer decisions
- ✅ Trusted domain filtering

**Comparison:**
- Original plan: 380 lines, basic policies
- Enhanced plan: 550 lines, session-core patterns
- **+170 lines = session-core compatibility**

**Recommendation: Proceed with enhanced approach!**

The implementation is still straightforward, low-risk, and now includes proven patterns from session-core that users already know and trust.

---

## Session-Core Migration Path

### From session-core to PolicyPeer

```rust
// Session-core
let coordinator = SessionManagerBuilder::new()
    .with_handler(Arc::new(AutoAnswerHandler))
    .build().await?;

// PolicyPeer equivalent
let peer = PolicyPeer::with_auto_answer("agent").await?;
```

```rust
// Session-core with QueueHandler
let queue = Arc::new(QueueHandler::new(100));
queue.set_notify_channel(tx);
let coordinator = SessionManagerBuilder::new()
    .with_handler(queue)
    .build().await?;

// PolicyPeer equivalent
let (peer, queue_rx) = PolicyPeer::with_queue("agent", 100).await?;
```

```rust
// Session-core with RoutingHandler
let mut router = RoutingHandler::new();
router.add_route("support@", "sip:queue@support");
let coordinator = SessionManagerBuilder::new()
    .with_handler(Arc::new(router))
    .build().await?;

// PolicyPeer equivalent
let peer = PolicyPeerBuilder::new("pbx")
    .incoming_call_policy(IncomingCallPolicy::Route {
        rules: vec![
            RoutingRule {
                pattern: "support@".to_string(),
                matches: MatchType::Contains,
                action: IncomingCallAction::Forward("sip:queue@support".to_string()),
            },
        ],
        default_action: Box::new(IncomingCallPolicy::Reject),
    })
    .build().await?;
```

**PolicyPeer provides session-core's patterns as first-class policies!**

---

## Next Steps

1. ✅ Review and approve this enhanced plan
2. ⏳ Implement `src/api/policy.rs` (~11 hours)
3. ⏳ Test with blind_transfer example
4. ⏳ Test Queue and Routing policies
5. ⏳ Document in main README
6. ⏳ Create migration guide from session-core
7. ⏳ Consider SimplePeer future (keep, deprecate, or enhance)

