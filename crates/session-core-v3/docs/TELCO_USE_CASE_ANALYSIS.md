# Telco Use Case Analysis: CallbackPeer vs EventStreamPeer

## Executive Summary

This document analyzes real-world telecommunications use cases to determine whether **CallbackPeer (Trait-Based)** and **EventStreamPeer (With Helpers)** serve different enough needs to justify implementing both.

**TL;DR:** They serve **significantly different** use cases. CallbackPeer excels at structured applications with clear event handlers, while EventStreamPeer shines in complex multi-call scenarios requiring per-call event isolation.

---

## Use Case Categories

### 1. Simple Call Center Agent Station
### 2. Advanced Contact Center (Multi-Call)
### 3. SIP Softphone (Desktop/Mobile)
### 4. PBX/Call Routing System
### 5. B2BUA/SIP Gateway
### 6. IVR (Interactive Voice Response)
### 7. Voicemail System
### 8. Call Recording System
### 9. SIP Proxy/Load Balancer
### 10. Emergency Services (E911)

---

## Detailed Analysis

### 1. Simple Call Center Agent Station

**Scenario:** Agent handles one call at a time, answers queue, handles transfers.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct AgentHandler {
    agent_id: String,
    status: Arc<RwLock<AgentStatus>>,
}

#[async_trait]
impl PeerHandler for AgentHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Check agent status
        if *self.status.read().await == AgentStatus::Available {
            *self.status.write().await = AgentStatus::OnCall;
            update_agent_ui("On Call").await;
            CallDecision::Accept
        } else {
            CallDecision::RejectBusy
        }
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        // Agent accepts all transfers from supervisor
        TransferDecision::Accept
    }
    
    async fn on_call_ended(&self, _call_id: CallId, _reason: String) {
        *self.status.write().await = AgentStatus::Available;
        update_agent_ui("Available").await;
    }
    
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        // Display DTMF to agent UI
        show_dtmf_digit(digit).await;
    }
}

let peer = CallbackPeer::new("agent001", Arc::new(AgentHandler { ... })).await?;
```

**Pros:**
- ✅ Clean structure - all agent logic in one place
- ✅ Easy to test - mock the trait
- ✅ UI updates integrated in handler methods

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("agent001").await?;

// Set up streams for each event type
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            if agent_status == AgentStatus::Available {
                peer.accept(&call.call_id).await.ok();
                agent_status = AgentStatus::OnCall;
            }
        }
    }
});

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut transfers = peer.transfers();
        while let Some(refer) = transfers.next().await {
            // Handle transfer
        }
    }
});
```

**Cons:**
- ❌ Logic scattered across multiple spawns
- ❌ Harder to maintain agent state
- ❌ More verbose for simple case

**Winner:** ✅ **CallbackPeer** - Cleaner for single-call, state-based logic

---

### 2. Advanced Contact Center (Multi-Call)

**Scenario:** Supervisor monitors multiple agents, handles escalations, can join/transfer any call.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct SupervisorHandler {
    active_calls: Arc<Mutex<HashMap<CallId, CallInfo>>>,
}

#[async_trait]
impl PeerHandler for SupervisorHandler {
    async fn on_call_answered(&self, call_id: CallId) {
        // Track call - but which agent?
        // Trait methods only get call_id, not call-specific context
        self.active_calls.lock().await.insert(call_id, CallInfo { ... });
    }
}
```

**Cons:**
- ❌ No per-call context
- ❌ Must track all calls in shared state
- ❌ Can't easily filter events by agent/queue

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("supervisor").await?;

// Spawn per-agent monitoring
for agent_id in agents {
    let agent_calls = peer.events_for_call(agent_call_id);
    
    tokio::spawn({
        let peer = peer.clone();
        async move {
            while let Some(event) = agent_calls.next().await {
                match event {
                    CallEvent::Ended { reason } => {
                        // This agent's call ended
                        update_dashboard(agent_id, "Available").await;
                    }
                    _ => {}
                }
            }
        }
    });
}

// Monitor for escalations
let mut escalations = peer.transfers()
    .filter(|refer| async move {
        refer.refer_to.contains("supervisor")
    });

while let Some(escalation) = escalations.next().await {
    // Handle escalation from any agent
}
```

