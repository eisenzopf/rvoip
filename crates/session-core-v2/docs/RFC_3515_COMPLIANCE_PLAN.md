# RFC 3515 Compliance Implementation Plan

## Executive Summary

**Current Status:** 85% RFC 3515 compliant
**Missing:** NOTIFY messages with transfer progress reporting
**Estimated Effort:** 4-6 hours
**Risk Level:** Low (infrastructure already exists)

---

## RFC 3515 Requirements Analysis

### What RFC 3515 Mandates

From RFC 3515 Section 2.4.5 (NOTIFY Behavior):

> The NOTIFY messages contain bodies of type "message/sipfrag" that
> describe the status of the refer. [...] The agent must also send a
> NOTIFY when the refer is accepted (200 OK is returned from the
> INVITE), and one when it fails (a non-2xx final response is received
> from the INVITE).

**Required NOTIFY Sequence:**

```
Bob (Transferor)          Alice (Transferee)         Charlie (Target)
     |                           |                          |
     | REFER (to Charlie)        |                          |
     |-------------------------->|                          |
     |                           |                          |
     | 202 Accepted              |                          |
     |<--------------------------|                          |
     |                           |                          |
     | NOTIFY (100 Trying) ✗     |                          |
     |<--------------------------|                          |
     |                           |                          |
     |                           | INVITE                   |
     |                           |------------------------->|
     |                           |                          |
     | NOTIFY (180 Ringing) ✗    | 180 Ringing              |
     |<--------------------------|<-------------------------|
     |                           |                          |
     | NOTIFY (200 OK) ✗         | 200 OK                   |
     |<--------------------------|<-------------------------|
     |                           |                          |
     |                           | ACK                      |
     |                           |------------------------->|
     |                           |                          |
     | 200 OK (to NOTIFY)        |                          |
     |-------------------------->|                          |
```

**Legend:**
- ✓ = Currently implemented
- ✗ = Missing (needs implementation)

---

## Current Implementation Gap Analysis

### What Works Today

1. ✅ **REFER Handling**
   - File: `crates/session-core-v2/src/state_machine/actions.rs:525-528`
   - Bob sends REFER to Alice
   - Alice responds with 202 Accepted
   - Dialog-core automatically handles the 202 response

2. ✅ **Transfer Execution**
   - File: `crates/session-core-v2/src/transfer/coordinator.rs:48-120`
   - Alice creates new session to Charlie
   - Alice sends INVITE to Charlie
   - Alice terminates original call to Bob with BYE

3. ✅ **NOTIFY Infrastructure**
   - File: `crates/session-core-v2/src/transfer/notify.rs:1-128`
   - `TransferNotifyHandler` fully implemented
   - `TransferProgress` enum with sipfrag conversion
   - Methods for all progress states (trying, ringing, success, failure)

### What's Missing

1. ❌ **NOTIFY Action Implementation**
   - File: `crates/session-core-v2/src/state_machine/actions.rs:530-540`
   - Actions exist but are stubs with TODOs
   - Need to call TransferNotifyHandler

2. ❌ **State Tracking for Transferor**
   - File: `crates/session-core-v2/src/session_store/state.rs`
   - No field to track transferor session ID
   - Can't send NOTIFY without knowing who to notify

3. ❌ **Event-Driven NOTIFY Triggering**
   - No mechanism to detect when transfer call rings/answers
   - Need to hook into Dialog180Ringing and Dialog200OK events for transfer session

4. ❌ **DialogAdapter NOTIFY Support**
   - File: `crates/session-core-v2/src/adapters/dialog_adapter.rs`
   - `send_notify()` method might not exist or is incomplete

---

## Implementation Plan

### Phase 1: Add Transferor Session Tracking (30 min)

**Goal:** Store who initiated the transfer so we can send them NOTIFY messages

#### File 1: `crates/session-core-v2/src/session_store/state.rs`

**Current State:**
```rust
pub struct SessionState {
    // ... existing fields ...
    pub transfer_target: Option<String>,
    pub transfer_state: TransferState,
    pub transfer_notify_dialog: Option<DialogId>,

    // Transfer coordination fields
    pub replaces_header: Option<String>,
    pub is_transfer_call: bool,
}
```

