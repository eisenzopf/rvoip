# Master State Table Design for Session-Core

## The Problem We're Solving

Currently, session-core is juggling:
- **2 Independent Layers**: dialog-core (SIP) and media-core (RTP)
- **2 Roles**: UAC (caller) and UAS (receiver)
- **9 Call States**: Initiating, Ringing, Active, OnHold, etc.
- **30+ Event Types**: From both layers plus internal events
- **3 Readiness Conditions**: That must all be true before audio flows
- **4 Different MediaFlowEstablished publishing points**: Depending on role and timing

This creates a combinatorial explosion of possibilities that's hard to reason about.

## Master State Table Concept

Instead of scattered logic, we have ONE table that answers: 
**"Given my current state and this event, what should I do?"**

```rust
pub struct MasterStateTable {
    // The single source of truth
    transitions: HashMap<StateKey, Transition>,
}

pub struct StateKey {
    role: Role,                    // UAC or UAS
    call_state: CallState,          // Current call state
    event_type: EventType,          // What just happened
    conditions: ConditionFlags,     // Current readiness flags
}

pub struct Transition {
    // Guards - must be true to take this transition
    required_conditions: ConditionFlags,
    
    // Actions - what to do
    actions: Vec<Action>,
    
    // State changes
    next_state: Option<CallState>,
    condition_updates: ConditionFlags,
    
    // Events to publish
    publish_events: Vec<EventTemplate>,
}
```

## The Master Table Structure

### Layer 1: Role-Specific Tables

```rust
// Separate tables for each role to reduce complexity
enum MasterTable {
    UAC(UACStateTable),
    UAS(UASStateTable),
    Common(CommonStateTable),  // Shared transitions
}
```

### Layer 2: State Context

```rust
// Everything we need to know about current state
struct SessionContext {
    // Identity
    session_id: SessionId,
    role: Role,
    
    // Current state
    call_state: CallState,
    dialog_state: DialogState,
    media_state: MediaState,
    
    // Readiness flags (the 3 conditions)
    dialog_established: bool,
    media_session_ready: bool,
    sdp_negotiated: bool,
    
    // Data
    local_sdp: Option<String>,
    remote_sdp: Option<String>,
    negotiated_config: Option<NegotiatedConfig>,
    
    // Timestamps
    created_at: Instant,
    state_entered_at: Instant,
}
```

### Layer 3: Event Classification

```rust
// Events grouped by source and purpose
enum EventClass {
    // From dialog-core (SIP layer)
    DialogEvent {
        event: DialogEventType,
        data: DialogEventData,
    },
    
    // From media-core (RTP layer)
    MediaEvent {
        event: MediaEventType,
        data: MediaEventData,
    },
    
    // Internal coordination events
    InternalEvent {
        event: InternalEventType,
    },
    
    // Timer events (if any)
    TimerEvent {
        timer_id: TimerId,
    },
}
```

## Master State Table for UAC (Caller)

| Current State | Event | Conditions Required | Actions | Next State | Publish Events |
|--------------|-------|-------------------|---------|------------|----------------|
| `Idle` | `MakeCall` | - | CreateSession, SendINVITE | `Initiating` | `SessionCreated` |
| `Initiating` | `Dialog:180Ringing` | - | UpdateState | `Ringing` | `StateChanged` |
| `Ringing` | `Dialog:200OK` | Has SDP answer | SendACK, NegotiateSDP | `Active` | `StateChanged` |
| `Active` | `Internal:ACKSent` | - | - | `Active` | `MediaEvent(uac)` |
| `Active` | `Media:Negotiated` | - | SetFlag(sdp_negotiated) | `Active` | `MediaNegotiated` |
| `Active` | `Media:SessionReady` | - | SetFlag(media_ready) | `Active` | `MediaSessionReady` |
| `Active` | `Internal:CheckReady` | All 3 flags true | CallEstablished | `Active` | `MediaFlowEstablished` |
| `Active` | `Dialog:BYE` | - | SendOK, Cleanup | `Terminating` | `SessionTerminating` |
| `Terminating` | `Media:Cleaned` | - | SetFlag(media_done) | `Terminating` | `CleanupConfirm` |
| `Terminating` | `Internal:AllClean` | All cleanup done | RemoveSession | `Terminated` | `SessionTerminated` |

