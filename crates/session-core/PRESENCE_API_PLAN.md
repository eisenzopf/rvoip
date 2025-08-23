# Presence API Plan for Session-Core

## Overview

This document outlines the plan to add SIP presence functionality to session-core, providing a simple, developer-friendly API that works transparently in both P2P and B2BUA scenarios.

**Update**: With the addition of `registrar-core` crate, the architecture has been refined:
- **registrar-core** handles all state management (registration, presence, subscriptions)
- **session-core** handles SIP signaling and OAuth authentication
- Integration uses direct API calls (inbound) and EventBus (outbound notifications)

## Background

SIP presence uses the SIMPLE (SIP for Instant Messaging and Presence Leveraging Extensions) protocol with three key methods:
- **PUBLISH** - User agents report their current status
- **SUBSCRIBE** - Other parties subscribe to presence updates
- **NOTIFY** - Presence server notifies subscribers of status changes

## Design Goals

1. **Simplicity** - Hide PUBLISH/SUBSCRIBE/NOTIFY complexity
2. **Transparency** - Same API for P2P and B2BUA scenarios
3. **Symmetry** - Consistent with SimplePeer/SimpleCall design
4. **Event-driven** - Async updates via channels
5. **Rust-idiomatic** - Builder patterns, async/await

## API Design

### Core Types

```rust
/// Presence status states
pub enum PresenceStatus {
    Available,      // "open" in PIDF
    Busy,          // "closed" with busy note
    Away,          // "closed" with away note
    DoNotDisturb,  // "closed" with DND
    Offline,       // "closed"
    InCall,        // "open" with in-call note
    Custom(String), // Custom status
}

/// Rich presence information
pub struct PresenceInfo {
    pub status: PresenceStatus,
    pub note: Option<String>,
    pub device: Option<String>,
    pub location: Option<String>,
    pub capabilities: Vec<String>, // ["audio", "video", "chat"]
}
```

### SimplePeer Extensions

```rust
impl SimplePeer {
    /// Set my presence status
    pub fn presence(&self, status: PresenceStatus) -> PresenceBuilder;
    
    /// Watch another user's presence
    pub async fn watch(&self, target: &str) -> Result<PresenceWatcher>;
}
```

### Usage Examples

#### Simple P2P Presence
```rust
// Alice sets her status
alice.presence(PresenceStatus::Available)
    .note("Working from home")
    .await?;

// Bob watches Alice
let mut watcher = bob.watch("alice@192.168.1.100:5060").await?;

// Bob gets notified of changes
if let Some(status) = watcher.recv().await {
    println!("Alice is now: {:?}", status);
}
```

#### With B2BUA Server (OAuth 2.0 Authentication)
```rust
// Get OAuth token first (from your OAuth provider)
let token = oauth_client.get_token("alice", "password").await?;

// Register with B2BUA using Bearer token (RFC 8898)
alice.register("sip:pbx.company.com")
    .auth_bearer(token)  // OAuth 2.0 Bearer token
    .await?;

// B2BUA validates token and registers user
bob.register("sip:pbx.company.com")
    .auth_bearer(bob_token)
    .await?;

// Same API! B2BUA handles routing
alice.presence(PresenceStatus::Available).await?;
let mut watcher = bob.watch("alice").await?;  // B2BUA routes by username
```

#### Buddy List
```rust
let mut buddies = BuddyList::new();
buddies.add(&peer, "bob@example.com").await?;
buddies.add(&peer, "charlie@example.com").await?;

// Poll for updates
for (buddy, status) in buddies.poll().await {
    println!("{} is now {:?}", buddy, status);
}
```

## Architecture Integration (Updated with registrar-core)

### OAuth 2.0 Authentication Layer

**New authentication module in session-core:**

