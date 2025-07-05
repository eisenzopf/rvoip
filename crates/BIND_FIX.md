# Bind Address Propagation Fix Plan

## Problem Statement

The bind address configuration (e.g., 173.225.104.102) is not properly propagating from higher-level libraries down to the transport layer. Instead, the libraries are defaulting to 0.0.0.0, causing the server to bind to all interfaces regardless of configuration.

## Design Philosophy

- **Default Behavior**: Use `0.0.0.0` (all interfaces) for servers, `127.0.0.1` (localhost) for clients
- **Specific Configuration**: When a specific IP is provided, it should propagate through all layers
- **Standard Ports**: Use standard SIP port 5060 as default, not port 0 (OS-assigned)
- **Media Ports**: Media can use port 0 since actual ports come from the RTP port range

## Root Causes

1. **client-core** - Missing API call to `with_local_bind_addr()` when building SessionCoordinator
2. **session-core** - Hardcoded "0.0.0.0" addresses in dialog layer instead of using configured bind address
3. **dialog-core** - Already correctly uses configured addresses (no fix needed)

## Implementation Plan

### Phase 1: Client-Core Fixes ✅

#### Task 1.1: Update ClientManager::new() ✅
**File:** `crates/client-core/src/client/manager.rs`  
**Line:** ~449  
**Status:** COMPLETE - Added `.with_local_bind_addr(config.local_sip_addr)` call

#### Task 1.2: Update ClientBuilder for media address ✅
**File:** `crates/client-core/src/client/builder.rs`  
**Line:** ~1108  
**Status:** COMPLETE - Media address now inherits SIP IP when not explicitly set

### Phase 2: Session-Core Fixes ✅

#### Task 2.1: Fix SessionManagerBuilder defaults ✅
**File:** `crates/session-core/src/api/builder.rs`  
**Line:** ~182  
**Status:** COMPLETE - Changed default from `"0.0.0.0:0"` to `"0.0.0.0:5060"` (proper SIP port)

#### Task 2.2: Ensure SessionCoordinator uses configured bind address ✅
**File:** `crates/session-core/src/coordinator/coordinator.rs`  
**Line:** ~81  
**Status:** NO CHANGE NEEDED - Already correct

### Phase 3: Dialog-Core Fixes ✅

#### Task 3.1: Fix DialogBuilder hardcoding ✅
**File:** `crates/session-core/src/dialog/builder.rs`  
**Lines:** ~72 and ~136  
**Status:** COMPLETE - Now uses configured bind address instead of hardcoded "0.0.0.0"

#### Task 3.2: Fix DialogConfigConverter ✅
**File:** `crates/session-core/src/dialog/config.rs`  
**Line:** ~30  
**Status:** COMPLETE - Now uses `self.session_config.local_bind_addr`

#### Task 3.3: Ensure dialog-core respects the bind address ✅
**File:** `crates/dialog-core/src/manager/unified.rs`  
**Status:** NO CHANGE NEEDED - Already correct

### Phase 4: Testing & Verification ✅

#### Task 4.1: Add unit tests for bind address propagation ✅
- `crates/client-core/src/client/tests.rs` - COMPLETE
- Tests added for:
  - `test_bind_address_propagation_via_config`
  - `test_bind_address_propagation_via_builder`
  - `test_media_address_inherits_sip_ip`

#### Task 4.2: Integration test 
**Status:** Not needed - call-engine API has changed

#### Task 4.3: End-to-end verification 
**Status:** PENDING - To be verified with rvoip_sip_server

## Implementation Order

1. ~~Start with Phase 3 (Dialog-Core) - fix the hardcoded addresses~~ ✅
2. ~~Then Phase 2 (Session-Core) - ensure proper defaults~~ ✅
3. ~~Then Phase 1 (Client-Core) - add the missing API call~~ ✅
4. ~~Finally Phase 4 - comprehensive testing~~ ✅ (partially)

## Verification Steps

After implementation:
1. Run unit tests: `cargo test bind_address -p client-core`
2. Run integration test: `cargo test bind_address -p call-engine`
3. Test with rvoip_sip_server binding to specific IP
4. Verify logs show correct bind address at all layers

## Expected Outcome

After these fixes, setting `bind_address = "173.225.104.102:5060"` in configuration should result in:
- Transport layer binding to `173.225.104.102:5060`
- No more `0.0.0.0` addresses in logs
- Server only accessible on the specified interface

## Summary of Changes Made

1. **client-core/src/client/manager.rs** - Added `.with_local_bind_addr()` call when building SessionCoordinator
2. **client-core/src/client/builder.rs** - Media address now inherits SIP address IP but keeps port 0 for automatic allocation
3. **client-core/src/client/config.rs** - Changed defaults:
   - SIP address from `127.0.0.1:0` to `127.0.0.1:5060` (proper SIP port)
   - Media address: `127.0.0.1:0` (port 0 triggers automatic allocation via rtp-core's GlobalPortAllocator)
4. **session-core/src/api/builder.rs** - Changed default bind address from `0.0.0.0:0` to `0.0.0.0:5060` (proper SIP port)
5. **session-core/src/dialog/builder.rs** - Fixed three hardcoded `0.0.0.0` addresses to use configured bind address
6. **session-core/src/dialog/config.rs** - Fixed to use `session_config.local_bind_addr` instead of hardcoded address
7. **client-core/src/client/tests.rs** - Added unit tests for bind address propagation

## Port Allocation Strategy

- **SIP Ports**: Use standard port 5060 by default (not port 0)
- **Media Ports**: Port 0 signals automatic allocation:
  - When media address has port 0, it means "allocate automatically when needed"
  - Actual port allocation happens when session-core creates media sessions
  - This follows the proper layering: client-core → session-core → media-core → rtp-core
- **Bind Address**: Default to 0.0.0.0 for servers, 127.0.0.1 for clients, but respect configured IPs

## Implementation Details

- **Proper Layering**: client-core only uses session-core APIs, not rtp-core directly
- **Lazy Allocation**: Media ports are allocated when actually needed (during calls)
- **Port 0 Semantics**: Port 0 in config means "automatic allocation"
- **Architecture Respect**: Each layer only uses the APIs exposed by the layer below it

## Next Steps

1. Run all tests to ensure changes work correctly ✅
2. Deploy to rvoip_sip_server and verify with specific bind address
3. Monitor logs to confirm transport binds to correct address

## Final Implementation Summary

The bind address propagation issue has been fixed with the following approach:

1. **SIP Bind Address**: Properly propagates from client-core through session-core to dialog-core
2. **Media Port Allocation**: Port 0 in config means "automatic allocation when needed"
3. **Proper Layering**: client-core only uses session-core APIs, maintaining architectural boundaries
4. **Tests**: All bind address tests pass, verifying the propagation works correctly

The fix ensures that when you configure a specific bind address (like 173.225.104.102), it will be properly used at all layers instead of defaulting to 0.0.0.0.

## Additional Tests Created

1. **automatic_port_allocation.rs** - New test file in client-core/tests/ that verifies:
   - Media ports can be set to 0 for automatic allocation
   - Multiple clients can use automatic port allocation without conflicts
   - Bind address propagates correctly with automatic ports
   - Media address inherits IP from SIP address when using automatic ports

All tests pass successfully, and the entire workspace compiles without errors. 