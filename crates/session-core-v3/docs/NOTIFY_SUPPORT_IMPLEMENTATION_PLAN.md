# NOTIFY Support Implementation Plan for Dialog-Core and Session-Core Integration

**Status**: Ready for Implementation
**Priority**: CRITICAL - Required for RFC 3515/6665 Compliance
**Estimated Effort**: 4-6 hours
**Target**: Full NOTIFY support with Subscription-State headers across all SIP call scenarios

---

## Executive Summary

### Current State
- ✅ **session-core-v2**: 100% complete NOTIFY infrastructure for transfers
- ✅ **sip-core**: Fixed TypedHeader integration for Subscription-State (Phase 1 complete)
- ❌ **dialog-core**: NOTIFY builder missing Subscription-State header - **ROOT CAUSE**
- ❌ **Result**: Bob rejects Alice's NOTIFY messages → RFC 3515 non-compliant

### Problem Analysis

**Error**:
```
[BOB] ERROR: NOTIFY requires Subscription-State header
```

**Root Cause**:
- dialog-core's `InDialogRequestBuilder` only adds Event header to NOTIFY requests
- Subscription-State header is never added, despite being required by RFC 6665
- Event type is hardcoded to "dialog" instead of using the event parameter ("refer" for transfers)

**Impact**:
- All NOTIFY messages are rejected by receiving party
- Blind transfer NOTIFY progress messages don't work
- RFC 3515/6665 non-compliant
- Can't send presence NOTIFY, dialog NOTIFY, or any other NOTIFY types

---

## Architecture Overview

### Current Message Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Transfer Scenario                             │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  1. Bob sends REFER to Alice (transfer to Charlie)                  │
│     └─> dialog-core creates implicit subscription (RFC 3515)        │
│         - Event: "refer"                                             │
│         - Subscription-State: "active"                               │
│                                                                       │
│  2. Alice accepts transfer (202 Accepted)                           │
│                                                                       │
│  3. Alice calls Charlie                                             │
│     └─> When Charlie responds, Alice should send NOTIFY to Bob:     │
│         ✅ Event: refer                                              │
│         ❌ Subscription-State: active;expires=60 (MISSING!)         │
│         ✅ Body: "SIP/2.0 100 Trying"                               │
│                                                                       │
│  4. Bob receives NOTIFY                                             │
│     └─> dialog-core validates incoming NOTIFY:                      │
│         ✅ Checks Event header (present)                            │
│         ❌ Checks Subscription-State header (MISSING - REJECTED!)   │
│                                                                       │
└─────────────────────────────────────────────────────────────────────┘
```

### Required vs Current Behavior

**Required by RFC 6665**:
```sip
NOTIFY sip:bob@example.com SIP/2.0
Event: refer                              ← Present ✅
Subscription-State: active;expires=60     ← MISSING ❌
Content-Type: message/sipfrag;version=2.0
Content-Length: 20

SIP/2.0 100 Trying
```

**Currently Sent by dialog-core**:
```sip
NOTIFY sip:bob@example.com SIP/2.0
Event: refer                              ← Present ✅
Content-Type: message/sipfrag;version=2.0
Content-Length: 20

SIP/2.0 100 Trying
```

---

## Implementation Plan

### Phase 1: Fix NOTIFY Request Building (CRITICAL - 2 hours)

#### Task 1.1: Add Subscription-State Support to InDialogRequestBuilder

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/client/builders.rs`

**Changes**:

1. **Add field to struct** (around line 772):
```rust
pub struct InDialogRequestBuilder {
    method: Method,
    body: Option<String>,
    content_type: Option<String>,
    event_type: Option<String>,
    subscription_state: Option<String>,  // ← NEW: For NOTIFY Subscription-State header
    // ... existing fields
}
```

2. **Add builder method** (around line 884):
```rust
/// Set the Subscription-State for NOTIFY requests (RFC 6665)
///
/// # Examples
/// - "active;expires=3600" - Active subscription
/// - "pending" - Pending subscription
/// - "terminated;reason=noresource" - Terminated subscription
pub fn with_subscription_state(mut self, state: impl Into<String>) -> Self {
    self.subscription_state = Some(state.into());
    self
}
```

