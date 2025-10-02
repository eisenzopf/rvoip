# NOTIFY Request Support - Implementation Plan

**Date:** 2025-10-02
**Crate:** rvoip-sip-core
**Goal:** Complete NOTIFY request support for RFC 3515 (REFER) and RFC 6665 (Event Notifications)

---

## Current State Analysis

### ✅ What Already Exists

1. **Method Enum** (`src/types/method.rs`)
   - ✅ `Method::Notify` variant exists
   - ✅ String conversion: "NOTIFY" ↔ `Method::Notify`

2. **Event Header** (`src/types/event.rs`)
   - ✅ `Event` struct fully implemented
   - ✅ `EventType` enum: Token, Package
   - ✅ Parser: `parser::headers::event`
   - ✅ TypedHeaderTrait: ✅ Implemented
   - ✅ TypedHeader enum: ✅ `Event(EventTypeData)` integrated
   - ✅ Display trait: `event_type;id=value;param=value`

3. **Subscription-State Header** (`src/types/subscription_state.rs`)
   - ✅ `SubscriptionState` struct fully implemented
   - ✅ `SubState` enum: Active, Pending, Terminated
   - ✅ `TerminationReason` enum: Timeout, Rejected, NoResource, etc.
   - ✅ Helper constructors: `active()`, `pending()`, `terminated()`
   - ✅ TypedHeaderTrait: ✅ Implemented
   - ✅ Parser: Built-in via FromStr
   - ✅ Display trait: `state;expires=N;reason=X`

4. **Generic Request Builder** (`src/types/sip_request.rs`)
   - ✅ `Request::new(Method::Notify)`
   - ✅ `.with_header(TypedHeader::Event(...))`
   - ✅ `.with_header(TypedHeader::SubscriptionState(...))`
   - ✅ `.with_body(bytes)`

### ❌ What's Missing

1. **TypedHeader Integration for Subscription-State**
   - File: `src/types/headers/typed_header.rs`
   - Line 140: `SubscriptionState(String)` ← Just a placeholder!
   - Needs: `SubscriptionState(SubscriptionStateType)`

2. **Subscription-State Parser Integration**
   - File: `src/types/headers/typed_header.rs`
   - Missing: `HeaderName::SubscriptionState` case in `from_raw_header()`
   - Needs: Parse raw bytes → SubscriptionState type

3. **NOTIFY Request Builder Helper** (Optional but useful)
   - No dedicated builder like `NotifyBuilder::new()`
   - Would provide ergonomic API for common NOTIFY patterns

4. **Content-Type for message/sipfrag** (RFC 3515 specific)
   - Need to support `Content-Type: message/sipfrag;version=2.0`
   - Used for transfer NOTIFY bodies

---

## Implementation Plan

### Phase 1: Fix TypedHeader Integration (CRITICAL)

**Priority:** HIGH
**Effort:** 30 minutes
**Files Changed:** 1

#### File: `src/types/headers/typed_header.rs`

**Step 1.1: Add Import** (Line ~59)
```rust
use crate::types::event::{Event as EventTypeData}; // Already exists
use crate::types::subscription_state::SubscriptionState as SubscriptionStateType; // ADD THIS
```

**Step 1.2: Fix TypedHeader Enum** (Line 140)

**Current:**
```rust
SubscriptionState(String), // Placeholder for SubscriptionState header type
```

**Replace with:**
```rust
SubscriptionState(SubscriptionStateType), // Proper type from types::subscription_state
```

**Step 1.3: Update header_name() Match** (Line ~228)

**Should already be correct:**
```rust
TypedHeader::SubscriptionState(_) => HeaderName::SubscriptionState,
```

**Step 1.4: Update Display Implementation** (Line ~414)

**Current:**
```rust
TypedHeader::SubscriptionState(state) => write!(f, "{}: {}", HeaderName::SubscriptionState, state),
```

**Should work as-is** because SubscriptionState implements Display. But verify with:
```rust
TypedHeader::SubscriptionState(sub_state) => {
    write!(f, "{}: {}", HeaderName::SubscriptionState, sub_state)
}
```

**Step 1.5: Add from_raw_header() Parser** (After line ~1159)

**Location:** Find the match statement for `HeaderName::Event`, add after it:

```rust
HeaderName::SubscriptionState => {
    // Parse raw bytes to SubscriptionState
    let value_str = std::str::from_utf8(value_bytes)
        .map_err(|e| Error::ParseError(
            format!("Invalid UTF-8 in Subscription-State header value: {}", e)
        ))?;

    // Use the FromStr implementation from subscription_state.rs
    Ok(TypedHeader::SubscriptionState(
        SubscriptionStateType::from_str(value_str)?
    ))
}
```

