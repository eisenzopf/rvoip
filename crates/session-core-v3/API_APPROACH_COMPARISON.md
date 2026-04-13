# Session-Core-v3 API Approach Comparison (Updated)

## Overview

This document compares **four different API approaches** for session-core-v3, all enhanced with patterns from **session-core**:

1. **SimplePeer** (Existing) - Sequential, blocking-style
2. **PolicyPeer** (New, Enhanced) - Policy-based with session-core patterns (Queue, Routing)
3. **CallbackPeer** (New, Trait-Based) - Trait-based callbacks like session-core's CallHandler
4. **EventStreamPeer** (New, With Helpers) - Stream-based with helper methods

---

## Quick Comparison Table (Updated)

| Aspect | SimplePeer | PolicyPeer (Enhanced) | CallbackPeer (Trait) | EventStreamPeer (With Helpers) |
|--------|-----------|----------------------|---------------------|-------------------------------|
| **Status** | ✅ Exists | 📋 Planned | 📋 Planned | 📋 Planned |
| **Code Size** | 200 lines | 550 lines | 500 lines | 620 lines |
| **Implementation Time** | N/A | 11 hours | 9 hours | 13 hours |
| **Learning Curve** | Very Low | Low | Low | Low→High |
| **Safe Defaults** | ❌ No | ✅ Yes | ✅ Yes (trait) | ✅ Yes (helpers) |
| **Session-Core Compat** | ❌ No | ✅ Queue/Route | ✅ Full (traits) | ⚠️ Partial |
| **Lost Events** | ⚠️ Yes | ✅ No | ✅ No | ✅ No |
| **Verbosity (Simple)** | Low | Low | Medium | Low |
| **Verbosity (Advanced)** | N/A | N/A | Medium | High |
| **Flexibility** | Low | High | Very High | Very High |
| **Multi-Call** | ⚠️ Limited | ✅ Yes | ✅ Yes | ✅ Yes (best) |
| **Async Transfers** | ❌ No | ✅ Yes | ✅ Yes | ✅ Yes |
| **Best For** | Tests | Production | session-core users | Power users |
| **Maintainability** | High | High | High | Medium |

---

## Side-by-Side Code Comparison

### Scenario: Handle Incoming Call + Transfer

#### SimplePeer (Existing)
```rust
let mut peer = SimplePeer::new("alice").await?;

// Wait for incoming call
let (call_id, from) = peer.wait_for_incoming_call().await?;
println!("Call from {}", from);
peer.accept(&call_id).await?;

// Talk
peer.exchange_audio(&call_id, duration, generator).await?;

// Wait for transfer
if let Some(refer) = peer.wait_for_refer().await? {
    let new_call = peer.call(&refer.refer_to).await?;
    // ...
}
```

**Lines of code:** ~10  
**Pros:** Very simple, easy to read  
**Cons:** Blocks on waits, events can be lost during exchange_audio

---

#### PolicyPeer (Enhanced with session-core patterns)
```rust
// SIMPLEST: One-line constructor (like session-core's AutoAnswerHandler)
let peer = PolicyPeer::with_auto_answer("alice").await?;

// Or with builder for custom config
let peer = PolicyPeerBuilder::new("alice")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;

// That's it! Everything handled automatically in background
let call = peer.call("sip:bob@...").await?;
```

**Lines of code:** ~2 (with helper) or ~6 (with builder)  
**Pros:** Shortest, safest defaults, session-core patterns (Queue/Route), automatic  
**Cons:** Less control for advanced cases (but has Queue and Route policies!)

---

#### CallbackPeer (Trait-Based, like session-core)
```rust
// Use built-in handler (like session-core's AutoAnswerHandler)
let peer = CallbackPeer::new("alice", Arc::new(AutoAnswerHandler)).await?;

// Or implement custom trait (clean, no Box::pin!)
#[derive(Debug)]
struct MyHandler;

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        println!("Call from {}", call.from);
        CallDecision::Accept
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        println!("Transfer to {}", refer.refer_to);
        TransferDecision::Accept
    }
}

let peer = CallbackPeer::new("alice", Arc::new(MyHandler)).await?;
let call = peer.call("sip:bob@...").await?;
```

