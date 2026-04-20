# Blind Transfer BYE Message Analysis

**Date:** 2025-10-02
**Test Run:** `run_blind_transfer.sh` logs from 2025-10-02 11:27:50
**Issue:** Charlie does not send BYE to Alice, Alice times out after 30 seconds

---

## Executive Summary

The blind transfer test fails because **Charlie cannot send a BYE message** when `hangup()` is called. The root cause is that Charlie's session is stuck in the "Answering" state (waiting for ACK), and the state table has **no transition defined for `HangupCall` from the "Answering" state**. Additionally, Alice never sends the required ACK after receiving Charlie's 200 OK response, which prevents both peers from reaching the "Active" state.

---

## Root Cause Analysis

### **PRIMARY ISSUE: Missing State Transition**

**Charlie is in "Answering" state when `hangup()` is called, and there's NO state transition defined for `HangupCall` event from the "Answering" state.**

#### Evidence:

1. **Charlie's Final State** ([charlie_20251002_112750.log:70]())
   ```
   State transition: Ringing -> Answering
   ```
   - Charlie accepts the call and sends 200 OK
   - Log ends immediately after - **never reaches "Active" state**

2. **State Table Gap** ([default.yaml:207-219]())
   - Transition from "Answering" → "Active" requires `DialogACK` event
   - Charlie never receives this ACK from Alice
   - State table defines `HangupCall` transitions for:
     - ✅ "Active" state (line 238-246)
     - ✅ "TransferringCall" state (line 458-465)
     - ✅ "ConsultationCall" state (line 512-521)
     - ❌ **"Answering" state - MISSING**

3. **Hangup Attempt Fails** ([peer3_target.rs:56]())
   ```rust
   charlie.hangup(&incoming.id).await?;
   ```
   - This calls `EventType::HangupCall` on the state machine
   - Since Charlie is in "Answering" state with no matching transition
   - **The event is silently ignored or fails without logs**
   - No `Action::SendBYE` is ever executed

---

### **SECONDARY ISSUE: Alice Never Sends ACK**

**Alice receives 200 OK from Charlie but never sends the required ACK response.**

#### Evidence:

1. **Alice Receives 200 OK** ([alice_20251002_112750.log:119-121]())
   ```
   Transaction Key(...) received success response 200 OK for dialog a3f04a55-0fa6-4951-ab7d-37a4096fe693
   Updating remote tag for dialog...
   Updated dialog lookup for confirmed dialog...
   ```

2. **No ACK Sent**
   - No log entry showing `Action::SendACK` execution
   - No dialog-core log showing ACK transmission

3. **Alice's State** ([alice_20251002_112750.log:112]())
   ```
   State transition: Idle -> Initiating
   ```
   - Alice's new session (`session-276aaec9-49fb-4180-af39-ccc0d5aa4a01`) goes to "Initiating"
   - **Never transitions to "Active"**
   - Should transition on `Dialog200OK` event per state table (line 117-133)

#### Why ACK Might Be Missing:

Looking at the state table transition for UAC receiving 200 OK:
```yaml
- role: "UAC"
  state: "Initiating"
  event:
    type: "Dialog200OK"
  next_state: "Active"
  actions:
    - type: "SendACK"
    - type: "NegotiateSDPAsUAC"
    - type: "StartMediaSession"
```

The `Dialog200OK` event should trigger ACK sending, but Alice's logs show no evidence this transition occurred.

---

### **TERTIARY ISSUE: Session ID Mismatch**

**Transfer coordinator creates a session with one ID but the actual session uses a different ID.**

#### Evidence:

[alice_20251002_112750.log:114]()
```
⚠️  Session ID mismatch: expected transfer-78361907-ffce-41d4-a35a-01f8da24c484,
    got session-276aaec9-49fb-4180-af39-ccc0d5aa4a01
```

#### Impact:

- Loss of session tracking in transfer coordinator
- Potential inability to route events to correct session
- Dialog operations (like BYE) may target wrong session
- Internal state inconsistency

#### Location:

[transfer/coordinator.rs]() - Session creation logic during blind transfer handling

---

## Observed Behavior

### Timeline of Events:

1. **Alice calls Bob** ✅
2. **Bob accepts and establishes call** ✅
3. **Bob sends REFER to Alice** ✅ (line 76, bob.log)
4. **Alice accepts REFER with 202 Accepted** ✅ (line 85, alice.log)
5. **Alice calls Charlie** ✅ (line 102, alice.log)
6. **Charlie accepts with 200 OK** ✅ (line 67, charlie.log)
7. **Alice receives 200 OK** ✅ (line 119-121, alice.log)
8. ❌ **Alice never sends ACK**
9. ❌ **Charlie stuck in "Answering" state**
10. **Charlie calls hangup()** (prints message but no SIP action)
11. ❌ **Charlie never sends BYE**
12. ❌ **Alice never receives BYE**
13. ❌ **Alice hangs indefinitely, killed by timeout (exit 124)**

### Log Gaps:

**Charlie's log:**
- ✅ Line 28-70: Normal incoming call handling
- ✅ Line 71: "💬 Now talking to Alice (post-transfer)..."
- ✅ Line 72: "📴 Hanging up..."
- ✅ Line 73: "✅ Test complete!"
- ❌ **NO SIP transaction logs after line 70**
- ❌ **NO BYE transaction creation**
- ❌ **NO state transition logs**

**Alice's log:**
- ✅ Line 1-122: Complete call flow including transfer
- ❌ **NO ACK sending after receiving 200 OK (line 119-121)**
- ❌ **NO transition to "Active" for new session**
- ❌ **NO incoming BYE received**
- ❌ **NO cleanup or termination sequence**
- ❌ **Process hangs until SIGTERM**

---

## Why Alice Times Out

Alice's process hangs because:

1. **SimplePeer Never Exits**
   - The `SimplePeer` wrapper waits for coordinator shutdown
   - Coordinator is still running because dialogs are active
   - No graceful shutdown mechanism

2. **Dialog Never Fully Established**
   - INVITE to Charlie sent
   - 200 OK received
   - **ACK never sent** → Dialog incomplete
   - State machine stuck in "Initiating"

