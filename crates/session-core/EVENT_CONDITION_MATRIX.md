# Event-Condition Matrix for Session-Core

## Problem Statement
Session-core coordinates between dialog-core (SIP) and media-core (RTP) with complex interdependencies. Events from either layer can arrive in different orders, creating race conditions and edge cases.

## Current Condition Matrix

### Session Readiness Flags
```
SessionReadiness {
    dialog_established: bool,   // Set when call state becomes Active
    media_session_ready: bool,  // Set when MediaSessionReady event received
    sdp_negotiated: bool,       // Set when MediaNegotiated event received
    local_sdp: Option<String>,  // Local SDP offer/answer
    remote_sdp: Option<String>, // Remote SDP offer/answer
}
```

### Event × Condition Combinations

| Event | dialog_established | media_session_ready | sdp_negotiated | Action |
|-------|-------------------|---------------------|----------------|--------|
| `SessionCreated(Active)` | ✓ Set | - | - | Store session |
| `StateChanged(→Active)` | ✓ Set | - | - | Start media session |
| `MediaSessionReady` | - | ✓ Set | - | Check all conditions |
| `MediaNegotiated` | - | - | ✓ Set | Check all conditions |
| All conditions met | ✓ | ✓ | ✓ | Trigger `on_call_established` |

## Race Conditions and Solutions

### Race Condition 1: UAS MediaFlowEstablished Timing
**Problem**: Bob (UAS) publishes MediaFlowEstablished before SimpleCall subscribes to events

**Current Solution**: 
- Publish in `rfc_compliant_media_creation_uas` event handler
- This fires AFTER SimpleCall has subscribed

**Event Sequence**:
1. Bob sends 200 OK → state becomes Active
2. SimpleCall.audio_channels() called → subscribes to events
3. Alice sends ACK
4. Bob receives ACK → `rfc_compliant_media_creation_uas` event
5. Event handler publishes MediaFlowEstablished
6. SimpleCall receives event ✓

### Race Condition 2: SDP Negotiation Storage
**Problem**: MediaControl::generate_sdp_answer wasn't storing negotiated config

**Solution**: 
- Changed to use `negotiate_sdp_as_uas` which stores config
- Config needed for MediaFlowEstablished event

### Race Condition 3: RTP Transmission Default
**Problem**: RTP sessions created with `transmission_enabled: false`

**Solution**: 
- Changed default to `true` in RtpSessionWrapper creation

## Event Order Scenarios

### Scenario A: Normal UAC Flow
```
1. UAC sends INVITE with SDP offer
2. UAS responds 200 OK with SDP answer
3. UAC sends ACK
4. UAC: StateChanged(→Active) → dialog_established = true
5. UAC: rfc_compliant_media_creation_uac → negotiate_sdp_as_uac
6. UAC: MediaNegotiated → sdp_negotiated = true
7. UAC: MediaSessionReady → media_session_ready = true
8. UAC: All conditions met → on_call_established
9. UAC: MediaFlowEstablished published
```

### Scenario B: Normal UAS Flow
```
1. UAS receives INVITE with SDP offer
2. UAS: negotiate_sdp_as_uas, stores config
3. UAS sends 200 OK with SDP answer
4. UAS: StateChanged(→Active) → dialog_established = true
5. UAS receives ACK
6. UAS: rfc_compliant_media_creation_uas event
7. UAS: MediaFlowEstablished published (from event handler)
8. UAS: MediaNegotiated → sdp_negotiated = true
9. UAS: MediaSessionReady → media_session_ready = true
10. UAS: All conditions met → on_call_established
```

### Scenario C: Early Media (Not Implemented)
```
1. UAC sends INVITE
2. UAS sends 183 Session Progress with SDP
3. Early media flows
4. UAS sends 200 OK
5. UAC sends ACK
6. Transition to confirmed media
```

## Edge Cases

