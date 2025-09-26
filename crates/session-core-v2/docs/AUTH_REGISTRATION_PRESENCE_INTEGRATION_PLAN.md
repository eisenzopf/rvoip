# Authentication, Registration & Presence Integration Strategy for Session-Core-v2

## Executive Summary

This document outlines a comprehensive strategy for integrating authentication (users-core), registration/location (registrar-core), and presence functionality into session-core-v2. The plan presents multiple implementation options with a clear recommendation to **implement AFTER the B2BUA functionality** to leverage its routing and session management capabilities.

## Strategic Decision: Implementation Timing

### RECOMMENDATION: Implement AFTER B2BUA ✓

**Rationale:**
1. **B2BUA provides the routing infrastructure** needed for registration/presence
2. **Authentication naturally fits into B2BUA routing decisions**
3. **Registration servers ARE specialized B2BUAs** that handle REGISTER messages
4. **Presence servers need B2BUA's multi-session management** for SUBSCRIBE/NOTIFY
5. **Testing is easier** with B2BUA's session bridging already in place

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                       SIP Client                              │
└────────────────────────┬─────────────────────────────────────┘
                         │ SIP REGISTER/SUBSCRIBE/PUBLISH
                         ▼
┌──────────────────────────────────────────────────────────────┐
│                    session-core-v2                            │
│                                                               │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                   SimpleB2bua                         │   │
│  │                                                       │   │
│  │  • Routes REGISTER to RegistrationHandler            │   │
│  │  • Routes SUBSCRIBE/PUBLISH to PresenceHandler       │   │
│  │  • Validates JWT tokens via AuthHandler              │   │
│  └──────────────────┬────────────────────────────────────┘   │
│                     │                                         │
│  ┌─────────────┐    │    ┌──────────────┐ ┌──────────────┐ │
│  │AuthHandler  │◄───┼───►│RegHandler    │ │PresHandler   │ │
│  │             │    │    │              │ │               │ │
│  │•JWT validate│    │    │•REGISTER     │ │•SUBSCRIBE     │ │
│  │•User context│    │    │•Location     │ │•NOTIFY        │ │
│  └──────┬──────┘    │    └──────┬───────┘ └──────┬────────┘ │
│         │           │           │                 │          │
└─────────┼───────────┼───────────┼─────────────────┼──────────┘
          │           │           │                 │
          ▼           │           ▼                 ▼
┌──────────────────┐  │  ┌──────────────┐ ┌──────────────┐
│   users-core     │  │  │registrar-core│ │registrar-core│
│                  │  │  │              │ │              │
│ •JWT issuer      │  │  │•User registry│ │•Presence mgmt│
│ •User store      │  │  │•Location     │ │•PIDF XML     │
│ •/auth/jwks.json │  │  │•Expiry mgmt  │ │•Subscriptions│
└──────────────────┘  │  └──────────────┘ └──────────────┘
                      │
                      ▼
              [B2BUA Routes Calls]
```

## Implementation Options

### Option 1: Lightweight Integration via B2BUA Routing (RECOMMENDED) ✓

Extend the B2BUA implementation to handle special SIP methods:

```rust
// In api/b2bua.rs
impl SimpleB2bua {
    /// Enable registration handling
    pub fn enable_registration(&mut self) -> RegistrationHandler {
        RegistrationHandler::new(self)
    }

    /// Enable presence handling
    pub fn enable_presence(&mut self) -> PresenceHandler {
        PresenceHandler::new(self)
    }

    /// Enable JWT authentication
    pub fn enable_auth(&mut self, jwks_url: String) -> AuthHandler {
        AuthHandler::new(self, jwks_url)
    }
}

// Usage - dead simple!
let mut b2bua = SimpleB2bua::new("sip-server", 5060).await?;

// Enable features with one line each
let auth = b2bua.enable_auth("http://users-core:8080/auth/jwks.json");
let reg = b2bua.enable_registration();
let pres = b2bua.enable_presence();

// Set routing for normal calls
b2bua.on_route(|from, to| async move {
    // Only route if authenticated and registered
    if auth.is_authenticated(from).await && reg.is_registered(to).await {
        reg.lookup_location(to).await
    } else {
        Err("Not authorized")
    }
});

b2bua.start().await
```

### Option 2: Dedicated Adapters in Session-Core-v2

Create separate adapters that work alongside B2BUA:

```rust
// New files in src/adapters/
- auth_adapter.rs       // JWT validation
- registrar_adapter.rs  // Registration handling
- presence_adapter.rs   // Presence management

