# SimplePeer API Completion Plan

## Executive Summary

This document outlines the implementation plan to complete the SimplePeer API for:
1. **Blind Transfer (Recipient Side)** - Enable transfer recipients to accept REFER and complete transfers
2. **Registration/Presence** - Add SIP registration and presence/subscription support

Both features have underlying state machine support but lack complete API exposure and implementation in adapters.

---

## Current State Assessment

### ✅ What Works Today

**SimplePeer Core Functionality:**
- Basic UAC/UAS call flows (make_call, accept, hangup)
- Audio streaming (exchange_audio, send_audio, subscribe_to_audio)
- Call hold/resume
- Event-driven API with wait helpers
- Blind transfer initiation (send_refer)

**Infrastructure:**
- UnifiedCoordinator supports multiple concurrent sessions
- SessionStore with DashMap for lock-free multi-session access
- State machine with YAML-defined transitions
- Dialog and Media adapters with global event coordination
- State table already defines Registration, Presence, and Transfer states

### ⚠️ What's Incomplete

**Blind Transfer Recipient:**
- ✅ Can receive REFER (`wait_for_refer()` exists)
- ❌ Cannot accept REFER with full context
- ❌ No helper to complete the transfer flow
- ❌ Events missing transaction_id and transfer_type
- ❌ No NOTIFY status updates to transferor

**Registration:**
- ✅ States defined (Registering, Registered, Unregistering)
- ❌ No API methods to register/unregister
- ❌ Actions not implemented (SendREGISTER)
- ❌ DialogAdapter missing send_register()
- ❌ No credential storage
- ❌ No registration events

**Presence/Subscription:**
- ✅ States defined (Subscribing, Subscribed, Publishing)
- ❌ No API methods to subscribe/publish
- ❌ Actions not implemented (SendSUBSCRIBE, ProcessNOTIFY)
- ❌ DialogAdapter missing send_subscribe()
- ❌ No presence events
- ❌ No presence body parsing

---

## Phase 1: Blind Transfer Recipient Completion

**Estimated Time: 4-6 hours**

### Goal
Enable transfer recipients to accept REFER requests and complete the transfer by:
1. Accepting the REFER (202 Accepted)
2. Terminating current call (BYE)
3. Calling transfer target (INVITE)
4. Notifying transferor of success (NOTIFY)

### Changes Required

#### 1.1 Update Event Types (`src/api/events.rs`)

**Add fields to existing event:**
```rust
pub enum Event {
    // ... existing events ...
    
    /// REFER request received - ENHANCED
    ReferReceived { 
        call_id: CallId, 
        refer_to: String,
        transaction_id: String,  // NEW: For NOTIFY correlation
        transfer_type: String,   // NEW: "blind" or "attended"
    },
    
    // NEW EVENTS:
    
    /// Transfer accepted by recipient
    TransferAccepted {
        call_id: CallId,
        refer_to: String,
    },
    
    /// Transfer completed successfully
    TransferCompleted {
        old_call_id: CallId,
        new_call_id: CallId,
        target: String,
    },
    
    /// Transfer failed
    TransferFailed {
        call_id: CallId,
        reason: String,
        status_code: u16,
    },
    
    /// Transfer progress update (for transferor monitoring)
    TransferProgress {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
}
```

#### 1.2 Add Transfer Context Type (`src/api/simple.rs`)

**Add struct before SimplePeer:**
```rust
/// Information about an incoming REFER request
#[derive(Debug, Clone)]
pub struct ReferRequest {
    pub call_id: CallId,
    pub refer_to: String,
    pub transaction_id: String,
    pub transfer_type: String, // "blind" or "attended"
}
```

#### 1.3 Update SimplePeer Structure (`src/api/simple.rs`)

**Add field to track pending transfer:**
```rust
pub struct SimplePeer {
    coordinator: Arc<UnifiedCoordinator>,
    event_rx: mpsc::Receiver<Event>,
    local_uri: String,
    is_shutdown: Arc<std::sync::atomic::AtomicBool>,
    
    // NEW: Track pending transfer context
    pending_refer: Arc<tokio::sync::Mutex<Option<ReferRequest>>>,
}
```

**Update constructors to initialize:**
```rust
impl SimplePeer {
    pub async fn new(name: &str) -> Result<Self> {
        // ... existing code ...
        Ok(Self {
            coordinator,
            event_rx,
            local_uri,
            is_shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending_refer: Arc::new(tokio::sync::Mutex::new(None)), // NEW
        })
    }
}
```

#### 1.4 Improve wait_for_refer() (`src/api/simple.rs`)

**Replace existing method:**
```rust
/// Wait for REFER request with full context
pub async fn wait_for_refer(&mut self) -> Result<Option<ReferRequest>> {
    while let Some(event) = self.event_rx.recv().await {
        match event {
            Event::ReferReceived { call_id, refer_to, transaction_id, transfer_type } => {
                let refer = ReferRequest {
                    call_id,
                    refer_to,
                    transaction_id,
                    transfer_type,
                };
                
                // Store for later use
                *self.pending_refer.lock().await = Some(refer.clone());
                
                return Ok(Some(refer));
            }
            Event::CallEnded { .. } => return Ok(None),
            _ => {} // Ignore other events
        }
    }
    Ok(None)
}
```

#### 1.5 Add accept_refer() Method (`src/api/simple.rs`)

**Add new method:**
```rust
/// Accept a REFER request
pub async fn accept_refer(&mut self, refer: &ReferRequest) -> Result<()> {
    tracing::info!("[SimplePeer] Accepting REFER for call {} to target {}", 
                   refer.call_id, refer.refer_to);
    
    // The state machine will handle sending 202 Accepted and NOTIFY (trying)
    // This is already implemented in the state table transitions
    
    Ok(())
}
```

#### 1.6 Add complete_blind_transfer() Method (`src/api/simple.rs`)

