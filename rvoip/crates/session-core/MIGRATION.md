# Session-Core Migration to Unified Dialog API

## Overview

This document summarizes the migration of `session-core` from the old split `DialogServer`/`DialogClient` API to the new `UnifiedDialogApi` from `dialog-core`.

## ✅ Completed Changes

### 1. Core Session Manager (`src/session/manager/core.rs`)
- **Updated imports**: Changed from `rvoip_dialog_core::api::{DialogServer, ...}` to `rvoip_dialog_core::UnifiedDialogApi`
- **Updated constructor**: `SessionManager::new()` now takes `Arc<UnifiedDialogApi>` instead of `Arc<DialogServer>`
- **Fixed method calls**: 
  - `dialog_manager.start()` → `dialog_api.start()`
  - `dialog_manager.send_request_in_dialog()` → `dialog_api.send_bye()`
  - `dialog_manager.create_outgoing_dialog()` → `dialog_api.create_dialog()`
  - Added proper call management with `dialog_api.make_call()`

### 2. Client API (`src/api/client/mod.rs`)
- **Updated constructors**: All client factory functions now accept `Arc<UnifiedDialogApi>` 
- **ClientSessionManager**: Updated to use unified API for dependency injection
- **Factory functions**: `create_full_client_manager()` and related functions updated

### 3. Server API (`src/api/server/mod.rs`)
- **Updated constructors**: All server factory functions now accept `Arc<UnifiedDialogApi>`
- **ServerSessionManager**: Updated to use unified API for dependency injection  
- **Factory functions**: `create_full_server_manager()` and related functions updated

### 4. Factory Functions (`src/api/factory.rs`)
- **Updated dependency injection**: `create_sip_server_with_managers()` and `create_sip_client_with_managers()` now use `UnifiedDialogApi`
- **Architecture compliance**: Proper separation between API layer and infrastructure

### 5. Session Manager Components
- **Lifecycle manager**: Updated to use `dialog_api` field instead of `dialog_manager`
- **Transfer manager**: Updated SIP operations to use unified API methods:
  - `send_request_in_dialog()` → `send_refer()` and `send_notify()`

### 6. Example Demonstration
- **Created `unified_api_demo.rs`**: Shows both client and server usage with the new unified API
- **Working example**: Demonstrates proper configuration and usage patterns

## 🏗️ Architecture Improvements

### Before (Split API)
```rust
// Old split API required choosing client or server mode
let dialog_server = Arc::new(DialogServer::new(config));
let dialog_client = Arc::new(DialogClient::new(config));
let session_manager = SessionManager::new(dialog_server, ...);
```

### After (Unified API) 
```rust
// New unified API handles all modes
let dialog_config = DialogManagerConfig::server(bind_addr)
    .with_domain("example.com")
    .build();
let dialog_api = Arc::new(UnifiedDialogApi::create(dialog_config).await?);
let session_manager = SessionManager::new(dialog_api, ...);
```

## 🎯 Benefits Achieved

1. **Simplified Architecture**: Single API for all SIP modes (client/server/hybrid)
2. **Better Abstraction**: Session-core now delegates ALL SIP protocol work to dialog-core
3. **Improved Configuration**: Unified configuration system across all layers
4. **Enhanced Flexibility**: Easy switching between client/server/hybrid modes
5. **Future-Proof**: Ready for upcoming unified features

## 📋 Migration Checklist

- [x] Update core SessionManager to use UnifiedDialogApi
- [x] Update client API to use unified dependency injection  
- [x] Update server API to use unified dependency injection
- [x] Update factory functions for proper architecture
- [x] Fix method calls to use new unified API methods
- [x] Update lifecycle and transfer managers
- [x] Create demonstration example
- [x] Verify core library compilation
- [ ] Update remaining examples and tests (can be done incrementally)

## 🔄 Backward Compatibility

The old API is still available in `dialog-core` for compatibility, but new code should use `UnifiedDialogApi`. The session-core public API remains largely unchanged - only the internal dependency injection has been updated.

## 📚 Usage Examples

### Client Configuration
```rust
use rvoip_session_core::api::client::{ClientConfig, create_full_client_manager};
use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};

let dialog_config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
    .with_from_uri("sip:alice@example.com")
    .build();
let dialog_api = Arc::new(UnifiedDialogApi::create(dialog_config).await?);

let client_config = ClientConfig {
    from_uri: Some("sip:alice@example.com".to_string()),
    max_sessions: 5,
    ..Default::default()
};

let client_manager = create_full_client_manager(dialog_api, client_config).await?;
```

### Server Configuration  
```rust
use rvoip_session_core::api::server::{ServerConfig, create_full_server_manager};
use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};

let dialog_config = DialogManagerConfig::server("127.0.0.1:5060".parse()?)
    .with_domain("example.com")
    .build();
let dialog_api = Arc::new(UnifiedDialogApi::create(dialog_config).await?);

let server_config = ServerConfig {
    server_name: "My PBX".to_string(),
    max_sessions: 1000,
    ..Default::default()
};

let server_manager = create_full_server_manager(dialog_api, server_config).await?;
```

## 🎉 Summary

The migration successfully modernizes session-core to use the unified dialog API while maintaining clean architectural separation. The core library compiles cleanly and the new API provides a much more streamlined and powerful interface for SIP session management. 