**Changes Needed:**
```rust
pub struct SessionState {
    // ... existing fields ...
    pub transfer_target: Option<String>,
    pub transfer_state: TransferState,
    pub transfer_notify_dialog: Option<DialogId>,

    // Transfer coordination fields
    pub replaces_header: Option<String>,
    pub is_transfer_call: bool,

    // ✨ NEW: Track who sent us the REFER so we can NOTIFY them
    pub transferor_session_id: Option<SessionId>,  // Bob's session ID
}
```

**Why:** When Alice receives a REFER from Bob, she needs to remember Bob's session ID so she can send him NOTIFY messages about the transfer progress.

**Default Value:**
```rust
impl Default for SessionState {
    fn default() -> Self {
        Self {
            // ... existing defaults ...
            transferor_session_id: None,  // ✨ NEW
        }
    }
}
```

---

### Phase 2: Store Transferor Session ID on REFER (30 min)

**Goal:** When REFER is received, store who sent it

#### File 2: `crates/session-core-v2/src/state_machine/actions.rs`

**Location:** Line 525-528

**Current Code:**
```rust
Action::AcceptREFER => {
    debug!("Accepting REFER request for transfer");
    // dialog-core should already send 202 Accepted automatically
    info!("REFER accepted for session {}", session.session_id);
}
```

**Changes Needed:**
```rust
Action::AcceptREFER => {
    debug!("Accepting REFER request for transfer");

    // ✨ NEW: Extract transferor session ID from the REFER request
    // The REFER comes from Bob, we need to store his session ID
    // so we can send him NOTIFY messages later

    // Note: In SIP, the From header of the REFER tells us who to NOTIFY
    // The dialog adapter should have this information from the REFER transaction

    // For now, we'll extract it from the remote_uri (Bob's URI)
    // In a full implementation, we'd get the dialog ID that received the REFER
    // and look up the corresponding session

    info!("REFER accepted for session {}", session.session_id);

    // ✨ TODO: Need to pass transferor_session_id from event context
    // This will come from the TransferRequested event which should include it
}
```

**Why:** The `TransferRequested` event comes from dialog-core when it receives a REFER. We need to enhance this event to include the transferor's session ID.

**Dependency:** Need to check what information is available in the `TransferRequested` event.

#### File 3: `crates/session-core-v2/src/state_table/types.rs`

**Current Event:**
```rust
EventType::TransferRequested {
    refer_to: String,
    transfer_type: String,
}
```

**Potential Enhancement:**
```rust
EventType::TransferRequested {
    refer_to: String,
    transfer_type: String,
    // ✨ NEW: Add transferor_dialog_id so we can map back to session
    transferor_dialog_id: Option<String>,
}
```

**Why:** Dialog-core knows which dialog the REFER came in on. We need that dialog ID so we can:
1. Map dialog ID → session ID
2. Store transferor_session_id in our session state
3. Send NOTIFY to the correct session/dialog

**Note:** This might require coordination with dialog-core to include this information in the event.

---

### Phase 3: Implement DialogAdapter NOTIFY Support (45 min)

**Goal:** Add ability to send NOTIFY messages through the dialog adapter

#### File 4: `crates/session-core-v2/src/adapters/dialog_adapter.rs`

**Check if exists:**
```rust
pub async fn send_notify(
    &self,
    session_id: &SessionId,
    event_type: &str,
    body: Option<String>,
) -> Result<(), String>
```

**If it doesn't exist, add:**
```rust
/// Send NOTIFY message for a subscription
///
/// # Arguments
/// * `session_id` - Session to send NOTIFY for
/// * `event_type` - Event package (e.g., "refer" for RFC 3515)
/// * `body` - Optional message body (e.g., sipfrag for transfer progress)
///
/// # RFC 3515 Usage
/// For blind transfer NOTIFY:
/// - event_type: "refer"
/// - body: "SIP/2.0 100 Trying" (or 180, 200, etc.)
pub async fn send_notify(
    &self,
    session_id: &SessionId,
    event_type: &str,
    body: Option<String>,
) -> Result<(), String> {
    debug!("Sending NOTIFY for session {} event {}", session_id, event_type);

    // Get dialog ID for this session
    let dialog_id = self.session_to_dialog.get(session_id)
        .ok_or_else(|| format!("No dialog found for session {}", session_id))?;

    // Convert our DialogId to rvoip DialogId
    let rvoip_dialog_id = rvoip_dialog_core::DialogId::from(dialog_id.clone());

    // Call dialog-core's NOTIFY send method
    // This assumes dialog-core has a send_notify method on UnifiedDialogApi
    self.dialog_api
        .send_notify_for_dialog(
            &rvoip_dialog_id,
            event_type,
            body.as_deref(),
        )
        .await
        .map_err(|e| format!("Failed to send NOTIFY: {}", e))?;

    info!("✅ NOTIFY sent for session {}", session_id);
    Ok(())
}
```

