# Server-Side REGISTER Handling - Implementation Plan

**Date:** October 16, 2025  
**Status:** NEEDS IMPLEMENTATION  
**Estimated:** 6-8 hours

---

## Current Situation

### What Works ✅
- **Client-side:** session-core-v3 can send REGISTER, handle 401, compute digest auth
- **Event bus:** Global event coordinator exists and is used for communication
- **Auth logic:** registrar-core has authentication validation code
- **Protocol:** dialog-core receives REGISTER and tries to emit events

### What's Missing ❌
- **Event definitions:** No event for incoming REGISTER from dialog-core → session-core-v3
- **Event definitions:** No event for session-core-v3 → dialog-core to send 401/200 response  
- **Handler:** session-core-v3 doesn't subscribe to or handle incoming REGISTER
- **Integration:** No code path from received REGISTER → authentication → response

---

## Architecture Assessment

### Existing Pattern (for INVITE):

**Dialog-core → Session-core:**
```rust
DialogToSessionEvent::IncomingCall {
    session_id: String,
    call_id: String,
    from: String,
    to: String,
    transaction_id: String,  // ✅ For sending response!
    source_addr: String,
}
```

**Session-core → Dialog-core:**
Uses `ReferResponse` pattern:
```rust
SessionToDialogEvent::ReferResponse {
    transaction_id: String,  // ✅ Tells dialog-core which transaction
    accept: bool,
    status_code: u16,
    reason: String,
}
```

### What We Need (for REGISTER):

**Dialog-core → Session-core: ❌ MISSING**
```rust
DialogToSessionEvent::IncomingRegister {
    transaction_id: String,      // NEW - to send response
    from_uri: String,
    contact_uri: String,
    expires: u32,
    authorization: Option<String>,  // NEW - Authorization header if present
    request_uri: String,
}
```

**Session-core → Dialog-core: ❌ MISSING**
```rust
SessionToDialogEvent::SendRegisterResponse {
    transaction_id: String,
    status_code: u16,           // 200, 401, 403, etc.
    www_authenticate: Option<String>,  // For 401 responses
    contact: Option<String>,    // Echo back contact in 200 OK
    expires: Option<u32>,       // Echo back expires in 200 OK
}
```

---

## Implementation Plan

### Step 1: Add Event Definitions to infra-common (1 hour)

**File:** `crates/infra-common/src/events/cross_crate.rs`

**Add to DialogToSessionEvent:**
```rust
pub enum DialogToSessionEvent {
    // ... existing events ...
    
    /// Incoming REGISTER request (server-side)
    IncomingRegister {
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        call_id: String,
    },
}
```

**Add to SessionToDialogEvent:**
```rust
pub enum SessionToDialogEvent {
    // ... existing events ...
    
    /// Send REGISTER response (401/200)
    SendRegisterResponse {
        transaction_id: String,
        status_code: u16,
        reason: String,
        www_authenticate: Option<String>,  // For 401
        contact: Option<String>,           // For 200
        expires: Option<u32>,              // For 200
    },
}
```

**Update session_id() extraction:**
```rust
impl RoutableEvent for RvoipCrossCrateEvent {
    fn session_id(&self) -> Option<&str> {
        match self {
            RvoipCrossCrateEvent::DialogToSession(event) => match event {
                // ... existing matches ...
                DialogToSessionEvent::IncomingRegister { .. } => None,  // No session yet
            },
            RvoipCrossCrateEvent::SessionToDialog(event) => match event {
                // ... existing matches ...
                SessionToDialogEvent::SendRegisterResponse { .. } => None,  // Transaction-based
            },
        }
    }
}
```

---

### Step 2: Update dialog-core to Emit IncomingRegister (1 hour)

**File:** `crates/dialog-core/src/protocol/register_handler.rs`

**Change from:**
```rust
let event = SessionCoordinationEvent::RegistrationRequest {
    transaction_id: transaction_id.clone(),
    from_uri,
    contact_uri,
    expires,
};
self.notify_session_layer(event).await?;
```

