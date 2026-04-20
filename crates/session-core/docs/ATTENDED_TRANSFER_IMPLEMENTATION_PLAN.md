# Attended Transfer Implementation Plan

## Overview
This document provides a detailed plan to implement fully functional attended transfer in session-core-v2, making the `start_attended_transfer()` and `complete_attended_transfer()` API methods work correctly.

## Current State Analysis

### What Works
- ✅ API methods exist in `unified.rs`
- ✅ State transitions defined in `default.yaml`
- ✅ `SendREFERWithReplaces` action implemented
- ✅ `send_refer_with_replaces()` in dialog adapter
- ✅ `ConsultationCall` state exists

### What's Broken
- ❌ Event name mismatch: API sends `StartAttendedTransfer`, YAML expects `StartConsultation`
- ❌ `HoldOriginalCall` action not implemented (treated as custom stub)
- ❌ `CreateConsultationDialog` action not implemented (treated as custom stub)
- ❌ No consultation session management (tracking two sessions)
- ❌ No transfer target storage mechanism
- ❌ Missing link between original call and consultation call

## Attended Transfer Flow

### Standard SIP Attended Transfer (RFC 5589)

```
Party A (Customer)    You (Transferor)    Party B (Transfer Target)    Party C (Final Target)
     |                      |                       |                          |
     |<------ INVITE ------>|                       |                          |
     |  (Active Call 1)     |                       |                          |
     |                      |                       |                          |
     |  (A gets put on hold)|                       |                          |
     |<---- re-INVITE ------|                       |                          |
     |  (a=sendonly)        |                       |                          |
     |                      |                       |                          |
     |                      |<------ INVITE ------>|                          |
     |                      |  (Consultation Call)  |                          |
     |                      |  (You talk to B)      |                          |
     |                      |                       |                          |
     |                      |------ REFER --------->|                          |
     |                      |  (Refer-To: C)        |                          |
     |                      |  (Replaces: Call-ID-A)|                          |
     |                      |                       |                          |
     |                      |<----- 202 Accepted ---|                          |
     |                      |                       |                          |
     |                      |<----- BYE ------------|                          |
     |                      |  (B hangs up with you)|                          |
     |                      |                       |                          |
     |                      |                       |<------ INVITE --------->|
     |                      |                       |  (B calls C with         |
     |                      |                       |   Replaces header)       |
     |<-------------------------------- INVITE with Replaces ----------------- |
     |  (C replaces your call with B's call)        |                          |
     |<------------------------------------------------- Connected ----------->|
     |  (A now talking to C, you're out of the picture)                        |
```

### Our Implementation Flow

```
State Flow:
Active (Call with A)
  → StartAttendedTransfer event
  → ConsultationCall state (A on hold, talking to B)
  → CompleteAttendedTransfer event
  → Terminated state (you drop out, A connects to B via REFER/Replaces)
```

## Implementation Tasks

### Phase 1: Fix Event Name Mismatch

**File:** `crates/session-core-v2/state_tables/default.yaml`

**Change:**
```yaml
# FROM:
  - role: "Both"
    state: "Active"
    event:
      type: "StartConsultation"

# TO:
  - role: "Both"
    state: "Active"
    event:
      type: "StartAttendedTransfer"
```

**Rationale:** Make YAML match the API event name for consistency.

---

### Phase 2: Add Session Fields for Transfer Tracking

**File:** `crates/session-core-v2/src/session_store/types.rs`

**Add fields to `Session` struct:**
```rust
pub struct Session {
    // ... existing fields ...

    /// Transfer target URI (for blind/attended transfer)
    pub transfer_target: Option<String>,

    /// Consultation call session ID (for attended transfer)
    pub consultation_session_id: Option<SessionId>,

    /// Original session ID (set in consultation call to link back)
    pub original_session_id: Option<SessionId>,

    /// Transfer state tracking
    pub transfer_state: TransferState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransferState {
    None,
    ConsultationInProgress,
    TransferInitiated,
    TransferCompleted,
}
```

