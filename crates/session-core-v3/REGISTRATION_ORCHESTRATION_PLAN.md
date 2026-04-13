# Registration Orchestration Implementation Plan

**Purpose:** Implement event-based orchestration for server-side REGISTER handling  
**Scope:** session-core-v3 coordinates between dialog-core and registrar-core via event bus  
**Estimated Time:** 4-6 hours  
**Status:** UPDATED - Always compile server-side (no feature flags)

---

## Executive Summary

Implement the proper event-driven architecture where **session-core-v3 orchestrates** authentication between dialog-core (protocol layer) and registrar-core (storage/validation layer).

### The Flow We're Implementing:

```
Network → dialog-core → Event Bus → session-core-v3 → registrar-core
         ← dialog-core ← Event Bus ← session-core-v3 ←
```

---

## Current Status

### ✅ What's Already Done:

1. **Event Definitions** - infra-common/src/events/cross_crate.rs
   - `DialogToSessionEvent::IncomingRegister` ✅
   - `SessionToDialogEvent::SendRegisterResponse` ✅

2. **dialog-core Emission** - dialog-core/src/protocol/register_handler.rs
   - Publishes `IncomingRegister` to global bus ✅
   - Extracts Authorization header ✅

3. **registrar-core Logic** - registrar-core/src/api/mod.rs
   - `authenticate_register()` method exists ✅
   - Returns (should_register, www_auth_challenge) ✅

### ❌ What's Missing:

1. **session-core-v3 Subscription** - Doesn't subscribe to `IncomingRegister` events
2. **session-core-v3 Orchestration** - Doesn't call registrar-core
3. **session-core-v3 Response Publishing** - Doesn't publish `SendRegisterResponse`
4. **dialog-core Response Handler** - Doesn't subscribe to `SendRegisterResponse`
5. **dialog-core SIP Response** - Doesn't send actual 401/200 based on events

---

## Implementation Plan

### Step 1: Recreate registration_adapter.rs (1.5 hours)

**File:** `crates/session-core-v3/src/adapters/registration_adapter.rs`

**Purpose:** Subscribe to IncomingRegister events, orchestrate auth, publish responses

**Code Structure:**
```rust
//! Server-side REGISTER request handler
//!
//! Orchestrates authentication between dialog-core and registrar-core

use std::sync::Arc;
use tracing::{info, warn, debug};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent},
};
use rvoip_registrar_core::{RegistrarService, ContactInfo, Transport};
use crate::errors::Result;

pub struct RegistrationAdapter {
    registrar: Arc<RegistrarService>,
    global_coordinator: Arc<GlobalEventCoordinator>,
}

impl RegistrationAdapter {
    pub fn new(
        registrar: Arc<RegistrarService>,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Self {
        Self { registrar, global_coordinator }
    }
    
    async fn handle_incoming_register(
        &self,
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
    ) -> Result<()> {
        // Extract username
        let username = extract_username(&from_uri)?;
        
        // Call registrar-core to authenticate
        let (should_register, www_auth) = self.registrar
            .authenticate_register(&username, authorization.as_deref(), "REGISTER", &from_uri)
            .await?;
        
        if should_register {
            // Build ContactInfo and register
            let contact = build_contact_info(&contact_uri, expires);
            self.registrar.register_user(&username, contact, Some(expires)).await?;
            
            // Publish 200 OK response event
            publish_200_ok(&self.global_coordinator, transaction_id, contact_uri, expires).await?;
        } else {
            // Publish 401 challenge event
            publish_401_challenge(&self.global_coordinator, transaction_id, www_auth).await?;
        }
        
        Ok(())
    }
    
    pub async fn start(self: Arc<Self>) -> Result<()> {
        // Subscribe to global event bus
        // Listen for DialogToSessionEvent::IncomingRegister
        // Call handle_incoming_register for each event
        // (Implementation details in full code)
    }
}
```

**Key Fixes from Previous Attempt:**
- Use correct subscriber API from GlobalEventCoordinator
- Handle event downcasting properly
- Build ContactInfo with all required fields (not missing any)
- Always compile (no feature flags - registrar-core is always available)

**Dependencies:**
- Add `rvoip-registrar-core` to session-core-v3 Cargo.toml (regular dependency)
- `chrono` and `uuid` already present (for ContactInfo fields)

---

### Step 2: Add Response Handler to dialog-core (2 hours)

**File:** `crates/dialog-core/src/manager/event_processing.rs` (NEW or add to existing)

**Purpose:** Subscribe to SendRegisterResponse events, send SIP responses

