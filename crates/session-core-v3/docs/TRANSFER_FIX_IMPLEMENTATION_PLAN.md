# Transfer Fix Implementation Plan - Detailed

**Date:** 2025-10-02
**Goal:** Fix blind transfer to be RFC-compliant, resolve BYE/ACK issues
**References:**
- [BYE_MESSAGE_ANALYSIS.md](../examples/blind_transfer/BYE_MESSAGE_ANALYSIS.md) - Bug details
- [TRANSFER_RFC_ANALYSIS.md](./TRANSFER_RFC_ANALYSIS.md) - RFC requirements

---

## Overview

This plan maintains the existing session-core-v2 architecture while fixing three critical issues:

1. **Missing ACK** - Alice doesn't send ACK to Charlie after 200 OK
2. **Wrong BYE responsibility** - Alice hangs up Bob (should be Bob hangs up Alice)
3. **Missing state transitions** - Charlie can't hang up from "Answering" state

The implementation uses **conditional transitions** based on session metadata rather than creating new states.

---

## Architecture Integration

### Core Principles

✅ **Keep:**
- State table YAML-driven architecture
- Generic states (Idle, Active, Terminating, etc.)
- Existing adapters (DialogAdapter, MediaAdapter)
- Transfer coordinator pattern

✅ **Add:**
- Transfer role tracking in SessionState
- Conditional guard evaluation
- New actions for transfer-specific behavior
- Partial cleanup actions

❌ **Avoid:**
- New transfer-specific states
- Duplicating state transitions
- Breaking existing non-transfer call flows

---

## Phase 1: Critical Bug Fixes (High Priority) 🔴

These must be fixed for basic transfer to work at all.

### 1.1 Fix Missing ACK from Alice to Charlie

**Problem:** Alice receives 200 OK but never sends ACK, leaving Charlie stuck in "Answering"

**Root Cause Analysis:**

After deep investigation, we discovered an **architectural layer boundary issue**:

1. **dialog-core DOES send ACK automatically** (see `protocol/response_handler.rs:273-284`) - this is correct per RFC 3261
2. **session-core-v2 state table has `SendACK` action** - attempting to control what dialog-core already handles
3. **dialog-core doesn't notify session-core after sending ACK** - session-core never knows ACK was sent
4. **Charlie (UAS) never receives `DialogACK` event** - can't transition from "Answering" → "Active"

**Correct Solution:** Preserve layer separation - dialog-core handles SIP protocol, session-core reacts to events.

---

#### Fix 1a: dialog-core publishes AckSent event

**File: `crates/dialog-core/src/events/session_coordination.rs`**

Add new event variant:

```rust
#[derive(Debug, Clone)]
pub enum SessionCoordinationEvent {
    // ... existing events ...

    /// ACK was sent by UAC for 200 OK response to INVITE
    AckSent {
        dialog_id: DialogId,
    },
}
```

---

#### Fix 1b: dialog-core publishes event after sending ACK

**File: `crates/dialog-core/src/protocol/response_handler.rs`**

**Change location:** Around line 280-284, after automatic ACK is sent

**Before:**
```rust
if let Err(e) = self.send_automatic_ack_for_2xx(&transaction_id, &response, &dialog_id).await {
    warn!("Failed to send automatic ACK for 200 OK to INVITE: {}", e);
} else {
    info!("Successfully sent automatic ACK for 200 OK to INVITE");
}
```

**After:**
```rust
if let Err(e) = self.send_automatic_ack_for_2xx(&transaction_id, &response, &dialog_id).await {
    warn!("Failed to send automatic ACK for 200 OK to INVITE: {}", e);
} else {
    info!("Successfully sent automatic ACK for 200 OK to INVITE");

    // Notify session-core that ACK was sent (for state machine transition)
    self.notify_session_layer(SessionCoordinationEvent::AckSent {
        dialog_id: dialog_id.clone(),
    }).await?;
}
```

---

#### Fix 1c: session-core-v2 receives AckSent event

**File: `crates/session-core-v2/src/adapters/session_event_handler.rs`**