**Why:** Need to track the relationship between original call and consultation call.

---

### Phase 3: Implement HoldOriginalCall Action

**File:** `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Add to `parse_action_by_name()`:**
```rust
fn parse_action_by_name(&self, name: &str) -> Result<Action> {
    match name {
        // ... existing actions ...

        "HoldOriginalCall" | "HoldCurrentCall" => Ok(Action::HoldCurrentCall),

        // ... rest of actions ...
    }
}
```

**File:** `crates/session-core-v2/src/state_machine/actions.rs`

**Add to `execute_action()`:**
```rust
Action::HoldCurrentCall => {
    debug!("Putting current call on hold for transfer");

    // Update media direction to sendonly (we can hear them, they hear hold music/silence)
    if let Some(media_id) = &session.media_session_id {
        let hold_direction = crate::types::MediaDirection::SendOnly;
        media_adapter.set_media_direction(media_id.clone(), hold_direction).await?;
    }

    // Send re-INVITE with sendonly SDP
    if let Some(hold_sdp) = media_adapter.create_hold_sdp().await.ok() {
        session.local_sdp = Some(hold_sdp.clone());
        dialog_adapter.send_reinvite_session(&session.session_id, hold_sdp).await?;
    }

    info!("Call {} put on hold", session.session_id);
}
```

**Why:** Must put original call on hold before starting consultation call.

---

### Phase 4: Implement CreateConsultationDialog Action

**File:** `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Add to `parse_action_by_name()`:**
```rust
"CreateConsultationDialog" | "CreateConsultationCall" => Ok(Action::CreateConsultationCall),
```

**File:** `crates/session-core-v2/src/state_machine/actions.rs`

**Update implementation:**
```rust
Action::CreateConsultationCall => {
    debug!("Creating consultation call for attended transfer");

    // Store the transfer target from the event
    // Note: The target should have been set when processing StartAttendedTransfer event
    if let Some(target) = &session.transfer_target {
        info!("Consultation call target: {}", target);
        // The actual consultation call creation happens in the state machine helpers
        // This action just marks that we're ready to create it
    } else {
        warn!("No transfer target set for consultation call");
    }
}
```

**File:** `crates/session-core-v2/src/state_machine/executor.rs`

**Update `process_event()` to capture transfer target:**
```rust
pub async fn process_event(
    &self,
    session_id: &SessionId,
    event: EventType,
) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
    // ... existing code ...

    // 1a. Store event-specific data in session state
    match &event {
        EventType::MakeCall { target } => {
            session.remote_uri = Some(target.clone());
        }
        EventType::IncomingCall { from, sdp } => {
            session.remote_uri = Some(from.clone());
            if let Some(sdp_data) = sdp {
                session.remote_sdp = Some(sdp_data.clone());
            }
        }
        // ADD THIS:
        EventType::StartAttendedTransfer { target } => {
            session.transfer_target = Some(target.clone());
            debug!("Set transfer target to: {}", target);
        }
        _ => {}
    }

    // ... rest of method ...
}
```

**Why:** Need to capture and store the transfer target from the event.

---

### Phase 5: Add Consultation Call Management to Helpers

**File:** `crates/session-core-v2/src/state_machine/helpers.rs`

**Add new helper method:**
```rust
/// Create consultation call for attended transfer
pub async fn create_consultation_call(
    &self,
    original_session_id: &SessionId,
    target: &str,
) -> Result<SessionId> {
    // Get original session to link them
    let mut original_session = self.state_machine.store.get_session(original_session_id).await?;

    // Create new session for consultation call
    let consultation_session_id = SessionId::new();

    // Determine local URI from original session
    let from = original_session.local_uri.clone()
        .unwrap_or_else(|| "sip:anonymous@localhost".to_string());

    // Create the consultation session
    self.create_session(
        consultation_session_id.clone(),
        from.clone(),
        target.to_string(),
        Role::UAC,
    ).await?;

    // Link consultation session back to original
    let mut consultation_session = self.state_machine.store.get_session(&consultation_session_id).await?;
    consultation_session.original_session_id = Some(original_session_id.clone());
    self.state_machine.store.update_session(consultation_session).await?;

    // Link original session to consultation
    original_session.consultation_session_id = Some(consultation_session_id.clone());
    original_session.transfer_state = TransferState::ConsultationInProgress;
    self.state_machine.store.update_session(original_session).await?;

    // Start the consultation call (send INVITE)
    self.state_machine.process_event(
        &consultation_session_id,
        EventType::MakeCall { target: target.to_string() },
    ).await?;

    info!("Created consultation call {} for transfer from {}", consultation_session_id, original_session_id);

    Ok(consultation_session_id)
}
```