**Code Structure:**
```rust
//! Event processing for dialog-core

use rvoip_infra_common::events::cross_crate::{RvoipCrossCrateEvent, SessionToDialogEvent};

pub struct SessionToDialogEventHandler {
    transaction_manager: Arc<TransactionManager>,
    global_coordinator: Arc<GlobalEventCoordinator>,
}

impl SessionToDialogEventHandler {
    pub async fn start(self: Arc<Self>) -> Result<()> {
        // Subscribe to global event bus
        let mut subscriber = self.global_coordinator
            .subscribe("rvoip_cross_crate_event")
            .await?;
        
        tokio::spawn(async move {
            loop {
                if let Some(event_arc) = subscriber.recv().await {
                    if let Some(event) = downcast_to_cross_crate(event_arc) {
                        if let RvoipCrossCrateEvent::SessionToDialog(
                            SessionToDialogEvent::SendRegisterResponse {
                                transaction_id,
                                status_code,
                                reason,
                                www_authenticate,
                                contact,
                                expires,
                            }
                        ) = event {
                            // Build SIP response
                            let response = build_register_response(
                                status_code,
                                reason,
                                www_authenticate,
                                contact,
                                expires,
                            );
                            
                            // Send via transaction manager
                            let tx_key = parse_transaction_key(&transaction_id);
                            self.transaction_manager.send_response(&tx_key, response).await;
                        }
                    }
                }
            }
        });
        
        Ok(())
    }
}
```

**Where to Add:**
- Option A: New file `dialog-core/src/manager/event_processing.rs`
- Option B: Add to `dialog-core/src/manager/unified.rs`
- Option C: Add to `dialog-core/src/api/unified.rs`

**Recommendation:** Option A (new file for clarity)

**Wire Up In:**
- `UnifiedDialogManager::with_global_events()` - Start the handler when creating manager

---

### Step 3: Update register_demo to Start Both (1 hour)

**File:** `crates/session-core-v3/examples/register_demo/main.rs`

**Changes:**

**Add at top:**
```rust
async fn start_embedded_registrar() -> Arc<RegistrarService> {
    // Create registrar with auth
    let registrar = RegistrarService::with_auth(
        ServiceMode::B2BUA,
        RegistrarConfig::default(),
        "test.local"
    ).await.expect("Failed to create registrar");
    
    // Add test user
    registrar.user_store().unwrap()
        .add_user("alice", "password123")
        .expect("Failed to add user");
    
    // Create and start RegistrationAdapter
    let adapter = Arc::new(RegistrationAdapter::new(
        registrar.clone(),
        global_coordinator().await.clone(),
    ));
    
    adapter.start().await.expect("Failed to start adapter");
    
    registrar
}
```

**In main():**
```rust
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()...;
    
    info!("Starting embedded registrar server...");
    let _registrar = start_embedded_registrar().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    info!("Creating coordinator...");
    let coordinator = UnifiedCoordinator::new(config).await?;
    
    // Rest of registration demo...
}
```

**Run with:**
```bash
cargo run --example register_demo
```

---

### Step 4: Wire Up dialog-core Event Handler (1 hour)

**File:** `crates/dialog-core/src/manager/unified.rs`

**In `with_global_events()` method:**
```rust
pub async fn with_global_events(...) -> DialogResult<Self> {
    // ... existing code ...
    
    // Start session-to-dialog event handler
    let event_handler = Arc::new(SessionToDialogEventHandler::new(
        core.transaction_manager.clone(),
        global_coordinator.clone(),
    ));
    
    event_handler.start().await?;
    
    Ok(Self { core, config, stats })
}
```

---

## File Inventory

### Files to Create:

1. **session-core-v3/src/adapters/registration_adapter.rs** (~200 lines)
   - RegistrationAdapter struct
   - handle_incoming_register() method
   - start() method with event subscription
   - Helper functions

2. **dialog-core/src/manager/event_processing.rs** (~150 lines)
   - SessionToDialogEventHandler struct  
   - start() method with event subscription
   - build_register_response() helper
   - parse_transaction_key() helper

### Files to Modify:

1. **session-core-v3/src/adapters/mod.rs**
   - Add `pub mod registration_adapter;`

2. **session-core-v3/Cargo.toml**
   - Add `rvoip-registrar-core = { path = "../registrar-core" }` (regular dependency)

3. **session-core-v3/examples/register_demo/main.rs**
   - Add embedded registrar startup (always included)

4. **dialog-core/src/manager/mod.rs**
   - Add `pub mod event_processing;`

5. **dialog-core/src/manager/unified.rs**
   - Start SessionToDialogEventHandler in with_global_events()

---

## Dependencies

### session-core-v3 Cargo.toml:
```toml
[dependencies]
rvoip-registrar-core = { path = "../registrar-core" }  # Regular dependency
chrono = "0.4"  # Already present
uuid = { version = "1.4", features = ["v4"] }  # Already present
```

