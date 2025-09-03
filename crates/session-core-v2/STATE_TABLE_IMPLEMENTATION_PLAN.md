# State Table Implementation Plan

## Executive Summary

### Scope of Changes
This is a **major architectural refactor** that would fundamentally change how session-core coordinates between dialog-core and media-core. The current event-driven approach with scattered condition checking would be replaced with a centralized state table.

### Recommendation: Create `session-core-v2` Crate
Given the scope, I strongly recommend creating a new crate (`session-core-v2` or `session-core-table`) to:
1. Preserve the working implementation
2. Allow gradual migration
3. A/B test both approaches
4. Keep the API interface stable

## Impact Assessment

### Complexity Reduction
| Component | Current Lines | Estimated New | Reduction |
|-----------|--------------|---------------|-----------|
| event_handler.rs | ~1400 | ~200 | 85% |
| coordinator.rs | ~800 | ~300 | 62% |
| Multiple condition checks | ~500 | 0 (in table) | 100% |
| **Total Core Logic** | **~2700** | **~500** | **81%** |

### New Complexity Added
| Component | Lines | Purpose |
|-----------|-------|---------|
| state_table.rs | ~600 | Table definition |
| state_machine.rs | ~400 | Table executor |
| transitions/uac.rs | ~300 | UAC transitions |
| transitions/uas.rs | ~300 | UAS transitions |
| **Total New** | **~1600** | |

### Net Result
- **Core logic**: 2700 → 500 lines (-81%)
- **Table definition**: 0 → 1600 lines (+1600)
- **Total**: 2700 → 2100 lines (-22%)
- **But**: Logic moves from imperative to declarative (huge win for correctness)

## File Structure Changes

### New Crate Structure
```
crates/session-core-v2/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── api/                    # Unchanged - preserve interface
│   │   ├── mod.rs
│   │   ├── types.rs            # Copy from current
│   │   ├── control.rs          # Copy from current
│   │   ├── media.rs            # Copy from current
│   │   └── call.rs             # Copy from current
│   │
│   ├── state_table/            # NEW - Core innovation
│   │   ├── mod.rs
│   │   ├── types.rs            # StateKey, Transition, Action
│   │   ├── builder.rs          # Table construction
│   │   ├── validator.rs        # Compile-time validation
│   │   └── tables/
│   │       ├── mod.rs
│   │       ├── uac.rs          # UAC transition table
│   │       ├── uas.rs          # UAS transition table
│   │       └── common.rs       # Shared transitions
│   │
│   ├── state_machine/          # NEW - Execution engine
│   │   ├── mod.rs
│   │   ├── executor.rs         # Main state machine
│   │   ├── actions.rs          # Action implementations
│   │   ├── guards.rs           # Condition checks
│   │   └── effects.rs          # Side effects
│   │
│   ├── session_store/          # REFACTORED - Simplified storage
│   │   ├── mod.rs
│   │   ├── store.rs            # Session storage
│   │   ├── state.rs            # SessionState struct
│   │   └── history.rs          # Optional history tracking
│   │
│   ├── coordinator/            # SIMPLIFIED - Thin wrapper
│   │   ├── mod.rs
│   │   └── coordinator.rs      # Delegates to state machine
│   │
│   └── adapters/               # NEW - Bridge to existing layers
│       ├── mod.rs
│       ├── dialog_adapter.rs   # dialog-core integration
│       └── media_adapter.rs    # media-core integration
```

## Detailed File Changes

### 1. Core State Table Definition
**File**: `src/state_table/types.rs`
```rust
pub struct StateKey {
    pub role: Role,
    pub state: CallState,
    pub event: EventType,
}

pub struct Transition {
    pub guards: Vec<Guard>,
    pub actions: Vec<Action>,
    pub next_state: Option<CallState>,
    pub condition_updates: ConditionUpdates,
    pub publish_events: Vec<EventTemplate>,
}

pub enum Action {
    // Dialog actions
    SendSIPResponse(u16, String),
    SendACK,
    SendBYE,
    
    // Media actions
    StartMediaSession,
    StopMediaSession,
    NegotiateSDPAsUAC,
    NegotiateSDPAsUAS,
    
    // State updates
    SetCondition(Condition, bool),
    StoreLocalSDP,
    StoreRemoteSDP,
    
    // Callbacks
    TriggerCallEstablished,
}
```

