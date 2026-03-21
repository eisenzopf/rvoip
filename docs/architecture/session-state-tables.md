# Session-Core State Management Tables

## Overview
This document maps out the complex state management and event coordination in session-core, which acts as the coordinator between dialog-core (SIP signaling) and media-core (RTP/media handling). The tables below represent the states, events, conditions, and actions that drive the session lifecycle.

## Core States Being Tracked

### 1. Session State (CallState)
| State | Description | Entry Conditions | Exit Conditions |
|-------|-------------|------------------|-----------------|
| `Initiating` | Call is being set up | Session created | 200 OK received (UAC) or ACK received (UAS) |
| `Ringing` | 180 Ringing received | 180 response received | 200 OK received |
| `Active` | Call is connected | 200 OK + ACK exchanged | BYE sent/received |
| `OnHold` | Call is on hold | Hold request | Resume request |
| `Transferring` | Call being transferred | REFER received | Transfer complete/failed |
| `Terminating` | Call ending | BYE sent/received | Cleanup complete |
| `Terminated` | Call ended | Cleanup complete | - |
| `Failed` | Call failed | Error condition | - |
| `Cancelled` | Call cancelled | CANCEL sent/received | - |

### 2. Session Readiness Conditions
| Condition | Description | Set When | Used For |
|-----------|-------------|----------|----------|
| `dialog_established` | SIP dialog confirmed | State changes to Active | Call establishment check |
| `media_session_ready` | Media session created | MediaSessionReady event | Call establishment check |
| `sdp_negotiated` | SDP offer/answer complete | MediaNegotiated event | Call establishment check |
| `local_sdp` | Local SDP available | SDP generated | on_call_established callback |
| `remote_sdp` | Remote SDP available | SDP received | on_call_established callback |

### 3. Role-Based States
| Role | Description | Behavior |
|------|-------------|----------|
| `UAC` (User Agent Client) | Originating call | Sends INVITE, waits for 200 OK, sends ACK |
| `UAS` (User Agent Server) | Receiving call | Receives INVITE, sends 200 OK, waits for ACK |

## Event Flow Tables

### Session Events
| Event | Triggered By | Updates | Actions | Next Events |
|-------|-------------|---------|---------|-------------|
| `SessionCreated` | New call initiated | Creates session | Initialize readiness tracking | `StateChanged` |
| `StateChanged` | Dialog events | Session state | Check readiness conditions | `MediaSessionReady`, `MediaNegotiated` |
| `IncomingCall` | INVITE received | Creates session | Notify handler | `StateChanged` |
| `MediaEvent` | Dialog/Media layers | Depends on event type | Handle specific media event | `MediaFlowEstablished` |
| `MediaNegotiated` | SDP negotiation | `sdp_negotiated = true` | Check call establishment | `on_call_established` |
| `MediaSessionReady` | Media created | `media_session_ready = true` | Check call establishment | `on_call_established` |
| `MediaFlowEstablished` | Media ready | - | Unblock audio channels | - |
| `SessionTerminating` | BYE/error | Session state | Start cleanup | `SessionTerminated` |
| `SessionTerminated` | Cleanup done | Remove session | Notify handler | - |

### Media Events
| Event String | Role | Trigger | Actions | Publishes |
|--------------|------|---------|---------|-----------|
| `rfc_compliant_media_creation_uac` | UAC | ACK sent | Negotiate SDP as UAC | `MediaFlowEstablished` |
| `rfc_compliant_media_creation_uas` | UAS | ACK received | Start media session | `MediaFlowEstablished` |
| `dialog_established` | Both | Dialog confirmed | Mark dialog ready | - |
| `media_started` | Both | Media flows | Update state | - |
| `media_stopped` | Both | Media ends | Cleanup | - |

## State Transition Tables

### UAC Call Flow
| Step | State | Event | Condition Check | Action | Next State |
|------|-------|-------|-----------------|--------|------------|
| 1 | `Initiating` | Send INVITE | - | Create session, send INVITE with SDP | `Initiating` |
| 2 | `Initiating` | 180 Ringing | - | Update state | `Ringing` |
| 3 | `Ringing` | 200 OK received | Has SDP answer | Send ACK, negotiate SDP | `Active` |
| 4 | `Active` | ACK sent | - | Publish `rfc_compliant_media_creation_uac` | `Active` |
| 5 | `Active` | MediaNegotiated | `sdp_negotiated = true` | Check readiness | `Active` |
| 6 | `Active` | MediaSessionReady | `media_session_ready = true` | Check readiness | `Active` |
| 7 | `Active` | All conditions met | All 3 flags true | Trigger `on_call_established` | `Active` |
| 8 | `Active` | MediaFlowEstablished | - | Enable audio channels | `Active` |

