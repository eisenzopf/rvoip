# Event Flow Fix Plan for session-core-v2

## Current State Analysis

### What's Working ✅
- Library compiles successfully
- GlobalEventCoordinator is integrated
- Adapters can publish events through GlobalEventCoordinator
- State machine processes events when directly called
- Basic state transitions work (Idle -> Initiating)

### What's NOT Working ❌
- **CrossCrateEvent handler only logs, doesn't process events**
- Events from dialog-core and media-core are not being converted to state machine events
- No actual event flow between layers
- The `as_any()` downcast issue prevents proper event type detection

## Root Cause
The `SessionCrossCrateEventHandler` cannot downcast `Arc<dyn CrossCrateEvent>` to concrete event types because:
1. The `as_any()` method is not accessible through the Arc
2. We need to properly implement the downcast pattern used in the existing codebase

## Solution Plan

### Phase 1: Fix Event Downcasting
Look at how the old session-core handles this in `/crates/session-core/src/events/adapter.rs`:
- They likely use a different pattern for downcasting
- Or they subscribe to specific event channels rather than using handlers

**Action Items:**
1. Study the old session-core event flow pattern
2. Check if we should use `subscribe()` instead of `register_handler()`
3. Implement proper event downcasting or use channel subscriptions

### Phase 2: Implement Proper Event Routing

#### Option A: Channel-Based Subscription (Likely Better)
```rust
// Instead of registering handlers:
let dialog_receiver = self.global_coordinator.subscribe("dialog_to_session").await?;
let media_receiver = self.global_coordinator.subscribe("media_to_session").await?;

// Spawn tasks to process events from channels
tokio::spawn(async move {
    while let Some(event) = dialog_receiver.recv().await {
        // Process dialog events
    }
});
```

#### Option B: Fix Downcast Pattern
```rust
// Need to find the correct way to access as_any() through Arc
// Possibly need to implement a custom trait or use a different approach
```

### Phase 3: Event Conversion Implementation

#### Dialog Events → State Machine Events
Map these DialogToSessionEvent variants to EventType:
- `IncomingCall` → `EventType::IncomingCall`
- `CallEstablished` → `EventType::Dialog200OK`
- `CallStateChanged::Ringing` → `EventType::Dialog180Ringing`
- `CallTerminated` → `EventType::DialogBYE`

#### Media Events → State Machine Events
Map these MediaToSessionEvent variants to EventType:
- `MediaStreamStarted` → `EventType::MediaSessionReady` + `EventType::MediaFlowEstablished`
- `MediaStreamStopped` → `EventType::MediaError`
- `MediaError` → `EventType::MediaError`

### Phase 4: Bidirectional Event Flow

#### Outbound Events (State Machine → Other Layers)
Currently, adapters publish events TO other layers. Need to ensure:
1. Dialog adapter publishes `SessionToDialogEvent` when state machine needs dialog actions
2. Media adapter publishes `SessionToMediaEvent` when state machine needs media actions

#### Inbound Events (Other Layers → State Machine)
Need to implement:
1. Subscribe to `dialog_to_session` events
2. Subscribe to `media_to_session` events
3. Convert and route to state machine

### Phase 5: Testing & Validation

1. Add comprehensive logging at each step:
   - When events are published
   - When events are received
   - When events are converted
   - When state machine processes events

2. Test scenarios:
   - UAC making a call
   - UAS receiving a call
   - Media negotiation
   - Call termination

## Implementation Steps

### Step 1: Research Old Pattern (30 min)
- [ ] Study `/crates/session-core/src/events/adapter.rs`
- [ ] Understand how they handle CrossCrateEvent downcasting
- [ ] Check if they use subscribe() vs register_handler()

### Step 2: Implement Event Subscription (1 hour)
- [ ] Replace register_handler with subscribe() if that's the pattern
- [ ] Create event processing tasks
- [ ] Handle event conversion properly

### Step 3: Fix Event Routing (2 hours)
- [ ] Implement dialog event → state machine routing
- [ ] Implement media event → state machine routing
- [ ] Add proper error handling

### Step 4: Add Bidirectional Flow (1 hour)
- [ ] Ensure outbound events work (state machine → adapters → other layers)
- [ ] Verify inbound events work (other layers → adapters → state machine)

### Step 5: Testing (1 hour)
- [ ] Add detailed logging
- [ ] Run api_peer_audio example
- [ ] Verify events flow correctly
- [ ] Fix any remaining issues

## Code Locations to Modify

1. **`/crates/session-core-v2/src/adapters/session_event_handler.rs`**
   - Completely rewrite to properly handle events

2. **`/crates/session-core-v2/src/api/unified.rs`**
   - Change from `register_handler()` to `subscribe()` if needed
   - Add event processing tasks

3. **`/crates/session-core-v2/src/adapters/dialog_adapter.rs`**
   - Ensure it publishes SessionToDialogEvent when needed

4. **`/crates/session-core-v2/src/adapters/media_adapter.rs`**
   - Ensure it publishes SessionToMediaEvent when needed

## Success Criteria

1. ✅ Events flow from dialog-core to session-core-v2 state machine
2. ✅ Events flow from media-core to session-core-v2 state machine  
3. ✅ State machine actions trigger events to dialog-core and media-core
4. ✅ The api_peer_audio example completes a full call flow
5. ✅ Logs show clear event flow between all layers

## Risk Mitigation

- **Risk**: The downcast pattern might not be fixable
- **Mitigation**: Use channel subscription pattern instead

- **Risk**: Event ordering issues
- **Mitigation**: Use proper async coordination and ensure events are processed sequentially per session

- **Risk**: Performance issues with event routing
- **Mitigation**: Use efficient channel sizes and consider batching if needed

## Timeline Estimate
Total: ~5-6 hours of focused work

## Next Immediate Action
Start by examining `/crates/session-core/src/events/adapter.rs` to understand the working pattern from the old implementation.