# Event System Fix Plan: Migrate to 100% GlobalEventCoordinator

## Overview
This plan addresses the complete migration from channel-based events to GlobalEventCoordinator-based events across session-core-v2, dialog-core, and media-core, while reinforcing the event-driven architecture.

## Architectural Principles

### Event-Driven System Design
The RVOIP system is fundamentally **event-driven and state-driven**:

1. **Events Drive State Transitions**: External events (SIP messages, RTP events, timers) trigger state machine transitions
2. **State Transitions Drive Actions**: The state machine determines what actions to take based on state transitions
3. **Actions May Be Direct or Event-Based**: 
   - **Direct calls**: Resource creation/destruction, configuration, deterministic operations
   - **Events**: State changes, asynchronous notifications, multi-consumer scenarios

### When to Use Direct Function Calls vs Events

**Direct Function Calls (session-core → dialog/media-core):**
- Creating resources: `dialog_api.make_call()`, `media_controller.create_session()`
- Destroying resources: `dialog_api.terminate()`, `media_controller.stop_session()`
- Configuration: `media_controller.set_codec()`, `dialog_api.set_transport()`
- Synchronous queries: `dialog_api.get_state()`, `media_controller.get_statistics()`
- Deterministic protocol actions: `dialog_api.send_bye()`, `dialog_api.send_response()`

**Events (dialog/media-core → session-core):**
- All state changes: `DialogStateChanged`, `MediaStreamStateChanged`
- External triggers: `IncomingCall`, `DtmfReceived`, `RtpTimeout`
- Asynchronous notifications: `MediaQualityDegraded`, `PacketLossDetected`
- Protocol events: `ReinviteReceived`, `TransferRequested`

### State Machine as the Central Driver
```
External Event → State Machine → State Transition → Action(s)
                       ↑                                 ↓
                       └─────── Internal Events ←────────┘
```

The state machine in session-core-v2:
1. Receives events from GlobalEventCoordinator
2. Determines valid state transitions
3. Executes actions (which may be direct calls OR publish new events)
4. Updates session state

## Current Issues

1. **Mixed Event Systems**: Both channels and GlobalEventCoordinator are in use
2. **String-Based Event Parsing**: Events are being parsed from debug strings
3. **Incomplete Event Coverage**: Missing key events (media quality, DTMF, etc.)
4. **Resource Creation Confusion**: Unclear dialog creation semantics
5. **Backward Compatibility Overhead**: Dual paths for everything

## Fix Plan

### Phase 1: Remove Channel-Based Communication

#### 1.1 Dialog-Core Channel Removal

**Files to modify:**
- `crates/session-core-v2/src/adapters/session_event_handler.rs`
- `crates/dialog-core/src/api/unified.rs`
- `crates/dialog-core/src/events/adapter.rs`

**Actions:**
1. Remove `setup_dialog_channels()` method from SessionCrossCrateEventHandler
2. Remove `set_session_coordinator()` and `set_dialog_event_sender()` from dialog-core API
3. Remove channel fields from DialogEventAdapter:
   ```rust
   // Remove these:
   dialog_event_sender: Arc<RwLock<Option<mpsc::Sender<DialogEvent>>>>,
   session_coordination_sender: Arc<RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>>,
   ```
4. Update DialogEventHub to be the ONLY event publisher

#### 1.2 Media-Core Channel Removal

**Files to modify:**
- `crates/session-core-v2/src/adapters/session_event_handler.rs`
- `crates/media-core/src/relay/controller.rs`
- `crates/media-core/src/events/adapter.rs`

**Actions:**
1. Remove `setup_media_channels()` method from SessionCrossCrateEventHandler
2. Remove channel-based event senders from MediaSessionController
3. Remove channel fields from MediaEventAdapter:
   ```rust
   // Remove these:
   media_event_sender: Arc<RwLock<Option<mpsc::Sender<MediaSessionEventType>>>>,
   integration_event_sender: Arc<RwLock<Option<mpsc::Sender<IntegrationEventType>>>>,
   ```

### Phase 2: Fix Event Deserialization

#### 2.1 Implement Proper Event Handling

**Key Principle**: Events should trigger state transitions, not direct actions. The state machine decides what to do.

**File to modify:**
- `crates/session-core-v2/src/adapters/session_event_handler.rs`

**Current problematic code:**
```rust
// This string parsing must go:
let event_str = format!("{:?}", event);
if event_str.contains("InitiateCall") {
    // String parsing logic
}
```