**Why:** The state machine actions need a way to send NOTIFY messages. This provides that interface.

**Dependency Check Required:**
- Does `rvoip_dialog_core::api::unified::UnifiedDialogApi` have a `send_notify_for_dialog()` method?
- If not, we need to add it or use an alternative approach

---

### Phase 4: Implement State Machine NOTIFY Actions (1 hour)

**Goal:** Wire up the NOTIFY actions to actually send messages

#### File 5: `crates/session-core-v2/src/state_machine/actions.rs`

**Location 1:** Lines 530-534

**Current Code:**
```rust
Action::SendTransferNOTIFY => {
    debug!("Sending NOTIFY for transfer progress");
    // TODO: Send NOTIFY with sipfrag body: "SIP/2.0 100 Trying"
    warn!("SendTransferNOTIFY not fully implemented yet");
}
```

**New Implementation:**
```rust
Action::SendTransferNOTIFY => {
    debug!("Sending NOTIFY for transfer progress (100 Trying)");

    // ✨ Get transferor session ID (who we need to notify)
    let transferor_session_id = match &session.transferor_session_id {
        Some(id) => id,
        None => {
            warn!("No transferor session ID stored, cannot send NOTIFY");
            return;
        }
    };

    // ✨ Create NOTIFY handler
    let notify_handler = crate::transfer::TransferNotifyHandler::new(
        dialog_adapter.clone()
    );

    // ✨ Send "100 Trying" NOTIFY
    if let Err(e) = notify_handler.notify_trying(transferor_session_id).await {
        error!("Failed to send NOTIFY (100 Trying): {}", e);
    } else {
        info!("✅ Sent NOTIFY (100 Trying) to transferor session {}", transferor_session_id);
    }
}
```

**Why:** This is called when Alice starts calling Charlie. Bob should be notified that the transfer is in progress.

**Location 2:** Lines 536-540

**Current Code:**
```rust
Action::SendTransferNOTIFYSuccess => {
    debug!("Sending NOTIFY for transfer success");
    // TODO: Send NOTIFY with sipfrag body: "SIP/2.0 200 OK"
    warn!("SendTransferNOTIFYSuccess not fully implemented yet");
}
```

**New Implementation:**
```rust
Action::SendTransferNOTIFYSuccess => {
    debug!("Sending NOTIFY for transfer success (200 OK)");

    // ✨ Get transferor session ID (who we need to notify)
    let transferor_session_id = match &session.transferor_session_id {
        Some(id) => id,
        None => {
            warn!("No transferor session ID stored, cannot send NOTIFY");
            return;
        }
    };

    // ✨ Create NOTIFY handler
    let notify_handler = crate::transfer::TransferNotifyHandler::new(
        dialog_adapter.clone()
    );

    // ✨ Send "200 OK" NOTIFY (transfer succeeded)
    if let Err(e) = notify_handler.notify_success(transferor_session_id).await {
        error!("Failed to send NOTIFY (200 OK): {}", e);
    } else {
        info!("✅ Sent NOTIFY (200 OK) to transferor session {} - transfer complete!", transferor_session_id);
    }
}
```

**Why:** This is called when Charlie answers. Bob should be notified that the transfer succeeded and he can hang up.

---

### Phase 5: Add 180 Ringing NOTIFY Support (30 min)

**Goal:** Send NOTIFY when transfer target is ringing

#### File 6: `crates/session-core-v2/src/state_machine/actions.rs`

