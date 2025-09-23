# Helper Methods vs Message Passing

## What CAN be done through message passing (Adapters)

The adapters handle all protocol-specific operations by passing messages to external crates:

### DialogAdapter (SIP Protocol)
- Send SIP messages: INVITE, ACK, BYE, CANCEL, etc.
- Handle SIP responses: 180 Ringing, 200 OK, etc.
- Manage SIP dialogs and transactions
- SDP offer/answer exchange
- REFER for transfers
- Re-INVITE for hold/resume

### MediaAdapter (RTP/Audio)
- Start/stop RTP flows
- Negotiate codecs
- Create audio mixers for conferences
- Send DTMF tones
- Handle audio routing/bridging
- Mute/unmute operations
- Recording start/stop

## What CAN'T be done through message passing (Helper Methods)

These operations require direct access to internal state or coordination:

### 1. Session Management
```rust
// Creating a session requires allocating storage
create_session(id, from, to, role)

// Cleaning up requires removing from multiple stores
cleanup_session(id)
```

### 2. State Queries
```rust
// Need direct access to session store
get_session_info(id) -> SessionInfo
get_state(id) -> CallState
list_sessions() -> Vec<SessionInfo>
is_in_conference(id) -> bool
get_session_history(id) -> Vec<TransitionRecord>
```

### 3. Subscription Management
```rust
// Need to maintain callback registry
subscribe(id, callback)
unsubscribe(id)
notify_subscribers(id, event)
```

### 4. Complex Coordinated Operations
```rust
// Conference creation needs to track multiple sessions
create_conference_from_call(session_id)
add_to_conference(host_id, participant_id)
bridge_calls(session1_id, session2_id)
```

### 5. Performance & Debugging
```rust
// Need access to timing data across components
get_call_metrics(id) -> CallMetrics
get_transition_history(id) -> Vec<TransitionRecord>
debug_session_state(id) -> DebugInfo
```

### 6. Convenience Methods
```rust
// High-level operations that coordinate multiple events
make_call(from, to) -> SessionId  // Creates session + sends MakeCall
transfer_with_consultation(id, target) // Coordinates multiple sessions
```

## Architecture Benefits

### Clean Separation
- **State Machine**: Pure coordination logic
- **Adapters**: Protocol-specific message passing
- **Helpers**: Convenience and query methods

### Single Source of Truth
- All business logic in state table (YAML)
- Helpers just provide convenient access
- No duplicate state management

### Example Flow

```rust
// In simple.rs
pub async fn make_call(&self, from: &str, to: &str) -> Result<CallId> {
    // Calls helper method
    self.helpers.make_call(from, to).await
}

// In helpers.rs
pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
    let id = SessionId::new();
    
    // 1. Create session (can't be done via message passing)
    self.create_session(id, from, to, Role::UAC).await?;
    
    // 2. Send event to state machine
    self.state_machine.process_event(id, EventType::MakeCall).await?;
    
    Ok(id)
}

// State machine processes event:
// - Looks up transition in state table
// - Executes actions via adapters (message passing)
// - Updates state
```

## Summary

- **Message Passing**: All protocol operations (SIP, RTP)
- **Helper Methods**: Session management, queries, subscriptions
- **State Machine**: Pure coordination using state table