**Lines of code:** ~2 (built-in) or ~15 (custom trait)  
**Pros:** Clean trait syntax, familiar to session-core users, built-in handlers, composable  
**Cons:** Must implement trait (but cleaner than closures!)

---

#### EventStreamPeer (With Helpers - NEW!)
```rust
// SIMPLEST: Use helper constructor (like session-core's AutoAnswerHandler)
let peer = EventStreamPeer::with_auto_answer("alice").await?;

// Or with builder (no manual spawning!)
let peer = EventStreamPeerBuilder::new("alice")
    .auto_accept_calls()
    .auto_accept_transfers()
    .build().await?;

// Or ADVANCED: Manual stream control
let peer = EventStreamPeer::new("alice").await?;

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            peer.accept(&call.call_id).await.unwrap();
        }
    }
});

// Main code
let call = peer.call("sip:bob@...").await?;
```

**Lines of code:** ~2 (with helper), ~6 (with builder), ~22 (manual streams)  
**Pros:** Simple for common cases, extremely powerful for advanced cases, composable streams  
**Cons:** Most code to maintain (620 lines), complex for advanced features

---

## Detailed Comparison

### 1. Configuration/Setup (Updated)

#### SimplePeer
```rust
let mut peer = SimplePeer::new("alice").await?;
```
**Complexity:** ⭐ (simplest)

#### PolicyPeer (Enhanced)
```rust
// Simplest: Helper constructor
let peer = PolicyPeer::with_auto_answer("alice").await?;

// Or builder
let peer = PolicyPeerBuilder::new("alice")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;
```
**Complexity:** ⭐ (helper) or ⭐⭐ (builder)

#### CallbackPeer (Trait-Based)
```rust
// With built-in handler
let peer = CallbackPeer::new("alice", Arc::new(AutoAnswerHandler)).await?;

// Or implement trait
#[derive(Debug)]
struct MyHandler;

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        CallDecision::Accept
    }
}

let peer = CallbackPeer::new("alice", Arc::new(MyHandler)).await?;
```
**Complexity:** ⭐⭐ (built-in) or ⭐⭐⭐ (custom trait)

#### EventStreamPeer (With Helpers)
```rust
// Simplest: Helper constructor
let peer = EventStreamPeer::with_auto_answer("alice").await?;

// Or builder
let peer = EventStreamPeerBuilder::new("alice")
    .auto_accept_calls()
    .auto_accept_transfers()
    .build().await?;

// Advanced: Manual streams
let peer = EventStreamPeer::new("alice").await?;
tokio::spawn({ ... }); // Manual control
```
**Complexity:** ⭐ (helper), ⭐⭐ (builder), or ⭐⭐⭐⭐ (manual)

---

### 2. Handling Incoming Calls

#### SimplePeer
```rust
let (call_id, from) = peer.wait_for_incoming_call().await?;
peer.accept(&call_id).await?;
```
**Pros:** Simple  
**Cons:** Blocks, misses calls during other operations

#### PolicyPeer (Enhanced)
```rust
// Simple: Use helper
let peer = PolicyPeer::with_auto_answer("alice").await?;

// Queue (like session-core's QueueHandler)
let (peer, queue_rx) = PolicyPeer::with_queue("alice", 100).await?;

// Route (like session-core's RoutingHandler)
.incoming_call_policy(IncomingCallPolicy::Route {
    rules: vec![RoutingRule { pattern: "support@", ... }],
    default_action: Box::new(IncomingCallPolicy::Reject),
})

// Manual:
.incoming_call_policy(IncomingCallPolicy::RequireManual)
peer.on_incoming_call(|call| Box::pin(async move {
    Ok(CallDecision::Accept)
})).await;
```
**Pros:** Automatic, session-core patterns built-in, Queue/Route policies  
**Cons:** Routing config can be verbose