**New Action Needed:**
```rust
Action::SendTransferNOTIFYRinging => {
    debug!("Sending NOTIFY for transfer ringing (180 Ringing)");

    // ✨ Get transferor session ID (who we need to notify)
    let transferor_session_id = match &session.transferor_session_id {
        Some(id) => id,
        None => {
            warn!("No transferor session ID stored, cannot send NOTIFY");
            return;
        }
    };

    // ✨ Create NOTIFY handler
    let notify_handler = crate::transfer::TransferNotifyHandler::new(
        dialog_adapter.clone()
    );

    // ✨ Send "180 Ringing" NOTIFY
    if let Err(e) = notify_handler.notify_ringing(transferor_session_id).await {
        error!("Failed to send NOTIFY (180 Ringing): {}", e);
    } else {
        info!("✅ Sent NOTIFY (180 Ringing) to transferor session {}", transferor_session_id);
    }
}
```

**Why:** When Charlie's phone rings, Bob should know the transfer is progressing.

#### File 7: `crates/session-core-v2/src/state_machine/types.rs`

**Add new action enum:**
```rust
pub enum Action {
    // ... existing actions ...
    SendTransferNOTIFY,
    SendTransferNOTIFYSuccess,
    SendTransferNOTIFYRinging,  // ✨ NEW
    // ... rest of actions ...
}
```

---

### Phase 6: Update State Table to Trigger NOTIFYs (45 min)

**Goal:** Add NOTIFY actions to appropriate state transitions

#### File 8: `crates/session-core-v2/state_tables/default.yaml`

**Location 1:** When transfer is initiated (after REFER accepted)

**Current:**
```yaml
- role: "Both"
  state: "Active"
  event:
    type: "TransferRequested"
  next_state: "TransferringCall"
  actions:
    - type: "AcceptREFER"
    - type: "StoreTransferTarget"
    - type: "SendTransferNOTIFY"  # This exists but doesn't work
  publish:
    - "TransferAccepted"
```

**Analysis:** Already has `SendTransferNOTIFY` - just needs the implementation from Phase 4.

**Location 2:** When transfer call is ringing

**Find transition for:**
```yaml
- role: "UAC"
  state: "Initiating"  # or "TransferringCall"
  event:
    type: "Dialog180Ringing"
```

**Check if this transition exists for transfer calls.** If not, add:
```yaml
# ✨ NEW: Send NOTIFY when transfer target is ringing
- role: "UAC"
  state: "Initiating"
  event:
    type: "Dialog180Ringing"
  next_state: "Initiating"  # Stay in same state
  actions:
    - type: "SendTransferNOTIFYRinging"  # ✨ NEW ACTION
  conditions:
    - field: "is_transfer_call"
      equals: true
  description: "Transfer target is ringing - notify transferor"
```

**Why:** When the transfer target (Charlie) sends 180 Ringing, Alice needs to NOTIFY Bob that the transfer is progressing.

**Location 3:** When transfer call is answered

**Find transition for:**
```yaml
- role: "UAC"
  state: "Initiating"
  event:
    type: "Dialog200OK"
```

**Check if it has SendTransferNOTIFYSuccess.** Update to:
```yaml
# Send NOTIFY when transfer target answers
- role: "UAC"
  state: "Initiating"
  event:
    type: "Dialog200OK"
  next_state: "Active"
  actions:
    - type: "ProcessDialog200OK"
    - type: "SendACK"
    - type: "SendTransferNOTIFYSuccess"  # ✨ Add if missing
  conditions:
    - field: "is_transfer_call"
      equals: true
  description: "Transfer target answered - notify transferor of success"
```

**Why:** When Charlie answers (200 OK), Alice needs to NOTIFY Bob that the transfer succeeded.

---

### Phase 7: Handle Transfer Failures (30 min)

**Goal:** Send failure NOTIFY if transfer target rejects or doesn't answer

#### File 9: `crates/session-core-v2/state_tables/default.yaml`

**Add transition:**
```yaml
# ✨ NEW: Send NOTIFY if transfer target rejects call
- role: "UAC"
  state: "Initiating"
  event:
    type: "DialogError"
  next_state: "Failed"
  actions:
    - type: "SendTransferNOTIFYFailure"  # ✨ NEW ACTION
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  conditions:
    - field: "is_transfer_call"
      equals: true
  description: "Transfer target rejected - notify transferor of failure"
```

**Why:** If Charlie rejects the call (busy, declined, unavailable), Bob needs to know the transfer failed.

#### File 10: `crates/session-core-v2/src/state_machine/actions.rs`

