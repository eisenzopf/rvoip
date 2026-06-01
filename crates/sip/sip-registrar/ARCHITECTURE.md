# Registrar-Core Architecture

## Overview

`registrar-core` implements a SIP Registrar and Presence Server that manages user registration, location services, and presence state. It integrates with `session-core` to provide these services while keeping the concerns cleanly separated.

## Core Design Principles

1. **State Management, Not Signaling**: This crate manages state; session-core handles SIP signaling
2. **Event-Driven Updates**: All state changes emit events for real-time updates
3. **Multi-Device Support**: Users can register from multiple devices simultaneously
4. **Automatic Presence**: Registered users automatically participate in presence
5. **Scalable Data Structures**: Uses DashMap for concurrent access patterns

## Component Architecture

```
┌─────────────────────────────────────────────┐
│             RegistrarService API            │
│         (High-level interface for           │
│          session-core integration)          │
└────────────┬────────────────┬───────────────┘
             │                │
    ┌────────▼──────┐   ┌─────▼──────┐
    │   Registrar   │   │  Presence  │
    │    Module     │   │   Module   │
    └───────────────┘   └────────────┘
             │                │
    ┌────────▼────────────────▼───────┐
    │        Event Bus                 │
    │   (infra-common integration)     │
    └──────────────────────────────────┘
```

## Module Breakdown

### Registrar Module (`src/registrar/`)

Manages user registrations and location services.

```rust
UserRegistry
    ├── register_user()      // Add/update registration
    ├── unregister_user()    // Remove registration
    ├── lookup_user()        // Find user's contacts
    ├── refresh_registration() // Update expiry
    └── expire_registrations() // Clean expired entries

LocationService
    ├── add_binding()        // User -> Contact mapping
    ├── remove_binding()     // Remove mapping
    ├── find_contacts()      // Get all contacts for user
    └── find_user()          // Reverse lookup

RegistrationManager
    ├── start_expiry_timer() // Background expiry task
    ├── validate_contact()   // Validate contact info
    └── generate_expires()   // Calculate expiry time
```

### Presence Module (`src/presence/`)

Manages presence state and subscriptions.

```rust
PresenceServer
    ├── update_presence()    // Update user's presence
    ├── get_presence()       // Query presence
    ├── subscribe()          // Create subscription
    ├── unsubscribe()        // Remove subscription
    └── get_watchers()       // Who's watching a user

PresenceStore
    ├── set_status()         // Store presence state
    ├── get_status()         // Retrieve presence
    ├── get_all_presence()   // Bulk query
    └── clear_presence()     // Remove presence

SubscriptionManager
    ├── add_subscription()   // Create new subscription
    ├── remove_subscription() // Delete subscription
    ├── get_subscribers()    // Who subscribes to user
    ├── get_subscriptions()  // What user subscribes to
    └── notify_subscribers() // Trigger notifications

PidfGenerator
    ├── create_pidf()        // Generate PIDF XML
    ├── parse_pidf()         // Parse PIDF XML
    └── validate_pidf()      // Validate format
```

## Data Models

### Registration Data

```rust
pub struct UserRegistration {
    pub user_id: String,
    pub contacts: Vec<ContactInfo>,
    pub expires: DateTime<Utc>,
    pub presence_enabled: bool,
    pub capabilities: Vec<String>,
}

pub struct ContactInfo {
    pub uri: String,              // sip:alice@192.168.1.100:5060
    pub instance_id: String,      // Unique device identifier
    pub transport: Transport,     // UDP, TCP, TLS, WS, WSS
    pub user_agent: String,       // Client software info
    pub expires: DateTime<Utc>,   // When this contact expires
    pub q_value: f32,            // Priority (0.0-1.0)
    pub received: Option<String>, // Actual source address
    pub path: Vec<String>,        // Route path (RFC 3327)
}
```

### Presence Data