**Why:** Need orchestration logic to create and link the two sessions.

---

### Phase 6: Update API to Create Consultation Call

**File:** `crates/session-core-v2/src/api/unified.rs`

**Update `start_attended_transfer()` to return consultation session ID:**
```rust
/// Start attended transfer - puts current call on hold and creates consultation call
pub async fn start_attended_transfer(&self, session_id: &SessionId, target: &str) -> Result<SessionId> {
    // First process the event to trigger hold and state change
    self.helpers.state_machine.process_event(
        session_id,
        EventType::StartAttendedTransfer { target: target.to_string() },
    ).await?;

    // Create the actual consultation call
    let consultation_id = self.helpers.create_consultation_call(session_id, target).await?;

    Ok(consultation_id)
}
```

**File:** `crates/session-core-v2/src/api/simple.rs`

**Update wrapper to match:**
```rust
/// Start attended transfer (returns consultation call ID)
pub async fn start_attended_transfer(&self, call_id: &CallId, target: &str) -> Result<CallId> {
    let session_id = SessionId(call_id.clone());
    let consultation_session_id = self.coordinator.start_attended_transfer(&session_id, target).await?;
    Ok(consultation_session_id.0)
}
```

**Why:** Application needs the consultation call ID to manage it (hang up, transfer, etc).

---

### Phase 7: Implement Complete Attended Transfer

**File:** `crates/session-core-v2/src/state_machine/executor.rs`

**Update to handle CompleteAttendedTransfer event:**
```rust
// In process_event(), add:
EventType::CompleteAttendedTransfer => {
    // Get consultation session info to build Replaces header
    if let Some(consultation_id) = &session.consultation_session_id {
        if let Ok(consultation_session) = self.store.get_session(consultation_id).await {
            // Extract dialog info needed for Replaces header
            session.transfer_target = consultation_session.remote_uri.clone();
            session.transfer_state = TransferState::TransferInitiated;
        }
    }
}
```

**File:** `crates/session-core-v2/src/adapters/dialog_adapter.rs`

**Update `send_refer_with_replaces()` to use consultation call info:**
```rust
pub async fn send_refer_with_replaces(&self, session_id: &SessionId, consultation_session_id: &SessionId) -> Result<()> {
    let dialog_id = self.session_to_dialog.get(session_id)
        .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
        .clone();

    // Get consultation dialog to build Replaces header
    let consultation_dialog_id = self.session_to_dialog.get(consultation_session_id)
        .ok_or_else(|| SessionError::SessionNotFound(consultation_session_id.0.clone()))?
        .clone();

    // Get consultation dialog info for Replaces header
    let dialog_info = self.dialog_api.get_dialog_info(&consultation_dialog_id).await
        .map_err(|e| SessionError::DialogError(format!("Failed to get dialog info: {}", e)))?;

    // Build Refer-To with Replaces header
    let refer_to = format!(
        "{}?Replaces={}%3Bto-tag%3D{}%3Bfrom-tag%3D{}",
        dialog_info.remote_uri,
        dialog_info.call_id,
        dialog_info.remote_tag,
        dialog_info.local_tag
    );

    // Send REFER with Replaces
    self.dialog_api
        .send_refer(&dialog_id, refer_to, Some("attended".to_string()))
        .await
        .map_err(|e| SessionError::DialogError(format!("Failed to send REFER with Replaces: {}", e)))?;

    tracing::info!("Sent REFER with Replaces for attended transfer from {} to {}",
                   session_id.0, consultation_session_id.0);
    Ok(())
}
```