**Step 1.6: Update typed_header() getter** (If exists around line ~298)

**Current (if exists):**
```rust
TypedHeader::SubscriptionState(h) if type_id_t == std::any::TypeId::of::<String>() =>
    Some(h as &dyn Any),
```

**Replace with:**
```rust
TypedHeader::SubscriptionState(h) if type_id_t == std::any::TypeId::of::<SubscriptionStateType>() =>
    Some(h as &dyn Any),
```

---

### Phase 2: NOTIFY Request Builder (Optional Enhancement)

**Priority:** MEDIUM
**Effort:** 1-2 hours
**Files Created:** 1-2

#### Option A: Simple Helper Functions

**File:** `src/types/sip_request.rs` (add to existing file)

```rust
impl Request {
    /// Create a NOTIFY request with Event and Subscription-State headers
    ///
    /// # Arguments
    /// * `event_package` - Event package name (e.g., "refer", "presence")
    /// * `subscription_state` - State of the subscription
    ///
    /// # Example
    /// ```rust
    /// use rvoip_sip_core::types::{Request, Method, SubscriptionState, TerminationReason};
    ///
    /// let notify = Request::notify(
    ///     "refer",
    ///     SubscriptionState::terminated(TerminationReason::NoResource)
    /// );
    /// ```
    pub fn notify(event_package: &str, subscription_state: SubscriptionState) -> Self {
        use crate::types::event::{Event, EventType};

        Self::new(Method::Notify)
            .with_header(TypedHeader::Event(
                Event::new(EventType::Token(event_package.to_string()))
            ))
            .with_header(TypedHeader::SubscriptionState(subscription_state))
    }

    /// Create a NOTIFY request for RFC 3515 blind transfer
    ///
    /// # Arguments
    /// * `sipfrag_body` - SIP status line (e.g., "SIP/2.0 100 Trying")
    ///
    /// # Example
    /// ```rust
    /// let notify = Request::notify_refer_progress("SIP/2.0 100 Trying");
    /// ```
    pub fn notify_refer_progress(sipfrag_body: &str) -> Self {
        use crate::types::event::{Event, EventType};
        use crate::types::content_type::ContentType;
        use crate::types::media_type::MediaType;

        Self::new(Method::Notify)
            .with_header(TypedHeader::Event(
                Event::new(EventType::Token("refer".to_string()))
            ))
            .with_header(TypedHeader::SubscriptionState(
                SubscriptionState::terminated(TerminationReason::NoResource)
            ))
            .with_header(TypedHeader::ContentType(
                ContentType::new(MediaType::new("message", "sipfrag"))
                    .with_param("version", "2.0")
            ))
            .with_body(sipfrag_body.as_bytes().to_vec())
    }

    /// Create a NOTIFY request for RFC 3515 transfer completion
    ///
    /// # Arguments
    /// * `status_code` - SIP status code (100, 180, 200, etc.)
    /// * `reason_phrase` - Reason phrase ("Trying", "Ringing", "OK")
    ///
    /// # Example
    /// ```rust
    /// let notify = Request::notify_refer_status(100, "Trying");
    /// ```
    pub fn notify_refer_status(status_code: u16, reason_phrase: &str) -> Self {
        let sipfrag = format!("SIP/2.0 {} {}", status_code, reason_phrase);
        Self::notify_refer_progress(&sipfrag)
    }
}
```

#### Option B: Dedicated Builder (More Ergonomic)

**File:** `src/builders/notify.rs` (NEW)

```rust
//! NOTIFY Request Builder
//!
//! Provides an ergonomic API for building NOTIFY requests per RFC 6665 and RFC 3515.

use crate::types::{Request, Method};
use crate::types::headers::TypedHeader;
use crate::types::event::{Event, EventType};
use crate::types::subscription_state::{SubscriptionState, TerminationReason};
use crate::types::content_type::ContentType;
use crate::types::media_type::MediaType;

/// Builder for NOTIFY requests
pub struct NotifyBuilder {
    request: Request,
}

impl NotifyBuilder {
    /// Create a new NOTIFY request builder
    pub fn new(event_package: &str) -> Self {
        Self {
            request: Request::new(Method::Notify)
                .with_header(TypedHeader::Event(
                    Event::new(EventType::Token(event_package.to_string()))
                ))
        }
    }

    /// Set the subscription state to active
    pub fn active(mut self, expires: u32) -> Self {
        self.request = self.request.with_header(
            TypedHeader::SubscriptionState(SubscriptionState::active(expires))
        );
        self
    }