**Location:** In the `handle_dialog_event` match statement where dialog events are processed

**Add new match arm:**

```rust
impl SessionCrossCrateEventHandler {
    async fn handle_dialog_event(&self, event: DialogToSessionEvent) -> Result<()> {
        match event {
            // ... existing event handlers ...

            DialogToSessionEvent::AckSent { dialog_id } => {
                // Find session by dialog ID
                if let Some(session_id) = self.dialog_adapter.get_session_for_dialog(&dialog_id).await? {
                    info!("ACK was sent by dialog-core for dialog {}, triggering DialogACK event", dialog_id);

                    // Trigger DialogACK event in state machine
                    // This allows UAS to transition from "Answering" -> "Active"
                    self.state_machine.process_event(
                        &session_id,
                        EventType::DialogACK,
                    ).await?;
                } else {
                    warn!("Received AckSent for unknown dialog {}", dialog_id);
                }
            }

            // ... other events ...
        }
        Ok(())
    }
}
```

---

#### Fix 1d: Remove or make SendACK action a no-op

**File: `crates/session-core-v2/src/state_machine/actions.rs`**

**Location:** In the `execute_action` match statement

**Change:**

```rust
Action::SendACK => {
    // NO-OP: dialog-core sends ACK automatically
    // This action exists in state table for documentation purposes only
    // The actual ACK is sent by dialog-core's protocol/response_handler.rs
    info!("SendACK action triggered (handled automatically by dialog-core)");
}
```

**OR** update state table to remove SendACK actions entirely (but keep for clarity).

---

**Testing:**
```bash
# Check Alice's logs
grep -n "ACK was sent by dialog-core\|DialogACK" logs/alice*.log
# Should show: "ACK was sent by dialog-core for dialog ..."
# Should show: "Triggering DialogACK event"

# Check Charlie's logs
grep -n "State transition.*Answering.*Active" logs/charlie*.log
# Should show: "State transition: Answering -> Active"
```

---

**Architecture Note:**

This fix preserves proper layer separation:
- ✅ **dialog-core:** Handles SIP protocol (sends ACK automatically per RFC 3261)
- ✅ **session-core-v2:** Reacts to protocol events (transitions state machine)
- ✅ **No duplicate ACKs:** Only dialog-core sends ACK on the wire
- ✅ **Clean event flow:** dialog-core → event → session-core-v2 → state transition

---

### 1.2 Add Missing "Answering" State Transitions

**Problem:** Charlie is stuck in "Answering" with no HangupCall or DialogBYE transitions

#### File: `crates/session-core-v2/state_tables/default.yaml`

**Changes Required:**

```yaml
# After line 219 (existing Answering → Active transition)

# UAS can hangup while waiting for ACK (before dialog is fully established)
- role: "UAS"
  state: "Answering"
  event:
    type: "HangupCall"
  next_state: "Terminating"
  actions:
    - type: "SendBYE"
    - type: "StopMediaSession"
  description: "UAS hangs up before receiving ACK"

# UAS receives BYE before ACK (peer changed their mind)
- role: "UAS"
  state: "Answering"
  event:
    type: "DialogBYE"
  next_state: "Terminated"
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  description: "Remote party sent BYE before ACK"
```

**Testing:**
```bash
# Run test and check Charlie's log
grep "State transition.*Answering" logs/charlie*.log
# Should show: "State transition: Answering -> Terminating"
grep "SendBYE\|send.*BYE" logs/charlie*.log
# Should show BYE being sent
```

---

### 1.3 Fix BYE Responsibility (Bob should hang up Alice)

**Problem:** Alice hangs up Bob when `terminate_old_call` is true, but RFC says Bob should hang up Alice

#### File: `crates/session-core-v2/src/transfer/coordinator.rs`

**Changes Required:**