3. **Update build() method to add header** (after line 919, after Event header):
```rust
// Add Event header for NOTIFY and SUBSCRIBE
if let Some(event) = &self.event_type {
    let event_header = Event::new(EventType::Token(event.clone()));
    builder = builder.header(TypedHeader::Event(event_header));
}

// ← ADD THIS: Add Subscription-State header for NOTIFY (RFC 6665)
if let Some(sub_state) = &self.subscription_state {
    use std::str::FromStr;
    use rvoip_sip_core::types::subscription_state::SubscriptionState as SubStateHeader;

    match SubStateHeader::from_str(sub_state) {
        Ok(state_header) => {
            builder = builder.header(TypedHeader::SubscriptionState(state_header));
        }
        Err(e) => {
            warn!("Failed to parse Subscription-State '{}': {}", sub_state, e);
            return Err(Error::Other(format!("Invalid Subscription-State: {}", e)));
        }
    }
}
```

4. **Update for_notify factory** (line 1004):
```rust
pub fn for_notify(event: impl Into<String>, body: Option<String>) -> Self {
    let mut builder = Self::new(Method::Notify)
        .with_event(event);

    if let Some(notification_body) = body {
        builder = builder.with_body(notification_body);
    }

    builder
    // Note: Subscription-State will be added later via with_subscription_state()
}
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib builders
```

---

#### Task 1.2: Update notify_for_dialog Helper Function

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/dialog/quick.rs`

**Changes**:

1. **Add subscription_state parameter** (line 374):
```rust
pub fn notify_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    event_type: impl Into<String>,
    notification_body: Option<String>,
    subscription_state: Option<String>,  // ← NEW PARAMETER
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>
) -> Result<Request>
```

2. **Pass subscription_state to builder** (around line 403):
```rust
let mut builder = InDialogRequestBuilder::for_notify(event_type, notification_body);

// Add subscription state if provided (required for RFC 6665 compliance)
if let Some(state) = subscription_state {
    builder = builder.with_subscription_state(state);
}

builder.from_dialog_enhanced(
    call_id,
    from_uri,
    from_tag,
    to_uri,
    to_tag,
    cseq,
    local_address,
    route_set
)?
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib quick
```

---

#### Task 1.3: Fix Request Building to Use Dialog Event Package and State

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/manager/transaction/request_operations.rs`

**Changes**:

1. **Extract dialog fields BEFORE building NOTIFY** (lines 320-339):
```rust
Method::Notify => {
    let remote_tag = remote_tag.ok_or_else(|| {
        crate::errors::DialogError::protocol_error(
            "NOTIFY request requires remote tag in established dialog"
        )
    })?;

    // Get event package from dialog (not hardcoded!)
    let event_type = match &dialog.event_package {
        Some(pkg) => pkg.clone(),
        None => {
            return Err(crate::errors::DialogError::protocol_error(
                "NOTIFY request requires event_package to be set on dialog"
            ));
        }
    };

    // Get subscription state from dialog for RFC 6665 compliance
    let subscription_state = dialog.subscription_state
        .as_ref()
        .map(|s| s.to_header_value());

    dialog_quick::notify_for_dialog(
        &template.call_id,
        &template.local_uri.to_string(),
        &local_tag,
        &template.remote_uri.to_string(),
        &remote_tag,
        event_type,  // ← Use dialog's event_package, not hardcoded "dialog"
        body_string,
        subscription_state,  // ← NEW: Pass subscription state from dialog
        template.cseq_number,
        self.local_address,
        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
    )
}
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib request_operations
```

---

