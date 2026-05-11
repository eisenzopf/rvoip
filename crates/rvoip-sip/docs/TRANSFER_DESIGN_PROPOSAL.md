# Transfer Design Proposal - Blind and Attended

## Current State Analysis

### Event Naming Inconsistency

We have **duplicate/overlapping event types** in the codebase:

| Purpose | API Event | State Table Event | Status |
|---------|-----------|-------------------|--------|
| **Blind Transfer** | `BlindTransfer` | `InitiateTransfer` | ❌ Mismatch |
| **Attended Transfer (Start)** | `StartAttendedTransfer` | `StartAttendedTransfer` | ✅ Match |
| **Attended Transfer (Complete)** | `CompleteAttendedTransfer` | `CompleteAttendedTransfer` | ✅ Match |

**Location of definitions:**
- **EventType enum**: `src/state_table/types.rs:135-136, 198-200`
- **State table**: `state_tables/default.yaml:343, 379, 392`

### Redundant Event Types

In `EventType` enum we have:
```rust
// User-facing events (used by API)
BlindTransfer { target: String },          // Line 135
AttendedTransfer { target: String },       // Line 136 (UNUSED!)

// Internal/state table events
InitiateTransfer { target: String },       // Line 198
StartAttendedTransfer { target: String },  // Line 199
CompleteAttendedTransfer,                  // Line 200
```

**Problem:**
- `AttendedTransfer` event exists but is NEVER used
- `BlindTransfer` is used by API but not by state table
- `InitiateTransfer` is used by state table but not by API

---

## SIP RFC Requirements

### RFC 5589: SIP Call Control - Transfer

Both blind and attended transfer use the **REFER method**, but differ in:

**Blind Transfer:**
1. Transferor sends REFER to transferee
2. Transferee initiates new call to transfer target
3. Transferor's call terminates
4. **Key:** Happens immediately, no consultation

**Attended Transfer:**
1. Transferor establishes consultation call to target
2. Transferor sends REFER with **Replaces header**
3. Transfer target receives new INVITE with Replaces
4. Transfer target replaces consultation call with transferee's call
5. **Key:** Consultation happens first, Replaces header used

### State Requirements

**Blind Transfer needs:**
- Active call (dialog established)
- Send REFER with Refer-To
- Transition to Transferring state
- Cleanup when NOTIFY received

**Attended Transfer needs:**
- Active call (original call)
- Put original call on hold
- Create consultation call (separate session)
- Link both sessions
- Send REFER with Replaces (using consultation dialog info)
- Terminate both calls when transfer completes

---

## Design Principles

### Principle 1: Consistency
**Use the same naming pattern for similar operations**

Both transfers should follow the same pattern:
- Start: `Start{Type}Transfer` or just `{Type}Transfer`
- Complete: `Complete{Type}Transfer` (if multi-step)
- Internal: `Initiate{Type}Transfer` (if needed)

### Principle 2: Clear Semantics
**Event names should reflect user intent, not SIP implementation**

- ✅ Good: `BlindTransfer` - user says "transfer this call"
- ❌ Bad: `InitiateTransfer` - unclear which type of transfer
- ✅ Good: `StartAttendedTransfer` - user says "start consultation for transfer"
- ✅ Good: `CompleteAttendedTransfer` - user says "complete the transfer"

### Principle 3: Minimize Duplication
**Don't have multiple events for the same purpose**

Current duplicates to eliminate:
- `BlindTransfer` vs `InitiateTransfer` - keep one
- `AttendedTransfer` (unused) - remove or use

---

## Recommended Approach: Option A (Preferred)

### Unified Event Naming

**User-facing events (API → State Machine):**
```rust
// Simple, clear names matching user intent
BlindTransfer { target: String }           // One-step transfer
StartAttendedTransfer { target: String }   // Begin consultation
CompleteAttendedTransfer                   // Finish transfer
CancelAttendedTransfer                     // Cancel consultation
```

**Internal events (State Machine internal):**
```rust
// Only if needed for internal state machine logic
TransferAccepted
TransferProgress
TransferComplete
TransferFailed
```

### State Table Changes

**Current:**
```yaml
# Blind transfer
- state: "Active"
  event:
    type: "InitiateTransfer"  # ❌ Wrong name
```

