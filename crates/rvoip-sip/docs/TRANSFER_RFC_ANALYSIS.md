# SIP Transfer RFC Analysis and Implementation Review

**Date:** 2025-10-02
**Author:** Analysis of RFC 3515, RFC 5589, and current rvoip implementation
**Purpose:** Understand RFC requirements vs. current implementation, determine if special state transitions are needed

---

## Executive Summary

After deep analysis of RFC 3515 (REFER method) and RFC 5589 (Call Transfer), here are the key findings:

### **Key RFC Requirements:**

1. **Transferor sends BYE** - The transferor (Bob) is responsible for terminating the original dialog with the transferee (Alice)
2. **BYE timing is flexible** - BYE can be sent immediately after REFER (blind transfer) or after waiting for NOTIFY success (consultative)
3. **REFER does NOT terminate dialog** - The REFER transaction itself only requests the transfer; dialog termination requires explicit BYE
4. **Transferee makes new call** - The transferee (Alice) initiates INVITE to transfer target (Charlie)
5. **NOTIFY provides status** - Transferee sends NOTIFY messages to transferor about transfer progress

### **Current Implementation Issues:**

1. ❌ **Backwards BYE responsibility** - Our code has Alice (transferee) trying to send BYE to Bob, but RFC says Bob (transferor) should send BYE to Alice
2. ❌ **Missing ACK** - Alice doesn't send ACK to Charlie after receiving 200 OK
3. ✅ **Transfer flow is mostly correct** - REFER sending, new call creation, NOTIFY support are implemented
4. ⚠️ **State transitions may need transfer-specific variants** - Current generic states don't capture transfer semantics well

### **Do We Need Special Transfer State Transitions?**

**YES, but with nuance:**

- We DON'T need completely separate states (like "TransferActive" vs "Active")
- We DO need **context-aware transitions** that behave differently based on transfer metadata
- We DO need **proper role-based transitions** (transferor vs transferee behavior differs)

---

## RFC 3515: The REFER Method

### Purpose

RFC 3515 defines the REFER method, which requests that the recipient refer to a resource provided in the request. The REFER method is the foundation for call transfer.

### Key Components

1. **REFER Request** - Contains Refer-To header with target URI
2. **202 Accepted Response** - Recipient acknowledges they will attempt the reference
3. **NOTIFY Mechanism** - Recipient sends status updates about the referenced action
4. **refer Event Package** - Defines the NOTIFY subscription for transfer status

### Critical RFC Statement

> "A successful REFER transaction does not terminate the session between the parties. If those parties wish to terminate their session, they must do so with a subsequent BYE request."

**Implication:** The transferor MUST explicitly send BYE - the REFER itself doesn't end the call.

---

## RFC 5589: Call Control - Transfer

### Transfer Types Defined

1. **Basic Transfer (Unattended/Blind)** - Transferor doesn't wait for confirmation
2. **Consultative Transfer** - Transferor waits for NOTIFY success before hanging up
3. **Attended Transfer** - Transferor establishes consultation call first, then uses Replaces header

### Section 6.1: Basic Transfer (Blind Transfer)

This is our use case. Here's what the RFC specifies:

#### Call Flow

```
Alice (Transferee)          Bob (Transferor)          Charlie (Transfer Target)
      |                           |                           |
      |<-------- REFER ------------|  Bob sends REFER to Alice
      |                           |  Refer-To: sip:charlie@...
      |                           |
      |-------- 202 Accepted ---->|  Alice accepts REFER
      |                           |
      |-------- NOTIFY ---------->|  Alice: "SIP/2.0 100 Trying"
      |<------- 200 OK -----------|
      |                           |
      |---------- BYE <-----------|  Bob hangs up (blind transfer)
      |--------> 200 OK ----------|
      |                           |
      |---------- INVITE ----------------------->|  Alice calls Charlie
      |<--------- 180 Ringing -------------------|
      |                           |
      |--------- NOTIFY --------->|  Alice: "SIP/2.0 180 Ringing"
      |<-------- 200 OK ----------|
      |                           |
      |<--------- 200 OK ------------------------|  Charlie answers
      |---------- ACK -------------------------->|
      |                           |
      |--------- NOTIFY --------->|  Alice: "SIP/2.0 200 OK"
      |<-------- 200 OK ----------|
      |                           |
      |<========== RTP ==========================>|  Alice talks to Charlie
```