```rust
// Line 257-269 - REMOVE this section entirely
// Step 6: Optionally terminate old call
if options.terminate_old_call {
    info!("📴 Terminating old call for session {}", transferee_session_id);

    if let Err(e) = self.state_machine_helpers.hangup(transferee_session_id).await {
        error!("Failed to hang up old call: {}", e);
    } else {
        info!("✅ Hung up old call");
    }
} else {
    info!("🔗 Keeping old call alive (managed transfer mode)");
}

// REPLACE WITH:
// Step 6: For blind transfer, transferee (Alice) does NOT hang up
// The transferor (Bob) is responsible for hanging up per RFC 5589
info!("🔗 Transferee keeps dialog alive for NOTIFY messages (RFC 5589)");

// Note: The transferor (Bob) should call hangup() after sending REFER
// This is controlled by the application logic, not the coordinator
```

#### File: `crates/session-core-v2/examples/blind_transfer/peer2_transferor.rs`

**Changes Required:**

```rust
// Around line 73 where Bob sends REFER
println!("[BOB] 🔄 Initiating blind transfer to Charlie at sip:charlie@127.0.0.1:5062...");
bob.transfer(&call_id, "sip:charlie@127.0.0.1:5062").await?;

println!("[BOB] ✅ Transfer initiated!");

// ADD THIS: Bob hangs up immediately (blind transfer)
println!("[BOB] 📴 Hanging up with Alice (blind transfer)...");
sleep(Duration::from_millis(100)).await; // Brief delay for REFER to be sent
bob.hangup(&call_id).await?;

println!("[BOB] 🔄 Alice should now be talking to Charlie");
println!("[BOB] ✅ Test complete!");
```

**Testing:**
```bash
grep "BYE" logs/*.log | grep -E "Alice|Bob"
# Should show: Bob sending BYE to Alice
# Should NOT show: Alice sending BYE to Bob
```

---

## Phase 2: Transfer-Aware State Transitions (Medium Priority) 🟡

These changes make the state machine aware of transfer context.

### 2.1 Add Transfer Role Tracking to SessionState

**Purpose:** Track whether session is transferor, transferee, or target

#### File: `crates/session-core-v2/src/session_store/state.rs`

**Changes Required:**

```rust
// Around line 79-83, add these fields after transferor_session_id:

    // Transfer role tracking
    pub is_transferor: bool,    // This session initiated a transfer (Bob)
    pub is_transferee: bool,    // This session received REFER (Alice)
    pub is_transfer_target: bool, // This session is the target of transfer (Charlie)

    // Track if we're in the middle of a transfer
    pub transfer_call_session_id: Option<SessionId>, // New call being established during transfer
```

```rust
// In impl SessionState, around line 120-126, initialize new fields:

    is_transferor: false,
    is_transferee: false,
    is_transfer_target: false,
    transfer_call_session_id: None,
```

**Testing:**
```rust
// Add unit test
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_roles() {
        let mut state = SessionState::new(SessionId::new(), Role::UAC);
        assert!(!state.is_transferor);
        assert!(!state.is_transferee);

        state.is_transferor = true;
        assert!(state.is_transferor);
    }
}
```

---

### 2.2 Add Transfer-Related Guards

**Purpose:** Enable conditional transitions based on transfer context

#### File: `crates/session-core-v2/src/state_table/types.rs`

**Changes Required:**

```rust
// Around line 305-320 where Guard enum is defined, add:

pub enum Guard {
    // ... existing guards ...

    // Transfer-related guards
    IsTransferor,
    IsTransferee,
    IsTransferTarget,
    HasActiveTransferCall,
    IsTransferInProgress,
}
```

#### File: `crates/session-core-v2/src/state_machine/guards.rs`

**Changes Required:**

```rust
// Around line 20-28, add guard implementations:

pub async fn check_guard(guard: &Guard, session: &SessionState) -> bool {
    match guard {
        // ... existing guards ...

        Guard::IsTransferor => session.is_transferor,
        Guard::IsTransferee => session.is_transferee,
        Guard::IsTransferTarget => session.is_transfer_target,
        Guard::HasActiveTransferCall => {
            session.transfer_call_session_id.is_some()
        }
        Guard::IsTransferInProgress => {
            session.is_transferee && session.transfer_call_session_id.is_some()
        }
    }
}
```