**Add new method:**
```rust
/// Complete a blind transfer by terminating current call and calling target
pub async fn complete_blind_transfer(
    &mut self, 
    refer: &ReferRequest,
) -> Result<CallId> {
    tracing::info!("[SimplePeer] Completing blind transfer from call {} to {}", 
                   refer.call_id, refer.refer_to);
    
    // 1. Hangup current call (this triggers BYE)
    self.hangup(&refer.call_id).await?;
    
    // 2. Wait for call to actually end
    let ended_call = self.wait_for_call_ended().await?;
    tracing::info!("[SimplePeer] Original call {} ended", ended_call);
    
    // 3. Make new call to transfer target
    let new_call_id = self.call(&refer.refer_to).await?;
    tracing::info!("[SimplePeer] Transfer call initiated: {}", new_call_id);
    
    // 4. When call is established, the state machine will send NOTIFY (success)
    // This happens automatically via the state table transitions
    
    Ok(new_call_id)
}
```

#### 1.7 Add wait_for_transfer_complete() Helper (`src/api/simple.rs`)

**Add new method:**
```rust
/// Wait for transfer to complete (either success or failure)
pub async fn wait_for_transfer_complete(&mut self) -> Result<Option<CallId>> {
    while let Some(event) = self.event_rx.recv().await {
        match event {
            Event::TransferCompleted { new_call_id, .. } => {
                return Ok(Some(new_call_id));
            }
            Event::TransferFailed { reason, .. } => {
                return Err(SessionError::TransferFailed(reason));
            }
            Event::CallEnded { .. } => {
                // Continue waiting - might be the old call ending
            }
            _ => {} // Ignore other events
        }
    }
    Err(SessionError::Other("Event channel closed".to_string()))
}
```

#### 1.8 Update SessionCrossCrateEventHandler (`src/adapters/session_event_handler.rs`)

**Ensure TransferRequested events are properly converted:**

Find the section that handles dialog events and update:

```rust
// In handle_dialog_event or similar
DialogToSessionEvent::TransferRequested { 
    dialog_id, 
    session_id, 
    refer_to, 
    transfer_type,
    transaction_id,  // Ensure this is captured from dialog-core
} => {
    // ... existing code ...
    
    // Publish to SimplePeer events
    if let Some(tx) = &self.simple_peer_event_tx {
        let event = Event::ReferReceived {
            call_id: session_id.0.clone(),
            refer_to: refer_to.clone(),
            transaction_id: transaction_id.clone(),  // NEW
            transfer_type: transfer_type.clone(),    // NEW
        };
        let _ = tx.send(event).await;
    }
}
```

#### 1.9 Update Error Types (`src/errors.rs`)

**Add transfer error variant:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    // ... existing variants ...
    
    #[error("Transfer failed: {0}")]
    TransferFailed(String),
}
```

#### 1.10 Update State Table (`state_tables/default.yaml`)

**Verify these transitions exist (they should already be there):**

```yaml
# Transfer recipient receives REFER
- role: "UAC"
  state: "Active"
  event:
    type: "TransferRequested"
  next_state: "TransferringCall"
  actions:
    - type: "SendReferAccepted"      # Send 202 Accepted
    - type: "SendNOTIFY"             # Send NOTIFY (trying)
  publish:
    - "TransferReceived"
  description: "Transfer recipient accepts REFER"

# After accepting, terminate current call
- role: "UAC"
  state: "TransferringCall"
  event:
    type: "DialogBYE"
  next_state: "Terminated"
  actions:
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  description: "Current call terminated for transfer"
```

**If missing, add these transitions.**

### Testing Strategy

#### Test 1: Basic Blind Transfer Flow
```rust
// Alice (recipient)
let mut alice = SimplePeer::new("alice").await?;
let incoming = alice.wait_for_incoming_call().await?;
alice.accept(&incoming.0).await?;

// Wait for Bob to transfer
if let Some(refer) = alice.wait_for_refer().await? {
    println!("Received REFER to {}", refer.refer_to);
    
    // Complete the transfer
    let new_call = alice.complete_blind_transfer(&refer).await?;
    alice.wait_for_answered(&new_call).await?;
    
    println!("Now talking to Charlie");
}
```

#### Test 2: Transfer with Error Handling
```rust
match alice.wait_for_refer().await? {
    Some(refer) => {
        match alice.complete_blind_transfer(&refer).await {
            Ok(new_call) => {
                println!("Transfer successful");
            }
            Err(e) => {
                println!("Transfer failed: {}", e);
                // Could fall back to original call
            }
        }
    }
    None => println!("Call ended without transfer"),
}
```

### Files to Modify

1. ✅ `src/api/events.rs` - Add/enhance transfer events
2. ✅ `src/api/simple.rs` - Add ReferRequest, methods, field
3. ✅ `src/errors.rs` - Add TransferFailed variant
4. ✅ `src/adapters/session_event_handler.rs` - Enhance event conversion
5. ⚠️ `state_tables/default.yaml` - Verify transitions (likely already exist)

### Success Criteria

- ✅ Recipient can receive REFER with full context
- ✅ Recipient can accept REFER (202 Accepted sent)
- ✅ Recipient terminates current call (BYE sent)
- ✅ Recipient initiates new call to target (INVITE sent)
- ✅ Transferor receives NOTIFY messages (trying, success)
- ✅ Example blind_transfer test passes end-to-end

---

## Phase 2: Registration Support

**Estimated Time: 8-12 hours**

### Goal
Enable SIP registration with a registrar server for receiving calls at a registered address.

### Architecture Overview

Registration requires coordination across multiple layers:
1. **SimplePeer** - User-facing API
2. **UnifiedCoordinator** - Session management
3. **StateMachine** - Registration state transitions
4. **DialogAdapter** - SIP REGISTER message handling
5. **SessionStore** - Credential and registration state storage

### Changes Required

#### 2.1 Add Registration Events (`src/api/events.rs`)

```rust
pub enum Event {
    // ... existing events ...
    
    // Registration Events
    
