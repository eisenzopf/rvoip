# Three Transfer Types Design - Comprehensive Approach

## Transfer Type Definitions

### 1. Blind Transfer (Unattended/Cold Transfer)
**User Story:** "Transfer this caller to someone else, I don't need to talk to them first"

**Flow:**
1. Alice calls Bob
2. Bob transfers Alice to Charlie **without talking to Charlie**
3. Bob's call ends immediately
4. Alice's phone receives REFER, calls Charlie
5. Alice and Charlie connected (Bob is out)

**SIP Method:** REFER (simple)

**Key Characteristics:**
- ⚡ Immediate - no consultation
- 🎲 Risky - transferor doesn't know if target will answer
- 🏃 Quick - one step operation
- 📞 Target doesn't know transfer is coming

**Use Cases:**
- IVR menu selections
- Basic call routing
- Quick handoffs

---

### 2. Attended Transfer (Warm Transfer/Consultation Transfer)
**User Story:** "Let me talk to the person first, make sure they can take the call, then transfer"

**Flow:**
1. Alice calls Bob
2. Bob puts Alice **on hold**
3. Bob calls Charlie (consultation call)
4. Bob talks to Charlie, explains situation
5. Bob completes transfer (sends REFER with Replaces)
6. Alice's call replaces Bob's consultation call with Charlie
7. Alice and Charlie connected (Bob is out)

**SIP Method:** REFER with Replaces header

**Key Characteristics:**
- 💬 Consultation - talk to target first
- ✅ Verified - know target will accept
- 🔄 Two sessions - original + consultation
- 📋 Context - can explain caller's situation
- 🚪 Exit - transferor drops out after

**Use Cases:**
- Customer service handoffs
- Expert consultation needed
- Explaining caller context to recipient

---

### 3. Managed Transfer (Supervised Transfer/3-Way Handoff)
**User Story:** "Put me on the line with both parties, introduce them, then optionally drop out"

**Flow:**
1. Alice calls Bob
2. Bob puts Alice on hold
3. Bob calls Charlie (consultation call)
4. Bob talks to Charlie
5. Bob **bridges all three parties** (3-way call/conference)
6. Bob introduces Alice to Charlie, all three talking
7. **Option A:** Bob drops out, Alice and Charlie continue
8. **Option B:** All three stay on call (ad-hoc conference)

**SIP Method:** Conference bridge + optional REFER

**Key Characteristics:**
- 👥 Three-way - all parties can talk
- 🎙️ Introduction - transferor stays on line
- 🎛️ Control - transferor manages the handoff
- 🚪 Optional exit - transferor can leave or stay
- 🔊 Conference - requires audio mixing

**Use Cases:**
- Complex support escalations
- Sales handoffs with introduction
- Legal/financial transfers requiring witness
- Training (supervisor listens to handoff)
- Collaborative problem solving

---

## Key Differences

| Aspect | Blind | Attended | Managed |
|--------|-------|----------|---------|
| **Consultation** | ❌ None | ✅ Yes (1-on-1) | ✅ Yes (1-on-1 then 3-way) |
| **Hold Original** | ❌ No | ✅ Yes | ✅ Yes |
| **Audio Mixing** | ❌ No | ❌ No | ✅ Yes (3-way) |
| **Transferor Talks** | ❌ No | ✅ To target only | ✅ To both parties |
| **Transferor Exit** | ✅ Immediate | ✅ After REFER | ⚠️ Optional |
| **Complexity** | Simple | Medium | Complex |
| **SIP Primitive** | REFER | REFER+Replaces | Conference+REFER |
| **Sessions** | 1 | 2 | 2 (then merged to conference) |
| **Media** | Direct RTP | Direct RTP | Mixed audio (B2BUA) |

---

## State Machine Requirements

### Blind Transfer States
```
[Active]
   ↓ BlindTransfer
[Transferring]
   ↓ TransferComplete
[Terminated]
```

**Required:**
- Active call
- Send REFER
- Terminate on NOTIFY 200 OK

---

### Attended Transfer States
```
[Active] (call with A)
   ↓ StartAttendedTransfer
[ConsultationCall] (A on hold, talking to C)
   ↓ CompleteAttendedTransfer
[Terminated] (A now talking to C, we're out)
```

**Required:**
- Hold original call (A)
- Create consultation call (C)
- Link both sessions
- Send REFER with Replaces
- Terminate both calls

---