// Integration in unified.rs
pub struct UnifiedCoordinator {
    // ... existing fields
    auth_adapter: Option<Arc<AuthAdapter>>,
    registrar_adapter: Option<Arc<RegistrarAdapter>>,
    presence_adapter: Option<Arc<PresenceAdapter>>,
}
```

### Option 3: Separate Crate (NOT Recommended)

Create `rvoip-sip-server` that combines all functionality - adds unnecessary complexity.

## Detailed Implementation Plan (Option 1 - Recommended)

### Phase 1: Authentication Handler

```rust
// src/api/b2bua/auth.rs
use users_core::jwt::{UserClaims, JwtValidator};
use std::sync::Arc;
use tokio::sync::RwLock;
use lru::LruCache;

pub struct AuthHandler {
    /// JWT validator (fetches JWKS from users-core)
    validator: Arc<JwtValidator>,

    /// Cache of validated tokens
    token_cache: Arc<RwLock<LruCache<String, UserClaims>>>,

    /// Reference to B2BUA for intercepting messages
    b2bua: Arc<SimpleB2bua>,
}

impl AuthHandler {
    pub async fn new(b2bua: Arc<SimpleB2bua>, jwks_url: String) -> Self {
        // Fetch JWKS from users-core endpoint
        let validator = JwtValidator::from_jwks_url(&jwks_url).await.unwrap();

        Self {
            validator: Arc::new(validator),
            token_cache: Arc::new(RwLock::new(LruCache::new(1000))),
            b2bua,
        }
    }

    /// Validate JWT from Authorization header
    pub async fn validate_sip_auth(&self, auth_header: &str) -> Result<UserClaims> {
        // Parse "Bearer <token>" or "Digest" format
        if let Some(token) = self.extract_bearer_token(auth_header) {
            // Check cache first
            if let Some(claims) = self.token_cache.read().await.get(token) {
                return Ok(claims.clone());
            }

            // Validate with users-core JWKS
            let claims = self.validator.validate(token).await?;

            // Cache for performance
            self.token_cache.write().await.put(token.to_string(), claims.clone());

            Ok(claims)
        } else {
            Err("No valid Bearer token found")
        }
    }

    /// Check if a SIP URI is authenticated
    pub async fn is_authenticated(&self, sip_uri: &str) -> bool {
        // Extract username from SIP URI
        let username = self.extract_username(sip_uri);

        // Check if we have valid token for this user
        // This would be tracked during REGISTER
        self.authenticated_users.read().await.contains(&username)
    }

    /// Handle authentication challenges
    pub async fn challenge_auth(&self, session_id: &SessionId) -> Result<()> {
        // Send 401 Unauthorized with WWW-Authenticate header
        self.b2bua.send_response(session_id, 401, "Unauthorized",
            vec![("WWW-Authenticate", "Bearer realm=\"rvoip\"")]
        ).await
    }
}
```

### Phase 2: Registration Handler

```rust
// src/api/b2bua/registration.rs
use registrar_core::api::{RegistrarService, ContactInfo};
use std::sync::Arc;

pub struct RegistrationHandler {
    /// Registrar service from registrar-core
    registrar: Arc<RegistrarService>,

    /// Reference to B2BUA
    b2bua: Arc<SimpleB2bua>,

    /// Auth handler for validation
    auth: Option<Arc<AuthHandler>>,
}

impl RegistrationHandler {
    pub async fn new(b2bua: Arc<SimpleB2bua>) -> Self {
        // Create registrar service (B2BUA mode for full features)
        let registrar = RegistrarService::new_b2bua().await.unwrap();

        Self {
            registrar: Arc::new(registrar),
            b2bua,
            auth: None,
        }
    }

    /// Link with auth handler for authenticated registration
    pub fn with_auth(&mut self, auth: Arc<AuthHandler>) {
        self.auth = Some(auth);
    }