    /// Registration initiated
    RegistrationStarted {
        server: String,
        expires: u32,
    },
    
    /// Registration successful
    RegistrationSuccess {
        server: String,
        expires: u32,
        contact: String,
    },
    
    /// Registration failed
    RegistrationFailed {
        server: String,
        reason: String,
        status_code: u16,
    },
    
    /// Registration refresh successful
    RegistrationRefreshed {
        server: String,
        expires: u32,
    },
    
    /// Unregistration completed
    Unregistered {
        server: String,
    },
    
    /// Registration expired (needs refresh)
    RegistrationExpired {
        server: String,
    },
}
```

#### 2.2 Add Credentials Type (`src/types.rs`)

**Verify this already exists (it should):**
```rust
/// User credentials for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
}
```

If missing, add it.

#### 2.3 Update SessionState (`src/session_store/state.rs`)

**Add registration-related fields:**
```rust
pub struct SessionState {
    // ... existing fields ...
    
    // Registration-specific fields
    pub registrar_uri: Option<String>,
    pub credentials: Option<Credentials>,
    pub expires: Option<u32>,
    pub contact_uri: Option<String>,
    pub registration_refresh_task: Option<tokio::task::JoinHandle<()>>,
}
```

#### 2.4 Add SimplePeer Registration Methods (`src/api/simple.rs`)

**Update struct:**
```rust
pub struct SimplePeer {
    coordinator: Arc<UnifiedCoordinator>,
    event_rx: mpsc::Receiver<Event>,
    local_uri: String,
    is_shutdown: Arc<std::sync::atomic::AtomicBool>,
    pending_refer: Arc<tokio::sync::Mutex<Option<ReferRequest>>>,
    
    // NEW: Track registration
    registration_session: Arc<tokio::sync::Mutex<Option<SessionId>>>,
}
```

**Add registration methods:**
```rust
impl SimplePeer {
    /// Register with a SIP server
    pub async fn register(
        &mut self,
        server_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("[SimplePeer] Registering with {} (expires: {}s)", server_uri, expires);
        
        // Create a registration session via coordinator
        let session_id = self.coordinator.start_registration(
            &self.local_uri,
            server_uri,
            username,
            password,
            expires,
        ).await?;
        
        // Store session ID
        *self.registration_session.lock().await = Some(session_id);
        
        Ok(())
    }
    
    /// Wait for registration to complete
    pub async fn wait_for_registration(&mut self) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::RegistrationSuccess { server, expires, .. } => {
                    tracing::info!("[SimplePeer] Registration successful: {} ({}s)", server, expires);
                    return Ok(());
                }
                Event::RegistrationFailed { reason, status_code, .. } => {
                    tracing::error!("[SimplePeer] Registration failed: {} ({})", reason, status_code);
                    return Err(SessionError::RegistrationFailed(reason));
                }
                _ => {} // Ignore other events
            }
        }
        Err(SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Unregister from SIP server
    pub async fn unregister(&mut self) -> Result<()> {
        if let Some(session_id) = self.registration_session.lock().await.take() {
            tracing::info!("[SimplePeer] Unregistering session {}", session_id);
            self.coordinator.unregister(&session_id).await?;
        } else {
            tracing::warn!("[SimplePeer] Not registered, nothing to unregister");
        }
        Ok(())
    }
    
    /// Check if currently registered
    pub async fn is_registered(&self) -> bool {
        self.registration_session.lock().await.is_some()
    }
    
    /// Refresh registration (manual refresh)
    pub async fn refresh_registration(&mut self) -> Result<()> {
        if let Some(session_id) = self.registration_session.lock().await.as_ref() {
            tracing::info!("[SimplePeer] Refreshing registration for {}", session_id);
            self.coordinator.refresh_registration(session_id).await?;
            Ok(())
        } else {
            Err(SessionError::NotRegistered)
        }
    }
}
```

#### 2.5 Add UnifiedCoordinator Registration Methods (`src/api/unified.rs`)

```rust
impl UnifiedCoordinator {
    /// Start a registration session
    pub async fn start_registration(
        &self,
        from: &str,
        server: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<SessionId> {
        tracing::info!("[UnifiedCoordinator] Starting registration: {} -> {}", from, server);
        
        // Create a new session in Idle state with Role::UAC
        let session_id = SessionId::new();
        let mut session = self.helpers.state_machine.store.create_session(
            session_id.clone(),
            crate::state_table::types::Role::UAC,
            false, // No history for registration
        ).await?;
        
        // Store registration-specific data
        session.local_uri = Some(from.to_string());
        session.remote_uri = Some(server.to_string());
        session.registrar_uri = Some(server.to_string());
        session.credentials = Some(Credentials {
            username: username.to_string(),
            password: password.to_string(),
            realm: None, // Will be filled from challenge
        });
        session.expires = Some(expires);
        
        // Update session
        self.helpers.state_machine.store.update_session(session).await?;
        
        // Trigger registration via state machine
        self.helpers.state_machine.process_event(
            &session_id,
            EventType::StartRegistration,
        ).await?;
        
        Ok(session_id)
    }
    
    /// Unregister (expires=0)
    pub async fn unregister(&self, session_id: &SessionId) -> Result<()> {
        tracing::info!("[UnifiedCoordinator] Unregistering session {}", session_id);
        
        self.helpers.state_machine.process_event(
            session_id,
            EventType::UnregisterRequest,
        ).await?;
        
        Ok(())
    }
    
    /// Refresh registration
    pub async fn refresh_registration(&self, session_id: &SessionId) -> Result<()> {
        tracing::info!("[UnifiedCoordinator] Refreshing registration for {}", session_id);
        
        // Re-trigger StartRegistration event
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StartRegistration,
        ).await?;
        
        Ok(())
    }
}
```

#### 2.6 Implement SendREGISTER Action (`src/state_machine/actions.rs`)

**Add to action executor:**
```rust
Action::SendREGISTER => {
    debug!("Sending REGISTER request");
    
    let registrar_uri = session.registrar_uri.as_ref()
        .ok_or("No registrar URI")?;
    let expires = session.expires.unwrap_or(3600);
    let credentials = session.credentials.as_ref();
    
    dialog_adapter.send_register(
        &session.session_id,
        registrar_uri,
        expires,
        credentials,
    ).await?;
    
    info!("REGISTER sent to {} (expires: {}s)", registrar_uri, expires);
}