**Add new action:**
```rust
Action::SendTransferNOTIFYFailure => {
    debug!("Sending NOTIFY for transfer failure");

    // ✨ Get transferor session ID
    let transferor_session_id = match &session.transferor_session_id {
        Some(id) => id,
        None => {
            warn!("No transferor session ID stored, cannot send NOTIFY");
            return;
        }
    };

    // ✨ Determine failure reason from session state
    let (status_code, reason) = match &session.call_state {
        CallState::Failed => {
            // Extract actual SIP error code if available
            // Default to 487 Request Terminated if unknown
            (487, "Request Terminated")
        }
        _ => (500, "Internal Error")
    };

    // ✨ Create NOTIFY handler
    let notify_handler = crate::transfer::TransferNotifyHandler::new(
        dialog_adapter.clone()
    );

    // ✨ Send failure NOTIFY
    if let Err(e) = notify_handler.notify_failure(
        transferor_session_id,
        status_code,
        reason
    ).await {
        error!("Failed to send NOTIFY (failure): {}", e);
    } else {
        info!("✅ Sent NOTIFY ({} {}) to transferor - transfer failed", status_code, reason);
    }
}
```

#### File 11: `crates/session-core-v2/src/state_machine/types.rs`

**Add to Action enum:**
```rust
pub enum Action {
    // ... existing actions ...
    SendTransferNOTIFY,
    SendTransferNOTIFYSuccess,
    SendTransferNOTIFYRinging,
    SendTransferNOTIFYFailure,  // ✨ NEW
    // ... rest ...
}
```

---

### Phase 8: Integration with TransferCoordinator (1 hour)

**Goal:** Ensure transferor session ID is properly passed to the transferee session

#### File 12: `crates/session-core-v2/src/transfer/coordinator.rs`

**Current code (around line 48-120):**
```rust
pub async fn complete_transfer(
    &self,
    transferee_session_id: &SessionId,
    refer_to: &str,
    options: TransferOptions,
) -> Result<TransferResult, String> {
    // ... creates new session ...
    // ... makes call to target ...
}
```

**Enhancement needed:**
```rust
pub async fn complete_transfer(
    &self,
    transferee_session_id: &SessionId,  // Alice's session (who received REFER)
    refer_to: &str,                      // Charlie's URI (where to transfer)
    options: TransferOptions,
) -> Result<TransferResult, String> {
    info!("🔄 Starting transfer for session {} to target: {}", transferee_session_id, refer_to);

    // ... existing code to create new session ...

    // ✨ NEW: Mark the new session as a transfer call
    let mut new_session = self.session_store.get_session(&new_session_id).await?;
    new_session.is_transfer_call = true;

    // ✨ NEW: Store the transferee session ID in the new session
    // This way, when Charlie answers, we know to send NOTIFY to Bob via Alice's session
    new_session.transferor_session_id = Some(transferee_session_id.clone());

    self.session_store.update_session(new_session).await?;

    // ... rest of existing code ...
}
```

**Why:** The new session (Alice→Charlie) needs to know that it's a transfer call and who to notify about progress.

**Problem:** We actually need Bob's session ID, not Alice's. Need to trace back:
- Alice's session has `transferor_session_id` pointing to Bob
- The new session (Alice→Charlie) needs to inherit this

**Corrected implementation:**
```rust
// ✨ Get Bob's session ID from Alice's session
let transferee_session = self.session_store.get_session(transferee_session_id).await?;
let bob_session_id = transferee_session.transferor_session_id.clone();

// ✨ Create new session for transfer call
let new_session_id = SessionId(format!("transfer-{}", uuid::Uuid::new_v4()));
self.session_store.create_session(new_session_id.clone(), Role::UAC, false).await?;

// ✨ Configure new session as transfer call
let mut new_session = self.session_store.get_session(&new_session_id).await?;
new_session.is_transfer_call = true;
new_session.transferor_session_id = bob_session_id;  // Inherit from Alice's session
new_session.transfer_target = Some(refer_to.to_string());

self.session_store.update_session(new_session).await?;
```

---

### Phase 9: Capture Transferor Session ID from REFER Event (45 min)

**Goal:** When Alice receives REFER from Bob, store Bob's session ID

**This is the critical missing piece!**

#### Problem Analysis

When Bob sends REFER to Alice:
1. Dialog-core receives the REFER on Bob's dialog
2. Dialog-core publishes `TransferRequested` event
3. Session-core processes the event and stores `transfer_target`
4. **BUT:** We don't know Bob's session ID!