#### File: `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Changes Required:**

```rust
// Around line 550-570 where guards are parsed from YAML, add:

fn parse_guard(guard_str: &str) -> Result<Guard, String> {
    match guard_str {
        // ... existing guard parsing ...

        "is_transferor" => Ok(Guard::IsTransferor),
        "is_transferee" => Ok(Guard::IsTransferee),
        "is_transfer_target" => Ok(Guard::IsTransferTarget),
        "has_active_transfer_call" => Ok(Guard::HasActiveTransferCall),
        "is_transfer_in_progress" => Ok(Guard::IsTransferInProgress),

        _ => Err(format!("Unknown guard: {}", guard_str))
    }
}
```

---

### 2.3 Add Transfer-Related Actions

**Purpose:** Actions to mark roles and handle partial cleanup

#### File: `crates/session-core-v2/src/state_table/types.rs`

**Changes Required:**

```rust
// Around line 366 in Action enum, add:

pub enum Action {
    // ... existing actions ...

    // Transfer role marking actions
    MarkAsTransferor,
    MarkAsTransferee,
    MarkAsTransferTarget,

    // Transfer-specific cleanup
    CleanupTransferorDialog, // Clean up Bob's dialog but keep Alice's session active
    AcceptREFER,             // Send 202 Accepted to REFER
    SendTransferNOTIFY { status: String }, // Send NOTIFY about transfer progress
    CreateTransferCall,      // Initiate new call to transfer target
}
```

#### File: `crates/session-core-v2/src/state_machine/actions.rs`

**Changes Required:**

```rust
// Around line 86-100, add action implementations:

pub async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    session_store: &Arc<SessionStore>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        // ... existing actions ...

        Action::MarkAsTransferor => {
            info!("Marking session {} as transferor", session.session_id);
            session.is_transferor = true;
        }

        Action::MarkAsTransferee => {
            info!("Marking session {} as transferee", session.session_id);
            session.is_transferee = true;
        }

        Action::MarkAsTransferTarget => {
            info!("Marking session {} as transfer target", session.session_id);
            session.is_transfer_target = true;
        }

        Action::CleanupTransferorDialog => {
            // Clean up dialog with transferor but keep media session
            // because we're using it for the new call with transfer target
            if let Some(dialog_id) = &session.dialog_id {
                info!("Cleaning up transferor dialog {} but keeping media for transfer", dialog_id);
                dialog_adapter.cleanup_dialog_only(dialog_id).await?;
                session.dialog_id = None;
            }
            // DON'T stop media - it's being used for transfer call
            // DON'T mark session as terminated
        }

        Action::AcceptREFER => {
            info!("Accepting REFER for session {}", session.session_id);
            dialog_adapter.send_refer_response(&session.session_id, 202, "Accepted").await?;
        }

        Action::SendTransferNOTIFY { status } => {
            info!("Sending NOTIFY with status '{}' for session {}", status, session.session_id);
            if let Some(transferor_id) = &session.transferor_session_id {
                dialog_adapter.send_notify_for_transfer(transferor_id, status).await?;
            }
        }

        Action::CreateTransferCall => {
            // This would trigger the transfer coordinator
            // For now, log that we need to create the call
            info!("CreateTransferCall action - would initiate call to transfer target");
            // Implementation TBD - might need to emit InternalMakeTransferCall event
        }
    }
    Ok(())
}
```

#### File: `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Changes Required:**