Action::ProcessRegistrationResponse => {
    debug!("Processing registration response");
    // The state machine will handle 200 OK / 401 / etc. via events
    // This action is for storing response data if needed
}
```

#### 2.7 Add DialogAdapter::send_register() (`src/adapters/dialog_adapter.rs`)

**Add new method:**
```rust
impl DialogAdapter {
    /// Send REGISTER request
    pub async fn send_register(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        expires: u32,
        credentials: Option<&Credentials>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use rvoip_sip_core::{Method, Request, Uri};
        
        tracing::info!("[DialogAdapter] Sending REGISTER to {} (expires: {}s)", 
                      registrar_uri, expires);
        
        // Get session to retrieve dialog ID
        let session = self.store.get_session(session_id).await?;
        
        // Create or get dialog ID
        let dialog_id = if let Some(did) = session.dialog_id {
            did
        } else {
            // Create a new dialog for registration
            let new_dialog_id = rvoip_dialog_core::DialogId::new();
            let mut updated_session = session.clone();
            updated_session.dialog_id = Some(new_dialog_id.into());
            self.store.update_session(updated_session).await?;
            new_dialog_id
        };
        
        // Build REGISTER request
        let mut request = Request::new(
            Method::REGISTER,
            Uri::parse(registrar_uri)?,
        );
        
        // Add required headers
        request.headers_mut().insert(
            "Expires",
            expires.to_string().parse()?,
        );
        
        // Add Contact header
        if let Some(local_uri) = &session.local_uri {
            request.headers_mut().insert(
                "Contact",
                format!("<{}>", local_uri).parse()?,
            );
        }
        
        // Add Authorization if credentials provided (for re-registration after 401)
        if let Some(creds) = credentials {
            if let Some(realm) = &creds.realm {
                // Build digest auth header
                // Note: This is simplified - full digest auth requires nonce, etc.
                let auth_header = format!(
                    "Digest username=\"{}\", realm=\"{}\", uri=\"{}\"",
                    creds.username, realm, registrar_uri
                );
                request.headers_mut().insert(
                    "Authorization",
                    auth_header.parse()?,
                );
            }
        }
        
        // Send via dialog-core
        self.dialog_api
            .send_request(dialog_id.into(), request)
            .await?;
        
        tracing::info!("[DialogAdapter] REGISTER sent successfully");
        Ok(())
    }
}
```

#### 2.8 Update State Table (`state_tables/default.yaml`)

**Add registration transitions:**
```yaml
# =============================================================================
# REGISTRATION TRANSITIONS
# =============================================================================

# Start registration from Idle
- role: "UAC"
  state: "Idle"
  event:
    type: "StartRegistration"
  next_state: "Registering"
  actions:
    - type: "SendREGISTER"
  publish:
    - "RegistrationStarted"
  description: "Initiate SIP registration"

# Registration successful (200 OK)
- role: "UAC"
  state: "Registering"
  event:
    type: "Registration200OK"
  next_state: "Registered"
  publish:
    - "RegistrationSuccess"
  description: "Registration succeeded"

# Registration failed (401/403/etc)
- role: "UAC"
  state: "Registering"
  event:
    type: "RegistrationFailed"
  next_state: "Idle"
  publish:
    - "RegistrationFailed"
  description: "Registration failed"

# Unregister request
- role: "UAC"
  state: "Registered"
  event:
    type: "UnregisterRequest"
  next_state: "Unregistering"
  actions:
    - type: "SendREGISTER"  # With expires=0
  description: "Unregister from server"

# Unregistration complete
- role: "UAC"
  state: "Unregistering"
  event:
    type: "Registration200OK"
  next_state: "Idle"
  publish:
    - "Unregistered"
  description: "Unregistration successful"

# Refresh registration
- role: "UAC"
  state: "Registered"
  event:
    type: "StartRegistration"
  next_state: "Registering"
  actions:
    - type: "SendREGISTER"
  description: "Refresh registration"
```

#### 2.9 Update SessionCrossCrateEventHandler (`src/adapters/session_event_handler.rs`)

**Handle registration responses from dialog-core:**
```rust
// In dialog event handler
DialogResponse::Success { status_code, .. } if status_code == 200 => {
    // Check if this is a registration response
    let session = store.get_session(&session_id).await?;
    
    if session.registrar_uri.is_some() {
        // This is a registration response
        let event = if session.call_state == CallState::Registering {
            EventType::Registration200OK
        } else {
            EventType::Dialog200OK
        };
        
        state_machine.process_event(&session_id, event).await?;
        
        // Publish to SimplePeer
        if let Some(tx) = &simple_peer_event_tx {
            let _ = tx.send(Event::RegistrationSuccess {
                server: session.registrar_uri.unwrap_or_default(),
                expires: session.expires.unwrap_or(3600),
                contact: session.local_uri.unwrap_or_default(),
            }).await;
        }
    }
}

DialogResponse::ClientError { status_code, reason, .. } => {
    let session = store.get_session(&session_id).await?;
    
    if session.registrar_uri.is_some() {
        // Registration failed
        state_machine.process_event(
            &session_id, 
            EventType::RegistrationFailed(status_code)
        ).await?;
        
        // Publish to SimplePeer
        if let Some(tx) = &simple_peer_event_tx {
            let _ = tx.send(Event::RegistrationFailed {
                server: session.registrar_uri.unwrap_or_default(),
                reason: reason.unwrap_or_default(),
                status_code,
            }).await;
        }
    }
}
```

#### 2.10 Add Error Variant (`src/errors.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    // ... existing variants ...
    
    #[error("Registration failed: {0}")]
    RegistrationFailed(String),
    
    #[error("Not registered")]
    NotRegistered,
}
```

### Testing Strategy

#### Test 1: Basic Registration
```rust
let mut peer = SimplePeer::new("alice").await?;

