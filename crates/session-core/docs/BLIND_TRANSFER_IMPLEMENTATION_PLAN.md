# Blind Transfer Implementation Plan

## Overview

This document outlines the implementation plan for blind transfer functionality in session-core-v2, designed to be **reusable** for attended and managed transfers.

## Design Philosophy

**Option 2: Event-Driven with Helpers**
- Low-level API: Publish `TransferRequested` event, application controls via helper methods
- SimplePeer API: Automatically handle transfers seamlessly
- Build foundation that works for all three transfer types (blind, attended, managed)

## Architecture

### Common Transfer Pattern

All three transfer types share the same core mechanism:

```
┌─────────────────┐
│ Transfer Types  │
├─────────────────┤
│ Blind          │──┐
│ Attended       │──┼──> TransferCoordinator
│ Managed        │──┘     (Shared Core Logic)
└─────────────────┘
```

**Common Operations (60-70% reuse):**
1. Create new session for transfer target
2. Make call to transfer target
3. Send NOTIFY messages about progress
4. Optionally terminate old call

**Type-Specific Differences:**
- **Blind**: No Replaces header, don't wait for establishment, terminate immediately
- **Attended**: Add Replaces header, wait for establishment, then terminate
- **Managed**: No Replaces, keep old call alive, create conference

## Implementation Components

### 1. TransferCoordinator Module

**Location:** `crates/session-core-v2/src/transfer/coordinator.rs`

**Purpose:** Centralized transfer logic shared by all transfer types

**Key Method:**
```rust
pub struct TransferCoordinator {
    session_store: Arc<SessionStore>,
    event_processor: Arc<EventProcessor>,
    dialog_adapter: Arc<DialogAdapter>,
}

impl TransferCoordinator {
    /// Complete a transfer by calling the transfer target
    /// Used by all three transfer types with different options
    pub async fn complete_transfer(
        &self,
        session_id: &SessionId,
        refer_to: &str,
        options: TransferOptions,
    ) -> Result<TransferResult>;
}

pub struct TransferOptions {
    /// For attended transfer - include Replaces header in INVITE
    pub replaces_header: Option<String>,

    /// Wait for new call to reach Active state before proceeding
    pub wait_for_establishment: bool,

    /// Terminate the old call after transfer completes
    pub terminate_old_call: bool,

    /// Send NOTIFY messages to transferor about progress
    pub send_notify: bool,
}

pub struct TransferResult {
    /// New session ID created for transfer target
    pub new_session_id: SessionId,

    /// New dialog ID
    pub new_dialog_id: Option<DialogId>,

    /// Transfer success/failure
    pub success: bool,
}
```

### 2. Session API Extensions

**Location:** `crates/session-core-v2/src/session/mod.rs`

Add helper methods to Session API for application use:

```rust
impl SessionManager {
    /// Complete a blind transfer (helper for applications)
    pub async fn complete_blind_transfer(
        &self,
        session_id: &SessionId,
        refer_to: &str,
    ) -> Result<SessionId>;

    /// Reject a transfer request
    pub async fn reject_transfer(
        &self,
        session_id: &SessionId,
    ) -> Result<()>;
}
```

### 3. SimplePeer Auto-Transfer

**Location:** `crates/session-core-v2/src/api/simple.rs`

SimplePeer automatically handles transfers without developer intervention:

```rust
impl SimplePeer {
    // Internal event handler
    async fn handle_transfer_requested(&self, session_id: SessionId, refer_to: String) {
        // Automatically complete transfer
        if let Err(e) = self.session_manager
            .complete_blind_transfer(&session_id, &refer_to)
            .await
        {
            error!("Auto-transfer failed: {}", e);
        }
    }
}
```

### 4. State Machine Actions

**Location:** `crates/session-core-v2/src/state_machine/actions.rs`

Update existing actions to use TransferCoordinator:

**Current Actions:**
- `AcceptTransferREFER` - Accept REFER with 202
- `SendTransferNOTIFY` - Send NOTIFY messages
- `StoreTransferTarget` - Store refer-to URI
- `TerminateCurrentCall` - Send BYE to old call

**Changes Needed:**
- Keep existing actions for low-level control
- Add new action: `CompleteBlindTransfer` that calls TransferCoordinator

### 5. NOTIFY Progress Reporting

**Location:** `crates/session-core-v2/src/transfer/notify.rs`

Implement SIP NOTIFY messages per RFC 3515:

```rust
pub struct TransferNotifyHandler {
    dialog_adapter: Arc<DialogAdapter>,
}

impl TransferNotifyHandler {
    /// Send NOTIFY to transferor about transfer progress
    pub async fn send_notify(
        &self,
        session_id: &SessionId,
        sipfrag: &str, // e.g., "SIP/2.0 100 Trying", "SIP/2.0 200 OK"
    ) -> Result<()>;
}
```

## Implementation Phases

### Phase 1: Core Transfer Infrastructure ✅ Ready to Start

**Goal:** Build reusable TransferCoordinator

**Tasks:**
1. Create `crates/session-core-v2/src/transfer/` module
2. Implement `TransferCoordinator` with `complete_transfer()` method
3. Implement `TransferOptions` and `TransferResult` types
4. Add NOTIFY handler for progress reporting

**Deliverable:** Reusable transfer coordination that works for all types

### Phase 2: Blind Transfer Implementation

**Goal:** Implement blind transfer end-to-end

**Tasks:**
1. Add `complete_blind_transfer()` to SessionManager
2. Update `TerminateCurrentCall` action to use TransferCoordinator
3. Test: Bob → Alice → Charlie blind transfer flow
4. Verify TransferRequested event triggers correctly

**Deliverable:** Working blind transfer in example code