**File:** `crates/session-core-v2/src/state_machine/actions.rs`

**Update `SendREFERWithReplaces`:**
```rust
Action::SendREFERWithReplaces => {
    debug!("Sending REFER with Replaces for attended transfer");
    if let Some(consultation_id) = &session.consultation_session_id {
        dialog_adapter.send_refer_with_replaces(&session.session_id, consultation_id).await?;
        session.transfer_state = TransferState::TransferCompleted;
    } else {
        error!("No consultation session ID for attended transfer");
    }
}
```

**Why:** Need proper SIP Replaces header to make attended transfer work correctly.

---

### Phase 8: Update complete_attended_transfer API

**File:** `crates/session-core-v2/src/api/unified.rs`

**Update signature to take consultation ID:**
```rust
/// Complete attended transfer - sends REFER with Replaces and terminates both calls
pub async fn complete_attended_transfer(
    &self,
    original_session_id: &SessionId,
    consultation_session_id: &SessionId
) -> Result<()> {
    // Update original session with consultation ID
    let mut session = self.helpers.state_machine.store.get_session(original_session_id).await?;
    session.consultation_session_id = Some(consultation_session_id.clone());
    self.helpers.state_machine.store.update_session(session).await?;

    // Process the complete event
    self.helpers.state_machine.process_event(
        original_session_id,
        EventType::CompleteAttendedTransfer,
    ).await?;

    Ok(())
}
```

**Alternative simpler API (if consultation ID is already stored):**
```rust
/// Complete attended transfer using stored consultation session
pub async fn complete_attended_transfer(&self, original_session_id: &SessionId) -> Result<()> {
    // Consultation ID should already be stored in the session
    self.helpers.state_machine.process_event(
        original_session_id,
        EventType::CompleteAttendedTransfer,
    ).await?;
    Ok(())
}
```

**Why:** Need to link the two sessions when completing the transfer.

---

### Phase 9: Add Consultation Call Cancellation

**File:** `crates/session-core-v2/state_tables/default.yaml`

**Add transition for canceling consultation:**
```yaml
  # Cancel consultation and return to original call
  - role: "Both"
    state: "ConsultationCall"
    event:
      type: "HangupCall"
    next_state: "Active"
    actions:
      - type: "TerminateConsultationCall"
      - type: "ResumeOriginalCall"
    publish:
      - "ConsultationCancelled"
    description: "Cancel consultation and resume original call"
```

**File:** `crates/session-core-v2/src/state_machine/actions.rs`

**Implement actions:**
```rust
Action::TerminateConsultationCall => {
    debug!("Terminating consultation call");
    if let Some(consultation_id) = &session.consultation_session_id {
        // Hang up the consultation call
        if let Ok(mut consultation_session) = session_store.get_session(consultation_id).await {
            // Send BYE if dialog exists
            if let Some(media_id) = &consultation_session.media_session_id {
                media_adapter.stop_media_session(media_id.clone()).await?;
            }
            dialog_adapter.send_bye(&consultation_id).await?;

            // Update state
            consultation_session.call_state = CallState::Terminated;
            session_store.update_session(consultation_session).await?;
        }

        // Clear consultation link
        session.consultation_session_id = None;
        session.transfer_state = TransferState::None;
    }
}

Action::RestoreMediaFlow => {
    debug!("Restoring media flow (unhold)");
    if let Some(media_id) = &session.media_session_id {
        let active_direction = crate::types::MediaDirection::SendRecv;
        media_adapter.set_media_direction(media_id.clone(), active_direction).await?;
    }

    // Send re-INVITE with sendrecv
    if let Some(active_sdp) = media_adapter.create_active_sdp().await.ok() {
        session.local_sdp = Some(active_sdp.clone());
        dialog_adapter.send_reinvite_session(&session.session_id, active_sdp).await?;
    }
}
```

**File:** `crates/session-core-v2/src/state_table/yaml_loader.rs`

