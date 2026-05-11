# Adapter Architecture Explanation

## Overview

The session-core-v2 library uses an **Adapter Pattern** to cleanly separate concerns:
- **State Machine**: Pure business logic that doesn't know about SIP or RTP specifics
- **Adapters**: Translation layer between external protocols and state machine events
- **State Table**: Declarative configuration of state transitions

## Key Components

### 1. State Machine (`state_machine/executor.rs`)
The state machine is the **brain** of the system:
- Reads state transitions from the YAML state table
- Executes transitions based on incoming events
- Triggers actions but doesn't implement them
- Completely protocol-agnostic

**Example**: The state machine knows about events like "IncomingCall" or "Dialog200OK" but doesn't know these come from SIP.

### 2. Adapters (`adapters/`)
Adapters are the **translators** between external libraries and the state machine:

#### DialogAdapter (`dialog_adapter.rs`)
- **Purpose**: Translates between dialog-core (SIP) and state machine events
- **Incoming**: Converts SIP messages (INVITE, 200 OK, BYE) → State machine events
- **Outgoing**: Executes state machine actions → SIP operations

```rust
// Example: SIP INVITE arrives
dialog-core → DialogAdapter → "IncomingCall" event → State Machine

// Example: State machine says "SendINVITE"
State Machine → "SendINVITE" action → DialogAdapter → dialog-core
```

#### MediaAdapter (`media_adapter.rs`)
- **Purpose**: Translates between media-core (RTP/Audio) and state machine events
- **Incoming**: Media events (flow established, error) → State machine events
- **Outgoing**: State machine actions → Media operations

```rust
// Example: RTP flow established
media-core → MediaAdapter → "MediaSessionReady" event → State Machine

// Example: State machine says "StartMediaFlow"
State Machine → "StartMediaFlow" action → MediaAdapter → media-core
```

#### SignalingInterceptor (`signaling_interceptor.rs`)
- **Purpose**: Creates new sessions for incoming calls
- **Special Role**: The ONLY component that creates sessions from external events
- Without this, incoming calls would be ignored

#### EventRouter (`event_router.rs`)
- **Purpose**: Distributes events to the right handlers
- Routes dialog events to DialogAdapter
- Routes media events to MediaAdapter
- Ensures proper event flow

### 3. State Table (`state_tables/default_state_table.yaml`)
The state table is the **configuration**:
- Defines all possible states
- Defines all transitions between states
- Specifies guards (conditions) and actions
- Completely declarative - no code

## Why This Architecture?

### 1. **Separation of Concerns**
```
SIP Protocol Details → DialogAdapter → Generic Events → State Machine
RTP/Media Details → MediaAdapter → Generic Events → State Machine
```

The state machine never needs to know about SIP headers, SDP negotiation, or RTP packets.

### 2. **Testability**
- State machine can be tested with mock events
- Adapters can be tested independently
- State table can be validated without running code

### 3. **Flexibility**
- Change behavior by editing YAML, not code
- Add new protocols by adding new adapters
- State machine remains unchanged

### 4. **Maintainability**
- Protocol changes only affect adapters
- Business logic changes only affect state table
- Clear boundaries between components

## Event Flow Example: Making a Call

1. **API Call**: `simple.rs` → `make_call()`
2. **State Machine**: Receives "MakeCall" event
3. **State Table**: Looks up transition from "Idle" state with "MakeCall" event
4. **Actions**: State table says execute "SendINVITE" action
5. **DialogAdapter**: Receives "SendINVITE" action, calls dialog-core
6. **SIP Protocol**: dialog-core sends actual SIP INVITE
7. **Response**: SIP 200 OK arrives at dialog-core
8. **DialogAdapter**: Translates to "Dialog200OK" event
9. **State Machine**: Transitions to "Active" state
10. **MediaAdapter**: Gets "StartMediaFlow" action, starts RTP

## Common Confusion Points

### Q: Why not put SIP logic in the state machine?
**A**: This would couple the state machine to SIP. With adapters, we could theoretically support other protocols (like WebRTC signaling) by just adding new adapters.

### Q: Why is the state table in YAML?
**A**: Declarative configuration is easier to understand, validate, and modify than code. You can see all possible state transitions in one place.

### Q: What happens without adapters?
**A**: The state machine would need to understand SIP messages, RTP packets, etc. This would make it complex, hard to test, and tightly coupled to specific protocols.

### Q: Why is SignalingInterceptor special?
**A**: For outgoing calls, the API creates the session. For incoming calls, something needs to create the session when the INVITE arrives - that's the SignalingInterceptor's job.

## Benefits of This Design

1. **Protocol Independence**: State machine doesn't care if you're using SIP, WebRTC, or something else
2. **Easy to Extend**: Add new features by updating the state table
3. **Clear Responsibilities**: Each component has one job
4. **Debugging**: Can trace events through the system easily
5. **Testing**: Can test business logic without real SIP/RTP

## Summary

Think of it like a restaurant:
- **State Machine**: The chef who follows recipes
- **State Table**: The cookbook with all recipes
- **Adapters**: The waiters who translate between customers and chef
- **External Libraries**: The customers speaking different languages (SIP, RTP)

The chef doesn't need to speak the customer's language - the waiters handle translation. The chef just needs to know how to follow recipes from the cookbook.