**Pros:**
- ✅ Per-call event isolation
- ✅ Filter by agent/queue easily
- ✅ Stream operators for complex logic

**Winner:** ✅ **EventStreamPeer** - Better for multi-call monitoring

---

### 3. SIP Softphone (Desktop/Mobile)

**Scenario:** User makes/receives calls, handles transfers, sends DTMF, manages call features.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct SoftphoneHandler {
    ui: Arc<SoftphoneUI>,
}

#[async_trait]
impl PeerHandler for SoftphoneHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Show UI notification
        self.ui.show_incoming_call(&call.from).await;
        
        // Wait for user decision (blocks handler, but that's ok)
        let decision = self.ui.wait_for_user_decision().await;
        
        match decision {
            UiDecision::Accept => CallDecision::Accept,
            UiDecision::Reject => CallDecision::Reject,
        }
    }
    
    async fn on_call_answered(&self, call_id: CallId) {
        self.ui.update_call_status(&call_id, "Connected");
        self.ui.enable_call_controls(&call_id);
    }
    
    async fn on_transfer(&self, refer: ReferRequest) -> TransferDecision {
        // Show transfer notification
        let approved = self.ui.show_transfer_dialog(&refer.refer_to).await;
        if approved {
            TransferDecision::Accept
        } else {
            TransferDecision::Reject
        }
    }
}
```

**Pros:**
- ✅ Natural mapping: trait methods → UI events
- ✅ All UI logic in one place
- ✅ Easy to understand and maintain

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("alice").await?;

// Spawn for incoming calls
tokio::spawn({
    let ui = ui.clone();
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            ui.show_incoming_call(&call.from).await;
            let decision = ui.wait_for_user_decision().await;
            // ...
        }
    }
});

// Spawn for transfers
tokio::spawn({
    let ui = ui.clone();
    async move {
        let mut transfers = peer.transfers();
        while let Some(refer) = transfers.next().await {
            ui.show_transfer_dialog(&refer.refer_to).await;
            // ...
        }
    }
});

// Spawn for each call feature...
```

**Cons:**
- ❌ UI logic scattered across multiple spawns
- ❌ More boilerplate
- ❌ Harder to track UI state

**Winner:** ✅ **CallbackPeer** - Better for UI applications

---

### 4. PBX/Call Routing System

**Scenario:** Route calls based on patterns, business hours, call volume, etc.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct PbxRoutingHandler {
    routes: Vec<Route>,
    business_hours: BusinessHours,
}

#[async_trait]
impl PeerHandler for PbxRoutingHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Check business hours
        if !self.business_hours.is_open() {
            return CallDecision::Forward("sip:voicemail@pbx".to_string());
        }
        
        // Pattern matching
        for route in &self.routes {
            if call.to.contains(&route.pattern) {
                return CallDecision::Forward(route.target.clone());
            }
        }
        
        CallDecision::Reject
    }
}
```

**Pros:**
- ✅ Simple routing logic
- ✅ Clear structure

**Cons:**
- ❌ All routing in one method
- ❌ Hard to add complex rules

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("pbx").await?;

// Pipeline: business hours filter → department routing → overflow
let mut routed_calls = peer.incoming_calls()
    .filter(|call| async move {
        business_hours.is_open()
    })
    .filter_map(|call| async move {
        // Department routing
        if call.to.contains("support@") {
            Some((call, "sip:queue@support"))
        } else if call.to.contains("sales@") {
            Some((call, "sip:queue@sales"))
        } else {
            None
        }
    })
    .then(|(call, target)| async move {
        // Check queue depth before forwarding
        if queue_depth(target).await < 10 {
            Some((call, target))
        } else {
            Some((call, "sip:overflow@pbx"))  // Overflow
        }
    });

while let Some((call, target)) = routed_calls.next().await {
    peer.reject(&call.call_id, &format!("Forward:{}", target)).await.ok();
}
```

