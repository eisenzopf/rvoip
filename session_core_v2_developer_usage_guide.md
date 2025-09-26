# Developer Usage Guide: Building SIP Servers with session-core-v2

## Overview

This guide explains how developers will use session-core-v2 with registration and authentication to build real SIP servers. It shows the complete flow from client authentication to SIP registration and session management.

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│                          Your SIP Server Application                      │
│                                                                          │
│  1. Initialize Services:                                                 │
│     - users-core (authentication service)                                │
│     - registrar-core (registration/presence service)                     │
│     - session-core-v2 (session management)                               │
│     - dialog-core (SIP protocol)                                         │
│     - Transport (UDP/TCP/TLS)                                           │
│                                                                          │
│  2. Handle Connections:                                                  │
│     - Accept SIP messages on port 5060                                   │
│     - Route through dialog-core → session-core-v2                        │
│     - Manage multiple user sessions                                      │
└──────────────────────────────────────────────────────────────────────────┘
```

## Complete Authentication & Registration Flow

### Step 1: User Gets Authentication Token (Out-of-band)

Before using SIP, users must authenticate with the users-core service to get a JWT token:

```bash
# User authenticates via REST API
POST http://auth.example.com:8081/auth/login
{
  "username": "alice",
  "password": "SecurePass123!"
}