#### Key Observations

1. **Bob (Transferor) sends BYE to Alice** - NOT the other way around
2. **BYE timing:** Immediately after receiving 202 Accepted (for blind transfer)
3. **Alice's responsibilities:**
   - Accept REFER (202)
   - Send NOTIFY updates
   - Initiate new INVITE to Charlie
   - Send ACK when Charlie answers
4. **Bob's responsibilities:**
   - Send REFER with Refer-To header
   - Receive NOTIFY updates
   - Send BYE to terminate original dialog

### RFC Quote on BYE Timing

> "If the Transferor's agent does not wish to participate in the remainder of the REFER process and has no intention of assisting with recovery from transfer failure, **it could emit a BYE to the Transferee as soon as the REFER transaction completes**. This flow is sometimes known as 'unattended transfer' or 'blind transfer'."

**Key Point:** "The Transferor's agent... could emit a BYE to the Transferee"

---

## Our Current Implementation Analysis

### What We Got Right ✅

1. **REFER Sending** - Bob sends REFER to Alice correctly
2. **Transfer Coordinator** - Handles transfer orchestration
3. **NOTIFY Support** - Infrastructure exists to send NOTIFY messages
4. **New Call Creation** - Alice creates new session to Charlie
5. **Transfer Metadata** - Sessions track transfer context

### What We Got Wrong ❌

#### 1. **BYE Responsibility is Backwards**

**RFC Says:** Transferor (Bob) sends BYE to Transferee (Alice)

**Our Code:**
- [coordinator.rs:258-266]() - Alice (transferee) hangs up Bob when `terminate_old_call` is true
- [peer3_target.rs:56]() - Charlie (transfer target) tries to hang up Alice

**Problem:** The wrong parties are initiating call termination.

**Why This Matters:**
- Bob should control when the original dialog ends
- Alice needs to keep dialog alive to send NOTIFY updates
- If Alice hangs up on Bob, Bob loses ability to track transfer status

#### 2. **Missing ACK from Alice to Charlie**

**RFC Requires:** UAC (Alice) MUST send ACK after receiving 200 OK to INVITE

**Our Logs:** Alice receives 200 OK but never sends ACK (see BYE_MESSAGE_ANALYSIS.md)

**Why This Matters:**
- Dialog with Charlie never completes establishment
- Charlie stuck in "Answering" state waiting for ACK
- Without confirmed dialog, BYE cannot be sent properly

#### 3. **Blind Transfer Terminates Too Early**

**RFC Allows:** Transferor may send BYE immediately after REFER transaction completes

**Our Code:** [coordinator.rs:246-255]() sends NOTIFY success immediately, then terminates old call

**Problem:** We're terminating before verifying the INVITE was even sent successfully

**Better Approach:**
- Send NOTIFY "100 Trying" immediately
- Send INVITE to target
- Bob sends BYE to Alice (not Alice to Bob)
- Alice sends NOTIFY "180/200" as transfer progresses
- Alice completes call with Charlie

### What's Partially Wrong ⚠️

#### 4. **State Transitions Don't Distinguish Transfer Context**

**Current State Table:**
- Generic "Active" state for all established calls
- Generic "HangupCall" event for all terminations
- No distinction between:
  - Regular call hangup
  - Transferee being hung up by transferor
  - Transfer target receiving call from transferee

**Example Problem:**

When Bob (transferor) sends BYE to Alice (transferee):
- Alice is in "Active" state with Bob
- Alice receives `DialogBYE` event
- State table transition: Active + DialogBYE → Terminating
- **But Alice shouldn't fully terminate** - she's in the middle of establishing call with Charlie!

**What Should Happen:**

Alice should:
1. Acknowledge Bob's BYE (200 OK)
2. Clean up dialog with Bob
3. **Continue** managing transfer to Charlie
4. Keep sending NOTIFY to Bob (even though dialog is terminated - NOTIFY uses event subscription)

---

## Do We Need Special Transfer State Transitions?

