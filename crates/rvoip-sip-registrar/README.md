# Registrar Core

A high-performance SIP Registrar and Presence Server for the rvoip ecosystem.

## Overview

`registrar-core` provides user registration and presence management functionality for SIP-based communication systems. It acts as a centralized service that:

- Manages user registrations (SIP REGISTER)
- Tracks user locations (multiple devices per user)
- Handles presence state (available, busy, away, etc.)
- Manages presence subscriptions (who's watching whom)
- Provides automatic buddy lists for registered users

## Architecture

### Separation of Concerns

This crate is designed to work alongside `session-core`:

- **session-core**: Handles SIP signaling and call sessions
- **registrar-core**: Manages user registration and presence state
- **dialog-core**: Handles SIP protocol details

### Integration Model

```
SIP Client → session-core → registrar-core
                 ↓              ↓
           (signaling)    (state mgmt)
                 ↓              ↓
           SIP Response   Event Updates
```

## Features

- **User Registration**: Track registered users and their contact locations
- **Multi-Device Support**: Users can register from multiple devices
- **Presence Management**: Store and distribute presence information
- **Automatic Buddy Lists**: Registered users automatically see each other
- **Event-Driven**: Publishes events via infra-common event bus
- **Scalable**: Uses efficient data structures (DashMap) for concurrent access
- **Standards Compliant**: Follows RFC 3903 (PUBLISH), RFC 6665 (SUBSCRIBE/NOTIFY)

## Usage

### P2P Mode (Optional Presence)

```rust
// P2P peers can optionally use presence
let registrar = RegistrarService::new_p2p().await?;
registrar.update_presence("alice", PresenceStatus::Available).await?;
```

### B2BUA Mode (Full Featured)

```rust
// B2BUA with automatic presence for all registered users
let registrar = RegistrarService::new_b2bua().await?;

// Users register
registrar.register_user("alice", contact_info).await?;

// Automatic buddy list
let buddies = registrar.get_buddy_list("alice").await?;

// Update presence
registrar.update_presence("alice", PresenceStatus::Busy).await?;
```

## Components

### Registrar Module
- `UserRegistry`: Manages user registrations and locations
- `LocationService`: Maps users to their contact addresses
- `RegistrationManager`: Handles registration expiry and refresh

### Presence Module
- `PresenceServer`: Core presence state management
- `SubscriptionManager`: Manages who's watching whom
- `PresenceStore`: Stores current presence state
- `PidfGenerator`: Creates/parses PIDF XML documents

### API Module
- `RegistrarService`: High-level API for session-core integration
- Event definitions for global event bus integration

## Design Principles

1. **Simplicity First**: P2P works without registration/presence
2. **Automatic Features**: Registered users get presence automatically
3. **Event-Driven**: All state changes publish events
4. **Scalable**: Designed for thousands of users
5. **Testable**: Clear interfaces and mockable components

## Integration with session-core

session-core integrates with registrar-core in two ways:

1. **Signaling Integration**: All SIP messages flow through session-core
2. **Direct API**: Non-SIP operations (get buddy list, query presence)

See [ARCHITECTURE.md](./ARCHITECTURE.md) for detailed integration patterns.