#### CallbackPeer (Trait-Based)
```rust
// Use built-in handler
let peer = CallbackPeer::new("alice", Arc::new(AutoAnswerHandler)).await?;

// Or custom trait (clean, no Box::pin!)
#[derive(Debug)]
struct MyHandler;

#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        if should_accept(&call) {
            CallDecision::Accept
        } else {
            CallDecision::Reject
        }
    }
}

let peer = CallbackPeer::new("alice", Arc::new(MyHandler)).await?;
```
**Pros:** Clean trait syntax, built-in handlers, familiar to session-core users  
**Cons:** Must implement trait (but no Box::pin!)

#### EventStreamPeer (With Helpers)
```rust
// Simple: Use helper
let peer = EventStreamPeer::with_auto_answer("alice").await?;

// Or builder with auto-spawn
let peer = EventStreamPeerBuilder::new("alice")
    .auto_accept_calls()
    .build().await?;

// Advanced: Manual stream control
let mut calls = peer.incoming_calls();
while let Some(call) = calls.next().await {
    if should_accept(&call) {
        peer.accept(&call.call_id).await?;
    } else {
        peer.reject(&call.call_id, "Declined").await?;
    }
}
```
**Pros:** Simple helpers + full stream power, composable  
**Cons:** Manual streams complex for beginners (but helpers solve this!)

---

### 3. Handling Transfers (Updated)

#### SimplePeer
```rust
if let Some(refer) = peer.wait_for_refer().await? {
    let new_call = peer.call(&refer.refer_to).await?;
}
```
**Pros:** Explicit  
**Cons:** Must know to call wait_for_refer(), blocks

#### PolicyPeer (Enhanced)
```rust
// Simple
.transfer_policy(TransferPolicy::AcceptBlind)  // Automatic!

// Or trusted domains only
.transfer_policy(TransferPolicy::AcceptTrusted {
    trusted_domains: &["company.com", "partner.com"],
})
```
**Pros:** One line, automatic, trusted domain filtering  
**Cons:** Can't customize per-transfer (but has trusted domains now!)

#### CallbackPeer (Trait-Based)
```rust
#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        if refer.refer_to.contains("trusted.com") {
            TransferDecision::Accept
        } else {
            TransferDecision::Reject
        }
    }
}
```
**Pros:** Clean trait method, custom logic per transfer  
**Cons:** Must implement trait

#### EventStreamPeer (With Helpers)
```rust
// Simple: Helper
peer.auto_accept_transfers();  // One line!

// Advanced: Manual stream
let mut transfers = peer.transfers();
while let Some(refer) = transfers.next().await {
    peer.hangup(&refer.call_id).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    peer.call(&refer.refer_to).await?;
}
```
**Pros:** Simple helpers OR full control  
**Cons:** Manual streams require orchestration

---

### 4. Multi-Call Support (Updated)

#### SimplePeer
```rust
// Limited - must track manually
let call1 = peer.call("sip:bob@...").await?;
let call2 = peer.call("sip:charlie@...").await?;
// Events from both calls mixed in one channel
```
**Rating:** ⭐⭐ (Limited)

#### PolicyPeer (Enhanced)
```rust
// Supported - policies apply to all calls
let call1 = peer.call("sip:bob@...").await?;
let call2 = peer.call("sip:charlie@...").await?;

// Queue policy handles all calls
let (peer, queue) = PolicyPeer::with_queue("agent", 100).await?;
```
**Rating:** ⭐⭐⭐⭐ (Good)

#### CallbackPeer (Trait-Based)
```rust
// Trait methods receive call_id for each call
#[async_trait]
impl PeerHandler for MyHandler {
    async fn on_call_answered(&self, call_id: CallId) {
        // Handle any call
    }
}

let call1 = peer.call("sip:bob@...").await?;
let call2 = peer.call("sip:charlie@...").await?;
```
**Rating:** ⭐⭐⭐⭐ (Good)

#### EventStreamPeer (With Helpers)
```rust
// Best - can filter by call_id
let call1 = peer.call("sip:bob@...").await?;
let call2 = peer.call("sip:charlie@...").await?;

let mut call1_events = peer.events_for_call(call1);
let mut call2_events = peer.events_for_call(call2);
// Completely isolated streams!
```
**Rating:** ⭐⭐⭐⭐⭐ (Excellent - best per-call isolation)

---

## Use Case Recommendations

### Test Automation
**Recommendation:** SimplePeer  
**Why:** Deterministic, simple, good enough for tests

