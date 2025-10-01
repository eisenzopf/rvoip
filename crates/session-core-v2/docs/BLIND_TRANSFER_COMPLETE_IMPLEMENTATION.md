# Blind Transfer - Complete Implementation Plan

## Executive Summary

Phase 1 (Quick Fix) is **COMPLETE**. The transferor (Bob) can now successfully send REFER messages to the transfer recipient (Alice). However, Alice cannot process the REFER and complete the transfer to Charlie.

**Root Cause:** dialog-core receives REFER messages but does NOT publish `TransferRequested` events to session-core.

This document provides a comprehensive Phase 2 implementation plan to complete blind transfer functionality.

---

## Current State Analysis

### ✅ What Works (Phase 1 Complete)

1. **Event Name Alignment**: `InitiateTransfer` → `BlindTransfer` everywhere
2. **Transfer Initiation**: Bob can trigger transfer from `Answering` or `Active` states
3. **REFER Message Sending**: `SendREFER` action sends REFER to Alice
4. **REFER Message Reception**: Alice's dialog-core receives REFER (creates ServerNonInviteTransaction)
5. **Transfer Target Capture**: `BlindTransfer` event captures target URI
6. **Event Handler Exists**: session-core has `handle_transfer_requested()` method

### ❌ What's Missing (Phase 2 Required)

1. **dialog-core Event Publishing**: When REFER is received, dialog-core does NOT publish `TransferRequested` event
2. **State Table Transitions**: No transitions defined for handling `TransferRequested` event
3. **Transfer Recipient Actions**: No actions to:
   - Accept REFER (send 202 Accepted)
   - Terminate current call (send BYE to Bob)
   - Initiate new call (send INVITE to Charlie)
   - Send NOTIFY to transferor with transfer status

### 📊 Test Results

From `/tmp/blind_transfer_test2.log`:

```
✅ Bob: Executes BlindTransfer (Answering → Transferring)
✅ Bob: Sends REFER to Alice with target "sip:charlie@127.0.0.1:5062"
✅ Alice: Receives REFER (ServerNonInviteTransaction created)
❌ Bob: Receives "481 Call/Transaction Does Not Exist" (Alice didn't respond properly)
❌ Alice: Never processes TransferRequested event
❌ Alice: Never calls Charlie
❌ Charlie: Never receives any call
```

---

## SIP Blind Transfer Protocol Flow

### RFC 3515 Specification

A complete blind transfer involves these SIP messages:

```
Alice (Transferee)          Bob (Transferor)          Charlie (Transfer Target)
      |                           |                           |
      |<-------- REFER ------------|  (1) Bob sends REFER to Alice
      |                           |      Refer-To: sip:charlie@...
      |                           |
      |--------- 202 Accepted --->|  (2) Alice accepts the REFER
      |                           |
      |--------- NOTIFY --------->|  (3) Alice notifies: "trying"
      |<-------- 200 OK ----------|      (transfer in progress)
      |                           |
      |--------- BYE ------------>|  (4) Alice hangs up with Bob
      |<-------- 200 OK ----------|
      |                           |
      |----------- INVITE ----------------------->|  (5) Alice calls Charlie
      |<---------- 180 Ringing -------------------|
      |<---------- 200 OK ------------------------|
      |----------- ACK -------------------------->|
      |                           |
      |--------- NOTIFY --------->|  (6) Alice notifies: "success"
      |<-------- 200 OK ----------|      (transfer complete)
      |                           |
      |<========== RTP ==========================>|  (7) Alice talks to Charlie
```

### Current vs Required Behavior

| Step | SIP Message | Current State | Required Action |
|------|-------------|---------------|-----------------|
| 1 | REFER | ✅ Sent by Bob | ✅ Working |
| 2 | 202 Accepted | ❌ Not sent | ⚠️ Need Action: AcceptREFER |
| 3 | NOTIFY (trying) | ❌ Not sent | ⚠️ Need Action: SendNOTIFY |
| 4 | BYE | ❌ Not sent | ⚠️ Need Action: TerminateCurrentCall |
| 5 | INVITE | ❌ Not sent | ⚠️ Need Action: InitiateTransferCall |
| 6 | NOTIFY (success) | ❌ Not sent | ⚠️ Need Action: SendNOTIFY |
| 7 | RTP | ❌ Not established | ✅ Should work once call is made |

---

## Phase 2: Implementation Plan

### Overview