**Proposed:**
```yaml
# Blind transfer
- state: "Active"
  event:
    type: "BlindTransfer"     # ✅ Matches API
  next_state: "Transferring"
  actions:
    - type: "SendREFER"
```

### EventType Cleanup

**Remove:**
```rust
InitiateTransfer { target: String }  // Replaced by BlindTransfer
AttendedTransfer { target: String }  // Unused, remove
```

**Keep:**
```rust
BlindTransfer { target: String }
StartAttendedTransfer { target: String }
CompleteAttendedTransfer
```

---

## Alternative: Option B (More Explicit)

If we want to distinguish user events from internal events:

### Two-Tier Event System

**Tier 1: User Events (API)**
```rust
// These map to user actions
BlindTransfer { target: String }
StartAttendedTransfer { target: String }
CompleteAttendedTransfer
```

**Tier 2: Internal Events (State Table)**
```rust
// State machine uses these internally
InitiateBlindTransfer { target: String }
InitiateAttendedTransfer { target: String }
CompleteTransfer
```

**Mapping in API:**
```rust
pub async fn blind_transfer(&self, session_id: &SessionId, target: &str) -> Result<()> {
    // API receives user event, translates to internal event
    self.helpers.state_machine.process_event(
        session_id,
        EventType::InitiateBlindTransfer { target: target.to_string() },
    ).await?;
    Ok(())
}
```

**Pros:**
- Clear separation of concerns
- State table events can change without affecting API

**Cons:**
- More complexity
- Duplication of event types
- Translation layer needed

---

## Recommended Implementation Plan

### Phase 1: Align Blind Transfer (Quick Win)

**Goal:** Get blind transfer working immediately

**Changes:**
1. **State Table** (`state_tables/default.yaml:343`)
   ```yaml
   # Change from:
   type: "InitiateTransfer"
   # To:
   type: "BlindTransfer"
   ```

2. **Remove Unused Event** (`src/state_table/types.rs:198`)
   ```rust
   // Remove this line:
   InitiateTransfer { target: String },
   ```

3. **Update Event Normalization** (`src/state_table/types.rs:260`)
   ```rust
   // Remove this line:
   EventType::InitiateTransfer { .. } => EventType::InitiateTransfer { target: String::new() },
   ```

**Result:** Blind transfer will work end-to-end

---

### Phase 2: Add Missing State Support

**Goal:** Handle transfers in more states

**Changes:**
1. **Add transition for Answering state** (optional, for early transfer)
   ```yaml
   - role: "Both"
     state: "Answering"
     event:
       type: "BlindTransfer"
     next_state: "Transferring"
     actions:
       - type: "SendREFER"
     description: "Blind transfer during call setup"
   ```

2. **Fix Answering → Active transition** (investigate first)
   - Determine why UAS calls stay in Answering
   - Add/verify ACK handling transition
   - Ensure calls reach Active state properly

---

### Phase 3: Verify Attended Transfer

**Goal:** Ensure attended transfer works with our implementation

**Test Cases:**
1. Start attended transfer → creates consultation call ✅ (implemented)
2. Consultation call establishes ✅ (should work)
3. Complete transfer → sends REFER with Replaces ✅ (implemented)
4. Cancel transfer → terminates consultation, resumes original ✅ (implemented)

**Potential Issues:**
- State synchronization between original and consultation sessions
- REFER with Replaces header construction
- Dialog-core support for Replaces

**Validation Needed:**
```rust
// Does dialog-core support this?
dialog_api.send_refer(&dialog_id, refer_to, Some("attended".to_string()))
```

Check: Does the `Some("attended")` parameter actually build a Replaces header?

---

### Phase 4: Remove Unused Events

**Goal:** Clean up event type definitions

**Changes:**
1. **Remove `AttendedTransfer`** (unused user event)
   ```rust
   // Remove from types.rs:136
   AttendedTransfer { target: String },
   ```

2. **Remove `InitiateBlindTransfer` and `InitiateAttendedTransfer` actions**
   - These actions exist but aren't used
   - Only `SendREFER` and `SendREFERWithReplaces` are needed

---