    /// Set the subscription state to pending
    pub fn pending(mut self, expires: u32) -> Self {
        self.request = self.request.with_header(
            TypedHeader::SubscriptionState(SubscriptionState::pending(expires))
        );
        self
    }

    /// Set the subscription state to terminated
    pub fn terminated(mut self, reason: TerminationReason) -> Self {
        self.request = self.request.with_header(
            TypedHeader::SubscriptionState(SubscriptionState::terminated(reason))
        );
        self
    }

    /// Set the body content
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.request = self.request.with_body(body);
        self
    }

    /// Set sipfrag body (for RFC 3515 transfer notifications)
    pub fn with_sipfrag(mut self, status_code: u16, reason_phrase: &str) -> Self {
        let sipfrag = format!("SIP/2.0 {} {}", status_code, reason_phrase);

        self.request = self.request
            .with_header(TypedHeader::ContentType(
                ContentType::new(MediaType::new("message", "sipfrag"))
                    .with_param("version", "2.0")
            ))
            .with_body(sipfrag.into_bytes());

        self
    }

    /// Build the final Request
    pub fn build(self) -> Request {
        self.request
    }
}

/// Convenience constructors for common NOTIFY patterns
impl NotifyBuilder {
    /// Create a NOTIFY for RFC 3515 blind transfer (100 Trying)
    pub fn refer_trying() -> Request {
        Self::new("refer")
            .terminated(TerminationReason::NoResource)
            .with_sipfrag(100, "Trying")
            .build()
    }

    /// Create a NOTIFY for RFC 3515 blind transfer (180 Ringing)
    pub fn refer_ringing() -> Request {
        Self::new("refer")
            .terminated(TerminationReason::NoResource)
            .with_sipfrag(180, "Ringing")
            .build()
    }

    /// Create a NOTIFY for RFC 3515 blind transfer (200 OK)
    pub fn refer_success() -> Request {
        Self::new("refer")
            .terminated(TerminationReason::NoResource)
            .with_sipfrag(200, "OK")
            .build()
    }

    /// Create a NOTIFY for RFC 3515 blind transfer failure
    pub fn refer_failure(status_code: u16, reason: &str) -> Request {
        Self::new("refer")
            .terminated(TerminationReason::NoResource)
            .with_sipfrag(status_code, reason)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_builder_refer_trying() {
        let notify = NotifyBuilder::refer_trying();

        assert_eq!(notify.method(), &Method::Notify);
        // Additional assertions for headers and body
    }

    #[test]
    fn test_notify_builder_custom() {
        let notify = NotifyBuilder::new("presence")
            .active(3600)
            .build();

        assert_eq!(notify.method(), &Method::Notify);
    }
}
```

**File:** `src/builders/mod.rs` (Update)
```rust
pub mod notify;  // ADD THIS LINE
```

---

### Phase 3: Content-Type Support for message/sipfrag

**Priority:** MEDIUM
**Effort:** 30 minutes
**Files Changed:** 1

#### File: `src/types/content_type.rs`

**Check if MediaType supports "message" type:**

```rust
// Should already work via:
ContentType::new(MediaType::new("message", "sipfrag"))
    .with_param("version", "2.0")
```

**If not, verify:**
1. MediaType constructor accepts arbitrary type/subtype strings
2. Parameters can be added via `.with_param()`

**Example Usage:**
```rust
use rvoip_sip_core::types::{ContentType, MediaType};

let sipfrag_content_type = ContentType::new(
    MediaType::new("message", "sipfrag")
).with_param("version", "2.0");

// Should produce: "Content-Type: message/sipfrag;version=2.0"
```

---

## Testing Strategy

### Unit Tests

#### File: `src/types/headers/typed_header.rs` (add tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::subscription_state::{SubscriptionState, TerminationReason};

    #[test]
    fn test_subscription_state_typed_header() {
        // Create SubscriptionState
        let sub_state = SubscriptionState::terminated(TerminationReason::NoResource);

        // Wrap in TypedHeader
        let typed = TypedHeader::SubscriptionState(sub_state);

        // Convert to string
        let header_str = typed.to_string();
        assert_eq!(header_str, "Subscription-State: terminated;reason=noresource");
    }

    #[test]
    fn test_parse_subscription_state_from_raw() {
        let raw_header = Header::new(
            HeaderName::SubscriptionState,
            HeaderValue::Raw(b"active;expires=3600".to_vec())
        );

        let typed = TypedHeader::from_raw_header(&raw_header).unwrap();

        match typed {
            TypedHeader::SubscriptionState(sub_state) => {
                assert_eq!(sub_state.state, SubState::Active);
                assert_eq!(sub_state.expires, Some(3600));
            }
            _ => panic!("Wrong variant"),
        }
    }
}
```

#### File: `src/builders/notify.rs` (if Option B chosen)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_builder_refer_trying() {
        let notify = NotifyBuilder::refer_trying();