```rust
// Around line 660-680 where actions are parsed, add:

fn parse_action(action_map: &serde_yaml::Mapping) -> Result<Action, String> {
    let action_type = action_map.get(&Value::String("type".to_string()))
        .and_then(|v| v.as_str())
        .ok_or("Action must have 'type' field")?;

    match action_type {
        // ... existing actions ...

        "MarkAsTransferor" => Ok(Action::MarkAsTransferor),
        "MarkAsTransferee" => Ok(Action::MarkAsTransferee),
        "MarkAsTransferTarget" => Ok(Action::MarkAsTransferTarget),
        "CleanupTransferorDialog" => Ok(Action::CleanupTransferorDialog),
        "AcceptREFER" => Ok(Action::AcceptREFER),
        "SendTransferNOTIFY" => {
            let status = action_map.get(&Value::String("status".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or("100 Trying")
                .to_string();
            Ok(Action::SendTransferNOTIFY { status })
        }
        "CreateTransferCall" => Ok(Action::CreateTransferCall),

        _ => Err(format!("Unknown action type: {}", action_type))
    }
}
```

---

### 2.4 Add Transfer-Aware State Transitions

**Purpose:** Handle BYE differently based on transfer context

#### File: `crates/session-core-v2/state_tables/default.yaml`

**Changes Required:**

```yaml
# BEFORE the existing "Both sides can hangup from Active" transition (line 237),
# ADD these transfer-specific transitions:

# ========== TRANSFER-SPECIFIC TRANSITIONS ==========

# Transferor (Bob) initiates blind transfer
- role: "Both"
  state: "Active"
  event:
    type: "BlindTransfer"
  next_state: "Active"  # Stay active while waiting to hang up
  actions:
    - type: "SendREFER"
    - type: "MarkAsTransferor"
  description: "Transferor initiates blind transfer - send REFER"

# Transferee (Alice) receives REFER and accepts it
- role: "Both"
  state: "Active"
  event:
    type: "TransferRequested"
  next_state: "Active"  # Stay active during transfer
  actions:
    - type: "AcceptREFER"
    - type: "SendTransferNOTIFY"
      status: "SIP/2.0 100 Trying"
    - type: "MarkAsTransferee"
    - type: "CreateTransferCall"  # Initiates INVITE to target
  description: "Transferee accepts transfer and calls target"

# Transferee (Alice) receives BYE from transferor (Bob) during transfer
- role: "Both"
  state: "Active"
  event:
    type: "DialogBYE"
  next_state: "Active"  # DON'T terminate - transfer call ongoing!
  guards:
    - is_transferee
    - has_active_transfer_call
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "CleanupTransferorDialog"
    # Keep media running for transfer call
    # Keep session active for NOTIFY messages
  description: "Transferor hung up during transfer - acknowledge but continue"

# Original generic "Both sides can hangup from Active" transition follows
# This will match when NOT in transfer context (no guards)
- role: "Both"
  state: "Active"
  event:
    type: "HangupCall"
  next_state: "Terminating"
  actions:
    - type: "SendBYE"
    - type: "StopMediaSession"
  description: "Hangup active call (normal, non-transfer)"

# Original "Both sides receive BYE" follows, but with guard to exclude transfers
- role: "Both"
  state: "Active"
  event:
    type: "DialogBYE"
  next_state: "Terminating"
  guards:
    - not: is_transfer_in_progress  # Only match if NOT in transfer
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
    - type: "StopMediaSession"
  description: "Remote hangup (normal, non-transfer)"
```

**Note on Guard Syntax:**

You may need to add support for `not:` guard syntax in yaml_loader.rs:

```rust
// In parse_guards function
fn parse_guard_condition(value: &Value) -> Result<Guard, String> {
    if let Some(map) = value.as_mapping() {
        // Check for "not: guard_name" syntax
        if let Some(not_val) = map.get(&Value::String("not".to_string())) {
            if let Some(guard_str) = not_val.as_str() {
                let inner_guard = parse_guard(guard_str)?;
                return Ok(Guard::Not(Box::new(inner_guard)));
            }
        }
    }

    // Normal guard parsing
    if let Some(guard_str) = value.as_str() {
        return parse_guard(guard_str);
    }

    Err("Invalid guard format".to_string())
}
```

And add `Not` variant to Guard enum:

```rust
pub enum Guard {
    // ... existing guards ...
    Not(Box<Guard>),  // Logical NOT
}
```

---

## Phase 3: Dialog Adapter Enhancements (Medium Priority) 🟡

