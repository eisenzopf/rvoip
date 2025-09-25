# Implementation Plan: REGISTER and Presence Support for session-core-v2

## Executive Summary

This plan outlines how to add REGISTER and presence capabilities to session-core-v2, integrating directly with users-core for authentication and registrar-core for registration/presence storage while maintaining RFC compliance (3261, 3856, 3863, 4479). The implementation leverages session-core-v2's state machine architecture and event-driven design.

**Important Update**: A new `users-core` crate has been created to handle user management and JWT token issuance. For this implementation, we'll integrate directly with users-core for authentication:
- `users-core`: Manages internal users, authenticates passwords, issues JWT tokens
- Direct JWT validation using users-core's public key
- Integration with auth-core can be added later for OAuth2 support
- See `/crates/users-core/README.md` for details

**Authentication Flow**:
1. User authenticates with users-core (username/password) and receives JWT token
2. User includes JWT token in SIP REGISTER Authorization header
3. session-core-v2 validates JWT using users-core's JWKS endpoint
4. Registration proceeds if token is valid

## Quick Start for Developers

To implement this plan, start with these files in order:

1. **Add Event Types**: `session-core-v2/src/state_table/types.rs`
2. **Create Adapters**: 
   - `session-core-v2/src/adapters/registrar_adapter.rs` (new file)
   - `session-core-v2/src/adapters/presence_adapter.rs` (new file)
   - `session-core-v2/src/adapters/auth_adapter.rs` (new file)
3. **Update State Tables**: `session-core-v2/state_tables/sip_client_states.yaml`
4. **Update Event Handler**: `session-core-v2/src/adapters/session_event_handler.rs`
5. **Complete MESSAGE in dialog-core**: `dialog-core/src/protocol/message_handler.rs` (new file)

## Current Implementation Status

### ✅ Already Implemented

1. **session-core-v2**:
   - Registration and presence states in `CallState` enum (Registering, Registered, Unregistering, Subscribing, Subscribed, Publishing)
   - Basic state table definitions with registration/presence states
   - Adapter architecture (DialogAdapter, MediaAdapter, EventRouter)
   - SessionCrossCrateEventHandler for cross-crate events

2. **dialog-core**:
   - REGISTER handler (`protocol/register_handler.rs`) ✓
   - SUBSCRIBE/NOTIFY handlers with SubscriptionManager ✓
   - PUBLISH support (`presence/publish.rs`) ✓
   - MESSAGE support (partial implementation exists)
   - UPDATE, INFO, OPTIONS, REFER handlers ✓

3. **users-core**:
   - JWT token issuance with RS256 ✓
   - JWKS endpoint at `/auth/jwks.json` ✓
   - User authentication service ✓
   - REST API for user management ✓

4. **registrar-core**:
   - Complete registration management (Registrar) ✓
   - Complete presence management (Presence) ✓
   - PIDF XML support ✓
   - Subscription management ✓
   - Event publishing ✓
   - RegistrarService API ✓

### ❌ Needs Implementation

1. **session-core-v2 Event Types**:
   - DialogREGISTER, DialogSUBSCRIBE, DialogNOTIFY, DialogPUBLISH events
   - Registration events: Register, Unregister, RegistrationSuccess, RegistrationFailure
   - Presence events: PublishPresence, SubscribePresence, UnsubscribePresence, PresenceNotify
   - Missing dialog events: DialogMESSAGE, DialogUPDATE, DialogOPTIONS, DialogINFO

2. **New Adapters**:
   - RegistrarAdapter (integrate with registrar-core)
   - PresenceAdapter (handle presence operations)
   - AuthAdapter (JWT validation with users-core)

3. **State Machine Updates**:
   - State transitions for registration and presence in YAML files
   - Action handlers for new operations
   - Guards for authentication validation

4. **Session Store Extensions**:
   - Registration and presence state fields
   - User claims storage from JWT
   - Subscription tracking

