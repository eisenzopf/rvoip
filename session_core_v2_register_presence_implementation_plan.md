# Implementation Plan: REGISTER and Presence Support for session-core-v2

## Executive Summary

This plan outlines how to add REGISTER and presence capabilities to session-core-v2, integrating with existing auth-core and registrar-core libraries while maintaining RFC compliance (3261, 3856, 3863, 4479). The implementation leverages session-core-v2's state machine architecture and event-driven design.

**Important Update**: A new `users-core` crate has been created to handle user management and JWT token issuance. This works in conjunction with auth-core:
- `users-core`: Manages internal users, authenticates passwords, issues JWT tokens
- `auth-core`: Validates all tokens (from users-core, OAuth2 providers, etc.)
- See `/crates/users-core/IMPLEMENTATION_PLAN.md` for details

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                           SIP Endpoints                              │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ SIP Messages
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         dialog-core                                  │
│  • REGISTER handler → forwards to session                          │
│  • SUBSCRIBE/NOTIFY via SubscriptionManager                        │
│  • PUBLISH support                                                 │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ Events
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      session-core-v2                                 │
│ ┌─────────────────┐ ┌──────────────────┐ ┌────────────────────┐   │
│ │ RegistrarAdapter│ │PresenceAdapter   │ │   AuthAdapter      │   │
│ │                 │ │                  │ │                    │   │
│ │ • REGISTER      │ │ • SUBSCRIBE      │ │ • Token validation │   │
│ │ • Location      │ │ • NOTIFY         │ │ • User context     │   │
│ │ • Expiry        │ │ • PUBLISH        │ │ • OAuth2 flows     │   │
│ └────────┬────────┘ └────────┬─────────┘ └──────────┬─────────┘   │
│          │                   │                       │              │
│          ▼                   ▼                       ▼              │
│ ┌───────────────────────────────────────────────────────────────┐  │
│ │                    State Machine                               │  │
│ │  States: Unregistered, Registering, Registered, etc.          │  │
│ │  Events: Register, Unregister, SubscribePresence, etc.        │  │
│ └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
                    │                    │                    │
                    ▼                    ▼                    ▼
         ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
         │  registrar-core  │ │   auth-core      │ │  media-core      │
         │                  │ │                  │ │                  │
         │ • User registry  │ │ • Token validate │ │ • RTP/SRTP       │
         │ • Presence store │ │ • OAuth2 & JWT   │ │ • Codecs         │
         │ • PIDF support   │ │ • Token cache    │ │ • Audio devices  │
         └──────────────────┘ └──────────────────┘ └──────────────────┘
                                        ▲
                                        │ Validates tokens from
                               ┌────────┴────────┐
                               │   users-core    │
                               │                 │
                               │ • Issue JWT     │
                               │ • User auth     │
                               └─────────────────┘
```

## Phase 1: Core Infrastructure

### 1.1 Add Missing Event Types to session-core-v2

**File: `src/state_table/types.rs`**

```rust
pub enum EventType {
    // ... existing events ...
    
    // Registration events
    Register { expires: u32 },
    Unregister,
    RegistrationSuccess { expires: u32 },
    RegistrationFailure { code: u16, reason: String },
    RegistrationExpiring,
    RegistrationRefresh,
    
    // Presence events  
    PublishPresence { status: String, note: Option<String> },
    SubscribePresence { target: String, expires: u32 },
    UnsubscribePresence { subscription_id: String },
    PresenceNotify { from: String, status: String, note: Option<String> },
    
    // Dialog events for REGISTER/SUBSCRIBE/NOTIFY/PUBLISH
    DialogREGISTER { from: String, contact: String, expires: u32 },
    DialogSUBSCRIBE { from: String, event: String, expires: u32 },
    DialogNOTIFY { subscription_state: String, body: Option<String> },
    DialogPUBLISH { event: String, body: Option<String> },
    DialogOPTIONS { from: String },
    DialogINFO { body: Option<String> },
    DialogUPDATE { sdp: Option<String> },
}
```

### 1.2 Add Registration and Presence States

**File: `src/types.rs`**

```rust
pub enum CallState {
    // ... existing states ...
    
    // Registration states
    Unregistered,
    Registering,
    Registered,
    Unregistering,
    
    // Presence states
    PresenceIdle,
    Publishing,
    Subscribed,
    