### dialog-core Cargo.toml:
```toml
# No new dependencies needed - uses existing infra-common
```

---

## Testing Strategy

### Unit Tests:

**session-core-v3:**
```rust
#[tokio::test]
async fn test_registration_adapter_handles_event() {
    // Create adapter
    // Publish IncomingRegister event
    // Verify SendRegisterResponse published
}
```

### Integration Test:

**register_demo with embedded server:**
```bash
cargo run --example register_demo
```

**Expected Flow:**
1. Embedded registrar starts, subscribes to events
2. Client sends REGISTER to 127.0.0.1:5060
3. dialog-core receives, publishes IncomingRegister
4. session-core-v3 RegistrationAdapter receives event
5. Calls registrar.authenticate_register()
6. Publishes SendRegisterResponse (401)
7. dialog-core handles event, sends 401 with WWW-Authenticate
8. Client receives 401, computes digest
9. Client sends REGISTER with Authorization
10. dialog-core publishes IncomingRegister (with auth this time)
11. RegistrationAdapter validates
12. Publishes SendRegisterResponse (200)
13. dialog-core sends 200 OK
14. ✅ Registration complete!

---

## Implementation Sequence

### Phase 1: Core Infrastructure (2 hours)

**Task 1.1:** Recreate registration_adapter.rs
- Struct definition
- handle_incoming_register() method
- extract_username() helper
- build_contact_info() helper

**Task 1.2:** Fix event subscription
- Use correct GlobalEventCoordinator API
- Handle event downcasting
- Proper error handling

**Task 1.3:** Add as regular module
- No feature gates needed
- registrar-core is regular dependency

### Phase 2: dialog-core Response Handling (2 hours)

**Task 2.1:** Create event_processing.rs
- SessionToDialogEventHandler struct
- Event subscription logic
- Response building

**Task 2.2:** Implement response sending
- Parse transaction_id to TransactionKey
- Build SIP Response with proper headers
- Send via transaction_manager

**Task 2.3:** Wire up in UnifiedDialogManager
- Start handler in with_global_events()
- Ensure global_coordinator is available

### Phase 3: Integration (1-2 hours)

**Task 3.1:** Update register_demo
- Add embedded registrar startup (always included)
- Start RegistrationAdapter
- Single command runs complete demo

**Task 3.2:** Test end-to-end
- Run with embedded registrar
- Verify 401 challenge works
- Verify digest auth works
- Verify 200 OK works

---

## Key Implementation Details

### registration_adapter.rs Event Subscription:

```rust
pub async fn start(self: Arc<Self>) -> Result<()> {
    let global_coord = self.global_coordinator.clone();
    
    // Get subscriber from global coordinator
    let subscriber = global_coord
        .create_subscriber()
        .await
        .map_err(|e| SessionError::InternalError(format!("Subscribe failed: {}", e)))?;
    
    tokio::spawn(async move {
        loop {
            // Receive events from bus
            if let Some(event) = subscriber.receive_event().await {
                // Try to downcast to RvoipCrossCrateEvent
                if let Ok(cross_crate) = event.downcast::<RvoipCrossCrateEvent>() {
                    if let RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::IncomingRegister { ... }
                    ) = &*cross_crate {
                        // Handle it
                        self.handle_incoming_register(...).await;
                    }
                }
            }
        }
    });
    
    Ok(())
}
```

**Note:** The exact API depends on how GlobalEventCoordinator::subscribe() works. Will need to check infra-common documentation.

### ContactInfo Construction:

```rust
fn build_contact_info(contact_uri: &str, expires: u32) -> ContactInfo {
    ContactInfo {
        uri: contact_uri.to_string(),
        instance_id: uuid::Uuid::new_v4().to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-session-core-v3".to_string(),
        expires: chrono::Utc::now() + chrono::Duration::seconds(expires as i64),
        q_value: 1.0,
        received: None,
        path: Vec::new(),
        methods: vec!["INVITE".to_string(), "ACK".to_string(), "BYE".to_string()],
    }
}
```

### dialog-core Response Building:

```rust
fn build_register_response(
    status_code: u16,
    www_authenticate: Option<String>,
    contact: Option<String>,
    expires: Option<u32>,
) -> Response {
    let mut response = Response::new(StatusCode::from_u16(status_code));
    
    // Add WWW-Authenticate for 401
    if status_code == 401 {
        if let Some(www_auth) = www_authenticate {
            response.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(www_auth.into_bytes())
            ));
        }
    }
    
    // Add Contact and Expires for 200
    if status_code == 200 {
        if let Some(contact_uri) = contact {
            // Add Contact header
        }
        if let Some(exp) = expires {
            response.headers.push(TypedHeader::Expires(Expires(exp)));
        }
    }
    
    response
}
```