5. **MESSAGE Handler Completion**:
   - Complete MESSAGE handler implementation in dialog-core

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
│ │ • REGISTER      │ │ • SUBSCRIBE      │ │ • JWT validation   │   │
│ │ • Location      │ │ • NOTIFY         │ │ • User context     │   │
│ │ • Expiry        │ │ • PUBLISH        │ │ • Token decode     │   │
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
         │  registrar-core  │ │   users-core     │ │  media-core      │
         │                  │ │                  │ │                  │
         │ • User registry  │ │ • JWT issuer     │ │ • RTP/SRTP       │
         │ • Presence store │ │ • Public key     │ │ • Codecs         │
         │ • PIDF support   │ │ • User store     │ │ • Audio devices  │
         └──────────────────┘ └──────────────────┘ └──────────────────┘
                                        │
                                        │ Direct JWT validation
                                        ▼
                               ┌─────────────────┐
                               │  JWKS endpoint  │
                               │ /auth/jwks.json │
                               └─────────────────┘
```

## Phase 1: Core Infrastructure

### 1.1 Add Missing Event Types to session-core-v2

**File: `src/state_table/types.rs`**

Note: CallState already has registration/presence states. Only need to add missing EventType variants.

```rust
pub enum EventType {
    // ... existing events ...
    
    // Registration events (NEW)
    Register { expires: u32, auth_token: Option<String> },
    Unregister,
    RegistrationSuccess { expires: u32 },
    RegistrationFailure { code: u16, reason: String },
    RegistrationExpiring,
    RegistrationRefresh,
    
    // Presence events (NEW)
    PublishPresence { status: String, note: Option<String> },
    SubscribePresence { target: String, expires: u32 },
    UnsubscribePresence { subscription_id: String },
    PresenceNotify { from: String, status: String, note: Option<String> },
    
    // Dialog events for REGISTER/SUBSCRIBE/NOTIFY/PUBLISH (NEW)
    DialogREGISTER { from: String, contact: String, expires: u32, auth_header: Option<String> },
    DialogSUBSCRIBE { from: String, event: String, expires: u32 },
    DialogNOTIFY { subscription_state: String, body: Option<String> },
    DialogPUBLISH { event: String, body: Option<String> },
    DialogOPTIONS { from: String },
    DialogINFO { body: Option<String> },
    DialogUPDATE { sdp: Option<String> },
    DialogMESSAGE { from: String, body: String },
}
```

### 1.2 Registration and Presence States Already Exist

**File: `src/types.rs`** - ALREADY HAS:

```rust
pub enum CallState {
    // ... existing states ...
    
    // Registration states (ALREADY EXISTS)
    Registering,
    Registered,
    Unregistering,
    
    // Subscription/Presence states (ALREADY EXISTS)
    Subscribing,
    Subscribed,
    Publishing,
    
    // Gateway/B2BUA states (ALREADY EXISTS)
    BridgeInitiating,
    BridgeActive,
}
```

Only need to add combined states if needed:
```rust
    // Combined states (NEW - optional)
    RegisteredIdle,
    RegisteredCalling,
    RegisteredInCall,
```

### 1.3 Create Adapter Modules

**File: `src/adapters/registrar_adapter.rs`** (NEW)

```rust
use std::sync::Arc;
use chrono::{Utc, Duration};
use rvoip_registrar_core::{RegistrarService, ContactInfo, Transport};
use crate::{SessionId, SessionStore, Result};
use super::AuthAdapter;

pub struct RegistrarAdapter {
    registrar: Arc<RegistrarService>,
    auth_adapter: Arc<AuthAdapter>,
    store: Arc<SessionStore>,
}

impl RegistrarAdapter {
    pub fn new(
        registrar: Arc<RegistrarService>,
        auth_adapter: Arc<AuthAdapter>,
        store: Arc<SessionStore>,
    ) -> Self {
        Self {
            registrar,
            auth_adapter,
            store,
        }
    }
    
    pub async fn handle_register(
        &self,
        session_id: &SessionId,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        auth_header: Option<String>,
    ) -> Result<()> {
        // 1. Validate authentication if header present
        if let Some(auth) = auth_header {
            let user_claims = self.auth_adapter.validate_sip_auth(&auth).await?;
            // Store user context in session
            self.store.set_user_context(session_id, user_claims).await?;
        } else {
            // No auth header - return 401 Unauthorized
            return Err(SessionError::Unauthorized("REGISTER requires authentication".into()));
        }
        
        // 2. Extract user ID from URI (e.g., "sip:alice@example.com" -> "alice")
        let user_id = extract_user_from_uri(&from_uri)?;
        
        // 3. Create contact info for registrar-core
        let contact_info = ContactInfo {
            uri: contact_uri.clone(),
            expires: Utc::now() + Duration::seconds(expires as i64),
            q_value: 1.0,
            call_id: session_id.to_string(),
            cseq: 1,
            user_agent: Some("rvoip/1.0".to_string()),
            transport: Transport::UDP, // TODO: Extract from contact URI
            instance_id: None,
            reg_id: None,
        };
        
        // 4. Register with registrar-core
        self.registrar.register_user(&user_id, contact_info, Some(expires)).await?;
        
        // 5. Update session state
        self.store.update_registration_state(session_id, true, expires).await?;
        
        Ok(())
    }
    