**Pros:**
- ✅ Complex routing pipelines
- ✅ Easy to add stages (filter → map → then)
- ✅ Composable logic

**Winner:** ✅ **EventStreamPeer** - Better for complex routing pipelines

**But:** PolicyPeer's Route policy might be sufficient for most PBX cases!

---

### 5. B2BUA/SIP Gateway

**Scenario:** Bridge calls between networks, transcode, handle two call legs per session.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct B2buaHandler {
    peer: Arc<CallbackPeer>,
    leg_mapping: Arc<Mutex<HashMap<CallId, CallId>>>,  // leg1 → leg2
}

#[async_trait]
impl PeerHandler for B2buaHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Accept inbound leg
        CallDecision::Accept
    }
    
    async fn on_call_answered(&self, inbound_leg: CallId) {
        // Create outbound leg
        let outbound_leg = self.peer.call("sip:destination@network-b").await.ok();
        
        // Map legs
        if let Some(outbound) = outbound_leg {
            self.leg_mapping.lock().await.insert(inbound_leg, outbound);
        }
    }
    
    async fn on_call_ended(&self, leg: CallId, _reason: String) {
        // Find and terminate other leg
        let other_leg = self.leg_mapping.lock().await.remove(&leg);
        if let Some(other) = other_leg {
            self.peer.hangup(&other).await.ok();
        }
    }
}
```

**Pros:**
- ✅ Clear leg coordination logic

**Cons:**
- ❌ Awkward: handler needs peer reference (circular)
- ❌ Manual leg tracking
- ❌ Can't monitor legs independently

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("b2bua").await?;

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut inbound_calls = peer.incoming_calls();
        
        while let Some(inbound) = inbound_calls.next().await {
            // Accept inbound leg
            peer.accept(&inbound.call_id).await.ok();
            
            // Create outbound leg
            let outbound_id = peer.call("sip:dest@network-b").await.ok();
            
            if let Some(outbound) = outbound_id {
                // Monitor BOTH legs independently!
                let inbound_events = peer.events_for_call(inbound.call_id.clone());
                let outbound_events = peer.events_for_call(outbound.clone());
                
                // Bridge them
                tokio::spawn(async move {
                    tokio::select! {
                        Some(CallEvent::Ended { .. }) = inbound_events.next() => {
                            // Inbound ended, hangup outbound
                            peer.hangup(&outbound).await.ok();
                        }
                        Some(CallEvent::Ended { .. }) = outbound_events.next() => {
                            // Outbound ended, hangup inbound
                            peer.hangup(&inbound.call_id).await.ok();
                        }
                    }
                });
            }
        }
    }
});
```

**Pros:**
- ✅ **Per-call event streams** (huge for B2BUA!)
- ✅ Independent monitoring of each leg
- ✅ No circular references
- ✅ Can use select! to race events

**Winner:** ✅ **EventStreamPeer** - Much better for B2BUA (per-call isolation critical!)

---

### 6. IVR (Interactive Voice Response)

**Scenario:** Answer calls, collect DTMF input, navigate menu trees, transfer to departments.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct IvrHandler {
    current_menu: Arc<RwLock<HashMap<CallId, MenuState>>>,
}