3. **No Termination Signal**
   - Charlie never sends BYE (can't from "Answering" state)
   - Alice has no reason to terminate
   - No timeout mechanism for incomplete dialogs
   - Test script must use SIGTERM (exit code 124)

---

## SIP Protocol Correctness Issues

### RFC 3261 Compliance Problems:

1. **Missing ACK (RFC 3261 Section 13.2.2.4)**
   - UAC MUST send ACK for 2xx responses to INVITE
   - Failure to send ACK leaves transaction incomplete
   - UAS will retransmit 200 OK (not observed in logs - may indicate dialog-core issue)

2. **BYE from Non-Established Dialog**
   - SIP allows BYE to be sent from UAS before receiving ACK
   - State table should support this scenario
   - Current implementation blocks it

3. **Dialog Termination**
   - Either peer should be able to send BYE at any time after 200 OK
   - Charlie cannot exercise this right due to state restriction

---

## Recommended Fixes

### Priority 1: Fix ACK Sending (Critical)

**Problem:** Alice doesn't send ACK after receiving 200 OK

**Solution:** Investigate why `Dialog200OK` event isn't being processed:
- Check dialog-core event emission for 200 OK responses
- Verify session-core-v2 event handler is subscribed
- Ensure event routing from dialog-to-session works for UAC
- Add explicit logging in state machine executor for unmatched events

**Files to Check:**
- `crates/session-core-v2/src/adapters/session_event_handler.rs`
- `crates/dialog-core/src/manager/transaction_integration.rs`
- `crates/session-core-v2/src/state_machine/executor.rs`

### Priority 2: Add Answering→Terminating Transition (High)

**Problem:** No `HangupCall` transition from "Answering" state

**Solution:** Add state table entry:

```yaml
# UAS can hangup while waiting for ACK
- role: "UAS"
  state: "Answering"
  event:
    type: "HangupCall"
  next_state: "Terminating"
  actions:
    - type: "SendBYE"
    - type: "StopMediaSession"
  description: "Hangup before receiving ACK (early termination)"
```

**File:** `crates/session-core-v2/state_tables/default.yaml`

### Priority 3: Fix Session ID Consistency (Medium)

**Problem:** Transfer coordinator creates session with wrong ID

**Solution:**
- Ensure `TransferCoordinator::complete_transfer()` uses actual session ID from `make_call()`
- Remove or fix the `transfer-*` session ID creation
- Update session tracking to use returned session ID

**File:** `crates/session-core-v2/src/transfer/coordinator.rs`

**Location:** Around line where warning is logged

### Priority 4: Add Graceful Shutdown (Medium)

**Problem:** Alice hangs indefinitely when transfer completes

**Solution:**
- Add timeout mechanism for incomplete dialogs (e.g., 32 seconds per RFC 3261)
- Implement `SimplePeer::shutdown()` method
- Add dialog cleanup when all sessions terminate
- Consider adding "transfer complete" event for transferee

**Files:**
- `crates/session-core-v2/src/api/simple.rs`
- `crates/session-core-v2/examples/blind_transfer/peer1_caller.rs`

### Priority 5: Handle Incoming BYE in Non-Active States (Low)

**Problem:** If BYE is received in "Initiating" or "Answering", no transition defined

**Solution:** Add transitions for `DialogBYE` in "Initiating", "Answering", "Ringing" states

```yaml
- role: "Both"
  state: "Initiating"
  event:
    type: "DialogBYE"
  next_state: "Terminated"
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  description: "Remote hangup during call setup"
```

---

## Testing Recommendations

After implementing fixes:

1. **Verify ACK Sending**
   ```bash
   grep -i "SendACK\|send.*ack" logs/*.log
   ```
   Should show ACK being sent by Alice after 200 OK

2. **Verify BYE Sending**
   ```bash
   grep -i "SendBYE\|send.*bye\|Method::Bye" logs/*.log
   ```
   Should show Charlie sending BYE

3. **Verify State Transitions**
   ```bash
   grep "State transition" logs/charlie*.log | tail -5
   grep "State transition" logs/alice*.log | tail -10
   ```
   Should show "Answering → Terminating" for Charlie
   Should show "Initiating → Active" for Alice's new session

4. **Verify Clean Exit**
   - Alice should exit cleanly without timeout
   - Exit code should be 0 (not 124)
   - Test should complete in < 20 seconds

5. **Capture SIP Traces** (optional)
   ```bash
   tcpdump -i lo0 -w blind_transfer.pcap port 5060 or port 5061 or port 5062
   ```
   Verify:
   - ACK from Alice to Charlie
   - BYE from Charlie to Alice
   - 200 OK from Alice to Charlie (BYE response)

---

## Additional Observations

### Session ID Tracking

The transfer coordinator creates a preliminary session ID (`transfer-78361907...`) but then the actual session created by `make_call()` has a different ID (`session-276aaec9...`). This suggests:

- The coordinator should track the actual session ID returned
- Or the coordinator should pass its session ID to `make_call()` to use
- Current mismatch could cause subtle bugs in session tracking

### Auto-Transfer Handler

From [api/simple.rs:58-60]():
```rust
// Enable automatic blind transfer handling
// This makes transfers "just work" for SimplePeer users
coordinator.enable_auto_transfer();
```

The auto-transfer handler successfully:
- ✅ Receives REFER
- ✅ Sends 202 Accepted
- ✅ Creates new call to transfer target
- ❌ But doesn't complete the dialog establishment
- ❌ And doesn't provide clean termination

### Missing Dialog-Core Logs

Charlie's log shows no dialog-core transaction logs after accepting the call. This could indicate:
- Hangup attempt failed silently
- No error was returned to application
- State machine rejected event without logging
- Need to add more detailed logging for unmatched events

---

## Conclusion

The blind transfer implementation is **80% correct** but fails at the final dialog establishment phase due to:

1. Missing ACK from UAC (Alice) - likely an event routing issue
2. Missing state transition for hangup from "Answering" - state table gap
3. Session ID inconsistency in transfer coordinator - tracking bug

All three issues are **fixable with targeted changes** to:
- Event handling in session-core-v2 (investigate ACK sending)
- State table (add Answering→Terminating transition)
- Transfer coordinator (fix session ID tracking)

Once fixed, the blind transfer should work correctly with proper SIP dialog lifecycle management.