#### Task 1.4: Add Helper Method to Dialog for Subscription-State

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/dialog/dialog_impl.rs`

**Changes**:

1. **Add helper method** (around line 400+):
```rust
/// Get the current Subscription-State as a header value string
///
/// Returns None if this dialog doesn't have an active subscription.
/// Used when building NOTIFY requests to include the required Subscription-State header.
pub fn get_subscription_state_header(&self) -> Option<String> {
    self.subscription_state.as_ref().map(|s| s.to_header_value())
}
```

**Note**: The SubscriptionState type in `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/dialog/subscription_state.rs` needs to implement `to_header_value()`:

```rust
impl SubscriptionState {
    /// Convert to header value format (e.g., "active;expires=3600")
    pub fn to_header_value(&self) -> String {
        match self {
            SubscriptionState::Active { expires } => {
                if let Some(exp) = expires {
                    format!("active;expires={}", exp)
                } else {
                    "active".to_string()
                }
            }
            SubscriptionState::Pending => "pending".to_string(),
            SubscriptionState::Terminated { reason, retry_after } => {
                let mut s = "terminated".to_string();
                if let Some(r) = reason {
                    s.push_str(&format!(";reason={}", r));
                }
                if let Some(retry) = retry_after {
                    s.push_str(&format!(";retry-after={}", retry));
                }
                s
            }
        }
    }
}
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib dialog_impl
```

---

### Phase 2: Update Public API (1 hour)

#### Task 2.1: Enhance send_notify API Signature

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/manager/unified.rs`

**Changes**:

1. **Update send_notify signature** (line 744):
```rust
/// Send a NOTIFY request within a dialog
///
/// # Arguments
/// * `dialog_id` - The dialog to send NOTIFY in
/// * `event` - Event package (e.g., "refer", "presence", "dialog")
/// * `body` - Optional notification body
/// * `subscription_state` - Optional subscription state (e.g., "active;expires=3600")
///                          If None, uses dialog's current subscription_state
pub async fn send_notify(
    &self,
    dialog_id: &DialogId,
    event: String,
    body: Option<String>,
    subscription_state: Option<String>  // ← NEW: Allow explicit state override
) -> ApiResult<TransactionKey> {
    debug!("Sending NOTIFY for event: {} with state: {:?}", event, subscription_state);

    // If subscription_state not provided, will use dialog's subscription_state in request_operations.rs
    // Store event in dialog's event_package if not already set

    let notify_body = body.map(|b| bytes::Bytes::from(b));
    self.send_request_in_dialog(dialog_id, Method::Notify, notify_body).await
}
```

**Issue**: This approach has a problem - we need to pass `event` and `subscription_state` through `send_request_in_dialog`, but it only takes `Method` and `body`.

**Better Solution**: Store event and subscription_state in the dialog BEFORE calling send_request_in_dialog:

```rust
pub async fn send_notify(
    &self,
    dialog_id: &DialogId,
    event: String,
    body: Option<String>,
    subscription_state: Option<String>
) -> ApiResult<TransactionKey> {
    debug!("Sending NOTIFY for event: {} with state: {:?}", event, subscription_state);

    // Update dialog's event_package and subscription_state before building request
    {
        let mut dialog = self.get_dialog_mut(dialog_id).await?;

        // Set event package if not already set or if different
        if dialog.event_package.as_ref() != Some(&event) {
            dialog.event_package = Some(event.clone());
        }

        // Set subscription state if provided
        if let Some(state_str) = subscription_state {
            use crate::dialog::subscription_state::SubscriptionState;

            // Parse state string to SubscriptionState enum
            dialog.subscription_state = Some(SubscriptionState::parse(&state_str)?);
        }
    }

    let notify_body = body.map(|b| bytes::Bytes::from(b));
    self.send_request_in_dialog(dialog_id, Method::Notify, notify_body).await
}
```

