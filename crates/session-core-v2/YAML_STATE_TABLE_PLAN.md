# YAML State Table Implementation Plan - Session Coordination Layer

## Executive Summary

The session-core-v2 library is a **coordination layer** between dialog-core (SIP) and media-core (RTP). It should NOT replicate the state machines already present in those layers. This plan focuses on the **session-level coordination** needed to manage the overall call lifecycle.

## Separation of Concerns

### What dialog-core Already Handles (DO NOT REPLICATE)
- SIP transaction state machines (INVITE client/server)
- Retransmission logic and timers (T1, T2, Timer B, etc.)
- Authentication challenge/response flows
- SIP message construction and parsing
- Dialog ID management
- Fork handling and early dialogs
- Transport layer (TCP/UDP/TLS)

### What media-core Already Handles (DO NOT REPLICATE)
- RTP session management
- Codec negotiation details
- RTCP reports and quality monitoring
- Jitter buffer management
- Media packet routing
- DTMF detection/generation
- Audio device management

### What session-core MUST Handle (COORDINATION ONLY)
- **Readiness Coordination**: When are BOTH dialog AND media ready?
- **State Synchronization**: Mapping dialog events to session states
- **Lifecycle Management**: When to establish, hold, resume, terminate
- **Event Publishing**: Application-level events based on combined state
- **Cross-Layer Actions**: Triggering actions in both layers together

## Current State Analysis

### Existing Transitions (34 total)
Session-core-v2 currently implements minimal coordination:
- **UAC coordination**: 13 transitions
- **UAS coordination**: 12 transitions  
- **Common operations**: 6 transitions (hold/resume)
- **Bridge coordination**: 3 transitions

### Required Transitions for Full Coordination (~85 total)
Not 148 as originally thought - many were duplicating lower-layer responsibilities.

## Session Coordination State Table

### Core Session States (High-Level Only)
```yaml
states:
  - Idle            # No session
  - Initiating      # Session setup started
  - Ringing         # Alerting (180 received/sent)
  - EarlyMedia      # Media before answer (183)
  - Active          # Dialog confirmed + Media flowing
  - OnHold          # Active but media suspended
  - Resuming        # Transitioning from hold
  - Bridged         # Connected to another session
  - Transferring    # REFER in progress
  - Terminating     # Cleanup initiated
  - Terminated      # Cleanup complete
  - Failed          # Session failed
```

### Coordination Conditions (What We Track)
```yaml
conditions:
  dialog_established: bool   # Dialog layer ready
  media_session_ready: bool  # Media layer ready  
  sdp_negotiated: bool      # SDP offer/answer complete
  # These determine when to fire onCallEstablished
```

### Events We Coordinate (Not Raw SIP)
```yaml
event_types:
  # Application triggers
  - MakeCall          # Start outgoing call
  - AcceptCall        # Accept incoming call
  - RejectCall        # Reject incoming call
  - HangupCall        # End active call
  - HoldCall          # Put on hold
  - ResumeCall        # Resume from hold
  
  # Dialog layer notifications (abstracted)
  - DialogProgress    # 1xx response (maps multiple SIP codes)
  - DialogEstablished # 2xx + ACK complete
  - DialogFailed      # 4xx/5xx/6xx response
  - DialogTerminated  # BYE complete
  - IncomingCall      # INVITE received
  
  # Media layer notifications
  - MediaReady        # RTP session created
  - MediaFlowing      # Bidirectional RTP confirmed
  - MediaFailed       # RTP setup failed
  - SDPNegotiated     # Offer/answer complete
  
  # Internal coordination
  - CheckReadiness    # Evaluate all conditions
  - PublishEstablished # Fire onCallEstablished
```

## Implementation Strategy

### Phase 1: YAML Infrastructure (Day 1-2)

#### 1.1 Add Dependencies
```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
```