        assert_eq!(notify.method(), &Method::Notify);

        // Check Event header
        let event = notify.typed_header::<Event>().unwrap();
        assert_eq!(event.to_string(), "refer");

        // Check Subscription-State header
        let sub_state = notify.typed_header::<SubscriptionState>().unwrap();
        assert_eq!(sub_state.state, SubState::Terminated);

        // Check body
        let body = String::from_utf8(notify.body().unwrap().to_vec()).unwrap();
        assert_eq!(body, "SIP/2.0 100 Trying");
    }

    #[test]
    fn test_notify_builder_custom_event() {
        let notify = NotifyBuilder::new("presence")
            .active(3600)
            .build();

        let event = notify.typed_header::<Event>().unwrap();
        assert_eq!(event.to_string(), "presence");
    }
}
```

### Integration Tests

#### File: `tests/notify_rfc3515_test.rs` (NEW)

```rust
//! RFC 3515 NOTIFY compliance tests

use rvoip_sip_core::types::{Request, Method, SubscriptionState, TerminationReason};
use rvoip_sip_core::types::event::{Event, EventType};
use rvoip_sip_core::types::headers::TypedHeader;

#[test]
fn test_rfc3515_notify_100_trying() {
    // Build NOTIFY per RFC 3515 section 2.4.5
    let notify = Request::new(Method::Notify)
        .with_header(TypedHeader::Event(
            Event::new(EventType::Token("refer".to_string()))
        ))
        .with_header(TypedHeader::SubscriptionState(
            SubscriptionState::terminated(TerminationReason::NoResource)
        ))
        .with_body(b"SIP/2.0 100 Trying".to_vec());

    assert_eq!(notify.method(), &Method::Notify);

    // Verify Event header
    let event = notify.typed_header::<Event>().unwrap();
    assert!(event.to_string().contains("refer"));

    // Verify Subscription-State header
    let sub_state = notify.typed_header::<SubscriptionState>().unwrap();
    assert_eq!(sub_state.reason, Some(TerminationReason::NoResource));

    // Verify body
    let body = String::from_utf8(notify.body().unwrap().to_vec()).unwrap();
    assert_eq!(body, "SIP/2.0 100 Trying");
}

#[test]
fn test_rfc3515_notify_200_ok() {
    let notify = Request::new(Method::Notify)
        .with_header(TypedHeader::Event(
            Event::new(EventType::Token("refer".to_string()))
        ))
        .with_header(TypedHeader::SubscriptionState(
            SubscriptionState::terminated(TerminationReason::NoResource)
        ))
        .with_body(b"SIP/2.0 200 OK".to_vec());

    let body = String::from_utf8(notify.body().unwrap().to_vec()).unwrap();
    assert_eq!(body, "SIP/2.0 200 OK");
}
```

---

## Implementation Checklist

### Phase 1: TypedHeader Integration (REQUIRED)
- [ ] Add SubscriptionState import to typed_header.rs
- [ ] Change `SubscriptionState(String)` to `SubscriptionState(SubscriptionStateType)`
- [ ] Add `HeaderName::SubscriptionState` parser in from_raw_header()
- [ ] Update typed_header() getter if needed
- [ ] Verify Display implementation works
- [ ] Test: Parse raw header → SubscriptionState type
- [ ] Test: SubscriptionState → String display

### Phase 2: Builder Helpers (OPTIONAL)
Choose Option A or B:
- [ ] **Option A**: Add helper methods to Request impl
  - [ ] `Request::notify(event, state)`
  - [ ] `Request::notify_refer_progress(sipfrag)`
  - [ ] `Request::notify_refer_status(code, reason)`
- [ ] **Option B**: Create NotifyBuilder
  - [ ] Create `src/builders/notify.rs`
  - [ ] Implement NotifyBuilder with fluent API
  - [ ] Add convenience constructors (refer_trying, refer_success, etc.)
  - [ ] Update `src/builders/mod.rs`

### Phase 3: Testing
- [ ] Unit tests for TypedHeader::SubscriptionState
- [ ] Unit tests for NotifyBuilder (if implemented)
- [ ] Integration tests for RFC 3515 compliance
- [ ] Verify message/sipfrag Content-Type works

### Phase 4: Documentation
- [ ] Update CHANGELOG.md with NOTIFY support
- [ ] Add examples to src/types/subscription_state.rs
- [ ] Add examples to builders/notify.rs (if created)
- [ ] Update main README with NOTIFY usage

---

## Usage Examples (After Implementation)

### Example 1: Manual NOTIFY Construction
```rust
use rvoip_sip_core::types::{Request, Method};
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::event::{Event, EventType};
use rvoip_sip_core::types::subscription_state::{SubscriptionState, TerminationReason};