**To:**
```rust
// Extract Authorization header
use rvoip_sip_core::types::headers::HeaderAccess;
let authorization = request.raw_header_value(
    &rvoip_sip_core::types::header::HeaderName::Authorization
);

// Publish to global event bus
let event = RvoipCrossCrateEvent::DialogToSession(
    DialogToSessionEvent::IncomingRegister {
        transaction_id: transaction_id.to_string(),
        from_uri: from_uri.to_string(),
        to_uri: from_uri.to_string(),  // To same as From for self-registration
        contact_uri: contact_uri.to_string(),
        expires,
        authorization,
        call_id: request.call_id().unwrap().value().to_string(),
    }
);

self.global_coordinator.publish(Arc::new(event)).await?;
```

---

### Step 3: Update dialog-core to Handle SendRegisterResponse (1 hour)

**File:** `crates/dialog-core/src/manager/event_processing.rs` (or similar)

**Add event handler:**
```rust
async fn handle_session_to_dialog_event(
    &self,
    event: SessionToDialogEvent,
) -> DialogResult<()> {
    match event {
        // ... existing handlers ...
        
        SessionToDialogEvent::SendRegisterResponse {
            transaction_id,
            status_code,
            reason,
            www_authenticate,
            contact,
            expires,
        } => {
            // Parse transaction ID
            let tx_key = parse_transaction_key(&transaction_id)?;
            
            // Build response
            let mut response = Response::new(StatusCode::from(status_code));
            
            // Add WWW-Authenticate if 401
            if let Some(www_auth) = www_authenticate {
                response.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(www_auth.into_bytes())
                ));
            }
            
            // Add Contact and Expires if 200
            if status_code == 200 {
                if let Some(contact_uri) = contact {
                    // Add Contact header
                }
                if let Some(exp) = expires {
                    // Add Expires header
                }
            }
            
            // Send response
            self.transaction_manager.send_response(&tx_key, response).await?;
        }
    }
}
```

---

### Step 4: Implement session-core-v3 Server-Side REGISTER Handler (3-4 hours)

**File:** `crates/session-core-v3/src/adapters/registration_handler.rs` (NEW)

```rust
//! Server-side REGISTER request handler

use std::sync::Arc;
use rvoip_infra_common::events::cross_crate::{
    DialogToSessionEvent, SessionToDialogEvent, RvoipCrossCrateEvent
};
use rvoip_registrar_core::RegistrarService;
use tracing::{info, warn};

pub struct RegistrationHandler {
    registrar: Arc<RegistrarService>,
    global_coordinator: Arc<GlobalEventCoordinator>,
}

impl RegistrationHandler {
    pub fn new(
        registrar: Arc<RegistrarService>,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Self {
        Self {
            registrar,
            global_coordinator,
        }
    }
    
    /// Handle incoming REGISTER request
    pub async fn handle_incoming_register(
        &self,
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        request_uri: String,
    ) -> Result<()> {
        info!("Handling incoming REGISTER from {}", from_uri);
        
        // Extract username from URI
        let username = extract_username(&from_uri)?;
        
        // Authenticate via registrar-core
        let (should_register, www_auth_challenge) = self.registrar
            .authenticate_register(
                &username,
                authorization.as_deref(),
                "REGISTER",
                &request_uri,
            )
            .await?;
        
        if should_register {
            // Valid credentials - register user
            info!("Authentication successful for {}", username);
            
            // Parse contact as ContactInfo
            let contact = /* parse contact_uri */;
            
            self.registrar.register_user(&username, contact, Some(expires)).await?;
            
            // Send 200 OK response
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 200,
                    reason: "OK".to_string(),
                    www_authenticate: None,
                    contact: Some(contact_uri),
                    expires: Some(expires),
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event)).await?;
            
            info!("✅ User {} registered successfully", username);
        } else {
            // Need authentication - send 401
            info!("Sending 401 challenge for {}", username);
            
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 401,
                    reason: "Unauthorized".to_string(),
                    www_authenticate: www_auth_challenge,
                    contact: None,
                    expires: None,
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event)).await?;
            
            info!("✅ Sent 401 challenge for {}", username);
        }
        
        Ok(())
    }
    
    /// Subscribe to IncomingRegister events
    pub async fn start(&self) -> Result<()> {
        let subscriber = self.global_coordinator
            .subscribe::<RvoipCrossCrateEvent>()
            .await?;
        
        let handler = self.clone();
        
        tokio::spawn(async move {
            loop {
                if let Some(event) = subscriber.recv().await {
                    if let RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::IncomingRegister {
                            transaction_id,
                            from_uri,
                            contact_uri,
                            expires,
                            authorization,
                            request_uri,
                            ..
                        }
                    ) = event {
                        let _ = handler.handle_incoming_register(
                            transaction_id,
                            from_uri,
                            contact_uri,
                            expires,
                            authorization,
                            request_uri,
                        ).await;
                    }
                }
            }
        });
        
        Ok(())
    }
}
```

