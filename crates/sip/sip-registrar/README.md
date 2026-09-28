# rvoip-sip-registrar

A high-performance SIP Registrar and Presence Server for the rvoip ecosystem.

## Overview

`rvoip-sip-registrar` provides user registration and presence management functionality for SIP-based communication systems. It acts as a centralized service that:

- Manages user registrations (SIP REGISTER)
- Tracks user locations (multiple devices per user)
- Handles presence state (available, busy, away, etc.)
- Manages presence subscriptions (who's watching whom)
- Provides automatic buddy lists for registered users

## Architecture

### Separation of Concerns

This crate is designed to work alongside the rest of the SIP stack:

- **rvoip-sip** (umbrella): Handles SIP signaling and call sessions
- **rvoip-sip-registrar**: Manages user registration and presence state
- **rvoip-sip-dialog**: Handles SIP protocol details (transactions and dialogs)
- **rvoip-auth-core**: Digest authentication primitives
- **rvoip-infra-common**: Global event bus

### Integration Model

```
SIP Client → rvoip-sip → rvoip-sip-registrar
                 ↓              ↓
           (signaling)    (state mgmt)
                 ↓              ↓
           SIP Response   Event Updates
```

## Install

```toml
[dependencies]
rvoip-sip-registrar = "0.3.10"
```

A runnable server is in [`examples/registrar_server.rs`](examples/registrar_server.rs):

```bash
cargo run -p rvoip-sip-registrar --example registrar_server
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
registrar.update_presence("alice", PresenceStatus::Available, None).await?;
```

### B2BUA Mode (Full Featured)

```rust
// B2BUA with automatic presence for all registered users
let registrar = RegistrarService::new_b2bua().await?;

// Users register (expires: None uses RegistrarConfig::default_expires)
registrar.register_user("alice", contact_info, None).await?;

// Automatic buddy list
let buddies = registrar.get_buddy_list("alice").await?;

// Update presence (optional free-text note)
registrar.update_presence("alice", PresenceStatus::Busy, Some("On a call".into())).await?;
```

### Authenticated Registrar and Registered Flows

```rust
use rvoip_sip_registrar::api::ServiceMode;
use rvoip_sip_registrar::{AddressOfRecord, RegistrarConfig, RegistrarService};

// Digest-authenticated service (rvoip-auth-core DigestAuthenticator + UserStore)
let registrar = RegistrarService::with_auth(
    ServiceMode::B2BUA,
    RegistrarConfig::default(),
    "example.com",
).await?;

// Verify a REGISTER's Authorization header against the Request-URI and AOR
let (ok, challenge) = registrar
    .authenticate_register_request("alice", authorization, "REGISTER", request_uri, aor_uri)
    .await?;

// AOR-keyed bindings
let aor = AddressOfRecord::parse("sip:alice@example.com").expect("valid AOR");
registrar.register_aor(&aor, contact_info, Some(3600)).await?;
let contacts = registrar.lookup_aor(&aor).await?;
let live = registrar.lookup_live_contacts(&aor, "INVITE").await?;
registrar.add_domain_alias("sip.example.com", "example.com");
```

What has landed on `RegistrarService`:

- **Digest auth**: `with_auth(mode, config, realm)`, `authenticate_register_request(..)`
  (and the older single-URI `authenticate_register(..)`), plus `user_store()` /
  `authenticator()` accessors.
- **Identity hooks**: `IdentityProvider` and `CredentialProvider` traits
  (`src/identity.rs`), installed with `with_identity_provider(..)`,
  `set_identity_provider(..)`, and `set_credential_provider(..)`.
- **RFC 5626 registered flows**: `new_registered_flow_token()`,
  `bind_registered_flow(..)`, `commit_registered_flow(..)`,
  `remove_registered_flow(..)`, `resolve_registered_flow(..)`,
  `set_registered_flow_reachability(..)`, and
  `mark_process_local_flow_unreachable(..)`.
- **AOR-keyed API**: `register_aor`, `lookup_aor`, `lookup_live_contacts`,
  `unregister_aor`, `refresh_registration_aor`, and `add_domain_alias`.

The credential store no longer keeps recoverable plaintext passwords; see
[MIGRATION_0_3.md](./MIGRATION_0_3.md) for the migration notes.

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
- `RegistrarService`: High-level API for `rvoip-sip` integration
- Event definitions for global event bus integration

## Design Principles

1. **Simplicity First**: P2P works without registration/presence
2. **Automatic Features**: Registered users get presence automatically
3. **Event-Driven**: All state changes publish events
4. **Scalable**: Designed for thousands of users
5. **Testable**: Clear interfaces and mockable components

## Integration with rvoip-sip

`rvoip-sip` integrates with `rvoip-sip-registrar` in two ways:

1. **Signaling Integration**: All SIP messages flow through `rvoip-sip` and `rvoip-sip-dialog`
2. **Direct API**: Non-SIP operations (get buddy list, query presence)

See [ARCHITECTURE.md](./ARCHITECTURE.md) for detailed integration patterns.