### Simple Scripts/Tools
**Recommendation:** SimplePeer or PolicyPeer  
**Why:** Minimal setup, safe defaults

### Call Center Agent
**Recommendation:** PolicyPeer  
**Why:** Auto-accept, auto-transfer, simple config
```rust
PolicyPeerBuilder::new("agent")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;
```

### Softphone with UI
**Recommendation:** CallbackPeer  
**Why:** Callbacks map nicely to UI events
```rust
peer.on_incoming_call(|call| Box::pin(async move {
    show_incoming_call_dialog(call);
    Ok(user_decision())
})).await;
```

### Advanced Call Processing
**Recommendation:** EventStreamPeer  
**Why:** Complex pipelines, filtering, composition
```rust
peer.incoming_calls()
    .filter(|call| is_vip(call))
    .map(|call| prioritize(call))
    .for_each(|call| handle(call))
    .await;
```

### B2BUA/Gateway
**Recommendation:** EventStreamPeer or CallbackPeer  
**Why:** Multi-call coordination, per-call event isolation

### IVR System
**Recommendation:** CallbackPeer or PolicyPeer  
**Why:** Event-driven, DTMF handling
```rust
// PolicyPeer
PolicyPeerBuilder::new("ivr")
    .incoming_call_policy(IncomingCallPolicy::Accept)
    .build().await?;

// Or CallbackPeer
peer.on_dtmf(|call, digit| Box::pin(async move {
    ivr_state_machine(call, digit);
})).await;
```

---

## Implementation Recommendation (Updated)

### Option 1: Implement All Three ⭐⭐⭐ STILL RECOMMENDED
- PolicyPeer for configuration-driven apps + session-core patterns
- CallbackPeer for session-core migration + trait-based apps
- EventStreamPeer for simple (helpers) + advanced (streams) apps
- Keep SimplePeer for tests

**Rationale:**
- Each API serves different use cases
- All include session-core's best patterns
- Total code: ~1670 lines (550+500+620)
- All share the same UnifiedCoordinator
- Session-core users have migration paths

**Timeline:** 33 hours total (11+9+13) - about 4-5 days

**Growth from original:** +370 lines, +7 hours
**Benefit:** Session-core compatibility, helper methods, built-in patterns

---

### Option 2: Implement One ⭐⭐
Choose based on your primary use case:

**If migrating from session-core:** CallbackPeer (9 hours) - trait-based, most compatible  
**If building call center:** PolicyPeer (11 hours) - has Queue and Route policies  
**If want simple + powerful:** EventStreamPeer (13 hours) - helpers for simple, streams for advanced  

Can add others later if needed.

---

### Option 3: Hybrid Approach ⭐⭐⭐
Implement PolicyPeer + expose event streams:

```rust
impl PolicyPeer {
    // Policy-based defaults
    pub async fn new(...) -> Self { ... }
    
    // But also expose streams for advanced users
    pub fn event_stream(&self) -> impl Stream<Item = Event> { ... }
    
    // And allow callback overrides
    pub async fn on_transfer<F>(&self, callback: F) { ... }
}
```

**Best of all worlds:**
- Safe defaults from policies
- Callbacks for overrides
- Streams for monitoring

**Code:** ~500 lines  
**Time:** 12 hours

---

## Decision Matrix

### Choose SimplePeer if:
- ✅ Writing tests or examples
- ✅ Simple, deterministic flows
- ✅ Don't care about lost events
- ✅ Learning SIP basics
- ❌ Production applications
- ❌ Need async event handling

**Verdict:** Keep for backward compatibility, not for new features

---

### Choose PolicyPeer if:
- ✅ Want safe defaults
- ✅ Simple configuration (3 settings)
- ✅ Call center or automated systems
- ✅ Don't need per-event customization
- ✅ Want shortest code
- ❌ Need runtime reconfiguration
- ❌ Need per-call different behavior

**Verdict:** Best for most production applications

---