## Master State Table for UAS (Receiver)

| Current State | Event | Conditions Required | Actions | Next State | Publish Events |
|--------------|-------|-------------------|---------|------------|----------------|
| `Idle` | `Dialog:INVITE` | Has SDP offer | CreateSession, NegotiateSDP | `Initiating` | `IncomingCall` |
| `Initiating` | `Handler:Accept` | - | Send200OK | `Active` | `StateChanged` |
| `Active` | `Dialog:ACK` | - | StartMedia | `Active` | `MediaEvent(uas)` |
| `Active` | `Internal:UASMedia` | Has negotiated | - | `Active` | `MediaFlowEstablished` |
| `Active` | `Media:Negotiated` | - | SetFlag(sdp_negotiated) | `Active` | `MediaNegotiated` |
| `Active` | `Media:SessionReady` | - | SetFlag(media_ready) | `Active` | `MediaSessionReady` |
| `Active` | `Internal:CheckReady` | All 3 flags true | CallEstablished | `Active` | - |
| `Active` | `User:Hangup` | - | SendBYE | `Terminating` | `SessionTerminating` |

## How Events Flow Through the Table

```rust
async fn process_event(
    &mut self,
    session_id: &SessionId,
    event: EventClass,
) -> Result<()> {
    // 1. Get current context
    let context = self.get_context(session_id)?;
    
    // 2. Build the state key
    let key = StateKey {
        role: context.role,
        call_state: context.call_state,
        event_type: event.classify(),
        conditions: context.get_condition_flags(),
    };
    
    // 3. Look up transition
    let transition = self.master_table.get(&key)
        .ok_or("Invalid transition")?;
    
    // 4. Check guards
    if !transition.check_conditions(&context) {
        return Ok(()); // Not ready yet
    }
    
    // 5. Execute actions
    for action in &transition.actions {
        self.execute_action(action, &mut context).await?;
    }
    
    // 6. Update state
    if let Some(next_state) = transition.next_state {
        context.call_state = next_state;
    }
    context.apply_condition_updates(transition.condition_updates);
    
    // 7. Publish events
    for event_template in &transition.publish_events {
        self.publish_event(event_template.instantiate(&context)).await?;
    }
    
    // 8. Check if this triggered any internal conditions
    if context.all_conditions_met() && !context.call_established_triggered {
        self.process_event(session_id, EventClass::Internal(CheckReady)).await?;
    }
    
    Ok(())
}
```

## Action Types

```rust
enum Action {
    // Dialog actions
    SendSIPMessage(SIPMessageType),
    UpdateDialogState(DialogState),
    
    // Media actions  
    StartMediaSession,
    StopMediaSession,
    NegotiateSDPAsUAC(String),
    NegotiateSDPAsUAS(String),
    
    // State updates
    SetConditionFlag(ConditionType, bool),
    StoreLocalSDP(String),
    StoreRemoteSDP(String),
    StoreNegotiatedConfig(NegotiatedConfig),
    
    // Handler callbacks
    NotifyHandler(HandlerEvent),
    
    // Cleanup
    StartCleanup(CleanupType),
    MarkCleanupComplete(LayerType),
}
```

## Advantages of Master State Table

### 1. **Single Source of Truth**
- All transitions in one place
- No scattered condition checking
- Easy to audit and verify

### 2. **Deterministic Behavior**
```rust
// Given a state and event, outcome is always the same
assert_eq!(
    table.lookup(state, event),
    table.lookup(state, event)
);
```

### 3. **Testability**
```rust
#[test]
fn test_uac_normal_flow() {
    let table = MasterStateTable::new();
    let mut context = SessionContext::new_uac();
    
    // Each transition is independently testable
    table.process(context, MakeCall);
    assert_eq!(context.call_state, Initiating);
    
    table.process(context, Dialog_200OK);
    assert_eq!(context.call_state, Active);
    // etc...
}
```