2. **Add convenience method for REFER NOTIFY** (new method):
```rust
/// Send NOTIFY for REFER implicit subscription (RFC 3515)
///
/// Automatically sets Event: refer and appropriate Subscription-State based on status code
///
/// # Arguments
/// * `dialog_id` - The dialog with the implicit REFER subscription
/// * `status_code` - SIP status code to report (100, 180, 200, etc.)
/// * `reason` - Reason phrase for the status
pub async fn send_refer_notify(
    &self,
    dialog_id: &DialogId,
    status_code: u16,
    reason: &str
) -> ApiResult<TransactionKey> {
    // RFC 3515: REFER creates implicit subscription that terminates after final response
    let subscription_state = if status_code >= 200 {
        "terminated;reason=noresource".to_string()  // Final response terminates subscription
    } else {
        "active;expires=60".to_string()  // Provisional response keeps subscription active
    };

    // Body is sipfrag format per RFC 3515
    let sipfrag_body = format!("SIP/2.0 {} {}", status_code, reason);

    self.send_notify(
        dialog_id,
        "refer".to_string(),
        Some(sipfrag_body),
        Some(subscription_state)
    ).await
}
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib unified
```

---

#### Task 2.2: Update Client/Server API Methods

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/api/client.rs`

**Changes**:

1. **Update send_notify** (around line 200+):
```rust
pub async fn send_notify(
    &self,
    event: impl Into<String>,
    body: Option<String>,
    subscription_state: Option<String>  // ← NEW
) -> ApiResult<TransactionKey> {
    self.dialog_handle.send_notify(event.into(), body, subscription_state).await
}

pub async fn send_refer_notify(
    &self,
    status_code: u16,
    reason: &str
) -> ApiResult<TransactionKey> {
    self.dialog_handle.send_refer_notify(status_code, reason).await
}
```

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/api/common.rs`

**Changes**:

1. **Update DialogHandle methods** (similar pattern):
```rust
pub async fn send_notify(
    &self,
    event: String,
    body: Option<String>,
    subscription_state: Option<String>
) -> ApiResult<TransactionKey> {
    // Forward to manager
    self.manager.send_notify(&self.dialog_id, event, body, subscription_state).await
}

pub async fn send_refer_notify(
    &self,
    status_code: u16,
    reason: &str
) -> ApiResult<TransactionKey> {
    self.manager.send_refer_notify(&self.dialog_id, status_code, reason).await
}
```

**Testing**:
```bash
cargo test -p rvoip-dialog-core --lib api
```

---

### Phase 3: Add Validation (1 hour)

#### Task 3.1: Create NOTIFY Request Validator

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/validators.rs`

**Changes**:

1. **Add validator function** (new function):
```rust
use rvoip_sip_core::types::event::Event;
use rvoip_sip_core::types::subscription_state::SubscriptionState as SubscriptionStateHeader;

/// Validate that a NOTIFY request has all required headers per RFC 6665
///
/// NOTIFY requires:
/// - Event header (identifies the event package)
/// - Subscription-State header (indicates subscription lifecycle state)
///
/// # Errors
/// Returns error if either required header is missing
pub fn validate_notify_request(request: &Request) -> Result<()> {
    // Check Event header (RFC 6665 Section 8.1.1)
    if request.typed_header::<Event>().is_none() {
        return Err(Error::Other(
            "NOTIFY request missing required Event header (RFC 6665)".to_string()
        ));
    }

    // Check Subscription-State header (RFC 6665 Section 8.1.1)
    if request.typed_header::<SubscriptionStateHeader>().is_none() {
        return Err(Error::Other(
            "NOTIFY request missing required Subscription-State header (RFC 6665)".to_string()
        ));
    }

    Ok(())
}
```

2. **Call validator in send_request_in_dialog** - Update `manager/transaction_integration.rs`:
```rust
// Before sending, validate NOTIFY requests
if request.method() == &Method::Notify {
    crate::transaction::validators::validate_notify_request(&request)?;
}

// Send the request
self.transaction_layer.send_request(request, destination).await?;
```

**Testing**:
```bash
# Test that invalid NOTIFY is rejected
cargo test -p rvoip-dialog-core validate_notify
```

---

### Phase 4: Update Session-Core-V2 Integration (1 hour)

#### Task 4.1: Update TransferNotifyHandler to Use New API

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/src/transfer/notify.rs`

**Changes**:

1. **Use new send_notify signature** (update all calls around lines 50-150):
```rust
// OLD:
pub async fn notify_trying(&self, transferor_session_id: &SessionId) -> Result<()> {
    let sipfrag_body = "SIP/2.0 100 Trying".to_string();

    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_notify(
                &dialog_id,
                "refer".to_string(),
                Some(sipfrag_body)
            )
            .await?;
    }
    Ok(())
}

// NEW (Option 1 - use send_notify with explicit state):
pub async fn notify_trying(&self, transferor_session_id: &SessionId) -> Result<()> {
    let sipfrag_body = "SIP/2.0 100 Trying".to_string();

    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_notify(
                &dialog_id,
                "refer".to_string(),
                Some(sipfrag_body),
                Some("active;expires=60".to_string())  // ← NEW: Explicit state
            )
            .await?;
    }
    Ok(())
}

// NEW (Option 2 - use send_refer_notify convenience method) - RECOMMENDED:
pub async fn notify_trying(&self, transferor_session_id: &SessionId) -> Result<()> {
    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_refer_notify(&dialog_id, 100, "Trying")  // ← Use convenience method
            .await?;
    }
    Ok(())
}
```

2. **Update all NOTIFY methods** (notify_ringing, notify_success, notify_failure):
```rust
pub async fn notify_ringing(&self, transferor_session_id: &SessionId) -> Result<()> {
    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_refer_notify(&dialog_id, 180, "Ringing")
            .await?;
    }
    Ok(())
}

pub async fn notify_success(&self, transferor_session_id: &SessionId) -> Result<()> {
    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_refer_notify(&dialog_id, 200, "OK")  // Terminates subscription
            .await?;
    }
    Ok(())
}

pub async fn notify_failure(
    &self,
    transferor_session_id: &SessionId,
    status_code: u16,
    reason: String
) -> Result<()> {
    if let Some(dialog_id) = self.get_dialog_id(transferor_session_id).await {
        self.dialog_adapter
            .send_refer_notify(&dialog_id, status_code, &reason)  // Terminates subscription
            .await?;
    }
    Ok(())
}
```

**Testing**:
```bash
cargo test -p rvoip-session-core-v2 --lib notify
```

---

#### Task 4.2: Update DialogAdapter Wrapper

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/src/adapters/dialog_adapter.rs`

**Changes**:

1. **Update send_notify signature** (around line 250+):
```rust
pub async fn send_notify(
    &self,
    dialog_id: &DialogId,
    event: String,
    body: Option<String>,
    subscription_state: Option<String>  // ← NEW
) -> Result<TransactionKey> {
    self.manager
        .send_notify(dialog_id, event, body, subscription_state)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send NOTIFY: {}", e))
}

pub async fn send_refer_notify(
    &self,
    dialog_id: &DialogId,
    status_code: u16,
    reason: &str
) -> Result<TransactionKey> {
    self.manager
        .send_refer_notify(dialog_id, status_code, reason)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send REFER NOTIFY: {}", e))
}
```

**Testing**:
```bash
cargo test -p rvoip-session-core-v2 --lib dialog_adapter
```

---

### Phase 5: Testing and Validation (1-2 hours)

#### Task 5.1: Unit Tests for NOTIFY Building

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/tests/notify_builder_test.rs` (new file)