### Choose CallbackPeer (Trait-Based) if:
- ✅ **Migrating from session-core** (same trait pattern!)
- ✅ Building UI application
- ✅ Need per-event custom logic
- ✅ Want clean, structured code
- ✅ Like OOP/trait patterns
- ✅ Want built-in handlers (AutoAnswer, RejectAll, Composite)
- ❌ Need runtime handler changes

**Verdict:** Best for session-core users and structured applications

---

### Choose EventStreamPeer (With Helpers) if:
- ✅ **Anyone!** (helpers make it beginner-friendly)
- ✅ Simple apps (use `with_auto_answer()`)
- ✅ Complex event processing (use manual streams)
- ✅ Want composable pipelines
- ✅ Multi-call with per-call isolation (best!)
- ✅ Reactive programming mindset
- ✅ Want both simple AND powerful

**Verdict:** Best for everyone - simple for beginners, powerful for experts

---

## Feature Comparison (Updated)

### Safe Defaults

| Feature | SimplePeer | PolicyPeer (Enhanced) | CallbackPeer (Trait) | EventStreamPeer (Helpers) |
|---------|-----------|---------------------|---------------------|--------------------------|
| Incoming Calls | ❌ Must handle | ✅ Reject default | ✅ Trait defaults | ✅ Via helpers |
| Transfers | ❌ Must wait | ✅ Reject default | ✅ Trait defaults | ✅ Via helpers |
| DTMF | ❌ Ignored | ✅ Logged | ✅ Trait defaults | ✅ Via helpers |
| Unhandled Events | ❌ Lost | ✅ Logged | ✅ Logged | ✅ Via helpers |
| Built-in Patterns | ❌ No | ✅ Queue/Route | ✅ AutoAnswer/Composite | ✅ with_auto_answer() |

**Winner:** All three new approaches have safe defaults now!

---

### Flexibility (Updated)

| Feature | SimplePeer | PolicyPeer (Enhanced) | CallbackPeer (Trait) | EventStreamPeer (Helpers) |
|---------|-----------|---------------------|---------------------|--------------------------|
| Per-Event Logic | ❌ No | ✅ Yes (Route policy) | ✅ Yes (trait methods) | ✅ Yes (streams) |
| Runtime Changes | ❌ No | ❌ No | ❌ No (build-time) | ✅ Yes (can spawn more) |
| Filtering | ❌ No | ✅ Route policy! | ✅ In trait | ✅ Stream ops (best) |
| Composition | ❌ No | ⚠️ Limited | ✅ CompositeHandler | ✅ Excellent |
| Per-Call Isolation | ❌ No | ❌ No | ⚠️ Via call_id | ✅ Via streams (best) |
| Built-in Patterns | ❌ No | ✅ Queue/Route | ✅ AutoAnswer/Composite | ✅ Helpers |

**Winner:** EventStreamPeer (most flexible for advanced), PolicyPeer (best built-in patterns)

---

### Developer Experience

| Aspect | SimplePeer | PolicyPeer | CallbackPeer | EventStreamPeer |
|--------|-----------|-----------|--------------|-----------------|
| Time to "Hello World" | 2 min | 3 min | 5 min | 10 min |
| Code Readability | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ |
| Debugging Ease | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐ |
| Documentation Needed | Low | Medium | Medium | High |
| Community Familiarity | High | Medium | High | Medium |