#[async_trait]
impl PeerHandler for IvrHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Auto-accept all calls
        self.current_menu.lock().await.insert(call.call_id.clone(), MenuState::MainMenu);
        play_greeting(&call.call_id).await;
        CallDecision::Accept
    }
    
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        let mut menus = self.current_menu.lock().await;
        let menu = menus.get_mut(&call_id).unwrap();
        
        match (&menu, digit) {
            (MenuState::MainMenu, '1') => {
                *menu = MenuState::Sales;
                play_message("Connecting to sales...").await;
            }
            (MenuState::MainMenu, '2') => {
                *menu = MenuState::Support;
                play_message("Connecting to support...").await;
            }
            _ => {}
        }
    }
}
```

**Pros:**
- ✅ Simple state tracking
- ✅ All IVR logic together

**Cons:**
- ❌ Must maintain per-call state manually
- ❌ DTMF scattered across calls

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::with_auto_answer("ivr").await?;

// Process each call independently
tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        
        while let Some(call) = calls.next().await {
            // Spawn IVR session for THIS call only
            tokio::spawn({
                let peer = peer.clone();
                let call_id = call.call_id.clone();
                
                async move {
                    // Get DTMF stream for THIS call only
                    let digits = peer.dtmf_stream()
                        .filter(|(id, _)| async move { *id == call_id })
                        .map(|(_, digit)| digit)
                        .take_while(|d| async move { *d != '#' })
                        .collect::<Vec<_>>()
                        .await;
                    
                    // Process collected digits
                    match digits.as_slice() {
                        ['1'] => peer.send_refer(&call_id, "sip:sales@pbx").await.ok(),
                        ['2'] => peer.send_refer(&call_id, "sip:support@pbx").await.ok(),
                        _ => {}
                    };
                }
            });
        }
    }
});
```

**Pros:**
- ✅ **Per-call DTMF isolation** (huge!)
- ✅ Each call gets own state machine
- ✅ Easy to collect digits (take_while)
- ✅ No shared state needed

**Winner:** ✅ **EventStreamPeer** - Per-call streams perfect for IVR!

---

### 7. Voicemail System

**Scenario:** Accept all calls, play greeting, record message, save to storage.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct VoicemailHandler {
    storage: Arc<VoicemailStorage>,
}

#[async_trait]
impl PeerHandler for VoicemailHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Auto-accept
        play_greeting(&call.call_id).await;
        start_recording(&call.call_id).await;
        CallDecision::Accept
    }
    
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        // Handle # to finish recording
        if digit == '#' {
            stop_recording(&call_id).await;
            save_to_storage(&call_id).await;
        }
    }
    
    async fn on_call_ended(&self, call_id: CallId, _reason: String) {
        // Save recording
        self.storage.save_message(&call_id).await.ok();
    }
}
```

**Pros:**
- ✅ Simple, clear flow
- ✅ All voicemail logic together

**Winner:** ✅ **CallbackPeer** - Simple, structured logic

---

### 8. Call Recording System

**Scenario:** Monitor all calls, record audio streams, store with metadata.

#### With CallbackPeer (Trait-Based)
```rust
#[async_trait]
impl PeerHandler for RecordingHandler {
    async fn on_call_answered(&self, call_id: CallId) {
        // Start recording - but only THIS call
        start_recording(&call_id).await;
    }
    
    async fn on_call_ended(&self, call_id: CallId, _reason: String) {
        // Stop recording
        stop_recording(&call_id).await;
    }
}
```

**Cons:**
- ❌ Can't easily get call-specific audio streams
- ❌ All calls handled the same way

#### With EventStreamPeer (With Helpers)
```rust
let peer = EventStreamPeer::new("recorder").await?;

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut answered = peer.events_lossy()
            .filter_map(|e| async move {
                match e {
                    Event::CallAnswered { call_id, .. } => Some(call_id),
                    _ => None
                }
            });
        
        while let Some(call_id) = answered.next().await {
            // Get audio stream for THIS call
            let mut audio = peer.subscribe_to_audio(&call_id).await.ok();
            
            // Record THIS call's audio independently
            tokio::spawn(async move {
                let mut recording = vec![];
                while let Some(frame) = audio.recv().await {
                    recording.extend(frame.samples);
                }
                save_recording(&call_id, &recording).await;
            });
        }
    }
});
```

**Pros:**
- ✅ Per-call audio streams
- ✅ Independent recordings

**Winner:** ✅ **EventStreamPeer** - Better for per-call audio isolation

---

### 9. SIP Proxy/Load Balancer

**Scenario:** Forward calls to available backend servers, health check, failover.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct LoadBalancerHandler {
    backends: Arc<RwLock<Vec<Backend>>>,
}

#[async_trait]
impl PeerHandler for LoadBalancerHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // Find available backend
        let backends = self.backends.read().await;
        let available = backends.iter()
            .find(|b| b.available_capacity() > 0);
        
        if let Some(backend) = available {
            CallDecision::Forward(backend.uri.clone())
        } else {
            CallDecision::RejectBusy
        }
    }
}
```

