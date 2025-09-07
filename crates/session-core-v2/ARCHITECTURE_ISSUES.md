# Architecture Issues in Session-Core-V2

## The Problem

We've recreated the same complexity we were trying to avoid. We have:

1. **State Machine** - The coordination engine (GOOD!)
2. **SessionManager** - Duplicate session tracking (BAD!)
3. **CallController** - Duplicate call logic (BAD!)
4. **ConferenceManager** - Conference logic outside state machine (BAD!)

## What's Wrong

### Current Flow:
```
API → CallController → SessionManager → State Machine → Adapters → External Libs
```

### Should Be:
```
API → State Machine → Adapters → External Libs
```

## Why This Happened

The managers were added because:
1. The state machine wasn't exposing a simple enough API
2. Business logic was split between code and YAML
3. Conference features weren't in the state table

## The Solution

### 1. Remove Management Layers
- Delete SessionManager (state machine already tracks sessions)
- Delete CallController (state table already defines call flows)
- Delete ConferenceManager (add conference states to state table)

### 2. Enhance State Machine
- Add helper methods for common operations
- Expose clean API directly from state machine
- Put ALL business logic in state table

### 3. Simple API Layer
Keep simple.rs and unified.rs as thin wrappers that just:
- Convert user calls to state machine events
- Return results from state machine

## Example: How make_call() Should Work

### Current (BAD):
```rust
// In CallController
pub async fn make_call(&self, from: String, to: String) -> Result<SessionId> {
    let session_id = self.session_manager.create_session(...).await?;
    let dialog_id = self.dialog_adapter.create_dialog(...).await?;
    self.session_manager.map_dialog(...);
    let media_id = self.media_adapter.create_media_session().await?;
    self.session_manager.map_media(...);
    self.dialog_adapter.send_invite(...).await?;
    // ... lots of orchestration logic
}
```

### Should Be (GOOD):
```rust
// In simple.rs
pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
    let session_id = SessionId::new();
    self.state_machine.process_event(
        session_id,
        EventType::MakeCall { from, to }
    ).await?;
    Ok(session_id)
}
```

All the orchestration logic should be in the state table:
```yaml
- role: "UAC"
  state: "Idle"
  event:
    type: "MakeCall"
  actions:
    - type: "CreateDialog"
    - type: "CreateMediaSession"
    - type: "SendINVITE"
  next_state: "Initiating"
```

## Benefits of Fixing This

1. **Single Source of Truth**: State table defines ALL behavior
2. **No Duplicate Logic**: No coordination code outside state machine
3. **Easy to Modify**: Change behavior by editing YAML
4. **Clear Architecture**: Events in → State Machine → Actions out
5. **Testable**: Test the state machine, not multiple managers

## Conference Support

Add conference states to the state table:
```yaml
states:
  - name: "InConference"
    description: "Participant in a conference"
  - name: "ConferenceHost"
    description: "Hosting a conference"

transitions:
  - role: "Both"
    state: "Active"
    event:
      type: "JoinConference"
    actions:
      - type: "CreateMixer"
      - type: "RedirectToMixer"
    next_state: "InConference"
```

## Summary

The current architecture defeats the purpose of the refactor. We need to:
1. Trust the state machine to be the coordinator
2. Remove redundant management layers
3. Put ALL business logic in the state table
4. Keep the API layer thin