**Create comprehensive tests**:
```rust
#[cfg(test)]
mod notify_builder_tests {
    use rvoip_dialog_core::transaction::client::builders::InDialogRequestBuilder;
    use rvoip_sip_core::types::Method;

    #[test]
    fn test_notify_with_subscription_state() {
        let builder = InDialogRequestBuilder::for_notify("refer", Some("SIP/2.0 100 Trying".to_string()))
            .with_subscription_state("active;expires=60");

        let request = builder.from_dialog_enhanced(
            "call-123",
            "sip:alice@example.com",
            "tag-alice",
            "sip:bob@example.com",
            "tag-bob",
            1,
            "192.168.1.10:5060".parse().unwrap(),
            None
        ).unwrap();

        // Verify Event header
        let event = request.typed_header::<Event>().unwrap();
        assert_eq!(event.event_type.to_string(), "refer");

        // Verify Subscription-State header
        use rvoip_sip_core::types::subscription_state::SubscriptionState;
        let sub_state = request.typed_header::<SubscriptionState>().unwrap();
        assert!(matches!(sub_state.state, SubState::Active));
        assert_eq!(sub_state.expires, Some(60));
    }

    #[test]
    fn test_notify_terminated_subscription() {
        let builder = InDialogRequestBuilder::for_notify("refer", Some("SIP/2.0 200 OK".to_string()))
            .with_subscription_state("terminated;reason=noresource");

        let request = builder.from_dialog_enhanced(
            "call-123",
            "sip:alice@example.com",
            "tag-alice",
            "sip:bob@example.com",
            "tag-bob",
            2,
            "192.168.1.10:5060".parse().unwrap(),
            None
        ).unwrap();

        use rvoip_sip_core::types::subscription_state::{SubscriptionState, SubState, TerminationReason};
        let sub_state = request.typed_header::<SubscriptionState>().unwrap();
        assert!(matches!(sub_state.state, SubState::Terminated));
        assert_eq!(sub_state.reason, Some(TerminationReason::NoResource));
    }
}
```

**Run tests**:
```bash
cargo test -p rvoip-dialog-core notify_builder_tests
```

---

#### Task 5.2: Integration Test for Blind Transfer

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/tests/blind_transfer_notify_test.rs` (new file)

**Create end-to-end test**:
```rust
#[tokio::test]
async fn test_blind_transfer_with_notify() {
    // Setup 3 peers: Alice, Bob, Charlie
    // 1. Bob calls Alice
    // 2. Bob sends REFER to Alice (transfer to Charlie)
    // 3. Alice accepts REFER (202)
    // 4. Alice calls Charlie
    // 5. Verify Alice sends NOTIFY to Bob with:
    //    - Event: refer
    //    - Subscription-State: active;expires=60
    //    - Body: SIP/2.0 100 Trying
    // 6. Charlie answers
    // 7. Verify Alice sends final NOTIFY to Bob with:
    //    - Event: refer
    //    - Subscription-State: terminated;reason=noresource
    //    - Body: SIP/2.0 200 OK
    // 8. Verify Bob receives and accepts all NOTIFY messages
}
```

**Run test**:
```bash
cargo test -p rvoip-session-core-v2 test_blind_transfer_with_notify
```

---

#### Task 5.3: Run Blind Transfer Example

**Command**:
```bash
cd /Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/examples/blind_transfer
./run_blind_transfer.sh
```

**Expected Output**:
```
[ALICE] ✅ Sent NOTIFY (100 Trying) with Subscription-State: active;expires=60
[BOB]   ✅ Received NOTIFY (100 Trying) - Event: refer, State: active
[ALICE] ✅ Sent NOTIFY (180 Ringing) with Subscription-State: active;expires=60
[BOB]   ✅ Received NOTIFY (180 Ringing) - Event: refer, State: active
[ALICE] ✅ Sent NOTIFY (200 OK) with Subscription-State: terminated;reason=noresource
[BOB]   ✅ Received NOTIFY (200 OK) - Event: refer, State: terminated
[BOB]   ✅ Subscription terminated, transfer complete
```

---

### Phase 6: Documentation Updates (30 minutes)

#### Task 6.1: Update dialog-core README

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/README.md`

**Add section**:
```markdown
## NOTIFY Message Support (RFC 6665)

dialog-core provides full RFC 6665 compliance for NOTIFY messages:

### Sending NOTIFY

```rust
// Send NOTIFY with explicit subscription state
dialog_handle.send_notify(
    "refer",
    Some("SIP/2.0 100 Trying".to_string()),
    Some("active;expires=60".to_string())
).await?;