    pub async fn handle_unregister(
        &self,
        session_id: &SessionId,
        user_uri: String,
    ) -> Result<()> {
        let user_id = extract_user_from_uri(&user_uri)?;
        
        // Unregister from registrar-core
        self.registrar.unregister_user(&user_id).await?;
        
        // Update session state
        self.store.update_registration_state(session_id, false, 0).await?;
        
        Ok(())
    }
}

fn extract_user_from_uri(uri: &str) -> Result<String> {
    // Extract username from SIP URI (e.g., "sip:alice@example.com" -> "alice")
    if let Some(user_part) = uri.strip_prefix("sip:").and_then(|s| s.split('@').next()) {
        Ok(user_part.to_string())
    } else {
        Err(SessionError::InvalidUri(uri.to_string()))
    }
}
```

**File: `src/adapters/presence_adapter.rs`** (NEW)

```rust
use std::sync::Arc;
use dashmap::DashMap;
use rvoip_registrar_core::{RegistrarService, PresenceStatus};
use crate::{SessionId, DialogId, SessionStore, Result};
use super::DialogAdapter;

pub struct PresenceAdapter {
    registrar: Arc<RegistrarService>,
    dialog_adapter: Arc<DialogAdapter>,
    store: Arc<SessionStore>,
    subscription_dialogs: Arc<DashMap<String, DialogId>>,
}

impl PresenceAdapter {
    pub fn new(
        registrar: Arc<RegistrarService>,
        dialog_adapter: Arc<DialogAdapter>,
        store: Arc<SessionStore>,
    ) -> Self {
        Self {
            registrar,
            dialog_adapter,
            store,
            subscription_dialogs: Arc::new(DashMap::new()),
        }
    }
    
    pub async fn handle_publish(
        &self,
        session_id: &SessionId,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Get user ID from session
        let user_id = self.store.get_user_id(session_id).await?;
        
        // Update presence in registrar-core
        self.registrar.update_presence(&user_id, status, note).await?;
        
        Ok(())
    }
    
    pub async fn handle_subscribe(
        &self,
        session_id: &SessionId,
        target: String,
        expires: u32,
    ) -> Result<String> {
        // Get subscriber ID from session
        let subscriber = self.store.get_user_id(session_id).await?;
        
        // Extract target user ID from SIP URI
        let target_user = extract_user_from_uri(&target)?;
        
        // Create subscription in registrar-core
        let subscription_id = self.registrar.subscribe_presence(
            &subscriber,
            &target_user,
            Some(expires)
        ).await?;
        
        // Store dialog mapping for NOTIFY routing
        if let Some(dialog_id) = self.store.get_dialog_id(session_id).await? {
            self.subscription_dialogs.insert(subscription_id.clone(), dialog_id);
        }
        
        // Store subscription in session
        self.store.add_subscription(session_id, subscription_id.clone(), target).await?;
        
        Ok(subscription_id)
    }
    
    pub async fn handle_unsubscribe(
        &self,
        session_id: &SessionId,
        subscription_id: String,
    ) -> Result<()> {
        // Unsubscribe in registrar-core
        self.registrar.unsubscribe_presence(&subscription_id).await?;
        
        // Remove dialog mapping
        self.subscription_dialogs.remove(&subscription_id);
        
        // Remove from session
        self.store.remove_subscription(session_id, &subscription_id).await?;
        
        Ok(())
    }
    
    pub async fn send_notify(
        &self,
        subscription_id: &str,
        presence_xml: String,
    ) -> Result<()> {
        // Find dialog for this subscription
        if let Some((_, dialog_id)) = self.subscription_dialogs.get(subscription_id) {
            // Send NOTIFY through dialog adapter
            self.dialog_adapter.send_notify(
                &dialog_id,
                "presence",
                Some(presence_xml),
            ).await?;
        }
        
        Ok(())
    }
}