#### 1.2 Create Coordination-Focused Schema
```yaml
# state_table.yaml - Session coordination only
version: "1.0"
metadata:
  description: "Session coordination between dialog and media layers"
  
transitions:
  - role: UAC
    state: Idle
    event: MakeCall
    guards:
      - description: "Application provided target"
    actions:
      - TriggerDialogINVITE    # Delegate to dialog-core
      - CreateMediaSession      # Delegate to media-core
    next_state: Initiating
    publish:
      - SessionCreated
    
  - role: UAC
    state: Initiating
    event: DialogProgress      # Abstracted from 180/183
    guards:
      - description: "Progress indicates ringing"
    next_state: Ringing
    publish:
      - CallRinging
    
  - role: UAC
    state: Ringing
    event: DialogEstablished   # Abstracted from 200 + ACK
    actions:
      - SetCondition(dialog_established, true)
      - CheckReadiness        # Check if media also ready
    next_state: Active
    
  - role: Both
    state: Active
    event: CheckReadiness
    guards:
      - AllConditionsMet     # dialog + media + sdp
    actions:
      - PublishEstablished   # Fire onCallEstablished
    publish:
      - CallEstablished
```

### Phase 2: Core Coordination Transitions (Day 3-5)

#### 2.1 Readiness Coordination Pattern
The key pattern - waiting for BOTH layers:
```yaml
# Pattern: Both conditions must be met
readiness_transitions:
  - role: Both
    state: Active
    event: MediaFlowing
    actions:
      - SetCondition(media_session_ready, true)
      - CheckReadiness
      
  - role: Both  
    state: Active
    event: SDPNegotiated
    actions:
      - SetCondition(sdp_negotiated, true)
      - CheckReadiness
      
  - role: Both
    state: Active
    event: CheckReadiness
    guards:
      - AllConditionsMet
    actions:
      - TriggerCallEstablished
    publish:
      - CallEstablished
```

#### 2.2 Hold/Resume Coordination
```yaml
hold_resume_transitions:
  - role: Both
    state: Active
    event: HoldCall
    actions:
      - SendReINVITE(direction=sendonly)  # Dialog action
      - SuspendMedia                       # Media action
    next_state: OnHold
    publish:
      - CallOnHold
      
  - role: Both
    state: OnHold
    event: ResumeCall
    actions:
      - SendReINVITE(direction=sendrecv)  # Dialog action
      - ResumeMedia                        # Media action  
    next_state: Resuming
    
  - role: Both
    state: Resuming
    event: DialogEstablished
    next_state: Active
    publish:
      - CallResumed
```

#### 2.3 Termination Coordination
```yaml
termination_transitions:
  - role: Both
    state: Active
    event: HangupCall
    actions:
      - SendBYE           # Dialog action
      - StopMedia         # Media action
    next_state: Terminating
    
  - role: Both
    state: Terminating
    event: DialogTerminated
    actions:
      - CleanupDialog
      - CleanupMedia
    next_state: Terminated
    publish:
      - CallTerminated
```

### Phase 3: Bridge and Transfer Coordination (Day 6-7)

#### 3.1 Bridge Coordination
```yaml
bridge_transitions:
  - role: Both
    state: Active
    event: BridgeToSession
    guards:
      - OtherSessionActive
    actions:
      - CreateMediaBridge    # Media layer bridge
      - LinkSessions        # Session tracking
    next_state: Bridged
    publish:
      - SessionsBridged
```

#### 3.2 Transfer Coordination (Via Dialog Layer)
```yaml
transfer_transitions:
  - role: Both
    state: Active
    event: InitiateTransfer
    actions:
      - SendREFER          # Dialog handles REFER/NOTIFY
    next_state: Transferring
    
  - role: Both
    state: Transferring
    event: TransferComplete
    next_state: Terminating
    publish:
      - TransferSucceeded
```

## Simplified Transition Count