Add missing methods to support transfer operations.

### 3.1 Add REFER Response Method

#### File: `crates/session-core-v2/src/adapters/dialog_adapter.rs`

**Changes Required:**

```rust
// Around line 350, add:

impl DialogAdapter {
    // ... existing methods ...

    /// Send response to REFER request
    pub async fn send_refer_response(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        // Use dialog-core to send response
        self.dialog_api
            .send_response_in_dialog(&dialog_id, status_code, reason.to_string(), None)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER response: {}", e)))?;

        info!("Sent {} {} response to REFER for session {}", status_code, reason, session_id);
        Ok(())
    }

    /// Send NOTIFY for transfer status
    pub async fn send_notify_for_transfer(
        &self,
        transferor_session_id: &SessionId,
        status: &str,
    ) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(transferor_session_id)
            .ok_or_else(|| SessionError::SessionNotFound(transferor_session_id.0.clone()))?
            .clone();

        // Create SIP message fragment body
        let body = format!("{}\r\n", status);

        // Use dialog-core to send NOTIFY
        self.dialog_api
            .send_notify(
                &dialog_id,
                "refer".to_string(),  // Event type
                Some(body),
                Some("active".to_string()),  // Subscription state
            )
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))?;

        info!("Sent NOTIFY with status '{}' for session {}", status, transferor_session_id);
        Ok(())
    }

    /// Cleanup dialog only (keep media session alive)
    pub async fn cleanup_dialog_only(&self, dialog_id: &DialogId) -> Result<()> {
        let rvoip_dialog_id: rvoip_dialog_core::DialogId = dialog_id.into();

        // Remove from mappings
        if let Some(session_id) = self.dialog_to_session.remove(&rvoip_dialog_id) {
            self.session_to_dialog.remove(&session_id.1);
            info!("Cleaned up dialog {} (kept media session)", dialog_id);
        }

        // Note: We don't send BYE here - that was already done
        // This is just cleanup of our internal mappings

        Ok(())
    }
}
```

---

## Phase 4: Testing and Validation (High Priority) 🔴

### 4.1 Unit Tests

#### File: `crates/session-core-v2/tests/transfer_state_tests.rs` (NEW)

```rust
//! Unit tests for transfer state transitions

use rvoip_session_core_v2::*;

#[tokio::test]
async fn test_transfer_role_marking() {
    // Test that MarkAsTransferor action works
    let mut session = SessionState::new(SessionId::new(), Role::UAC);
    assert!(!session.is_transferor);

    // Execute MarkAsTransferor action
    session.is_transferor = true;
    assert!(session.is_transferor);
}

#[tokio::test]
async fn test_transfer_guards() {
    let mut session = SessionState::new(SessionId::new(), Role::UAC);

    // Test IsTransferee guard
    assert!(!check_guard(&Guard::IsTransferee, &session).await);
    session.is_transferee = true;
    assert!(check_guard(&Guard::IsTransferee, &session).await);

    // Test HasActiveTransferCall guard
    assert!(!check_guard(&Guard::HasActiveTransferCall, &session).await);
    session.transfer_call_session_id = Some(SessionId::new());
    assert!(check_guard(&Guard::HasActiveTransferCall, &session).await);
}

#[tokio::test]
async fn test_transferee_receives_bye_during_transfer() {
    // Test that transferee stays active when transferor sends BYE
    // ... setup state machine with transfer context ...
    // ... process DialogBYE event ...
    // ... assert session is still Active, not Terminating ...
}
```

### 4.2 Integration Test Updates

#### File: `crates/session-core-v2/examples/blind_transfer/run_blind_transfer.sh`

**Changes Required:**