let notify = Request::new(Method::Notify)
    .with_header(TypedHeader::Event(
        Event::new(EventType::Token("refer".to_string()))
    ))
    .with_header(TypedHeader::SubscriptionState(
        SubscriptionState::terminated(TerminationReason::NoResource)
    ))
    .with_body(b"SIP/2.0 100 Trying".to_vec());
```

### Example 2: Using Request Helper (Option A)
```rust
use rvoip_sip_core::types::{Request, SubscriptionState, TerminationReason};

let notify = Request::notify_refer_status(100, "Trying");
```

### Example 3: Using NotifyBuilder (Option B)
```rust
use rvoip_sip_core::builders::NotifyBuilder;

let notify = NotifyBuilder::refer_trying();  // Shortcut for 100 Trying

// Or custom:
let notify = NotifyBuilder::new("refer")
    .terminated(TerminationReason::NoResource)
    .with_sipfrag(180, "Ringing")
    .build();
```

---

## Downstream Impact

### Dialog-Core Changes After SIP-Core Fix

Once sip-core TypedHeader is fixed, dialog-core can use it:

**File:** `crates/dialog-core/src/api/common.rs`

**Current send_notify:**
```rust
pub async fn send_notify(&self, event: String, body: Option<String>) -> ApiResult<TransactionKey> {
    info!("Sending NOTIFY for dialog {} event {}", self.dialog_id, event);

    let notify_body = body.map(|b| bytes::Bytes::from(b));
    self.send_request_with_key(Method::Notify, notify_body).await
}
```

**Enhanced send_notify (after sip-core fix):**
```rust
pub async fn send_notify(
    &self,
    event: String,
    body: Option<String>,
    subscription_state: SubscriptionState  // NEW parameter
) -> ApiResult<TransactionKey> {
    info!("Sending NOTIFY for dialog {} event {}", self.dialog_id, event);

    // Build request with proper headers
    let mut request = Request::new(Method::Notify)
        .with_header(TypedHeader::Event(
            Event::new(EventType::Token(event))
        ))
        .with_header(TypedHeader::SubscriptionState(subscription_state));

    if let Some(body_str) = body {
        request = request.with_body(body_str.into_bytes());
    }

    // Send via transaction manager
    self.send_request_internal(request).await
}
```

---

## Estimated Effort

| Phase | Task | Time | Priority |
|-------|------|------|----------|
| 1 | Fix TypedHeader integration | 30 min | CRITICAL |
| 2A | Add Request helper methods | 30 min | OPTIONAL |
| 2B | Create NotifyBuilder | 1-2 hours | OPTIONAL |
| 3 | Content-Type verification | 15 min | LOW |
| 4 | Unit tests | 1 hour | HIGH |
| 5 | Integration tests | 30 min | MEDIUM |
| 6 | Update README.md | 30 min | HIGH |
| 7 | Documentation | 30 min | MEDIUM |

**Total (Minimum):** 3 hours (Phase 1 + tests + README)
**Total (With Builder):** 4.5-5.5 hours (All phases)

---

## Success Criteria

### Must Have ✅
- [ ] TypedHeader::SubscriptionState uses proper type (not String)
- [ ] SubscriptionState header parses correctly from raw bytes
- [ ] Can build NOTIFY with Event + Subscription-State headers
- [ ] All tests pass
- [ ] Dialog-core can use the enhanced API
- [ ] README.md updated to reflect NOTIFY/SUBSCRIBE support

### Nice to Have 🎯
- [ ] NotifyBuilder with fluent API
- [ ] Convenience methods for RFC 3515 REFER notifications
- [ ] Comprehensive examples in documentation

---

## Next Steps

1. **Review this plan** - Confirm approach and priority
2. **Implement Phase 1** - Critical TypedHeader fix (30 min)
3. **Test Phase 1** - Verify parsing works
4. **Decide on Phase 2** - Choose Option A (simple) or Option B (builder)
5. **Implement chosen option** - 30 min to 2 hours
6. **Update dialog-core** - Use new sip-core capabilities
7. **End-to-end test** - Verify RFC 3515 blind transfer works

---

**Document Version:** 1.0
**Last Updated:** 2025-10-02
**Author:** Claude (AI Assistant)
**Status:** Ready for Review