**Winner:** SimplePeer (but doesn't work for production)

---

### Production Readiness

| Aspect | SimplePeer | PolicyPeer | CallbackPeer | EventStreamPeer |
|--------|-----------|-----------|--------------|-----------------|
| Lost Events | ❌ Yes | ✅ No | ✅ No | ✅ No |
| Buffer Overflow | ⚠️ Possible | ✅ Handled | ✅ Handled | ✅ Handled |
| Security | ❌ No defaults | ✅ Safe defaults | ❌ No defaults | ❌ No defaults |
| Error Handling | ⚠️ Basic | ✅ Comprehensive | ✅ Good | ✅ Good |
| Monitoring | ❌ No | ✅ Stats | ✅ Stats | ✅ Stats |
| Multi-Call | ⚠️ Limited | ✅ Yes | ✅ Yes | ✅ Yes |

**Winner:** PolicyPeer (most production-ready out of box)

---

### Code Maintainability (Updated)

| Aspect | SimplePeer | PolicyPeer (Enhanced) | CallbackPeer (Trait) | EventStreamPeer (Helpers) |
|--------|-----------|---------------------|---------------------|--------------------------|
| Lines of Code | 200 | 550 | 500 | 620 |
| Complexity | Low | Medium | Low | Medium |
| Dependencies | Few | Few | Few (async-trait) | Medium (tokio-stream) |
| Test Coverage | Easy | Easy | Easy | Medium |
| Future Additions | Hard | Easy | Easy | Easy |
| Session-Core Compat | ❌ | ✅ Queue/Route | ✅ Full (traits) | ⚠️ Partial |

**Winner:** CallbackPeer (best compatibility), PolicyPeer (best features/complexity ratio)

---

## Recommendation by User Profile (Updated)

### Beginner VoIP Developer
**Use:** SimplePeer → PolicyPeer or EventStreamPeer  
**Path:** Start with SimplePeer for learning, then use helpers (`with_auto_answer()`)

### Migrating from session-core
**Use:** CallbackPeer (trait-based)  
**Why:** Same trait pattern, has AutoAnswerHandler/QueueHandler equivalents, minimal changes

### Experienced Rust Developer
**Use:** EventStreamPeer  
**Why:** Stream power for complex cases, helpers for simple cases, best of both worlds

### Enterprise Developer
**Use:** PolicyPeer (Enhanced)  
**Why:** Safe defaults, Queue/Route policies, simple config, compliance-friendly

### Call Center / Production
**Use:** PolicyPeer (Enhanced) or CallbackPeer  
**Why:** Queue policy, proven patterns, session-core compatibility

### Indie Developer / Startup
**Use:** PolicyPeer or EventStreamPeer (with helpers)  
**Why:** Fastest to production with one-line constructors, safe, maintainable

---

## Implementation Strategy

### Recommended: Implement PolicyPeer First

**Why:**
1. Solves the core problem (lost events)
2. Smallest code (380 lines)
3. Safe defaults (most important)
4. Fastest to implement (8 hours)
5. Good for most use cases

**Then Add (Optional):**
- CallbackPeer if UI developers request it
- EventStreamPeer if advanced users request it

### Alternative: Implement All Three

**Why:**
1. Serves all user types
2. Each API is focused and simple
3. Total code still manageable (~1300 lines)
4. Users choose the best fit

**Timeline:** 26 hours (3-4 days)

---

## Real-World Usage Predictions

### By User Type
- **Call Centers:** 80% PolicyPeer, 20% CallbackPeer
- **Softphones:** 60% CallbackPeer, 30% PolicyPeer, 10% EventStreamPeer
- **B2BUA/Gateways:** 60% EventStreamPeer, 40% CallbackPeer
- **IVR Systems:** 50% CallbackPeer, 50% PolicyPeer
- **Testing:** 90% SimplePeer, 10% EventStreamPeer

### By Experience Level
- **Beginners:** 100% SimplePeer (learning)
- **Intermediate:** 70% PolicyPeer, 30% CallbackPeer
- **Advanced:** 50% EventStreamPeer, 30% CallbackPeer, 20% PolicyPeer

---

## Final Recommendation (Updated)

### 🥇 First Priority: CallbackPeer (Trait-Based)
**Implement immediately** - Best session-core compatibility, clean trait syntax, built-in handlers, proven patterns.
**Timeline:** 9 hours, 500 lines
**Migration:** Easiest for session-core users

### 🥈 Second Priority: PolicyPeer (Enhanced)  
**Implement next** - Simpler than traits for basic cases, has Queue/Route policies, session-core patterns.
**Timeline:** 11 hours, 550 lines
**Best for:** Production apps, call centers, simple configuration

### 🥉 Third Priority: EventStreamPeer (With Helpers)
**Implement if time allows** - Most powerful, but most code. Helpers make it accessible.
**Timeline:** 13 hours, 620 lines
**Best for:** Advanced users + anyone using helpers

### Keep: SimplePeer
**Don't remove** - Still valuable for tests, examples, and learning.

---

## Alternative: Start with CallbackPeer

**Why CallbackPeer first?**
1. ✅ Best session-core compatibility (users can migrate easily)
2. ✅ Proven trait pattern (session-core validates this works)
3. ✅ Cleanest user code (no Box::pin)
4. ✅ Built-in handlers included
5. ✅ Shortest timeline (9 hours)
6. ✅ Medium code size (500 lines - not too much)

**Then add PolicyPeer or EventStreamPeer based on feedback.**

---

## Side-by-Side Feature Matrix

| Feature | Simple | Policy (Enhanced) | Callback (Trait) | Stream (Helpers) |
|---------|--------|------------------|------------------|------------------|
| Auto-accept calls | ❌ | ✅ | ✅ (built-in) | ✅ (helper) |
| Auto-reject calls | ❌ | ✅ | ✅ (built-in) | ✅ (helper) |
| Auto-transfer | ❌ | ✅ | ✅ (built-in) | ✅ (helper) |
| Queue pattern | ❌ | ✅ (policy) | ⚠️ Manual | ⚠️ Manual |
| Routing pattern | ❌ | ✅ (policy) | ⚠️ Manual | ✅ (streams) |
| Custom call logic | ⚠️ | ✅ (Manual policy) | ✅ (trait) | ✅ (streams) |
| Per-call events | ❌ | ❌ | ⚠️ Via call_id | ✅ (best) |
| Stream operators | ❌ | ❌ | ❌ | ✅ |
| Runtime config | ❌ | ❌ | ❌ | ✅ |
| Composite/Chain | ❌ | ⚠️ Future | ✅ (built-in) | ✅ (merge) |
| Type safety | ✅ | ✅ | ✅ | ✅ |
| Lost events | ⚠️ | ✅ | ✅ | ✅ |
| Setup complexity (simple) | ⭐ | ⭐ | ⭐⭐ | ⭐ |
| Setup complexity (advanced) | N/A | ⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ |
| Code size | 200 | 550 | 500 | 620 |
| session-core compat | ❌ | ⚠️ Patterns | ✅ Full | ❌ |
| Best for | Tests | Production | session-core users | Everyone |

---

## Migration Paths

### From SimplePeer to PolicyPeer (Easiest)
```rust
// Before
let mut peer = SimplePeer::new("alice").await?;

// After  
let peer = PolicyPeerBuilder::new("alice")
    .transfer_policy(TransferPolicy::AcceptBlind)
    .build().await?;

// Remove all wait_for_* calls
```

### From SimplePeer to CallbackPeer
```rust
// Before
peer.wait_for_incoming_call().await?;

// After
peer.on_incoming_call(|call| Box::pin(async move {
    Ok(CallDecision::Accept)
})).await;
```

### From PolicyPeer to CallbackPeer
```rust
// Before
.incoming_call_policy(IncomingCallPolicy::Accept)

// After
peer.on_incoming_call(|call| Box::pin(async move {
    Ok(CallDecision::Accept)
})).await;
```

### From CallbackPeer to EventStreamPeer
```rust
// Before
peer.on_transfer(|refer| Box::pin(...)).await;

// After
let mut transfers = peer.transfers();
while let Some(refer) = transfers.next().await {
    // ...
}
```

---

## Conclusion (Updated with Session-Core Patterns)

### Summary of Approaches

1. **SimplePeer** - Simple but broken for production
2. **PolicyPeer (Enhanced)** - Safe defaults + session-core patterns (Queue/Route)
3. **CallbackPeer (Trait-Based)** - session-core's CallHandler pattern with more events
4. **EventStreamPeer (With Helpers)** - Simple helpers + powerful streams

### Final Recommendation (REVISED)

**Implement CallbackPeer first** (9 hours, 500 lines), then **add PolicyPeer** (11 hours, 550 lines).

**Why CallbackPeer first?**
- ✅ Best session-core migration path
- ✅ Proven trait pattern (session-core validates it works)
- ✅ Cleanest user code (no Box::pin)
- ✅ Built-in handlers (AutoAnswer, RejectAll, Composite)
- ✅ Shortest timeline
- ✅ Most familiar to existing rvoip users

**Why PolicyPeer second?**
- ✅ Simpler for basic cases
- ✅ Session-core patterns as policies (Queue, Route)
- ✅ Safe defaults without trait implementation
- ✅ Good for call centers and production

**EventStreamPeer third (optional):**
- ✅ Most powerful
- ✅ Has helpers for simplicity
- ✅ Best per-call isolation
- ❌ Most code (620 lines)
- ❌ Longest timeline (13 hours)

All three approaches now include session-core's best patterns and solve the event loss problem.

---

## Appendix: Session-Core Compatibility Matrix

| session-core Feature | PolicyPeer (Enhanced) | CallbackPeer (Trait) | EventStreamPeer (Helpers) |
|---------------------|---------------------|---------------------|--------------------------|
| AutoAnswerHandler | ✅ with_auto_answer() | ✅ AutoAnswerHandler | ✅ with_auto_answer() |
| QueueHandler | ✅ Queue policy | ⚠️ Manual | ⚠️ Manual |
| RoutingHandler | ✅ Route policy | ⚠️ Manual in trait | ✅ Stream filters |
| CompositeHandler | ⚠️ Future | ✅ CompositeHandler | ⚠️ Merge streams |
| CallHandler trait | ❌ Different pattern | ✅ PeerHandler trait | ❌ Different pattern |
| CallDecision::Defer | ✅ Yes | ⚠️ Via Continue | ❌ No |
| CallDecision::Forward | ✅ Yes | ⚠️ Via trait | ❌ No |
| Migration Effort | Medium | ⭐ Easiest | Hard |

**Winner:** CallbackPeer has best session-core compatibility

---

## Can We Combine Them?

### Yes! Each API Can Coexist

```rust
// All use the same UnifiedCoordinator
pub mod simple;        // SimplePeer
pub mod policy;        // PolicyPeer (Enhanced)
pub mod callback;      // CallbackPeer (Trait-Based)
pub mod event_stream;  // EventStreamPeer (With Helpers)

// Users import what they need
use rvoip_session_core_v3::api::policy::PolicyPeer;
use rvoip_session_core_v3::api::callback::{CallbackPeer, PeerHandler};
```

### Or: Hybrid Features

```rust
// PolicyPeer could expose event streams for monitoring
impl PolicyPeer {
    pub fn event_stream(&self) -> impl Stream<Item = Event> {
        // Expose events even with policies
    }
}

// CallbackPeer could accept trait OR closures
impl CallbackPeer {
    pub async fn new_with_handler<H: PeerHandler>(handler: Arc<H>) -> Self;
    pub async fn new_with_closures(...) -> Self;  // Future
}
```

**Best of all approaches in one API!**

---

## Summary of Enhancements

All three approaches were enhanced with session-core's best patterns:

### PolicyPeer: +170 lines, +3 hours
- ✅ Queue policy (like QueueHandler)
- ✅ Route policy (like RoutingHandler)
- ✅ Pre-configured constructors (with_auto_answer, etc.)
- ✅ Forward and Defer decisions
- ✅ Trusted domains filtering

### CallbackPeer: Changed to trait-based, ~same size
- ✅ PeerHandler trait (like CallHandler)
- ✅ No more Box::pin boilerplate
- ✅ Built-in handlers (AutoAnswer, RejectAll, Composite)
- ✅ Clean async methods
- ✅ Full session-core compatibility

### EventStreamPeer: +260 lines, +5 hours
- ✅ Helper methods (auto_accept_calls, etc.)
- ✅ Pre-configured constructors (with_auto_answer, etc.)
- ✅ Builder with auto-spawn
- ✅ Simple for beginners, powerful for experts

**Total growth:** +430 lines, +8 hours
**Value:** Session-core compatibility, proven patterns, better UX

---

## Next Steps

1. ✅ Review all three enhanced plans
2. ✅ Reviewed session-core patterns
3. ⏳ Choose implementation strategy:
   - **Recommended:** CallbackPeer first (session-core compat)
   - **Then:** PolicyPeer (simpler for basic cases)
   - **Optional:** EventStreamPeer (power users)
4. ⏳ Implement chosen approach(es)
5. ⏳ Test and iterate
6. ⏳ Create migration guide from session-core
7. ⏳ Document and release