## State Machine Transitions - Complete Picture

### Blind Transfer Flow

```
[Active]
   |
   | BlindTransfer { target: "sip:charlie@..." }
   |
   v
[Transferring]
   |--- Actions:
   |      - SendREFER(target)
   |      - (State machine sends REFER to remote party)
   |
   |--- Remote party receives REFER:
   |      - Sends NOTIFY with transfer status
   |      - Initiates new call to target
   |
   |--- TransferComplete event:
   |      - From NOTIFY 200 OK
   v
[Terminated]
   |--- Actions:
   |      - CleanupDialog
   |      - CleanupMedia
```

**Key States:**
- **Active** - Call established, ready to transfer
- **Transferring** - REFER sent, waiting for completion
- **Terminated** - Transfer complete, call ended

---

### Attended Transfer Flow

```
[Active] (original call with A)
   |
   | StartAttendedTransfer { target: "sip:charlie@..." }
   |
   v
[ConsultationCall]
   |--- Actions:
   |      - HoldCurrentCall (A on hold)
   |      - CreateConsultationCall (new session created)
   |
   |--- Consultation session established (separate state machine)
   |--- Talk to transfer target (C)
   |
   |--- Option 1: CompleteAttendedTransfer
   |      |
   |      v
   |   [Terminated]
   |      |--- Actions:
   |           - SendREFERWithReplaces(consultation_dialog_id)
   |           - CleanupDialog (both dialogs)
   |           - CleanupMedia (both media sessions)
   |
   |--- Option 2: CancelAttendedTransfer (HangupCall event)
   |      |
   |      v
   |   [Active]
   |      |--- Actions:
   |           - TerminateConsultationCall
   |           - ResumeOriginalCall (unhold A)
```

**Key States:**
- **Active** - Original call established
- **ConsultationCall** - Dual state: original on hold, consultation active
- **Terminated** - Both calls ended, transfer complete
- **Active** (again) - If transfer cancelled, return here

**Key Difference from Blind:**
- Two simultaneous sessions (original + consultation)
- Must track session linking
- REFER includes Replaces header pointing to consultation dialog

---

## API Consistency Check

### Current API (SimplePeer)

```rust
// Blind transfer
pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()>

// Attended transfer
pub async fn start_attended_transfer(&self, call_id: &CallId, target: &str) -> Result<CallId>
pub async fn complete_attended_transfer(&self, call_id: &CallId) -> Result<()>
pub async fn cancel_attended_transfer(&self, call_id: &CallId) -> Result<()>
```

**Recommendations:**
1. ✅ Keep `transfer()` for blind transfer - simple, clear
2. ✅ Keep `start_attended_transfer()` - returns consultation call ID
3. ✅ Keep `complete_attended_transfer()` - finishes transfer
4. ✅ Keep `cancel_attended_transfer()` - cleanup
5. ❓ Consider: Rename `transfer()` → `blind_transfer()` for clarity?

### Proposed (Explicit Naming)

```rust
// Make blind vs attended explicit
pub async fn blind_transfer(&self, call_id: &CallId, target: &str) -> Result<()>

pub async fn start_attended_transfer(&self, call_id: &CallId, target: &str) -> Result<CallId>
pub async fn complete_attended_transfer(&self, call_id: &CallId) -> Result<()>
pub async fn cancel_attended_transfer(&self, call_id: &CallId) -> Result<()>

// Keep transfer() as alias for blind_transfer() for backwards compat
pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()> {
    self.blind_transfer(call_id, target).await
}
```

---

## Testing Requirements

### Blind Transfer Test
```rust
#[tokio::test]
async fn test_blind_transfer_active_state() {
    // 1. Alice calls Bob
    // 2. Bob accepts, call reaches Active
    // 3. Bob calls transfer(&call_id, "sip:charlie@...")
    // 4. Verify:
    //    - REFER sent to Alice
    //    - Bob's state → Transferring
    //    - Alice receives REFER
    //    - Alice calls Charlie
    //    - Bob's call terminates
}

#[tokio::test]
async fn test_blind_transfer_answering_state() {
    // If we support early transfer:
    // 1. Alice calls Bob
    // 2. Bob accepts but call not yet Active
    // 3. Bob calls transfer() immediately
    // 4. Should either:
    //    - Work (if we add Answering transition)
    //    - Return clear error (if we don't)
}
```

