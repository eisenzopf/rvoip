# SIP Registration Demo

This example demonstrates SIP REGISTER handling with digest authentication using session-core-v3.

## Architecture

The demo is split into two processes to demonstrate real client-server communication:

1. **UAS (Server)** - Receives REGISTER requests and orchestrates authentication
2. **UAC (Client)** - Sends REGISTER requests with digest authentication

### Event Flow

```text
┌─────────┐                                    ┌─────────┐
│   UAC   │                                    │   UAS   │
│ (5061)  │                                    │ (5060)  │
└────┬────┘                                    └────┬────┘
     │                                              │
     │  1. REGISTER (no auth)                      │
     │────────────────────────────────────────────>│
     │                                              │
     │                                        ┌─────▼──────┐
     │                                        │dialog-core │
     │                                        └─────┬──────┘
     │                                              │ IncomingRegister
     │                                              │ event
     │                                        ┌─────▼──────────┐
     │                                        │session-core-v3 │
     │                                        │ (orchestrator) │
     │                                        └─────┬──────────┘
     │                                              │
     │                                        ┌─────▼──────────┐
     │                                        │registrar-core  │
     │                                        │  authenticate  │
     │                                        └─────┬──────────┘
     │                                              │
     │                                        ┌─────▼──────────┐
     │                                        │session-core-v3 │
     │                                        └─────┬──────────┘
     │                                              │ SendRegisterResponse
     │                                              │ event (401)
     │                                        ┌─────▼──────┐
     │                                        │dialog-core │
     │                                        └─────┬──────┘
     │  2. 401 Unauthorized                        │
     │<────────────────────────────────────────────┘
     │    WWW-Authenticate: Digest realm="test.local"
     │
┌────▼────┐
│session- │ Compute digest
│core-v3  │ MD5(username:realm:password)
└────┬────┘
     │
     │  3. REGISTER (with Authorization)
     │────────────────────────────────────────────>│
     │    Authorization: Digest username="alice"... │
     │                                              │
     │                                        [Same orchestration
     │                                         flow as above]
     │                                              │
     │  4. 200 OK                                   │
     │<────────────────────────────────────────────┘
     │    Contact: <sip:alice@127.0.0.1:5061>
     │    Expires: 3600
     │
     ✅ Registration complete!
```

## Components Demonstrated

### Server (UAS) - `uas.rs`

**Purpose:** Handles incoming REGISTER requests via session-core-v3 orchestration

**Key Components:**
- `UnifiedCoordinator` - Main session-core-v3 API
- `start_registration_server()` - Starts the registration adapter
- `RegistrationAdapter` - Subscribes to IncomingRegister events
- `RegistrarService` - Validates credentials and stores registrations
- `DialogEventHub` - Handles SendRegisterResponse events

**Event Flow:**
1. dialog-core receives REGISTER → publishes `IncomingRegister` event
2. RegistrationAdapter receives event → calls registrar-core
3. registrar-core validates credentials → returns (should_register, challenge)
4. RegistrationAdapter publishes `SendRegisterResponse` event
5. DialogEventHub receives event → sends SIP response

### Client (UAC) - `uac.rs`

**Purpose:** Sends REGISTER requests with automatic digest authentication

**Key Components:**
- `UnifiedCoordinator` - Main session-core-v3 API
- `register()` - High-level registration API
- State machine - Handles registration flow transitions
- `DialogAdapter` - Sends REGISTER via dialog-core
- auth-core - Computes digest authentication

**State Flow:**
1. `Idle` + `StartRegistration` → `Registering`
2. Receives 401 → `Authenticating`
3. Computes digest → Sends authenticated REGISTER
4. Receives 200 → `Registered`

## Usage

### Terminal 1: Start the Server

```bash
cd crates/session-core-v3
cargo run --example register_uas
```

Expected output:
```
🚀 Starting Registration Server (UAS)
=====================================

📡 Creating server coordinator...
✅ Server coordinator created on 127.0.0.1:5060

🔐 Starting registration server...
✅ Registrar server started

👥 Registered users:
  - alice / password123
  - bob / secret456

📞 Server ready to accept REGISTER requests on 127.0.0.1:5060

💡 Run the client: cargo run --example register_uac

Press Ctrl+C to stop the server...
```