Phase 2 has **4 major components**:

1. **dialog-core Changes**: Publish `TransferRequested` events when REFER is received
2. **State Table Changes**: Add transitions to handle transfer recipient flow
3. **Action Implementation**: Implement actions for REFER acceptance and call switching
4. **Testing**: Update blind_transfer example to verify end-to-end flow

---

## Component 1: dialog-core Event Publishing

### File: `crates/dialog-core/src/manager/transaction_integration.rs`

**Goal:** When REFER is received, publish `TransferRequested` event to session-core

**Location:** Find where incoming REFER transactions are processed

**Implementation:**

```rust
// In transaction_integration.rs, around the transaction event handling

// When ServerNonInviteTransaction is created for REFER method
if method == Method::REFER {
    // Extract Refer-To header
    let refer_to = request.headers()
        .get("Refer-To")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    // Extract Replaces header if present (attended transfer)
    let replaces = request.headers()
        .get("Replaces")
        .and_then(|h| h.to_str().ok());

    let transfer_type = if replaces.is_some() { "attended" } else { "blind" };

    // Send 202 Accepted response
    self.send_response(&transaction_key, 202, "Accepted").await?;

    // Publish TransferRequested event
    if let Some(coordinator) = &self.event_coordinator {
        let event = DialogToSessionEvent::TransferRequested {
            dialog_id: dialog_id.clone(),
            session_id: session_id.clone(),
            refer_to: refer_to.to_string(),
            transfer_type: transfer_type.to_string(),
        };
        coordinator.publish("dialog_to_session", Arc::new(event)).await?;
    }
}
```

**Files to Modify:**

1. `crates/dialog-core/src/manager/transaction_integration.rs` - Add REFER handling
2. `crates/dialog-core/src/events/dialog_to_session.rs` - Ensure `TransferRequested` variant exists

---

## Component 2: State Table Transitions

### File: `crates/session-core-v2/state_tables/default.yaml`

**Goal:** Define state machine transitions for transfer recipient (Alice)

**Transitions to Add:**

```yaml
# =============================================================================
# BLIND TRANSFER RECIPIENT HANDLING
# =============================================================================

# Transfer recipient receives REFER while in Active call
- role: "UAC"
  state: "Active"
  event:
    type: "TransferRequested"
  next_state: "TransferringCall"
  actions:
    - type: "AcceptREFER"              # Send 202 Accepted
    - type: "SendNOTIFY"               # Send NOTIFY SIP/2.0 100 Trying
    - type: "StoreTransferTarget"      # Store refer_to URI for later
  publish:
    - "TransferReceived"
  description: "UAC receives REFER for blind transfer"

# Also handle if we're still in Initiating state (call not fully active yet)
- role: "UAC"
  state: "Initiating"
  event:
    type: "TransferRequested"
  next_state: "TransferringCall"
  actions:
    - type: "AcceptREFER"
    - type: "SendNOTIFY"
    - type: "StoreTransferTarget"
  publish:
    - "TransferReceived"
  description: "UAC receives REFER during call setup"

# Internal event: After accepting REFER, terminate current call
- role: "UAC"
  state: "TransferringCall"
  event:
    type: "InternalProceedWithTransfer"
  next_state: "Idle"
  actions:
    - type: "TerminateCurrentCall"     # Send BYE to Bob
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "ReadyForTransferCall"
  description: "Terminate current call to proceed with transfer"

# After cleanup, make new call to transfer target
- role: "UAC"
  state: "Idle"
  event:
    type: "InternalMakeTransferCall"
  next_state: "Initiating"
  actions:
    - type: "CreateDialog"
    - type: "InitiateMedia"
    - type: "SendINVITE"
  publish:
    - "TransferCallInitiated"
  description: "Initiate call to transfer target"

# When transfer call is established, send success NOTIFY
- role: "UAC"
  state: "Active"
  event:
    type: "InternalTransferCallEstablished"
  next_state: "Active"
  actions:
    - type: "SendNOTIFYSuccess"        # NOTIFY SIP/2.0 200 OK
  publish:
    - "TransferCompleted"
  description: "Transfer call established successfully"
```

**State Added:**
- `TransferringCall`: Intermediate state while processing transfer

**Events Added:**
- `InternalProceedWithTransfer`: Trigger to start transfer process
- `InternalMakeTransferCall`: Trigger to call transfer target
- `InternalTransferCallEstablished`: Trigger when transfer succeeds