**Replace with:**
```rust
#[async_trait]
impl CrossCrateEventHandler for SessionCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        // Use the same approach as dialog-core's event_hub.rs
        if let Ok(rvoip_event_arc) = Arc::downcast::<RvoipCrossCrateEvent>(event) {
            match rvoip_event_arc.as_ref() {
                RvoipCrossCrateEvent::DialogToSession(dialog_event) => {
                    self.handle_dialog_to_session_event(dialog_event).await?;
                }
                RvoipCrossCrateEvent::MediaToSession(media_event) => {
                    self.handle_media_to_session_event(media_event).await?;
                }
                _ => {
                    debug!("Unhandled event type");
                }
            }
        }
        Ok(())
    }
}
```

#### 2.2 Remove String Extraction Functions

**Actions:**
1. Remove `extract_session_id()`, `convert_dialog_event()`, `convert_media_event()` methods
2. Implement proper typed event handlers for each event variant

### Phase 3: Complete Event Coverage

#### 3.1 Add Missing Media Events

**Files to modify:**
- `crates/infra-common/src/events/cross_crate.rs`
- `crates/media-core/src/events/event_hub.rs`

**New events to add:**
```rust
pub enum MediaToSessionEvent {
    // Existing events...
    
    // Add these:
    MediaQualityDegraded {
        session_id: String,
        metrics: MediaQualityMetrics,
        severity: QualitySeverity,
    },
    DtmfDetected {
        session_id: String,
        digit: char,
        duration_ms: u32,
    },
    RtpTimeout {
        session_id: String,
        last_packet_time: u64,
    },
    PacketLossThresholdExceeded {
        session_id: String,
        loss_percentage: f32,
    },
}
```

#### 3.2 Add Missing Dialog Events

**New events to add:**
```rust
pub enum DialogToSessionEvent {
    // Existing events...
    
    // Add these:
    DialogStateChanged {
        session_id: String,
        old_state: DialogState,
        new_state: DialogState,
    },
    ReinviteReceived {
        session_id: String,
        sdp: Option<String>,
    },
    TransferRequested {
        session_id: String,
        refer_to: String,
        transfer_type: TransferType,
    },
}
```

### Phase 4: Fix Resource Creation Semantics

#### 4.1 Clarify DialogAdapter Methods

**File to modify:**
- `crates/session-core-v2/src/adapters/dialog_adapter.rs`

**Option A: Remove confusing methods**
```rust
// Remove these:
pub async fn create_dialog(&self, from: &str, to: &str) -> Result<DialogId>
pub async fn send_invite(&self, dialog_id: DialogId) -> Result<()>

// Keep only:
pub async fn send_invite_with_details(...) // This actually creates the dialog
```

**Option B: Make them functional**
```rust
pub async fn create_dialog(&self, from: &str, to: &str) -> Result<DialogId> {
    // Actually create a dialog in dialog-core
    let dialog_handle = self.dialog_api.create_dialog(from, to).await?;
    let dialog_id = dialog_handle.id();
    // Store mappings...
    Ok(dialog_id)
}
```

### Phase 5: Clean Up Event Flow

#### 5.1 Remove Backward Compatibility Functions

**Files to modify:**
- `crates/session-core-v2/src/adapters/session_event_handler.rs`
- `crates/dialog-core/src/events/adapter.rs`
- `crates/media-core/src/events/adapter.rs`

**Actions:**
1. Remove `handle_session_coordination_event()`
2. Remove `handle_dialog_event()`
3. Remove `handle_media_event()`
4. Remove all backward compatibility event conversion functions

#### 5.2 Consolidate Event Publishing

**Ensure single path for event publishing:**
```rust
// In dialog-core
impl DialogEventHub {
    pub async fn publish_event(&self, event: DialogToSessionEvent) -> Result<()> {
        let cross_crate_event = RvoipCrossCrateEvent::DialogToSession(event);
        self.global_coordinator.publish(Arc::new(cross_crate_event)).await
    }
}
```

### Phase 6: Document Clear Boundaries

#### 6.1 Create Interface Documentation

**New files to create:**
- `crates/session-core-v2/docs/DIALOG_INTERFACE.md`
- `crates/session-core-v2/docs/MEDIA_INTERFACE.md`

**Content structure:**
```markdown
# Dialog Interface

## Direct Calls (Session → Dialog)
- `make_call()` - Initiate outbound call
- `send_bye()` - Terminate call
- `send_update()` - Send re-INVITE
- `send_response()` - Send SIP response

## Events (Dialog → Session)
- `IncomingCall` - New incoming call
- `CallEstablished` - Call connected
- `CallTerminated` - Call ended
- `DialogStateChanged` - State transition
```