**Add parsing:**
```rust
"ResumeOriginalCall" => Ok(Action::RestoreMediaFlow),
```

**Why:** User might decide not to complete transfer and return to original call.

---

### Phase 10: Add Cancel Attended Transfer API

**File:** `crates/session-core-v2/src/api/unified.rs`

**Add new method:**
```rust
/// Cancel attended transfer and return to original call
pub async fn cancel_attended_transfer(&self, original_session_id: &SessionId) -> Result<()> {
    // Terminate consultation and resume original
    self.helpers.state_machine.process_event(
        original_session_id,
        EventType::HangupCall,
    ).await?;
    Ok(())
}
```

**File:** `crates/session-core-v2/src/api/simple.rs`

**Add wrapper:**
```rust
/// Cancel attended transfer
pub async fn cancel_attended_transfer(&self, call_id: &CallId) -> Result<()> {
    self.coordinator.cancel_attended_transfer(&SessionId(call_id.clone())).await
}
```

**Why:** Provide escape hatch if user changes their mind.

---

## Testing Plan

### Unit Tests

**File:** `crates/session-core-v2/tests/attended_transfer_tests.rs`

```rust
#[tokio::test]
async fn test_attended_transfer_full_flow() {
    // 1. Create session A (active call)
    // 2. Start attended transfer to B
    //    - Verify A goes to OnHold
    //    - Verify consultation session created
    //    - Verify consultation call to B initiated
    // 3. Wait for consultation call to establish
    // 4. Complete attended transfer
    //    - Verify REFER with Replaces sent
    //    - Verify original session terminated
    //    - Verify consultation session terminated
}

#[tokio::test]
async fn test_attended_transfer_cancellation() {
    // 1. Create session A (active call)
    // 2. Start attended transfer to B
    // 3. Cancel transfer (hang up consultation)
    //    - Verify consultation call terminated
    //    - Verify A returns to Active
    //    - Verify A is unheld (sendrecv)
}

#[tokio::test]
async fn test_attended_transfer_consultation_rejects() {
    // 1. Create session A (active call)
    // 2. Start attended transfer to B
    // 3. B rejects consultation call (486 Busy)
    //    - Verify consultation terminated
    //    - Verify A returns to Active
    //    - Verify proper error event published
}
```

### Integration Tests

**File:** `crates/session-core-v2/examples/attended_transfer_example.rs`

```rust
// Full attended transfer scenario with real SIP dialogs
async fn main() -> Result<()> {
    // Setup three peers: Alice, Bob (transferor), Charlie
    // 1. Alice calls Bob
    // 2. Bob starts attended transfer to Charlie
    // 3. Bob talks to Charlie
    // 4. Bob completes transfer
    // 5. Alice now talking to Charlie, Bob drops out
}
```

---

## State Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    Attended Transfer Flow                    │
└─────────────────────────────────────────────────────────────┘

    [Active]  <-- Original call with Party A
       |
       | StartAttendedTransfer { target: B }
       | Actions:
       |   - HoldCurrentCall (send re-INVITE with sendonly)
       |   - CreateConsultationCall (initiate call to B)
       v
 [ConsultationCall]  <-- A on hold, talking to B
       |
       |--- HangupCall --> [Active]  (cancel, resume A)
       |    Actions:
       |      - TerminateConsultationCall
       |      - RestoreMediaFlow (unhold A)
       |
       | CompleteAttendedTransfer
       | Actions:
       |   - SendREFERWithReplaces (tell B to call A)
       |   - CleanupDialog (hang up with B)
       |   - CleanupMedia
       v
  [Terminated]  <-- You're out, A and B now connected
```

---

## API Usage Example

```rust
use rvoip_session_core_v2::api::UnifiedCoordinator;

