# Presence API Plan for Session-Core

## Overview

This document outlines the plan to add SIP presence functionality to session-core, providing a simple, developer-friendly API that works transparently in both P2P and B2BUA scenarios.

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

#### With B2BUA Server (Transparent)
```rust
// Register with B2BUA (acts as presence server)
alice.register("sip:pbx.company.com").await?;
bob.register("sip:pbx.company.com").await?;

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

## Architecture Integration

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

## Implementation Phases

### Phase 1: Core Support (transaction/dialog layers)
- Add PUBLISH method support to transaction-core
- Add SUBSCRIBE/NOTIFY to dialog-core
- Implement subscription dialog state machine

### Phase 2: Session-Core Integration
- Create PresenceCoordinator
- Implement PIDF XML handling
- Add presence event routing

### Phase 3: API Layer
- Implement api/presence.rs
- Add SimplePeer extensions
- Create PresenceWatcher and BuddyList

### Phase 4: Testing & Polish
- Unit tests for each layer
- Integration tests for P2P and B2BUA
- Documentation and examples

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

## References

- RFC 3856 - A Presence Event Package for SIP
- RFC 3863 - Presence Information Data Format (PIDF)
- RFC 3903 - SIP Extension for Event State Publication (PUBLISH)
- RFC 6665 - SIP-Specific Event Notification (SUBSCRIBE/NOTIFY)