    // Combined states (can be registered and in a call)
    RegisteredIdle,
    RegisteredCalling,
    RegisteredInCall,
}
```

### 1.3 Create Adapter Modules

**File: `src/adapters/registrar_adapter.rs`**

```rust
use std::sync::Arc;
use rvoip_registrar_core::{RegistrarService, api::ServiceMode};
use rvoip_auth_core::{AuthenticationService, UserContext};

pub struct RegistrarAdapter {
    registrar: Arc<RegistrarService>,
    auth_service: Arc<dyn AuthenticationService>,
    store: Arc<SessionStore>,
}

impl RegistrarAdapter {
    pub async fn handle_register(
        &self,
        session_id: &SessionId,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        auth_header: Option<String>,
    ) -> Result<()> {
        // 1. Validate authentication
        if let Some(token) = auth_header {
            let user_context = self.auth_service.validate_token(&token).await?;
            // Store user context in session
        }
        
        // 2. Register with registrar-core
        let contact_info = ContactInfo {
            uri: contact_uri,
            expires: Utc::now() + Duration::seconds(expires as i64),
            // ... other fields
        };
        
        self.registrar.register_user(&from_uri, contact_info, Some(expires)).await?;
        
        // 3. Update session state
        self.store.update_registration_state(session_id, true).await?;
        
        Ok(())
    }
}
```

**File: `src/adapters/presence_adapter.rs`**

```rust
pub struct PresenceAdapter {
    registrar: Arc<RegistrarService>,
    dialog_adapter: Arc<DialogAdapter>,
    subscription_dialogs: Arc<DashMap<String, DialogId>>,
}

impl PresenceAdapter {
    pub async fn handle_publish(
        &self,
        session_id: &SessionId,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Update presence in registrar-core
        let user_id = self.get_user_id(session_id).await?;
        self.registrar.update_presence(&user_id, status, note).await?;
        Ok(())
    }
    
    pub async fn handle_subscribe(
        &self,
        session_id: &SessionId,
        target: String,
        expires: u32,
    ) -> Result<String> {
        // Create subscription in registrar-core
        let subscriber = self.get_user_id(session_id).await?;
        let subscription_id = self.registrar.subscribe_presence(
            &subscriber,
            &target,
            Some(expires)
        ).await?;
        
        // Store dialog mapping for NOTIFY
        let dialog_id = self.get_dialog_id(session_id).await?;
        self.subscription_dialogs.insert(subscription_id.clone(), dialog_id);
        
        Ok(subscription_id)
    }
}
```

## Phase 2: State Machine Integration

### 2.1 Update State Tables

**File: `state_tables/default_state_table.yaml`**

```yaml
states:
  # Registration states
  - name: "Unregistered"
    description: "Not registered with SIP server"
  - name: "Registering"
    description: "REGISTER in progress"
  - name: "Registered"
    description: "Successfully registered"
  - name: "RegisteredIdle"
    description: "Registered and ready for calls"
    
transitions:
  # Registration flow
  - role: "Both"
    state: "Unregistered"
    event:
      type: "Register"
    actions:
      - type: "SendREGISTER"
    next_state: "Registering"
    
  - role: "Both"
    state: "Registering"
    event:
      type: "DialogREGISTER"
    guards:
      - type: "HasAuthentication"
    actions:
      - type: "ProcessREGISTER"
      - type: "SendSIPResponse"
        args:
          code: 200
          reason: "OK"
    next_state: "Registered"
    effects:
      - type: "StartRegistrationTimer"
      
  # Presence subscription
  - role: "Both"
    state: "Registered"
    event:
      type: "SubscribePresence"
    actions:
      - type: "SendSUBSCRIBE"
    next_state: "Registered"
    
  # Presence publishing
  - role: "Both"  
    state: "Registered"
    event:
      type: "PublishPresence"
    actions:
      - type: "SendPUBLISH"
    next_state: "Publishing"
```

### 2.2 Add New Actions

**File: `src/state_table/types.rs`**

```rust
pub enum Action {
    // ... existing actions ...
    
    // Registration actions
    SendREGISTER,
    ProcessREGISTER,
    RefreshRegistration,
    ClearRegistration,
    