    /// Handle REGISTER request
    pub async fn handle_register(&self,
        session_id: SessionId,
        from: String,
        contact: String,
        expires: u32,
        auth_header: Option<String>
    ) -> Result<()> {
        // Step 1: Validate authentication if required
        if let Some(auth) = &self.auth {
            if let Some(header) = auth_header {
                match auth.validate_sip_auth(&header).await {
                    Ok(claims) => {
                        // Verify the FROM matches the authenticated user
                        if !from.contains(&claims.username) {
                            return self.send_response(session_id, 403, "Forbidden").await;
                        }
                    }
                    Err(_) => {
                        // Send 401 challenge
                        return auth.challenge_auth(&session_id).await;
                    }
                }
            } else {
                // No auth provided - send challenge
                return auth.challenge_auth(&session_id).await;
            }
        }

        // Step 2: Process registration
        if expires == 0 {
            // Unregister
            self.registrar.unregister_user(&from).await?;
            self.send_response(session_id, 200, "OK").await
        } else {
            // Register/refresh
            let contact_info = ContactInfo {
                uri: contact.clone(),
                expires,
                q_value: 1.0,
                received: None,  // Will be set from network layer
                user_agent: None,
            };

            self.registrar.register_user(&from, contact_info, Some(expires)).await?;

            // Send 200 OK with Contact header showing actual expiry
            self.send_response_with_headers(session_id, 200, "OK",
                vec![("Contact", &format!("{};expires={}", contact, expires))]
            ).await
        }
    }

    /// Lookup where a user can be reached
    pub async fn lookup_location(&self, user: &str) -> Result<String> {
        let contacts = self.registrar.lookup_user(user).await?;

        // Return highest priority contact
        contacts.first()
            .map(|c| c.uri.clone())
            .ok_or("User not registered")
    }

    /// Check if user is registered
    pub async fn is_registered(&self, user: &str) -> bool {
        self.registrar.is_registered(user).await
    }
}
```

### Phase 3: Presence Handler

```rust
// src/api/b2bua/presence.rs
use registrar_core::api::{RegistrarService, PresenceStatus};

pub struct PresenceHandler {
    /// Registrar service (includes presence)
    registrar: Arc<RegistrarService>,

    /// Active subscriptions
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,

    /// B2BUA reference
    b2bua: Arc<SimpleB2bua>,
}

impl PresenceHandler {
    pub async fn new(b2bua: Arc<SimpleB2bua>, registrar: Arc<RegistrarService>) -> Self {
        Self {
            registrar,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            b2bua,
        }
    }

    /// Handle SUBSCRIBE request
    pub async fn handle_subscribe(&self,
        session_id: SessionId,
        from: String,
        to: String,
        event: String,
        expires: u32
    ) -> Result<()> {
        if event != "presence" {
            return self.send_response(session_id, 489, "Bad Event").await;
        }

        // Create subscription
        let subscription_id = self.registrar.subscribe_presence(
            &from, &to, Some(expires)
        ).await?;

        // Store subscription info for NOTIFY
        self.subscriptions.write().await.insert(subscription_id.clone(),
            SubscriptionInfo {
                subscriber: from.clone(),
                target: to.clone(),
                dialog_id: session_id,
                expires_at: Instant::now() + Duration::from_secs(expires as u64),
            }
        );

        // Send 200 OK
        self.send_response(session_id, 200, "OK").await?;

        // Send immediate NOTIFY with current state
        self.send_notify(&subscription_id).await
    }

    /// Handle PUBLISH request
    pub async fn handle_publish(&self,
        session_id: SessionId,
        from: String,
        event: String,
        body: Option<String>
    ) -> Result<()> {
        if event != "presence" {
            return self.send_response(session_id, 489, "Bad Event").await;
        }

        // Parse PIDF XML if provided
        let presence = if let Some(pidf) = body {
            self.registrar.parse_pidf(&pidf).await?
        } else {
            // Default to available
            PresenceState {
                status: PresenceStatus::Available,
                note: None,
            }
        };

        // Update presence
        self.registrar.update_presence(&from, presence.status, presence.note).await?;

        // Send 200 OK
        self.send_response(session_id, 200, "OK").await
    }