**Pros:**
- ✅ Simple load balancing logic

**Winner:** ✅ **CallbackPeer** - Sufficient for basic load balancing

**But:** PolicyPeer's Route policy might be even simpler!

---

### 10. Emergency Services (E911)

**Scenario:** High reliability, call priority, geographic routing, never drop calls.

#### With CallbackPeer (Trait-Based)
```rust
#[derive(Debug)]
struct E911Handler {
    location_service: Arc<LocationService>,
}

#[async_trait]
impl PeerHandler for E911Handler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        // ALWAYS accept emergency calls
        let location = self.location_service.lookup(&call.from).await;
        let psap = find_nearest_psap(&location).await;
        
        // Forward to appropriate PSAP
        CallDecision::Forward(psap.uri)
    }
    
    async fn on_call_failed(&self, call_id: CallId, status_code: u16, reason: String) {
        // CRITICAL: Emergency call failed!
        alert_supervisor("E911 FAILURE", &call_id, status_code, &reason).await;
        
        // Try backup PSAPs
        retry_with_backup(&call_id).await;
    }
}
```

**Pros:**
- ✅ Critical logic in one place
- ✅ Easy to audit for compliance
- ✅ Clear failure handling

**Winner:** ✅ **CallbackPeer** - Better for critical, auditable systems

---

## Summary Matrix

| Use Case | Best API | Why |
|----------|---------|-----|
| **Simple Call Center Agent** | CallbackPeer | ✅ Structured logic, UI mapping |
| **Multi-Call Contact Center** | EventStreamPeer | ✅ Per-call monitoring, filtering |
| **SIP Softphone** | CallbackPeer | ✅ UI event mapping, simple flow |
| **PBX Routing** | EventStreamPeer or PolicyPeer | ✅ Pipeline routing (Stream) or Route policy (Policy) |
| **B2BUA/Gateway** | EventStreamPeer | ✅ Per-call isolation, leg monitoring |
| **IVR System** | EventStreamPeer | ✅ Per-call DTMF collection, independent sessions |
| **Voicemail** | CallbackPeer | ✅ Simple, clear flow |
| **Call Recording** | EventStreamPeer | ✅ Per-call audio streams |
| **Load Balancer** | CallbackPeer or PolicyPeer | ✅ Simple routing logic |
| **E911/Emergency** | CallbackPeer | ✅ Critical logic, auditable |

---

## Key Differentiators

### CallbackPeer Excels When:
1. **Single call at a time** - Agent stations, softphones
2. **UI applications** - Trait methods map to UI events
3. **Structured logic** - All event handling in one place
4. **Simple state** - Don't need per-call isolation
5. **Auditable systems** - E911, compliance
6. **Testing** - Easy to mock traits

### EventStreamPeer Excels When:
1. **Multiple concurrent calls** - B2BUA, contact center supervisor
2. **Per-call isolation needed** - IVR, call recording
3. **Complex event pipelines** - Advanced routing, filtering
4. **Per-call audio streams** - Recording, monitoring
5. **Reactive processing** - Stream operators (filter, map, merge)
6. **Per-call state machines** - IVR sessions

---

## Critical Distinction: **Per-Call Event Isolation**

This is the **killer feature** that makes EventStreamPeer unique:

### CallbackPeer
```rust
async fn on_dtmf(&self, call_id: CallId, digit: char) {
    // Gets DTMF from ALL calls
    // Must check call_id manually
    // Can't easily collect digits for ONE call
}
```

### EventStreamPeer
```rust
// Get DTMF stream for ONE specific call
let dtmf_for_call_123 = peer.dtmf_stream()
    .filter(|(id, _)| async move { *id == call_123 })
    .map(|(_, digit)| digit);

// Collect until '#'
let digits: Vec<char> = dtmf_for_call_123
    .take_while(|d| async move { *d != '#' })
    .collect()
    .await;
```

