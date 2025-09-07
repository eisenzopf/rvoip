# Session-Core-V2 Simplification Plan

## Goal
Remove redundant management layers and make the state machine the single source of truth for all coordination.

## Architecture Changes

### Before (Complex):
```
API Layer (simple.rs/unified.rs)
    ↓
Management Layer (SessionManager, CallController, ConferenceManager)
    ↓
State Machine
    ↓
Adapters (DialogAdapter, MediaAdapter)
    ↓
External Libraries (dialog-core, media-core)
```

### After (Simple):
```
API Layer (simple.rs/unified.rs) - Thin wrapper
    ↓
State Machine - All coordination logic
    ↓
Adapters (DialogAdapter, MediaAdapter) - Protocol translation only
    ↓
External Libraries (dialog-core, media-core)
```

## Implementation Steps

### 1. Enhance State Table (state_tables/default_state_table.yaml)
Add missing states and transitions:
- Conference states (ConferenceHost, InConference)
- Bridge states (Bridged)
- All call control transitions
- Error handling transitions

### 2. Enhance State Machine (state_machine/executor.rs)
Add convenience methods:
- `create_session(from, to, role) -> SessionId`
- `get_session_info(session_id) -> SessionInfo`
- `list_active_sessions() -> Vec<SessionInfo>`
- `subscribe_to_events(session_id, callback)`

### 3. Simplify API Layer (api/simple.rs and api/unified.rs)
Transform into thin wrappers:
```rust
// Instead of complex orchestration
pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
    let session_id = SessionId::new();
    self.state_machine.create_session(session_id.clone(), from, to, Role::UAC).await?;
    self.state_machine.process_event(
        &session_id,
        EventType::MakeCall { target: to.to_string() }
    ).await?;
    Ok(session_id)
}
```

### 4. Remove Redundant Files
Delete:
- `api/session_manager.rs`
- `api/call_controller.rs`
- `api/conference_manager.rs`

### 5. Update Session Registry
Keep only the registry for ID mappings (it's still useful for adapters).

## Benefits

1. **Single Source of Truth**: All business logic in state table
2. **No Duplication**: No redundant session tracking
3. **Easy to Understand**: Linear flow from API to state machine to adapters
4. **Easy to Modify**: Change behavior by editing YAML only
5. **Better Testing**: Test state transitions, not implementation details

## Example: Conference Feature

Instead of ConferenceManager, add to state table:

```yaml
states:
  - name: "ConferenceHost"
    description: "Hosting a conference call"
  - name: "InConference"
    description: "Participating in a conference"

transitions:
  # Create conference
  - role: "Both"
    state: "Active"
    event:
      type: "CreateConference"
    actions:
      - type: "CreateAudioMixer"
      - type: "RedirectToMixer"
    next_state: "ConferenceHost"
    
  # Add participant
  - role: "Both"
    state: "ConferenceHost"
    event:
      type: "AddParticipant"
    actions:
      - type: "BridgeToMixer"
    
  # Join conference
  - role: "Both"
    state: "Active"
    event:
      type: "JoinConference"
    actions:
      - type: "ConnectToMixer"
    next_state: "InConference"
```

## Timeline

1. First, enhance state table with all features
2. Update state machine actions to support new features
3. Simplify API layer
4. Remove redundant managers
5. Update tests