    /// Send NOTIFY to subscriber
    async fn send_notify(&self, subscription_id: &str) -> Result<()> {
        let sub = self.subscriptions.read().await.get(subscription_id).cloned()
            .ok_or("Subscription not found")?;

        // Get current presence as PIDF
        let pidf = self.registrar.generate_pidf(&sub.target).await?;

        // Create NOTIFY through B2BUA
        self.b2bua.send_notify(
            sub.dialog_id,
            "presence",
            subscription_id,
            pidf
        ).await
    }
}
```

### Phase 4: Integration into B2BUA

```rust
// Extend src/api/b2bua.rs
impl SimpleB2bua {
    /// Enable full SIP server features
    pub async fn enable_sip_server(&mut self, jwks_url: String) -> Result<()> {
        // Create handlers
        let auth = Arc::new(AuthHandler::new(self.inner.clone(), jwks_url).await?);
        let mut reg = RegistrationHandler::new(self.inner.clone()).await?;
        reg.with_auth(auth.clone());
        let pres = PresenceHandler::new(self.inner.clone(), reg.registrar.clone()).await?;

        // Store handlers
        self.inner.auth_handler = Some(auth.clone());
        self.inner.reg_handler = Some(Arc::new(reg));
        self.inner.pres_handler = Some(Arc::new(pres));

        // Modify routing to check registration
        let reg_clone = self.inner.reg_handler.clone().unwrap();
        self.on_route(move |from, to| {
            let reg = reg_clone.clone();
            async move {
                if reg.is_registered(&to).await {
                    reg.lookup_location(&to).await.unwrap_or(to.to_string())
                } else {
                    to.to_string()  // Try direct routing
                }
            }
        });

        Ok(())
    }
}

// Update B2buaInner::handle_incoming_call to detect special methods
impl B2buaInner {
    async fn handle_incoming_message(&self, msg: IncomingMessage) -> Result<()> {
        match msg.method.as_str() {
            "REGISTER" => {
                if let Some(reg) = &self.reg_handler {
                    reg.handle_register(msg.session_id, msg.from, msg.contact,
                        msg.expires, msg.auth_header).await
                } else {
                    self.send_response(msg.session_id, 501, "Not Implemented").await
                }
            }
            "SUBSCRIBE" => {
                if let Some(pres) = &self.pres_handler {
                    pres.handle_subscribe(msg.session_id, msg.from, msg.to,
                        msg.event, msg.expires).await
                } else {
                    self.send_response(msg.session_id, 501, "Not Implemented").await
                }
            }
            "PUBLISH" => {
                if let Some(pres) = &self.pres_handler {
                    pres.handle_publish(msg.session_id, msg.from,
                        msg.event, msg.body).await
                } else {
                    self.send_response(msg.session_id, 501, "Not Implemented").await
                }
            }
            "INVITE" => {
                // Normal call handling through B2BUA
                self.handle_incoming_call(msg.into()).await
            }
            _ => {
                self.send_response(msg.session_id, 501, "Not Implemented").await
            }
        }
    }
}
```

## Usage Examples

### Example 1: Simple SIP Server with Auth & Registration

```rust
use rvoip_session_core_v2::api::SimpleB2bua;

#[tokio::main]
async fn main() -> Result<()> {
    let mut b2bua = SimpleB2bua::new("sip-server", 5060).await?;

    // Enable SIP server features with one line!
    b2bua.enable_sip_server("http://localhost:8080/auth/jwks.json").await?;

    b2bua.start().await
}
```

### Example 2: PBX with Authentication

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let mut b2bua = SimpleB2bua::new("pbx", 5060).await?;

    // Enable features
    let auth = b2bua.enable_auth("http://users-core:8080/auth/jwks.json");
    let reg = b2bua.enable_registration();

    // Custom routing based on authentication
    b2bua.on_route(move |from, to| {
        let auth = auth.clone();
        let reg = reg.clone();
        async move {
            // Verify caller is authenticated
            if !auth.is_authenticated(&from).await {
                return Err("Caller not authenticated");
            }

            // Route to registered user or voicemail
            if reg.is_registered(&to).await {
                reg.lookup_location(&to).await
            } else {
                Ok("sip:voicemail@system.local".to_string())
            }
        }
    });

    b2bua.start().await
}
```