    // Presence actions
    SendSUBSCRIBE,
    SendNOTIFY,
    SendPUBLISH,
    UpdatePresenceState,
    ProcessPresenceNotify,
}
```

### 2.3 Implement Action Handlers

**File: `src/state_machine/actions.rs`**

```rust
pub async fn execute_action(
    action: &Action,
    session: &mut Session,
    // ... other params
) -> Result<()> {
    match action {
        Action::SendREGISTER => {
            // Get registration parameters from session
            let expires = session.registration_expires.unwrap_or(3600);
            
            // Send through dialog adapter
            dialog_adapter.send_register(
                &session.session_id,
                &session.local_uri,
                expires,
            ).await?;
        }
        
        Action::SendSUBSCRIBE => {
            let target = // extract from event context
            let expires = 3600;
            
            presence_adapter.send_subscribe(
                &session.session_id,
                target,
                expires,
            ).await?;
        }
        
        Action::SendPUBLISH => {
            let status = // extract from event context
            let note = // extract from event context
            
            presence_adapter.send_publish(
                &session.session_id,
                status,
                note,
            ).await?;
        }
        // ... other actions
    }
}
```

## Phase 3: Event Routing

### 3.1 Update Session Event Handler

**File: `src/adapters/session_event_handler.rs`**

```rust
impl SessionCrossCrateEventHandler {
    // Add handlers for new dialog events
    
    async fn handle_dialog_register(&self, event_str: &str) -> Result<()> {
        let from = self.extract_field(event_str, "from: \"").unwrap_or_default();
        let contact = self.extract_field(event_str, "contact: \"").unwrap_or_default();
        let expires = self.extract_field(event_str, "expires: ")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(3600);
        
        // Find or create session
        let session_id = self.find_or_create_session(&from).await?;
        
        // Process event
        self.state_machine.process_event(
            &session_id,
            EventType::DialogREGISTER { from, contact, expires }
        ).await?;
        
        Ok(())
    }
    
    async fn handle_presence_notify(&self, event_str: &str) -> Result<()> {
        let from = self.extract_field(event_str, "from: \"").unwrap_or_default();
        let status = self.extract_field(event_str, "status: \"").unwrap_or_default();
        let note = self.extract_field(event_str, "note: \"");
        
        // Find session by subscription
        if let Some(session_id) = self.find_session_by_subscription(&from).await {
            self.state_machine.process_event(
                &session_id,
                EventType::PresenceNotify { from, status, note }
            ).await?;
        }
        
        Ok(())
    }
}
```

### 3.2 Add Event Publishing

**File: `src/adapters/event_router.rs`**

```rust
impl EventRouter {
    pub async fn route_action(&self, action: &Action, session: &Session) -> Result<()> {
        match action {
            Action::SendREGISTER => {
                // Publish to dialog-core
                self.publish_event(SessionToDialogEvent::SendRegister {
                    session_id: session.session_id.clone(),
                    from_uri: session.local_uri.clone(),
                    expires: session.registration_expires.unwrap_or(3600),
                }).await?;
            }
            
            Action::SendSUBSCRIBE => {
                self.publish_event(SessionToDialogEvent::SendSubscribe {
                    session_id: session.session_id.clone(),
                    event: "presence".to_string(),
                    target: session.presence_target.clone().unwrap_or_default(),
                    expires: 3600,
                }).await?;
            }
            // ... other actions
        }
    }
}
```

## Phase 4: Authentication Integration

### 4.1 Create Auth Adapter

**File: `src/adapters/auth_adapter.rs`**

```rust
use rvoip_auth_core::{AuthenticationService, OAuth2Config, OAuth2Validator};

pub struct AuthAdapter {
    validator: Arc<OAuth2Validator>,
    config: OAuth2Config,
}

impl AuthAdapter {
    pub async fn new(oauth_config: OAuth2Config) -> Result<Self> {
        let validator = OAuth2Validator::new(oauth_config.clone()).await?;
        Ok(Self {
            validator: Arc::new(validator),
            config: oauth_config,
        })
    }
    
    pub async fn validate_sip_auth(
        &self,
        auth_header: &str,
    ) -> Result<UserContext> {
        // Extract bearer token
        let token = self.extract_bearer_token(auth_header)?;
        
        // Validate with auth-core
        let user_context = self.validator.validate_bearer_token(&token).await?;
        
        Ok(user_context)
    }
    