### Short Answer: YES, with specific patterns

We don't need completely new states, but we need **transfer-aware transition logic**.

### Recommended Approach: Role-Based Transfer Context

#### Option 1: Implicit Context (Metadata-Based) ⭐ **RECOMMENDED**

Keep generic states but add **conditional actions** based on session metadata:

```yaml
# When transferee receives BYE from transferor
- role: "UAC"  # Alice is UAC in the original call with Bob
  state: "Active"
  event:
    type: "DialogBYE"
  next_state: "Active"  # Stay active! We have another call ongoing
  conditions:
    is_transfer_in_progress: true  # Check session metadata
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "CleanupOriginalTransferDialog"  # Clean up Bob's dialog
    # DON'T stop media - we're using it for Charlie's call
    # DON'T terminate - transfer to Charlie is ongoing
  description: "Transferor hung up during transfer - acknowledge but continue"

# Normal BYE (not during transfer)
- role: "Both"
  state: "Active"
  event:
    type: "DialogBYE"
  next_state: "Terminating"
  conditions:
    is_transfer_in_progress: false
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "StopMediaSession"
  description: "Normal remote hangup"
```

**Advantages:**
- Reuses existing states
- Clean separation of concerns
- Easy to understand intent
- Minimal state table complexity

**Implementation:**
- Add `is_transfer_in_progress` condition checker
- Add `is_transferor_dialog` session metadata
- Add `CleanupOriginalTransferDialog` action (partial cleanup)

#### Option 2: Explicit Transfer States (NOT RECOMMENDED)

Create dedicated states like:
- "TransferActiveAsTransferor"
- "TransferActiveAsTransferee"
- "TransferActiveAsTarget"

**Disadvantages:**
- State explosion (3x states for every scenario)
- Harder to maintain
- Doesn't provide clear value over metadata approach

### Specific State Transitions Needed

#### 1. **Transferor (Bob) Initiates Transfer**

**Current:**
```yaml
- role: "Both"
  state: "Active"
  event:
    type: "BlindTransfer"
  next_state: "Transferring"
  actions:
    - type: "SendREFER"
```

**Should Be:**
```yaml
- role: "Both"
  state: "Active"
  event:
    type: "BlindTransfer"
  next_state: "Active"  # Stay in Active!
  actions:
    - type: "SendREFER"
    - type: "MarkAsTransferor"  # Set metadata
  description: "Initiate blind transfer - send REFER"

# Then Bob decides when to hang up (immediate for blind, delayed for consultative)
- role: "Both"
  state: "Active"
  event:
    type: "HangupCall"
  next_state: "Terminating"
  conditions:
    is_transferor: true
  actions:
    - type: "SendBYE"
    - type: "StopMediaSession"
  description: "Transferor hangs up after REFER"
```

#### 2. **Transferee (Alice) Receives REFER**

**Current:** Missing - no transition defined

**Should Be:**
```yaml
- role: "Both"
  state: "Active"
  event:
    type: "TransferRequested"
  next_state: "Active"  # Stay active during transfer
  actions:
    - type: "AcceptREFER"  # Send 202 Accepted
    - type: "SendNOTIFY"   # 100 Trying
    - type: "MarkAsTransferee"
    - type: "CreateTransferCall"  # New INVITE to target
  description: "Accept transfer request and call target"

# Alice receives BYE from Bob while transfer in progress
- role: "Both"
  state: "Active"
  event:
    type: "DialogBYE"
  next_state: "Active"  # DON'T terminate - call with Charlie ongoing
  conditions:
    is_transferee: true
    has_active_transfer_call: true
  actions:
    - type: "SendSIPResponse"
      code: 200
    - type: "CleanupTransferorDialog"
    # Keep media running for Charlie call
  description: "Transferor hung up - continue transfer call"
```

#### 3. **Transfer Target (Charlie) Receives Call**

**Current:** Generic incoming call - works OK