```bash
# Add validation after test runs

echo ""
echo "======================================"
echo "🔍 Validating Transfer Flow"
echo "======================================"

# Check ACK was sent
if grep -q "SendACK" "$ALICE_LOG"; then
    echo "✅ Alice sent ACK to Charlie"
else
    echo "❌ Alice did NOT send ACK to Charlie"
    FAILED=1
fi

# Check BYE responsibility
if grep -q "Bob.*send.*BYE.*Alice" "$BOB_LOG"; then
    echo "✅ Bob sent BYE to Alice (correct per RFC)"
elif grep -q "Alice.*send.*BYE.*Bob" "$ALICE_LOG"; then
    echo "❌ Alice sent BYE to Bob (wrong - should be Bob to Alice)"
    FAILED=1
else
    echo "⚠️  No BYE found in logs"
    FAILED=1
fi

# Check Charlie sent BYE
if grep -q "send.*BYE" "$CHARLIE_LOG"; then
    echo "✅ Charlie sent BYE to Alice"
else
    echo "❌ Charlie did NOT send BYE"
    FAILED=1
fi

# Check state transitions
if grep -q "State transition.*Answering.*Active" "$CHARLIE_LOG"; then
    echo "✅ Charlie reached Active state"
else
    echo "⚠️  Charlie never reached Active state (stuck in Answering?)"
fi

# Check Alice didn't time out
if [ "$ALICE_EXIT" -eq 124 ]; then
    echo "❌ Alice timed out (should exit cleanly)"
    FAILED=1
else
    echo "✅ Alice exited cleanly"
fi

if [ "$FAILED" -eq 1 ]; then
    echo ""
    echo "❌ Transfer validation FAILED"
    exit 1
else
    echo ""
    echo "✅ All transfer validations PASSED"
fi
```

### 4.3 SIP Trace Validation

```bash
# Capture SIP messages for RFC compliance checking
tcpdump -i lo0 -w /tmp/blind_transfer_$(date +%Y%m%d_%H%M%S).pcap \
    'port 5060 or port 5061 or port 5062' &
TCPDUMP_PID=$!

# Run test
./run_blind_transfer.sh

# Stop capture
kill $TCPDUMP_PID

# Analyze with tshark (if available)
tshark -r /tmp/blind_transfer_*.pcap -Y 'sip' -T fields \
    -e frame.time -e ip.src -e ip.dst -e sip.Method -e sip.Status-Code
```

Expected SIP flow:
```
1. Alice → Bob: INVITE
2. Bob → Alice: 200 OK
3. Alice → Bob: ACK
4. Bob → Alice: REFER
5. Alice → Bob: 202 Accepted
6. Alice → Bob: NOTIFY (100 Trying)
7. Bob → Alice: BYE  ← MUST BE THIS DIRECTION
8. Alice → Bob: 200 OK
9. Alice → Charlie: INVITE
10. Charlie → Alice: 200 OK
11. Alice → Charlie: ACK  ← MUST BE PRESENT
12. Charlie → Alice: BYE
13. Alice → Charlie: 200 OK
```

---

## Implementation Order

### Week 1: Critical Fixes
1. ✅ Day 1-2: Fix ACK sending (§1.1)
2. ✅ Day 2-3: Add Answering transitions (§1.2)
3. ✅ Day 3-4: Fix BYE responsibility (§1.3)
4. ✅ Day 4-5: Test and validate Phase 1

### Week 2: Transfer Awareness
5. ✅ Day 6-7: Add transfer role tracking (§2.1)
6. ✅ Day 7-8: Add transfer guards (§2.2)
7. ✅ Day 8-9: Add transfer actions (§2.3)
8. ✅ Day 9-10: Add state transitions (§2.4)

### Week 3: Refinement
9. ✅ Day 11-12: Dialog adapter enhancements (§3.1)
10. ✅ Day 12-13: Write unit tests (§4.1)
11. ✅ Day 13-14: Integration testing (§4.2)
12. ✅ Day 14-15: SIP trace validation (§4.3)

---

## Success Criteria

### Must Have ✅
- [ ] Alice sends ACK to Charlie after 200 OK
- [ ] Charlie can hang up from any state
- [ ] Bob sends BYE to Alice (not vice versa)
- [ ] Alice doesn't time out (exits cleanly)
- [ ] Test completes in < 20 seconds

