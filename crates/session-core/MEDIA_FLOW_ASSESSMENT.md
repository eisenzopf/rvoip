# Session-Core Media Flow Assessment and Fix Plan

## Executive Summary

The session-core library fails to establish bidirectional audio when UAC and UAS run as separate processes. The root cause is that each side only sets up media flow for SENDING packets to the remote party, but doesn't properly trigger the remote party to send packets back. This is fundamentally an event handling issue where the UAS side doesn't properly respond to SDP negotiation completion.

## Test Results

1. **Integrated test** (same process): ✅ Bidirectional audio works
2. **Split test with SessionManager API**: ⚠️ Only UAS→UAC audio works  
3. **Split test with UAC/UAS APIs**: ❌ No audio works in either direction

All tests run on localhost (127.0.0.1) with no NAT or firewall issues.

## Root Cause Analysis

### The Core Problem

When UAC and UAS run as separate processes, each process only calls `establish_media_flow()` for its OUTBOUND direction:

```rust
// establish_media_flow() only sets up SENDING, not RECEIVING
pub async fn establish_media_flow(&self, dialog_id: &DialogId, remote_addr: SocketAddr) -> Result<()> {
    // Sets where to SEND packets
    self.update_rtp_remote_addr(dialog_id, remote_addr).await?;
    
    // Starts SENDING audio
    self.start_audio_transmission(dialog_id).await?;
    
    // Does NOT tell remote party to send packets back!
}
```

### Why Single Process Works

When UAC and UAS run in the same process:
- They share the same `MediaSessionController` instance
- Both RTP sessions exist in the same memory space
- The controller can directly route packets between sessions
- Both directions get established because they share state

### Why Separate Processes Fail

When UAC and UAS run as separate processes:

**Process A (UAC)**:
- Creates RTP session on port 40000
- Sends SDP offer with port 40000
- Receives SDP answer with port 42000
- Calls `establish_media_flow()` to send TO port 42000
- Starts sending audio → Process B receives it

**Process B (UAS)**:
- Creates RTP session on port 42000  
- Receives SDP offer with port 40000
- Sends SDP answer with port 42000
- **PROBLEM**: Doesn't call `establish_media_flow()` properly
- Never starts sending audio → Process A receives nothing

## Event Flow Analysis

### Current Event Flow (Broken)

The media establishment is event-driven based on SIP dialog events:

#### UAC Side Events:
1. **`OutgoingCallPrepared`** → Creates media session, generates SDP offer
2. **`CallInitiated`** → Sends INVITE with SDP offer
3. **`CallAnswered` (200 OK received)** → Processes SDP answer
4. **`MediaNegotiated`** → Calls `establish_media_flow()` ✅ WORKS
5. **`AckSent`** → ACK sent to UAS

#### UAS Side Events:
1. **`IncomingCall`** → Receives INVITE with SDP offer
2. **`CallAccepted`** → Creates media session, generates SDP answer
3. **`CallAnswered` (200 OK sent)** → Should establish media but doesn't
4. **`AckReceived`** → Currently tries to establish media here (TOO LATE!)
5. **`MediaNegotiated`** → Never properly triggered for UAS

### The Specific Problem in event_handler.rs

In `coordinator/event_handler.rs`, the UAC establishes media flow correctly:

```rust
// Line ~756 - UAC side (WORKS)
CoordinatorEvent::MediaNegotiated { session_id, negotiated } => {
    if let Some(dialog_id) = self.get_media_dialog_id(&session_id).await {
        self.media_manager.controller.establish_media_flow(
            &dialog_id, 
            negotiated.remote_addr
        ).await?;
    }
}
```

But the UAS side has problems:

```rust
// Line ~924 - UAS side (BROKEN)
CoordinatorEvent::DialogEstablished { session_id, .. } => {
    // This event comes AFTER ACK received
    // But by then, UAC is already sending media
    // And we never properly trigger MediaNegotiated for UAS
}
```

## The Fix

### Core Solution

The UAS must establish its outbound media flow immediately after SDP negotiation completes, not after receiving ACK. This requires fixing the event flow:

### 1. Fix UAS Media Flow Trigger

The UAS should establish media flow when it sends 200 OK, not when it receives ACK:

```rust
// In event_handler.rs, when UAS sends 200 OK with SDP answer:
CoordinatorEvent::CallAnswered { session_id, dialog_id, local_sdp, remote_sdp } => {
    if self.is_uas(&session_id) && remote_sdp.is_some() {
        // Extract remote RTP address from the OFFER we received
        let remote_addr = extract_rtp_address(&remote_sdp.unwrap())?;
        
        // Establish UAS→UAC media flow immediately
        if let Some(media_dialog_id) = self.get_media_dialog_id(&session_id).await {
            self.media_manager.controller.establish_media_flow(
                &media_dialog_id,
                remote_addr
            ).await?;
            
            tracing::info!("UAS established media flow to UAC at {}", remote_addr);
        }
    }
}
```

### 2. Ensure MediaNegotiated Event for UAS

The UAS should trigger `MediaNegotiated` event after processing the SDP offer:

```rust
// When UAS accepts call and generates SDP answer:
CoordinatorEvent::CallAccepted { session_id, sdp_offer } => {
    // Generate SDP answer
    let sdp_answer = self.generate_sdp_answer(&sdp_offer).await?;
    
    // Trigger MediaNegotiated for UAS
    self.send_event(CoordinatorEvent::MediaNegotiated {
        session_id: session_id.clone(),
        negotiated: NegotiatedMedia {
            codec: extract_codec(&sdp_offer),
            local_addr: my_rtp_addr,
            remote_addr: extract_rtp_address(&sdp_offer),
        }
    });
}
```

### 3. Fix Event Handler State Machine

Ensure both UAC and UAS follow the same state machine for media:

```rust
enum MediaState {
    Idle,
    Negotiating,     // SDP offer/answer in progress
    Ready,           // SDP negotiated, ready to establish flow
    Established,     // establish_media_flow() called
    Active,          // Bidirectional media flowing
}
```

Both sides should transition through states based on events:
- **UAC**: Idle → Negotiating (INVITE sent) → Ready (200 OK received) → Established → Active
- **UAS**: Idle → Negotiating (INVITE received) → Ready (200 OK sent) → Established → Active

## Implementation Steps

### Step 1: Add Session Role Tracking

```rust
enum SessionRole {
    UAC,  // Initiator/Caller
    UAS,  // Responder/Callee  
}

impl SessionCoordinator {
    async fn get_session_role(&self, session_id: &SessionId) -> Option<SessionRole> {
        // Track whether session initiated the call or received it
    }
}
```

### Step 2: Fix UAS Event Handling

```rust
// In handle_incoming_call()
async fn handle_incoming_call(&mut self, invite: IncomingCall) {
    let session_id = self.create_session(SessionRole::UAS);
    
    // Process SDP offer immediately
    let remote_sdp = invite.sdp;
    let remote_rtp = extract_rtp_address(&remote_sdp)?;
    
    // Generate answer
    let local_sdp = self.generate_answer(&remote_sdp)?;
    
    // Store negotiated media info
    self.sessions.get_mut(&session_id).unwrap().negotiated_media = Some(NegotiatedMedia {
        remote_addr: remote_rtp,
        local_addr: my_rtp_port,
        codec: agreed_codec,
    });
}

// In handle_call_answered() for UAS
async fn handle_call_answered(&mut self, session_id: SessionId) {
    if self.get_session_role(&session_id) == Some(SessionRole::UAS) {
        // UAS sends 200 OK - establish media NOW
        if let Some(negotiated) = self.get_negotiated_media(&session_id) {
            self.establish_media_flow(&session_id, negotiated.remote_addr).await?;
        }
    }
}
```

### Step 3: Fix UAC Event Handling

```rust
// In handle_call_answered() for UAC  
async fn handle_call_answered(&mut self, session_id: SessionId, sdp_answer: String) {
    if self.get_session_role(&session_id) == Some(SessionRole::UAC) {
        // UAC receives 200 OK - establish media
        let remote_rtp = extract_rtp_address(&sdp_answer)?;
        self.establish_media_flow(&session_id, remote_rtp).await?;
    }
}
```

## Testing the Fix

### 1. Verify Events Fire Correctly

Add logging to confirm event sequence:

```rust
tracing::info!("[{}] Event: {:?}, Media State: {:?}", 
    session_role, event_name, media_state);
```

Expected UAC sequence:
1. `[UAC] Event: CallInitiated, Media State: Negotiating`
2. `[UAC] Event: CallAnswered, Media State: Ready`
3. `[UAC] Event: MediaFlowEstablished, Media State: Established`

Expected UAS sequence:
1. `[UAS] Event: IncomingCall, Media State: Negotiating`
2. `[UAS] Event: CallAnswered, Media State: Ready`
3. `[UAS] Event: MediaFlowEstablished, Media State: Established`

### 2. Verify Bidirectional Flow

After both sides establish media:
- UAC RTP session should have remote_addr = UAS RTP port
- UAS RTP session should have remote_addr = UAC RTP port
- Both should be in "transmitting" state
- Both should receive audio frames

## Success Criteria

1. **Split process test works**: Both UAC and UAS receive audio when in separate processes
2. **Events fire symmetrically**: Both sides trigger MediaNegotiated → MediaFlowEstablished
3. **Timing is correct**: UAS establishes flow when sending 200 OK, not after ACK
4. **State machine is consistent**: Both sides follow same state transitions

## Timeline

- **Day 1-2**: Add session role tracking and logging
- **Day 3-4**: Fix UAS to establish media when sending 200 OK  
- **Day 5-6**: Ensure MediaNegotiated events fire for both sides
- **Day 7-8**: Test and verify bidirectional audio works
- **Day 9-10**: Clean up and optimize event flow

## Conclusion

The problem is not about networking or NAT traversal. It's about proper event handling:

1. **UAS doesn't trigger media flow establishment** at the right time
2. **MediaNegotiated event doesn't fire** for UAS 
3. **No session role tracking** to handle UAC vs UAS differently

The fix requires ensuring both UAC and UAS:
- Respond to the same SDP negotiation events
- Call `establish_media_flow()` after SDP negotiation completes
- Track their role (caller vs callee) to handle events appropriately

This is a state machine and event handling problem, not a media or networking problem.