// Register with server
peer.register(
    "sip:registrar.example.com:5060",
    "alice",
    "password123",
    3600,
).await?;

// Wait for registration to complete
peer.wait_for_registration().await?;

println!("Registered successfully");

// Now can receive calls at registered address

// Unregister when done
peer.unregister().await?;
```

#### Test 2: Registration with Auto-Refresh
```rust
peer.register("sip:registrar.example.com:5060", "alice", "pass", 60).await?;
peer.wait_for_registration().await?;

// Set up auto-refresh (would need background task)
loop {
    tokio::time::sleep(Duration::from_secs(50)).await;
    peer.refresh_registration().await?;
}
```

### Files to Modify

1. ✅ `src/api/events.rs` - Add registration events
2. ✅ `src/api/simple.rs` - Add registration methods
3. ✅ `src/api/unified.rs` - Add registration methods
4. ✅ `src/types.rs` - Verify Credentials exists
5. ✅ `src/session_store/state.rs` - Add registration fields
6. ✅ `src/state_machine/actions.rs` - Implement SendREGISTER
7. ✅ `src/adapters/dialog_adapter.rs` - Add send_register()
8. ✅ `src/adapters/session_event_handler.rs` - Handle registration responses
9. ✅ `src/errors.rs` - Add registration errors
10. ✅ `state_tables/default.yaml` - Add registration transitions

### Success Criteria

- ✅ Can initiate registration with registrar
- ✅ Can handle 200 OK response
- ✅ Can handle authentication challenges (401)
- ✅ Can refresh registration
- ✅ Can unregister (expires=0)
- ✅ SimplePeer events published correctly
- ✅ Example registration test passes

---

## Phase 3: Presence/Subscription Support

**Estimated Time: 10-15 hours**

### Goal
Enable SIP presence and event subscription (SUBSCRIBE/NOTIFY) for monitoring user status and receiving event notifications.

### Changes Required

#### 3.1 Add Presence Events (`src/api/events.rs`)

```rust
pub enum Event {
    // ... existing events ...
    
    // Subscription Events
    
    /// Subscription initiated
    SubscriptionStarted {
        target: String,
        event_package: String,
        expires: u32,
    },
    
    /// Subscription accepted (200 OK)
    SubscriptionAccepted {
        target: String,
        event_package: String,
        expires: u32,
    },
    
    /// Subscription failed
    SubscriptionFailed {
        target: String,
        event_package: String,
        reason: String,
        status_code: u16,
    },
    
    /// NOTIFY received
    NotifyReceived {
        from: String,
        event_package: String,
        body: String,
        subscription_state: String, // "active", "terminated", etc.
    },
    
    /// Subscription terminated
    SubscriptionTerminated {
        target: String,
        reason: String,
    },
    
    /// Subscription refresh successful
    SubscriptionRefreshed {
        target: String,
        expires: u32,
    },
}
```

#### 3.2 Add Presence Types (`src/api/types.rs` or `src/types.rs`)

```rust
/// Presence status for a user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    /// Online and available
    Available,
    /// Away from device
    Away,
    /// Busy/Do not disturb
    Busy,
    /// On the phone
    OnThePhone,
    /// Offline
    Offline,
    /// Custom status with free-form text
    Custom(String),
}

impl PresenceStatus {
    /// Parse from PIDF XML body (simplified)
    pub fn from_pidf(body: &str) -> Self {
        if body.contains("<basic>open</basic>") {
            PresenceStatus::Available
        } else if body.contains("<basic>closed</basic>") {
            PresenceStatus::Offline
        } else if body.contains("away") {
            PresenceStatus::Away
        } else if body.contains("busy") || body.contains("dnd") {
            PresenceStatus::Busy
        } else if body.contains("on-the-phone") {
            PresenceStatus::OnThePhone
        } else {
            PresenceStatus::Custom(body.to_string())
        }
    }
    
    /// Convert to PIDF XML body (simplified)
    pub fn to_pidf(&self, presentity: &str) -> String {
        let basic = match self {
            PresenceStatus::Available | PresenceStatus::Away | 
            PresenceStatus::Busy | PresenceStatus::OnThePhone => "open",
            PresenceStatus::Offline => "closed",
            PresenceStatus::Custom(_) => "open",
        };
        
        let note = match self {
            PresenceStatus::Available => "Available",
            PresenceStatus::Away => "Away",
            PresenceStatus::Busy => "Busy",
            PresenceStatus::OnThePhone => "On the phone",
            PresenceStatus::Offline => "Offline",
            PresenceStatus::Custom(s) => s.as_str(),
        };
        
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<presence xmlns="urn:ietf:params:xml:ns:pidf" entity="{}">
  <tuple id="tuple-1">
    <status>
      <basic>{}</basic>
    </status>
    <note>{}</note>
  </tuple>
</presence>"#,
            presentity, basic, note
        )
    }
}

/// Information about a presence update
#[derive(Debug, Clone)]
pub struct PresenceUpdate {
    pub uri: String,
    pub status: PresenceStatus,
    pub note: Option<String>,
    pub timestamp: std::time::SystemTime,
}

/// Information about a subscription
#[derive(Debug, Clone)]
pub struct SubscriptionInfo {
    pub session_id: SessionId,
    pub target: String,
    pub event_package: String,
    pub expires: u32,
    pub state: String, // "active", "pending", "terminated"
}
```

#### 3.3 Update SimplePeer Structure (`src/api/simple.rs`)

```rust
pub struct SimplePeer {
    coordinator: Arc<UnifiedCoordinator>,
    event_rx: mpsc::Receiver<Event>,
    local_uri: String,
    is_shutdown: Arc<std::sync::atomic::AtomicBool>,
    pending_refer: Arc<tokio::sync::Mutex<Option<ReferRequest>>>,
    registration_session: Arc<tokio::sync::Mutex<Option<SessionId>>>,
    