**Enhancement:**
```yaml
- role: "UAS"
  state: "Ringing"
  event:
    type: "AcceptCall"
  next_state: "Answering"
  actions:
    - type: "GenerateLocalSDP"
    - type: "NegotiateSDPAsUAS"
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
  description: "Accept incoming call (including transfer calls)"

# Charlie can hang up from Answering state (FIX for current bug)
- role: "UAS"
  state: "Answering"
  event:
    type: "HangupCall"
  next_state: "Terminating"
  actions:
    - type: "SendBYE"
    - type: "StopMediaSession"
  description: "Hangup before receiving ACK"

# Charlie receives BYE while in Answering
- role: "UAS"
  state: "Answering"
  event:
    type: "DialogBYE"
  next_state: "Terminated"
  actions:
    - type: "SendSIPResponse"
      code: 200
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  description: "Remote party hung up before ACK"
```

---

## Required New Actions

Based on RFC requirements, we need these new actions:

### 1. `AcceptREFER`
```rust
Action::AcceptREFER => {
    // Send 202 Accepted response to REFER request
    dialog_adapter.send_refer_response(&session.session_id, 202, "Accepted").await?;
}
```

### 2. `SendNOTIFY`
```rust
Action::SendNOTIFY { status } => {
    // Send NOTIFY with SIP fragment body
    // E.g., "SIP/2.0 100 Trying", "SIP/2.0 200 OK"
    dialog_adapter.send_notify_for_refer(&session.session_id, status).await?;
}
```

### 3. `CreateTransferCall`
```rust
Action::CreateTransferCall => {
    // Create new session for transfer target
    // Mark it with transfer metadata
    // Trigger MakeCall event on new session
    transfer_coordinator.initiate_transfer_call(&session.session_id).await?;
}
```

### 4. `CleanupTransferorDialog` (partial cleanup)
```rust
Action::CleanupTransferorDialog => {
    // Clean up dialog with transferor (Bob)
    // But DON'T stop media - it's being used for transfer target (Charlie)
    // DON'T mark session as terminated - transfer call ongoing
    dialog_adapter.cleanup_dialog_only(&session.dialog_id).await?;
}
```

### 5. `MarkAsTransferor` / `MarkAsTransferee`
```rust
Action::MarkAsTransferor => {
    session.is_transferor = true;
}

Action::MarkAsTransferee => {
    session.is_transferee = true;
}
```

---

## Required New Conditions

### 1. `is_transferor`
```rust
Condition::IsTransferor => {
    session.is_transferor
}
```

### 2. `is_transferee`
```rust
Condition::IsTransferee => {
    session.is_transferee
}
```

### 3. `is_transfer_in_progress`
```rust
Condition::IsTransferInProgress => {
    session.is_transferee && session.transfer_call_session_id.is_some()
}
```

### 4. `has_active_transfer_call`
```rust
Condition::HasActiveTransferCall => {
    if let Some(transfer_session_id) = &session.transfer_call_session_id {
        if let Ok(transfer_session) = session_store.get_session(transfer_session_id).await {
            return matches!(transfer_session.call_state, CallState::Active | CallState::Initiating);
        }
    }
    false
}
```

---

## Corrected Transfer Flow (RFC Compliant)

### Blind Transfer (Should Be)

```
Alice (Transferee)          Bob (Transferor)          Charlie (Transfer Target)
      |                           |                           |
      | [Active call with Bob]    |                           |
      |                           |                           |
      |<-------- REFER ------------|  (1) Bob sends REFER
      |                           |      Refer-To: sip:charlie@...
      |                           |
      |-------- 202 Accepted ---->|  (2) Alice accepts
      |-------- NOTIFY ---------->|  (3) Alice: "100 Trying"
      |<------- 200 OK -----------|
      |                           |
      |---------- BYE <-----------|  (4) Bob hangs up (blind)
      |--------> 200 OK ----------|      Alice cleans up Bob's dialog
      |                           |
      |---------- INVITE ----------------------->|  (5) Alice calls Charlie
      |<--------- 180 Ringing -------------------|
      |                           |
      |--------- NOTIFY --------->|  (6) Alice: "180 Ringing"
      |<-------- 200 OK ----------|      (sent to Bob's event subscription)
      |                           |
      |<--------- 200 OK ------------------------|  (7) Charlie answers
      |---------- ACK -------------------------->|  (8) Alice sends ACK
      |                           |
      |--------- NOTIFY --------->|  (9) Alice: "200 OK" (success)
      |<-------- 200 OK ----------|
      |                           |
      |<========== RTP ==========================>|  (10) Alice talks to Charlie
```