    pub async fn generate_www_authenticate(&self) -> String {
        format!("Bearer realm=\"{}\"", self.config.realm)
    }
}
```

### 4.2 Update Dialog Adapter for Auth

**File: `src/adapters/dialog_adapter.rs`**

```rust
impl DialogAdapter {
    pub async fn send_register_with_auth(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        expires: u32,
        auth_token: Option<String>,
    ) -> Result<()> {
        let mut headers = HashMap::new();
        
        if let Some(token) = auth_token {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", token),
            );
        }
        
        // Send REGISTER through dialog-core
        self.dialog_api.send_register(
            from_uri,
            expires,
            headers,
        ).await?;
        
        Ok(())
    }
}
```

## Phase 5: Session Store Extensions

### 5.1 Add Registration and Presence Fields

**File: `src/session_store/state.rs`**

```rust
pub struct Session {
    // ... existing fields ...
    
    // Registration state
    pub is_registered: bool,
    pub registration_expires: Option<u32>,
    pub registration_timer: Option<tokio::task::JoinHandle<()>>,
    pub auth_context: Option<UserContext>,
    
    // Presence state
    pub presence_status: PresenceStatus,
    pub presence_note: Option<String>,
    pub presence_subscriptions: Vec<SubscriptionInfo>,
    pub presence_target: Option<String>, // For outgoing subscriptions
}

#[derive(Debug, Clone)]
pub struct SubscriptionInfo {
    pub subscription_id: String,
    pub target: String,
    pub dialog_id: DialogId,
    pub expires_at: DateTime<Utc>,
}
```

## Phase 6: API Extensions

### 6.1 Add Registration and Presence APIs

**File: `src/api/mod.rs`**

```rust
impl SessionCoordinator {
    /// Register with SIP server
    pub async fn register(
        &self,
        from_uri: &str,
        auth_token: Option<String>,
        expires: Option<u32>,
    ) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Create session in Unregistered state
        self.store.create_session(
            session_id.clone(),
            Role::Both,
            false, // not a call session
        ).await?;
        
        // Store auth token if provided
        if let Some(token) = auth_token {
            self.store.set_auth_token(&session_id, token).await?;
        }
        
        // Trigger registration
        self.state_machine.process_event(
            &session_id,
            EventType::Register { expires: expires.unwrap_or(3600) }
        ).await?;
        
        Ok(session_id)
    }
    
    /// Update presence status
    pub async fn publish_presence(
        &self,
        session_id: &SessionId,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::PublishPresence { 
                status: status.to_string(),
                note,
            }
        ).await
    }
    
    /// Subscribe to another user's presence
    pub async fn subscribe_presence(
        &self,
        session_id: &SessionId,
        target: &str,
        expires: Option<u32>,
    ) -> Result<String> {
        // Process through state machine
        self.state_machine.process_event(
            session_id,
            EventType::SubscribePresence {
                target: target.to_string(),
                expires: expires.unwrap_or(3600),
            }
        ).await?;
        
        // Return subscription ID
        self.store.get_last_subscription_id(session_id).await
    }
}
```

## Phase 7: Timer Management

### 7.1 Registration Refresh Timer

**File: `src/session_store/timers.rs`**

```rust
pub struct RegistrationTimerManager {
    timers: Arc<DashMap<SessionId, JoinHandle<()>>>,
}

impl RegistrationTimerManager {
    pub fn start_refresh_timer(
        &self,
        session_id: SessionId,
        expires: u32,
        state_machine: Arc<StateMachine>,
    ) {
        // Cancel existing timer
        if let Some((_, handle)) = self.timers.remove(&session_id) {
            handle.abort();
        }
        
        // Start new timer at 90% of expiry time
        let refresh_after = Duration::from_secs((expires * 9 / 10) as u64);
        
        let handle = tokio::spawn(async move {
            tokio::time::sleep(refresh_after).await;
            
            // Trigger refresh event
            let _ = state_machine.process_event(
                &session_id,
                EventType::RegistrationRefresh,
            ).await;
        });
        
        self.timers.insert(session_id, handle);
    }
}
```

## Phase 8: Missing Protocol Support

### 8.1 Complete Missing Event Types in session-core-v2

**File: `src/state_table/types.rs`**

Add these missing event types:

```rust
pub enum EventType {
    // ... existing events ...
    