// Convenience method for REFER implicit subscriptions (RFC 3515)
dialog_handle.send_refer_notify(100, "Trying").await?;
dialog_handle.send_refer_notify(200, "OK").await?;  // Terminates subscription
```

### Required Headers

All NOTIFY messages must include:
- **Event**: Event package (e.g., "refer", "presence", "dialog")
- **Subscription-State**: Subscription lifecycle state
  - `active;expires=N` - Active subscription
  - `pending` - Pending subscription
  - `terminated;reason=X` - Terminated subscription

### Subscription States

- `active` - Subscription is active
- `pending` - Subscription pending approval
- `terminated` - Subscription ended
  - Reasons: `deactivated`, `probation`, `rejected`, `timeout`, `giveup`, `noresource`
```

---

#### Task 6.2: Update session-core-v2 README

**File**: `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/README.md`

**Add section**:
```markdown
## Blind Transfer (RFC 3515) - NOTIFY Support

Session-core-v2 automatically sends NOTIFY progress messages during blind transfers:

### Transfer Flow with NOTIFY

1. **Transferor sends REFER** → Creates implicit subscription
2. **Transferee accepts** → 202 Accepted
3. **Transferee calls target** → Sends NOTIFY updates to transferor:
   - `100 Trying` → `Subscription-State: active;expires=60`
   - `180 Ringing` → `Subscription-State: active;expires=60`
   - `200 OK` → `Subscription-State: terminated;reason=noresource`
4. **Subscription terminates** → Transfer complete

### Automatic NOTIFY Sending

Enable auto-transfer to get automatic NOTIFY messages:

```rust
peer.enable_auto_transfer();

// When REFER received, peer automatically:
// 1. Accepts REFER (202)
// 2. Calls transfer target
// 3. Sends NOTIFY for each call state change
// 4. Terminates subscription when call connects
```

### Manual NOTIFY Control

For custom transfer logic:

```rust
// Send NOTIFY manually
transfer_notify_handler.notify_trying(&transferor_session_id).await?;
transfer_notify_handler.notify_ringing(&transferor_session_id).await?;
transfer_notify_handler.notify_success(&transferor_session_id).await?;
```
```

---

## Summary and Timeline

### Implementation Phases

| Phase | Task | Time | Priority | Status |
|-------|------|------|----------|--------|
| 1.1 | Add Subscription-State to InDialogRequestBuilder | 45 min | CRITICAL | ⏳ Pending |
| 1.2 | Update notify_for_dialog helper | 30 min | CRITICAL | ⏳ Pending |
| 1.3 | Fix request_operations.rs to use dialog fields | 30 min | CRITICAL | ⏳ Pending |
| 1.4 | Add dialog helper for subscription state | 15 min | CRITICAL | ⏳ Pending |
| 2.1 | Update send_notify API | 30 min | HIGH | ⏳ Pending |
| 2.2 | Update client/server APIs | 30 min | HIGH | ⏳ Pending |
| 3.1 | Add NOTIFY validation | 1 hour | HIGH | ⏳ Pending |
| 4.1 | Update TransferNotifyHandler | 30 min | MEDIUM | ⏳ Pending |
| 4.2 | Update DialogAdapter | 15 min | MEDIUM | ⏳ Pending |
| 5.1 | Unit tests | 1 hour | MEDIUM | ⏳ Pending |
| 5.2 | Integration test | 1 hour | MEDIUM | ⏳ Pending |
| 5.3 | Run blind transfer example | 15 min | MEDIUM | ⏳ Pending |
| 6.1 | Update dialog-core README | 15 min | LOW | ⏳ Pending |
| 6.2 | Update session-core-v2 README | 15 min | LOW | ⏳ Pending |

**Total Estimated Time**: 6 hours 30 minutes

### Critical Path (Minimum for RFC Compliance)

**Phase 1 (2 hours)** - Must complete for basic NOTIFY support:
1. Add Subscription-State to builder (1.1)
2. Update notify_for_dialog (1.2)
3. Fix request building (1.3)
4. Add dialog helper (1.4)

### Success Criteria