---

## Component 3: Action Implementation

### File: `crates/session-core-v2/src/state_machine/actions.rs`

**New Actions Required:**

#### 3.1 AcceptREFER

```rust
Action::AcceptREFER => {
    debug!("Accepting REFER request");
    // dialog-core should already send 202 Accepted automatically
    // This action is a placeholder for future enhancements
    info!("REFER accepted for session {}", session.session_id);
}
```

#### 3.2 SendNOTIFY

```rust
Action::SendNOTIFY => {
    debug!("Sending NOTIFY for transfer progress");
    // Send NOTIFY with sipfrag body: "SIP/2.0 100 Trying"
    if let Some(notify_to_dialog) = &session.transfer_notify_dialog {
        dialog_adapter.send_notify(
            notify_to_dialog,
            "refer".to_string(),
            Some("SIP/2.0 100 Trying".to_string())
        ).await?;
        info!("Sent NOTIFY (trying) for transfer");
    }
}
```

#### 3.3 SendNOTIFYSuccess

```rust
Action::SendNOTIFYSuccess => {
    debug!("Sending NOTIFY for transfer success");
    // Send NOTIFY with sipfrag body: "SIP/2.0 200 OK"
    if let Some(notify_to_dialog) = &session.transfer_notify_dialog {
        dialog_adapter.send_notify(
            notify_to_dialog,
            "refer".to_string(),
            Some("SIP/2.0 200 OK".to_string())
        ).await?;
        info!("Sent NOTIFY (success) for transfer");
    }
}
```

#### 3.4 StoreTransferTarget

```rust
Action::StoreTransferTarget => {
    debug!("Storing transfer target from REFER");
    // Transfer target is already captured from TransferRequested event
    // This action ensures it's persisted in session state
    if let Some(target) = &session.transfer_target {
        info!("Stored transfer target: {}", target);
    }
}
```

#### 3.5 TerminateCurrentCall

```rust
Action::TerminateCurrentCall => {
    debug!("Terminating current call for transfer");
    // Send BYE to current dialog
    dialog_adapter.send_bye(&session.session_id).await?;

    // Wait briefly for BYE to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    info!("Terminated current call for session {}", session.session_id);

    // Trigger next step: make transfer call
    self.process_event(
        &session.session_id,
        EventType::InternalMakeTransferCall
    ).await?;
}
```

**Note:** `TerminateCurrentCall` is different from `SendBYE` - it includes cleanup and triggers the next phase.

### File: `crates/session-core-v2/src/session_store/state.rs`

**Add field to SessionState:**

```rust
pub struct SessionState {
    // ... existing fields ...

    /// Dialog to send NOTIFY messages to (for transfer progress)
    pub transfer_notify_dialog: Option<DialogId>,
}
```

### File: `crates/session-core-v2/src/state_machine/executor.rs`

**Capture transfer info from TransferRequested event:**

```rust
// In process_event(), before state table lookup:

EventType::TransferRequested { refer_to, transfer_type } => {
    session.transfer_target = Some(refer_to.clone());
    session.transfer_notify_dialog = session.dialog_id.clone(); // Send NOTIFY back to Bob
    debug!("Captured transfer request: target={}, type={}", refer_to, transfer_type);
}
```

### File: `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Add new action types to parser:**

```rust
"AcceptREFER" => Ok(Action::AcceptREFER),
"SendNOTIFY" => Ok(Action::SendNOTIFY),
"SendNOTIFYSuccess" => Ok(Action::SendNOTIFYSuccess),
"StoreTransferTarget" => Ok(Action::StoreTransferTarget),
"TerminateCurrentCall" => Ok(Action::TerminateCurrentCall),
```

### File: `crates/session-core-v2/src/state_table/types.rs`

**Add new action variants:**

```rust
pub enum Action {
    // ... existing actions ...

    // Transfer recipient actions
    AcceptREFER,
    SendNOTIFY,
    SendNOTIFYSuccess,
    StoreTransferTarget,
    TerminateCurrentCall,
}
```

**Add new event types:**

```rust
pub enum EventType {
    // ... existing events ...

    // Transfer coordination events
    InternalProceedWithTransfer,
    InternalMakeTransferCall,
    InternalTransferCallEstablished,
}
```

### File: `crates/session-core-v2/src/state_table/types.rs`

**Add new state:**