### Managed Transfer States
```
[Active] (call with A)
   ↓ StartManagedTransfer
[ConsultationCall] (A on hold, talking to C)
   ↓ BridgeWithOriginal or CreateConference
[ThreeWayCall] (A ↔ You ↔ C all talking)
   ↓ Option 1: CompleteManagedTransfer
[Terminated] (A ↔ C, you drop out)

   ↓ Option 2: ContinueConference
[Conference] (A ↔ You ↔ C, you stay)
```

**Required:**
- Hold original call (A)
- Create consultation call (C)
- **Create audio mixer/conference**
- **Bridge A → mixer ← C → mixer ← You**
- Send audio from all parties to all others
- Option to drop out (A ↔ C bridge remains)
- Option to stay (maintain 3-way conference)

---

## Event Naming Strategy

### Consistent Naming Pattern

Use `{Type}Transfer` for one-step operations, `Start{Type}Transfer` for multi-step:

```rust
// Blind Transfer (one step)
BlindTransfer { target: String }

// Attended Transfer (two steps)
StartAttendedTransfer { target: String }
CompleteAttendedTransfer
CancelAttendedTransfer  // Optional: cancel consultation

// Managed Transfer (three steps)
StartManagedTransfer { target: String }
BridgeManagedTransfer    // Join all three parties
CompleteManagedTransfer  // Drop out, leave A ↔ C
CancelManagedTransfer    // Cancel consultation
```

**Internal Events (state machine):**
```rust
TransferAccepted
TransferProgress
TransferComplete
TransferFailed

ConferenceCreated
ConferenceActive
ConferenceTerminated
```

---

## API Design

### SimplePeer API

```rust
impl SimplePeer {
    // ===== Blind Transfer =====

    /// Blind transfer - immediate transfer without consultation
    pub async fn blind_transfer(&self, call_id: &CallId, target: &str) -> Result<()>;

    // Alias for backward compatibility
    pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.blind_transfer(call_id, target).await
    }

    // ===== Attended Transfer =====

    /// Start attended transfer - puts caller on hold, creates consultation call
    /// Returns the consultation call ID
    pub async fn start_attended_transfer(
        &self,
        call_id: &CallId,
        target: &str
    ) -> Result<CallId>;

    /// Complete attended transfer - sends REFER with Replaces, drops out
    pub async fn complete_attended_transfer(&self, call_id: &CallId) -> Result<()>;

    /// Cancel attended transfer - terminates consultation, resumes original
    pub async fn cancel_attended_transfer(&self, call_id: &CallId) -> Result<()>;

    // ===== Managed Transfer =====

    /// Start managed transfer - puts caller on hold, creates consultation call
    /// Returns the consultation call ID
    pub async fn start_managed_transfer(
        &self,
        call_id: &CallId,
        target: &str
    ) -> Result<CallId>;

    /// Bridge all three parties into conference (caller, you, target)
    pub async fn bridge_managed_transfer(&self, call_id: &CallId) -> Result<()>;

    /// Complete managed transfer - drop out, leave caller and target connected
    pub async fn complete_managed_transfer(&self, call_id: &CallId) -> Result<()>;

    /// Cancel managed transfer - terminate consultation, resume original
    pub async fn cancel_managed_transfer(&self, call_id: &CallId) -> Result<()>;

    /// Stay in the call - convert to permanent 3-way conference
    pub async fn continue_managed_transfer_as_conference(&self, call_id: &CallId) -> Result<()>;
}
```

---

## State Table Design

### Blind Transfer (Simple)

```yaml
# Blind transfer
- role: "Both"
  state: "Active"
  event:
    type: "BlindTransfer"
  next_state: "Transferring"
  actions:
    - type: "SendREFER"
  publish:
    - "TransferInitiated"
  description: "Immediate transfer without consultation"

- role: "Both"
  state: "Transferring"
  event:
    type: "TransferComplete"
  next_state: "Terminated"
  actions:
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "TransferSucceeded"
```

---

### Attended Transfer (Medium Complexity)