### 2. UAC Transition Table
**File**: `src/state_table/tables/uac.rs`
```rust
pub fn build_uac_table() -> Vec<(StateKey, Transition)> {
    vec![
        // Initiating + 180 Ringing → Ringing
        (
            StateKey {
                role: Role::UAC,
                state: CallState::Initiating,
                event: EventType::Dialog180Ringing,
            },
            Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(CallState::Ringing),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![EventTemplate::StateChanged],
            }
        ),
        
        // Ringing + 200 OK → Active
        (
            StateKey {
                role: Role::UAC,
                state: CallState::Ringing,
                event: EventType::Dialog200OK,
            },
            Transition {
                guards: vec![Guard::HasRemoteSDP],
                actions: vec![
                    Action::SendACK,
                    Action::NegotiateSDPAsUAC,
                ],
                next_state: Some(CallState::Active),
                condition_updates: ConditionUpdates {
                    dialog_established: Some(true),
                    ..Default::default()
                },
                publish_events: vec![
                    EventTemplate::StateChanged,
                    EventTemplate::MediaEvent("rfc_compliant_media_creation_uac"),
                ],
            }
        ),
        
        // ... 50+ more transitions
    ]
}
```

### 3. State Machine Executor
**File**: `src/state_machine/executor.rs`
```rust
pub struct StateMachine {
    table: Arc<MasterTable>,
    store: Arc<SessionStore>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
}

impl StateMachine {
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: Event,
    ) -> Result<()> {
        // 1. Get current state
        let mut session = self.store.get_session(session_id)?;
        
        // 2. Look up transition
        let key = StateKey {
            role: session.role,
            state: session.state,
            event: event.event_type(),
        };
        
        let transition = self.table.get(&key)
            .ok_or_else(|| Error::InvalidTransition(key))?;
        
        // 3. Check guards
        for guard in &transition.guards {
            if !self.check_guard(guard, &session).await? {
                return Ok(()); // Not ready
            }
        }
        
        // 4. Execute actions
        for action in &transition.actions {
            self.execute_action(action, &mut session).await?;
        }
        
        // 5. Update state
        if let Some(next_state) = transition.next_state {
            session.transition_to(next_state);
        }
        
        // 6. Update conditions
        session.apply_condition_updates(&transition.condition_updates);
        
        // 7. Save state
        self.store.update_session(session)?;
        
        // 8. Publish events
        for event_template in &transition.publish_events {
            let event = event_template.instantiate(&session);
            self.publish_event(event).await?;
        }
        
        // 9. Check for triggered conditions
        if session.all_conditions_met() && !session.call_established {
            self.process_event(
                session_id,
                Event::Internal(InternalEvent::CheckCallEstablished)
            ).await?;
        }
        
        Ok(())
    }
}
```

### 4. Simplified Coordinator
**File**: `src/coordinator/coordinator.rs`
```rust
pub struct SessionCoordinator {
    state_machine: Arc<StateMachine>,
    event_receiver: mpsc::Receiver<Event>,
}

impl SessionCoordinator {
    pub async fn run(mut self) {
        while let Some(event) = self.event_receiver.recv().await {
            if let Some(session_id) = event.session_id() {
                if let Err(e) = self.state_machine.process_event(&session_id, event).await {
                    log::error!("Failed to process event: {}", e);
                }
            }
        }
    }
}
```

## Migration Strategy

### Phase 1: Create New Crate (Week 1)
1. Copy `session-core` to `session-core-v2`
2. Keep API layer unchanged
3. Set up new project structure

### Phase 2: Build State Tables (Week 2-3)
1. Define all state transitions
2. Create table builder
3. Validate completeness

### Phase 3: Implement State Machine (Week 3-4)
1. Build executor
2. Implement actions
3. Create adapters

### Phase 4: Testing (Week 5-6)
1. Unit test each transition
2. Integration test with real calls
3. Fuzz testing for edge cases