fn extract_user_from_uri(uri: &str) -> Result<String> {
    // Reuse the same helper function
    if let Some(user_part) = uri.strip_prefix("sip:").and_then(|s| s.split('@').next()) {
        Ok(user_part.to_string())
    } else {
        Err(SessionError::InvalidUri(uri.to_string()))
    }
}
```

## Phase 2: State Machine Integration

### 2.1 Update State Tables

Note: Some registration/presence states already exist in `state_tables/sip_client_states.yaml` and `enhanced_state_table.yaml`. Need to add transitions and complete the implementation.

**File: `state_tables/sip_client_states.yaml`** - ADD TRANSITIONS:

```yaml
# Add to existing transitions section:
transitions:
  # Registration flow (NEW)
  - role: "Both"
    state: "Idle"
    event:
      type: "Register"
    actions:
      - type: "CreateSession"
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
      
  - role: "Both"
    state: "Registering"
    event:
      type: "DialogREGISTER"
    guards:
      - type: "NoAuthentication"
    actions:
      - type: "SendSIPResponse"
        args:
          code: 401
          reason: "Unauthorized"
          headers:
            WWW-Authenticate: "Bearer realm=\"rvoip\""
    next_state: "Idle"
    
  # Unregister flow (NEW)
  - role: "Both"
    state: "Registered"
    event:
      type: "Unregister"
    actions:
      - type: "SendREGISTER"
        args:
          expires: 0
    next_state: "Unregistering"
    
  - role: "Both"
    state: "Unregistering"
    event:
      type: "RegistrationSuccess"
    actions:
      - type: "ClearRegistration"
    next_state: "Idle"
    effects:
      - type: "StopRegistrationTimer"
      
  # Registration refresh (NEW)
  - role: "Both"
    state: "Registered"
    event:
      type: "RegistrationExpiring"
    actions:
      - type: "RefreshRegistration"
    next_state: "Registered"
    
  # Presence subscription (NEW)
  - role: "Both"
    state: "Registered"
    event:
      type: "SubscribePresence"
    actions:
      - type: "SendSUBSCRIBE"
    next_state: "Subscribing"
    
  - role: "Both"
    state: "Subscribing"
    event:
      type: "DialogSUBSCRIBE"
    actions:
      - type: "ProcessSUBSCRIBE"
      - type: "SendSIPResponse"
        args:
          code: 200
          reason: "OK"
    next_state: "Subscribed"
    
  # Presence publishing (NEW)
  - role: "Both"  
    state: "Registered"
    event:
      type: "PublishPresence"
    actions:
      - type: "SendPUBLISH"
    next_state: "Publishing"
    
  - role: "Both"
    state: "Publishing"
    event:
      type: "DialogPUBLISH"
    actions:
      - type: "ProcessPUBLISH"
      - type: "SendSIPResponse"
        args:
          code: 200
          reason: "OK"
    next_state: "Registered"
    
  # Presence notification (NEW)
  - role: "Both"
    state: "Subscribed"
    event:
      type: "DialogNOTIFY"
    actions:
      - type: "ProcessPresenceNotify"
      - type: "SendSIPResponse"
        args:
          code: 200
          reason: "OK"
    next_state: "Subscribed"
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
use rvoip_users_core::{UserClaims, JwtIssuer};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use reqwest::Client;

pub struct AuthAdapter {
    jwks_endpoint: String,
    public_key: Option<DecodingKey>,
    http_client: Client,
}

impl AuthAdapter {
    pub async fn new(users_core_url: &str) -> Result<Self> {
        let jwks_endpoint = format!("{}/auth/jwks.json", users_core_url);
        let http_client = Client::new();
        
        // Fetch public key at startup
        let public_key = Self::fetch_public_key(&http_client, &jwks_endpoint).await.ok();
        
        Ok(Self {
            jwks_endpoint,
            public_key,
            http_client,
        })
    }
    
    async fn fetch_public_key(client: &Client, jwks_url: &str) -> Result<DecodingKey> {
        let jwks = client.get(jwks_url).send().await?
            .json::<serde_json::Value>().await?;
        
        // Extract RSA public key from JWKS
        let key = &jwks["keys"][0];
        let n = key["n"].as_str().ok_or("Missing modulus")?;
        let e = key["e"].as_str().ok_or("Missing exponent")?;
        
        // Build RSA key from components
        DecodingKey::from_rsa_components(n, e)
            .map_err(|e| anyhow::anyhow!("Invalid RSA key: {}", e))
    }
    