```yaml
# Start attended transfer
- role: "Both"
  state: "Active"
  event:
    type: "StartAttendedTransfer"
  next_state: "ConsultationCall"
  actions:
    - type: "HoldCurrentCall"
    - type: "CreateConsultationCall"
  publish:
    - "ConsultationStarted"

# Complete attended transfer
- role: "Both"
  state: "ConsultationCall"
  event:
    type: "CompleteAttendedTransfer"
  next_state: "Terminated"
  actions:
    - type: "SendREFERWithReplaces"
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "AttendedTransferCompleted"

# Cancel attended transfer
- role: "Both"
  state: "ConsultationCall"
  event:
    type: "CancelAttendedTransfer"
  next_state: "Active"
  actions:
    - type: "TerminateConsultationCall"
    - type: "ResumeOriginalCall"
  publish:
    - "ConsultationCancelled"
```

---

### Managed Transfer (Complex - NEW)

```yaml
# Start managed transfer
- role: "Both"
  state: "Active"
  event:
    type: "StartManagedTransfer"
  next_state: "ConsultationCall"
  actions:
    - type: "HoldCurrentCall"
    - type: "CreateConsultationCall"
  publish:
    - "ConsultationStarted"
  description: "Start consultation for managed transfer"

# Bridge all three parties
- role: "Both"
  state: "ConsultationCall"
  event:
    type: "BridgeManagedTransfer"
  next_state: "ThreeWayCall"
  actions:
    - type: "CreateConference"
    - type: "AddOriginalToConference"
    - type: "AddConsultationToConference"
    - type: "AddSelfToConference"
  publish:
    - "ThreeWayCallActive"
  description: "Join all three parties in conference"

# Complete transfer - drop out
- role: "Both"
  state: "ThreeWayCall"
  event:
    type: "CompleteManagedTransfer"
  next_state: "Terminated"
  actions:
    - type: "RemoveSelfFromConference"
    - type: "BridgeRemainingParties"  # Direct A ↔ C
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "ManagedTransferCompleted"
  description: "Drop out, leave caller and target connected"

# Stay in call - convert to conference
- role: "Both"
  state: "ThreeWayCall"
  event:
    type: "ContinueManagedTransferAsConference"
  next_state: "Conference"
  actions:
    - type: "ConvertToStableConference"
  publish:
    - "ConferenceEstablished"
  description: "Keep all three parties in permanent conference"

# Cancel managed transfer
- role: "Both"
  state: "ConsultationCall"
  event:
    type: "CancelManagedTransfer"
  next_state: "Active"
  actions:
    - type: "TerminateConsultationCall"
    - type: "ResumeOriginalCall"
  publish:
    - "ConsultationCancelled"

# Cancel from three-way
- role: "Both"
  state: "ThreeWayCall"
  event:
    type: "CancelManagedTransfer"
  next_state: "Active"
  actions:
    - type: "DestroyConference"
    - type: "TerminateConsultationCall"
    - type: "ResumeOriginalCall"
  publish:
    - "ManagedTransferCancelled"
```

---

## Required New Actions

For managed transfer, we need:

```rust
// Conference management
Action::CreateConference,                  // Create audio mixer
Action::AddOriginalToConference,           // Add held call to mixer
Action::AddConsultationToConference,       // Add consultation call to mixer
Action::AddSelfToConference,               // Add our audio to mixer
Action::RemoveSelfFromConference,          // Remove our audio from mixer
Action::DestroyConference,                 // Tear down mixer

// Bridge management
Action::BridgeRemainingParties,            // Direct A ↔ C connection
Action::ConvertToStableConference,         // Make conference permanent

// State management
Action::ResumeOriginalCall,                // Unhold (already exists)
Action::TerminateConsultationCall,         // Cleanup consultation (already exists)
```

---

## Implementation Phases

### Phase 1: Fix Blind Transfer (Week 1)
**Goal:** Get basic blind transfer working

1. ✅ Change `InitiateTransfer` → `BlindTransfer` in state table
2. ✅ Fix call establishment (Answering → Active)
3. ✅ Test blind transfer example
4. ✅ Verify REFER handling

**Deliverable:** Working blind transfer end-to-end

---

### Phase 2: Verify Attended Transfer (Week 2)
**Goal:** Ensure attended transfer works (already implemented)

5. ✅ Test attended transfer state machine
6. ✅ Verify REFER with Replaces in dialog-core
7. ✅ Create attended transfer example
8. ✅ Test consultation cancellation

**Deliverable:** Working attended transfer end-to-end

---

### Phase 3: Implement Managed Transfer (Week 3-4)
**Goal:** Add managed transfer with conferencing

9. 🆕 Add `ThreeWayCall` and `Conference` states
10. 🆕 Implement conference creation actions
11. 🆕 Add audio mixing support in media-core
12. 🆕 Implement bridge management
13. 🆕 Add managed transfer API methods
14. 🆕 Create managed transfer example