### 4. **Debugging**
```rust
// Can log every transition with full context
log::info!("Transition: {:?} + {:?} -> {:?}", 
    state, event, next_state);
```

### 5. **Formal Verification**
```rust
// Can prove properties about the state machine
fn verify_no_orphan_states(table: &MasterStateTable) -> bool {
    // Every state must have at least one exit transition
    for state in CallState::all() {
        if !table.has_exit_transition(state) {
            return false;
        }
    }
    true
}
```

## Handling Complex Scenarios

### Race Condition: MediaFlowEstablished Timing
**Old Way**: Complex logic checking when to publish based on role and state
**New Way**: Explicit entries in the table

```rust
// UAC publishes after SDP negotiation
(UAC, Active, Media:Negotiated, _) -> publish MediaFlowEstablished

// UAS publishes after ACK received  
(UAS, Active, Internal:UASMedia, has_negotiated) -> publish MediaFlowEstablished
```

### Edge Case: Re-INVITE
**Old Way**: Special handling scattered through code
**New Way**: Just more rows in the table

```rust
(*, Active, Dialog:INVITE, _) -> MediaUpdate -> Active
(*, Active, Media:Updated, _) -> SetFlag(sdp_negotiated) -> Active
```

## Implementation Strategy

### Phase 1: Build the Table
```rust
lazy_static! {
    static ref MASTER_TABLE: MasterStateTable = {
        let mut table = MasterStateTable::new();
        
        // UAC transitions
        table.add_uac_transitions();
        
        // UAS transitions  
        table.add_uas_transitions();
        
        // Common transitions
        table.add_common_transitions();
        
        table.validate().expect("Invalid state table");
        table
    };
}
```

### Phase 2: Migrate Event Handlers
Replace current event handlers with:
```rust
impl SessionCoordinator {
    async fn handle_event(&self, event: SessionEvent) -> Result<()> {
        let session_id = event.session_id();
        let event_class = EventClass::from(event);
        
        MASTER_TABLE.process_event(
            &mut self.get_context(session_id)?,
            event_class
        ).await
    }
}
```

### Phase 3: Validate Completeness
```rust
// Ensure every possible state+event combo is handled
for state in CallState::all() {
    for event in EventType::all() {
        assert!(
            table.has_transition(state, event) ||
            table.has_default(state, event),
            "Missing transition: {:?} + {:?}", state, event
        );
    }
}
```

## Example: Complete UAC Call Flow in Table

| Step | State | Event | Actions | Next State |
|------|-------|-------|---------|------------|
| 1 | Idle | MakeCall | CreateSession, SendINVITE | Initiating |
| 2 | Initiating | Timer:100ms | SendTrying | Initiating |
| 3 | Initiating | Dialog:180 | - | Ringing |
| 4 | Ringing | Dialog:200OK | SendACK, NegotiateSDP | Active |
| 5 | Active | Internal:ACKSent | SetFlag(dialog_established) | Active |
| 6 | Active | Media:Negotiated | SetFlag(sdp_negotiated) | Active |
| 7 | Active | Media:Ready | SetFlag(media_ready) | Active |
| 8 | Active | Internal:AllReady | TriggerCallEstablished, PublishMediaFlow | Active |
| 9 | Active | User:Hangup | SendBYE | Terminating |
| 10 | Terminating | Dialog:200OK | StartCleanup | Terminating |
| 11 | Terminating | Media:Cleaned | MarkMediaDone | Terminating |
| 12 | Terminating | Internal:AllClean | RemoveSession | Terminated |

## Conclusion

A master state table would:
1. **Reduce complexity** from O(states × events × conditions) to O(1) lookup
2. **Eliminate race conditions** by making all transitions explicit
3. **Improve maintainability** by centralizing all logic
4. **Enable formal verification** of correctness
5. **Simplify debugging** with complete transition logging

The key insight is that session-core is fundamentally a **state machine coordinator** between two other state machines (dialog-core and media-core). By making this explicit with a master table, we can manage the complexity in a structured, testable way.