#### 6.2 Create Type-Safe Interfaces

**New file:**
- `crates/session-core-v2/src/interfaces/mod.rs`

```rust
/// Commands session-core can send to dialog-core
pub trait DialogCommands {
    async fn initiate_call(&self, from: &str, to: &str, sdp: Option<String>) -> Result<DialogId>;
    async fn terminate_call(&self, dialog_id: DialogId) -> Result<()>;
    async fn send_response(&self, dialog_id: DialogId, code: u16, sdp: Option<String>) -> Result<()>;
}

/// Events session-core receives from dialog-core
pub trait DialogEventHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> Result<()>;
    async fn on_call_established(&self, event: CallEstablishedEvent) -> Result<()>;
    async fn on_call_terminated(&self, event: CallTerminatedEvent) -> Result<()>;
}
```

### Phase 7: Reinforce Event-Driven Principles

#### 7.1 Ensure State Machine Centrality

**Actions:**
1. Audit all event handlers to ensure they only trigger state transitions
2. Move any direct actions from event handlers into state machine actions
3. Ensure no business logic exists outside the state machine

**Example refactor:**
```rust
// BAD: Event handler taking direct action
async fn handle_media_quality_degraded(&self, event: MediaQualityDegraded) {
    // Don't do this - handler is making decisions
    if event.packet_loss > 10.0 {
        self.media_adapter.switch_codec("OPUS").await?;
    }
}

// GOOD: Event handler triggers state transition
async fn handle_media_quality_degraded(&self, event: MediaQualityDegraded) {
    // Let state machine decide what to do
    self.state_machine.process_event(
        &SessionId(event.session_id),
        EventType::MediaQualityDegraded { 
            metrics: event.metrics 
        }
    ).await?;
}
```

#### 7.2 Document Event Flow Patterns

Create examples showing common event-driven patterns:
1. **Call Setup**: IncomingCall → State: Proceeding → Action: create_media_session() → MediaSessionCreated → State: Negotiating
2. **Quality Adaptation**: PacketLossDetected → State: Evaluating → Action: analyze_metrics() → Decision → Action: switch_codec()
3. **Error Recovery**: RtpTimeout → State: Recovering → Action: reinvite() → ReinviteAcked → State: Active

## Implementation Order

1. **Week 1**: Remove all channel-based communication (Phase 1)
2. **Week 1**: Fix event deserialization (Phase 2)
3. **Week 2**: Add missing events (Phase 3)
4. **Week 2**: Fix resource creation semantics (Phase 4)
5. **Week 3**: Clean up event flow (Phase 5)
6. **Week 3**: Document interfaces (Phase 6)
7. **Week 4**: Reinforce event-driven principles (Phase 7)

## Testing Strategy

1. **Unit Tests**: Test each event handler individually
2. **Integration Tests**: Test full event flow scenarios
3. **Example Verification**: Ensure api_peer_audio still works after each phase

## Success Criteria

- [ ] Zero channel-based events in the codebase
- [ ] All events use typed structures (no string parsing)
- [ ] Complete event coverage for all state changes
- [ ] Clear, documented interfaces between layers
- [ ] api_peer_audio example works with bidirectional audio
- [ ] No backward compatibility code remains
- [ ] All event handlers only trigger state transitions (no direct actions)
- [ ] State machine is the sole decision maker for actions
- [ ] Event flows are documented and follow patterns

## Compatibility with Event-Driven Architecture

This plan is fully compatible with the event-driven nature of the system:

1. **Deterministic Operations**: Direct function calls are used only for deterministic operations (resource creation, configuration) that session-core initiates
2. **Event-Driven Behavior**: All dynamic behavior emerges from events triggering state transitions
3. **State Machine Authority**: The state machine remains the central authority for all decisions
4. **No Hidden Logic**: Business logic exists only in the state machine, not in event handlers or adapters

The key insight is that while session-core **does** create instances and call functions in dialog-core and media-core, these are primarily for:
- Initial resource setup
- Configuration
- Cleanup
- Executing decisions made by the state machine

All dynamic, reactive behavior flows through events and state transitions.

## Rollback Plan

Each phase should be implemented in a separate branch and tested thoroughly before merging. If issues arise, we can rollback individual phases without affecting the entire migration.