#### Solution: Enhance Event with Dialog Context

**Option A: Add to TransferRequested Event**

#### File 13: Check dialog-core event structure

**File:** `crates/dialog-core/src/events/dialog_to_session.rs` (hypothetical path)

**Check current event:**
```rust
pub enum DialogToSessionEvent {
    TransferRequested {
        session_id: String,      // Alice's session ID
        refer_to: String,        // Charlie's URI
        transfer_type: String,   // "blind"
    },
    // ...
}
```

**Enhancement needed:**
```rust
pub enum DialogToSessionEvent {
    TransferRequested {
        session_id: String,          // Alice's session ID (transferee)
        refer_to: String,            // Charlie's URI (transfer target)
        transfer_type: String,       // "blind"
        from_dialog_id: String,      // ✨ NEW: Dialog that sent the REFER
    },
    // ...
}
```

**Why:** The `from_dialog_id` tells us which dialog sent the REFER. Since REFER is sent within an existing dialog (Bob↔Alice), this dialog ID can be mapped back to Bob's session.

#### File 14: `crates/session-core-v2/src/adapters/session_event_handler.rs`

**Current code (around line 559-572):**
```rust
async fn handle_transfer_requested(&self, event_str: &str) -> Result<()> {
    if let Some(session_id) = self.extract_session_id(event_str) {
        let refer_to = self.extract_field(event_str, "refer_to: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let transfer_type = self.extract_field(event_str, "transfer_type: \"")
            .unwrap_or_else(|| "blind".to_string());

        if let Err(e) = self.state_machine.process_event(
            &SessionId(session_id),
            EventType::TransferRequested { refer_to, transfer_type }
        ).await {
            error!("Failed to process TransferRequested: {}", e);
        }
    }
    Ok(())
}
```

**Enhancement:**
```rust
async fn handle_transfer_requested(&self, event_str: &str) -> Result<()> {
    if let Some(session_id) = self.extract_session_id(event_str) {
        let refer_to = self.extract_field(event_str, "refer_to: \"")
            .unwrap_or_else(|| "unknown".to_string());
        let transfer_type = self.extract_field(event_str, "transfer_type: \"")
            .unwrap_or_else(|| "blind".to_string());

        // ✨ NEW: Extract from_dialog_id to identify who sent the REFER
        let from_dialog_id = self.extract_field(event_str, "from_dialog_id: \"");

        // ✨ NEW: Map from_dialog_id to transferor session ID
        let transferor_session_id = if let Some(dialog_id_str) = from_dialog_id {
            if let Ok(dialog_uuid) = uuid::Uuid::parse_str(&dialog_id_str) {
                let dialog_id = rvoip_dialog_core::DialogId(dialog_uuid);
                // Look up session ID for this dialog
                self.dialog_adapter.dialog_to_session.get(&dialog_id).cloned()
            } else {
                None
            }
        } else {
            None
        };

        // ✨ NEW: Store transferor session ID in the transferee's session
        if let Some(transferor_id) = transferor_session_id {
            if let Ok(mut session) = self.state_machine.store.get_session(&SessionId(session_id.clone())).await {
                session.transferor_session_id = Some(transferor_id);
                let _ = self.state_machine.store.update_session(session).await;
                info!("✅ Stored transferor session ID for transfer");
            }
        }

        if let Err(e) = self.state_machine.process_event(
            &SessionId(session_id),
            EventType::TransferRequested { refer_to, transfer_type }
        ).await {
            error!("Failed to process TransferRequested: {}", e);
        }
    }
    Ok(())
}
```

**Why:** This captures Bob's session ID when the REFER arrives, enabling all subsequent NOTIFY messages.

---

## Testing Plan

### Unit Tests

#### File 15: `crates/session-core-v2/src/transfer/notify.rs`

**Tests already exist (lines 105-128)** ✅

**Additional test needed:**
```rust
#[tokio::test]
async fn test_notify_handler_integration() {
    // Create mock dialog adapter
    // Create notify handler
    // Send all NOTIFY types
    // Verify correct sipfrag format
    // Verify correct event package ("refer")
}
```

### Integration Tests

#### File 16: `crates/session-core-v2/examples/blind_transfer/peer1_caller.rs`