```rust
// session-core/src/auth/oauth.rs
use jsonwebtoken::{decode, decode_header, jwk, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// JWKS URI for JWT validation (e.g., https://auth.example.com/.well-known/jwks.json)
    pub jwks_uri: Option<String>,
    
    /// Token introspection endpoint (e.g., https://auth.example.com/oauth2/introspect)
    pub introspect_uri: Option<String>,
    
    /// Required scopes for different operations
    pub required_scopes: OAuth2Scopes,
    
    /// Cache validated tokens for performance
    pub cache_ttl: Duration,
    
    /// OAuth realm for WWW-Authenticate headers
    pub realm: String,
}

#[derive(Debug, Clone)]
pub struct OAuth2Scopes {
    pub register: Vec<String>,  // ["sip:register"]
    pub call: Vec<String>,      // ["sip:call"]
    pub presence: Vec<String>,  // ["sip:presence"]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub subject: String,         // User identity (maps to SIP user)
    pub scopes: Vec<String>,     // Granted permissions
    pub expires_at: DateTime<Utc>,
    pub client_id: String,       // Which SIP client
    pub realm: Option<String>,   // Authorization realm
}

pub struct OAuth2Validator {
    config: OAuth2Config,
    http_client: Client,
    jwks_cache: Arc<RwLock<Option<JwkSet>>>,
    token_cache: Arc<DashMap<String, (TokenInfo, Instant)>>,
}

impl OAuth2Validator {
    pub async fn new(config: OAuth2Config) -> Result<Self> {
        let mut validator = Self {
            config,
            http_client: Client::new(),
            jwks_cache: Arc::new(RwLock::new(None)),
            token_cache: Arc::new(DashMap::new()),
        };
        
        // Pre-fetch JWKS if configured
        if let Some(uri) = &validator.config.jwks_uri {
            validator.fetch_jwks().await?;
        }
        
        Ok(validator)
    }
    
    /// Validate a Bearer token from Authorization header
    pub async fn validate_bearer_token(&self, token: &str) -> Result<TokenInfo> {
        // Check cache first
        if let Some(cached) = self.get_cached_token(token) {
            return Ok(cached);
        }
        
        // Try JWT validation first (faster, no network)
        if let Ok(info) = self.validate_jwt(token).await {
            self.cache_token(token, &info);
            return Ok(info);
        }
        
        // Fall back to introspection (network call)
        if let Some(uri) = &self.config.introspect_uri {
            let info = self.introspect_token(token, uri).await?;
            self.cache_token(token, &info);
            return Ok(info);
        }
        
        Err(AuthError::InvalidToken("Unable to validate token"))
    }
    
    /// Check if token has required scopes for operation
    pub fn check_scopes(&self, token_info: &TokenInfo, operation: &str) -> bool {
        let required = match operation {
            "REGISTER" => &self.config.required_scopes.register,
            "INVITE" => &self.config.required_scopes.call,
            "PUBLISH" | "SUBSCRIBE" => &self.config.required_scopes.presence,
            _ => return true, // No scope requirement
        };
        
        required.iter().all(|scope| token_info.scopes.contains(scope))
    }
    
    /// Generate WWW-Authenticate header for 401 response
    pub fn www_authenticate_header(&self, error: Option<&str>) -> String {
        let mut header = format!("Bearer realm=\"{}\"", self.config.realm);
        if let Some(err) = error {
            header.push_str(&format!(", error=\"{}\"", err));
        }
        header
    }
}
```

### Integration with registrar-core