---

### Step 5: Wire Up in UnifiedCoordinator (1 hour)

**File:** `crates/session-core-v3/src/api/unified.rs`

```rust
impl UnifiedCoordinator {
    pub async fn new_with_registrar(
        config: Config,
        registrar: Arc<RegistrarService>,
    ) -> Result<Arc<Self>> {
        // ... create coordinator as usual ...
        
        // Create and start registration handler
        let registration_handler = RegistrationHandler::new(
            registrar,
            global_coordinator.clone(),
        );
        
        registration_handler.start().await?;
        
        // ... rest of setup ...
    }
}
```

---

### Step 6: Update Tests (2 hours)

**File:** `crates/session-core-v3/tests/registration_test.rs`

```rust
async fn start_registrar_server(port: u16, realm: &str) -> (Arc<RegistrarService>, Arc<DialogServer>) {
    // ... create transport, transaction manager, dialog server as before ...
    
    // Create registrar
    let registrar = RegistrarService::with_auth(...).await?;
    
    // Wire up registration handler in session-core-v3 pattern
    let registration_handler = RegistrationHandler::new(
        registrar.clone(),
        global_coordinator.clone(),
    );
    
    registration_handler.start().await?;
    
    // Start dialog server
    dialog_server.start().await?;  // Now will properly handle REGISTER!
    
    (registrar, dialog_server)
}
```

---

## Summary of Missing Pieces

### infra-common: ❌ Missing Events

**Need to add:**
1. `DialogToSessionEvent::IncomingRegister` - Dialog tells session about REGISTER
2. `SessionToDialogEvent::SendRegisterResponse` - Session tells dialog what response to send

**Estimated:** 1 hour

### dialog-core: ⚠️ Partially Done

**Need to change:**
1. ✅ Already emits event (but to wrong system)
2. ❌ Change to use `IncomingRegister` event on global bus
3. ❌ Subscribe to `SendRegisterResponse` events
4. ❌ Handle `SendRegisterResponse` by sending actual SIP response

**Estimated:** 2 hours

### session-core-v3: ❌ Not Started

**Need to add:**
1. ❌ New module: `src/adapters/registration_handler.rs`
2. ❌ Subscribe to `IncomingRegister` events
3. ❌ Call `registrar.authenticate_register()`
4. ❌ Publish `SendRegisterResponse` event
5. ❌ Wire up in `UnifiedCoordinator`

**Estimated:** 3-4 hours

---

## Total Work Required

| Component | Task | Hours |
|-----------|------|-------|
| infra-common | Add event types | 1 |
| dialog-core | Update to use new events | 2 |
| session-core-v3 | Implement server-side handler | 3-4 |
| Tests | Wire up and verify | 1 |
| **TOTAL** | | **7-8 hours** |

---

## Why This Wasn't Done in Sprint 1

**Sprint 1 Scope:** Client-side registration only
- ✅ Clients can register with servers
- ✅ Clients can handle authentication
- ✅ Sufficient for building SIP client applications (PolicyPeer, CallbackPeer, EventStreamPeer)

**Server-side registration:** Different use case
- Building a SIP **server** (registrar, proxy, B2BUA)
- Not needed for client-only applications
- Deferred to later sprint or production deployment needs

---

## Recommendation

### For Sprint 2 (PolicyPeer, CallbackPeer, EventStreamPeer):
**No changes needed** - All three APIs are SIP **clients** that register WITH servers, not act AS servers.

Current implementation is perfect for this use case.

### For Building a SIP Server:
Implement the above plan (7-8 hours) to handle incoming REGISTER requests.

---

## Architecture Correctness

Your assessment is **100% correct**:

✅ **dialog-core** receives REGISTER, publishes event to bus  
✅ **session-core-v3** subscribes to event, coordinates with registrar-core  
✅ **session-core-v3** publishes response event back to bus  
✅ **dialog-core** subscribes to response event, sends SIP response

**This matches the existing pattern for IncomingCall/ReferResponse!**

The only thing missing is the actual implementation of these events and handlers.