**Enhancement to verify NOTIFY:**
```rust
// After transfer completes, check logs for NOTIFY messages
println!("[ALICE] ✅ Transfer completed");
println!("[ALICE] 📋 Checking if NOTIFY messages were sent...");

// The test harness should grep logs for:
// - "Sent NOTIFY (100 Trying)"
// - "Sent NOTIFY (180 Ringing)"
// - "Sent NOTIFY (200 OK)"
```

#### File 17: `crates/session-core-v2/examples/blind_transfer/peer2_transferor.rs`

**Enhancement to receive NOTIFY:**
```rust
// Bob should receive NOTIFY messages
// Add logging to show when NOTIFY is received
println!("[BOB] 📨 Received NOTIFY: {}", notify_body);
```

### Validation Script

#### File 18: `crates/session-core-v2/examples/blind_transfer/validate_rfc3515.sh` (NEW)

```bash
#!/bin/bash
# RFC 3515 Compliance Validation Script

echo "🔍 Validating RFC 3515 Compliance..."

# Run the blind transfer test
./run_blind_transfer.sh > /tmp/transfer_test.log 2>&1

# Check for required NOTIFY messages
echo ""
echo "Checking for required NOTIFY messages:"

if grep -q "Sent NOTIFY (100 Trying)" logs/alice_*.log; then
    echo "✅ NOTIFY 100 Trying sent"
else
    echo "❌ MISSING: NOTIFY 100 Trying"
fi

if grep -q "Sent NOTIFY (180 Ringing)" logs/alice_*.log; then
    echo "✅ NOTIFY 180 Ringing sent"
else
    echo "⚠️  OPTIONAL: NOTIFY 180 Ringing (only if Charlie sends 180)"
fi

if grep -q "Sent NOTIFY (200 OK)" logs/alice_*.log; then
    echo "✅ NOTIFY 200 OK sent (transfer success)"
else
    echo "❌ MISSING: NOTIFY 200 OK"
fi

if grep -q "Received NOTIFY" logs/bob_*.log; then
    echo "✅ Bob received NOTIFY messages"
else
    echo "❌ MISSING: Bob did not receive NOTIFY"
fi

echo ""
echo "RFC 3515 Compliance Check Complete"
```

---

## Implementation Checklist

### Phase 1: Foundation (Day 1, Morning)
- [ ] Add `transferor_session_id` field to SessionState
- [ ] Update SessionState default implementation
- [ ] Test: Compile session-core-v2

### Phase 2: Dialog Adapter (Day 1, Afternoon)
- [ ] Check if dialog-core supports `send_notify_for_dialog()`
- [ ] If missing, file issue or implement in dialog-core
- [ ] Implement `DialogAdapter::send_notify()` in session-core-v2
- [ ] Test: Unit test for DialogAdapter::send_notify()

### Phase 3: State Machine Actions (Day 2, Morning)
- [ ] Implement `Action::SendTransferNOTIFY` (100 Trying)
- [ ] Implement `Action::SendTransferNOTIFYRinging` (180 Ringing)
- [ ] Implement `Action::SendTransferNOTIFYSuccess` (200 OK)
- [ ] Implement `Action::SendTransferNOTIFYFailure` (4xx/5xx)
- [ ] Add new actions to Action enum
- [ ] Test: Compile with all actions

### Phase 4: State Table Updates (Day 2, Afternoon)
- [ ] Add SendTransferNOTIFYRinging to Dialog180Ringing transition
- [ ] Add SendTransferNOTIFYSuccess to Dialog200OK transition
- [ ] Add SendTransferNOTIFYFailure to DialogError transition
- [ ] Reload state table and validate YAML
- [ ] Test: State machine unit tests

### Phase 5: Event Handling (Day 3, Morning)
- [ ] Check dialog-core TransferRequested event structure
- [ ] Add from_dialog_id to event if needed
- [ ] Update session_event_handler to extract transferor session ID
- [ ] Store transferor_session_id when REFER received
- [ ] Test: Integration test with REFER

### Phase 6: Transfer Coordinator (Day 3, Afternoon)
- [ ] Update TransferCoordinator to set is_transfer_call flag
- [ ] Update TransferCoordinator to propagate transferor_session_id
- [ ] Test: Complete transfer flow