**This is impossible with CallbackPeer!**

---

## Decision Framework

### Implement BOTH if you need:
- ✅ Session-core migration (CallbackPeer)
- ✅ B2BUA/Gateway (EventStreamPeer)
- ✅ IVR with DTMF collection (EventStreamPeer)
- ✅ Multi-call scenarios (EventStreamPeer)
- ✅ Simple softphone (CallbackPeer)

### Implement ONLY CallbackPeer if:
- Your users primarily need:
  - Simple agents (one call at a time)
  - UI applications
  - Session-core migration
  - **No per-call event isolation**

### Implement ONLY EventStreamPeer if:
- Your users primarily need:
  - B2BUA/gateways
  - IVR systems
  - Multi-call monitoring
  - **Per-call event isolation is critical**

---

## Overlapping Use Cases

Some use cases work with either API:

| Use Case | CallbackPeer | EventStreamPeer | Winner |
|----------|--------------|----------------|--------|
| **Softphone** | ✅ Natural UI mapping | ⚠️ Works but verbose | CallbackPeer |
| **Simple Agent** | ✅ Structured | ⚠️ Overkill | CallbackPeer |
| **PBX Routing** | ⚠️ Manual routing | ✅ Pipeline routing | EventStream OR PolicyPeer |
| **Load Balancer** | ✅ Simple logic | ⚠️ Overkill | CallbackPeer OR PolicyPeer |
| **B2BUA** | ⚠️ Awkward | ✅ Perfect | EventStreamPeer |
| **IVR** | ⚠️ Can work | ✅ Per-call DTMF | EventStreamPeer |
| **Recording** | ⚠️ Can work | ✅ Per-call audio | EventStreamPeer |

---

## Real-World Usage Predictions

### If you implement ONLY CallbackPeer:
- ✅ Covers: 60% of use cases
  - Softphones ✅
  - Simple agents ✅
  - Voicemail ✅
  - E911 ✅
  - Session-core migration ✅
- ❌ Struggles with: 40% of use cases
  - B2BUA ❌ (awkward)
  - IVR with DTMF ❌ (no per-call collection)
  - Multi-call monitoring ❌ (no isolation)
  - Recording ❌ (no per-call audio)

### If you implement ONLY EventStreamPeer:
- ✅ Covers: 90% of use cases (with helpers)
  - Simple cases ✅ (via helpers)
  - B2BUA ✅
  - IVR ✅
  - Multi-call ✅
  - Recording ✅
- ⚠️ Awkward for: 10% of use cases
  - UI applications ⚠️ (trait mapping cleaner)
  - Session-core migration ⚠️ (different pattern)

### If you implement BOTH:
- ✅ Covers: 100% of use cases optimally
- ✅ Users pick the right tool
- ❌ More code to maintain (+1120 lines total)

---

## Code Complexity Comparison

### CallbackPeer: Softphone Example
```rust
#[derive(Debug)]
struct SoftphoneHandler { ui: Arc<UI> }

#[async_trait]
impl PeerHandler for SoftphoneHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        self.ui.show_notification(&call.from).await;
        let decision = self.ui.wait_for_decision().await;
        if decision { CallDecision::Accept } else { CallDecision::Reject }
    }
}

let peer = CallbackPeer::new("alice", Arc::new(SoftphoneHandler { ui })).await?;
```

**Lines:** ~15

### EventStreamPeer: Softphone Example
```rust
let peer = EventStreamPeer::new("alice").await?;

tokio::spawn({
    let ui = ui.clone();
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            ui.show_notification(&call.from).await;
            let decision = ui.wait_for_decision().await;
            if decision {
                peer.accept(&call.call_id).await.ok();
            } else {
                peer.reject(&call.call_id, "Declined").await.ok();
            }
        }
    }
});
```

**Lines:** ~20 (+5 lines, more verbose)

---