```rust
// session-core/src/coordinator/registrar_integration.rs
use registrar_core::{RegistrarService, RegistrarEvent, PresenceEvent};
use infra_common::events::api::EventSystem;

pub struct RegistrarIntegration {
    /// The registrar service instance
    registrar: Arc<RegistrarService>,
    
    /// OAuth validator for authentication
    oauth: Arc<OAuth2Validator>,
    
    /// Event bus for receiving notifications
    event_bus: Arc<EventSystem>,
    
    /// Mapping of subscription_id to SIP dialog
    subscription_dialogs: Arc<DashMap<String, DialogId>>,
}

impl RegistrarIntegration {
    pub async fn new(
        oauth_config: OAuth2Config,
        registrar_config: RegistrarConfig,
        event_bus: Arc<EventSystem>,
    ) -> Result<Self> {
        let oauth = Arc::new(OAuth2Validator::new(oauth_config).await?);
        let mut registrar = RegistrarService::new_b2bua().await?;
        registrar.set_event_bus(event_bus.clone());
        
        Ok(Self {
            registrar: Arc::new(registrar),
            oauth,
            event_bus,
            subscription_dialogs: Arc::new(DashMap::new()),
        })
    }
    
    /// Handle incoming REGISTER request
    pub async fn handle_register(
        &self,
        message: &SipMessage,
        token: Option<&str>,
    ) -> Result<SipMessage> {
        // 1. Validate OAuth token
        let token_info = match token {
            Some(t) => self.oauth.validate_bearer_token(t).await?,
            None => {
                // Return 401 Unauthorized with WWW-Authenticate
                return Ok(self.create_401_response(message));
            }
        };
        
        // 2. Check scopes
        if !self.oauth.check_scopes(&token_info, "REGISTER") {
            return Ok(self.create_403_response(message, "insufficient_scope"));
        }
        
        // 3. Extract contact info from SIP message
        let contact = self.extract_contact_info(message)?;
        let expires = message.get_expires().unwrap_or(3600);
        
        // 4. Register with registrar-core
        self.registrar.register_user(
            &token_info.subject,
            contact,
            Some(expires),
        ).await?;
        
        // 5. Create 200 OK response
        Ok(self.create_200_ok(message, expires))
    }
    
    /// Handle incoming PUBLISH request
    pub async fn handle_publish(
        &self,
        message: &SipMessage,
        token_info: &TokenInfo,
    ) -> Result<SipMessage> {
        // 1. Parse PIDF from body
        let pidf = message.body.as_ref()
            .ok_or(ProtocolError::MissingBody)?;
        let presence = self.registrar.parse_pidf(pidf).await?;
        
        // 2. Update presence in registrar-core
        self.registrar.update_presence(
            &token_info.subject,
            presence.status,
            presence.note,
        ).await?;
        
        // 3. Create 200 OK response
        Ok(self.create_200_ok(message, 3600))
    }
    
    /// Handle incoming SUBSCRIBE request
    pub async fn handle_subscribe(
        &self,
        message: &SipMessage,
        token_info: &TokenInfo,
        dialog_id: DialogId,
    ) -> Result<SipMessage> {
        // 1. Extract target from Request-URI
        let target = self.extract_subscribe_target(message)?;
        let expires = message.get_expires().unwrap_or(3600);
        
        // 2. Create subscription in registrar-core
        let subscription_id = self.registrar.subscribe_presence(
            &token_info.subject,
            &target,
            Some(expires),
        ).await?;
        
        // 3. Map subscription to dialog
        self.subscription_dialogs.insert(subscription_id.clone(), dialog_id);
        
        // 4. Create 200 OK response
        let response = self.create_200_ok(message, expires);
        
        // 5. Send immediate NOTIFY with current state
        self.send_initial_notify(&subscription_id, &target, dialog_id).await?;
        
        Ok(response)
    }
    
    /// Start event listener for registrar notifications
    pub async fn start_event_listener(&self) {
        let registrar = self.registrar.clone();
        let dialogs = self.subscription_dialogs.clone();
        
        // Subscribe to presence events
        let mut subscriber = self.event_bus.subscribe::<PresenceEvent>().await.unwrap();
        
        tokio::spawn(async move {
            while let Some(event) = subscriber.recv().await {
                match event {
                    PresenceEvent::Updated { user, status, note, watchers_notified } => {
                        // Generate NOTIFY for each watcher
                        for watcher in watchers_notified {
                            if let Some(dialog_id) = dialogs.get(&watcher) {
                                // Generate and send NOTIFY through session-core
                                let pidf = registrar.generate_pidf(&user).await.unwrap();
                                // ... send NOTIFY with PIDF body
                            }
                        }
                    }
                    PresenceEvent::SubscriptionExpired { subscription_id, .. } => {
                        // Remove dialog mapping
                        dialogs.remove(&subscription_id);
                    }
                    _ => {}
                }
            }
        });
    }
}
```