    pub async fn validate_sip_auth(
        &self,
        auth_header: &str,
    ) -> Result<UserClaims> {
        // Extract bearer token
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| anyhow::anyhow!("Invalid authorization header"))?;
        
        // Get decoding key (fetch if not cached)
        let decoding_key = match &self.public_key {
            Some(key) => key.clone(),
            None => {
                let key = Self::fetch_public_key(&self.http_client, &self.jwks_endpoint).await?;
                key
            }
        };
        
        // Configure validation
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["https://users.rvoip.local"]);
        validation.set_audience(&["rvoip-api", "rvoip-sip"]);
        
        // Decode and validate JWT
        let token_data = decode::<UserClaims>(token, &decoding_key, &validation)
            .map_err(|e| anyhow::anyhow!("JWT validation failed: {}", e))?;
        
        Ok(token_data.claims)
    }
    
    pub fn generate_www_authenticate(&self) -> String {
        "Bearer realm=\"rvoip\"".to_string()
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
    pub user_claims: Option<UserClaims>,  // JWT claims from users-core
    
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

Note: MESSAGE method is not currently implemented in dialog-core despite being referenced in transaction layer. Need to add complete handler.

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
// Add to existing protocol handlers:
pub mod message_handler;
pub use message_handler::MessageHandler;
```

**File: `dialog-core/src/manager/core.rs`**

```rust
// In handle_request() method, add MESSAGE case:
match request.method() {
    Method::Invite => self.handle_invite(request, source).await,
    Method::Bye => self.handle_bye(request).await,
    Method::Cancel => self.handle_cancel(request).await,
    Method::Ack => self.handle_ack(request).await,
    Method::Options => self.handle_options(request, source).await,
    Method::Register => self.handle_register(request, source).await,
    Method::Update => self.handle_update(request).await,
    Method::Info => self.handle_info(request, source).await,
    Method::Refer => self.handle_refer(request, source).await,
    Method::Subscribe => self.handle_subscribe(request, source).await,
    Method::Notify => self.handle_notify(request, source).await,
    Method::Message => self.handle_message(request, source).await,  // NEW
    Method::Publish => self.handle_publish(request, source).await,  // NEW
    method => {
        warn!("Unsupported SIP method: {}", method);
        Err(DialogError::protocol_error(&format!("Unsupported method: {}", method)))
    }
}

// Add handler methods:
pub async fn handle_message(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
    <Self as super::protocol_handlers::MessageHandler>::handle_message_method(self, request, source).await
}

pub async fn handle_publish(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
    // PUBLISH is handled via presence module
    // For now, forward to session layer
    // TODO: Integrate with presence/publish.rs
    let from_uri = request.from()
        .ok_or_else(|| DialogError::protocol_error("PUBLISH missing From header"))?
        .uri().clone();
    
    let event = "presence"; // Default event type
    let body = request.body().to_vec();
    
    let server_transaction = self.transaction_manager
        .create_server_transaction(request.clone(), source)
        .await
        .map_err(|e| DialogError::TransactionError {
            message: format!("Failed to create server transaction for PUBLISH: {}", e),
        })?;
    
    let transaction_id = server_transaction.id().clone();
    
    let event = SessionCoordinationEvent::PublishRequest {
        transaction_id,
        from_uri,
        event_type: event.to_string(),
        body,
    };
    
    self.notify_session_layer(event).await
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

### Phase 1: Core Infrastructure (3-4 days)
- ✅ Review existing implementations
- Add missing event types to session-core-v2
- Create RegistrarAdapter, PresenceAdapter, AuthAdapter
- Update session store with registration/presence fields

### Phase 2: State Machine Integration (2-3 days)
- Update state tables with registration/presence transitions
- Implement action handlers for new operations
- Add authentication guards
- Wire up event routing in SessionCrossCrateEventHandler

### Phase 3: Authentication Integration (2 days)
- Create AuthAdapter for JWT validation
- Integrate with users-core JWKS endpoint
- Add authentication to REGISTER flow
- Handle 401 Unauthorized responses

### Phase 4: Registration Flow (2-3 days)
- Complete REGISTER handling end-to-end
- Implement registration refresh timers
- Add de-registration support
- Test multi-device registration

### Phase 5: Presence Implementation (3-4 days)
- Complete PUBLISH flow
- Implement SUBSCRIBE/NOTIFY handling
- Add presence state management
- Test presence updates and notifications

### Phase 6: Testing & Polish (2-3 days)
- Integration tests for all flows
- Performance optimization
- Documentation updates
- Example applications

**Total Estimated Time: 2-3 weeks**

## Detailed Task List

### Phase 1: Core Infrastructure (3-4 days)

#### Event Types and States
- [ ] Add missing EventType variants to `session-core-v2/src/state_table/types.rs`:
  - [ ] Register { expires: u32, auth_token: Option<String> }
  - [ ] Unregister
  - [ ] RegistrationSuccess { expires: u32 }
  - [ ] RegistrationFailure { code: u16, reason: String }
  - [ ] RegistrationExpiring
  - [ ] RegistrationRefresh
  - [ ] PublishPresence { status: String, note: Option<String> }
  - [ ] SubscribePresence { target: String, expires: u32 }
  - [ ] UnsubscribePresence { subscription_id: String }
  - [ ] PresenceNotify { from: String, status: String, note: Option<String> }
  - [ ] DialogREGISTER { from: String, contact: String, expires: u32, auth_header: Option<String> }
  - [ ] DialogSUBSCRIBE { from: String, event: String, expires: u32 }
  - [ ] DialogNOTIFY { subscription_state: String, body: Option<String> }
  - [ ] DialogPUBLISH { event: String, body: Option<String> }
  - [ ] DialogOPTIONS { from: String }
  - [ ] DialogINFO { body: Option<String> }
  - [ ] DialogUPDATE { sdp: Option<String> }
  - [ ] DialogMESSAGE { from: String, body: String }

#### Create New Adapters
- [ ] Create `session-core-v2/src/adapters/auth_adapter.rs`:
  - [ ] Implement AuthAdapter struct
  - [ ] Add new() constructor with users-core URL
  - [ ] Implement fetch_public_key() for JWKS
  - [ ] Implement validate_sip_auth() for JWT validation
  - [ ] Implement generate_www_authenticate()
  - [ ] Add tests for JWT validation

- [ ] Create `session-core-v2/src/adapters/registrar_adapter.rs`:
  - [ ] Implement RegistrarAdapter struct
  - [ ] Add new() constructor
  - [ ] Implement handle_register()
  - [ ] Implement handle_unregister()
  - [ ] Add extract_user_from_uri() helper
  - [ ] Add tests for registration flow

- [ ] Create `session-core-v2/src/adapters/presence_adapter.rs`:
  - [ ] Implement PresenceAdapter struct
  - [ ] Add new() constructor
  - [ ] Implement handle_publish()
  - [ ] Implement handle_subscribe()
  - [ ] Implement handle_unsubscribe()
  - [ ] Implement send_notify()
  - [ ] Add tests for presence operations

- [ ] Update `session-core-v2/src/adapters/mod.rs`:
  - [ ] Add module declarations for new adapters
  - [ ] Add public re-exports

#### Session Store Extensions
- [ ] Update `session-core-v2/src/session_store/state.rs`:
  - [ ] Add is_registered: bool field
  - [ ] Add registration_expires: Option<u32> field
  - [ ] Add registration_timer: Option<tokio::task::JoinHandle<()>> field
  - [ ] Add user_claims: Option<UserClaims> field
  - [ ] Add presence_status: PresenceStatus field
  - [ ] Add presence_note: Option<String> field
  - [ ] Add presence_subscriptions: Vec<SubscriptionInfo> field
  - [ ] Add presence_target: Option<String> field

- [ ] Create SubscriptionInfo struct in session store
- [ ] Add helper methods to SessionStore:
  - [ ] set_user_context()
  - [ ] get_user_id()
  - [ ] update_registration_state()
  - [ ] add_subscription()
  - [ ] remove_subscription()
  - [ ] get_dialog_id()

### Phase 2: State Machine Integration (2-3 days)

#### State Table Updates
- [ ] Update `session-core-v2/state_tables/sip_client_states.yaml`:
  - [ ] Add registration flow transitions (Idle → Registering → Registered)
  - [ ] Add unregister flow transitions
  - [ ] Add registration refresh transitions
  - [ ] Add presence subscription transitions
  - [ ] Add presence publishing transitions
  - [ ] Add presence notification handling

#### Action Handlers
- [ ] Update `session-core-v2/src/state_table/types.rs` Action enum:
  - [ ] SendREGISTER
  - [ ] ProcessREGISTER
  - [ ] RefreshRegistration
  - [ ] ClearRegistration
  - [ ] SendSUBSCRIBE
  - [ ] SendNOTIFY
  - [ ] SendPUBLISH
  - [ ] UpdatePresenceState
  - [ ] ProcessPresenceNotify
  - [ ] SendUPDATE
  - [ ] SendOPTIONS
  - [ ] SendINFO
  - [ ] SendMESSAGE
  - [ ] SendPRACK

- [ ] Implement action handlers in `session-core-v2/src/state_machine/actions.rs`:
  - [ ] handle_send_register()
  - [ ] handle_process_register()
  - [ ] handle_send_subscribe()
  - [ ] handle_send_publish()
  - [ ] handle_send_notify()
  - [ ] handle_update_presence_state()

#### Guards
- [ ] Add new guards to `session-core-v2/src/state_machine/guards.rs`:
  - [ ] HasAuthentication
  - [ ] NoAuthentication
  - [ ] IsRegistered
  - [ ] HasActiveSubscription

#### Event Routing
- [ ] Update `session-core-v2/src/adapters/session_event_handler.rs`:
  - [ ] Add handle_dialog_register()
  - [ ] Add handle_dialog_subscribe()
  - [ ] Add handle_dialog_notify()
  - [ ] Add handle_dialog_publish()
  - [ ] Add handle_dialog_update()
  - [ ] Add handle_dialog_message()
  - [ ] Add handle_dialog_options()
  - [ ] Add handle_dialog_info()
  - [ ] Update handle() method with new event cases

### Phase 3: Authentication Integration (2 days)

- [ ] Configure AuthAdapter in UnifiedCoordinator:
  - [ ] Add users_core_url configuration parameter
  - [ ] Initialize AuthAdapter in create()
  - [ ] Pass AuthAdapter to RegistrarAdapter

- [ ] Add authentication error handling:
  - [ ] Handle 401 Unauthorized responses
  - [ ] Add retry logic with credentials
  - [ ] Update SessionError with Unauthorized variant

- [ ] Create authentication tests:
  - [ ] Test valid JWT token validation
  - [ ] Test expired token handling
  - [ ] Test invalid token handling
  - [ ] Test missing authentication

### Phase 4: Registration Flow (2-3 days)

- [ ] Complete registration flow integration:
  - [ ] Wire up RegistrarAdapter in UnifiedCoordinator
  - [ ] Connect to registrar-core service
  - [ ] Test REGISTER request handling
  - [ ] Test authentication validation
  - [ ] Test successful registration
  - [ ] Test registration failure cases

- [ ] Implement registration timers:
  - [ ] Create RegistrationTimerManager
  - [ ] Implement start_refresh_timer()
  - [ ] Implement stop_timer()
  - [ ] Add timer cleanup on shutdown

- [ ] Add registration API methods to UnifiedCoordinator:
  - [ ] register()
  - [ ] unregister()
  - [ ] is_registered()
  - [ ] get_registration_info()

- [ ] Test multi-device scenarios:
  - [ ] Multiple registrations for same user
  - [ ] Contact priority handling
  - [ ] Parallel forking support

### Phase 5: Presence Implementation (3-4 days)

- [ ] Complete PUBLISH flow:
  - [ ] Wire up PresenceAdapter in UnifiedCoordinator
  - [ ] Test presence updates
  - [ ] Test PIDF XML generation
  - [ ] Handle publish errors

- [ ] Complete SUBSCRIBE/NOTIFY flow:
  - [ ] Test presence subscriptions
  - [ ] Test NOTIFY generation
  - [ ] Test subscription expiry
  - [ ] Test unsubscribe

- [ ] Add presence API methods to UnifiedCoordinator:
  - [ ] publish_presence()
  - [ ] subscribe_presence()
  - [ ] unsubscribe_presence()
  - [ ] get_presence()
  - [ ] get_buddy_list()

- [ ] Integration with registrar-core:
  - [ ] Test automatic buddy lists (B2BUA mode)
  - [ ] Test presence aggregation
  - [ ] Test watcher notifications

### Phase 6: Dialog-Core MESSAGE Handler (1 day)

- [ ] Create `dialog-core/src/protocol/message_handler.rs`:
  - [ ] Implement MessageHandler trait
  - [ ] Implement handle_message_method()
  - [ ] Handle in-dialog vs out-of-dialog
  - [ ] Send 200 OK response

- [ ] Update `dialog-core/src/protocol/mod.rs`:
  - [ ] Add message_handler module
  - [ ] Export MessageHandler trait

- [ ] Update `dialog-core/src/manager/core.rs`:
  - [ ] Add Method::Message case in handle_request()
  - [ ] Add handle_message() method
  - [ ] Add Method::Publish case
  - [ ] Add handle_publish() method

- [ ] Add MESSAGE tests:
  - [ ] Test in-dialog MESSAGE
  - [ ] Test out-of-dialog MESSAGE
  - [ ] Test different content types

### Phase 7: Testing & Polish (2-3 days)

#### Integration Tests
- [ ] Create registration integration tests:
  - [ ] test_full_registration_flow()
  - [ ] test_registration_with_auth()
  - [ ] test_registration_refresh()
  - [ ] test_multi_device_registration()

- [ ] Create presence integration tests:
  - [ ] test_presence_publish()
  - [ ] test_presence_subscribe_notify()
  - [ ] test_buddy_list()
  - [ ] test_presence_aggregation()

- [ ] Create end-to-end tests:
  - [ ] test_register_then_call()
  - [ ] test_presence_during_call()
  - [ ] test_message_delivery()

#### Documentation
- [ ] Update session-core-v2 README with registration/presence examples
- [ ] Add registration flow diagram
- [ ] Add presence flow diagram
- [ ] Document authentication requirements
- [ ] Create example applications:
  - [ ] Basic SIP client with registration
  - [ ] Presence-enabled softphone
  - [ ] Instant messaging client

#### Performance Optimization
- [ ] Profile registration refresh timers
- [ ] Optimize presence notification batching
- [ ] Add connection pooling for registrar-core
- [ ] Implement presence caching

#### Final Cleanup
- [ ] Remove any debug logging
- [ ] Ensure all tests pass
- [ ] Run clippy and fix warnings
- [ ] Update CHANGELOG
- [ ] Create migration guide from session-core v1

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
- All REGISTER requests must include Authorization header with JWT Bearer token
- Direct JWT validation using users-core's JWKS endpoint
- Public key cached for performance
- Users must authenticate with users-core first to get JWT token
- Future: Support for OAuth2 tokens via auth-core integration

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

This implementation plan provides a complete path to add REGISTER, presence support, and full SIP method coverage to session-core-v2. The plan has been updated based on a thorough review of the existing codebase, showing that much of the infrastructure is already in place.

**What's Already Done:**
- dialog-core has REGISTER, SUBSCRIBE/NOTIFY, PUBLISH handlers ✓
- registrar-core has complete registration and presence management ✓
- users-core provides JWT authentication with JWKS endpoint ✓
- session-core-v2 has registration/presence states defined ✓

**What Needs Implementation:**
- Missing event types in session-core-v2 (DialogREGISTER, etc.)
- New adapters: RegistrarAdapter, PresenceAdapter, AuthAdapter
- State transitions for registration/presence in YAML files
- Session store extensions for registration state
- Event routing in SessionCrossCrateEventHandler
- MESSAGE handler completion in dialog-core

**Key Benefits:**
1. **Minimal New Code**: Most infrastructure already exists
2. **Direct Integration**: Uses users-core directly for JWT validation
3. **Reuse**: Leverages existing registrar-core and dialog-core implementations
4. **Clean Architecture**: Fits naturally into session-core-v2's adapter pattern
5. **RFC Compliant**: Follows all relevant SIP RFCs (3261, 3262, 3311, 3265/6665, 3428, 3515, 3856, 3863, 3903, 4479)
6. **Quick Implementation**: 2-3 weeks due to existing infrastructure
7. **Testable**: Each component can be tested in isolation

The implementation focuses on connecting existing components rather than building from scratch, making it a relatively straightforward integration project.