async fn perform_attended_transfer(coordinator: &UnifiedCoordinator) -> Result<()> {
    // Assume we have an active call with customer
    let customer_session_id = SessionId("session-123".to_string());

    // Step 1: Start attended transfer to agent
    let agent_consultation_id = coordinator
        .start_attended_transfer(&customer_session_id, "sip:agent@company.com")
        .await?;

    println!("Customer on hold, now talking to agent");
    println!("Consultation call ID: {}", agent_consultation_id);

    // Talk to agent, explain the situation...
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Option A: Complete the transfer
    coordinator
        .complete_attended_transfer(&customer_session_id)
        .await?;
    println!("Transfer complete! Customer now connected to agent.");

    // Option B: Cancel the transfer (if needed)
    // coordinator.cancel_attended_transfer(&customer_session_id).await?;
    // println!("Transfer cancelled, back to customer");

    Ok(())
}
```

---

## Dependencies on Other Components

### dialog-core Requirements
- ✅ `send_refer()` with Replaces support - Already exists
- ❓ `get_dialog_info()` to extract Call-ID, tags for Replaces header - **May need to add**
- ✅ `send_reinvite()` for hold/resume - Already exists

### media-core Requirements
- ✅ `set_media_direction()` for hold/resume - Already exists
- ✅ `create_hold_sdp()` - May need to add if not present
- ✅ `create_active_sdp()` - Already exists
- ✅ `stop_media_session()` - Already exists

---

## Rollout Strategy

### Phase 1: Foundation (Days 1-2)
- Fix event name mismatch
- Add session fields
- Implement HoldCurrentCall action
- Basic state transitions working

### Phase 2: Consultation Call (Days 3-4)
- Implement CreateConsultationCall action
- Add consultation call management to helpers
- Link original and consultation sessions
- Update API to return consultation ID

### Phase 3: Complete Transfer (Days 5-6)
- Implement proper REFER with Replaces
- Extract dialog info for Replaces header
- Handle transfer completion flow
- Add cancellation support

### Phase 4: Testing & Polish (Days 7-8)
- Write unit tests
- Write integration tests
- Test with real SIP endpoints
- Documentation and examples

---

## Success Criteria

- [ ] `start_attended_transfer()` puts original call on hold
- [ ] `start_attended_transfer()` creates and returns consultation call ID
- [ ] User can talk on consultation call while original is on hold
- [ ] `complete_attended_transfer()` sends proper REFER with Replaces
- [ ] After transfer, original party connects to consultation party
- [ ] Transferor's calls both terminate after successful transfer
- [ ] `cancel_attended_transfer()` terminates consultation and resumes original
- [ ] All state transitions follow the documented flow
- [ ] Integration test passes with real SIP dialogs

---

## Notes

### SIP REFER with Replaces Format
```
REFER sip:bob@example.com SIP/2.0
Refer-To: <sip:charlie@example.com?Replaces=call-id-123%3Bto-tag%3Dtag1%3Bfrom-tag%3Dtag2>
```

The Replaces header tells the consultation party (B) to:
1. Call the transfer target (C) specified in Refer-To
2. Include a Replaces header in that INVITE
3. The Replaces header references the original call-id and tags
4. When C receives the INVITE with Replaces, they replace their call with A with this new call from B

### Alternative: No Consultation Call
Some implementations do "semi-attended" transfer:
- Put A on hold
- Send REFER immediately without actually calling B
- B's phone rings for A (INVITE with Replaces)

Our implementation does true attended transfer where transferor talks to B before completing.

---

## Future Enhancements

1. **Transfer timeout** - Auto-cancel if consultation takes too long
2. **Transfer to voicemail** - Detect if consultation party doesn't answer
3. **Three-way conference** - Before transfer, briefly join all three parties
4. **Transfer with announcement** - Play message to A when transfer completes
5. **Transfer queue** - Handle multiple pending transfers
6. **Replaces-draft-01 support** - Handle older SIP implementations

---

## Related Documents

- [STATE_EVENT_ACTION_MAPPING.md](./STATE_EVENT_ACTION_MAPPING.md) - State machine reference
- [RFC 5589](https://tools.ietf.org/html/rfc5589) - SIP Call Control - Transfer
- [RFC 3891](https://tools.ietf.org/html/rfc3891) - SIP Replaces Header