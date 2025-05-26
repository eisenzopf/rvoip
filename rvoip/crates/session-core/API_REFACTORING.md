# RVOIP Session Core API Refactoring

## Overview

The RVOIP Session Core API has been refactored to provide a cleaner, more organized structure with separate client and server modules. This refactoring improves code organization, maintainability, and developer experience.

## New API Structure

```
src/
├── api/
│   ├── mod.rs              # Main API module with re-exports
│   ├── client/
│   │   └── mod.rs          # Client-specific API
│   └── server/
│       └── mod.rs          # Server-specific API
├── lib.rs                  # Updated to use new API structure
└── ... (other modules)
```

## Client API (`rvoip_session_core::api::client`)

### Key Components

#### `ClientConfig`
Configuration structure for SIP client applications:

```rust
use rvoip_session_core::api::client::ClientConfig;

let config = ClientConfig {
    display_name: "Alice's Phone".to_string(),
    uri: "sip:alice@example.com".to_string(),
    contact: "sip:alice@192.168.1.100:5060".to_string(),
    auth_user: Some("alice".to_string()),
    auth_password: Some("secret123".to_string()),
    user_agent: "My-SIP-Client/1.0".to_string(),
    max_concurrent_calls: 5,
    auto_answer: false,
    ..Default::default()
};
```

#### `ClientSessionManager`
Enhanced session manager with client-specific functionality:

```rust
use rvoip_session_core::api::client::{create_full_client_manager, ClientConfig};

let client = create_full_client_manager(transaction_manager, config).await?;

// Client operations
let session = client.make_call(destination_uri).await?;
client.answer_call(&session_id).await?;
client.hold_call(&session_id).await?;
client.resume_call(&session_id).await?;
client.transfer_call(&session_id, target_uri, transfer_type).await?;
client.end_call(&session_id).await?;

// Get active calls
let active_calls = client.get_active_calls();

// Registration management
client.set_registered(true);
let is_registered = client.is_registered();
```

#### Factory Functions

- `create_client_session_manager()` - Creates basic SessionManager for client use
- `create_client_session_manager_sync()` - Synchronous version
- `create_full_client_manager()` - Creates full ClientSessionManager with enhanced features
- `create_full_client_manager_sync()` - Synchronous version

## Server API (`rvoip_session_core::api::server`)

### Key Components

#### `ServerConfig`
Configuration structure for SIP server applications:

```rust
use rvoip_session_core::api::server::ServerConfig;

let config = ServerConfig {
    server_name: "My PBX".to_string(),
    domain: "example.com".to_string(),
    max_sessions: 10000,
    session_timeout: 3600,
    max_calls_per_user: 5,
    enable_routing: true,
    enable_transfer: true,
    enable_conference: false,
    user_agent: "My-SIP-Server/1.0".to_string(),
    ..Default::default()
};
```

#### `ServerSessionManager`
Enhanced session manager with server-specific functionality:

```rust
use rvoip_session_core::api::server::{create_full_server_manager, ServerConfig, UserRegistration, RouteInfo};

let server = create_full_server_manager(transaction_manager, config).await?;

// Handle incoming calls
let session = server.handle_incoming_call(&request).await?;

// User registration management
let registration = UserRegistration {
    user_uri: Uri::from_str("sip:bob@example.com")?,
    contact_uri: Uri::from_str("sip:bob@192.168.1.101:5060")?,
    expires: SystemTime::now() + Duration::from_secs(3600),
    user_agent: Some("Client/1.0".to_string()),
};
server.register_user(registration).await?;
server.unregister_user(&user_uri).await?;

// Call routing
let routes = server.route_call(&session_id, &target_uri).await?;
let route = RouteInfo {
    target_uri: Uri::from_str("sip:gateway@192.168.1.1")?,
    priority: 1,
    weight: 100,
    description: "Primary Gateway".to_string(),
};
server.add_route("*.pstn".to_string(), route).await?;

// Server statistics
let stats = server.get_server_stats().await;
println!("Active sessions: {}", stats.active_sessions);
println!("Registered users: {}", stats.registered_users);

// Cleanup
let cleaned = server.cleanup_expired_registrations().await?;
```

#### Supporting Types

- `UserRegistration` - User registration information
- `RouteInfo` - Call routing information
- `ServerStats` - Server statistics

#### Factory Functions

- `create_server_session_manager()` - Creates basic SessionManager for server use
- `create_server_session_manager_sync()` - Synchronous version
- `create_full_server_manager()` - Creates full ServerSessionManager with enhanced features
- `create_full_server_manager_sync()` - Synchronous version

## Main API Module (`rvoip_session_core::api`)

### API Capabilities

```rust
use rvoip_session_core::api::{get_api_capabilities, is_feature_supported};

// Get all capabilities
let capabilities = get_api_capabilities();
println!("Call transfer: {}", capabilities.call_transfer);
println!("Media coordination: {}", capabilities.media_coordination);
println!("Max sessions: {}", capabilities.max_sessions);

// Check specific features
if is_feature_supported("call_transfer") {
    println!("Call transfer is supported");
}
```

### Constants

- `API_VERSION` - Current API version
- `SUPPORTED_SIP_VERSIONS` - Supported SIP protocol versions
- `DEFAULT_USER_AGENT` - Default user agent string

## Migration Guide

### From Old API

**Before:**
```rust
use rvoip_session_core::{client, server};

// Client
let client_manager = client::create_client_session_manager(tm, config).await?;

// Server
let server_manager = server::create_server_session_manager(tm, config).await?;
```

**After:**
```rust
use rvoip_session_core::api::{client, server};

// Client
let client_manager = client::create_full_client_manager(tm, config).await?;

// Server
let server_manager = server::create_full_server_manager(tm, config).await?;
```

### Benefits of New Structure

1. **Clear Separation**: Client and server concerns are clearly separated
2. **Enhanced Functionality**: Each manager provides role-specific methods
3. **Better Configuration**: More comprehensive configuration options
4. **Feature Detection**: Built-in capability detection
5. **Improved Documentation**: Better organized and documented API
6. **Future-Proof**: Easier to extend with new features

## Examples

See `examples/api_demo.rs` for a comprehensive demonstration of the new API structure.

## Compatibility

The new API maintains backward compatibility through re-exports in the main module. However, it's recommended to migrate to the new structured API for better functionality and future compatibility.

## Features Supported

- ✅ Call Transfer (RFC 3515 compliant)
- ✅ Media Coordination during transfers
- ✅ Call Hold/Resume
- ✅ Call Routing
- ✅ User Registration
- ⏳ Conference Calls (planned)

## Performance

The new API structure maintains the same high-performance characteristics:
- Zero-copy event system integration
- Efficient session management
- Optimized media coordination
- Production-ready error handling 