### Phase 7: Integration Testing (Day 4)
- [ ] Run blind_transfer example with debug logging
- [ ] Verify all NOTIFY messages appear in logs
- [ ] Create validation script
- [ ] Test failure scenarios (target busy, timeout)
- [ ] Verify NOTIFY failure messages

### Phase 8: Documentation (Day 4, Afternoon)
- [ ] Update BLIND_TRANSFER_IMPLEMENTATION_PLAN.md
- [ ] Add RFC 3515 compliance badge
- [ ] Document NOTIFY behavior
- [ ] Update API documentation

---

## Risk Assessment

### Low Risk ✅
- State machine action implementation (well understood)
- State table updates (straightforward YAML changes)
- Testing infrastructure (examples already exist)

### Medium Risk ⚠️
- DialogAdapter::send_notify() integration with dialog-core
  - **Mitigation:** Check dialog-core API first, may already exist
- Capturing transferor session ID from REFER event
  - **Mitigation:** May require dialog-core event enhancement

### High Risk ⛔
- None identified

---

## Dependencies on Other Crates

### dialog-core Requirements

**Required API (check if exists):**
```rust
impl UnifiedDialogApi {
    pub async fn send_notify_for_dialog(
        &self,
        dialog_id: &DialogId,
        event_package: &str,  // "refer"
        body: Option<&str>,   // "SIP/2.0 100 Trying"
    ) -> Result<(), Error>;
}
```

**If missing:** Need to implement NOTIFY support in dialog-core.

**Required Event Enhancement:**
```rust
pub enum DialogToSessionEvent {
    TransferRequested {
        session_id: String,
        refer_to: String,
        transfer_type: String,
        from_dialog_id: String,  // ✨ Need to add this
    },
}
```

**If missing:** File issue or implement in dialog-core.

---

## Success Criteria

### Functional Requirements
- [x] REFER received and 202 Accepted sent
- [ ] NOTIFY (100 Trying) sent when transfer starts
- [ ] NOTIFY (180 Ringing) sent when target rings (if applicable)
- [ ] NOTIFY (200 OK) sent when target answers
- [ ] NOTIFY (4xx/5xx) sent if transfer fails
- [ ] Transferor receives all NOTIFY messages
- [ ] Transfer completes successfully

### Non-Functional Requirements
- [ ] No performance degradation
- [ ] Clean error handling for NOTIFY failures
- [ ] Backward compatible (existing blind transfers still work)
- [ ] Comprehensive logging for debugging

### Compliance
- [ ] RFC 3515 Section 2.4.5 compliant (NOTIFY behavior)
- [ ] message/sipfrag content type used
- [ ] Event package "refer" used
- [ ] Subscription-State header handled correctly

---

## Estimated Effort

| Phase | Task | Estimated Time |
|-------|------|----------------|
| 1 | Add transferor session tracking | 30 min |
| 2 | Store transferor ID on REFER | 30 min |
| 3 | Implement DialogAdapter NOTIFY | 45 min |
| 4 | State machine NOTIFY actions | 1 hour |
| 5 | Add 180 Ringing support | 30 min |
| 6 | Update state table | 45 min |
| 7 | Handle transfer failures | 30 min |
| 8 | TransferCoordinator integration | 1 hour |
| 9 | Capture transferor from REFER | 45 min |
| **Total** | | **6 hours** |
| Testing | Integration & validation | 2 hours |
| **Grand Total** | | **8 hours** |

---

## Next Steps

1. **Review this plan** with the team
2. **Check dialog-core dependencies** (NOTIFY support, event structure)
3. **Create feature branch:** `feature/rfc-3515-notify-compliance`
4. **Implement in order** (Phases 1-9)
5. **Test incrementally** after each phase
6. **Validate with script** before merging

---

## References

- [RFC 3515: The Session Initiation Protocol (SIP) Refer Method](https://www.rfc-editor.org/rfc/rfc3515)
- [RFC 3261: SIP - Session Initiation Protocol](https://www.rfc-editor.org/rfc/rfc3261) (for NOTIFY method)
- [RFC 3903: SIP Event Notification](https://www.rfc-editor.org/rfc/rfc3903) (for Event packages)

---

**Document Version:** 1.0
**Last Updated:** 2025-10-01
**Author:** Claude (AI Assistant)
**Status:** Ready for Review