### Phase 5: Migration (Week 7-8)
1. Run both implementations in parallel
2. Compare outputs
3. Gradually switch traffic

## Risk Assessment

### Risks
| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| Missing edge cases | Medium | High | Extensive testing, parallel run |
| Performance regression | Low | Medium | Benchmark before/after |
| Integration issues | Medium | High | Keep adapters thin |
| Incomplete table | High | High | Automated validation |

### Benefits
| Benefit | Certainty | Impact |
|---------|-----------|--------|
| Reduced bugs | High | High |
| Easier debugging | High | High |
| Formal verification | Medium | Very High |
| Faster development | Medium | Medium |
| Better documentation | High | Medium |

## Code Comparison

### Current Approach (Scattered Logic)
```rust
// In event_handler.rs
if event == "rfc_compliant_media_creation_uas" {
    if let Some(negotiated) = self.get_negotiated_config(&session_id).await {
        tracing::info!("Publishing MediaFlowEstablished for UAS {}", session_id);
        let _ = self.publish_event(SessionEvent::MediaFlowEstablished {
            session_id: session_id.clone(),
            local_addr: negotiated.local_addr.to_string(),
            remote_addr: negotiated.remote_addr.to_string(),
            direction: MediaFlowDirection::Both,
        }).await;
    }
}

// Plus similar logic in 3 other places...
```

### New Approach (Table Entry)
```rust
// In uas.rs table
(
    StateKey {
        role: Role::UAS,
        state: CallState::Active,
        event: EventType::MediaEvent("rfc_compliant_media_creation_uas"),
    },
    Transition {
        guards: vec![Guard::HasNegotiatedConfig],
        actions: vec![],
        next_state: None,
        condition_updates: ConditionUpdates::none(),
        publish_events: vec![EventTemplate::MediaFlowEstablished],
    }
),
```

## Decision Points

### Option 1: Full Rewrite in New Crate ✅ (Recommended)
**Pros**:
- Clean slate
- No risk to production
- Can run both in parallel
- Easier to compare

**Cons**:
- Duplicate code initially
- Longer timeline
- Need to maintain both

### Option 2: Gradual Refactor in Place
**Pros**:
- No duplication
- Immediate benefits
- Single codebase

**Cons**:
- High risk
- Hard to rollback
- Complex migration

### Option 3: Hybrid Approach
**Pros**:
- Start with high-value paths
- Learn as we go
- Lower initial investment

**Cons**:
- Two systems in one
- Complexity during transition
- Harder to reason about

## Performance Analysis

### Current Implementation
- Event processing: ~500ns average
- Condition checking: ~200ns × 3-5 checks
- Total per event: ~1.5μs

### State Table Implementation
- Table lookup: ~100ns (HashMap)
- Guard checking: ~50ns × 2-3 guards
- Action execution: ~300ns
- Total per event: ~500ns

**Expected improvement**: 3× faster event processing

## Maintenance Benefits

### Current: Finding All MediaFlowEstablished Publishing
```bash
# Need to grep through multiple files
grep -r "MediaFlowEstablished" src/
# Returns 4 different locations with different conditions
```

### New: Finding All MediaFlowEstablished Publishing
```rust
// Just search the table
table.transitions
    .filter(|t| t.publish_events.contains(&EventTemplate::MediaFlowEstablished))
    .collect()
// Returns all transitions that publish this event
```

## Recommendation

### Create `session-core-v2` as Experimental Crate

**Why**:
1. **Risk Mitigation**: Current system works, don't break it
2. **Clean Architecture**: Start fresh without legacy constraints
3. **Parallel Testing**: Run both, compare outputs
4. **Gradual Migration**: Switch when confident
5. **Preservation**: Keep working implementation as reference

**Timeline**: 6-8 weeks to production-ready

**Success Metrics**:
- 100% of current tests pass
- 50% reduction in event handler code
- 0 race conditions in state transitions
- 3× improvement in event processing time

This is a **major architectural change** but the benefits (correctness, maintainability, performance) justify the investment, especially if done safely in a new crate.