### CallbackPeer: IVR Example
```rust
#[derive(Debug)]
struct IvrHandler {
    menus: Arc<Mutex<HashMap<CallId, MenuState>>>,
}

#[async_trait]
impl PeerHandler for IvrHandler {
    async fn on_incoming_call(&self, call: IncomingCallInfo) -> CallDecision {
        self.menus.lock().await.insert(call.call_id.clone(), MenuState::Main);
        CallDecision::Accept
    }
    
    async fn on_dtmf(&self, call_id: CallId, digit: char) {
        let menu = self.menus.lock().await.get(&call_id).cloned();
        // Process digit...
        // Problem: DTMF from all calls mixed together!
    }
}
```

**Lines:** ~20
**Problem:** Can't collect digits for one call easily

### EventStreamPeer: IVR Example
```rust
let peer = EventStreamPeer::with_auto_answer("ivr").await?;

tokio::spawn({
    let peer = peer.clone();
    async move {
        let mut calls = peer.incoming_calls();
        while let Some(call) = calls.next().await {
            // Per-call IVR session
            tokio::spawn({
                let peer = peer.clone();
                let call_id = call.call_id;
                async move {
                    // Collect DTMF for THIS call only
                    let digits = peer.dtmf_stream()
                        .filter(|(id, _)| async move { *id == call_id })
                        .map(|(_, d)| d)
                        .take_while(|d| async move { *d != '#' })
                        .collect().await;
                    
                    // Process collected digits
                    handle_ivr_input(&call_id, digits).await;
                }
            });
        }
    }
});
```

**Lines:** ~25
**Benefit:** Per-call DTMF collection (can't do this with CallbackPeer!)

---

## Performance Considerations

### CallbackPeer
- Single event processor
- All events routed to one trait
- Lower overhead
- **Best for:** < 1000 concurrent calls

### EventStreamPeer
- Broadcast to multiple consumers
- Can spawn per-call tasks
- Higher overhead (broadcast channel)
- **Best for:** Any scale (can spawn/stop consumers dynamically)

---

## Recommendation

### Implement BOTH ⭐⭐⭐⭐⭐

**Why:**
1. **Different strengths:**
   - CallbackPeer: Structured apps, UI, session-core migration
   - EventStreamPeer: B2BUA, IVR, multi-call, per-call isolation

2. **Complementary:**
   - CallbackPeer for 60% of use cases (simple)
   - EventStreamPeer for 40% of use cases (complex multi-call)
   - Together cover 100% optimally

3. **User choice:**
   - Session-core users → CallbackPeer (easy migration)
   - New users, simple apps → CallbackPeer or PolicyPeer
   - B2BUA/IVR developers → EventStreamPeer

4. **Reasonable cost:**
   - CallbackPeer: 9 hours, 500 lines
   - EventStreamPeer: 13 hours, 620 lines
   - **Total: 22 hours, 1120 lines**
   - **Benefit:** Cover all use cases optimally

### Alternative: Start with CallbackPeer Only

If time/resources are limited:
- Implement CallbackPeer first (9 hours)
- Cover 60% of use cases well
- Add EventStreamPeer later if users need:
  - B2BUA functionality
  - IVR with per-call DTMF
  - Per-call event isolation

---

## Conclusion

**CallbackPeer and EventStreamPeer serve fundamentally different needs:**

| Dimension | CallbackPeer | EventStreamPeer |
|-----------|--------------|----------------|
| **Best for** | Structured, single-call | Multi-call, reactive |
| **Key feature** | Clean trait syntax | Per-call event isolation |
| **Session-core compat** | ✅ Full | ❌ No |
| **Use case overlap** | 40% unique | 40% unique |
| **Shared use cases** | 20% (both work) | 20% (both work) |

**Verdict:** 
- ✅ **Implement both** - They complement each other
- ⚠️ **Or start with CallbackPeer** - Add EventStream later if needed
- ❌ **Don't skip both** - One is essential for production use

The key question: **Do your users need B2BUA or IVR with per-call DTMF collection?**
- **Yes** → Implement both
- **No** → CallbackPeer is sufficient

**My recommendation:** Implement both. 22 hours is reasonable for comprehensive coverage.