# Response includes JWT token
{
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "refresh_token": "eyJhbGciOiJSUzI1NiIs...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

### Step 2: User Includes Token in SIP REGISTER

The SIP client includes the JWT token in the Authorization header:

```
REGISTER sip:example.com SIP/2.0
Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds
From: <sip:alice@example.com>;tag=1928301774
To: <sip:alice@example.com>
Call-ID: a84b4c76e66710@pc33.example.com
CSeq: 1 REGISTER
Contact: <sip:alice@192.168.1.100:5060>
Authorization: Bearer eyJhbGciOiJSUzI1NiIs...
Expires: 3600
Content-Length: 0
```

### Step 3: Server Processes REGISTER

The server validates the token and registers the user:

```
200 OK
Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds
From: <sip:alice@example.com>;tag=1928301774
To: <sip:alice@example.com>;tag=37GkEhwl6
Call-ID: a84b4c76e66710@pc33.example.com
CSeq: 1 REGISTER
Contact: <sip:alice@192.168.1.100:5060>;expires=3600
Content-Length: 0
```

## Building a Complete SIP Server

Here's how developers will build a SIP server with registration support:

### 1. Server Initialization Code

```rust
use rvoip_session_core_v2::{UnifiedCoordinator, SessionBuilder};
use rvoip_dialog_core::DialogClient;
use rvoip_registrar_core::RegistrarService;
use rvoip_users_core::{init as init_users, UsersConfig};
use rvoip_sip_transport::{UdpTransport, TransportLayer};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize users-core authentication service
    let users_config = UsersConfig {
        database_url: "sqlite://users.db".to_string(),
        api_bind_address: "127.0.0.1:8081".parse()?,
        ..Default::default()
    };
    let auth_service = init_users(users_config).await?;
    
    // 2. Start users-core REST API in background (for user login)
    tokio::spawn(async move {
        let app = rvoip_users_core::api::create_router(auth_service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:8081").await?;
        axum::serve(listener, app).await
    });
    
    // 3. Initialize registrar-core for registration storage
    let registrar = RegistrarService::new_b2bua().await?;
    
    // 4. Initialize dialog-core for SIP protocol handling
    let transport = Arc::new(UdpTransport::bind("0.0.0.0:5060").await?);
    let dialog_client = DialogClient::new(transport.clone()).await?;
    
    // 5. Initialize session-core-v2 with all services
    let coordinator = SessionBuilder::new()
        .with_dialog_client(dialog_client.clone())
        .with_registrar_service(registrar.clone())
        .with_users_core_url("http://127.0.0.1:8081")  // For JWT validation
        .with_state_table_path("state_tables/sip_server.yaml")
        .build()
        .await?;
    
    // 6. Start the SIP server
    println!("SIP Server listening on 0.0.0.0:5060");
    println!("Auth API available at http://127.0.0.1:8081");
    
    // Keep running
    tokio::signal::ctrl_c().await?;
    coordinator.shutdown().await?;
    
    Ok(())
}
```

### 2. State Table Configuration

**File: `state_tables/sip_server.yaml`**

```yaml
# SIP Server State Table - Handles multiple user sessions
version: "1.0"

metadata:
  description: "SIP server with registration and call handling"

states:
  # Session states (per user connection)
  - name: "Connected"
    description: "Transport connection established"
  - name: "Authenticating"
    description: "Processing authentication"
  - name: "Registered"
    description: "User successfully registered"
  - name: "InCall"
    description: "User in active call"

transitions:
  # Handle incoming REGISTER
  - role: "Server"
    state: "Connected"
    event:
      type: "DialogREGISTER"
    guards:
      - type: "HasValidJWT"
    actions:
      - type: "RegisterUser"
      - type: "SendSIPResponse"
        args:
          code: 200
          reason: "OK"
    next_state: "Registered"
    
  # Handle REGISTER without auth
  - role: "Server"
    state: "Connected"
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
            WWW-Authenticate: "Bearer realm=\"example.com\""
    next_state: "Connected"
```

### 3. Multi-User Session Management

The server manages multiple user sessions simultaneously:

```rust
// Inside UnifiedCoordinator (automatic session management)

// When REGISTER arrives:
// 1. Check if session exists for this user
let session_id = coordinator.find_or_create_session(
    &from_uri,     // e.g., "sip:alice@example.com"
    source_addr,   // Client's IP address
    Role::Server   // We're the server side
).await?;

// 2. Process through state machine
coordinator.process_event(
    &session_id,
    EventType::DialogREGISTER {
        from: from_uri,
        contact: contact_uri,
        expires: 3600,
        auth_header: Some(auth_header),
    }
).await?;

// 3. Session is now tracked with registration state
// Future requests from same user will use same session
```

### 4. Complete User Flow Example

```rust
// Example: Alice registers and makes a call

// 1. Alice logs in via REST API (gets JWT token)
let token = alice_client.login("alice", "password").await?;

// 2. Alice's SIP client sends REGISTER with token
alice_sip_client.register("sip:example.com", token).await?;
// → Server validates JWT with users-core
// → Server registers Alice in registrar-core
// → Server maintains Alice's session in session-core-v2

// 3. Bob also registers
let bob_token = bob_client.login("bob", "password").await?;
bob_sip_client.register("sip:example.com", bob_token).await?;

// 4. Alice calls Bob
alice_sip_client.call("sip:bob@example.com").await?;
// → Server looks up Bob's location in registrar-core
// → Server routes call through session-core-v2
// → Both Alice and Bob have separate sessions

// 5. Server maintains both sessions
let sessions = coordinator.list_sessions().await?;
// Returns: ["alice@example.com", "bob@example.com"]

// 6. Sessions include registration state
let alice_info = coordinator.get_session_info(&alice_session_id).await?;
println!("Alice registered: {}", alice_info.is_registered);
println!("Alice expires: {:?}", alice_info.registration_expires);
```

## Key Integration Points

### 1. Authentication Flow
```
┌─────────┐     REST      ┌────────────┐
│ Client  │──────────────▶│users-core  │
│         │◀──────────────│   (8081)   │
└─────────┘   JWT Token   └────────────┘
     │                           ▲
     │ SIP REGISTER             │
     │ (with JWT)               │ Validate
     ▼                          │ JWT
┌─────────┐               ┌────────────┐
│  Your   │──────────────▶│  session-  │
│  Server │               │  core-v2   │
│ (5060)  │◀──────────────│            │
└─────────┘   200 OK      └────────────┘
                                │
                                ▼ Store
                          ┌────────────┐
                          │ registrar- │
                          │    core    │
                          └────────────┘
```

### 2. Session Lifecycle

```rust
// Session creation (automatic on first message)
REGISTER → Create Session → Validate Auth → Store Registration

// Session includes:
struct UserSession {
    session_id: SessionId,
    user_uri: String,              // "sip:alice@example.com"
    transport_addr: SocketAddr,    // Client's IP:port
    is_registered: bool,
    registration_expires: DateTime<Utc>,
    auth_claims: UserClaims,       // From JWT
    active_calls: Vec<CallId>,     // Current calls
    presence_status: PresenceStatus,
}

// Session cleanup (automatic on expiry)
Registration Expires → Clear Session → Notify Watchers
```

### 3. Handling Multiple Users

The server automatically manages sessions per user:

```rust
// Built into UnifiedCoordinator
impl UnifiedCoordinator {
    // Sessions are indexed by:
    // 1. User URI (for user lookup)
    // 2. Call-ID (for dialog correlation)
    // 3. Transport address (for connection tracking)
    
    // When SIP message arrives:
    async fn handle_message(&self, msg: Message, source: SocketAddr) {
        // 1. Find or create session
        let session_id = self.find_session(&msg).await
            .unwrap_or_else(|| self.create_session(&msg, source).await);
        
        // 2. Process through state machine
        self.process_message(session_id, msg).await;
    }
}
```

## Example: Building a SIP Registrar Server

Here's a complete example of a SIP registrar server:

```rust
// examples/sip_registrar_server.rs

use rvoip_session_core_v2::{UnifiedCoordinator, SessionBuilder};
use rvoip_dialog_core::DialogClient;
use rvoip_registrar_core::RegistrarService;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize all services
    let registrar = RegistrarService::new_b2bua().await?;
    let dialog = DialogClient::new_with_transport("udp:0.0.0.0:5060").await?;
    
    let coordinator = SessionBuilder::new()
        .with_dialog_client(dialog)
        .with_registrar_service(registrar)
        .with_users_core_url("http://localhost:8081")
        .enable_registration()
        .enable_presence()
        .build()
        .await?;
    
    println!("SIP Registrar Server ready on :5060");
    
    // The coordinator handles everything:
    // - Accepts SIP connections
    // - Validates authentication
    // - Manages registrations
    // - Handles presence
    // - Routes calls
    
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

## Developer Benefits

1. **Automatic Session Management**: No manual session tracking needed
2. **Integrated Authentication**: JWT validation built into the flow
3. **Multi-User by Default**: Handles thousands of concurrent users
4. **State Machine Driven**: Predictable behavior for all scenarios
5. **Event-Driven**: Easy to extend and monitor

## Common Patterns

### Pattern 1: SIP Proxy Server
```rust
let coordinator = SessionBuilder::new()
    .with_role(Role::Proxy)
    .enable_registration()
    .enable_call_routing()
    .build().await?;
```

### Pattern 2: SIP Registrar Only
```rust
let coordinator = SessionBuilder::new()
    .with_role(Role::Registrar)
    .enable_registration()
    .disable_call_handling()
    .build().await?;
```

### Pattern 3: B2BUA with Full Features
```rust
let coordinator = SessionBuilder::new()
    .with_role(Role::B2BUA)
    .enable_registration()
    .enable_presence()
    .enable_conferencing()
    .enable_call_recording()
    .build().await?;
```

## Summary

Developers building SIP servers with session-core-v2 will:

1. **Initialize Services**: Set up users-core, registrar-core, and session-core-v2
2. **Configure State Tables**: Define server behavior for different scenarios
3. **Let the Framework Handle**: Session management, authentication, and routing
4. **Focus on Business Logic**: Add custom features without worrying about SIP details

The key insight is that session-core-v2 acts as the **orchestrator** that connects all the services together, managing sessions automatically based on the configured state machine.