### Edge Case 1: Upfront SDP in INVITE
- **Condition**: UAC includes complete SDP in initial INVITE
- **Handling**: Store SDP in session registry, mark has_local_sdp
- **Special Event**: Publishes MediaFlowEstablished in MediaNegotiated handler

### Edge Case 2: Re-INVITE (Media Update)
- **Condition**: Established call receives new INVITE with different SDP
- **Handling**: MediaUpdate event → renegotiate SDP
- **State**: Remains Active during renegotiation

### Edge Case 3: CANCEL During Setup
- **Condition**: CANCEL received before 200 OK
- **Handling**: StateChanged(→Cancelled)
- **Cleanup**: Skip media session creation

### Edge Case 4: Simultaneous BYE
- **Condition**: Both parties send BYE at same time
- **Handling**: First BYE triggers SessionTerminating
- **Cleanup**: Two-phase termination ensures proper cleanup

## Condition Evaluation Points

The system checks all three conditions at these points:

1. **After StateChanged to Active**
   - Sets `dialog_established = true`
   - Calls `check_and_trigger_call_established()`

2. **After MediaSessionReady**
   - Sets `media_session_ready = true`
   - Calls `check_and_trigger_call_established()`

3. **After MediaNegotiated**
   - Sets `sdp_negotiated = true`
   - Calls `check_and_trigger_call_established()`

4. **After SDP Events**
   - Updates local/remote SDP
   - May call `check_and_trigger_call_established()`

## Critical Timing Dependencies

### Must Happen in Order:
1. Session created before any events processed
2. Dialog established before media can start
3. SDP negotiated before RTP can flow
4. MediaFlowEstablished before audio channels unblock

### Can Happen in Any Order:
1. Dialog establishment vs SDP negotiation
2. Media session creation vs SDP storage
3. Local vs remote SDP availability

### Must NOT Happen:
1. Media before SDP negotiation
2. RTP before ports allocated
3. Audio channels before MediaFlowEstablished
4. Cleanup before termination signal

## Proposed Improvements

### 1. Explicit State Machine
Replace distributed condition checking with central state machine:

```rust
enum SessionPhase {
    Initializing,
    SignalingComplete,
    MediaPreparing,
    MediaReady,
    Active,
    Terminating,
}

struct PhaseTransition {
    from: SessionPhase,
    event: EventType,
    conditions: ConditionSet,
    to: SessionPhase,
    side_effects: Vec<SideEffect>,
}
```

### 2. Event Sequencer
Buffer and reorder events to ensure correct sequence:

```rust
struct EventSequencer {
    pending: VecDeque<SessionEvent>,
    required_order: Vec<EventType>,
    received: HashSet<EventType>,
}
```

### 3. Condition Registry
Centralize condition tracking:

```rust
struct ConditionRegistry {
    conditions: HashMap<ConditionType, bool>,
    watchers: HashMap<ConditionSet, WatcherCallback>,
}
```

### 4. Deterministic Event Publishing
Make event publishing locations explicit:

```rust
trait EventPublisher {
    fn publish_points(&self) -> Vec<PublishPoint>;
    fn can_publish(&self, event: &EventType) -> bool;
}
```

## Testing Matrix

| Test Case | UAC State | UAS State | Expected Result |
|-----------|-----------|-----------|-----------------|
| Normal call | All conditions met | All conditions met | Both receive audio |
| Fast ACK | ACK before subscribe | - | MediaFlowEstablished received |
| Slow media | - | Media after ACK | Waits for media ready |
| Early terminate | BYE during setup | - | Proper cleanup |
| Network delay | - | Delayed RTP | Buffers until ready |

## Conclusion

The current system works but relies on careful event ordering and timing. A state table approach would:
1. Make all transitions explicit
2. Eliminate race conditions
3. Simplify debugging
4. Enable formal verification
5. Reduce code complexity

The MediaFlowEstablished event is the critical synchronization point that ensures bidirectional media readiness before audio flows.