### Event System Integration (infra-common)

We'll leverage the existing global event architecture from `infra-common/src/events`:

1. **Define Presence Events**
```rust
// In session-core/src/events/presence_events.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PresenceEvent {
    /// Presence status published
    PresencePublished {
        entity: String,
        status: PresenceStatus,
        note: Option<String>,
        timestamp: DateTime<Utc>,
    },
    
    /// Subscription request received
    SubscriptionRequest {
        subscriber: String,
        target: String,
        expires: u32,
    },
    
    /// Subscription accepted
    SubscriptionAccepted {
        subscription_id: String,
        subscriber: String,
        target: String,
    },
    
    /// Presence notification
    PresenceNotification {
        subscription_id: String,
        entity: String,
        status: PresenceStatus,
        note: Option<String>,
    },
    
    /// Subscription terminated
    SubscriptionTerminated {
        subscription_id: String,
        reason: String,
    },
}

impl Event for PresenceEvent {
    fn event_type() -> EventType {
        "presence"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

2. **Use Existing Event Adapters**
   - `SessionEventAdapter` in session-core already exists
   - `DialogEventAdapter` in dialog-core handles dialog events
   - Add presence event routing to these adapters

3. **Event Flow Through Layers**
```
SimplePeer.presence() 
    ↓
PresenceCoordinator
    ↓
EventBus.publish(PresencePublished) ← Global Event Bus
    ↓
DialogEventAdapter (subscribes to presence events)
    ↓
Dialog creates PUBLISH transaction
    ↓
Network

SimplePeer.watch()
    ↓
EventBus.subscribe<PresenceNotification>
    ↓
Channel to PresenceWatcher
```

### Layer Responsibilities

1. **transaction-core**
   - Handle PUBLISH, SUBSCRIBE, NOTIFY transactions
   - Emit transaction events via existing adapter

2. **dialog-core**
   - Maintain SUBSCRIBE dialogs (long-lived)
   - Track subscription state
   - Handle dialog refresh/expiry
   - Use `DialogEventAdapter` for presence events

3. **session-core**
   - Presence coordinator (parallel to SessionCoordinator)
   - PIDF XML generation/parsing
   - Subscription management
   - Use `SessionEventAdapter` for presence events
   - Bridge between EventBus and PresenceWatcher channels

4. **api/presence.rs**
   - High-level presence API
   - Builder patterns
   - Subscribe to EventBus for notifications
   - Convert events to channel messages for watchers

### Data Flow

```
SimplePeer.presence()
    ↓
PresenceCoordinator.publish()
    ↓
Transaction (PUBLISH)
    ↓
Network

SimplePeer.watch()
    ↓
PresenceCoordinator.subscribe()
    ↓
Dialog (SUBSCRIBE)
    ↓
Network
    ↓
Dialog (NOTIFY received)
    ↓
PresenceCoordinator.handle_notify()
    ↓
Channel to PresenceWatcher
```

## Implementation Phases (Updated)

### Phase 0: OAuth 2.0 Authentication (NEW - 2 days)
**Priority: CRITICAL - Must be done first**

1. **Add OAuth module to session-core**
   - JWT validation with JWKS support
   - Token introspection client
   - Token caching for performance
   - Scope validation

2. **Add Subscription-State header to sip-core**
```rust
// sip-core/src/headers/subscription_state.rs
pub struct SubscriptionState {
    pub state: SubState,
    pub expires: Option<u32>,
    pub reason: Option<String>,
    pub retry_after: Option<u32>,
}