### Example 3: Presence-Enabled Server

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let mut b2bua = SimpleB2bua::new("presence-server", 5060).await?;

    // Enable all features
    b2bua.enable_sip_server("http://users-core:8080/auth/jwks.json").await?;

    // Add presence-aware routing
    let pres = b2bua.presence_handler();
    b2bua.on_route(move |from, to| {
        let pres = pres.clone();
        async move {
            // Check if target is available
            let presence = pres.get_presence(&to).await?;
            match presence.status {
                PresenceStatus::Available => {
                    // Route normally
                    pres.registrar.lookup_location(&to).await
                }
                PresenceStatus::Busy | PresenceStatus::DoNotDisturb => {
                    // Route to voicemail
                    Ok("sip:voicemail@system.local".to_string())
                }
                _ => {
                    // Try anyway
                    pres.registrar.lookup_location(&to).await
                }
            }
        }
    });

    b2bua.start().await
}
```

## Implementation Timeline

### Prerequisites (Week 0)
- ✅ B2BUA implementation complete
- ✅ users-core with JWT/JWKS support
- ✅ registrar-core with full registration/presence

### Phase 1: Authentication (Week 1)
- [ ] Day 1-2: Create AuthHandler with JWT validation
- [ ] Day 3: Integrate with B2BUA message flow
- [ ] Day 4: Add authentication challenge/response
- [ ] Day 5: Test with users-core JWKS endpoint

### Phase 2: Registration (Week 2)
- [ ] Day 1-2: Create RegistrationHandler
- [ ] Day 3: Integrate with registrar-core
- [ ] Day 4: Add location lookup for routing
- [ ] Day 5: Test registration expiry and refresh

### Phase 3: Presence (Week 3)
- [ ] Day 1-2: Create PresenceHandler
- [ ] Day 3: SUBSCRIBE/NOTIFY implementation
- [ ] Day 4: PUBLISH and PIDF support
- [ ] Day 5: Test presence updates and notifications

### Phase 4: Integration Testing (Week 4)
- [ ] Day 1-2: End-to-end authentication flow
- [ ] Day 3: Multi-user registration scenarios
- [ ] Day 4: Presence subscription testing
- [ ] Day 5: Documentation and examples

## Comparison of Implementation Options

| Aspect | Option 1: B2BUA Extension | Option 2: Separate Adapters | Option 3: New Crate |
|--------|---------------------------|----------------------------|-------------------|
| **Simplicity** | ⭐⭐⭐⭐⭐ One-line enable | ⭐⭐⭐ More configuration | ⭐⭐ Complex setup |
| **Integration** | ⭐⭐⭐⭐⭐ Natural fit | ⭐⭐⭐⭐ Good integration | ⭐⭐ Loose coupling |
| **Performance** | ⭐⭐⭐⭐⭐ Shared state | ⭐⭐⭐⭐ Good | ⭐⭐⭐ Extra overhead |
| **Flexibility** | ⭐⭐⭐⭐ Configurable | ⭐⭐⭐⭐⭐ Very flexible | ⭐⭐⭐ Limited |
| **Testing** | ⭐⭐⭐⭐⭐ Easy with B2BUA | ⭐⭐⭐ More complex | ⭐⭐ Separate tests |
| **Maintenance** | ⭐⭐⭐⭐⭐ Single module | ⭐⭐⭐ Multiple files | ⭐⭐ Separate crate |

## Success Criteria

1. **Authentication**: JWT validation in < 5ms with caching
2. **Registration**: Handle 10,000+ concurrent registrations
3. **Presence**: Real-time updates with < 100ms latency
4. **Integration**: Works seamlessly with SimplePeer clients
5. **Simplicity**: Full SIP server in < 5 lines of code

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| JWT validation performance | High | Implement LRU cache for validated tokens |
| Registration state sync | Medium | Use registrar-core's built-in expiry management |
| Presence notification storms | Medium | Batch notifications, implement rate limiting |
| Complex authentication flows | Low | Start with Bearer tokens, add Digest later |

## Migration Path

For existing B2BUA implementations:

```rust
// Before: Basic B2BUA
let mut b2bua = SimpleB2bua::new("gateway", 5060).await?;
b2bua.on_route(|_, to| async move { to.to_string() });

// After: Full SIP server (just one line!)
let mut b2bua = SimpleB2bua::new("gateway", 5060).await?;
b2bua.enable_sip_server("http://users:8080/auth/jwks.json").await?;
b2bua.on_route(|_, to| async move { to.to_string() });
```

## Conclusion

The recommended approach is to **implement authentication, registration, and presence AFTER the B2BUA functionality** by extending SimpleB2bua with optional handlers. This approach:

1. **Maintains simplicity** - One line to enable each feature
2. **Leverages B2BUA routing** - Natural integration point
3. **Reuses infrastructure** - No duplicate session management
4. **Progressive enhancement** - Add features as needed
5. **Production ready** - Can handle real-world SIP server requirements

The key insight is that a **SIP registrar/presence server IS a specialized B2BUA** that routes REGISTER/SUBSCRIBE/PUBLISH messages to internal handlers instead of forwarding them. By building on top of the B2BUA implementation, we get a complete SIP server with minimal additional code.