---

## Dependency Graph

### Architecture:
```
session-core-v3
  → dialog-core (sends REGISTER, handles protocol)
  → auth-core (computes digest auth)
  → registrar-core (validates credentials, stores registrations)
```

**Always Compiled:**
- Client-side registration (send REGISTER to external servers)
- Server-side orchestration (handle REGISTER from clients)
- Full bidirectional capability

---

## Testing Plan

### Unit Tests:

**Test 1:** RegistrationAdapter handles IncomingRegister
```rust
#[cfg(feature = "server-side-registration")]
#[tokio::test]
async fn test_adapter_handles_incoming_register() {
    // Publish IncomingRegister event
    // Verify SendRegisterResponse published
}
```

**Test 2:** dialog-core sends response on event
```rust
#[tokio::test]
async fn test_dialog_sends_response_on_event() {
    // Publish SendRegisterResponse event
    // Verify SIP response sent on transaction
}
```

### Integration Test:

**register_demo with embedded server:**
```bash
# Run with server support
cargo run --example register_demo --features server-side-registration

# Expected:
# 1. Embedded registrar starts
# 2. Client sends REGISTER
# 3. Server sends 401
# 4. Client computes digest
# 5. Client sends REGISTER with auth
# 6. Server validates and sends 200
# 7. ✅ Registration complete
```

---

## Risks and Mitigations

### Risk 1: GlobalEventCoordinator API
**Risk:** Subscribe API may not match assumptions  
**Mitigation:** Check infra-common/src/events/coordinator.rs for correct API  
**Time Impact:** +30 min

### Risk 2: Event Downcasting
**Risk:** Type downcasting may fail  
**Mitigation:** Use proper trait bounds and Any conversions  
**Time Impact:** +30 min

### Risk 3: Transaction Key Parsing
**Risk:** String transaction_id → TransactionKey conversion  
**Mitigation:** Use existing parsing utilities in dialog-core  
**Time Impact:** +30 min

### Risk 4: Response Header Construction
**Risk:** Complex header building in dialog-core  
**Mitigation:** Use existing response builders  
**Time Impact:** +1 hour

**Total Contingency:** +2.5 hours (worst case: 6-8.5 hours total)

---

## Success Criteria

### Must Have:
- [ ] registration_adapter.rs compiles without errors
- [ ] event_processing.rs compiles without errors
- [ ] register_demo runs with `--features server-side-registration`
- [ ] Full REGISTER flow completes (401 → auth → 200)
- [ ] All existing tests still pass

### Nice to Have:
- [ ] Unit tests for event handlers
- [ ] Performance metrics (latency of event flow)
- [ ] Comprehensive error handling

---

## Alternative Approach (If Event Bus is Complex)

### Direct Callback Pattern (Simpler, 2-3 hours):

Instead of event bus, add direct callbacks to dialog-core:

```rust
// In dialog-core
pub trait RegisterHandler {
    async fn handle_register(
        &self,
        from: String,
        contact: String,
        expires: u32,
        authorization: Option<String>,
    ) -> (u16, Option<String>);  // (status_code, www_authenticate)
}

dialog_server.set_register_handler(handler);
```

**Pros:** Simpler, faster to implement  
**Cons:** Not using event bus, tighter coupling

---

## Recommendation

**Proceed with event bus approach** (4-6 hours) because:
- ✅ Matches existing architecture (IncomingCall uses events)
- ✅ Loose coupling
- ✅ More flexible
- ✅ Events already defined (infra-common)
- ✅ dialog-core already publishes IncomingRegister

**The infrastructure is 60% done - just need to wire up the handlers.**

---

## Next Steps if Approved

1. Implement registration_adapter.rs with correct event subscription
2. Implement dialog-core event_processing.rs for response handling
3. Update register_demo with embedded registrar
4. Test end-to-end
5. Document and commit

**Estimated:** 4-6 hours focused work

---

## Configuration Summary

**Decisions Made:**
1. ✅ **Event bus approach** - Use adapters and global event coordinator
2. ✅ **Always compile** - registrar-core is regular dependency (not optional)
3. ✅ **Embedded in register_demo** - Single command runs complete flow

**Implementation:**
- registration_adapter.rs: Always compiled in session-core-v3
- register_demo: Always includes embedded registrar
- Single `cargo run --example register_demo` does everything

**Simplified from original plan:** No feature flags, no conditional compilation.

---

## Ready to Implement?

This plan implements full event-bus orchestration where session-core-v3 coordinates between dialog-core and registrar-core, exactly as you described.

**Approve to proceed with implementation?**