**Deliverable:** Working managed transfer with 3-way calling

---

### Phase 4: Polish & Documentation (Week 5)
**Goal:** Production ready

15. 📚 Document all three transfer types
16. 🧪 Comprehensive test suite
17. 🎯 Performance optimization
18. 🔍 Edge case handling
19. 📖 User guide with examples
20. 🎨 Consistent API across all types

**Deliverable:** Production-ready transfer feature set

---

## Architecture Requirements

### For Managed Transfer

**Media Core Needs:**
```rust
// Audio mixer capability
pub trait MediaAdapter {
    // Create a conference mixer
    async fn create_conference_mixer(&self) -> Result<MixerId>;

    // Add a session's audio to the mixer
    async fn add_to_mixer(&self, mixer_id: &MixerId, session_id: &SessionId) -> Result<()>;

    // Remove a session from mixer
    async fn remove_from_mixer(&self, mixer_id: &MixerId, session_id: &SessionId) -> Result<()>;

    // Destroy mixer
    async fn destroy_mixer(&self, mixer_id: &MixerId) -> Result<()>;

    // Bridge two sessions directly (after removing from mixer)
    async fn bridge_sessions(&self, session_a: &SessionId, session_b: &SessionId) -> Result<()>;
}
```

**Session Store Needs:**
```rust
pub struct SessionState {
    // ... existing fields ...

    // Transfer tracking
    pub consultation_session_id: Option<SessionId>,
    pub original_session_id: Option<SessionId>,
    pub transfer_type: TransferType,

    // Conference tracking
    pub conference_id: Option<MixerId>,
    pub conference_participants: Vec<SessionId>,
}

pub enum TransferType {
    None,
    Blind,
    Attended,
    Managed,
}
```

---

## Comparison with Industry Standards

### Cisco/Avaya/Asterisk Terminology

| Our Term | Cisco | Avaya | Asterisk | SIP RFC |
|----------|-------|-------|----------|---------|
| **Blind Transfer** | Blind Transfer | Send Calls | Blind Transfer | REFER |
| **Attended Transfer** | Consult Transfer | Conference/Transfer | Attended Transfer | REFER+Replaces |
| **Managed Transfer** | Supervised Transfer | Conference/Drop | Supervised Transfer | Conference |

**Note:** "Supervised Transfer" is the most common industry term for what we call "Managed Transfer"

---

## Example Scenarios

### Scenario 1: Customer Service (Attended)
```
1. Customer calls support (Alice → Bob)
2. Bob realizes needs expert (starts attended transfer)
3. Bob calls expert Charlie (consultation)
4. Bob explains: "Customer has payment issue"
5. Charlie: "OK, transfer them"
6. Bob completes transfer
7. Alice now talking to Charlie
```

**Transfer Type:** Attended (Bob talks to Charlie privately first)

---

### Scenario 2: Sales Handoff (Managed)
```
1. Lead calls sales (Alice → Bob)
2. Bob qualifies lead, needs manager
3. Bob starts managed transfer to manager Charlie
4. Bob talks to Charlie: "Hot lead, ready to buy"
5. Charlie: "Great, bring them in"
6. Bob bridges all three: "Alice, meet Charlie, our sales manager"
7. All three talking, Bob introduces
8. Bob: "Charlie will take care of you from here"
9. Bob drops out, Alice and Charlie continue
```

**Transfer Type:** Managed (Bob introduces both parties)

---

### Scenario 3: Emergency Transfer (Blind)
```
1. Caller dials wrong department (Alice → Bob)
2. Bob: "You need sales, let me transfer you"
3. *Blind transfer to sales*
4. Alice's phone automatically dials sales
```

**Transfer Type:** Blind (No consultation needed)

---

## User Experience Flow Charts

### Blind Transfer
```
User Action                  System Response
──────────────────────────  ─────────────────────────────
1. Call connected           → [Active state]
2. Click "Transfer"         → Show transfer dialog
3. Enter target number      → Validate input
4. Click "Send"             → SendREFER
                            → [Transferring state]
5. Wait...                  → NOTIFY received
                            → [Terminated state]
6. Call ends                → Success notification
```

**UI Elements Needed:**
- Transfer button
- Number input field
- Send button
- Status indicator

---