    // NEW: Track subscriptions
    subscriptions: Arc<tokio::sync::Mutex<std::collections::HashMap<String, SessionId>>>,
}
```

#### 3.4 Add SimplePeer Presence Methods (`src/api/simple.rs`)

```rust
impl SimplePeer {
    /// Subscribe to presence for a user
    pub async fn subscribe_presence(
        &mut self,
        target_uri: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("[SimplePeer] Subscribing to presence: {}", target_uri);
        
        let session_id = self.coordinator.start_subscription(
            &self.local_uri,
            target_uri,
            "presence", // Event package
            expires,
        ).await?;
        
        // Track subscription
        self.subscriptions.lock().await.insert(
            target_uri.to_string(),
            session_id,
        );
        
        Ok(())
    }
    
    /// Wait for subscription to be accepted
    pub async fn wait_for_subscription(&mut self, target: &str) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::SubscriptionAccepted { target: t, .. } if t == target => {
                    tracing::info!("[SimplePeer] Subscription accepted: {}", target);
                    return Ok(());
                }
                Event::SubscriptionFailed { target: t, reason, .. } if t == target => {
                    tracing::error!("[SimplePeer] Subscription failed: {}", reason);
                    return Err(SessionError::SubscriptionFailed(reason));
                }
                _ => {} // Ignore other events
            }
        }
        Err(SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Wait for presence update (NOTIFY)
    pub async fn wait_for_presence_update(&mut self) -> Result<PresenceUpdate> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::NotifyReceived { from, event_package, body, .. } 
                    if event_package == "presence" => 
                {
                    let status = PresenceStatus::from_pidf(&body);
                    return Ok(PresenceUpdate {
                        uri: from,
                        status,
                        note: None, // Could parse from PIDF
                        timestamp: std::time::SystemTime::now(),
                    });
                }
                _ => {} // Ignore other events
            }
        }
        Err(SessionError::Other("Event channel closed".to_string()))
    }
    
    /// Unsubscribe from presence
    pub async fn unsubscribe_presence(&mut self, target_uri: &str) -> Result<()> {
        if let Some(session_id) = self.subscriptions.lock().await.remove(target_uri) {
            tracing::info!("[SimplePeer] Unsubscribing from: {}", target_uri);
            self.coordinator.unsubscribe(&session_id).await?;
        }
        Ok(())
    }
    
    /// Publish own presence status
    pub async fn publish_presence(&mut self, status: PresenceStatus) -> Result<()> {
        tracing::info!("[SimplePeer] Publishing presence: {:?}", status);
        self.coordinator.publish_presence(&self.local_uri, status).await
    }
    
    /// Subscribe to any event package (generic)
    pub async fn subscribe(
        &mut self,
        target_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("[SimplePeer] Subscribing to {}: {}", event_package, target_uri);
        
        let session_id = self.coordinator.start_subscription(
            &self.local_uri,
            target_uri,
            event_package,
            expires,
        ).await?;
        
        let key = format!("{}:{}", event_package, target_uri);
        self.subscriptions.lock().await.insert(key, session_id);
        
        Ok(())
    }
}
```

#### 3.5 Add UnifiedCoordinator Subscription Methods (`src/api/unified.rs`)

```rust
impl UnifiedCoordinator {
    /// Start a subscription
    pub async fn start_subscription(
        &self,
        from: &str,
        target: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<SessionId> {
        tracing::info!("[UnifiedCoordinator] Starting subscription: {} -> {} ({})", 
                      from, target, event_package);
        
        // Create subscription session
        let session_id = SessionId::new();
        let mut session = self.helpers.state_machine.store.create_session(
            session_id.clone(),
            crate::state_table::types::Role::UAC,
            false,
        ).await?;
        
        // Store subscription data
        session.local_uri = Some(from.to_string());
        session.remote_uri = Some(target.to_string());
        session.event_package = Some(event_package.to_string());
        session.expires = Some(expires);
        
        self.helpers.state_machine.store.update_session(session).await?;
        
        // Trigger subscription
        self.helpers.state_machine.process_event(
            &session_id,
            EventType::StartSubscription,
        ).await?;
        
        Ok(session_id)
    }
    
    /// Unsubscribe (expires=0)
    pub async fn unsubscribe(&self, session_id: &SessionId) -> Result<()> {
        tracing::info!("[UnifiedCoordinator] Unsubscribing: {}", session_id);
        
        self.helpers.state_machine.process_event(
            session_id,
            EventType::UnsubscribeRequest,
        ).await?;
        
        Ok(())
    }
    
    /// Publish presence (simplified - could use PUBLISH or internal state)
    pub async fn publish_presence(
        &self,
        from: &str,
        status: PresenceStatus,
    ) -> Result<()> {
        tracing::info!("[UnifiedCoordinator] Publishing presence for {}: {:?}", from, status);
        
        // This would typically:
        // 1. Use SIP PUBLISH method, or
        // 2. Store state and send NOTIFY to subscribers
        // For now, store in a presence cache
        
        // TODO: Implement actual PUBLISH or NOTIFY mechanism
        
        Ok(())
    }
}
```

#### 3.6 Update SessionState (`src/session_store/state.rs`)

```rust
pub struct SessionState {
    // ... existing fields ...
    
