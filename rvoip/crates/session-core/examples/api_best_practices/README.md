# Clean API Examples - Best Practices

This directory contains examples demonstrating the recommended way to use the session-core API.

## Examples

### 1. `uac_client_clean.rs` - Clean UAC Client
Demonstrates a SIP User Agent Client (UAC) using only the public API:
- Uses only `api::*` imports - no internal access
- Leverages `SessionControl` and `MediaControl` trait methods
- Proper error handling and statistics monitoring
- Clean separation of concerns

### 2. `uas_server_clean.rs` - Clean UAS Server  
Demonstrates a SIP User Agent Server (UAS) using only the public API:
- Generates SDP answers using `MediaControl::generate_sdp_answer()`
- Establishes media flow using `MediaControl::establish_media_flow()`
- Monitors call quality with `MediaControl::get_media_statistics()`
- No direct access to internal components

## Key API Features Demonstrated

### ✅ SDP Answer Generation (Fixed!)
The UAS server now properly generates SDP answers using the new API method:
```rust
MediaControl::generate_sdp_answer(&coordinator, &call.id, &sdp_offer).await
```

### ✅ Programmatic Call Control
Both immediate and deferred call acceptance patterns are supported:
```rust
// Immediate (in CallHandler)
CallDecision::Accept(Some(sdp_answer))

// Deferred (programmatic)
CallDecision::Defer
// ... later ...
SessionControl::accept_incoming_call(&coordinator, &call, Some(sdp_answer)).await
```

### ✅ Media Control
Clean media flow establishment and monitoring:
```rust
MediaControl::establish_media_flow(&coordinator, session_id, &remote_addr).await
MediaControl::get_media_statistics(&coordinator, &session_id).await
```

## Running the Examples

### Individual Examples
```bash
# Run the UAS server
cargo run --bin uas_server_clean -- --port 5062 --auto-accept

# In another terminal, run the UAC client
cargo run --bin uac_client_clean -- --target 127.0.0.1:5062 --num-calls 2
```

### Automated Test Script
Run both examples together with automated verification:
```bash
./run_clean_examples.sh
```

The script will:
1. Start the UAS server
2. Wait for it to be ready
3. Run the UAC client to make test calls
4. Verify all API methods work correctly
5. Check that no internal access was used
6. Generate detailed logs and statistics

### Script Options
```bash
# Environment variables
SERVER_PORT=5062        # UAS listening port (default: 5062)
CLIENT_PORT=5061        # UAC port (default: 5061)
NUM_CALLS=3            # Number of calls to make (default: 2)
CALL_DURATION=10       # Duration of each call in seconds (default: 5)
CALL_DELAY=3           # Delay between calls (default: 2)
LOG_LEVEL=debug        # Log level (default: info)

# Example with custom settings
NUM_CALLS=5 CALL_DURATION=15 ./run_clean_examples.sh
```

## Best Practices Demonstrated

1. **Single Import**: Use only `use rvoip_session_core::api::*;`
2. **No Internal Access**: Never access `coordinator.dialog_manager`, etc.
3. **Clean Error Handling**: Proper error propagation and logging
4. **Statistics Monitoring**: Use API methods for monitoring
5. **Proper Shutdown**: Clean shutdown with statistics summary

## Logs

After running the test script, check the `logs/` directory for:
- `clean_api_test_*.log` - Combined test log with analysis
- `uas_clean_*.log` - UAS server detailed logs
- `uac_clean_*.log` - UAC client detailed logs

## Migration from Old Examples

If you're migrating from the older examples that accessed internal components:

### ❌ Old Way (Don't do this)
```rust
// Direct internal access
coordinator.dialog_manager.accept_incoming_call(&session_id).await
coordinator.media_manager.create_media_session(...).await
```

### ✅ New Way (Clean API)
```rust
// Use trait methods
SessionControl::accept_incoming_call(&coordinator, &call, Some(sdp)).await
MediaControl::create_media_session(&coordinator, &session_id).await
```

See the full [Migration Guide](../../src/api/MIGRATION_GUIDE.md) for more details. 