### Attended Transfer
```
User Action                  System Response
──────────────────────────  ─────────────────────────────
1. Call connected           → [Active state]
2. Click "Consult Transfer" → Show transfer dialog
3. Enter target number      → Validate input
4. Click "Start"            → HoldCurrentCall
                            → CreateConsultationCall
                            → [ConsultationCall state]
5. Talk to target           → Two active sessions
6. Click "Complete"         → SendREFERWithReplaces
                            → [Terminated state]
7. Calls end                → Success notification

Alternative: Click "Cancel" → TerminateConsultation
                            → ResumeOriginalCall
                            → [Active state]
```

**UI Elements Needed:**
- Consult Transfer button
- Number input field
- Start button
- Talk time with target
- Complete/Cancel buttons
- Status indicator for both calls

---

### Managed Transfer
```
User Action                  System Response
──────────────────────────  ─────────────────────────────
1. Call connected           → [Active state]
2. Click "Managed Transfer" → Show transfer dialog
3. Enter target number      → Validate input
4. Click "Start"            → HoldCurrentCall
                            → CreateConsultationCall
                            → [ConsultationCall state]
5. Talk to target           → One-on-one with target
6. Click "Join All"         → CreateConference
                            → AddToMixer (all 3)
                            → [ThreeWayCall state]
7. All talking              → Conference active
8a. Click "Drop Out"        → RemoveSelfFromMixer
                            → BridgeRemaining
                            → [Terminated state]
   OR
8b. Click "Stay On"         → ConvertToConference
                            → [Conference state]

Alternative: Click "Cancel" → TerminateConsultation
                            → ResumeOriginalCall
                            → [Active state]
```

**UI Elements Needed:**
- Managed Transfer button
- Number input field
- Start button
- Join All button
- Drop Out / Stay On buttons
- Visual indicator of who's talking
- Mute controls for all parties
- Status indicator

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    // Blind Transfer
    #[tokio::test]
    async fn test_blind_transfer_from_active()

    #[tokio::test]
    async fn test_blind_transfer_invalid_state()

    // Attended Transfer
    #[tokio::test]
    async fn test_attended_transfer_complete()

    #[tokio::test]
    async fn test_attended_transfer_cancel()

    #[tokio::test]
    async fn test_attended_transfer_consultation_fails()

    // Managed Transfer
    #[tokio::test]
    async fn test_managed_transfer_complete()

    #[tokio::test]
    async fn test_managed_transfer_stay_on()

    #[tokio::test]
    async fn test_managed_transfer_cancel_before_bridge()

    #[tokio::test]
    async fn test_managed_transfer_cancel_after_bridge()

    #[tokio::test]
    async fn test_managed_transfer_audio_mixing()
}
```

### Integration Tests

Create example directories:
- `examples/blind_transfer/` ✅ (already exists)
- `examples/attended_transfer/` (need to create)
- `examples/managed_transfer/` (need to create)

---

## Migration from Attended Transfer Implementation

**Good News:** We already implemented attended transfer, which shares ~80% of the logic needed for managed transfer!

**What We Have:**
- ✅ Consultation call creation
- ✅ Session linking
- ✅ Hold/resume functionality
- ✅ State transitions
- ✅ Cleanup on cancel

**What We Need to Add:**
- 🆕 Conference/mixer creation
- 🆕 3-way audio bridging
- 🆕 ThreeWayCall state
- 🆕 Drop-out logic (bridge remaining)
- 🆕 Stay-on logic (permanent conference)

**Code Reuse:**
```rust
// Attended and Managed share this:
StartAttendedTransfer / StartManagedTransfer
  ↓
[ConsultationCall state]
  ↓ (consultation established)

// Then diverge:
Attended: CompleteAttendedTransfer → SendREFERWithReplaces → Terminated
Managed:  BridgeManagedTransfer → ThreeWayCall → [Complete/Continue]
```

---

## Recommendation: Implementation Order

1. **First: Fix Blind Transfer** (1 week)
   - Simplest, gets basic transfer working
   - Unblocks testing of REFER mechanism
   - Validates state machine approach

2. **Second: Validate Attended Transfer** (1 week)
   - Already implemented, just test
   - Validates consultation pattern
   - Validates session linking

3. **Third: Implement Managed Transfer** (2-3 weeks)
   - Most complex, builds on others
   - Requires conference/mixer support
   - Provides complete feature set

This progressive approach minimizes risk and validates each component before building on it.