pub enum SubState {
    Active,
    Pending,
    Terminated,
}
```

### Phase 1: Registrar Integration (2-3 days)
**Was "Core Support" - Now focuses on integration**

1. **Add registrar-core dependency to session-core**
```toml
[dependencies]
rvoip-registrar-core = { path = "../registrar-core" }
jsonwebtoken = "9.0"
reqwest = { version = "0.11", features = ["json"] }
```

2. **Create RegistrarIntegration coordinator**
   - Initialize RegistrarService
   - Set up OAuth2Validator
   - Wire EventBus connections

3. **Modify SessionManager to intercept methods**
   - REGISTER → RegistrarIntegration::handle_register
   - PUBLISH → RegistrarIntegration::handle_publish
   - SUBSCRIBE → RegistrarIntegration::handle_subscribe

### Phase 2: SIP Signaling Updates (2-3 days)
**Was "Session-Core Integration" - Now includes refresh management**

1. **Update message processing pipeline**
   - Extract Bearer tokens from Authorization header
   - Route presence methods to RegistrarIntegration
   - Generate NOTIFY from registrar events

2. **Dialog management for subscriptions**
   - Map subscription_id ↔ dialog_id
   - Handle subscription refresh
   - Clean up expired subscriptions

3. **Implement PresenceRefreshManager**
   - PUBLISH refresh timers (before expiry)
   - SUBSCRIBE dialog refresh (RFC 6665 compliance)
   - P2P heartbeat for direct connections
   - Offline detection and status updates

### Phase 3: API Layer (1-2 days)
**Unchanged - Still needed for user-facing API**

- Implement api/presence.rs
- Add SimplePeer extensions with OAuth
- Create PresenceWatcher and BuddyList
- Add `.auth_bearer()` builder method

### Phase 4: Testing & Polish (1-2 days)
- OAuth validation tests
- Integration tests with mock OAuth server
- P2P and B2BUA presence tests
- Documentation updates

## Technical Considerations

### PIDF XML Format
```xml
<?xml version="1.0" encoding="UTF-8"?>
<presence xmlns="urn:ietf:params:xml:ns:pidf"
          entity="sip:alice@example.com">
  <tuple id="t1">
    <status>
      <basic>open</basic>
    </status>
    <note>Available</note>
  </tuple>
</presence>
```

### Subscription Management
- Subscriptions have expiry times (typically 3600 seconds)
- Must handle subscription refresh
- Clean up expired subscriptions
- Handle authorization (who can see presence)

### Presence Refresh for Connected Peers

**Automatic Presence Updates:**
Connected peers need periodic presence refreshes to maintain accurate state:

```rust
// session-core/src/coordinator/presence_refresh.rs
pub struct PresenceRefreshManager {
    /// Active peer connections that need presence updates
    active_peers: Arc<DashMap<String, PeerPresenceState>>,
    
    /// Refresh intervals for different scenarios
    refresh_config: RefreshConfig,
    
    /// Integration with registrar
    registrar: Arc<RegistrarIntegration>,
}

#[derive(Debug, Clone)]
pub struct RefreshConfig {
    /// How often to send PUBLISH to refresh our own presence (RFC 3903)
    pub publish_interval: Duration,      // Default: 3600s (1 hour)
    
    /// How often to refresh SUBSCRIBE dialogs (RFC 6665)
    pub subscribe_refresh: Duration,      // Default: 3300s (55 min, before expiry)
    
    /// Heartbeat interval for P2P presence (no server)
    pub p2p_heartbeat: Duration,         // Default: 30s
    
    /// Grace period before considering peer offline
    pub offline_threshold: Duration,     // Default: 90s (3 missed heartbeats)
}

pub struct PeerPresenceState {
    pub peer_id: String,
    pub last_seen: Instant,
    pub subscription_id: Option<String>,
    pub dialog_id: Option<DialogId>,
    pub refresh_timer: Option<JoinHandle<()>>,
}

impl PresenceRefreshManager {
    /// Start refresh timers for a peer
    pub async fn start_peer_refresh(&self, peer_id: String, mode: RefreshMode) {
        let state = PeerPresenceState {
            peer_id: peer_id.clone(),
            last_seen: Instant::now(),
            subscription_id: None,
            dialog_id: None,
            refresh_timer: None,
        };
        
        match mode {
            RefreshMode::B2BUA => {
                // Schedule PUBLISH refresh before expiry
                let handle = self.schedule_publish_refresh(peer_id.clone());
                state.refresh_timer = Some(handle);
                
                // Schedule SUBSCRIBE refresh for watched peers
                if let Some(sub_id) = &state.subscription_id {
                    self.schedule_subscribe_refresh(sub_id.clone());
                }
            }
            RefreshMode::P2P => {
                // Start P2P heartbeat timer
                let handle = self.start_p2p_heartbeat(peer_id.clone());
                state.refresh_timer = Some(handle);
            }
        }
        
        self.active_peers.insert(peer_id, state);
    }
    
