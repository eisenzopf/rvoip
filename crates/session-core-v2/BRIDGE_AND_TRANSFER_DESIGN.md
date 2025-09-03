# Bridge and Transfer Design

## Overview

Session-core-v2 supports various call control features through its state table architecture:

## 1. Call Bridging (2-Party)

The system supports bridging two active calls together:

```rust
// Bridge two sessions
session1.send_event(EventType::BridgeSessions { 
    other_session: session2.id() 
})

// States: Active -> Bridged
// Actions: CreateBridge, media mixing coordination
```

### Supported Bridge Scenarios:
- **Active-to-Active**: Bridge two active calls
- **Hold-to-Active**: Bridge a held call with an active call (auto-resumes)
- **Bridge Destruction**: Automatic cleanup when either party hangs up

## 2. Call Transfer

### Blind Transfer (Immediate)
Transfer the call immediately without consultation:

```rust
session.send_event(EventType::BlindTransfer { 
    target: "sip:destination@example.com" 
})

// States: Active -> Transferring -> Terminated
// SIP: Sends REFER message to remote party
```

### Attended Transfer (Consultative)
Consult with the destination before transferring:

```rust
session.send_event(EventType::AttendedTransfer { 
    target: "sip:destination@example.com" 
})

// States: Active -> Transferring
// Actions: 
// 1. Put original call on hold
// 2. Create consultation call
// 3. Bridge or transfer based on consultation result
```

## 3. Conference Calls (3+ Parties)

For true multi-party conference calls (3 or more participants), use the separate conference crate:

```rust
// Conference functionality is in a separate crate
use rvoip_conference::{ConferenceRoom, ConferenceMixer};

// Create conference room
let room = ConferenceRoom::new();

// Add participants
room.add_participant(session1).await?;
room.add_participant(session2).await?;
room.add_participant(session3).await?;
```

### Why Separate Conference Crate?

1. **Complexity**: Conference calls require:
   - Audio mixing for N participants
   - Participant management (mute, kick, etc.)
   - Recording capabilities
   - Floor control

2. **Specialized Use Case**: Not all VoIP applications need conferencing

3. **Performance**: Conference mixing is CPU-intensive and benefits from specialized optimization

## State Table Entries

### Bridge States
```rust
CallState::Active -> CallState::Bridged     // Create bridge
CallState::OnHold -> CallState::Bridged     // Bridge from hold
CallState::Bridged -> CallState::Terminating // Destroy bridge
```

### Transfer States
```rust
CallState::Active -> CallState::Transferring  // Initiate transfer
CallState::Transferring -> CallState::Terminated // Complete transfer
```

## Implementation Status

✅ **Implemented**:
- 2-party bridge state transitions
- Blind transfer state transitions
- Attended transfer state transitions
- Hold/Resume for bridge preparation

🔄 **Planned** (separate crate):
- N-party conference rooms
- Audio mixing for 3+ parties
- Conference recording
- Participant management

## Usage Examples

### Simple 2-Party Bridge
```rust
// Alice calls Bob
let alice = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
alice.make_call("sip:bob@example.com").await?;

// Alice calls Charlie
let alice_charlie = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
alice_charlie.make_call("sip:charlie@example.com").await?;

// Bridge Bob and Charlie (Alice drops out)
coordinator.bridge_sessions(&alice.id, &alice_charlie.id).await?;
```

### Consultative Transfer
```rust
// Alice is talking to Bob
let alice = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
alice.make_call("sip:bob@example.com").await?;

// Alice wants to transfer Bob to Charlie (with consultation)
alice.transfer("sip:charlie@example.com", true).await?;
// This will:
// 1. Put Bob on hold
// 2. Call Charlie
// 3. Let Alice talk to Charlie
// 4. Complete transfer if Charlie accepts
```

## SIP Protocol Details

### Blind Transfer
```
REFER sip:bob@example.com SIP/2.0
Refer-To: <sip:charlie@example.com>
```

### Attended Transfer
```
1. Put current call on hold (re-INVITE with a=sendonly)
2. Make consultation call (new INVITE)
3. Send REFER with Replaces header
4. Original call terminates with BYE
```

## Audio Mixing Note

For 2-party bridges, audio is simply relayed between the two endpoints. For 3+ party conferences, audio mixing is required:

- Each participant's audio must be mixed with all others
- Each participant receives a unique mix (minus their own audio)
- Requires significant CPU for real-time mixing
- Best handled by specialized conference crate with optimized mixing algorithms