    // Missing dialog events
    DialogUPDATE { sdp: Option<String> },
    DialogMESSAGE { from: String, body: String },
    DialogPUBLISH { event: String, body: Option<String> },
    DialogOPTIONS { from: String },
    DialogINFO { body: Option<String> },
    DialogPRACK { rack: String },
    DialogREINVITE { sdp: Option<String> },
}

pub enum Action {
    // ... existing actions ...
    
    // Missing SIP method actions
    SendUPDATE,
    SendOPTIONS,
    SendINFO,
    SendMESSAGE,
    SendPRACK,
    ProcessUPDATE,
    ProcessMESSAGE,
    ProcessOPTIONS,
    ProcessINFO,
}
```

### 8.2 Complete MESSAGE Support in dialog-core

**File: `dialog-core/src/protocol/message_handler.rs`** (NEW)

```rust
//! MESSAGE Request Handler for Dialog-Core
//!
//! This module handles MESSAGE requests according to RFC 3428.

use std::net::SocketAddr;
use tracing::debug;
use rvoip_sip_core::{Request, StatusCode};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator};

pub trait MessageHandler {
    fn handle_message_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

impl MessageHandler for DialogManager {
    async fn handle_message_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing MESSAGE request from {}", source);
        
        // Extract message content
        let content_type = request.content_type()
            .map(|ct| ct.to_string())
            .unwrap_or_else(|| "text/plain".to_string());
        
        let body = request.body().to_vec();
        
        // Create server transaction
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for MESSAGE: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Check if this is in-dialog or out-of-dialog
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // In-dialog MESSAGE
            debug!("MESSAGE is within dialog {}", dialog_id);
            
            let event = SessionCoordinationEvent::InstantMessage {
                dialog_id: Some(dialog_id),
                transaction_id,
                from: request.from_header().uri.to_string(),
                to: request.to_header().uri.to_string(),
                content_type,
                body,
            };
            
            self.notify_session_layer(event).await?;
        } else {
            // Out-of-dialog MESSAGE
            debug!("MESSAGE is out-of-dialog");
            
            let event = SessionCoordinationEvent::InstantMessage {
                dialog_id: None,
                transaction_id,
                from: request.from_header().uri.to_string(),
                to: request.to_header().uri.to_string(),
                content_type,
                body,
            };
            
            self.notify_session_layer(event).await?;
        }
        
        // Send 200 OK response
        let response = crate::transaction::utils::response_builders::create_response(
            &request,
            StatusCode::Ok
        );
        
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send MESSAGE response: {}", e),
            })?;
        
        debug!("MESSAGE request processed");
        Ok(())
    }
}
```

### 8.3 Update dialog-core Protocol Routing

**File: `dialog-core/src/protocol/mod.rs`**

```rust
//! - [`message_handler`]: MESSAGE requests (RFC 3428) - instant messaging

pub mod message_handler;
pub use message_handler::MessageHandler;
```

**File: `dialog-core/src/manager/core.rs`**

```rust
// In handle_request() method, add:
Method::Message => self.handle_message(request, source).await,
Method::Publish => {
    // PUBLISH is already handled via presence module
    self.handle_publish(request, source).await
}
```

### 8.4 Update Session Event Handler

**File: `src/adapters/session_event_handler.rs`**

Add handlers for new dialog events:

```rust
// In handle() method, add cases:
else if event_str.contains("DialogUPDATE") {
    self.handle_dialog_update(&event_str).await?;
} else if event_str.contains("DialogMESSAGE") {
    self.handle_dialog_message(&event_str).await?;
} else if event_str.contains("DialogPUBLISH") {
    self.handle_dialog_publish(&event_str).await?;
} else if event_str.contains("DialogOPTIONS") {
    self.handle_dialog_options(&event_str).await?;
} else if event_str.contains("DialogINFO") {
    self.handle_dialog_info(&event_str).await?;
}

// Add handler implementations:
async fn handle_dialog_update(&self, event_str: &str) -> Result<()> {
    let session_id = self.extract_session_id(event_str).unwrap_or_default();
    let sdp = self.extract_field(event_str, "sdp: \"");
    
    self.state_machine.process_event(
        &SessionId(session_id),
        EventType::DialogUPDATE { sdp }
    ).await?;
    
    Ok(())
}