### Key Corrections from Current Implementation

1. **Step 4:** Bob sends BYE to Alice (not Alice to Bob)
2. **Step 8:** Alice MUST send ACK to Charlie (currently missing)
3. **Steps 3, 6, 9:** Alice sends NOTIFY updates even after BYE (uses event subscription, not dialog)
4. **Alice's state:** Remains "Active" throughout - doesn't go to "Terminating" when Bob hangs up

---

## Implementation Priority

### High Priority (Blocking Issues) 🔴

1. **Fix ACK sending** - Alice must send ACK to Charlie
   - Location: `state_machine/executor.rs` or event handler
   - Missing: `Dialog200OK` event not triggering `SendACK` action

2. **Fix BYE responsibility** - Bob should hang up Alice, not vice versa
   - Location: `transfer/coordinator.rs:258-266`
   - Change: Remove `terminate_old_call` from transferee side
   - Add: Transferor calls `hangup()` after sending REFER

3. **Add Answering→Terminating transition** - Charlie can't hang up
   - Location: `state_tables/default.yaml`
   - Add: HangupCall and DialogBYE transitions for "Answering" state

### Medium Priority (RFC Compliance) 🟡

4. **Add transfer-aware BYE handling** - Alice shouldn't terminate when Bob hangs up during transfer
   - Location: `state_tables/default.yaml`
   - Add: Conditional DialogBYE transition for transferees
   - Add: `is_transferee` condition checker

5. **Implement proper NOTIFY flow** - Alice should send status updates
   - Location: Already partially implemented in `transfer/notify.rs`
   - Fix: Send NOTIFY even after dialog with Bob is terminated

6. **Add TransferRequested event handling** - Alice needs to accept REFER properly
   - Location: `state_tables/default.yaml`
   - Add: Complete transition with AcceptREFER and CreateTransferCall actions

### Low Priority (Nice to Have) 🟢

7. **Add transfer failure recovery** - Handle cases where transfer to Charlie fails
8. **Add consultative transfer** - Wait for success NOTIFY before hanging up
9. **Add transfer progress tracking** - Better visibility into transfer state

---

## Conclusion

### Summary

**YES, we need transfer-specific state transitions**, but not completely separate states. The key is:

1. **Add conditional logic** based on transfer role (transferor/transferee/target)
2. **Fix BYE responsibility** - transferor hangs up transferee per RFC
3. **Fix ACK sending** - critical for dialog establishment
4. **Add partial cleanup actions** - clean up one dialog while keeping another active

### Why Generic States Aren't Enough

The problem with purely generic states is that **a transferee is simultaneously**:
- Terminating one call (with transferor)
- Establishing another call (with target)
- Maintaining event subscription (for NOTIFY)

A single "Active" or "Terminating" state can't capture this complexity without conditional logic.

### Recommended Architecture

```
State Machine
    ↓
Check Conditions (is_transferor, is_transferee, has_transfer_call, etc.)
    ↓
Execute Appropriate Actions
    ↓
Transition to Next State (may stay in same state!)
```

This gives us **role-based polymorphism** without state explosion.

---

## Next Steps

1. Read BYE_MESSAGE_ANALYSIS.md for specific bug details
2. Implement Priority 1 fixes (ACK, BYE, Answering transitions)
3. Add transfer-aware conditions and actions
4. Update state table with conditional transitions
5. Test end-to-end transfer flow
6. Verify RFC compliance with SIP trace analysis

---

## References

- [RFC 3515 - The SIP REFER Method](https://datatracker.ietf.org/doc/html/rfc3515)
- [RFC 5589 - SIP Call Control - Transfer](https://datatracker.ietf.org/doc/html/rfc5589)
- [RFC 3891 - The SIP Replaces Header](https://datatracker.ietf.org/doc/html/rfc3891)
- [Current Implementation Analysis](./BYE_MESSAGE_ANALYSIS.md)
- [Blind Transfer Implementation](./BLIND_TRANSFER_COMPLETE_IMPLEMENTATION.md)