```rust
pub enum CallState {
    // ... existing states ...
    TransferringCall,  // Intermediate state during transfer processing
}
```

---

## Component 4: Orchestration & Timing

### Challenge: Asynchronous State Transitions

The transfer flow requires **multiple asynchronous steps**:

1. Accept REFER → Send NOTIFY → Terminate call → Make new call → Send NOTIFY

**Problem:** Actions execute synchronously within a single state transition.

**Solution:** Use internal events to chain transitions:

```
TransferRequested → [AcceptREFER, SendNOTIFY, StoreTarget] → TransferringCall
                 ↓ (emit InternalProceedWithTransfer)
TransferringCall → [TerminateCurrentCall, Cleanup] → Idle
                 ↓ (emit InternalMakeTransferCall from TerminateCurrentCall action)
Idle → [CreateDialog, InitiateMedia, SendINVITE] → Initiating
     ↓ (normal call flow)
Active → [SendNOTIFYSuccess] → Active
```

### Implementation in `actions.rs`:

```rust
Action::TerminateCurrentCall => {
    debug!("Terminating current call for transfer");
    dialog_adapter.send_bye(&session.session_id).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // After BYE, trigger transition to Idle and then make transfer call
    // This is done by emitting internal events

    // Store the transfer target before state changes
    let transfer_target = session.transfer_target.clone()
        .ok_or_else(|| SessionError::InternalError("No transfer target".to_string()))?;

    // The state machine will transition to Idle automatically
    // We need to schedule the next call after this action completes

    // Use a helper to schedule the call after cleanup
    let session_id = session.session_id.clone();
    let state_machine = self.state_machine.clone();

    tokio::spawn(async move {
        // Wait for cleanup to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Make the transfer call
        if let Err(e) = state_machine.process_event(
            &session_id,
            EventType::MakeCall { target: transfer_target }
        ).await {
            error!("Failed to make transfer call: {}", e);
        }
    });
}
```

**Better Approach:** Use `publish` events to trigger next transition:

In `default.yaml`:
```yaml
- role: "UAC"
  state: "TransferringCall"
  event:
    type: "DialogBYE"  # Wait for BYE confirmation
  next_state: "Idle"
  actions:
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "TransferCallTerminated"
  description: "Current call terminated, ready for transfer"

# Then have a listener trigger MakeCall when cleanup is done
```

---

## Component 5: Testing Updates

### File: `crates/session-core-v2/examples/blind_transfer/peer1_caller.rs`

**Update Alice to handle transfer:**

```rust
// After call is established
println!("[ALICE] 💬 Talking to Bob...");
sleep(Duration::from_secs(5)).await;

println!("[ALICE] ⏳ Waiting for Bob to transfer me to Charlie...");
sleep(Duration::from_secs(8)).await;

// After transfer, Alice should be talking to Charlie
println!("[ALICE] 💬 Now talking to Charlie (post-transfer)...");
sleep(Duration::from_secs(3)).await;

println!("[ALICE] 📴 Hanging up with Charlie...");
// Hangup will now terminate the call with Charlie, not Bob
alice.hangup(&incoming.id).await?;
```

### Expected Log Output

After implementation, we should see:

```
[BOB] Executing transition: Answering + BlindTransfer → Transferring
[BOB] REFER sent to Alice
[ALICE] Received TransferRequested: refer_to=sip:charlie@127.0.0.1:5062
[ALICE] Executing transition: Active + TransferRequested → TransferringCall
[ALICE] 202 Accepted sent to Bob
[ALICE] NOTIFY (trying) sent to Bob
[ALICE] Executing transition: TransferringCall + InternalProceedWithTransfer → Idle
[ALICE] BYE sent to Bob
[BOB] Received 202 Accepted (transfer accepted)
[BOB] Received NOTIFY (trying)
[BOB] Received BYE from Alice
[BOB] Dialog terminated
[ALICE] Executing transition: Idle + MakeCall → Initiating
[ALICE] INVITE sent to Charlie
[CHARLIE] Received incoming call from Alice
[CHARLIE] 200 OK sent to Alice
[ALICE] Received 200 OK from Charlie
[ALICE] Executing transition: Initiating + Dialog200OK → Active
[ALICE] NOTIFY (success) sent to Bob
[BOB] Received NOTIFY (success) - transfer complete
```

---

## Implementation Phases

### Phase 2A: Foundation (Estimated: 2-3 hours)