### Phase 1: Core Coordination (45 transitions)
- **Call Setup**: 8 transitions (make, accept, reject)
- **Readiness Sync**: 12 transitions (3 conditions × 2 roles × 2 states)
- **Call Teardown**: 6 transitions (hang up, cleanup)
- **Progress Events**: 6 transitions (ringing, early media)
- **Error Cases**: 13 transitions (timeouts, failures)

### Phase 2: Extended Features (40 transitions)
- **Hold/Resume**: 8 transitions
- **Re-INVITE**: 6 transitions (session modification)
- **Bridge Operations**: 6 transitions
- **Transfer Operations**: 8 transitions
- **Error Recovery**: 12 transitions

**Total: ~85 transitions** (not 148 - we don't duplicate lower layers)

## What We DON'T Include

### NOT in Session-Core State Table:
1. **SIP Transaction States** - dialog-core handles
2. **Retransmission Logic** - dialog-core handles
3. **Authentication Flows** - dialog-core handles
4. **Media Packet Routing** - media-core handles
5. **Codec Details** - media-core handles
6. **Transport Issues** - handled by lower layers

## File Structure

```
crates/session-core-v2/
├── state_tables/
│   ├── session_coordination.yaml    # Main coordination table
│   ├── examples/
│   │   ├── minimal.yaml            # Basic call only
│   │   └── extended.yaml           # With hold/bridge/transfer
│   └── schemas/
│       └── coordination_schema.json # Validation schema
├── src/
│   ├── state_table/
│   │   ├── yaml_loader.rs          # YAML loading
│   │   └── mod.rs                  # Table management
│   ├── state_machine/
│   │   └── executor.rs              # Coordination logic only
│   └── adapters/
│       ├── dialog_adapter.rs       # Thin dialog-core wrapper
│       └── media_adapter.rs        # Thin media-core wrapper
```

## Example YAML Structure

```yaml
# session_coordination.yaml
version: "1.0"
purpose: "Session-level coordination between dialog and media layers"

# Define coordination-only states
states:
  - name: Idle
    description: "No active session"
  - name: Active
    description: "Both dialog and media established"
    
# Define coordination conditions
conditions:
  - name: dialog_established
    description: "Dialog layer confirmed ready"
  - name: media_session_ready
    description: "Media layer confirmed ready"
  - name: sdp_negotiated
    description: "SDP offer/answer complete"

# Coordination transitions only
transitions:
  - role: UAC
    state: Active
    event: 
      type: CheckReadiness
    guards:
      - all_conditions_met
    actions:
      - type: PublishEvent
        event: CallEstablished
    description: "Fire callback when all layers ready"
```

## Success Metrics

1. **Clean Separation**: No duplication of lower-layer logic
2. **Coordination Focus**: Only ~85 transitions needed (not 148)
3. **Simplicity**: Each transition has clear coordination purpose
4. **Maintainability**: YAML clearly shows coordination logic
5. **Performance**: Minimal overhead over direct layer calls

## Timeline

| Day | Task | Transitions |
|-----|------|------------|
| 1-2 | YAML infrastructure | 0 → framework |
| 3-4 | Core coordination | 0 → 45 |
| 5-6 | Extended features | 45 → 85 |
| 7 | Testing & validation | 85 (complete) |

## Key Principles

1. **Delegate, Don't Duplicate**: Let dialog-core and media-core handle their domains
2. **Coordinate, Don't Control**: Synchronize state, don't manage protocols
3. **Abstract, Don't Expose**: Hide SIP/RTP details from applications
4. **React, Don't Poll**: Event-driven coordination only

## Conclusion

This revised plan focuses session-core-v2 on its true purpose: **coordinating between dialog and media layers**. By not duplicating the state machines already in dialog-core and media-core, we achieve:

- Simpler state table (~85 vs 148 transitions)
- Clear separation of concerns
- Easier maintenance
- Better performance
- Proper abstraction for applications

The YAML format will make the coordination logic transparent and maintainable.