- [ ] All NOTIFY messages include Subscription-State header
- [ ] Bob accepts Alice's NOTIFY messages without errors
- [ ] Blind transfer completes with full NOTIFY progress reporting
- [ ] Event package is dynamic (not hardcoded to "dialog")
- [ ] Subscription terminates properly after final response
- [ ] All tests pass
- [ ] Blind transfer example runs successfully
- [ ] README documentation updated

### Post-Implementation

**Benefits Achieved**:
- ✅ Full RFC 6665 compliance for NOTIFY
- ✅ Full RFC 3515 compliance for blind transfers
- ✅ Support for all event packages (refer, presence, dialog, etc.)
- ✅ Proper subscription lifecycle management
- ✅ Validation prevents invalid NOTIFY messages
- ✅ Clean API for NOTIFY in session-core-v2

**Future Enhancements** (Optional):
- Attended transfer support (RFC 3891)
- Presence NOTIFY support
- Dialog state NOTIFY support
- Message waiting indicator (MWI) NOTIFY

---

## Files Changed Summary

### dialog-core (8 files)
1. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/client/builders.rs` - Add Subscription-State support
2. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/dialog/quick.rs` - Update notify_for_dialog
3. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/manager/transaction/request_operations.rs` - Fix NOTIFY building
4. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/dialog/dialog_impl.rs` - Add helper method
5. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/dialog/subscription_state.rs` - Add to_header_value()
6. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/transaction/validators.rs` - Add validation
7. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/manager/unified.rs` - Update API
8. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/api/client.rs` - Update client API
9. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/src/api/common.rs` - Update DialogHandle

### session-core-v2 (2 files)
1. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/src/transfer/notify.rs` - Use new API
2. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/src/adapters/dialog_adapter.rs` - Update wrapper

### Tests (2 new files)
1. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/tests/notify_builder_test.rs` - Unit tests
2. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/tests/blind_transfer_notify_test.rs` - Integration test

### Documentation (2 files)
1. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/dialog-core/README.md` - Add NOTIFY section
2. `/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/README.md` - Add transfer NOTIFY section

---

## Implementation Order

**Recommended sequence for minimal disruption**:

1. **Start with dialog-core foundation** (Phase 1)
   - Builders, helpers, request building
   - Get NOTIFY messages sending with headers

2. **Add validation** (Phase 3)
   - Ensure no invalid NOTIFY can be sent
   - Catch issues early

3. **Update APIs** (Phase 2)
   - Convenience methods
   - Clean interface for session-core-v2

4. **Update session-core-v2** (Phase 4)
   - Use new APIs
   - Simplify transfer code

5. **Test thoroughly** (Phase 5)
   - Unit tests
   - Integration tests
   - Example validation

6. **Document** (Phase 6)
   - README updates
   - API documentation

---

## Risk Assessment

### Low Risk
- Adding fields to builders (backward compatible)
- Adding new methods (non-breaking)
- Validation (can be disabled for testing)

### Medium Risk
- Changing send_notify signature (API breaking)
  - Mitigation: Add new method, deprecate old one
- Changing notify_for_dialog signature (internal API)
  - Mitigation: Only used internally

### High Risk
- None identified

### Rollback Plan
- All changes are additive
- Can fall back to old send_notify signature
- Validation can be feature-flagged

---

## Questions and Decisions

### Q1: Should subscription_state be required or optional in send_notify?
**Decision**: Optional with fallback to dialog's subscription_state
- Allows flexibility
- Uses dialog state by default
- Can override when needed

### Q2: Should we add send_refer_notify convenience method?
**Decision**: Yes, add it
- RFC 3515 is common use case
- Simplifies client code
- Handles subscription termination correctly

### Q3: Should validation be strict or lenient?
**Decision**: Strict for outgoing, lenient for incoming
- Outgoing: Must have headers (we control this)
- Incoming: Already validated by dialog-core

### Q4: Should Event type be stored in dialog or passed each time?
**Decision**: Store in dialog.event_package, use in request building
- Follows existing pattern
- Consistent with subscription model
- Avoids passing through every layer

---

This implementation plan provides a complete roadmap to achieve full RFC 3515/6665 compliance for NOTIFY messages across the rvoip stack.