### Phase 3: SimplePeer Auto-Transfer

**Goal:** Make SimplePeer handle transfers automatically

**Tasks:**
1. Add internal TransferRequested event handler to SimplePeer
2. Automatically call `complete_blind_transfer()` when event received
3. Test with SimplePeer example
4. No developer code needed for transfers

**Deliverable:** Zero-code transfers in SimplePeer

### Phase 4: Testing & Validation

**Goal:** Comprehensive testing

**Tasks:**
1. Unit tests for TransferCoordinator
2. Integration test: Alice → Bob → Charlie
3. Test NOTIFY message flow
4. Test error cases (target unreachable, etc.)
5. Update blind_transfer example to show both manual and auto modes

**Deliverable:** Production-ready blind transfer

## Future Extensions (Attended & Managed)

### Attended Transfer (Phase 5 - Future)

**Changes needed:**
```rust
// Add Replaces header support
complete_transfer(session_id, refer_to, TransferOptions {
    replaces_header: Some(replaces),  // ← New
    wait_for_establishment: true,      // ← Different from blind
    terminate_old_call: true,
    send_notify: true,
});
```

**New components:**
- Parse Replaces header from REFER
- Add Replaces header to outgoing INVITE
- Match and replace existing dialog

### Managed Transfer (Phase 6 - Future)

**Changes needed:**
```rust
// Step 1: Consultation call
let consult = complete_transfer(session_id, refer_to, TransferOptions {
    replaces_header: None,
    wait_for_establishment: true,
    terminate_old_call: false,         // ← Keep original call!
    send_notify: false,
});

// Step 2: Create conference
create_conference(vec![original_session, consult_session]);

// Step 3: Bridge audio
// (Requires mixer/conference implementation)
```

**New components:**
- Conference/mixer creation
- 3-way audio bridging
- Participant management

## Code Reuse Matrix

| Component | Blind | Attended | Managed | Reuse % |
|-----------|-------|----------|---------|---------|
| TransferCoordinator core | ✅ | ✅ | ✅ | 100% |
| Create new session | ✅ | ✅ | ✅ | 100% |
| Make call to target | ✅ | ✅ | ✅ | 100% |
| Send NOTIFY | ✅ | ✅ | ✅ | 100% |
| Hang up old call | ✅ | ✅ | ✅ | 100% |
| Replaces header | ❌ | ✅ | ❌ | 33% |
| Wait for establishment | ❌ | ✅ | ✅ | 66% |
| Conference/mixer | ❌ | ❌ | ✅ | 33% |

**Overall reuse: ~70%** of blind transfer code will be reused for attended/managed

## API Examples

### Low-Level API (Event-Driven)

```rust
// Application using session-core-v2 directly
let mut events = session_manager.subscribe_events();

while let Some(event) = events.recv().await {
    match event {
        SessionEvent::TransferRequested { session_id, refer_to, transfer_type } => {
            // Option A: Auto-accept
            session_manager.complete_blind_transfer(&session_id, &refer_to).await?;

            // Option B: Prompt user first
            if user_approves_transfer(&refer_to) {
                session_manager.complete_blind_transfer(&session_id, &refer_to).await?;
            } else {
                session_manager.reject_transfer(&session_id).await?;
            }
        }
        _ => {}
    }
}
```

### SimplePeer API (Automatic)

```rust
// No transfer code needed - it just works!
let mut peer = SimplePeer::new("alice", "127.0.0.1:5060").await?;

// Transfers are handled automatically internally
// Developer never sees TransferRequested events
```

## Success Criteria

### Phase 1-2 Complete When:
- ✅ Bob sends REFER to Alice
- ✅ Alice receives TransferRequested event
- ✅ Alice's state machine transitions: Active → TransferringCall
- ✅ Alice automatically calls Charlie
- ✅ Alice sends BYE to Bob
- ✅ Alice and Charlie are in Active call
- ✅ Charlie receives IncomingCall event
- ✅ NOTIFY messages sent to Bob about transfer progress

### Phase 3 Complete When:
- ✅ SimplePeer handles transfer with zero developer code
- ✅ Example still shows manual option for low-level API users

### Phase 4 Complete When:
- ✅ All tests passing
- ✅ Error cases handled gracefully
- ✅ Documentation updated
- ✅ Example code demonstrates both manual and automatic modes

## Files to Modify/Create

### New Files:
- `crates/session-core-v2/src/transfer/mod.rs` (module)
- `crates/session-core-v2/src/transfer/coordinator.rs` (core logic)
- `crates/session-core-v2/src/transfer/notify.rs` (NOTIFY handler)
- `crates/session-core-v2/src/transfer/types.rs` (TransferOptions, TransferResult)

### Modified Files:
- `crates/session-core-v2/src/lib.rs` (export transfer module)
- `crates/session-core-v2/src/session/mod.rs` (add helper methods)
- `crates/session-core-v2/src/api/simple.rs` (auto-transfer handler)
- `crates/session-core-v2/src/state_machine/actions.rs` (use TransferCoordinator)
- `crates/session-core-v2/examples/blind_transfer/*.rs` (demonstrate usage)

## References

- RFC 3515: The Session Initiation Protocol (SIP) Refer Method
- RFC 5589: Session Initiation Protocol (SIP) Call Control - Transfer
- `crates/session-core-v2/docs/THREE_TRANSFER_TYPES_DESIGN.md` - Overall transfer design
- `crates/dialog-core/src/manager/response_lifecycle.rs` - Response lifecycle hooks

## Notes

- This plan prioritizes **code reuse** over quick implementation
- Building the foundation correctly now saves refactoring later
- TransferCoordinator will be the core for all three transfer types
- SimplePeer provides zero-code experience while low-level API maintains flexibility