### Attended Transfer Test
```rust
#[tokio::test]
async fn test_attended_transfer_complete() {
    // 1. Alice calls Bob
    // 2. Bob: consultation_id = start_attended_transfer(&call_id, "sip:charlie@...")
    // 3. Verify:
    //    - Alice on hold
    //    - Consultation call to Charlie created
    //    - Bob can talk to Charlie
    // 4. Bob: complete_attended_transfer(&call_id)
    // 5. Verify:
    //    - REFER with Replaces sent
    //    - Alice calls Charlie with Replaces header
    //    - Bob's calls terminate
    //    - Alice and Charlie connected
}

#[tokio::test]
async fn test_attended_transfer_cancel() {
    // 1-3: Same as above
    // 4. Bob: cancel_attended_transfer(&call_id)
    // 5. Verify:
    //    - Consultation call terminated
    //    - Alice taken off hold
    //    - Bob and Alice talking again
}
```

---

## Migration Path

### For Existing Code

If code exists using `InitiateTransfer`:

**Option 1: Silent Migration (Recommended)**
- State table changes only
- No API changes needed
- Existing code continues to work

**Option 2: Deprecation Path**
```rust
// Mark old event as deprecated
#[deprecated(note = "Use BlindTransfer instead")]
InitiateTransfer { target: String },

// State table supports both temporarily
- event:
    type: "InitiateTransfer"  # Old name
- event:
    type: "BlindTransfer"     # New name
```

---

## Summary Recommendations

### 🎯 Immediate Actions (Phase 1)

1. **Change state table:** `InitiateTransfer` → `BlindTransfer`
2. **Remove unused event:** Delete `InitiateTransfer` from EventType
3. **Test blind transfer** - Should work end-to-end

### ⚙️ Near-term (Phase 2)

4. **Investigate Answering→Active transition** - Why do calls get stuck?
5. **Add Answering state support** (optional) - For early transfer
6. **Increase example wait times** - Ensure calls reach Active before transfer

### 🧪 Validation (Phase 3)

7. **Test attended transfer** - Verify REFER with Replaces works
8. **Check dialog-core** - Ensure Replaces header construction works
9. **Create attended transfer example** - Similar to blind transfer example

### 🧹 Cleanup (Phase 4)

10. **Remove unused events** - `AttendedTransfer`, action duplicates
11. **Document transfer API** - Clear guide on blind vs attended
12. **Add state validation** - Return errors for invalid states

---

## Decision Matrix

| Aspect | Option A (Unified) | Option B (Two-Tier) | Recommendation |
|--------|-------------------|---------------------|----------------|
| **Simplicity** | ✅ Simple | ❌ Complex | **Option A** |
| **Consistency** | ✅ One event = one purpose | ⚠️ Two events per purpose | **Option A** |
| **Flexibility** | ⚠️ API changes affect state table | ✅ Can evolve independently | - |
| **Clarity** | ✅ Clear user intent | ⚠️ Translation layer obscures | **Option A** |
| **Migration** | ✅ Easy | ⚠️ More changes needed | **Option A** |

**Recommendation: Option A (Unified)** - Use the same event names in API and state table for simplicity and clarity.

---

## Open Questions

1. **REFER with Replaces**: Does dialog-core properly construct the Replaces header?
   - Need to verify: `send_refer(&dialog_id, refer_to, Some("attended"))`
   - May need to enhance to: `send_refer_with_replaces(&dialog_id, &consultation_dialog_id)`

2. **Call state progression**: Why do UAS calls stay in Answering?
   - Is ACK being received?
   - Is there a missing state transition?
   - Should we add logging to diagnose?

3. **Transfer timing**: Should we support transfer before call is Active?
   - SIP allows it (REFER can be sent anytime after INVITE)
   - But semantics are unclear
   - Safer to require Active state

4. **API naming**: Should `transfer()` be renamed to `blind_transfer()`?
   - Pro: Explicit and clear
   - Con: Breaking change for existing code
   - Compromise: Add `blind_transfer()`, keep `transfer()` as alias