async fn handle_dialog_message(&self, event_str: &str) -> Result<()> {
    let session_id = self.extract_session_id(event_str).unwrap_or_default();
    let from = self.extract_field(event_str, "from: \"").unwrap_or_default();
    let body = self.extract_field(event_str, "body: \"").unwrap_or_default();
    
    self.state_machine.process_event(
        &SessionId(session_id),
        EventType::DialogMESSAGE { from, body }
    ).await?;
    
    Ok(())
}
```

## Implementation Timeline

### Week 1-2: Core Infrastructure
- Add event types and states
- Create adapter modules
- Set up basic registration flow

### Week 3-4: State Machine Integration  
- Update state tables
- Implement action handlers
- Wire up event routing

### Week 5-6: Authentication & Presence
- Integrate auth-core
- Implement presence publishing
- Add subscription handling

### Week 7-8: Testing & Polish
- Integration tests
- Performance optimization
- Documentation

## Testing Strategy

### Unit Tests
- Test each adapter in isolation
- Mock registrar-core and auth-core
- Verify state transitions

### Integration Tests
```rust
#[tokio::test]
async fn test_full_registration_flow() {
    let coordinator = create_test_coordinator().await;
    
    // Register
    let session_id = coordinator.register(
        "sip:alice@example.com",
        Some("test-token"),
        Some(3600),
    ).await.unwrap();
    
    // Verify registered state
    let session = coordinator.get_session(&session_id).await.unwrap();
    assert_eq!(session.state, CallState::Registered);
    assert!(session.is_registered);
}

#[tokio::test]
async fn test_presence_flow() {
    // Register first
    let session_id = register_user("alice").await;
    
    // Publish presence
    coordinator.publish_presence(
        &session_id,
        PresenceStatus::Available,
        Some("At my desk".to_string()),
    ).await.unwrap();
    
    // Subscribe to bob's presence
    let sub_id = coordinator.subscribe_presence(
        &session_id,
        "sip:bob@example.com",
        None,
    ).await.unwrap();
    
    // Should receive NOTIFY
    // ... verify NOTIFY handling
}
```

## Considerations

### RFC Compliance
- **RFC 3261**: Basic SIP including REGISTER, INVITE, BYE, CANCEL, ACK, OPTIONS, INFO
- **RFC 3262**: PRACK (Reliable provisional responses) - placeholder, returns 501
- **RFC 3311**: UPDATE (Session modification)
- **RFC 3265/6665**: SUBSCRIBE/NOTIFY (SIP Events)
- **RFC 3428**: MESSAGE (Instant messaging)
- **RFC 3515**: REFER (Call transfer)
- **RFC 3856**: Presence Event Package (event: presence)
- **RFC 3863**: PIDF format for presence documents
- **RFC 3903**: PUBLISH (Presence state publication)
- **RFC 4479**: Presence data model (multiple devices per user)

### Security
- All REGISTER requests must include Authorization header
- OAuth2 bearer tokens for authentication
- Token validation cached for performance
- Support for token refresh

### Performance
- Registration refresh timers use tokio tasks
- Presence updates batched for multiple watchers
- PIDF generation cached
- Subscription state in memory

### Backward Compatibility
- session-core v1 patterns used as reference
- Similar API surface for easy migration
- Event naming consistent with existing patterns

## Summary

This implementation plan provides a complete path to add REGISTER, presence support, and full SIP method coverage to session-core-v2. It leverages existing libraries (auth-core, registrar-core) while maintaining the clean state machine architecture of session-core-v2. The phased approach allows for incremental development and testing.

Key benefits:
1. **Reuse**: Leverages battle-tested registrar-core and auth-core
2. **Clean Architecture**: Fits naturally into session-core-v2's adapter pattern
3. **RFC Compliant**: Follows all relevant SIP RFCs (3261, 3262, 3311, 3265/6665, 3428, 3515, 3856, 3863, 3903, 4479)
4. **Complete Coverage**: Supports all major SIP methods including REGISTER, UPDATE, MESSAGE, PUBLISH, SUBSCRIBE/NOTIFY
5. **Extensible**: Easy to add more presence features and complete PRACK support later
6. **Testable**: Each component can be tested in isolation