### Terminal 2: Run the Client

```bash
cd crates/session-core-v3
cargo run --example register_uac
```

Expected output:
```
🚀 Starting Registration Client (UAC)
======================================

📱 Creating client coordinator...
✅ Client coordinator created on 127.0.0.1:5061

🔐 Starting registration...
Registration parameters:
  Registrar: sip:127.0.0.1:5060
  From: sip:alice@127.0.0.1
  Contact: sip:alice@127.0.0.1:5061
  Username: alice
  Expires: 3600 seconds

✅ Registration initiated!
   Session ID: session-xxx

⏳ Waiting for registration to complete...

✅ Registration successful!
   User alice is now registered with the server

🔄 Keeping registration alive...

🔄 Refreshing registration (attempt 1/3)...
✅ Registration refreshed successfully

📤 Unregistering...
✅ Unregistered successfully

✅ Demo completed successfully!
```

## What This Demonstrates

### Event-Driven Orchestration
- ✅ Cross-crate event communication via `GlobalEventCoordinator`
- ✅ Trait-based event downcasting (no unsafe `Arc::downcast`)
- ✅ Loose coupling between layers

### Server-Side REGISTER Handling
- ✅ dialog-core publishes `IncomingRegister` events
- ✅ session-core-v3 orchestrates via `RegistrationAdapter`
- ✅ registrar-core validates credentials
- ✅ session-core-v3 publishes `SendRegisterResponse` events
- ✅ dialog-core sends SIP responses

### Client-Side REGISTER Flow
- ✅ State machine-driven registration
- ✅ Automatic 401 challenge handling
- ✅ Digest authentication computation
- ✅ Registration refresh
- ✅ Unregistration

### Digest Authentication (RFC 2617)
- ✅ Server generates nonce and realm
- ✅ Client computes MD5(username:realm:password)
- ✅ Client computes response hash
- ✅ Server validates credentials
- ✅ Shared auth-core module for both client and server

## Implementation Details

### Key Files

**session-core-v3:**
- `src/adapters/registration_adapter.rs` - Server-side orchestration
- `src/adapters/dialog_adapter.rs` - Client-side REGISTER sending
- `src/api/unified.rs` - `register()` and `start_registration_server()` APIs

**dialog-core:**
- `src/protocol/register_handler.rs` - REGISTER protocol handling
- `src/events/event_hub.rs` - SendRegisterResponse event handling

**registrar-core:**
- `src/api/mod.rs` - `authenticate_register()` method
- `src/registrar/user_store.rs` - User credential storage

**auth-core:**
- `src/sip_digest.rs` - Shared digest authentication logic

**infra-common:**
- `src/events/cross_crate.rs` - Event definitions

### Event Definitions

```rust
// dialog-core → session-core-v3
DialogToSessionEvent::IncomingRegister {
    transaction_id: String,
    from_uri: String,
    to_uri: String,
    contact_uri: String,
    expires: u32,
    authorization: Option<String>,
    call_id: String,
}

// session-core-v3 → dialog-core
SessionToDialogEvent::SendRegisterResponse {
    transaction_id: String,
    status_code: u16,
    reason: String,
    www_authenticate: Option<String>,
    contact: Option<String>,
    expires: Option<u32>,
}
```

## Testing

Build both examples:
```bash
cargo build --example register_uas
cargo build --example register_uac
```

Run the full flow:
```bash
# Terminal 1
cargo run --example register_uas

# Terminal 2 (after server is ready)
cargo run --example register_uac
```

## Troubleshooting

### "Address already in use"
Make sure no other process is using ports 5060 or 5061:
```bash
lsof -i :5060
lsof -i :5061
```

### Registration times out
- Check that the UAS is running and listening on 5060
- Verify network connectivity between processes
- Check firewall settings

### Authentication fails
- Verify username/password match between UAC and UAS
- Check that realm is consistent ("test.local")
- Review logs for digest computation errors

## Architecture Benefits

1. **Clean Separation** - Client and server run in separate processes
2. **Real Network** - Actual UDP communication between processes  
3. **Event-Driven** - All cross-crate communication via global event bus
4. **Trait-Based** - No unsafe downcasting, uses proper trait methods
5. **Testable** - Each component can be tested independently
6. **Production-Ready** - Architecture scales to multi-server deployments