**Files:**
1. `dialog-core/src/manager/transaction_integration.rs` - Publish TransferRequested
2. `dialog-core/src/events/dialog_to_session.rs` - Add/verify TransferRequested event
3. `session-core-v2/src/state_table/types.rs` - Add new actions, events, state
4. `session-core-v2/src/state_table/yaml_loader.rs` - Parse new action types
5. `session-core-v2/src/session_store/state.rs` - Add transfer_notify_dialog field

**Testing:**
- Build to ensure no compilation errors
- Verify TransferRequested event is published when REFER is received

### Phase 2B: Actions (Estimated: 3-4 hours)

**Files:**
1. `session-core-v2/src/state_machine/actions.rs` - Implement all 5 new actions
2. `session-core-v2/src/state_machine/executor.rs` - Capture transfer info from event

**Testing:**
- Unit tests for each action
- Verify NOTIFY messages are sent
- Verify BYE is sent to terminate current call

### Phase 2C: State Table (Estimated: 1-2 hours)

**Files:**
1. `session-core-v2/state_tables/default.yaml` - Add all transfer recipient transitions

**Testing:**
- Load state table and verify transitions are parsed
- Run blind_transfer example and check state transitions in logs

### Phase 2D: Integration & Testing (Estimated: 2-3 hours)

**Files:**
1. `examples/blind_transfer/peer1_caller.rs` - Update timing and expectations
2. `examples/blind_transfer/run_blind_transfer.sh` - Increase timeout if needed

**Testing:**
- Run full blind_transfer example
- Verify Charlie receives the transferred call
- Verify Bob receives NOTIFY messages
- Check for any race conditions or timing issues

**Total Estimated Time: 8-12 hours**

---

## Success Criteria

A successful implementation will produce this test output:

```
✅ Bob initiates blind transfer
✅ Alice receives REFER
✅ Alice sends 202 Accepted to Bob
✅ Alice sends NOTIFY (trying) to Bob
✅ Alice sends BYE to Bob
✅ Bob receives all responses
✅ Alice makes new call to Charlie
✅ Charlie receives incoming call from Alice
✅ Charlie answers call
✅ Alice and Charlie establish media
✅ Alice sends NOTIFY (success) to Bob
✅ Bob receives transfer success notification
✅ Test completes without errors or timeouts
```

---

## Alternative Approach: Simplified Flow

If the full RFC 3515 flow is too complex, we can implement a **simplified blind transfer** without NOTIFY messages:

### Simplified Flow

```
1. Bob sends REFER to Alice
2. Alice sends 202 Accepted
3. Alice sends BYE to Bob
4. Alice makes new call to Charlie
5. Done (no NOTIFY messages)
```

This removes the NOTIFY actions and simplifies testing, but is **not RFC compliant**.

**Recommendation:** Implement full flow for production readiness.

---

## Risk Assessment

### Low Risk
- Adding new event types ✅
- Adding new state ✅
- Implementing AcceptREFER, StoreTransferTarget ✅

### Medium Risk
- Coordinating asynchronous state transitions ⚠️
- Timing issues between BYE and new INVITE ⚠️
- Session ID management during transfer ⚠️

### High Risk
- dialog-core changes (could affect other features) 🔴
- Race conditions in state machine ⚠️
- NOTIFY message sequencing ⚠️

### Mitigation Strategies

1. **For dialog-core changes:** Make changes minimal and focused only on REFER handling
2. **For timing issues:** Add internal delays and use event-driven transitions
3. **For race conditions:** Use proper locking and state validation
4. **For testing:** Add extensive logging and run multiple iterations

---

## Future Enhancements (Post Phase 2)

1. **Attended Transfer Support:** Use REFER with Replaces header
2. **Transfer Status Subscription:** Bob subscribes to Alice's transfer events
3. **Transfer Failure Handling:** Handle cases where transfer call fails
4. **Multiple Transfer Types:** Support consultative and supervised transfers
5. **Transfer to Conference:** Allow transferring to multi-party conference

---

## Conclusion

Phase 2 requires changes across multiple layers:
- **dialog-core:** Publish TransferRequested events
- **session-core state table:** Add transfer recipient transitions
- **session-core actions:** Implement 5 new actions
- **session state:** Add transfer tracking fields

The most complex part is **coordinating asynchronous state transitions** to ensure the call to Charlie happens after the call to Bob is properly terminated.

Estimated total implementation time: **8-12 hours** for a complete, tested solution.