### Should Have ✅
- [ ] Transfer-aware state transitions work
- [ ] NOTIFY messages sent correctly
- [ ] SIP trace shows RFC-compliant flow
- [ ] No spurious errors in logs

### Nice to Have ⭐
- [ ] Graceful failure handling if Charlie rejects
- [ ] Progress indicators during transfer
- [ ] Detailed transfer state tracking

---

## Rollback Plan

If issues arise:

1. **Phase 1 Issues:** Each fix is independent - can revert individual changes
2. **Phase 2 Issues:** New guards/actions won't break existing non-transfer calls
3. **Phase 3 Issues:** Dialog adapter additions are additive only

**Safe Rollback Points:**
- After §1.1: Basic calls still work
- After §1.2: Answering state functional
- After §1.3: Non-transfer calls unaffected

---

## Files Modified Summary

### dialog-core Changes (2 files)
1. `crates/dialog-core/src/events/session_coordination.rs` - Add AckSent event variant
2. `crates/dialog-core/src/protocol/response_handler.rs` - Publish AckSent after automatic ACK

### session-core-v2 Core Logic (9 files)
3. `src/session_store/state.rs` - Add transfer role fields
4. `src/state_table/types.rs` - Add guards and actions
5. `src/state_machine/guards.rs` - Implement guard checks
6. `src/state_machine/actions.rs` - Make SendACK no-op, implement new actions
7. `src/state_table/yaml_loader.rs` - Parse new guards/actions
8. `src/adapters/dialog_adapter.rs` - Add REFER/NOTIFY methods, get_session_for_dialog()
9. `src/adapters/session_event_handler.rs` - Handle AckSent event
10. `src/transfer/coordinator.rs` - Remove wrong BYE logic
11. `state_tables/default.yaml` - Add transitions

### Examples (2 files)
12. `examples/blind_transfer/peer2_transferor.rs` - Bob hangs up
13. `examples/blind_transfer/run_blind_transfer.sh` - Add validation

### Tests (2 files - NEW)
14. `tests/transfer_state_tests.rs` - Unit tests
15. `tests/transfer_integration_tests.rs` - Integration tests

### Documentation (0 files - already exists)
- BYE_MESSAGE_ANALYSIS.md
- TRANSFER_RFC_ANALYSIS.md
- This file

**Total: 15 files modified (2 in dialog-core, 9 in session-core-v2, 2 examples, 2 new tests)**

---

## Notes for Maintainers

### Design Decisions

**Why metadata-based conditions instead of new states?**
- A transferee is simultaneously in two call contexts
- Generic "Active" state is correct for both calls
- Conditional transitions handle context-specific behavior
- Avoids state explosion (3x states for every scenario)

**Why keep dialog alive after BYE?**
- RFC 5589 requires NOTIFY messages after dialog termination
- NOTIFY uses event subscription, not dialog
- Transferee needs to track transfer progress
- Clean separation of dialog lifecycle vs transfer lifecycle

**Why not use B2BUA pattern?**
- Session-core-v2 is endpoint-focused, not B2BUA
- Transfer is an endpoint capability per RFC
- B2BUA would require different architecture
- Current approach aligns with RFC 3515/5589

### Future Enhancements

**Attended Transfer:**
- Requires consultation call support
- Replaces header generation
- More complex state coordination

**Transfer Recovery:**
- Handle 4xx/5xx responses to INVITE
- Allow transferee to reconnect to transferor
- Notify transferor of failure

**Transfer Progress:**
- Richer NOTIFY messages
- Real-time status updates
- Progress bars in UI

---

## References

- [RFC 3515 - SIP REFER Method](https://datatracker.ietf.org/doc/html/rfc3515)
- [RFC 5589 - SIP Call Control - Transfer](https://datatracker.ietf.org/doc/html/rfc5589)
- [BYE_MESSAGE_ANALYSIS.md](../examples/blind_transfer/BYE_MESSAGE_ANALYSIS.md)
- [TRANSFER_RFC_ANALYSIS.md](./TRANSFER_RFC_ANALYSIS.md)