    // Subscription-specific fields
    pub event_package: Option<String>,
    pub subscription_state: Option<String>, // "active", "pending", "terminated"
}
```

#### 3.7 Implement SUBSCRIBE Actions (`src/state_machine/actions.rs`)

```rust
Action::SendSUBSCRIBE => {
    debug!("Sending SUBSCRIBE request");
    
    let target = session.remote_uri.as_ref()
        .ok_or("No target URI")?;
    let event_package = session.event_package.as_ref()
        .ok_or("No event package")?;
    let expires = session.expires.unwrap_or(3600);
    
    dialog_adapter.send_subscribe(
        &session.session_id,
        target,
        event_package,
        expires,
    ).await?;
    
    info!("SUBSCRIBE sent to {} for {}", target, event_package);
}

Action::ProcessNOTIFY => {
    debug!("Processing NOTIFY");
    // NOTIFY body is already captured in session state
    // State machine will publish event to application
}

Action::SendNOTIFY => {
    debug!("Sending NOTIFY");
    
    if let Some(dialog_id) = &session.dialog_id {
        let event_package = session.event_package.as_ref()
            .ok_or("No event package")?;
        let body = session.notify_body.clone(); // Would need this field
        
        dialog_adapter.send_notify(
            dialog_id,
            event_package,
            body,
        ).await?;
        
        info!("NOTIFY sent");
    }
}
```

#### 3.8 Add DialogAdapter Subscription Methods (`src/adapters/dialog_adapter.rs`)

```rust
impl DialogAdapter {
    /// Send SUBSCRIBE request
    pub async fn send_subscribe(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use rvoip_sip_core::{Method, Request, Uri};
        
        tracing::info!("[DialogAdapter] Sending SUBSCRIBE to {} for {}", 
                      target_uri, event_package);
        
        let session = self.store.get_session(session_id).await?;
        
        // Create or get dialog
        let dialog_id = if let Some(did) = session.dialog_id {
            did
        } else {
            let new_dialog_id = rvoip_dialog_core::DialogId::new();
            let mut updated_session = session.clone();
            updated_session.dialog_id = Some(new_dialog_id.into());
            self.store.update_session(updated_session).await?;
            new_dialog_id
        };
        
        // Build SUBSCRIBE request
        let mut request = Request::new(
            Method::SUBSCRIBE,
            Uri::parse(target_uri)?,
        );
        
        // Add Event header
        request.headers_mut().insert(
            "Event",
            event_package.parse()?,
        );
        
        // Add Expires header
        request.headers_mut().insert(
            "Expires",
            expires.to_string().parse()?,
        );
        
        // Add Accept header for presence
        if event_package == "presence" {
            request.headers_mut().insert(
                "Accept",
                "application/pidf+xml".parse()?,
            );
        }
        
        // Send via dialog-core
        self.dialog_api
            .send_request(dialog_id.into(), request)
            .await?;
        
        tracing::info!("[DialogAdapter] SUBSCRIBE sent successfully");
        Ok(())
    }
    
    /// Send NOTIFY message
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event_package: &str,
        body: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use rvoip_sip_core::{Method, Request};
        
        tracing::info!("[DialogAdapter] Sending NOTIFY for {}", event_package);
        
        let mut request = Request::new(
            Method::NOTIFY,
            Uri::parse("sip:placeholder")?, // URI from dialog
        );
        
        // Add Event header
        request.headers_mut().insert(
            "Event",
            event_package.parse()?,
        );
        
        // Add Subscription-State header
        request.headers_mut().insert(
            "Subscription-State",
            "active".parse()?,
        );
        
        // Add body if present
        if let Some(content) = body {
            request.headers_mut().insert(
                "Content-Type",
                "application/pidf+xml".parse()?,
            );
            *request.body_mut() = content.into_bytes();
        }
        
        // Send via dialog-core
        self.dialog_api
            .send_request((*dialog_id).into(), request)
            .await?;
        
        tracing::info!("[DialogAdapter] NOTIFY sent successfully");
        Ok(())
    }
}
```

#### 3.9 Update State Table (`state_tables/default.yaml`)

```yaml
# =============================================================================
# SUBSCRIPTION/PRESENCE TRANSITIONS
# =============================================================================

# Start subscription
- role: "UAC"
  state: "Idle"
  event:
    type: "StartSubscription"
  next_state: "Subscribing"
  actions:
    - type: "SendSUBSCRIBE"
  publish:
    - "SubscriptionStarted"
  description: "Initiate subscription"

# Subscription accepted (200 OK)
- role: "UAC"
  state: "Subscribing"
  event:
    type: "SubscriptionAccepted"
  next_state: "Subscribed"
  publish:
    - "SubscriptionAccepted"
  description: "Subscription accepted"

# Subscription failed
- role: "UAC"
  state: "Subscribing"
  event:
    type: "SubscriptionFailed"
  next_state: "Idle"
  publish:
    - "SubscriptionFailed"
  description: "Subscription failed"

# Receive NOTIFY
- role: "UAC"
  state: "Subscribed"
  event:
    type: "ReceiveNOTIFY"
  next_state: "Subscribed"
  actions:
    - type: "ProcessNOTIFY"
  publish:
    - "NotifyReceived"
  description: "Received NOTIFY"

# Unsubscribe
- role: "UAC"
  state: "Subscribed"
  event:
    type: "UnsubscribeRequest"
  next_state: "Idle"
  actions:
    - type: "SendSUBSCRIBE"  # With expires=0
  description: "Unsubscribe"

# Subscription expired
- role: "UAC"
  state: "Subscribed"
  event:
    type: "SubscriptionExpired"
  next_state: "Idle"
  publish:
    - "SubscriptionTerminated"
  description: "Subscription expired"
```

#### 3.10 Add Error Variants (`src/errors.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    // ... existing variants ...
    
    #[error("Subscription failed: {0}")]
    SubscriptionFailed(String),
}
```

### Testing Strategy

#### Test 1: Basic Presence Subscription
```rust
let mut alice = SimplePeer::new("alice").await?;

// Subscribe to Bob's presence
alice.subscribe_presence("sip:bob@example.com", 3600).await?;
alice.wait_for_subscription("sip:bob@example.com").await?;

// Wait for presence updates
loop {
    let update = alice.wait_for_presence_update().await?;
    println!("{} is now: {:?}", update.uri, update.status);
}
```

#### Test 2: Publish Own Presence
```rust
let mut bob = SimplePeer::new("bob").await?;