    /// Schedule PUBLISH refresh (for our own presence)
    fn schedule_publish_refresh(&self, peer_id: String) -> JoinHandle<()> {
        let registrar = self.registrar.clone();
        let interval = self.refresh_config.publish_interval;
        
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            
            loop {
                ticker.tick().await;
                
                // Re-PUBLISH current presence to refresh expiry
                if let Err(e) = registrar.refresh_presence(&peer_id).await {
                    warn!("Failed to refresh presence for {}: {}", peer_id, e);
                    break;
                }
                
                debug!("Refreshed presence for {}", peer_id);
            }
        })
    }
    
    /// Schedule SUBSCRIBE refresh (for watched peers)
    fn schedule_subscribe_refresh(&self, subscription_id: String) -> JoinHandle<()> {
        let registrar = self.registrar.clone();
        let interval = self.refresh_config.subscribe_refresh;
        
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            
            loop {
                ticker.tick().await;
                
                // Send SUBSCRIBE with same dialog to refresh
                if let Err(e) = registrar.refresh_subscription(&subscription_id).await {
                    warn!("Failed to refresh subscription {}: {}", subscription_id, e);
                    break;
                }
                
                debug!("Refreshed subscription {}", subscription_id);
            }
        })
    }
    
    /// P2P heartbeat for direct peer connections
    fn start_p2p_heartbeat(&self, peer_id: String) -> JoinHandle<()> {
        let active_peers = self.active_peers.clone();
        let interval = self.refresh_config.p2p_heartbeat;
        let offline_threshold = self.refresh_config.offline_threshold;
        
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            
            loop {
                ticker.tick().await;
                
                // Send OPTIONS or custom PING to peer
                if let Some(mut peer_state) = active_peers.get_mut(&peer_id) {
                    let elapsed = peer_state.last_seen.elapsed();
                    
                    if elapsed > offline_threshold {
                        // Mark peer as offline
                        warn!("Peer {} appears offline (no response for {:?})", peer_id, elapsed);
                        // Emit PresenceEvent::Updated with Offline status
                        break;
                    }
                    
                    // Send heartbeat (OPTIONS or lightweight message)
                    debug!("Sending P2P presence heartbeat to {}", peer_id);
                    // ... send heartbeat message
                }
            }
        })
    }
    
    /// Handle incoming presence heartbeat (P2P mode)
    pub async fn handle_heartbeat(&self, peer_id: &str) {
        if let Some(mut state) = self.active_peers.get_mut(peer_id) {
            state.last_seen = Instant::now();
            debug!("Received heartbeat from {}", peer_id);
        }
    }
    
    /// Stop refresh timers for a peer
    pub async fn stop_peer_refresh(&self, peer_id: &str) {
        if let Some((_, state)) = self.active_peers.remove(peer_id) {
            if let Some(timer) = state.refresh_timer {
                timer.abort();
            }
            info!("Stopped presence refresh for {}", peer_id);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RefreshMode {
    /// B2BUA mode - use PUBLISH/SUBSCRIBE refresh
    B2BUA,
    /// P2P mode - use direct heartbeats
    P2P,
}
```

**Integration with SimplePeer API:**

```rust
impl SimplePeer {
    /// Enable automatic presence refresh
    pub fn with_presence_refresh(mut self, config: RefreshConfig) -> Self {
        self.presence_refresh = Some(PresenceRefreshManager::new(config));
        self
    }
    
    /// Start presence with automatic refresh
    pub async fn presence(&self, status: PresenceStatus) -> Result<()> {
        // Set initial presence
        self.set_presence(status).await?;
        
        // Start refresh timer if configured
        if let Some(refresh_mgr) = &self.presence_refresh {
            let mode = if self.is_registered() {
                RefreshMode::B2BUA
            } else {
                RefreshMode::P2P
            };
            
            refresh_mgr.start_peer_refresh(self.identity.clone(), mode).await;
        }
        
        Ok(())
    }
}
```

### P2P vs B2BUA Routing

**P2P Mode:**
- Direct PUBLISH/SUBSCRIBE between peers
- Each peer maintains its own subscription state
- Limited scalability for buddy lists

**B2BUA Mode:**
- B2BUA acts as presence server
- Central subscription management
- Efficient for large buddy lists
- Can enforce presence policies

## Security & Privacy

### OAuth 2.0 Bearer Token Authentication (RFC 8898)

**Registration Flow with OAuth:**
1. Client obtains access token from OAuth Authorization Server
2. Client sends REGISTER with `Authorization: Bearer <token>` header
3. Session-core validates token (JWT validation or introspection)
4. If valid, user is registered in registrar-core
5. Token scopes determine permissions (register, call, presence)

**Token Validation Options:**
- **JWT Self-Validation**: Validate signature using OAuth server's public key
- **Token Introspection**: Call OAuth server's `/introspect` endpoint
- **Cached Validation**: Cache validated tokens for performance

**Authorization Headers:**
```
REGISTER sip:example.com SIP/2.0
Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...
```

**Error Response for Invalid Token:**
```
SIP/2.0 401 Unauthorized
WWW-Authenticate: Bearer realm="example.com",
                  error="invalid_token",
                  error_description="The access token expired"
```

### Traditional Security Features
- Presence authorization (accept/reject watchers)
- Filtered presence (show different status to different watchers)
- Encryption of presence data
- Rate limiting for presence updates

## Future Enhancements

1. **Rich Presence**
   - Device capabilities
   - Geographic location
   - Calendar integration
   - Multiple devices per user

2. **Presence Policies**
   - Whitelist/blacklist
   - Time-based rules
   - Group-based visibility

3. **Federation**
   - Cross-domain presence
   - XMPP gateway integration

## Success Criteria

1. ✅ Simple API requiring no SIP knowledge
2. ✅ Transparent P2P and B2BUA operation
3. ✅ Async/await with channels for updates
4. ✅ Consistent with existing SimplePeer design
5. ✅ Extensible for rich presence

## Open Questions

1. Should presence updates automatically trigger on call state changes?
2. How to handle presence aggregation for multiple devices?
3. Should we support presence authorization UI/UX helpers?
4. Integration with external presence sources (calendar, etc.)?

## Updated Status with registrar-core

With registrar-core now implemented, the completion status has improved significantly:

### What's Complete ✅
- **registrar-core**: 100% - Full registration, presence, and subscription management
- **PIDF XML**: Complete in registrar-core
- **Subscription management**: Complete in registrar-core
- **Event system**: Ready in infra-common
- **Basic SIP methods**: Supported in sip-core

### What's Needed ❌
- **OAuth 2.0**: Not implemented (Phase 0)
- **Subscription-State header**: Not in sip-core
- **Session-core integration**: Not connected to registrar-core
- **SIP signaling routing**: Not intercepting REGISTER/PUBLISH/SUBSCRIBE
- **API layer**: SimplePeer extensions not implemented

### Revised Effort Estimate
**Total Estimated Effort**: 7-9 days (reduced from 8-12 days)
- Phase 0 (OAuth): 2 days
- Phase 1 (Integration): 2-3 days
- Phase 2 (Signaling): 2 days
- Phase 3 (API): 1-2 days
- Phase 4 (Testing): 1-2 days

## References

- RFC 3856 - A Presence Event Package for SIP
- RFC 3863 - Presence Information Data Format (PIDF)
- RFC 3903 - SIP Extension for Event State Publication (PUBLISH)
- RFC 6665 - SIP-Specific Event Notification (SUBSCRIBE/NOTIFY)
- RFC 8898 - Third-Party Token-Based Authentication and Authorization for SIP