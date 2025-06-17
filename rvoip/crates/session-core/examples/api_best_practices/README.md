# Session-Core API Best Practices

This directory contains reference examples demonstrating the recommended patterns for using the session-core API.

## Why These Examples?

The existing examples in the parent directory were created during early development and sometimes access internal implementation details. While they continue to work, these new examples show the **preferred patterns** for new code.

## Key Principles

### 1. Single Clean Import
```rust
use rvoip_session_core::api::*;  // Everything you need
```

**Not recommended:**
```rust
use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder};
use rvoip_session_core::api::{CallHandler, CallSession};
```

### 2. Use Trait Methods, Not Internal Fields
```rust
// ✅ Good - Using MediaControl trait
MediaControl::create_media_session(&coordinator, &session_id).await?;
MediaControl::generate_sdp_answer(&coordinator, &session_id, offer).await?;

// ❌ Bad - Direct internal access
coordinator.media_manager.create_media_session(&session_id).await?;
```

### 3. Leverage Helper Functions
```rust
// ✅ Good - Using provided parser
let sdp_info = parse_sdp_connection(sdp)?;
let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);

// ❌ Bad - Manual string parsing
let mut ip = None;
let mut port = None;
for line in sdp.lines() {
    // ... manual parsing ...
}
```

## Examples Included

### `uac_client_clean.rs`
Demonstrates that UAC implementations already follow best practices:
- Clean API usage for outgoing calls
- Proper call lifecycle management
- Statistics monitoring via API

### `uas_server_clean.rs`
Shows how UAS servers should be implemented:
- Uses new `generate_sdp_answer()` for SDP negotiation
- No direct access to `media_manager`
- Clean error handling and logging

## Migration Guide

If you have code using the old patterns, here's how to migrate:

### Before (Direct Access):
```rust
// Creating media session
coordinator.media_manager.create_media_session(&call.id).await?;

// Updating with SDP
coordinator.media_manager.update_media_session(&call.id, sdp).await?;

// Generating SDP
coordinator.generate_sdp_offer(&call.id).await?;
```

### After (Clean API):
```rust
// Creating media session
MediaControl::create_media_session(&coordinator, &call.id).await?;

// Updating with SDP
MediaControl::update_remote_sdp(&coordinator, &call.id, sdp).await?;

// Generating SDP answer
MediaControl::generate_sdp_answer(&coordinator, &call.id, offer).await?;
```

## Running the Examples

```bash
# Run the clean UAS server
cargo run --example uas_server_clean -- --port 5062

# In another terminal, run the clean UAC client
cargo run --example uac_client_clean -- --target 127.0.0.1:5062 --num-calls 3
```

## Benefits of These Patterns

1. **Future-proof**: Internal implementation can change without breaking your code
2. **Cleaner**: Less cognitive load, clearer intent
3. **Consistent**: Same patterns work for both UAC and UAS
4. **Type-safe**: Compiler enforces correct usage through trait bounds
5. **Discoverable**: All functionality available through the `api` module

## Questions?

If you need functionality not available through the public API, please open an issue rather than accessing internals directly. The API is designed to be complete for all common use cases. 