### UAS Call Flow
| Step | State | Event | Condition Check | Action | Next State |
|------|-------|-------|-----------------|--------|------------|
| 1 | - | INVITE received | Has SDP offer | Create session, negotiate SDP | `Initiating` |
| 2 | `Initiating` | Accept call | - | Send 200 OK with SDP answer | `Initiating` |
| 3 | `Initiating` | 200 OK sent | - | Update state | `Active` |
| 4 | `Active` | ACK received | - | Publish `rfc_compliant_media_creation_uas` | `Active` |
| 5 | `Active` | MediaEvent(uas) | - | Publish `MediaFlowEstablished` | `Active` |
| 6 | `Active` | MediaNegotiated | `sdp_negotiated = true` | Check readiness | `Active` |
| 7 | `Active` | MediaSessionReady | `media_session_ready = true` | Check readiness | `Active` |
| 8 | `Active` | All conditions met | All 3 flags true | Trigger `on_call_established` | `Active` |

## Condition Checking Logic

### Call Establishment Conditions
```
IF dialog_established AND media_session_ready AND sdp_negotiated THEN
    IF local_sdp EXISTS AND remote_sdp EXISTS THEN
        TRIGGER on_call_established(session, local_sdp, remote_sdp)
        REMOVE session from readiness tracking
    END IF
END IF
```

### MediaFlowEstablished Publishing Rules
| Scenario | When Published | By Component | Condition |
|----------|---------------|--------------|-----------|
| UAC after negotiation | After negotiate_sdp_as_uac | coordinator.rs | Has negotiated config |
| UAS after negotiation | After negotiate_sdp_as_uas | coordinator.rs | Has negotiated config |
| UAS after ACK | On `rfc_compliant_media_creation_uas` | event_handler.rs | Has negotiated config |
| UAC with upfront SDP | On MediaNegotiated event | event_handler.rs | Has local SDP |

## Event Handler Decision Tables

### IncomingCall Event
| Handler Response | Action | Next State |
|-----------------|--------|------------|
| `Accept` | Send 200 OK | `Active` |
| `Reject` | Send 486 Busy | `Failed` |
| `Defer` | Store for later | `Initiating` |

### State Change Events
| From State | To State | Actions |
|------------|----------|---------|
| `Initiating` | `Active` | Start media session, mark dialog established |
| `Active` | `OnHold` | Pause media streams |
| `OnHold` | `Active` | Resume media streams |
| `Active` | `Terminating` | Stop media, cleanup |
| Any | `Failed` | Stop media, cleanup |

## Cleanup Coordination

### Two-Phase Termination
| Phase | Event | Actions | Tracking |
|-------|-------|---------|----------|
| Phase 1 | `SessionTerminating` | Stop media, notify layers | Create CleanupTracker |
| Phase 2 | `CleanupConfirmation` | Layer confirms cleanup | Update tracker |
| Complete | `SessionTerminated` | Remove session | Delete tracker |

### Cleanup Tracker
| Layer | Field | Set When |
|-------|-------|----------|
| Media | `media_done` | Media layer cleanup complete |
| Client | `client_done` | API client notified |
| Dialog | (implicit) | Dialog terminated |

## Timer Usage (Test Only)

**Library**: NO TIMERS - Fully event-driven

**Test Code** (`audio_utils.rs`):
- Send pacing: 20ms intervals (simulates real-time audio)
- Receive timeout: 100ms per attempt
- Max timeout: 3 seconds (30 consecutive timeouts)

## Key Invariants

1. **Call Establishment**: Must have all three conditions (`dialog_established`, `media_session_ready`, `sdp_negotiated`) AND both SDPs
2. **MediaFlowEstablished**: Published exactly once per session when bidirectional media confirmed
3. **Role Consistency**: Session role (UAC/UAS) determines event flow and SDP negotiation order
4. **Cleanup Order**: Media stops before dialog terminates
5. **Event Ordering**: State changes trigger condition checks which may trigger callbacks

## Potential Simplifications

### State Table Approach
Instead of complex conditional logic, use a state table:

```rust
struct StateTransition {
    current_state: CallState,
    event: SessionEvent,
    conditions: Vec<Condition>,
    actions: Vec<Action>,
    next_state: CallState,
    publish_events: Vec<SessionEvent>,
}

// Table-driven state machine
let transitions = vec![
    StateTransition {
        current_state: CallState::Initiating,
        event: SessionEvent::StateChanged(Active),
        conditions: vec![],
        actions: vec![Action::MarkDialogEstablished],
        next_state: CallState::Active,
        publish_events: vec![],
    },
    // ... more transitions
];
```

### Benefits of State Tables
1. **Clarity**: All transitions in one place
2. **Testability**: Each transition independently testable
3. **Maintainability**: Add new states/events without touching logic
4. **Debugging**: Log state transitions with full context
5. **Validation**: Detect invalid transitions at compile time

### Proposed Structure
```rust
pub struct SessionStateMachine {
    transitions: HashMap<(CallState, EventType), TransitionRule>,
    conditions: HashMap<SessionId, SessionConditions>,
    actions: Vec<Box<dyn Action>>,
}

pub struct TransitionRule {
    required_conditions: Vec<Condition>,
    actions: Vec<ActionType>,
    next_state: Option<CallState>,
    publish_events: Vec<EventTemplate>,
}
```

This would replace the current scattered condition checking with a centralized, table-driven approach that's easier to understand and maintain.