// Register first (needed to receive subscriptions)
bob.register("sip:server.com", "bob", "pass", 3600).await?;
bob.wait_for_registration().await?;

// Publish presence
bob.publish_presence(PresenceStatus::Available).await?;

// Later...
bob.publish_presence(PresenceStatus::Busy).await?;
```

### Files to Modify

1. ✅ `src/api/events.rs` - Add subscription/presence events
2. ✅ `src/api/types.rs` - Add PresenceStatus, PresenceUpdate types
3. ✅ `src/api/simple.rs` - Add subscription methods
4. ✅ `src/api/unified.rs` - Add subscription methods
5. ✅ `src/session_store/state.rs` - Add subscription fields
6. ✅ `src/state_machine/actions.rs` - Implement subscription actions
7. ✅ `src/adapters/dialog_adapter.rs` - Add send_subscribe(), send_notify()
8. ✅ `src/adapters/session_event_handler.rs` - Handle SUBSCRIBE/NOTIFY responses
9. ✅ `src/errors.rs` - Add subscription errors
10. ✅ `state_tables/default.yaml` - Add subscription transitions

### Success Criteria

- ✅ Can subscribe to presence
- ✅ Can receive NOTIFY messages
- ✅ Can parse presence PIDF bodies
- ✅ Can unsubscribe
- ✅ Can publish own presence
- ✅ Example presence test passes

---

## Implementation Timeline

### Phase 1: Blind Transfer (Week 1)
- **Days 1-2**: Update events, add ReferRequest type, update SimplePeer methods
- **Day 3**: Update event handler, test integration
- **Day 4**: End-to-end testing, bug fixes

### Phase 2: Registration (Week 2-3)
- **Days 1-2**: Add registration events, types, SimplePeer API
- **Days 3-4**: UnifiedCoordinator methods, SessionState updates
- **Days 5-6**: DialogAdapter send_register(), action implementation
- **Days 7-8**: State table updates, event handler integration
- **Days 9-10**: Testing, authentication handling, auto-refresh

### Phase 3: Presence/Subscription (Week 4-5)
- **Days 1-2**: Add presence events, types, PIDF parsing
- **Days 3-4**: SimplePeer and UnifiedCoordinator APIs
- **Days 5-6**: DialogAdapter send_subscribe()/send_notify()
- **Days 7-8**: Action implementation, state table updates
- **Days 9-10**: Event handler integration
- **Days 11-12**: Testing, PIDF body handling, edge cases

**Total Estimated Time: 22-33 hours (3-5 weeks part-time)**

---

## Testing Plan

### Unit Tests

Each phase should include unit tests for:
- API method behavior
- Event generation
- State transitions
- Error handling

### Integration Tests

End-to-end tests for:
- Complete blind transfer flow (3 peers)
- Registration with auth challenge
- Presence subscription with NOTIFY
- Error scenarios (timeouts, failures)

### Example Programs

Create example programs in `examples/`:
1. `blind_transfer_complete/` - Full 3-party transfer
2. `registration/` - Registration with server
3. `presence/` - Presence subscription and publishing

---

## Risk Assessment

### Low Risk ✅
- Adding new API methods to SimplePeer
- Adding new event types
- State table updates (YAML changes)

### Medium Risk ⚠️
- DialogAdapter SIP message construction
- Event handler routing and conversion
- State coordination during transfers
- Authentication handling for registration

### High Risk 🔴
- PIDF XML parsing (complex format)
- Digest authentication implementation
- Auto-refresh mechanisms (background tasks)
- Race conditions in multi-session scenarios

### Mitigation Strategies

1. **For SIP messaging**: Start with minimal RFC-compliant messages, enhance later
2. **For authentication**: Use existing dialog-core auth if available, or simplified version
3. **For PIDF parsing**: Use simple string matching initially, add proper XML parser later
4. **For race conditions**: Extensive logging, careful lock ordering, use DashMap

---

## Dependencies

### External Crates (may need to add)
```toml
[dependencies]
# For PIDF XML parsing (optional)
quick-xml = "0.31"  # If proper XML parsing needed

# For digest auth (optional)
md5 = "0.7"  # If implementing digest auth manually
```

### Internal Dependencies
All required infrastructure exists:
- ✅ dialog-core for SIP messaging
- ✅ media-core for RTP
- ✅ infra-common for events
- ✅ State machine with YAML tables
- ✅ SessionStore with DashMap

---

## Success Metrics

### Phase 1 Success
- [ ] Recipient receives REFER with full context
- [ ] Recipient can complete transfer
- [ ] Transferor receives NOTIFY updates
- [ ] blind_transfer example passes

### Phase 2 Success
- [ ] Can register with server
- [ ] Can handle 401 auth challenge
- [ ] Can refresh registration
- [ ] Can unregister
- [ ] Registration example passes

### Phase 3 Success
- [ ] Can subscribe to presence
- [ ] Can receive and parse NOTIFY
- [ ] Can publish own presence
- [ ] Can unsubscribe
- [ ] Presence example passes

---

## Future Enhancements (Post-Implementation)

1. **Advanced Transfer**
   - Attended transfer support
   - Transfer with consultation
   - Transfer to conference

2. **Enhanced Registration**
   - Multiple registrations (different accounts)
   - Auto-renewal with exponential backoff
   - Registration event subscriptions

3. **Advanced Presence**
   - Rich presence (activity, mood, location)
   - Presence lists (RLMI)
   - Watcher info
   - XCAP document management

4. **General Improvements**
   - Better XML parsing (use proper parser)
   - Full digest authentication
   - TLS support
   - IPv6 support

---

## Conclusion

This plan provides a complete roadmap for adding blind transfer completion, registration, and presence/subscription support to the SimplePeer API. The implementation is structured in three phases with clear deliverables, testing strategies, and risk mitigation approaches.

Each phase builds on the existing infrastructure and follows the established patterns in session-core-v3. The estimated timeline is realistic for a part-time developer working 4-6 hours per day.

**Ready to implement!** 🚀