```rust
pub struct PresenceState {
    pub user_id: String,
    pub basic_status: BasicStatus,    // Open/Closed
    pub extended_status: Option<ExtendedStatus>,
    pub note: Option<String>,
    pub activities: Vec<Activity>,
    pub devices: Vec<DevicePresence>,
    pub last_updated: DateTime<Utc>,
    pub expires: Option<DateTime<Utc>>,
}

pub enum BasicStatus {
    Open,    // Available for communication
    Closed,  // Not available
}

pub enum ExtendedStatus {
    Available,
    Away,
    Busy,
    DoNotDisturb,
    OnThePhone,
    InMeeting,
    Custom(String),
}

pub struct DevicePresence {
    pub instance_id: String,
    pub status: BasicStatus,
    pub note: Option<String>,
    pub capabilities: Vec<String>,
}

pub struct Subscription {
    pub id: String,
    pub subscriber: String,       // Who's watching
    pub target: String,          // Who they're watching
    pub state: SubscriptionState,
    pub expires: DateTime<Utc>,
    pub event_id: u32,          // For event sequencing
    pub accept_types: Vec<String>, // PIDF, XPIDF, etc.
}
```

## Integration Patterns

### Session-Core Integration

Session-core communicates with registrar-core in two ways:

#### 1. Signaling-Triggered Operations

When session-core receives SIP messages, it calls registrar-core:

```rust
// In session-core
async fn handle_register(&self, request: SipRequest) -> SipResponse {
    let contact = extract_contact(&request)?;
    self.registrar.register_user(user, contact).await?;
    build_ok_response()
}

async fn handle_publish(&self, request: SipRequest) -> SipResponse {
    let presence = parse_pidf(&request.body)?;
    self.registrar.update_presence(user, presence).await?;
    build_ok_response()
}
```

#### 2. Direct API Queries

For non-SIP operations:

```rust
// Get buddy list (no SIP signaling involved)
let buddies = registrar.get_buddy_list(user).await?;

// Administrative queries
let all_users = registrar.list_registered_users().await?;
```

### Event Bus Integration

All state changes emit events via the global event bus:

```rust
pub enum RegistrarEvent {
    UserRegistered { user: String, contact: ContactInfo },
    UserUnregistered { user: String },
    RegistrationExpired { user: String },
    PresenceUpdated { user: String, state: PresenceState },
    SubscriptionCreated { subscriber: String, target: String },
    SubscriptionTerminated { id: String },
}

// Session-core subscribes to these events
event_bus.subscribe::<PresenceUpdated>(|event| {
    // Generate NOTIFY messages for subscribers
    generate_notify(event.user, event.state).await
});
```

## Automatic Buddy Lists

When a user registers with presence enabled:

1. User is added to the global registry
2. User automatically subscribes to all other registered users
3. All other registered users are notified of new user
4. Presence updates flow automatically

```rust
async fn auto_subscribe_buddies(&self, user: &str) -> Result<()> {
    let all_users = self.registry.list_users().await?;
    
    for other_user in all_users {
        if other_user != user {
            // Create bidirectional subscriptions
            self.presence.subscribe(user, &other_user).await?;
            self.presence.subscribe(&other_user, user).await?;
        }
    }
    
    // Emit event for initial presence fetch
    self.event_bus.publish(BuddyListUpdated { user, buddies }).await?;
    Ok(())
}
```

## Scalability Considerations

### Concurrent Access
- Uses `DashMap` for lock-free concurrent reads/writes
- Sharded by user ID to reduce contention

### Memory Management
- Expired registrations cleaned up by background task
- Presence states have configurable TTL
- Subscription limits per user

### Performance Optimizations
- Bulk operations for buddy list updates
- Cached PIDF documents for unchanged presence
- Event batching for multiple updates

## Security & Privacy

### Registration Security
- Validates contact addresses
- Supports authentication (via session-core)
- Rate limiting on registrations

### Presence Privacy
- Configurable presence policies
- Blacklist/whitelist support (future)
- Selective presence disclosure

## Testing Strategy

### Unit Tests
- Each module tested independently
- Mock event bus for event testing
- Time-based tests for expiry

### Integration Tests
- Full registration/presence flow
- Multi-user scenarios
- Event propagation verification

### Performance Tests
- Concurrent registration benchmarks
- Presence update throughput
- Memory usage under load

## Future Enhancements

1. **Persistent Storage**: Add database backend (PostgreSQL/Redis)
2. **Clustering**: Distributed registrar for HA
3. **Rich Presence**: Calendar integration, location
4. **XCAP Support**: Presence authorization rules
5. **Federation**: Cross-domain presence